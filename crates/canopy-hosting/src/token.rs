//! Finding the user's access token: provider environment variables, or git's credential helpers.

use std::io::{Read, Write};
use std::process::Stdio;
use std::time::{Duration, Instant};

use canopy_core::forge::ForgeKind;

/// How long `git credential fill` may run before it is killed.
const CREDENTIAL_TIMEOUT: Duration = Duration::from_secs(10);

/// The variables each provider's own CLI reads, in priority order.
fn env_names(kind: ForgeKind) -> [&'static str; 2] {
    match kind {
        ForgeKind::GitHub => ["GH_TOKEN", "GITHUB_TOKEN"],
        ForgeKind::GitLab => ["GITLAB_TOKEN", "GL_TOKEN"],
    }
}

fn token_from(kind: ForgeKind, lookup: impl Fn(&str) -> Option<String>) -> Option<String> {
    env_names(kind)
        .into_iter()
        .filter_map(lookup)
        .map(|v| v.trim().to_owned())
        .find(|v| !v.is_empty())
}

/// A token from the environment: `GH_TOKEN` then `GITHUB_TOKEN` for GitHub, `GITLAB_TOKEN` then
/// `GL_TOKEN` for GitLab. Trimmed; empty values are skipped.
pub fn token_from_env(kind: ForgeKind) -> Option<String> {
    token_from(kind, |name| std::env::var(name).ok())
}

/// The password git's credential helpers hold for `https://<host>`, which for GitHub and GitLab
/// is a personal access or OAuth token.
///
/// Runs `git credential fill` without any prompt (no terminal, no askpass, Git Credential
/// Manager non-interactive) and kills it after 10 seconds. Returns `None` when no helper knows
/// the host. The secret is never logged.
pub fn token_from_git_credentials(host: &str) -> Option<String> {
    let host = host.trim();
    if host.is_empty() || host.contains(['\n', '\r', '\0']) {
        return None;
    }
    let mut cmd = canopy_core::cli::command(None);
    cmd.args(["-c", "core.askPass=", "credential", "fill"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .env_remove("GIT_ASKPASS")
        .env_remove("SSH_ASKPASS")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(error = %e, "could not run git credential fill");
            return None;
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let request = format!("protocol=https\nhost={host}\n\n");
        if let Err(e) = stdin.write_all(request.as_bytes()) {
            tracing::debug!(error = %e, "could not write to git credential fill");
        }
        // Dropping stdin closes it, which ends git's input.
    }
    // Read on a separate thread so a full pipe never stalls the child.
    let reader = child.stdout.take().map(|mut out| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = out.read_to_end(&mut buf);
            buf
        })
    });
    let deadline = Instant::now() + CREDENTIAL_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() >= deadline => {
                tracing::warn!(host, "git credential fill timed out; killing it");
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => {
                tracing::debug!(error = %e, "waiting for git credential fill failed");
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let output = reader.and_then(|r| r.join().ok()).unwrap_or_default();
    if !status.is_some_and(|s| s.success()) {
        tracing::debug!(host, "no stored git credential for host");
        return None;
    }
    password_from(&String::from_utf8_lossy(&output))
}

/// The `password=` value of `git credential` output.
fn password_from(output: &str) -> Option<String> {
    output
        .lines()
        .find_map(|l| l.strip_prefix("password="))
        .map(|p| p.trim_end_matches('\r').to_owned())
        .filter(|p| !p.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_priority_and_trimming() {
        let vars = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| (*v).to_owned())
            }
        };
        assert_eq!(
            token_from(
                ForgeKind::GitHub,
                vars(&[("GH_TOKEN", " gh1 \n"), ("GITHUB_TOKEN", "gh2")])
            )
            .as_deref(),
            Some("gh1")
        );
        assert_eq!(
            token_from(
                ForgeKind::GitHub,
                vars(&[("GH_TOKEN", "  "), ("GITHUB_TOKEN", "gh2")])
            )
            .as_deref(),
            Some("gh2")
        );
        assert_eq!(
            token_from(
                ForgeKind::GitLab,
                vars(&[("GH_TOKEN", "gh"), ("GL_TOKEN", "gl")])
            )
            .as_deref(),
            Some("gl")
        );
        assert_eq!(
            token_from(
                ForgeKind::GitLab,
                vars(&[("GITLAB_TOKEN", "a"), ("GL_TOKEN", "b")])
            )
            .as_deref(),
            Some("a")
        );
        assert_eq!(token_from(ForgeKind::GitLab, vars(&[])), None);
    }

    #[test]
    fn parses_password_line() {
        assert_eq!(
            password_from("protocol=https\nhost=github.com\nusername=x\npassword=ghp_a=b\n")
                .as_deref(),
            Some("ghp_a=b")
        );
        assert_eq!(password_from("protocol=https\nhost=x\n"), None);
        assert_eq!(password_from("password=\n"), None);
    }

    #[test]
    fn rejects_injection_in_host() {
        assert_eq!(
            token_from_git_credentials("github.com\nhost=evil.example"),
            None
        );
        assert_eq!(token_from_git_credentials(""), None);
    }
}
