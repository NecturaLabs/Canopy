//! Credential prompts for git and OpenSSH.
//!
//! Git never prompts on a terminal here (`GIT_TERMINAL_PROMPT=0`). Instead Canopy registers its
//! own executable as `GIT_ASKPASS` and `SSH_ASKPASS` (see `canopy_core::cli::set_askpass`). When
//! git needs a user name, password or passphrase, or ssh asks about an unknown host key, it runs
//! `canopy "<prompt>"` with [`PORT_ENV`] and [`TOKEN_ENV`] set. That process ([`run_client`])
//! connects to the running app over loopback, the app shows the question in a dialog, and the
//! answer goes back to git on standard output.
//!
//! The listener accepts only connections from 127.0.0.1 that present the random token, which only
//! the git and ssh processes Canopy starts receive. Answers are never logged.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use serde::{Deserialize, Serialize};

/// The port of the running app's listener.
pub const PORT_ENV: &str = "CANOPY_ASKPASS_PORT";
/// The secret a prompt must present.
pub const TOKEN_ENV: &str = "CANOPY_ASKPASS_TOKEN";
/// Set by OpenSSH: `confirm` for a yes/no approval, `none` for a message without an answer.
const SSH_MODE_ENV: &str = "SSH_ASKPASS_PROMPT";
/// Longest request accepted, far above any real prompt.
const MAX_REQUEST: u64 = 64 * 1024;

/// What kind of answer a prompt needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    /// A password, passphrase, PIN or token: typed hidden.
    Secret,
    /// Plain text, such as a user name.
    Text,
    /// A question answered by typing `yes` or `no` (ssh's unknown host key).
    YesNo,
    /// An approval answered by the exit status (`SSH_ASKPASS_PROMPT=confirm`).
    Approve,
    /// A message that needs no answer (`SSH_ASKPASS_PROMPT=none`, e.g. "touch your key").
    Notice,
}

impl PromptKind {
    /// Classifies `prompt`; `ssh_mode` is OpenSSH's `SSH_ASKPASS_PROMPT`.
    pub fn of(prompt: &str, ssh_mode: &str) -> Self {
        match ssh_mode {
            "confirm" => return PromptKind::Approve,
            "none" => return PromptKind::Notice,
            _ => {}
        }
        let lower = prompt.to_lowercase();
        // Words outside quotes: the quoted URL or key path must not decide.
        let words: Vec<&str> = lower
            .split('\'')
            .step_by(2)
            .flat_map(|part| part.split(|c: char| !c.is_alphanumeric()))
            .collect();
        const SECRET: [&str; 7] = [
            "password",
            "passphrase",
            "pin",
            "token",
            "secret",
            "otp",
            "code",
        ];
        if lower.contains("(yes/no") {
            PromptKind::YesNo
        } else if words.iter().any(|w| SECRET.contains(w)) {
            PromptKind::Secret
        } else {
            PromptKind::Text
        }
    }
}

/// A question from git or ssh, waiting for the user. Dropping it without [`Prompt::answer`]
/// cancels it.
#[derive(Debug, Clone)]
pub struct Prompt {
    pub text: String,
    pub kind: PromptKind,
    reply: Sender<Option<String>>,
}

impl Prompt {
    /// A prompt nobody waits for, to render the dialog for screenshots.
    pub fn preview(text: &str) -> Self {
        let (reply, _) = bounded(1);
        Self {
            text: text.to_owned(),
            kind: PromptKind::of(text, ""),
            reply,
        }
    }

    /// Sends the answer, or `None` to cancel (git then fails with its own message).
    pub fn answer(&self, answer: Option<String>) {
        let _ = self.reply.try_send(answer);
    }
}

#[derive(Serialize, Deserialize)]
struct Request {
    token: String,
    prompt: String,
    #[serde(default)]
    mode: String,
}

#[derive(Serialize, Deserialize)]
struct Response {
    answer: Option<String>,
}

/// The app's listener; prompts arrive on [`Server::prompts`].
#[derive(Debug)]
pub struct Server {
    pub prompts: Receiver<Prompt>,
    port: u16,
    token: String,
}

impl Server {
    /// Starts listening on a free loopback port. `wake` runs when a prompt arrives.
    pub fn start(wake: impl Fn() + Send + Sync + 'static) -> std::io::Result<Self> {
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))?;
        let port = listener.local_addr()?.port();
        let token = random_token()?;
        let (tx, rx) = unbounded();
        let expected = token.clone();
        let wake = std::sync::Arc::new(wake);
        std::thread::Builder::new()
            .name("askpass".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    let tx = tx.clone();
                    let expected = expected.clone();
                    let wake = wake.clone();
                    let _ = std::thread::Builder::new()
                        .name("askpass-prompt".into())
                        .spawn(move || {
                            if let Err(e) = serve(stream, &expected, &tx, &*wake) {
                                tracing::debug!("credential prompt connection: {e}");
                            }
                        });
                }
            })?;
        Ok(Self {
            prompts: rx,
            port,
            token,
        })
    }

    /// The environment git and ssh need to reach this server.
    pub fn env(&self) -> Vec<(String, String)> {
        vec![
            (PORT_ENV.into(), self.port.to_string()),
            (TOKEN_ENV.into(), self.token.clone()),
        ]
    }
}

fn random_token() -> std::io::Result<String> {
    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes).map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Equal-time comparison, so the token cannot be guessed byte by byte from response times.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// Handles one connection: checks the token, hands the prompt to the UI and waits for the
/// answer.
fn serve(
    stream: TcpStream,
    token: &str,
    prompts: &Sender<Prompt>,
    wake: &(dyn Fn() + Send + Sync),
) -> std::io::Result<()> {
    if !stream.peer_addr()?.ip().is_loopback() {
        return Ok(());
    }
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut line = String::new();
    BufReader::new((&stream).take(MAX_REQUEST)).read_line(&mut line)?;
    let request: Request = serde_json::from_str(&line).map_err(std::io::Error::other)?;
    if !same(&request.token, token) {
        tracing::warn!("credential prompt with a wrong token refused");
        return Ok(());
    }
    let (reply, answer) = bounded(1);
    let kind = PromptKind::of(&request.prompt, &request.mode);
    let prompt = Prompt {
        text: request.prompt.trim().to_owned(),
        kind,
        reply,
    };
    if prompts.send(prompt).is_err() {
        return Ok(());
    }
    wake();
    // Waits as long as the user needs; a dropped prompt (dialog closed, app exiting) cancels.
    let answer = answer.recv().unwrap_or(None);
    let mut out = serde_json::to_string(&Response { answer }).map_err(std::io::Error::other)?;
    out.push('\n');
    (&stream).write_all(out.as_bytes())
}

/// Asks the app at `port` and returns the answer (`None`: cancelled).
fn ask(port: u16, token: &str, prompt: &str, mode: &str) -> std::io::Result<Option<String>> {
    let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))?;
    let mut request = serde_json::to_string(&Request {
        token: token.to_owned(),
        prompt: prompt.to_owned(),
        mode: mode.to_owned(),
    })
    .map_err(std::io::Error::other)?;
    request.push('\n');
    (&stream).write_all(request.as_bytes())?;
    let mut line = String::new();
    BufReader::new((&stream).take(MAX_REQUEST)).read_line(&mut line)?;
    let response: Response = serde_json::from_str(&line).map_err(std::io::Error::other)?;
    Ok(response.answer)
}

/// Whether this process was started by git or ssh as a credential prompt.
pub fn is_client() -> bool {
    std::env::var_os(PORT_ENV).is_some() && std::env::args_os().len() == 2
}

/// Runs as the credential prompt: asks the app, prints the answer and returns the exit code
/// (0 answered, 1 cancelled or failed).
pub fn run_client() -> i32 {
    let prompt = std::env::args().nth(1).unwrap_or_default();
    let port = std::env::var(PORT_ENV).ok().and_then(|p| p.parse().ok());
    let token = std::env::var(TOKEN_ENV).unwrap_or_default();
    let mode = std::env::var(SSH_MODE_ENV).unwrap_or_default();
    let Some(port) = port else {
        eprintln!("canopy: {PORT_ENV} is not a port number");
        return 1;
    };
    match ask(port, &token, &prompt, &mode) {
        Ok(Some(answer)) => {
            println!("{answer}");
            0
        }
        Ok(None) => 1,
        Err(e) => {
            eprintln!("canopy: could not reach Canopy for the credential prompt: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_are_classified() {
        let k = |p: &str| PromptKind::of(p, "");
        assert_eq!(k("Username for 'https://github.com': "), PromptKind::Text);
        assert_eq!(
            k("Password for 'https://ada@github.com': "),
            PromptKind::Secret
        );
        assert_eq!(
            k("Enter passphrase for key '/home/ada/.ssh/id_ed25519': "),
            PromptKind::Secret
        );
        assert_eq!(
            k("Are you sure you want to continue connecting (yes/no/[fingerprint])? "),
            PromptKind::YesNo
        );
        assert_eq!(
            k("Username for 'https://gitlab.code.org': "),
            PromptKind::Text
        );
        assert_eq!(k("Verification code: "), PromptKind::Secret);
        assert_eq!(
            PromptKind::of("Allow use of key?", "confirm"),
            PromptKind::Approve
        );
        assert_eq!(
            PromptKind::of("Confirm user presence", "none"),
            PromptKind::Notice
        );
    }

    #[test]
    fn tokens_compare_exactly() {
        assert!(same("abc", "abc"));
        assert!(!same("abc", "abd"));
        assert!(!same("abc", "abcd"));
        assert_eq!(random_token().map(|t| t.len()).ok(), Some(48));
    }

    fn answer_with(server: &Server, answer: Option<&str>) -> std::thread::JoinHandle<String> {
        let rx = server.prompts.clone();
        let answer = answer.map(str::to_owned);
        std::thread::spawn(move || {
            let prompt = rx.recv().unwrap_or_else(|e| panic!("no prompt: {e}"));
            let text = prompt.text.clone();
            prompt.answer(answer);
            text
        })
    }

    #[test]
    fn a_prompt_round_trips_through_the_app() -> std::io::Result<()> {
        let server = Server::start(|| {})?;
        let ui = answer_with(&server, Some("hunter2"));
        let answer = ask(server.port, &server.token, "Password for 'https://x': ", "")?;
        assert_eq!(answer.as_deref(), Some("hunter2"));
        assert_eq!(ui.join().ok().as_deref(), Some("Password for 'https://x':"));

        let ui = answer_with(&server, None);
        assert_eq!(ask(server.port, &server.token, "Username: ", "")?, None);
        let _ = ui.join();
        Ok(())
    }

    #[test]
    fn a_wrong_token_gets_no_prompt() -> std::io::Result<()> {
        let server = Server::start(|| {})?;
        assert!(ask(server.port, "not-the-token", "Password: ", "").is_err());
        assert!(server.prompts.try_recv().is_err());
        Ok(())
    }
}
