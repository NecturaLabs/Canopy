//! Credential prompts for git and OpenSSH.
//!
//! Git never prompts on a terminal here (`GIT_TERMINAL_PROMPT=0`). Instead Canopy registers its
//! own executable as `GIT_ASKPASS` and `SSH_ASKPASS` (see `canopy_core::cli::set_askpass`). When
//! git needs a user name, password or passphrase, or ssh asks about an unknown host key, it runs
//! `canopy "<prompt>"` with [`PORT_ENV`] and [`TOKEN_ENV`] set. That process ([`run_client`])
//! connects to the running app over loopback, the app shows the question in a dialog, and the
//! answer goes back to git on standard output.
//!
//! The listener accepts only connections from 127.0.0.1 that present the random token. The token
//! is in the environment of every git command Canopy runs, so the processes git itself starts
//! (hooks, credential helpers, `core.sshCommand`, Git LFS) can read it too and could show a
//! question in Canopy; that is the same trust a terminal gives them. Other local users cannot.
//! At most [`MAX_CONNECTIONS`] prompts are served at once, a request must arrive within
//! [`REQUEST_DEADLINE`], and a prompt whose git process went away closes its dialog. Answers are
//! never logged.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use serde::{Deserialize, Serialize};

/// The port of the running app's listener.
pub const PORT_ENV: &str = "CANOPY_ASKPASS_PORT";
/// The secret a prompt must present.
pub const TOKEN_ENV: &str = "CANOPY_ASKPASS_TOKEN";
/// Set by OpenSSH: `confirm` for a yes/no approval, `none` for a message without an answer.
const SSH_MODE_ENV: &str = "SSH_ASKPASS_PROMPT";
/// Longest request accepted, far above any real prompt.
const MAX_REQUEST: usize = 64 * 1024;
/// Prompts served at the same time; more connections are closed at once.
pub const MAX_CONNECTIONS: usize = 8;
/// How long a client has to send its whole request.
pub const REQUEST_DEADLINE: Duration = Duration::from_secs(10);

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
    /// Classifies `prompt`; `ssh_mode` is OpenSSH's `SSH_ASKPASS_PROMPT`. Anything not known
    /// to be harmless is treated as a secret and typed hidden: only git's and ssh's user-name
    /// questions are plain text.
    pub fn of(prompt: &str, ssh_mode: &str) -> Self {
        match ssh_mode {
            "confirm" => return PromptKind::Approve,
            "none" => return PromptKind::Notice,
            _ => {}
        }
        let lower = prompt.trim_start().to_lowercase();
        if lower.contains("(yes/no") {
            PromptKind::YesNo
        } else if lower.starts_with("username for ")
            || lower.starts_with("username:")
            || lower.starts_with("user name")
            || lower.starts_with("login:")
        {
            PromptKind::Text
        } else {
            PromptKind::Secret
        }
    }
}

/// A question from git or ssh, waiting for the user. Dropping every copy without
/// [`Prompt::answer`] cancels it.
#[derive(Debug, Clone)]
pub struct Prompt {
    pub text: String,
    pub kind: PromptKind,
    reply: Sender<Option<String>>,
    /// Cleared when the asking process went away; the dialog then closes.
    waiting: Arc<AtomicBool>,
}

impl Prompt {
    /// A prompt nobody waits for, to render the dialog for screenshots.
    pub fn preview(text: &str) -> Self {
        let (reply, _) = bounded(1);
        Self {
            text: text.to_owned(),
            kind: PromptKind::of(text, ""),
            reply,
            waiting: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Whether git or ssh still waits for this answer.
    pub fn is_waiting(&self) -> bool {
        self.waiting.load(Ordering::Relaxed)
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
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let open = Arc::new(AtomicUsize::new(0));
        std::thread::Builder::new()
            .name("askpass".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    if open.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
                        tracing::warn!("credential prompt refused: too many open prompts");
                        continue;
                    }
                    open.fetch_add(1, Ordering::SeqCst);
                    let (tx, expected, wake, slots) =
                        (tx.clone(), expected.clone(), wake.clone(), open.clone());
                    let spawned = std::thread::Builder::new()
                        .name("askpass-prompt".into())
                        .spawn(move || {
                            if let Err(e) = serve(stream, &expected, &tx, &*wake) {
                                tracing::debug!("credential prompt connection: {e}");
                            }
                            slots.fetch_sub(1, Ordering::SeqCst);
                        });
                    if spawned.is_err() {
                        open.fetch_sub(1, Ordering::SeqCst);
                    }
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

/// Reads one line from `stream`, giving up at `deadline` or after [`MAX_REQUEST`] bytes.
fn read_request(stream: &TcpStream, deadline: Instant) -> std::io::Result<String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "request not received in time",
            ));
        }
        stream.set_read_timeout(Some(left))?;
        let n = (&*stream).read(&mut chunk)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > MAX_REQUEST {
            return Err(std::io::Error::other("request too long"));
        }
        if let Some(end) = buf.iter().position(|b| *b == b'\n') {
            buf.truncate(end);
            break;
        }
    }
    String::from_utf8(buf).map_err(std::io::Error::other)
}

/// Whether the other end closed the connection (git or ssh gave up or was killed).
fn hung_up(stream: &TcpStream) -> bool {
    if stream.set_nonblocking(true).is_err() {
        return false;
    }
    let closed = matches!(stream.peek(&mut [0u8; 1]), Ok(0));
    let _ = stream.set_nonblocking(false);
    closed
}

/// Handles one connection: checks the token, hands the prompt to the UI and waits for the
/// answer, or until the asking process goes away.
fn serve(
    stream: TcpStream,
    token: &str,
    prompts: &Sender<Prompt>,
    wake: &(dyn Fn() + Send + Sync),
) -> std::io::Result<()> {
    if !stream.peer_addr()?.ip().is_loopback() {
        return Ok(());
    }
    let line = read_request(&stream, Instant::now() + REQUEST_DEADLINE)?;
    let request: Request = serde_json::from_str(&line).map_err(std::io::Error::other)?;
    if !same(&request.token, token) {
        tracing::warn!("credential prompt with a wrong token refused");
        return Ok(());
    }
    let (reply, answer) = bounded(1);
    let kind = PromptKind::of(&request.prompt, &request.mode);
    let waiting = Arc::new(AtomicBool::new(true));
    let prompt = Prompt {
        text: request.prompt.trim().to_owned(),
        kind,
        reply,
        waiting: waiting.clone(),
    };
    tracing::info!(kind = ?kind, "credential prompt");
    if prompts.send(prompt).is_err() {
        return Ok(());
    }
    wake();
    // Waits as long as the user needs. Every copy of the prompt dropped (dialog closed, app
    // exiting) cancels; the client hanging up withdraws the question.
    let answer = loop {
        match answer.recv_timeout(Duration::from_millis(400)) {
            Ok(a) => break a,
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break None,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                if hung_up(&stream) {
                    waiting.store(false, Ordering::Relaxed);
                    wake();
                    tracing::info!(kind = ?kind, "credential prompt withdrawn by git");
                    return Ok(());
                }
            }
        }
    };
    waiting.store(false, Ordering::Relaxed);
    // Whether it was answered, never the answer.
    tracing::info!(kind = ?kind, answered = answer.is_some(), "credential prompt closed");
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
    (&stream)
        .take(MAX_REQUEST as u64)
        .read_to_string(&mut line)?;
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

    #[test]
    fn a_prompt_is_withdrawn_when_the_client_hangs_up() -> std::io::Result<()> {
        let server = Server::start(|| {})?;
        let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, server.port))?;
        let request = format!(
            "{{\"token\":\"{}\",\"prompt\":\"Password: \"}}\n",
            server.token
        );
        (&stream).write_all(request.as_bytes())?;
        let prompt = server
            .prompts
            .recv_timeout(Duration::from_secs(5))
            .map_err(std::io::Error::other)?;
        assert!(prompt.is_waiting());
        drop(stream);
        let deadline = Instant::now() + Duration::from_secs(5);
        while prompt.is_waiting() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!prompt.is_waiting());
        Ok(())
    }

    #[test]
    fn a_silent_client_is_dropped() -> std::io::Result<()> {
        let (a, b) = {
            let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
            let a = TcpStream::connect(l.local_addr()?)?;
            (a, l.accept()?.0)
        };
        let err = read_request(&b, Instant::now() + Duration::from_millis(200));
        assert!(err.is_err());
        drop(a);
        Ok(())
    }
}
