//! The sign-in of the GitHub CLI (`gh`) or the GitLab CLI (`glab`): which of them is installed,
//! whether it is signed in to a host, and the token it holds for that host.
//!
//! The CLI is a local program, not a service: Canopy runs it without a shell, with a fixed
//! argument list and the host as one argument, a short deadline and no prompt, and reads what it
//! prints. The token lives in the returned [`CliLogin`] only, which hides it from `Debug`; it is
//! never logged, written to a file or put in a URL. The CLI is started without the token
//! environment variables, so it reports only its own stored sign-in and never one that Canopy's
//! per-host rules for those variables ([`crate::token_from_env_for`]) would have refused.

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

/// The variables that hand a CLI a token (or the host to use it for) instead of its own stored
/// sign-in.
const CREDENTIAL_ENV: [&str; 9] = [
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GH_ENTERPRISE_TOKEN",
    "GITHUB_ENTERPRISE_TOKEN",
    "GITLAB_TOKEN",
    "GL_TOKEN",
    "GITLAB_ACCESS_TOKEN",
    "OAUTH_TOKEN",
    "GITLAB_HOST",
];

/// What the CLI holds for one host. Its token stays in memory.
#[derive(Clone, PartialEq, Eq)]
pub struct CliLogin {
    token: String,
    /// The account the CLI is signed in as, when it said.
    pub user: Option<String>,
}

impl CliLogin {
    /// The token, to be handed to the HTTP client and nowhere else.
    pub fn into_token(self) -> String {
        self.token
    }
}

impl std::fmt::Debug for CliLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CliLogin")
            .field("token", &"<redacted>")
            .field("user", &self.user)
            .finish()
    }
}

/// Why the CLI gave no token. The text is meant for the user.
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
    #[error("The {} did not give a token: {message}", cli_name(*.kind))]
    Failed { kind: ForgeKind, message: String },
}

impl CliError {
    /// The account the CLI is signed in as, when the error knows it.
    pub fn user(&self) -> Option<&str> {
        match self {
            Self::MissingScope { user, .. } => user.as_deref(),
            _ => None,
        }
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
    match kind {
        ForgeKind::GitHub => {
            format!("Run `gh auth refresh --hostname {host} --scopes repo` to add it.")
        }
        ForgeKind::GitLab => format!(
            "Run `glab auth login --hostname {host}` with a token that has the `api` scope."
        ),
    }
}

/// The token the CLI holds for `host` (`name[:port]`), with the account it belongs to.
///
/// Blocking: runs `gh auth status` and `gh auth token`, or `glab auth status --show-token`, for
/// at most ten seconds in all, so call it from a background thread.
pub fn login_from_cli(kind: ForgeKind, host: &str) -> Result<CliLogin, CliError> {
    login_with(kind, host, OsStr::new(program(kind)))
}

/// [`login_from_cli`] with the program to run, so tests can use a fake one.
fn login_with(kind: ForgeKind, host: &str, program: &OsStr) -> Result<CliLogin, CliError> {
    let result = read_login(kind, host, program);
    // Neither arm names the token: a login's `Debug` hides it and an error's words are `shorten`ed.
    match &result {
        Ok(_) => tracing::debug!(host, cli = self::program(kind), "using the CLI's sign-in"),
        Err(CliError::NotInstalled { .. }) => {
            tracing::debug!(cli = self::program(kind), "the CLI is not installed");
        }
        Err(e) => {
            tracing::info!(host, cli = self::program(kind), problem = %e, "the CLI gave no token");
        }
    }
    result
}

fn read_login(kind: ForgeKind, host: &str, program: &OsStr) -> Result<CliLogin, CliError> {
    let host = host.trim();
    let failed = |message: &str| CliError::Failed {
        kind,
        message: shorten(message),
    };
    // A host that could be taken for an option, or hold a second argument, never reaches a CLI.
    if host.is_empty()
        || host.starts_with('-')
        || host.contains(|c: char| c.is_whitespace() || c.is_control())
    {
        return Err(failed("the host name is not valid"));
    }
    let deadline = Instant::now() + CLI_TIMEOUT;
    let run = |args: &[&str]| match run_capture(program, args, deadline) {
        Ok(out) => Ok(out),
        Err(RunError::NotFound) => Err(CliError::NotInstalled { kind }),
        Err(RunError::TimedOut) => Err(failed("it took longer than 10 seconds")),
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
            let hint = hide(&status.text, report.token.as_deref());
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
            if out.success {
                Some(out.text.trim().to_owned())
            } else {
                return Err(failed(&hide(&out.text, None)));
            }
        }
        ForgeKind::GitLab => report.token,
    };
    match token.filter(|t| usable(t)) {
        Some(token) => Ok(CliLogin {
            token,
            user: report.user,
        }),
        None => Err(failed("it printed no usable token")),
    }
}

/// What a CLI says when it is not signed in to a host.
const SIGNED_OUT_WORDS: [&str; 6] = [
    "not logged",
    "no accounts",
    "no token",
    "no oauth token",
    "not authenticated",
    "unknown host",
];

/// CLI output made safe to show: no token the CLI printed, no known token shapes.
fn hide(text: &str, token: Option<&str>) -> String {
    let text = match token {
        Some(t) if !t.is_empty() => text.replace(t, "<redacted>"),
        _ => text.to_owned(),
    };
    shorten(&text)
}

/// A token is one visible word, no longer than any real one.
fn usable(token: &str) -> bool {
    !token.is_empty() && token.len() <= MAX_TOKEN_LEN && token.chars().all(|c| c.is_ascii_graphic())
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
///   ✓ Token found: glpat-…
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
                "token" | "token found"
                    if !value.is_empty() && !value.chars().all(|c| c == '*') =>
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
    let mut text = out.text(until);
    text.push('\n');
    text.push_str(&err.text(until));
    Ok(Captured {
        success: status.success(),
        text,
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
            login_with(kind, host, self.path().as_os_str())
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
        assert_eq!(login.user.as_deref(), Some("octocat"));
        assert_eq!(login.clone().into_token(), TOKEN);
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
            let err = login_with(kind, "github.com", missing.as_os_str()).unwrap_err();
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

    #[test]
    fn gitlab_signed_in_on_a_self_hosted_host() {
        let fake = Fake::new(&format!(
            r#"echo "gitlab.corp:8443" >&2
echo "  ✓ Logged in to gitlab.corp:8443 as tanuki (/home/t/.config/glab-cli/config.yml)" >&2
echo "  ✓ Token found: {TOKEN}" >&2"#
        ));
        let login = fake.login(ForgeKind::GitLab, "gitlab.corp:8443").unwrap();
        assert_eq!(login.user.as_deref(), Some("tanuki"));
        assert_eq!(login.into_token(), TOKEN);
        assert_eq!(
            fake.calls(),
            ["auth status --hostname gitlab.corp:8443 --show-token"]
        );
    }

    const GITLAB_CORP: &str =
        "  ✓ Logged in to gitlab.corp as tanuki (/c.yml)\n  ✓ Token found: glpat-a";

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
        assert_eq!(login.user.as_deref(), Some("hubot"));
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
