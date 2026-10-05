//! Finding the user's access token: provider environment variables, or git's credential helpers.

use std::io::{Read, Write};
use std::process::Stdio;
use std::time::{Duration, Instant};

use canopy_core::forge::{ForgeKind, normalise_host};

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
///
/// This ignores the host, so a GitLab token meant for gitlab.com would also be offered to any
/// host recognised as GitLab, including one recognised only because its name contains
/// "gitlab" (`gitlab.attacker.example`). Use [`token_from_env_for`].
#[deprecated(
    note = "ignores the host and can leak a token to an untrusted server; use token_from_env_for"
)]
pub fn token_from_env(kind: ForgeKind) -> Option<String> {
    token_from(kind, |name| std::env::var(name).ok())
}

/// A token from the environment for the project on `host`, or `None` when the environment's
/// tokens are not meant for that host.
///
/// - GitHub: `GH_TOKEN` then `GITHUB_TOKEN`, only for `github.com`.
/// - GitLab: `GITLAB_TOKEN` then `GL_TOKEN`, only for `gitlab.com`, for the host named by the
///   `GITLAB_HOST` variable (which the `glab` CLI reads alongside the token), or for a host in
///   `trusted_gitlab_hosts`, the self-hosted instances the user listed in Canopy's settings.
///   A host recognised as GitLab only because its name contains "gitlab" never gets an
///   environment token.
///
/// Hosts are compared by name only: case, a scheme, credentials, a port and a path are ignored
/// on every side (see [`canopy_core::forge::normalise_host`]). Values are trimmed; empty values
/// are skipped.
pub fn token_from_env_for(
    kind: ForgeKind,
    host: &str,
    trusted_gitlab_hosts: &[String],
) -> Option<String> {
    let lookup = |name: &str| std::env::var(name).ok();
    env_token_for(kind, host, trusted_gitlab_hosts, lookup)
}

/// [`token_from_env_for`] with an injectable environment.
fn env_token_for(
    kind: ForgeKind,
    host: &str,
    trusted_gitlab_hosts: &[String],
    lookup: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    let host = normalise_host(host)?;
    let allowed = match kind {
        ForgeKind::GitHub => host == "github.com",
        ForgeKind::GitLab => {
            host == "gitlab.com"
                || lookup("GITLAB_HOST")
                    .as_deref()
                    .and_then(normalise_host)
                    .is_some_and(|h| h == host)
                || trusted_gitlab_hosts
                    .iter()
                    .filter_map(|h| normalise_host(h))
                    .any(|h| h == host)
        }
    };
    if allowed {
        token_from(kind, lookup)
    } else {
        tracing::debug!(
            host = %host,
            "not offering an environment token to a host it was not set for"
        );
        None
    }
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

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name: &str| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    const ALL: &[(&str, &str)] = &[
        ("GH_TOKEN", "gh"),
        ("GITHUB_TOKEN", "gh2"),
        ("GITLAB_TOKEN", "gl"),
        ("GL_TOKEN", "gl2"),
    ];

    #[test]
    fn env_tokens_go_only_to_their_own_hosts() {
        let none: &[String] = &[];
        let trusted = vec!["https://Code.Corp:8443/".to_owned(), "git.corp".to_owned()];
        // (kind, host, trusted list, expected)
        let cases: &[(ForgeKind, &str, &[String], Option<&str>)] = &[
            (ForgeKind::GitHub, "github.com", none, Some("gh")),
            (ForgeKind::GitHub, "GitHub.COM", none, Some("gh")),
            (ForgeKind::GitHub, "https://github.com/", none, Some("gh")),
            (ForgeKind::GitHub, "github.com.evil.example", none, None),
            (ForgeKind::GitHub, "evil.example", none, None),
            (ForgeKind::GitLab, "gitlab.com", none, Some("gl")),
            (ForgeKind::GitLab, "GITLAB.com", none, Some("gl")),
            // Auto-detected because the first label contains "gitlab": never trusted.
            (ForgeKind::GitLab, "gitlab.evil.example", none, None),
            (ForgeKind::GitLab, "gitlab.attacker.io", none, None),
            (ForgeKind::GitLab, "my-gitlab.corp", none, None),
            (ForgeKind::GitLab, "gitlab.com.evil.example", none, None),
            // Hosts the user listed in settings, compared by name only.
            (ForgeKind::GitLab, "code.corp", &trusted, Some("gl")),
            (ForgeKind::GitLab, "git.corp", &trusted, Some("gl")),
            (ForgeKind::GitLab, "gitlab.evil.example", &trusted, None),
            // A GitLab host in the settings list is still not a GitHub host.
            (ForgeKind::GitHub, "code.corp", &trusted, None),
            (ForgeKind::GitLab, "", none, None),
        ];
        for (kind, host, list, want) in cases {
            assert_eq!(
                env_token_for(*kind, host, list, env(ALL)).as_deref(),
                *want,
                "{kind:?} {host:?} {list:?}"
            );
        }
    }

    #[test]
    fn gitlab_host_variable_trusts_one_more_host() {
        const BARE_HOST: &[(&str, &str)] = &[("GL_TOKEN", "gl2"), ("GITLAB_HOST", "git.corp")];
        const GH_HOST: &[(&str, &str)] = &[("GH_TOKEN", "gh"), ("GITLAB_HOST", "git.corp")];
        const BAD_HOST: &[(&str, &str)] = &[("GITLAB_TOKEN", "gl"), ("GITLAB_HOST", "  ")];
        const WITH_HOST: &[(&str, &str)] = &[
            ("GITLAB_TOKEN", "gl"),
            ("GITLAB_HOST", "https://GitLab.Example.org:8443/"),
        ];
        let none: &[String] = &[];
        for (host, want) in [
            ("gitlab.example.org", Some("gl")),
            ("GITLAB.EXAMPLE.ORG", Some("gl")),
            ("gitlab.com", Some("gl")),
            ("gitlab.evil.example", None),
            ("example.org", None),
        ] {
            assert_eq!(
                env_token_for(ForgeKind::GitLab, host, none, env(WITH_HOST)).as_deref(),
                want,
                "{host}"
            );
        }
        assert_eq!(
            env_token_for(ForgeKind::GitLab, "git.corp", none, env(BARE_HOST)).as_deref(),
            Some("gl2")
        );
        // GITLAB_HOST does not make a GitHub token flow anywhere.
        assert_eq!(
            env_token_for(ForgeKind::GitHub, "git.corp", none, env(GH_HOST)),
            None
        );
        // An unparsable GITLAB_HOST trusts nothing.
        assert_eq!(
            env_token_for(ForgeKind::GitLab, "gitlab.example.org", none, env(BAD_HOST)),
            None
        );
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
