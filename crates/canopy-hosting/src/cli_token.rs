//! The sign-in of the GitHub CLI (`gh`) or the GitLab CLI (`glab`): which of them is installed,
//! whether it is signed in to a host, and the token it holds for that host.
//!
//! The CLI is a local program, not a service: Canopy runs it without a shell, with a fixed
//! argument list and the host as one argument, a short deadline and no prompt, and reads what it
//! prints. The token lives in the returned [`CliLogin`] only, which hides it from `Debug`; it is
//! never logged, written to a file or put in a URL. The CLI is started without the token
//! environment variables, so it reports only its own stored sign-in: the CLI is Canopy's only
//! source of a credential.

use std::ffi::OsStr;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

use canopy_core::forge::ForgeKind;

use crate::http::shorten;

/// How long the CLI may take in all, `auth status` and `auth token` together, before it is killed.
const CLI_TIMEOUT: Duration = Duration::from_secs(10);
/// Bytes read from a pipe at a time.
const DRAIN_CHUNK: usize = 4096;
/// How long the output is read for after the CLI has exited.
const DRAIN_GRACE: Duration = Duration::from_millis(500);
/// How often a running CLI is polled.
const POLL: Duration = Duration::from_millis(10);
/// The most output read from the CLI: a sign-in report is a few lines.
const MAX_OUTPUT: u64 = 64 * 1024;
/// The longest token accepted: real tokens are far shorter; anything longer is not one.
const MAX_TOKEN_LEN: usize = 4096;

/// The variables that hand a CLI a token (or the host to use it for, or a CI job's sign-in)
/// instead of its own stored sign-in.
const CREDENTIAL_ENV: [&str; 15] = [
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GH_ENTERPRISE_TOKEN",
    "GITHUB_ENTERPRISE_TOKEN",
    "GITLAB_TOKEN",
    "GL_TOKEN",
    "GITLAB_ACCESS_TOKEN",
    "OAUTH_TOKEN",
    "GITLAB_HOST",
    "GL_HOST",
    "GITLAB_URI",
    "GITLAB_API_HOST",
    "GH_HOST",
    "GLAB_ENABLE_CI_AUTOLOGIN",
    "CI_JOB_TOKEN",
];

/// What the CLI said about its sign-in to one host, without the token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliAccount {
    /// The account the CLI is signed in as, when it said.
    pub user: Option<String>,
    /// The scopes it listed, `None` when it lists none (`glab` does not).
    pub scopes: Option<Vec<String>>,
}

/// What the CLI holds for one host. Its token stays in memory.
#[derive(Clone, PartialEq, Eq)]
pub struct CliLogin {
    token: String,
    pub account: CliAccount,
}

impl CliLogin {
    /// The token, to be handed to the HTTP client and nowhere else, and the account it belongs to.
    pub fn into_parts(self) -> (String, CliAccount) {
        (self.token, self.account)
    }
}

impl std::fmt::Debug for CliLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CliLogin")
            .field("token", &"<redacted>")
            .field("account", &self.account)
            .finish()
    }
}

/// Why the CLI's sign-in cannot be used. The text is meant for the user.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CliError {
    /// The program is not on `PATH`: nothing to report, the next credential source applies.
    #[error("The {} is not installed.", cli_name(*.kind))]
    NotInstalled { kind: ForgeKind },
    /// Installed, but not signed in to the host.
    #[error(
        "The {} is not signed in to {host}. Run `{} auth login --hostname {host}` to sign in.",
        cli_name(*.kind),
        program(*.kind)
    )]
    SignedOut { kind: ForgeKind, host: String },
    /// Signed in, but the token lacks a scope Canopy needs.
    #[error(
        "The {} is signed in to {host}{}, but its token lacks the `{scope}` scope. {}",
        cli_name(*.kind),
        as_user(user.as_deref()),
        scope_fix(*.kind, host)
    )]
    MissingScope {
        kind: ForgeKind,
        host: String,
        user: Option<String>,
        scope: &'static str,
    },
    /// It ran and failed, took too long, or printed something Canopy cannot use.
    #[error("The {} could not be used: {message}", cli_name(*.kind))]
    Failed { kind: ForgeKind, message: String },
}

impl CliError {
    /// The provider whose CLI this is about.
    pub const fn kind(&self) -> ForgeKind {
        match self {
            Self::NotInstalled { kind }
            | Self::SignedOut { kind, .. }
            | Self::MissingScope { kind, .. }
            | Self::Failed { kind, .. } => *kind,
        }
    }

    /// The account the CLI is signed in as, when the error knows it.
    pub fn user(&self) -> Option<&str> {
        match self {
            Self::MissingScope { user, .. } => user.as_deref(),
            _ => None,
        }
    }

    /// What to do besides running [`Self::fix_command`]: GitLab's sign-in asks which scopes to
    /// grant, so the missing one has to be allowed there.
    pub fn fix_note(&self) -> Option<String> {
        match self {
            Self::MissingScope {
                kind: ForgeKind::GitLab,
                scope,
                ..
            } => Some(format!("allow the {scope} scope")),
            _ => None,
        }
    }

    /// The command that fixes it, to run in a terminal: signing in, or adding the missing scope.
    pub fn fix_command(&self) -> Option<String> {
        match self {
            Self::SignedOut { kind, host } => Some(login_command(*kind, host)),
            Self::MissingScope { kind, host, .. } => Some(scope_command(*kind, host)),
            Self::NotInstalled { .. } | Self::Failed { .. } => None,
        }
    }
}

/// The command that signs the CLI in to `host`.
pub fn login_command(kind: ForgeKind, host: &str) -> String {
    format!("{} auth login --hostname {host}", program(kind))
}

/// The command that gives the CLI's sign-in to `host` the scope Canopy needs.
fn scope_command(kind: ForgeKind, host: &str) -> String {
    match kind {
        ForgeKind::GitHub => format!("gh auth refresh --hostname {host} --scopes repo"),
        ForgeKind::GitLab => login_command(kind, host),
    }
}

/// The CLI's official install page.
pub fn install_page(kind: ForgeKind) -> &'static str {
    match kind {
        ForgeKind::GitHub => "https://cli.github.com",
        ForgeKind::GitLab => "https://gitlab.com/gitlab-org/cli#installation",
    }
}

fn cli_name(kind: ForgeKind) -> &'static str {
    match kind {
        ForgeKind::GitHub => "GitHub CLI (gh)",
        ForgeKind::GitLab => "GitLab CLI (glab)",
    }
}

fn program(kind: ForgeKind) -> &'static str {
    match kind {
        ForgeKind::GitHub => "gh",
        ForgeKind::GitLab => "glab",
    }
}

/// The scope Canopy needs: GitHub's `repo` and GitLab's `api` (reading and acting on requests).
fn required_scope(kind: ForgeKind) -> &'static str {
    match kind {
        ForgeKind::GitHub => "repo",
        ForgeKind::GitLab => "api",
    }
}

fn as_user(user: Option<&str>) -> String {
    user.map_or_else(String::new, |u| format!(" as {u}"))
}

fn scope_fix(kind: ForgeKind, host: &str) -> String {
    let command = scope_command(kind, host);
    match kind {
        ForgeKind::GitHub => format!("Run `{command}` to add it."),
        ForgeKind::GitLab => format!(
            "Run `{command}` and allow the {} scope.",
            required_scope(kind)
        ),
    }
}

/// The token the CLI holds for `host` (`name[:port]`), with the account it belongs to.
///
/// Blocking: runs `gh auth status` and `gh auth token`, or `glab auth status --show-token`, for
/// at most ten seconds in all, so call it from a background thread.
pub fn login_from_cli(kind: ForgeKind, host: &str) -> Result<CliLogin, CliError> {
    login_with(kind, host, OsStr::new(program(kind)), CLI_TIMEOUT)
}

/// [`login_from_cli`] with the program to run and the time it may take, so tests can use a fake
/// one and a short deadline.
fn login_with(
    kind: ForgeKind,
    host: &str,
    program: &OsStr,
    timeout: Duration,
) -> Result<CliLogin, CliError> {
    let result = read_login(kind, host, program, timeout);
    // Neither arm names the token: a login's `Debug` hides it and an error's words are `shorten`ed.
    match &result {
        Ok(_) => tracing::debug!(host, cli = self::program(kind), "using the CLI's sign-in"),
        Err(CliError::NotInstalled { .. }) => {
            tracing::debug!(cli = self::program(kind), "the CLI is not installed");
        }
        Err(e) => {
            tracing::info!(host, cli = self::program(kind), problem = %e, "the CLI's sign-in cannot be used");
        }
    }
    result
}

fn read_login(
    kind: ForgeKind,
    host: &str,
    program: &OsStr,
    timeout: Duration,
) -> Result<CliLogin, CliError> {
    let host = host.trim();
    let failed = |message: &str| CliError::Failed {
        kind,
        message: shorten(message),
    };
    // Only a host name (and port) ever reaches a CLI or the command Canopy offers to copy: nothing
    // that could be taken for an option, a second argument or shell syntax.
    if !is_host_name(host) {
        return Err(failed("the host name is not valid"));
    }
    let deadline = Instant::now() + timeout;
    let run = |args: &[&str]| match run_capture(program, args, deadline) {
        Ok(out) => Ok(out),
        Err(RunError::NotFound) => Err(CliError::NotInstalled { kind }),
        Err(RunError::TimedOut) => Err(failed(&took_longer(timeout))),
        Err(RunError::Io(e)) => Err(failed(&format!("it could not be run ({e})"))),
    };
    let status_args: &[&str] = match kind {
        ForgeKind::GitHub => &["auth", "status", "--hostname", host],
        ForgeKind::GitLab => &["auth", "status", "--hostname", host, "--show-token"],
    };
    let status = run(status_args)?;
    let report = parse_status(&status.text, host);
    // The CLI exits non-zero when any account on the host has a problem; a line saying the host
    // is signed in still counts.
    if !report.signed_in {
        let signed_out = {
            let t = status.text.to_lowercase();
            SIGNED_OUT_WORDS.iter().any(|w| t.contains(w))
        };
        return Err(if signed_out {
            CliError::SignedOut {
                kind,
                host: host.to_owned(),
            }
        } else {
            let hint = hide(&failure_lines(&status.text), report.token.as_deref());
            failed(if hint.is_empty() {
                "it printed nothing"
            } else {
                &hint
            })
        });
    }
    let scope = required_scope(kind);
    if report
        .scopes
        .as_ref()
        .is_some_and(|s| !s.iter().any(|have| have == scope))
    {
        return Err(CliError::MissingScope {
            kind,
            host: host.to_owned(),
            user: report.user,
            scope,
        });
    }
    let token = match kind {
        ForgeKind::GitHub => {
            let out = run(&["auth", "token", "--hostname", host])?;
            // Standard error may carry a warning (a keyring notice); the token is on standard output.
            if out.success {
                Some(out.stdout.trim().to_owned())
            } else {
                return Err(failed(&hide(&out.text, None)));
            }
        }
        ForgeKind::GitLab => report.token,
    };
    match token.filter(|t| usable(t)) {
        Some(token) => Ok(CliLogin {
            token,
            account: CliAccount {
                user: report.user,
                scopes: report.scopes,
            },
        }),
        None => Err(failed("it printed no usable token")),
    }
}

/// Why a CLI that ran past `timeout` was stopped, in the unit the timeout was given in.
fn took_longer(timeout: Duration) -> String {
    if timeout.subsec_nanos() == 0 {
        format!("it took longer than {} seconds", timeout.as_secs())
    } else {
        format!("it took longer than {} ms", timeout.as_millis())
    }
}

/// The most digits a TCP port has (65535).
const PORT_DIGITS: usize = 5;

/// Whether `host` is `name` or `name:port`, the name made of letters, digits, `.`, `_` and `-` and not
/// starting with `-` or `.`.
fn is_host_name(host: &str) -> bool {
    let (name, port) = host
        .split_once(':')
        .map_or((host, None), |(n, p)| (n, Some(p)));
    !name.is_empty()
        && !name.starts_with(['-', '.'])
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && port.is_none_or(|p| {
            !p.is_empty() && p.len() <= PORT_DIGITS && p.bytes().all(|b| b.is_ascii_digit())
        })
}

/// Whether the key of an `auth status` line, as `glab` and `gh` word it, introduces a token.
fn names_a_token(key: &str) -> bool {
    let key = key.to_lowercase();
    key == "token" || key.starts_with("token found")
}

/// What a CLI says when it is not signed in to a host.
const SIGNED_OUT_WORDS: [&str; 8] = [
    "not logged",
    "no accounts",
    "no token",
    "no oauth token",
    "not authenticated",
    "not been authenticated",
    "have been authenticated",
    "unknown host",
];

/// The lines of a failing `auth status` that say what failed, when any do: the host's name and the
/// lines about protocols and endpoints around them only get in the way.
fn failure_lines(text: &str) -> String {
    const WORDS: [&str; 6] = ["fail", "error", "denied", "invalid", "expired", "unauthor"];
    let said: Vec<&str> = text
        .lines()
        .filter(|line| {
            let line = line.to_lowercase();
            WORDS.iter().any(|w| line.contains(w))
        })
        .collect();
    if said.is_empty() {
        text.to_owned()
    } else {
        said.join("\n")
    }
}

/// CLI output made safe to show: no token the CLI printed (a line that names a token has its value
/// replaced, whoever it belongs to), no known token shapes.
fn hide(text: &str, token: Option<&str>) -> String {
    let text = match token {
        Some(t) if !t.is_empty() => text.replace(t, "<redacted>"),
        _ => text.to_owned(),
    };
    let text: Vec<String> = text
        .lines()
        .map(|line| {
            let bare = line.trim_start_matches(|c: char| !c.is_alphanumeric());
            match bare.split_once(':') {
                Some((key, _)) if names_a_token(key) => format!("{key}: <redacted>"),
                _ => line.to_owned(),
            }
        })
        .collect();
    shorten(&text.join("\n"))
}

/// A token is one visible word, no longer than any real one, and not one the CLI masked.
fn usable(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= MAX_TOKEN_LEN
        && token.chars().all(|c| c.is_ascii_graphic())
        && !masked(token)
}

/// The fewest asterisks in a row that mark a token the CLI masked (`glpat-****`).
const MASK_RUN: &str = "****";

/// Whether the CLI printed a masked token instead of the token: all asterisks, or a run of them.
fn masked(token: &str) -> bool {
    token.chars().all(|c| c == '*') || token.contains(MASK_RUN)
}

/// What a CLI's `auth status` said about one host.
#[derive(Default)]
struct StatusReport {
    signed_in: bool,
    user: Option<String>,
    /// The scopes it listed, `None` when it listed none.
    scopes: Option<Vec<String>>,
    /// A token it printed (`glab auth status --show-token`).
    token: Option<String>,
}

/// One account the CLI listed for the host.
#[derive(Default)]
struct Account {
    /// `Active account: true`: the one `gh auth token` answers for.
    active: bool,
    report: StatusReport,
}

/// Reads `gh auth status` and `glab auth status` output:
///
/// ```text
///   ✓ Logged in to github.com account octocat (keyring)
///   - Active account: true
///   - Token scopes: 'gist', 'read:org', 'repo'
///   ✓ Logged in to gitlab.example.com as octocat (/home/o/.config/glab-cli/config.yml)
///   ✓ Token found in operating system keyring: glpat-…
/// ```
///
/// Each `Logged in to` line opens an account. Only accounts of exactly `host` (name and port) are
/// read, and a line belongs to the account above it, so another host's scopes or token are never
/// taken for this one's. Of several accounts on the host, the active one counts, else the first.
fn parse_status(text: &str, host: &str) -> StatusReport {
    let host = host.to_lowercase();
    let mut accounts: Vec<Account> = Vec::new();
    // Whether the lines being read belong to an account of `host` (the last one in `accounts`).
    let mut in_host = false;
    for line in text.lines() {
        let line = line
            .trim_start_matches(|c: char| !c.is_alphanumeric())
            .trim();
        let lower = line.to_lowercase();
        if let Some(rest) = lower.strip_prefix("logged in to ") {
            in_host = rest.split_whitespace().next() == Some(host.as_str());
            if in_host {
                let user = line
                    .split_whitespace()
                    .skip_while(|w| !matches!(*w, "account" | "as"))
                    .nth(1)
                    .map(str::to_owned);
                accounts.push(Account {
                    active: false,
                    report: StatusReport {
                        signed_in: true,
                        user,
                        ..StatusReport::default()
                    },
                });
            }
        } else if lower.starts_with("failed to log in to ") {
            // An account that does not work: its lines are not those of the account above it.
            in_host = false;
        } else if in_host
            && let Some(account) = accounts.last_mut()
            && let Some((key, value)) = line.split_once(':')
        {
            let value = value.trim();
            match key.trim().to_lowercase().as_str() {
                "active account" => account.active = value.eq_ignore_ascii_case("true"),
                "token scopes" => {
                    let scopes: Vec<String> = value
                        .split(',')
                        .map(|s| s.trim().trim_matches(['\'', '"']).to_owned())
                        .filter(|s| !s.is_empty() && s != "none")
                        .collect();
                    account.report.scopes = (!scopes.is_empty()).then_some(scopes);
                }
                // `glab` words it "Token found: …" (older) or "Token found in <where it is
                // stored>: …" (newer).
                key if (key == "token" || key.starts_with("token found"))
                    && !value.is_empty()
                    && !masked(value) =>
                {
                    account.report.token = Some(value.to_owned());
                }
                _ => {}
            }
        }
    }
    let chosen = accounts.iter().position(|a| a.active).unwrap_or(0);
    accounts
        .into_iter()
        .nth(chosen)
        .map(|a| a.report)
        .unwrap_or_default()
}

/// What a finished CLI printed.
struct Captured {
    success: bool,
    /// Standard output, then standard error.
    text: String,
    /// Standard output alone.
    stdout: String,
}

enum RunError {
    NotFound,
    TimedOut,
    Io(std::io::Error),
}

#[expect(
    clippy::disallowed_methods,
    reason = "the GitHub and GitLab CLIs are the only programs besides git that canopy-hosting runs, and only to read their sign-in"
)]
fn command(program: &OsStr) -> Command {
    let mut cmd = Command::new(program);
    for name in CREDENTIAL_ENV {
        cmd.env_remove(name);
    }
    cmd.env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("GLAB_CHECK_UPDATE", "false")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// A pipe being read on a thread, so a full pipe never stalls the child.
struct Drain {
    /// What has been read so far.
    buf: Arc<Mutex<Vec<u8>>>,
    /// Sent to when the pipe reached its end (or [`MAX_OUTPUT`]).
    done: mpsc::Receiver<()>,
}

impl Drain {
    fn start<R: Read + Send + 'static>(pipe: Option<R>) -> Self {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let (tx, done) = mpsc::channel();
        let shared = Arc::clone(&buf);
        // The thread outlives the call when a grandchild of the CLI keeps the pipe open; it
        // holds nothing but the pipe and ends with it.
        std::thread::spawn(move || {
            if let Some(pipe) = pipe {
                let mut pipe = pipe.take(MAX_OUTPUT);
                let mut chunk = [0_u8; DRAIN_CHUNK];
                while let Ok(n @ 1..) = pipe.read(&mut chunk) {
                    shared
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .extend_from_slice(&chunk[..n]);
                }
            }
            let _ = tx.send(());
        });
        Self { buf, done }
    }

    /// What was read, waiting for the end of the pipe no longer than until `until`: a program the
    /// CLI started may hold the pipe open for good, and what the CLI itself wrote is there already.
    fn text(&self, until: Instant) -> String {
        let _ = self
            .done
            .recv_timeout(until.saturating_duration_since(Instant::now()));
        let bytes = self.buf.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// Runs `program args` and collects its output, killing it at `deadline`. Reading the output ends
/// by then too, however long a program the CLI started keeps its pipes.
fn run_capture(program: &OsStr, args: &[&str], deadline: Instant) -> Result<Captured, RunError> {
    let mut child = command(program).args(args).spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            RunError::NotFound
        } else {
            RunError::Io(e)
        }
    })?;
    let out = Drain::start(child.stdout.take());
    let err = Drain::start(child.stderr.take());
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RunError::TimedOut);
            }
            Ok(None) => std::thread::sleep(POLL),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RunError::Io(e));
            }
        }
    };
    let until = (Instant::now() + DRAIN_GRACE).min(deadline);
    let stdout = out.text(until);
    let text = format!("{stdout}\n{}", err.text(until));
    Ok(Captured {
        success: status.success(),
        text,
        stdout,
    })
}

#[cfg(test)]
#[cfg(unix)]
#[expect(
    clippy::significant_drop_tightening,
    reason = "a fake CLI holds the lock that serialises the tests until the test ends"
)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    /// Fake programs are written and run one at a time: another thread forking while a script's
    /// file is still open for writing makes its `exec` fail with "text file busy".
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

    const TOKEN: &str = "gho_SECRETSECRETSECRET1234567890";

    /// A fake CLI that runs `body` (shell) and whose argument log is written to `args.log`.
    struct Fake {
        dir: tempfile::TempDir,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl Fake {
        fn new(body: &str) -> Self {
            let guard = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("cli");
            let script = format!(
                "#!/bin/sh\necho \"$@\" >> '{}/args.log'\n{body}\n",
                dir.path().display()
            );
            std::fs::write(&path, script).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            Self { dir, _guard: guard }
        }

        fn path(&self) -> PathBuf {
            self.dir.path().join("cli")
        }

        fn calls(&self) -> Vec<String> {
            std::fs::read_to_string(self.dir.path().join("args.log"))
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect()
        }

        fn login(&self, kind: ForgeKind, host: &str) -> Result<CliLogin, CliError> {
            login_with(kind, host, self.path().as_os_str(), CLI_TIMEOUT)
        }
    }

    fn gh_signed_in(scopes: &str) -> String {
        format!(
            r#"case "$2" in
status) echo "github.com"; echo "  ✓ Logged in to github.com account octocat (keyring)"; echo "  - Token: gho_****"; echo "  - Token scopes: {scopes}";;
token) echo "{TOKEN}";;
esac"#
        )
    }

    #[test]
    fn github_signed_in() {
        let fake = Fake::new(&gh_signed_in("'gist', 'read:org', 'repo'"));
        let login = fake.login(ForgeKind::GitHub, "github.com").unwrap();
        assert_eq!(login.account.user.as_deref(), Some("octocat"));
        assert_eq!(
            login.account.scopes,
            Some(vec!["gist".into(), "read:org".into(), "repo".into()])
        );
        assert_eq!(login.clone().into_parts().0, TOKEN);
        assert_eq!(
            fake.calls(),
            [
                "auth status --hostname github.com",
                "auth token --hostname github.com"
            ]
        );
        let shown = format!("{login:?}");
        assert!(
            !shown.contains(TOKEN) && !shown.contains("SECRET"),
            "{shown}"
        );
    }

    #[test]
    fn github_without_the_repo_scope() {
        let fake = Fake::new(&gh_signed_in("'gist', 'read:org'"));
        let err = fake.login(ForgeKind::GitHub, "github.com").unwrap_err();
        assert!(
            matches!(&err, CliError::MissingScope { scope: "repo", .. }),
            "{err:?}"
        );
        assert_eq!(err.user(), Some("octocat"));
        let text = err.to_string();
        assert!(
            text.contains("signed in to github.com as octocat"),
            "{text}"
        );
        assert!(
            text.contains("gh auth refresh --hostname github.com --scopes repo"),
            "{text}"
        );
        assert!(!text.contains(TOKEN));
        assert_eq!(fake.calls().len(), 1, "the token is not even asked for");
    }

    #[test]
    fn github_without_a_scope_list_is_used() {
        let fake = Fake::new(&gh_signed_in("none"));
        fake.login(ForgeKind::GitHub, "github.com").unwrap();
    }

    #[test]
    fn github_signed_out() {
        let fake = Fake::new(
            r#"echo "You are not logged into any accounts on github.com" >&2; echo "To log in, run: gh auth login" >&2; exit 1"#,
        );
        let err = fake.login(ForgeKind::GitHub, "github.com").unwrap_err();
        assert!(matches!(&err, CliError::SignedOut { .. }), "{err:?}");
        assert!(
            err.to_string()
                .contains("gh auth login --hostname github.com")
        );
    }

    #[test]
    fn github_failing_and_leaking_a_token_in_its_words() {
        let fake = Fake::new(&format!(
            r#"echo "X Failed to log in to github.com account octocat (keyring)" >&2; echo " - The token {TOKEN} is invalid" >&2; exit 1"#
        ));
        let err = fake.login(ForgeKind::GitHub, "github.com").unwrap_err();
        let CliError::Failed { message, .. } = &err else {
            panic!("{err:?}");
        };
        assert!(message.contains("Failed to log in"), "{message}");
        assert!(!format!("{err} {err:?}").contains("SECRET"), "{err:?}");
    }

    #[test]
    fn github_token_command_failing() {
        let fake = Fake::new(
            r#"case "$2" in
status) echo "  ✓ Logged in to github.com account octocat (keyring)";;
token) echo "keyring is locked" >&2; exit 1;;
esac"#,
        );
        let err = fake.login(ForgeKind::GitHub, "github.com").unwrap_err();
        assert!(err.to_string().contains("keyring is locked"), "{err}");
    }

    #[test]
    fn cli_not_installed() {
        let _guard = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("no-such-cli");
        for kind in [ForgeKind::GitHub, ForgeKind::GitLab] {
            let err = login_with(kind, "github.com", missing.as_os_str(), CLI_TIMEOUT).unwrap_err();
            assert_eq!(err, CliError::NotInstalled { kind });
        }
    }

    #[test]
    fn a_hung_cli_is_killed() {
        let fake = Fake::new("sleep 30");
        let started = Instant::now();
        let deadline = started + Duration::from_millis(300);
        let result = run_capture(fake.path().as_os_str(), &["auth"], deadline);
        assert!(matches!(result, Err(RunError::TimedOut)));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    /// A CLI that hangs while asked for its sign-in ends in a failure that says so, in time.
    #[test]
    fn a_hung_sign_in_check_fails_in_time() {
        let fake = Fake::new("sleep 30");
        let started = Instant::now();
        let err = login_with(
            ForgeKind::GitHub,
            "github.com",
            fake.path().as_os_str(),
            Duration::from_millis(300),
        )
        .unwrap_err();
        assert!(
            matches!(&err, CliError::Failed { message, .. } if message.contains("took longer than 300 ms")),
            "{err:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    /// A warning on standard error (a keyring notice) does not spoil the token on standard output.
    #[test]
    fn a_warning_beside_the_token_is_not_part_of_it() {
        let fake = Fake::new(&format!(
            r#"case "$2" in
status) echo "  ✓ Logged in to github.com account octocat (keyring)"; echo "  - Token scopes: 'repo'";;
token) echo "warning: the keyring is slow to answer" >&2; echo "{TOKEN}";;
esac"#
        ));
        let login = fake.login(ForgeKind::GitHub, "github.com").unwrap();
        assert_eq!(login.into_parts().0, TOKEN);
    }

    /// Output past [`MAX_OUTPUT`] is not read: a sign-in report that starts beyond it does not
    /// count, what is shown stays short, and a CLI that keeps writing does not hold the caller.
    #[test]
    fn output_past_the_limit_is_not_read() {
        let signed_in = format!(
            r#"echo "  ✓ Logged in to gitlab.com as tanuki" >&2; echo "  ✓ Token found: {TOKEN}" >&2"#
        );
        let fake = Fake::new(&signed_in);
        assert!(
            fake.login(ForgeKind::GitLab, "gitlab.com").is_ok(),
            "the report alone is read"
        );
        drop(fake);
        let fake = Fake::new(&format!(
            "head -c {} /dev/zero | tr '\\0' 'a' >&2; echo >&2\n{signed_in}",
            MAX_OUTPUT * 2
        ));
        let started = Instant::now();
        let err = fake.login(ForgeKind::GitLab, "gitlab.com").unwrap_err();
        let CliError::Failed { message, .. } = &err else {
            panic!("{err:?}");
        };
        assert!(!message.contains(TOKEN));
        assert!(message.len() < 1024, "{}", message.len());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    /// Bytes that are not UTF-8 neither stop a report from being read nor pass as a token.
    #[test]
    fn output_that_is_not_utf8() {
        let fake = Fake::new(&format!(
            r#"case "$2" in
status) printf '\377\376 github.com\n'; echo "  ✓ Logged in to github.com account octocat (keyring)";;
token) echo "{TOKEN}";;
esac"#
        ));
        let login = fake.login(ForgeKind::GitHub, "github.com").unwrap();
        assert_eq!(login.account.user.as_deref(), Some("octocat"));
        drop(fake);
        let fake = Fake::new(
            r#"case "$2" in
status) echo "  ✓ Logged in to github.com account octocat (keyring)";;
token) printf 'gho_\377\376abc\n';;
esac"#,
        );
        assert!(matches!(
            fake.login(ForgeKind::GitHub, "github.com"),
            Err(CliError::Failed { .. })
        ));
    }

    /// A token the CLI masked (`glpat-****`) is never taken for the token.
    #[test]
    fn a_masked_token_is_not_used() {
        for shown in ["glpat-****", "glpat-****abcd", "**********"] {
            let fake = Fake::new(&format!(
                r#"echo "  ✓ Logged in to gitlab.com as tanuki" >&2; echo "  ✓ Token found: {shown}" >&2"#
            ));
            assert!(
                matches!(
                    fake.login(ForgeKind::GitLab, "gitlab.com"),
                    Err(CliError::Failed { .. })
                ),
                "{shown}"
            );
            drop(fake);
            assert!(!usable(shown), "{shown}");
        }
        assert!(usable(TOKEN));
    }

    #[test]
    fn gitlab_signed_in_on_a_self_hosted_host() {
        let fake = Fake::new(&format!(
            r#"echo "gitlab.corp:8443" >&2
echo "  ✓ Logged in to gitlab.corp:8443 as tanuki (/home/t/.config/glab-cli/config.yml)" >&2
echo "  ✓ Token found: {TOKEN}" >&2"#
        ));
        let login = fake.login(ForgeKind::GitLab, "gitlab.corp:8443").unwrap();
        assert_eq!(login.account.user.as_deref(), Some("tanuki"));
        assert_eq!(login.into_parts().0, TOKEN);
        assert_eq!(
            fake.calls(),
            ["auth status --hostname gitlab.corp:8443 --show-token"]
        );
    }

    /// What `glab auth status --show-token` prints in the source of its current release
    /// (`internal/commands/auth/status/status.go`): the report goes to standard error, the token
    /// line names where the token is stored, and no scopes are listed.
    #[test]
    fn gitlab_signed_in_as_the_current_glab_words_it() {
        let fake = Fake::new(&format!(
            r#"echo "gitlab.com" >&2
echo "  ✓ Logged in to gitlab.com as tanuki (keyring)" >&2
echo "  ✓ Git operations for gitlab.com configured to use ssh protocol." >&2
echo "  ✓ API calls for gitlab.com are made over https protocol." >&2
echo "  ✓ REST API Endpoint: https://gitlab.com/api/v4/" >&2
echo "  ✓ GraphQL Endpoint: https://gitlab.com/api/graphql/" >&2
echo "  ✓ Token found in operating system keyring: {TOKEN}" >&2"#
        ));
        let login = fake.login(ForgeKind::GitLab, "gitlab.com").unwrap();
        assert_eq!(login.account.user.as_deref(), Some("tanuki"));
        assert_eq!(login.account.scopes, None);
        assert_eq!(login.into_parts().0, TOKEN);
    }

    #[test]
    fn gitlab_not_signed_in_to_the_host() {
        for words in [
            "✗ gitlab.corp has not been authenticated with glab; run `glab auth login --hostname gitlab.corp` to authenticate",
            "no GitLab instances have been authenticated with glab; run `glab auth login` to authenticate",
        ] {
            let fake = Fake::new(&format!("echo '{words}' >&2; exit 1"));
            let err = fake.login(ForgeKind::GitLab, "gitlab.corp").unwrap_err();
            assert_eq!(
                err,
                CliError::SignedOut {
                    kind: ForgeKind::GitLab,
                    host: "gitlab.corp".into()
                },
                "{words}"
            );
            assert_eq!(
                err.fix_command().as_deref(),
                Some("glab auth login --hostname gitlab.corp")
            );
        }
    }

    /// A host whose token the CLI cannot find has a sign-in line but nothing to hand over.
    #[test]
    fn gitlab_signed_in_without_a_token_to_hand_over() {
        let fake = Fake::new(
            r#"echo "  ✓ Logged in to gitlab.corp as tanuki (keyring)" >&2
echo "  ! No token found (checked config file, keyring, and environment variables)." >&2"#,
        );
        let err = fake.login(ForgeKind::GitLab, "gitlab.corp").unwrap_err();
        assert!(
            matches!(&err, CliError::Failed { message, .. } if message.contains("no usable token")),
            "{err:?}"
        );
    }

    #[test]
    fn a_failing_gitlab_sign_in_is_named_without_its_token() {
        let fake = Fake::new(&format!(
            r#"echo "  ✗ gitlab.com: API call failed: 401 Unauthorized" >&2
echo "  ✓ Token found in configuration file (plaintext): {TOKEN}" >&2
exit 1"#
        ));
        let err = fake.login(ForgeKind::GitLab, "gitlab.com").unwrap_err();
        let CliError::Failed { message, .. } = &err else {
            panic!("{err:?}");
        };
        assert_eq!(message, "✗ gitlab.com: API call failed: 401 Unauthorized");
        assert!(!format!("{err} {err:?}").contains("SECRET"), "{err:?}");
    }

    #[test]
    fn the_active_account_of_a_host_decides_the_scopes() {
        // The inactive account is listed last and lacks `repo`; `gh auth token` answers for the
        // active one.
        let fake = Fake::new(&format!(
            r#"case "$2" in
status) echo "github.com"
echo "  ✓ Logged in to github.com account hubot (keyring)"; echo "  - Active account: true"; echo "  - Token scopes: 'repo'"
echo "  ✓ Logged in to github.com account octocat (keyring)"; echo "  - Active account: false"; echo "  - Token scopes: 'gist'";;
token) echo "{TOKEN}";;
esac"#
        ));
        let login = fake.login(ForgeKind::GitHub, "github.com").unwrap();
        assert_eq!(login.account.user.as_deref(), Some("hubot"));
        drop(fake);
        // And the other way round: the active account lacks the scope.
        let fake = Fake::new(
            r#"echo "  ✓ Logged in to github.com account octocat (keyring)"; echo "  - Active account: false"; echo "  - Token scopes: 'repo'"
echo "  ✓ Logged in to github.com account hubot (keyring)"; echo "  - Active account: true"; echo "  - Token scopes: 'gist'""#,
        );
        let err = fake.login(ForgeKind::GitHub, "github.com").unwrap_err();
        assert_eq!(err.user(), Some("hubot"));
    }

    #[test]
    fn a_program_the_cli_left_holding_its_pipe_does_not_hold_the_caller() {
        let fake = Fake::new("sleep 20 &\necho kept; echo 'also kept' >&2");
        for (limit, bound) in [(5000, 2500), (100, 400)] {
            let started = Instant::now();
            let deadline = started + Duration::from_millis(limit);
            let out = run_capture(fake.path().as_os_str(), &["auth"], deadline)
                .unwrap_or_else(|_| panic!("it should finish"));
            assert!(
                started.elapsed() < Duration::from_millis(bound),
                "{:?}",
                started.elapsed()
            );
            assert!(out.success);
            if limit > 1000 {
                assert!(
                    out.text.contains("kept") && out.text.contains("also kept"),
                    "{}",
                    out.text
                );
            }
        }
    }

    #[test]
    fn gitlab_signed_out_and_masked_token() {
        let fake = Fake::new(r#"echo "x No token found for gitlab.com" >&2; exit 1"#);
        assert!(matches!(
            fake.login(ForgeKind::GitLab, "gitlab.com"),
            Err(CliError::SignedOut { .. })
        ));
        drop(fake);
        let fake = Fake::new(
            r#"echo "  ✓ Logged in to gitlab.com as tanuki" >&2; echo "  ✓ Token found: **********" >&2"#,
        );
        assert!(matches!(
            fake.login(ForgeKind::GitLab, "gitlab.com"),
            Err(CliError::Failed { .. })
        ));
    }

    #[test]
    fn gitlab_without_the_api_scope() {
        let fake = Fake::new(&format!(
            r#"echo "  ✓ Logged in to gitlab.com as tanuki" >&2; echo "  - Token scopes: read_api, read_repository" >&2; echo "  ✓ Token found: {TOKEN}" >&2"#
        ));
        let err = fake.login(ForgeKind::GitLab, "gitlab.com").unwrap_err();
        assert!(
            matches!(&err, CliError::MissingScope { scope: "api", .. }),
            "{err:?}"
        );
        assert!(!format!("{err} {err:?}").contains("SECRET"));
    }

    #[test]
    fn a_token_that_is_not_one_word_is_refused() {
        let fake = Fake::new(
            r#"case "$2" in
status) echo "  ✓ Logged in to github.com account octocat (keyring)";;
token) printf 'abc\ndef';;
esac"#,
        );
        assert!(matches!(
            fake.login(ForgeKind::GitHub, "github.com"),
            Err(CliError::Failed { .. })
        ));
    }

    #[test]
    fn a_host_that_looks_like_an_option_never_reaches_the_cli() {
        let fake = Fake::new("exit 0");
        for host in [
            "",
            "--token",
            "-h",
            "a b",
            "a\nb",
            "github.com --hostname evil",
            "$(id)",
            "a;b",
            "a`b`",
            "a|b",
            ".hidden",
            "host:",
            "host:12ab",
            "host:123456",
            "host:80:80",
        ] {
            assert!(
                matches!(
                    fake.login(ForgeKind::GitHub, host),
                    Err(CliError::Failed { .. })
                ),
                "{host:?}"
            );
        }
        assert_eq!(fake.calls(), Vec::<String>::new());
    }

    /// A token the CLI prints in a line that names it never reaches the message, even one without a
    /// known prefix and one on a line that belongs to no account.
    #[test]
    fn a_token_without_a_known_shape_is_hidden_from_a_failure() {
        const BARE: &str = "0123456789abcdef0123456789abcdef";
        let fake = Fake::new(&format!(
            r#"echo "  ✗ gitlab.com: API call failed: 401 Unauthorized" >&2
echo "  ✓ Token found in configuration file (plaintext): {BARE}" >&2
exit 1"#
        ));
        let err = fake.login(ForgeKind::GitLab, "gitlab.com").unwrap_err();
        let shown = format!("{err} {err:?}");
        assert!(shown.contains("API call failed: 401"), "{shown}");
        assert!(!shown.contains(BARE), "{shown}");
    }

    #[test]
    fn the_token_is_never_logged() {
        use std::sync::{Arc, Mutex as StdMutex};

        #[derive(Clone, Default)]
        struct Sink(Arc<StdMutex<Vec<u8>>>);
        impl std::io::Write for Sink {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Sink {
            type Writer = Self;
            fn make_writer(&'a self) -> Self {
                self.clone()
            }
        }

        let sink = Sink::default();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(sink.clone())
            .finish();
        let fake = Fake::new(&gh_signed_in("'repo'"));
        let outcome = tracing::subscriber::with_default(subscriber, || {
            fake.login(ForgeKind::GitHub, "github.com")
        });
        assert!(outcome.is_ok(), "{outcome:?}");
        let logged = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
        assert!(
            logged.contains("github.com"),
            "something is logged: {logged:?}"
        );
        assert!(
            !logged.contains("SECRET") && !logged.contains(TOKEN),
            "{logged}"
        );
    }
}

/// What needs no program to run, so it is tested on every platform (the fake CLIs above are shell
/// scripts).
#[cfg(test)]
mod platform_tests {
    use super::*;

    const GITLAB_CORP: &str =
        "  ✓ Logged in to gitlab.corp as tanuki (/c.yml)\n  ✓ Token found: glpat-a";

    #[test]
    fn what_fixes_each_problem() {
        let (gh, glab) = (ForgeKind::GitHub, ForgeKind::GitLab);
        let missing = |kind| CliError::MissingScope {
            kind,
            host: "h.example".into(),
            user: None,
            scope: required_scope(kind),
        };
        assert_eq!(
            missing(gh).fix_command().as_deref(),
            Some("gh auth refresh --hostname h.example --scopes repo")
        );
        assert_eq!(
            missing(glab).fix_command().as_deref(),
            Some("glab auth login --hostname h.example")
        );
        assert_eq!(missing(gh).fix_note(), None);
        let note = missing(glab).fix_note().unwrap();
        assert_eq!(note, "allow the api scope");
        assert!(
            missing(glab).to_string().contains(&note),
            "the message says the same"
        );
        assert_eq!(CliError::NotInstalled { kind: gh }.fix_command(), None);
        assert_eq!(
            CliError::Failed {
                kind: gh,
                message: "x".into()
            }
            .fix_command(),
            None
        );
        assert_eq!(install_page(gh), "https://cli.github.com");
        assert!(install_page(glab).starts_with("https://gitlab.com/"));
    }

    #[test]
    fn a_host_with_another_port_is_not_signed_in() {
        assert!(parse_status(GITLAB_CORP, "gitlab.corp").signed_in);
        assert!(!parse_status(GITLAB_CORP, "gitlab.corp:8443").signed_in);
        let with_port = "  ✓ Logged in to gitlab.corp:8443 as tanuki (/c.yml)";
        assert!(!parse_status(with_port, "gitlab.corp").signed_in);
        assert!(parse_status(with_port, "GitLab.Corp:8443").signed_in);
        // A longer name that merely contains the host is another host.
        let longer = "  ✓ Logged in to my-gitlab.corp.example as tanuki (/c.yml)";
        assert!(!parse_status(longer, "gitlab.corp").signed_in);
    }

    #[test]
    fn scopes_and_token_of_another_host_are_not_used() {
        let text = "\
github.com
  ✓ Logged in to github.com account octocat (keyring)
  - Token scopes: 'repo'
  ✓ Token found: ghp_other
ghe.corp
  ✓ Logged in to ghe.corp account octocat (keyring)
  - Token scopes: 'gist'
";
        let report = parse_status(text, "ghe.corp");
        assert_eq!(report.scopes, Some(vec!["gist".to_owned()]));
        assert_eq!(report.token, None);
        let report = parse_status(text, "github.com");
        assert_eq!(report.scopes, Some(vec!["repo".to_owned()]));
        assert_eq!(report.token.as_deref(), Some("ghp_other"));
        // Lines before any account, and those of an account that failed, belong to nobody.
        let text = "  - Token scopes: 'repo'\n  X Failed to log in to github.com account old (keyring)\n  - Token scopes: 'repo'\n  ✓ Logged in to github.com account octocat (keyring)\n";
        assert_eq!(parse_status(text, "github.com").scopes, None);
    }

    #[test]
    fn host_names_with_ports_and_underscores_are_accepted() {
        for host in [
            "github.com",
            "ghe.corp:8443",
            "git_lab.corp-1.example",
            "localhost",
        ] {
            assert!(is_host_name(host), "{host}");
        }
    }

    #[test]
    fn credential_variables_do_not_reach_the_cli() {
        let cmd = command(OsStr::new("gh"));
        let removed: Vec<_> = cmd
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(name, _)| name.to_string_lossy().into_owned())
            .collect();
        for name in CREDENTIAL_ENV {
            assert!(removed.iter().any(|r| r == name), "{name} is passed on");
        }
        // The host and CI variables `gh` and `glab` read besides their token variables.
        for name in [
            "GL_HOST",
            "GITLAB_URI",
            "GITLAB_API_HOST",
            "GH_HOST",
            "GLAB_ENABLE_CI_AUTOLOGIN",
            "CI_JOB_TOKEN",
        ] {
            assert!(removed.iter().any(|r| r == name), "{name} is passed on");
        }
    }

    #[test]
    fn a_masked_token_is_never_usable() {
        for shown in ["glpat-****", "glpat-****abcd", "****", "**********"] {
            assert!(!usable(shown), "{shown}");
        }
        assert!(usable("glpat-abcdefghijklmnop1234"));
    }

    #[test]
    fn a_timeout_is_named_in_its_own_unit() {
        assert_eq!(took_longer(CLI_TIMEOUT), "it took longer than 10 seconds");
        assert_eq!(
            took_longer(Duration::from_millis(300)),
            "it took longer than 300 ms"
        );
    }
}
