//! Running the `git` executable.
//!
//! Every mutation goes through here rather than through libgit2, because only `git` itself
//! honours everything a user's setup relies on: credential helpers and SSH agents, hooks
//! (`pre-commit`, `commit-msg`, `post-checkout`), commit and tag signing, clean/smudge filters such
//! as Git LFS, sparse checkouts and `includeIf` configuration. Fork and Sourcetree make the same
//! choice.
//!
//! Commands never wait for a terminal: `GIT_TERMINAL_PROMPT=0` makes a missing credential an error
//! instead of a hang, and `GIT_EDITOR=true` accepts default messages for `--continue`.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use crate::{Error, Result};

/// Builds a `git` command for `workdir` with a non-interactive environment.
pub fn command(workdir: Option<&Path>) -> Command {
    let mut cmd = Command::new("git");
    if let Some(dir) = workdir {
        cmd.current_dir(dir);
    }
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("GIT_MERGE_AUTOEDIT", "no")
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

fn subcommand(args: &[&str]) -> String {
    args.iter()
        .find(|a| !a.starts_with('-'))
        .copied()
        .unwrap_or("git")
        .to_owned()
}

fn failure(args: &[&str], stdout: &[u8], stderr: &[u8]) -> Error {
    let mut msg = String::from_utf8_lossy(stderr).trim().to_owned();
    if msg.is_empty() {
        msg = String::from_utf8_lossy(stdout).trim().to_owned();
    }
    Error::Cli {
        command: subcommand(args),
        stderr: msg,
    }
}

/// Runs `git args…` in `workdir` and returns its standard output.
pub fn run(workdir: &Path, args: &[&str]) -> Result<String> {
    tracing::debug!(?args, "git");
    let out = command(Some(workdir))
        .args(args)
        .output()
        .map_err(Error::GitMissing)?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(failure(args, &out.stdout, &out.stderr))
    }
}

/// Like [`run`], feeding `input` to standard input.
pub fn run_with_input(workdir: &Path, args: &[&str], input: &[u8]) -> Result<String> {
    tracing::debug!(?args, "git (stdin)");
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
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(failure(args, &out.stdout, &out.stderr))
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
    tracing::debug!(?args, "git (streaming)");
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
