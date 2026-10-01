//! Running the `git` executable.
//!
//! Every mutation goes through here rather than through libgit2, because only `git` itself
//! honours everything a user's setup relies on: credential helpers and SSH agents, hooks
//! (`pre-commit`, `commit-msg`, `post-checkout`), commit and tag signing, clean/smudge filters such
//! as Git LFS, sparse checkouts and `includeIf` configuration. Fork and Sourcetree make the same
//! choice.
//!
//! Commands never wait for a terminal: `GIT_TERMINAL_PROMPT=0` makes a missing credential an error
//! instead of a hang, and `GIT_EDITOR=true` accepts default messages for `--continue`. When the
//! application registers an [`Askpass`] program with [`set_askpass`], git and OpenSSH ask it for
//! credentials instead; git consults `GIT_ASKPASS` before it checks `GIT_TERMINAL_PROMPT`.

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{OnceLock, PoisonError, RwLock};
use std::time::Instant;

use crate::{Error, Result};

/// A credential prompt program that git and OpenSSH run instead of asking on a terminal.
///
/// Git runs it as `program <prompt>` and reads the answer from its standard output
/// (`GIT_ASKPASS`); OpenSSH does the same for passphrases and host-key questions
/// (`SSH_ASKPASS` with `SSH_ASKPASS_REQUIRE=force`, OpenSSH 8.4 or later).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Askpass {
    /// The executable to run.
    pub program: PathBuf,
    /// Extra environment for it (for example how to reach the application), set on every git
    /// command.
    pub env: Vec<(String, String)>,
}

fn askpass_slot() -> &'static RwLock<Option<Askpass>> {
    static SLOT: OnceLock<RwLock<Option<Askpass>>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(None))
}

/// Registers the credential prompt used by every later [`command`], process-wide; `None` turns
/// it off. A `GIT_ASKPASS`, `SSH_ASKPASS` or `SSH_ASKPASS_REQUIRE` the user already set in the
/// environment wins over it; the extra `env` pairs are always set.
pub fn set_askpass(askpass: Option<Askpass>) {
    let mut slot = askpass_slot()
        .write()
        .unwrap_or_else(PoisonError::into_inner);
    *slot = askpass;
}

/// Builds a `git` command for `workdir` with a non-interactive environment.
pub fn command(workdir: Option<&Path>) -> Command {
    let mut cmd = Command::new("git");
    if let Some(dir) = workdir {
        cmd.current_dir(dir);
    }
    apply_env(&mut cmd);
    cmd.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Sets the non-interactive environment, and the registered [`Askpass`] if any, on `cmd`.
pub(crate) fn apply_env(cmd: &mut Command) {
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("GIT_MERGE_AUTOEDIT", "no");
    let slot = askpass_slot()
        .read()
        .unwrap_or_else(PoisonError::into_inner);
    let Some(askpass) = slot.as_ref() else {
        return;
    };
    if !is_set(cmd, "GIT_ASKPASS") {
        cmd.env("GIT_ASKPASS", &askpass.program);
    }
    if !is_set(cmd, "SSH_ASKPASS") {
        cmd.env("SSH_ASKPASS", &askpass.program);
        if !is_set(cmd, "SSH_ASKPASS_REQUIRE") {
            cmd.env("SSH_ASKPASS_REQUIRE", "force");
        }
    }
    for (key, value) in &askpass.env {
        cmd.env(key, value);
    }
}

/// Whether `key` reaches the child: set on `cmd` itself, or inherited and not removed from it.
fn is_set(cmd: &Command, key: &str) -> bool {
    // Windows environment names are case-insensitive.
    let same = |k: &OsStr| {
        if cfg!(windows) {
            k.eq_ignore_ascii_case(key)
        } else {
            k == OsStr::new(key)
        }
    };
    match cmd.get_envs().find(|(k, _)| same(k)) {
        Some((_, value)) => value.is_some(),
        None => std::env::var_os(key).is_some(),
    }
}

fn subcommand(args: &[&str]) -> String {
    args.iter()
        .find(|a| !a.starts_with('-'))
        .copied()
        .unwrap_or("git")
        .to_owned()
}

/// The error for a failed `git args…`: git's standard error, or standard output when standard
/// error is empty.
pub(crate) fn failure(args: &[&str], stdout: &[u8], stderr: &[u8]) -> Error {
    let mut msg = String::from_utf8_lossy(stderr).trim().to_owned();
    if msg.is_empty() {
        msg = String::from_utf8_lossy(stdout).trim().to_owned();
    }
    Error::Cli {
        command: subcommand(args),
        stderr: msg,
    }
}

/// `args` as they may appear in a log: credentials in URLs, commit and tag messages, and the
/// values of `-c key=value` overrides are replaced, since any of them can hold a secret or
/// personal text.
pub fn loggable(args: &[&str]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut hide_next = false;
    for arg in args {
        if hide_next {
            out.push("<redacted>".to_owned());
            hide_next = false;
            continue;
        }
        let a = *arg;
        if matches!(a, "-m" | "--message" | "-c" | "-F" | "--file") {
            hide_next = true;
            out.push(a.to_owned());
        } else if let Some((flag, _)) = a
            .split_once('=')
            .filter(|(f, _)| matches!(*f, "--message" | "-m" | "--file"))
        {
            out.push(format!("{flag}=<redacted>"));
        } else if a.starts_with("-m") && a.len() > 2 && !a.starts_with("--") {
            out.push("-m<redacted>".to_owned());
        } else {
            out.push(redact_url(a));
        }
    }
    out
}

/// Removes the `user:password@` part of a URL inside `text`.
pub fn redact_url(text: &str) -> String {
    let Some(scheme_end) = text.find("://") else {
        return text.to_owned();
    };
    let rest = &text[scheme_end + 3..];
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    match rest[..authority_end].rfind('@') {
        Some(at) => format!("{}://<redacted>@{}", &text[..scheme_end], &rest[at + 1..]),
        None => text.to_owned(),
    }
}

/// Logs a finished git command: at debug level always, at warn level when it failed.
fn record(args: &[&str], started: Instant, status: Option<i32>, stderr: &[u8]) {
    let ms = started.elapsed().as_millis() as u64;
    let command = subcommand(args);
    let shown = loggable(args);
    if status == Some(0) {
        tracing::debug!(%command, args = ?shown, ms, "git finished");
    } else {
        let text = String::from_utf8_lossy(stderr);
        let tail: String = text
            .trim()
            .chars()
            .rev()
            .take(2000)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        tracing::warn!(%command, args = ?shown, exit = ?status, ms, stderr = %redact_url(&tail), "git failed");
    }
}

/// Runs `git args…` in `workdir` and returns its standard output.
pub fn run(workdir: &Path, args: &[&str]) -> Result<String> {
    let started = Instant::now();
    let out = command(Some(workdir))
        .args(args)
        .output()
        .map_err(Error::GitMissing)?;
    record(args, started, out.status.code(), &out.stderr);
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(failure(args, &out.stdout, &out.stderr))
    }
}

/// The result of a command whose exit code carries an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// The exit code; `None` when a signal ended the process.
    pub code: Option<i32>,
    /// Standard output, as bytes.
    pub stdout: Vec<u8>,
    /// Standard error, as bytes.
    pub stderr: Vec<u8>,
}

impl Output {
    /// The error [`run`] would have returned for this output.
    pub(crate) fn error(&self, args: &[&str]) -> Error {
        failure(args, &self.stdout, &self.stderr)
    }
}

/// Like [`run`], but a non-zero exit is not an error: for commands such as `merge-tree` and
/// `diff --quiet` that answer through their exit code. Only failing to start git is an error.
pub fn run_unchecked(workdir: &Path, args: &[&str]) -> Result<Output> {
    let started = Instant::now();
    let out = command(Some(workdir))
        .args(args)
        .output()
        .map_err(Error::GitMissing)?;
    let code = out.status.code();
    // Exit code 1 is an answer for these commands, not a failure worth a warning.
    record(
        args,
        started,
        code.map(|c| if c == 1 { 0 } else { c }),
        &out.stderr,
    );
    Ok(Output {
        code,
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

/// Like [`run`], feeding `input` to standard input.
pub fn run_with_input(workdir: &Path, args: &[&str], input: &[u8]) -> Result<String> {
    let started = Instant::now();
    let mut child = command(Some(workdir))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(Error::GitMissing)?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input)?;
    }
    let out = child.wait_with_output()?;
    record(args, started, out.status.code(), &out.stderr);
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(failure(args, &out.stdout, &out.stderr))
    }
}

/// Runs `git args…` in `workdir`, feeding `input` to standard input, and returns the exit code and
/// standard output for commands whose exit status is part of the answer (`git check-ignore`
/// exits 1 for "not ignored", `git config --get` for "not set"). Exit codes of `ok_codes` count
/// as answers; any other code is an error carrying git's standard error.
pub(crate) fn run_answer(
    workdir: &Path,
    args: &[&str],
    input: &[u8],
    ok_codes: &[i32],
) -> Result<(i32, Vec<u8>)> {
    let started = Instant::now();
    let mut child = command(Some(workdir))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(Error::GitMissing)?;
    if let Some(mut stdin) = child.stdin.take() {
        // A git that exits early closes its end; its exit status and stderr say why.
        match stdin.write_all(input) {
            Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.into());
            }
            _ => {}
        }
    }
    let out = child.wait_with_output()?;
    let code = out.status.code();
    match code {
        Some(c) if ok_codes.contains(&c) => {
            record(args, started, Some(0), &out.stderr);
            Ok((c, out.stdout))
        }
        _ => {
            record(args, started, code, &out.stderr);
            Err(failure(args, &out.stdout, &out.stderr))
        }
    }
}

/// Runs a long command (clone, fetch, pull, push) and reports each progress line from standard
/// error to `progress` as it arrives. Returns standard error's last lines on success, which is
/// where git puts its summary.
pub fn run_streaming(
    workdir: Option<&Path>,
    args: &[&str],
    mut progress: impl FnMut(&str),
) -> Result<String> {
    let started = Instant::now();
    tracing::info!(command = %subcommand(args), args = ?loggable(args), "git started");
    let mut child = command(workdir)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(Error::GitMissing)?;

    let mut stdout = child.stdout.take();
    let stdout_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(out) = stdout.as_mut() {
            let _ = out.read_to_end(&mut buf);
        }
        buf
    });

    let mut stderr_all = Vec::new();
    if let Some(mut err) = child.stderr.take() {
        let mut line = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = match err.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            };
            for &b in &chunk[..n] {
                if b == b'\r' || b == b'\n' {
                    if !line.is_empty() {
                        progress(String::from_utf8_lossy(&line).trim());
                        line.clear();
                    }
                    if b == b'\n' {
                        stderr_all.push(b'\n');
                    }
                } else {
                    line.push(b);
                    stderr_all.push(b);
                }
            }
        }
        if !line.is_empty() {
            progress(String::from_utf8_lossy(&line).trim());
        }
    }

    let status = child.wait()?;
    record(args, started, status.code(), &stderr_all);
    if status.success() {
        tracing::info!(command = %subcommand(args), ms = started.elapsed().as_millis() as u64, "git succeeded");
    }
    let stdout = stdout_reader.join().unwrap_or_default();
    if status.success() {
        let text = String::from_utf8_lossy(&stderr_all);
        let tail: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        let start = tail.len().saturating_sub(3);
        Ok(tail[start..].join("\n"))
    } else {
        Err(failure(args, &stdout, &stderr_all))
    }
}

/// The installed git version, e.g. `git version 2.43.0`.
pub fn version() -> Result<String> {
    let out = command(None)
        .arg("--version")
        .output()
        .map_err(Error::GitMissing)?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// The installed git's major and minor version, e.g. `(2, 43)`; `None` when git cannot run or
/// prints something unexpected.
pub fn version_number() -> Option<(u32, u32)> {
    static VERSION: OnceLock<Option<(u32, u32)>> = OnceLock::new();
    *VERSION.get_or_init(|| parse_version(&version().ok()?))
}

/// Parses `git version 2.43.0` (or `git version 2.39.3 (Apple Git-146)`, or
/// `git version 2.45.1.windows.1`) into `(2, 43)`.
fn parse_version(text: &str) -> Option<(u32, u32)> {
    let number = text
        .strip_prefix("git version ")?
        .split_whitespace()
        .next()?;
    let mut parts = number.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

/// Whether git has `merge-tree --write-tree` (Git 2.38 or later).
pub(crate) fn has_merge_tree_write_tree() -> bool {
    version_number().is_some_and(|v| v >= (2, 38))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logged_arguments_hide_secrets_and_messages() {
        assert_eq!(
            loggable(&[
                "clone",
                "--progress",
                "https://ada:ghp_x@github.com/o/r.git",
                "dest"
            ]),
            [
                "clone",
                "--progress",
                "https://<redacted>@github.com/o/r.git",
                "dest"
            ]
        );
        assert_eq!(
            loggable(&["commit", "-m", "secret plans", "--message=more"]),
            ["commit", "-m", "<redacted>", "--message=<redacted>"]
        );
        assert_eq!(
            loggable(&["-c", "http.extraheader=AUTHORIZATION: x", "fetch"]),
            ["-c", "<redacted>", "fetch"]
        );
        assert_eq!(loggable(&["tag", "-mhello"]), ["tag", "-m<redacted>"]);
        assert_eq!(redact_url("ssh://git@host/x"), "ssh://<redacted>@host/x");
        assert_eq!(redact_url("https://host/a@b"), "https://host/a@b");
        assert_eq!(redact_url("no url"), "no url");
    }

    #[test]
    fn parses_git_versions() {
        assert_eq!(parse_version("git version 2.43.0"), Some((2, 43)));
        assert_eq!(
            parse_version("git version 2.39.3 (Apple Git-146)"),
            Some((2, 39))
        );
        assert_eq!(parse_version("git version 2.45.1.windows.1"), Some((2, 45)));
        assert_eq!(parse_version("git found"), None);
        assert!(version_number().is_some());
    }

    #[test]
    fn unchecked_runs_return_the_exit_code() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = run_unchecked(dir.path(), &["rev-parse", "--git-dir"]).expect("git runs");
        assert_ne!(out.code, Some(0));
        assert!(!out.stderr.is_empty());
        let out = run_unchecked(dir.path(), &["--version"]).expect("git runs");
        assert_eq!(out.code, Some(0));
    }

    #[test]
    fn reports_stderr_on_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = run(dir.path(), &["log"]).expect_err("not a repository");
        match err {
            Error::Cli { command, stderr } => {
                assert_eq!(command, "log");
                assert!(
                    stderr.to_lowercase().contains("not a git repository"),
                    "{stderr}"
                );
            }
            other => panic!("unexpected error {other:?}"),
        }
    }

    #[test]
    fn streams_progress_lines() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut lines = Vec::new();
        run_streaming(Some(dir.path()), &["init", "-q", "streamed"], |l| {
            lines.push(l.to_owned())
        })
        .expect("init");
        assert!(dir.path().join("streamed/.git").is_dir());
    }

    /// Serialises the tests that change the process-wide hook and turns it off afterwards, even
    /// when the test panics.
    struct HookGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl HookGuard {
        fn set(askpass: Option<Askpass>) -> Self {
            static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let lock = LOCK.lock().unwrap_or_else(PoisonError::into_inner);
            set_askpass(askpass);
            Self { _lock: lock }
        }
    }

    impl Drop for HookGuard {
        fn drop(&mut self) {
            set_askpass(None);
        }
    }

    fn askpass(program: &str) -> Askpass {
        Askpass {
            program: PathBuf::from(program),
            env: vec![("CANOPY_ASKPASS_TOKEN".into(), "42".into())],
        }
    }

    /// A command as the user's environment might hand it to us: `inherited` pairs set, the
    /// askpass variables otherwise absent whatever the test runner's environment holds.
    fn user_command(inherited: &[(&str, &str)]) -> Command {
        let mut cmd = Command::new("git");
        for key in ["GIT_ASKPASS", "SSH_ASKPASS", "SSH_ASKPASS_REQUIRE"] {
            cmd.env_remove(key);
        }
        for (k, v) in inherited {
            cmd.env(k, v);
        }
        apply_env(&mut cmd);
        cmd
    }

    fn env_of(cmd: &Command, key: &str) -> Option<String> {
        cmd.get_envs()
            .find(|(k, _)| *k == OsStr::new(key))
            .and_then(|(_, v)| v)
            .map(|v| v.to_string_lossy().into_owned())
    }

    #[test]
    fn askpass_sets_git_and_ssh_variables() {
        let _hook = HookGuard::set(Some(askpass("/opt/canopy")));
        let cmd = user_command(&[]);
        assert_eq!(env_of(&cmd, "GIT_ASKPASS").as_deref(), Some("/opt/canopy"));
        assert_eq!(env_of(&cmd, "SSH_ASKPASS").as_deref(), Some("/opt/canopy"));
        assert_eq!(
            env_of(&cmd, "SSH_ASKPASS_REQUIRE").as_deref(),
            Some("force")
        );
        assert_eq!(env_of(&cmd, "CANOPY_ASKPASS_TOKEN").as_deref(), Some("42"));
        assert_eq!(env_of(&cmd, "GIT_TERMINAL_PROMPT").as_deref(), Some("0"));
    }

    #[test]
    fn askpass_respects_the_users_own_programs() {
        let _hook = HookGuard::set(Some(askpass("/opt/canopy")));
        let cmd = user_command(&[("GIT_ASKPASS", "/usr/bin/mine")]);
        assert_eq!(
            env_of(&cmd, "GIT_ASKPASS").as_deref(),
            Some("/usr/bin/mine")
        );
        assert_eq!(env_of(&cmd, "SSH_ASKPASS").as_deref(), Some("/opt/canopy"));

        let cmd = user_command(&[
            ("SSH_ASKPASS", "/usr/bin/ksshaskpass"),
            ("SSH_ASKPASS_REQUIRE", "prefer"),
        ]);
        assert_eq!(env_of(&cmd, "GIT_ASKPASS").as_deref(), Some("/opt/canopy"));
        assert_eq!(
            env_of(&cmd, "SSH_ASKPASS").as_deref(),
            Some("/usr/bin/ksshaskpass")
        );
        assert_eq!(
            env_of(&cmd, "SSH_ASKPASS_REQUIRE").as_deref(),
            Some("prefer")
        );
        assert_eq!(env_of(&cmd, "CANOPY_ASKPASS_TOKEN").as_deref(), Some("42"));
    }

    #[test]
    fn no_askpass_leaves_the_environment_alone() {
        let _hook = HookGuard::set(None);
        let cmd = user_command(&[]);
        for key in ["GIT_ASKPASS", "SSH_ASKPASS", "SSH_ASKPASS_REQUIRE"] {
            assert_eq!(env_of(&cmd, key), None, "{key}");
        }
        assert_eq!(env_of(&cmd, "CANOPY_ASKPASS_TOKEN"), None);
    }

    #[test]
    fn askpass_survives_a_poisoned_lock() {
        let _hook = HookGuard::set(None);
        let _ = std::thread::spawn(|| {
            let _held = askpass_slot().write();
            panic!("poison the askpass lock");
        })
        .join();
        assert!(askpass_slot().is_poisoned());
        set_askpass(Some(askpass("/opt/canopy")));
        let cmd = user_command(&[]);
        assert_eq!(env_of(&cmd, "GIT_ASKPASS").as_deref(), Some("/opt/canopy"));
        askpass_slot().clear_poison();
    }

    /// Git asks the registered program for a username and password although terminal prompts are
    /// off, and the program sees the extra environment.
    #[cfg(unix)]
    #[test]
    fn git_reads_credentials_from_the_askpass_program() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let script = dir.path().join("askpass.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\ncase \"$1\" in\n  Username*) echo ada ;;\n  *) echo \"secret-$CANOPY_ASKPASS_TOKEN\" ;;\nesac\n",
        )
        .expect("write script");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let _hook = HookGuard::set(Some(Askpass {
            program: script,
            env: vec![("CANOPY_ASKPASS_TOKEN".into(), "42".into())],
        }));

        let mut cmd = user_command(&[]);
        let mut child = cmd
            .current_dir(dir.path())
            .args(["credential", "fill"])
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", dir.path().join("no-global"))
            .env("HOME", dir.path())
            .env("GIT_CEILING_DIRECTORIES", dir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn git");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(b"protocol=https\nhost=example.com\n\n")
            .expect("write");
        let out = child.wait_with_output().expect("git credential fill");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.lines().any(|l| l == "username=ada"), "{stdout}");
        assert!(
            stdout.lines().any(|l| l == "password=secret-42"),
            "{stdout}"
        );
    }
}
