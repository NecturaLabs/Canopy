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

use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{OnceLock, PoisonError, RwLock};
use std::time::{Duration, Instant};

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

/// Registers the credential prompt used by every later [`command`], process-wide.
///
/// `None` turns it off. A `GIT_ASKPASS`, `SSH_ASKPASS` or `SSH_ASKPASS_REQUIRE` the user already
/// set in the environment wins over it; the extra `env` pairs are always set.
pub fn set_askpass(askpass: Option<Askpass>) {
    let mut slot = askpass_slot()
        .write()
        .unwrap_or_else(PoisonError::into_inner);
    *slot = askpass;
}

/// Builds a `git` command for `workdir` with a non-interactive environment.
#[expect(
    clippy::disallowed_methods,
    reason = "the one place Canopy builds a git command"
)]
pub fn command(workdir: Option<&Path>) -> Command {
    let mut cmd = Command::new("git");
    // A repository can set core.fsmonitor to a shell command. Never execute it while
    // inspecting a repository that a user has just opened or cloned.
    cmd.args(["-c", "core.fsmonitor=false"]);
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

/// The git versions, as `(major, minor)`, that brought what Canopy asks of them.
pub(crate) const ZDIFF3_SINCE: (u32, u32) = (2, 35);
pub(crate) const MERGE_AUTOSTASH_SINCE: (u32, u32) = (2, 27);
/// `git pull` reads `pull.autoStash`; older git ignores it (git 2.51 release notes).
pub(crate) const PULL_AUTOSTASH_SINCE: (u32, u32) = (2, 51);
/// `gpg.format = ssh` (git 2.34 release notes).
pub(crate) const SSH_SIGNING_SINCE: (u32, u32) = (2, 34);
pub(crate) const INITIAL_BRANCH_SINCE: (u32, u32) = (2, 28);
pub(crate) const MERGE_TREE_WRITE_TREE_SINCE: (u32, u32) = (2, 38);
/// Git's exit status for a fatal error.
pub(crate) const EXIT_FATAL: i32 = 128;
/// `git config --unset-all` exits with this when the key was not set.
pub(crate) const CONFIG_KEY_ABSENT: i32 = 5;
/// The shortest `-m<message>` argument that still carries a message: the flag plus a letter.
const MIN_ATTACHED_MESSAGE_ARG: usize = 3;
/// The length of `://`, which ends a URL's scheme.
const SCHEME_SEPARATOR_LEN: usize = 3;
/// The most characters of a failed command's output kept in its error.
const ERROR_TAIL_CHARS: usize = 2000;
/// How often a running git is polled for its end.
const POLL_INTERVAL: Duration = Duration::from_millis(10);
/// The size of the buffer a git's stderr is read through.
const STDERR_CHUNK: usize = 4096;

/// The variables that point git at a repository or reshape its object store
/// (`git rev-parse --local-env-vars`, less the configuration ones, which belong to the user).
/// Canopy picks the repository by the working directory it gives a command, and libgit2 reads
/// none of these. One inherited from the environment (a git alias or hook that starts Canopy sets
/// `GIT_DIR`) would send a command to another repository than the one Canopy checked and shows.
const REPOSITORY_ENV: [&str; 12] = [
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_DIR",
    "GIT_GRAFT_FILE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_OBJECT_DIRECTORY",
    "GIT_PREFIX",
    "GIT_REPLACE_REF_BASE",
    "GIT_SHALLOW_FILE",
    "GIT_WORK_TREE",
];

/// Sets the non-interactive environment, and the registered [`Askpass`] if any, on `cmd`, and
/// keeps the repository variables of Canopy's own environment out of it.
#[expect(
    clippy::significant_drop_tightening,
    reason = "the askpass settings are read under one lock, so they come from the same app"
)]
pub(crate) fn apply_env(cmd: &mut Command) {
    for key in REPOSITORY_ENV {
        cmd.env_remove(key);
    }
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        // `sequence.editor` in a repository's configuration beats `GIT_EDITOR`, and names a
        // program: only this variable outranks it.
        .env("GIT_SEQUENCE_EDITOR", "true")
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

/// Adds `args` to `cmd`. `git submodule` is a shell script in Git for Windows, started in its MSYS
/// shell, whose runtime expands wildcards in the arguments it starts with: the submodule
/// `deps/lib[1]` would name an existing `deps/lib1`. For that command alone, wildcards are left
/// unexpanded; every other git process keeps the user's own MSYS options, since `noglob` also
/// changes how the shell reads quotes in the commands a user's configuration runs.
fn with_args<'a>(cmd: &'a mut Command, args: &[&str]) -> &'a mut Command {
    if cfg!(windows) && subcommand(args) == "submodule" {
        cmd.env("MSYS", msys_without_glob(std::env::var_os("MSYS")));
    }
    cmd.args(args)
}

/// The MSYS runtime options `current` asks for, with argument wildcards left unexpanded: the
/// options are words, and the last word about a setting wins.
fn msys_without_glob(current: Option<OsString>) -> OsString {
    match current {
        Some(mut options) if !options.is_empty() => {
            options.push(" noglob");
            options
        }
        _ => OsString::from("noglob"),
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

/// The git sub-command in `args`: the first argument that is neither an option nor the value of
/// one of git's own `-c <name=value>` and `-C <path>` options (so `-c a=b fetch` is `fetch`).
fn subcommand(args: &[&str]) -> String {
    let mut args = args.iter().copied();
    while let Some(a) = args.next() {
        if matches!(a, "-c" | "-C") {
            args.next();
        } else if !a.starts_with('-') {
            return a.to_owned();
        }
    }
    "git".to_owned()
}

/// The error for a failed `git args…`: git's standard error, or standard output when standard
/// error is empty.
pub(crate) fn failure(args: &[&str], stdout: &[u8], stderr: &[u8]) -> Error {
    let mut msg = String::from_utf8_lossy(stderr).trim().to_owned();
    if msg.is_empty() {
        String::from_utf8_lossy(stdout).trim().clone_into(&mut msg);
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
    let config = subcommand(args) == "config";
    for arg in args {
        if hide_next {
            out.push("<redacted>".to_owned());
            hide_next = false;
            continue;
        }
        let a = *arg;
        if matches!(a, "-m" | "--message" | "-c" | "-F" | "--file")
            || (config
                && matches!(
                    a,
                    "user.name"
                        | "user.email"
                        | "author.name"
                        | "author.email"
                        | "committer.name"
                        | "committer.email"
                ))
        {
            hide_next = true;
            out.push(a.to_owned());
        } else if let Some((flag, _)) = a
            .split_once('=')
            .filter(|(f, _)| matches!(*f, "--message" | "-m" | "--file"))
        {
            out.push(format!("{flag}=<redacted>"));
        } else if a.starts_with("-m") && a.len() >= MIN_ATTACHED_MESSAGE_ARG && !a.starts_with("--")
        {
            out.push("-m<redacted>".to_owned());
        } else if let Some(flag) = ["-S", "-G"]
            .into_iter()
            .find(|f| a.starts_with(f) && a.len() > f.len())
        {
            // A history search for text that may be a secret.
            out.push(format!("{flag}<redacted>"));
        } else {
            out.push(redact_url(a));
        }
    }
    out
}

/// Removes the `user:password@` part of every URL inside `text`.
#[expect(
    clippy::string_slice,
    reason = "offsets come from find and userinfo, so they fall between characters"
)]
pub fn redact_url(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("://") {
        let (head, tail) = rest.split_at(i + SCHEME_SEPARATOR_LEN);
        out.push_str(head);
        let ends_text = |c: char| c.is_whitespace() || matches!(c, '\'' | '"' | '<' | '>' | '`');
        match userinfo(tail, ends_text) {
            Some(user) => {
                out.push_str(&tail[..user.start]);
                out.push_str("<redacted>");
                rest = &tail[user.end..];
            }
            None => rest = tail,
        }
    }
    out.push_str(rest);
    out
}

/// Where the user name and password lie in `after`, the text after a URL's `://`.
///
/// They run from the end of any further slashes (git and libcurl skip them) to the last `@`
/// before the first slash or `stop` character. Git splits the user from the host at the first
/// slash only, so a `?` or `#` does not end them. `None` when there is no `@` there.
#[expect(
    clippy::string_slice,
    reason = "offsets come from find, rfind and an ASCII trim, so they fall between characters"
)]
pub fn userinfo(after: &str, stop: impl Fn(char) -> bool) -> Option<std::ops::Range<usize>> {
    let start = after.len() - after.trim_start_matches('/').len();
    let rest = &after[start..];
    let end = rest
        .find(|c: char| c == '/' || stop(c))
        .unwrap_or(rest.len());
    let at = rest[..end].rfind('@')?;
    Some(start..start + at)
}

/// Whether `git args…` finished with an answer: success, or `git config` reporting an unset key
/// (exit 1 with nothing on standard error).
fn answered(args: &[&str], status: Option<i32>, stderr: &[u8]) -> bool {
    status == Some(0)
        || (status == Some(1)
            && subcommand(args) == "config"
            && stderr.iter().all(u8::is_ascii_whitespace))
}

/// Logs a finished git command: at debug level when it answered, at warn level when it failed.
fn record(args: &[&str], started: Instant, status: Option<i32>, stderr: &[u8]) {
    let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let command = subcommand(args);
    let shown = loggable(args);
    if answered(args, status, stderr) {
        tracing::debug!(%command, args = ?shown, ms, "git finished");
    } else {
        let text = String::from_utf8_lossy(stderr);
        let tail: String = text
            .trim()
            .chars()
            .rev()
            .take(ERROR_TAIL_CHARS)
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
    let out = with_args(&mut command(Some(workdir)), args)
        .output()
        .map_err(Error::GitMissing)?;
    record(args, started, out.status.code(), &out.stderr);
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(failure(args, &out.stdout, &out.stderr))
    }
}

/// Runs `git args… path` with a native, possibly non-UTF-8 final path argument. The path is
/// converted only for the redacted log and error text, never for the process itself.
pub fn run_with_path(workdir: &Path, args: &[&str], path: &Path) -> Result<String> {
    let started = Instant::now();
    let out = with_args(&mut command(Some(workdir)), args)
        .arg(path)
        .output()
        .map_err(Error::GitMissing)?;
    let shown_path = path.to_string_lossy();
    let mut shown = args.to_vec();
    shown.push(shown_path.as_ref());
    record(&shown, started, out.status.code(), &out.stderr);
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(failure(&shown, &out.stdout, &out.stderr))
    }
}

/// Runs `cmd`, a git command built with [`command`], and collects its output, but gives up after
/// `limit`: the child is killed and the call fails. For reads that share a thread with other
/// work, where a repository whose configuration makes git block must not stall everything behind it.
pub(crate) fn output_within(cmd: Command, args: &[&str], limit: Duration) -> Result<Output> {
    output_within_input(cmd, args, limit, None)
}

/// A git child whose standard output and error are each drained by a thread of its own, so that
/// no pipe waits on another: git may answer while it still reads (`check-ignore --stdin`), and a
/// caller writing everything before reading anything would block once the answer fills the pipe.
struct Piped {
    child: Child,
    /// The pipe to feed, when one was asked for.
    stdin: Option<ChildStdin>,
    stdout: std::thread::JoinHandle<Vec<u8>>,
    stderr: std::thread::JoinHandle<Vec<u8>>,
}

/// Starts `cmd` (a git command built with [`command`]) with `args`, giving it a pipe for standard
/// input when `feed` is set and the null device otherwise.
fn spawn_piped(mut cmd: Command, args: &[&str], feed: bool) -> Result<Piped> {
    fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> std::thread::JoinHandle<Vec<u8>> {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut buf);
            }
            buf
        })
    }
    let mut child = with_args(&mut cmd, args)
        .stdin(if feed { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(Error::GitMissing)?;
    Ok(Piped {
        stdin: child.stdin.take(),
        stdout: drain(child.stdout.take()),
        stderr: drain(child.stderr.take()),
        child,
    })
}

/// [`output_within`] feeding `input` to standard input from a thread of its own, so a git that
/// stops reading cannot hold up the limit. Write errors are ignored, so this is only for commands
/// that tolerate truncated input (today `fetch --stdin` in `partial.rs`), never `commit -F -` or
/// `apply`.
pub(crate) fn output_within_input(
    cmd: Command,
    args: &[&str],
    limit: Duration,
    input: Option<Vec<u8>>,
) -> Result<Output> {
    let started = Instant::now();
    let Piped {
        mut child,
        stdin,
        stdout,
        stderr,
    } = spawn_piped(cmd, args, input.is_some())?;
    if let (Some(input), Some(mut stdin)) = (input, stdin) {
        // A git that exits early closes its end; its exit status and stderr say why. A writer
        // still blocked on a git that timed out is left to end when the pipe closes.
        std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
    }
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= limit => {
                reap(child);
                record(args, started, None, b"timed out");
                // The readers are left to finish by themselves: a grandchild may still hold the pipes.
                return Err(Error::invalid(format!(
                    "git {} did not answer within {} seconds.",
                    subcommand(args),
                    limit.as_secs()
                )));
            }
            Ok(None) => std::thread::sleep(POLL_INTERVAL),
            Err(e) => {
                reap(child);
                return Err(e.into());
            }
        }
    };
    let out = Output {
        code: status.code(),
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    };
    record(args, started, out.code, &out.stderr);
    Ok(out)
}

/// Kills and waits for `child` on a thread of its own: a git stuck in uninterruptible I/O (a hung
/// network mount) cannot be reaped, and the caller must not wait for it.
fn reap(mut child: Child) {
    std::thread::spawn(move || {
        let _ = child.kill();
        let _ = child.wait();
    });
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
    run_unchecked_with(command(Some(workdir)), args)
}

/// Asks git for its messages in English: `LC_ALL=C` outranks every other locale variable and
/// `LANGUAGE` is set as well, which gettext reads first when the locale is not `C`.
fn english(cmd: &mut Command) {
    cmd.env("LC_ALL", "C").env("LANGUAGE", "C");
}

/// [`run_unchecked`] with git's messages in English, for output that Canopy parses.
pub(crate) fn run_unchecked_english(workdir: &Path, args: &[&str]) -> Result<Output> {
    let mut cmd = command(Some(workdir));
    english(&mut cmd);
    run_unchecked_with(cmd, args)
}

fn run_unchecked_with(mut cmd: Command, args: &[&str]) -> Result<Output> {
    let started = Instant::now();
    let out = with_args(&mut cmd, args)
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

/// Like [`run_with_input`], but a git still running after `limit` is killed and the call fails
/// with [`Error::Invalid`] saying it did not answer.
pub(crate) fn run_with_input_within(
    workdir: &Path,
    args: &[&str],
    input: &[u8],
    limit: Duration,
) -> Result<String> {
    let out = output_within_input(command(Some(workdir)), args, limit, Some(input.to_vec()))?;
    if out.code == Some(0) {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(failure(args, &out.stdout, &out.stderr))
    }
}

/// Runs `cmd`, a git command built with [`command`], with `input` as its standard input and
/// waits for it to finish, with no time limit. The output is drained on threads of its own while
/// the input is written, so neither direction can wait on the other however much there is of it.
///
/// A git that exits before reading everything closes its end of the input: that is not an error
/// here, its exit code and standard error say why. Any other failure to write is one, and git is
/// killed before it can act on input that was cut short.
fn output_with_input(cmd: Command, args: &[&str], input: &[u8]) -> Result<Output> {
    let Piped {
        mut child,
        stdin,
        stdout,
        stderr,
    } = spawn_piped(cmd, args, !input.is_empty())?;
    // No input: the null device gives git its end at once, as a pipe closed at once would.
    if let Some(mut stdin) = stdin {
        // Dropping `stdin` afterwards closes the pipe, which is git's end of the input.
        if let Err(write_error) = stdin.write_all(input)
            && write_error.kind() != std::io::ErrorKind::BrokenPipe
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(write_error.into());
        }
    }
    let status = child.wait()?;
    Ok(Output {
        code: status.code(),
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

/// Like [`run`], feeding `input` to standard input.
pub fn run_with_input(workdir: &Path, args: &[&str], input: &[u8]) -> Result<String> {
    let started = Instant::now();
    let out = output_with_input(command(Some(workdir)), args, input)?;
    record(args, started, out.code, &out.stderr);
    if out.code == Some(0) {
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
    let out = output_with_input(command(Some(workdir)), args, input)?;
    let code = out.code;
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
/// error to `progress` as it arrives.
///
/// Returns standard error on success, one line per line git finished (without blank lines),
/// which is where git puts its summary and what it did to each ref.
pub fn run_streaming(
    workdir: Option<&Path>,
    args: &[&str],
    progress: impl FnMut(&str),
) -> Result<String> {
    run_streaming_env(workdir, args, &[], progress)
}

/// [`run_streaming`] with extra environment variables for the command (`GIT_LFS_FORCE_PROGRESS`
/// for git-lfs, which shows no progress unless it writes to a terminal).
pub fn run_streaming_env(
    workdir: Option<&Path>,
    args: &[&str],
    env: &[(&str, &str)],
    mut progress: impl FnMut(&str),
) -> Result<String> {
    let started = Instant::now();
    tracing::info!(command = %subcommand(args), args = ?loggable(args), "git started");
    let mut child = with_args(&mut command(workdir), args)
        .envs(env.iter().copied())
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

    // Standard error as a terminal would show it: a carriage return starts the line over, so
    // git's progress ("Receiving objects: 40%", "Rebasing (1/3)") is replaced by what follows
    // it on the same line instead of being glued to it.
    let mut stderr_all = Vec::new();
    if let Some(mut err) = child.stderr.take() {
        let mut segment = Vec::new();
        let mut shown = Vec::new();
        let mut chunk = [0u8; STDERR_CHUNK];
        loop {
            let n = match err.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            };
            for &b in &chunk[..n] {
                if b == b'\r' || b == b'\n' {
                    if !segment.is_empty() {
                        progress(String::from_utf8_lossy(&segment).trim());
                        shown = std::mem::take(&mut segment);
                    }
                    if b == b'\n' {
                        stderr_all.append(&mut shown);
                        stderr_all.push(b'\n');
                    }
                } else {
                    segment.push(b);
                }
            }
        }
        if !segment.is_empty() {
            progress(String::from_utf8_lossy(&segment).trim());
            shown = segment;
        }
        stderr_all.append(&mut shown);
    }

    let status = child.wait()?;
    record(args, started, status.code(), &stderr_all);
    if status.success() {
        tracing::info!(command = %subcommand(args), ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX), "git succeeded");
    }
    let stdout = stdout_reader.join().unwrap_or_default();
    if status.success() {
        let text = String::from_utf8_lossy(&stderr_all);
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        Ok(lines.join("\n"))
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
    version_number().is_some_and(|v| v >= MERGE_TREE_WRITE_TREE_SINCE)
}

#[cfg(test)]
mod tests {
    use crate::test_support::isolate_git;
    #[test]
    fn msys_never_expands_wildcards_and_keeps_the_users_options() {
        isolate_git!(msys_never_expands_wildcards_and_keeps_the_users_options);
        assert_eq!(msys_without_glob(None), "noglob");
        assert_eq!(msys_without_glob(Some(OsString::new())), "noglob");
        assert_eq!(
            msys_without_glob(Some(OsString::from("winsymlinks:nativestrict glob"))),
            "winsymlinks:nativestrict glob noglob"
        );
    }

    #[test]
    fn only_git_submodule_runs_with_msys_wildcards_off() {
        isolate_git!(only_git_submodule_runs_with_msys_wildcards_off);
        let msys = |args: &[&str]| {
            let mut cmd = command(None);
            with_args(&mut cmd, args);
            cmd.get_envs()
                .find(|(k, _)| k.eq_ignore_ascii_case("MSYS"))
                .and_then(|(_, v)| v.map(OsStr::to_os_string))
        };
        let submodule = msys(&["--literal-pathspecs", "-c", "a=b", "submodule", "update"]);
        if cfg!(windows) {
            assert!(
                submodule.is_some_and(|v| v.to_string_lossy().ends_with("noglob")),
                "the shell wrapper keeps paths as given"
            );
        } else {
            assert_eq!(submodule, None, "no MSYS outside Windows");
        }
        assert_eq!(
            msys(&["status"]),
            None,
            "other commands keep the user's options"
        );
    }

    use super::*;

    /// Output that is parsed is asked for in the C locale, whatever the user's language is.
    #[test]
    fn english_output_pins_the_locale() {
        isolate_git!(english_output_pins_the_locale);
        let mut cmd = command(None);
        english(&mut cmd);
        let value = |key: &str| {
            cmd.get_envs()
                .find(|(k, _)| *k == OsStr::new(key))
                .and_then(|(_, v)| v.map(OsStr::to_os_string))
        };
        assert_eq!(value("LC_ALL"), Some("C".into()));
        assert_eq!(value("LANGUAGE"), Some("C".into()));
    }

    #[cfg(unix)]
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "the test must demonstrate that ordinary git executes the fixture's fsmonitor command"
    )]
    fn repository_fsmonitor_command_is_not_executed() {
        use std::os::unix::fs::PermissionsExt;
        isolate_git!(repository_fsmonitor_command_is_not_executed);

        let dir = tempfile::tempdir().expect("temporary repository");
        run(dir.path(), &["init", "-q"]).expect("initialize repository");
        let script = dir.path().join("fsmonitor");
        let marker = dir.path().join("fsmonitor-ran");
        std::fs::write(&script, "#!/bin/sh\ntouch fsmonitor-ran\n").expect("create harmless probe");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("make probe executable");
        run(
            dir.path(),
            &["config", "--local", "core.fsmonitor", "./fsmonitor"],
        )
        .expect("configure repository probe");

        // Verify the fixture would execute its configured command with an ordinary git.
        let baseline = Command::new("git")
            .current_dir(dir.path())
            .args(["status", "--porcelain"])
            .output()
            .expect("run unprotected git");
        assert!(baseline.status.success(), "fixture git status failed");
        assert!(
            marker.exists(),
            "fixture did not activate the fsmonitor command"
        );
        std::fs::remove_file(&marker).expect("reset probe");

        run(dir.path(), &["status", "--porcelain"]).expect("safe git status");
        assert!(
            !marker.exists(),
            "repository fsmonitor command was executed"
        );
    }

    /// A repository whose `key` names a script that leaves a marker file, with two commits.
    #[cfg(unix)]
    fn poisoned_repository(key: &str) -> (tempfile::TempDir, PathBuf) {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("temporary repository");
        let path = dir.path();
        run(path, &["init", "-q"]).expect("initialize repository");
        run(path, &["config", "user.name", "Ada"]).expect("name");
        run(path, &["config", "user.email", "ada@example.com"]).expect("email");
        for message in ["first", "second"] {
            run(path, &["commit", "-q", "--allow-empty", "-m", message]).expect("commit");
        }
        let script = path.join("probe");
        std::fs::write(&script, "#!/bin/sh\ntouch ran\nexit 0\n").expect("probe");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        run(path, &["config", "--local", key, "./probe"]).expect("poison");
        let marker = path.join("ran");
        (dir, marker)
    }

    /// Ordinary git, as a user's terminal runs it: no editor variables from this process.
    #[cfg(unix)]
    #[expect(
        clippy::disallowed_methods,
        reason = "the test must demonstrate that ordinary git executes the fixture's command"
    )]
    fn plain_git(dir: &Path, args: &[&str]) {
        let _ = Command::new("git")
            .current_dir(dir)
            .args(args)
            .env_remove("GIT_EDITOR")
            .env_remove("GIT_SEQUENCE_EDITOR")
            .env_remove("EDITOR")
            .env_remove("VISUAL")
            .output()
            .expect("run unprotected git");
    }

    #[cfg(unix)]
    #[test]
    fn repository_editors_are_not_executed() {
        isolate_git!(repository_editors_are_not_executed);
        for (key, args) in [
            ("core.editor", vec!["commit", "-q", "--allow-empty"]),
            ("sequence.editor", vec!["rebase", "-i", "HEAD~1"]),
        ] {
            let (dir, marker) = poisoned_repository(key);
            plain_git(dir.path(), &args);
            assert!(marker.exists(), "fixture did not activate {key}");
            std::fs::remove_file(&marker).expect("reset probe");
            let _ = run_unchecked(dir.path(), &["rebase", "--abort"]);
            let _ = run_unchecked(dir.path(), &args);
            assert!(!marker.exists(), "{key} was executed by Canopy's git");
        }
    }

    #[cfg(unix)]
    #[test]
    fn repository_pager_command_is_not_executed() {
        use std::os::unix::fs::PermissionsExt;
        isolate_git!(repository_pager_command_is_not_executed);

        let dir = tempfile::tempdir().expect("temporary repository");
        let path = dir.path();
        run(path, &["init", "-q"]).expect("initialize repository");
        run(path, &["config", "user.name", "Ada"]).expect("name");
        run(path, &["config", "user.email", "ada@example.com"]).expect("email");
        run(path, &["commit", "-q", "--allow-empty", "-m", "first"]).expect("commit");
        let pager = path.join("pager");
        let marker = path.join("pager-ran");
        std::fs::write(&pager, "#!/bin/sh\ntouch pager-ran\ncat\n").expect("create probe");
        std::fs::set_permissions(&pager, std::fs::Permissions::from_mode(0o755))
            .expect("make probe executable");
        run(path, &["config", "core.pager", "./pager"]).expect("pager");
        run(path, &["config", "pager.log", "true"]).expect("pager.log");

        let out = run(path, &["log", "--oneline"]).expect("log");
        assert!(out.contains("first"), "{out}");
        assert!(!marker.exists(), "repository pager command was executed");
    }

    #[test]
    fn a_git_that_exits_before_reading_its_input_reports_its_own_error() {
        isolate_git!(a_git_that_exits_before_reading_its_input_reports_its_own_error);
        let dir = tempfile::tempdir().expect("tmp");
        run(dir.path(), &["init", "-q"]).expect("init");
        // More than a pipe buffer, so writing it fails once git has exited.
        let input = vec![b'x'; 1 << 20];
        match run_with_input(dir.path(), &["rev-parse", "--verify", "missing"], &input) {
            Err(Error::Cli { stderr, .. }) => assert!(stderr.contains("fatal"), "{stderr}"),
            other => panic!("expected git's error, got {other:?}"),
        }
    }

    #[test]
    fn logged_arguments_hide_secrets_and_messages() {
        isolate_git!(logged_arguments_hide_secrets_and_messages);
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
        // The text of a history search is never logged; a bare -S (sign) has nothing to hide.
        assert_eq!(
            loggable(&["log", "-Ssecret-token", "-G(pass)=x", "--"]),
            ["log", "-S<redacted>", "-G<redacted>", "--"]
        );
        assert_eq!(loggable(&["commit", "-S"]), ["commit", "-S"]);
        assert_eq!(redact_url("ssh://git@host/x"), "ssh://<redacted>@host/x");
        assert_eq!(redact_url("https://host/a@b"), "https://host/a@b");
        assert_eq!(redact_url("no url"), "no url");
        // Git and libcurl skip extra slashes; git splits user and host at the first slash only.
        for text in [
            "fatal: unable to access 'https:///u:secret@github.com/o/r.git/': error",
            "https:////secret@host/r",
            "https://u:secret?x@host/r",
            "from https://a@one/x to 'https://u:secret@two/y'",
        ] {
            let shown = redact_url(text);
            assert!(!shown.contains("secret"), "{shown}");
        }
        assert_eq!(
            redact_url("from https://a@one/x to https://two/y@z"),
            "from https://<redacted>@one/x to https://two/y@z"
        );
        assert_eq!(
            redact_url(
                "fatal: from https://alice:first-secret@one/r to https://bob:second-secret@two/r"
            ),
            "fatal: from https://<redacted>@one/r to https://<redacted>@two/r"
        );
        assert_eq!(
            loggable(&[
                "config",
                "--local",
                "--replace-all",
                "--",
                "user.email",
                "ada@example.com"
            ]),
            [
                "config",
                "--local",
                "--replace-all",
                "--",
                "user.email",
                "<redacted>"
            ]
        );
    }

    #[test]
    fn parses_git_versions() {
        isolate_git!(parses_git_versions);
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
        isolate_git!(unchecked_runs_return_the_exit_code);
        let dir = tempfile::tempdir().expect("tempdir");
        let out = run_unchecked(dir.path(), &["rev-parse", "--git-dir"]).expect("git runs");
        assert_ne!(out.code, Some(0));
        assert!(!out.stderr.is_empty(), "git explained nothing");
        let out = run_unchecked(dir.path(), &["--version"]).expect("git runs");
        assert_eq!(out.code, Some(0));
    }

    /// #196: `git config` exiting 1 with nothing on standard error is how it says a key is unset,
    /// an answer rather than a failure; a configuration git cannot read still fails loudly.
    #[test]
    fn an_unset_config_key_is_an_answer_not_a_failure() {
        isolate_git!(an_unset_config_key_is_an_answer_not_a_failure);
        let dir = tempfile::tempdir().expect("tempdir");
        run(dir.path(), &["init", "-q"]).expect("init");
        let get = ["config", "--get", "--", "canopy.unsetKey"];
        let unset = command(Some(dir.path()))
            .args(get)
            .output()
            .expect("git config");
        assert_eq!(unset.status.code(), Some(1));
        assert!(answered(&get, unset.status.code(), &unset.stderr));

        std::fs::write(dir.path().join(".git").join("config"), "[broken\n").expect("break config");
        let broken = command(Some(dir.path()))
            .args(get)
            .output()
            .expect("git config");
        assert!(!broken.status.success() && !broken.stderr.is_empty());
        assert!(!answered(&get, broken.status.code(), &broken.stderr));
        // Exit 1 from anything else is still a failure.
        assert!(!answered(&["fetch", "origin"], Some(1), b""));
    }

    #[test]
    fn subcommand_skips_option_values() {
        isolate_git!(subcommand_skips_option_values);
        assert_eq!(subcommand(&["fetch", "origin"]), "fetch");
        assert_eq!(subcommand(&["--no-pager", "log"]), "log");
        assert_eq!(
            subcommand(&["-c", "fetch.negotiationAlgorithm=noop", "fetch", "origin"]),
            "fetch"
        );
        assert_eq!(
            subcommand(&["-C", "/some/dir", "-c", "a=b", "status"]),
            "status"
        );
        assert_eq!(subcommand(&["-c", "a=b"]), "git");
        assert_eq!(subcommand(&[]), "git");
    }

    #[test]
    fn reports_stderr_on_failure() {
        isolate_git!(reports_stderr_on_failure);
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
        isolate_git!(streams_progress_lines);
        let dir = tempfile::tempdir().expect("tempdir");
        run_streaming(Some(dir.path()), &["init", "-q", "streamed"], |_| {}).expect("init");
        assert!(dir.path().join("streamed/.git").is_dir());

        let mut lines = Vec::new();
        run_streaming(
            Some(dir.path()),
            &["clone", "--progress", "streamed", "copy"],
            |l| lines.push(l.to_owned()),
        )
        .expect("clone");
        assert!(
            lines.iter().any(|l| l.starts_with("Cloning into 'copy'")),
            "{lines:?}"
        );
        assert!(
            lines.iter().all(|l| !l.is_empty() && l.trim() == l),
            "{lines:?}"
        );
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
    #[expect(
        clippy::disallowed_methods,
        reason = "tests drive git directly to build and check repositories"
    )]
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
    #[expect(
        clippy::disallowed_methods,
        reason = "tests drive git directly to build and check repositories"
    )]
    fn a_repository_chosen_by_the_environment_does_not_reach_git() {
        isolate_git!(a_repository_chosen_by_the_environment_does_not_reach_git);
        let git = |dir: &Path, args: &[&str]| {
            let out = Command::new("git")
                .current_dir(dir)
                .args(args)
                .output()
                .expect("git runs");
            assert!(out.status.success(), "git {args:?}");
            String::from_utf8_lossy(&out.stdout).trim().to_owned()
        };
        let (mine, other) = (
            tempfile::tempdir().expect("tempdir"),
            tempfile::tempdir().expect("tempdir"),
        );
        git(mine.path(), &["init", "-q"]);
        git(other.path(), &["init", "-q"]);

        // As if Canopy had been started by a git alias inside `other`: every variable set.
        let inherited: Vec<(&str, String)> = REPOSITORY_ENV
            .iter()
            .map(|key| (*key, other.path().join(".git").display().to_string()))
            .collect();
        let mut cmd = Command::new("git");
        for (key, value) in &inherited {
            cmd.env(key, value);
        }
        apply_env(&mut cmd);
        for key in REPOSITORY_ENV {
            assert!(
                cmd.get_envs()
                    .any(|(k, v)| k == OsStr::new(key) && v.is_none()),
                "{key} is kept out of the command"
            );
        }
        // Run for real the way `command` does: `mine` is where git works, whatever the
        // environment said.
        let out = cmd
            .current_dir(mine.path())
            .args(["rev-parse", "--absolute-git-dir"])
            .output()
            .expect("git runs");
        assert!(out.status.success());
        let found = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        assert_eq!(
            found.canonicalize().expect("exists"),
            mine.path().join(".git").canonicalize().expect("exists")
        );
    }

    #[test]
    fn askpass_sets_git_and_ssh_variables() {
        isolate_git!(askpass_sets_git_and_ssh_variables);
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
        isolate_git!(askpass_respects_the_users_own_programs);
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

    #[cfg(unix)]
    #[test]
    fn a_git_that_never_answers_is_killed_at_the_limit() {
        isolate_git!(a_git_that_never_answers_is_killed_at_the_limit);
        let mut cmd = command(None);
        cmd.args(["-c", "alias.nap=!sleep 3"]);
        let started = Instant::now();
        let err = output_within(cmd, &["nap"], Duration::from_millis(300))
            .expect_err("a command that outlasts the limit fails");
        assert!(started.elapsed() < Duration::from_secs(2), "{err}");
        assert!(err.to_string().contains("did not answer"), "{err}");
        let quick = output_within(command(None), &["--version"], Duration::from_secs(30))
            .expect("a quick command finishes");
        assert_eq!(quick.code, Some(0));
        assert!(!quick.stdout.is_empty(), "git printed no version");
    }

    #[cfg(unix)]
    #[test]
    fn a_git_given_input_is_killed_at_the_limit() {
        isolate_git!(a_git_given_input_is_killed_at_the_limit);
        let dir = tempfile::tempdir().expect("tmp");
        run(dir.path(), &["init", "-q"]).expect("init");
        run(dir.path(), &["config", "alias.nap", "!sleep 3"]).expect("alias");
        let started = Instant::now();
        let err = run_with_input_within(
            dir.path(),
            &["nap"],
            &vec![b'x'; 1 << 20],
            Duration::from_millis(300),
        )
        .expect_err("outlasts the limit");
        assert!(started.elapsed() < Duration::from_secs(2), "{err}");
        assert!(err.to_string().contains("did not answer"), "{err}");
        let out = run_with_input_within(
            dir.path(),
            &["hash-object", "--stdin"],
            b"hello\n",
            Duration::from_secs(30),
        )
        .expect("a quick command finishes");
        assert_eq!(out.trim(), "ce013625030ba8dba906f756967f9e9ca394464a");
    }

    #[test]
    fn no_askpass_leaves_the_environment_alone() {
        isolate_git!(no_askpass_leaves_the_environment_alone);
        let _hook = HookGuard::set(None);
        let cmd = user_command(&[]);
        for key in ["GIT_ASKPASS", "SSH_ASKPASS", "SSH_ASKPASS_REQUIRE"] {
            assert_eq!(env_of(&cmd, key), None, "{key}");
        }
        assert_eq!(env_of(&cmd, "CANOPY_ASKPASS_TOKEN"), None);
    }

    #[test]
    fn askpass_survives_a_poisoned_lock() {
        isolate_git!(askpass_survives_a_poisoned_lock);
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
        isolate_git!(git_reads_credentials_from_the_askpass_program);

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

    /// More than a pipe holds (64 KiB on Linux), for a git that never reads it.
    const UNREAD_INPUT: usize = 1 << 20;

    /// A git that exits without reading its input closes the pipe under the writer: that is not an
    /// error, and git's exit status and standard error are what the caller sees. The old runners
    /// passed this too: it guards the `BrokenPipe` path of the shared one.
    #[test]
    fn a_git_that_ignores_its_input_still_answers() {
        isolate_git!(a_git_that_ignores_its_input_still_answers);
        let dir = tempfile::tempdir().expect("tempdir");
        let input = vec![b'x'; UNREAD_INPUT];
        let version = run_with_input(dir.path(), &["version"], &input).expect("git version");
        assert!(version.starts_with("git version"), "{version}");
        let (code, out) =
            run_answer(dir.path(), &["version"], &input, &[0]).expect("git version answers");
        assert_eq!(code, 0);
        assert!(out.starts_with(b"git version"));
        let err = run_with_input(dir.path(), &["no-such-subcommand"], &input)
            .expect_err("an unknown subcommand fails");
        assert!(
            err.to_string().contains("no-such-subcommand"),
            "git's own message: {err}"
        );
    }

    /// `run_with_input` shares the writer thread with `run_answer`: a git that answers while it
    /// still reads must not hold it up either (issue #196).
    #[test]
    fn run_with_input_reads_the_answer_while_it_writes() {
        use std::sync::mpsc;
        /// Generous for a slow machine; a hung git never answers.
        const LIMIT: Duration = Duration::from_secs(60);
        isolate_git!(run_with_input_reads_the_answer_while_it_writes);

        let dir = tempfile::tempdir().expect("tempdir");
        run(dir.path(), &["init", "-q"]).expect("git init");
        // git check-ignore exits 1 when it ignores nothing, which `run_with_input` reports.
        std::fs::write(dir.path().join(".gitignore"), "dir*\n").expect("write .gitignore");
        let mut input = Vec::new();
        for i in 0..4_000 {
            input.extend_from_slice(format!("dir{i}/{}\0", "x".repeat(150)).as_bytes());
        }
        let expected = input.len();
        let path = dir.path().to_owned();
        let (tx, rx) = mpsc::channel();
        // Not joined: a deadlocked run must fail the test, not hang it.
        std::thread::spawn(move || {
            let args = ["check-ignore", "-v", "-n", "--no-index", "-z", "--stdin"];
            let _ = tx.send(run_with_input(&path, &args, &input));
        });
        let out = rx
            .recv_timeout(LIMIT)
            .expect("git check-ignore answered within the limit")
            .expect("git check-ignore");
        // Four fields per path where the input has one, so every path is answered.
        assert!(out.len() > expected, "{} bytes for {expected}", out.len());
    }
}
