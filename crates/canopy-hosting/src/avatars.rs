//! Author account pictures, restricted to the repository's forge and GitHub's avatar CDN.

use std::fmt::Write as _;
use std::time::Duration;

use canopy_core::forge::{Forge, ForgeKind};
use serde::Deserialize;
use ureq::http::Uri;
use ureq::tls::{RootCerts, TlsConfig};

use crate::error::{Error, Result};
use crate::http::{Http, encode, is_loopback_base, user_agent};

/// Opaque cache key scoped to the host and author, shared across repositories on that host.
pub fn cache_key(forge: &Forge, email: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(forge.authority().as_bytes());
    hash.update([0]);
    hash.update(email.as_bytes());
    hash.finalize().iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// Maximum compressed picture size; the application also bounds decoded dimensions.
pub const MAX_AVATAR_BYTES: u64 = 2 * 1024 * 1024;

/// A validated picture URL. Only a provider response for the repository's host can construct it.
#[derive(Clone)]
pub struct Avatar {
    url: String,
    loopback: bool,
}

impl std::fmt::Debug for Avatar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Avatar").finish_non_exhaustive()
    }
}

impl Avatar {
    /// Downloads this picture without credentials or redirects, on a background thread.
    #[expect(
        clippy::map_err_ignore,
        reason = "kept as it was; docs/backlog.md has the entry to carry the transport's message"
    )]
    pub fn download(&self) -> Result<Vec<u8>> {
        let mut config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .https_only(!self.loopback)
            .timeout_connect(Some(Duration::from_secs(5)))
            .timeout_global(Some(Duration::from_secs(10)))
            .user_agent(user_agent())
            .tls_config(
                TlsConfig::builder()
                    .root_certs(RootCerts::PlatformVerifier)
                    .build(),
            );
        if self.loopback {
            config = config.proxy(None);
        }
        let agent = ureq::Agent::new_with_config(config.build());
        let mut response = agent
            .get(&self.url)
            .header("Accept", "image/png, image/jpeg")
            .call()
            .map_err(|_| Error::Network("Could not download the profile picture".into()))?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(Error::Api {
                status,
                message: "Profile picture is unavailable".into(),
            });
        }
        response
            .body_mut()
            .with_config()
            .limit(MAX_AVATAR_BYTES)
            .read_to_vec()
            .map_err(|_| Error::Network("Profile picture is unreadable or exceeds 2 MiB".into()))
    }
}

fn validate(url: &str, forge: &Forge, api_base: &str) -> Option<Avatar> {
    if url.bytes().any(|b| b.is_ascii_control() || b == b'\\') {
        return None;
    }
    let uri: Uri = url.parse().ok()?;
    let authority = uri.authority()?.as_str();
    if authority.contains('@') || uri.path().is_empty() {
        return None;
    }
    // Tests and screenshots may fetch only from the exact loopback API server, never the web.
    let loopback = is_loopback_base(api_base);
    let allowed = if loopback {
        let base: Uri = api_base.parse().ok()?;
        uri.scheme_str() == Some("http") && uri.authority() == base.authority()
    } else {
        uri.scheme_str() == Some("https")
            && (authority == forge.authority()
                || (forge.kind == ForgeKind::GitHub
                    && forge.host == "github.com"
                    && authority == "avatars.githubusercontent.com")
                || (forge.kind == ForgeKind::GitLab
                    && uri.path().starts_with("/avatar/")
                    && matches!(
                        authority,
                        "gravatar.com"
                            | "www.gravatar.com"
                            | "secure.gravatar.com"
                            | "0.gravatar.com"
                            | "1.gravatar.com"
                            | "2.gravatar.com"
                            | "seccdn.libravatar.org"
                    )))
    };
    allowed.then(|| Avatar {
        url: url.to_owned(),
        loopback,
    })
}

#[derive(Deserialize)]
struct Account {
    avatar_url: Option<String>,
}

#[derive(Deserialize)]
struct Commit {
    sha: String,
    author: Option<Account>,
}

pub(crate) fn lookup(
    http: &Http,
    forge: &Forge,
    project: &str,
    commit: &str,
    email: &str,
) -> Result<Option<Avatar>> {
    let url = match forge.kind {
        ForgeKind::GitHub => {
            if !matches!(commit.len(), 40 | 64) || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(Error::Parse("Invalid commit id for profile lookup".into()));
            }
            // The list endpoint, not `commits/<sha>`: that one carries the changed files and, for a
            // large commit, outgrows the lookup's size limit.
            let commits: Vec<Commit> =
                http.avatar_json(&format!("{project}/commits?sha={commit}&per_page=1"))?;
            commits
                .into_iter()
                .next()
                .filter(|c| c.sha.eq_ignore_ascii_case(commit))
                .and_then(|c| c.author)
                .and_then(|a| a.avatar_url)
        }
        ForgeKind::GitLab => {
            if email.is_empty() || email.len() > 320 || email.chars().any(char::is_control) {
                return Ok(None);
            }
            let account: Account =
                http.avatar_json(&format!("/avatar?email={}&size=64", encode(email)))?;
            account.avatar_url
        }
    };
    Ok(url.and_then(|u| validate(&u, forge, http.base())))
}

/// The status [`is_unknown_commit`] recognises, as GitHub's own `commits/<sha>` endpoint answers.
const UNKNOWN_COMMIT: u16 = 422;

/// Whether a lookup failed only because the host does not have that commit (it was never pushed
/// there): the author may still be found through another of their commits.
///
/// A 404 says the same only once [`crate::Client::author_repository`] finds the repository.
pub fn is_unknown_commit(error: &Error) -> bool {
    matches!(error, Error::Api { status, .. } if *status == UNKNOWN_COMMIT)
}

/// Asks for the repository itself, the way [`lookup`] asks for a commit. GitHub answers 404 both
/// for a commit it does not have and for a repository it will not show (private, without a token
/// that can see it); `Ok` here means the repository is visible, so the commit is the unknown one.
/// GitLab's lookup names no repository: there nothing is sent, and the lookup's 404 stands.
pub(crate) fn repository(http: &Http, kind: ForgeKind, project: &str) -> Result<()> {
    match kind {
        ForgeKind::GitHub => http
            .avatar_json::<serde::de::IgnoredAny>(project)
            .map(|_| ()),
        ForgeKind::GitLab => Err(Error::NotFound(
            "GitLab picture lookups name no repository".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picture_hosts_are_exact_and_never_contain_credentials() {
        let github = Forge::parse("https://github.com/example/repo", &[]).unwrap();
        assert!(
            validate(
                "https://avatars.githubusercontent.com/u/1",
                &github,
                "https://api.github.com"
            )
            .is_some()
        );
        for url in [
            "http://avatars.githubusercontent.com/u/1",
            "https://avatars.githubusercontent.com.evil/u/1",
            "https://avatars.githubusercontent.com@evil/u/1",
            "https://user:token@github.com/a",
            "file:///a",
            "https://avatars.githubusercontent.com:8443/u/1",
            "https://www.gravatar.com/avatar/1",
            "https://github.com/a\\b",
        ] {
            assert!(
                validate(url, &github, "https://api.github.com").is_none(),
                "{url}"
            );
        }
        let gitlab = Forge::parse(
            "https://gitlab.corp:8443/group/repo",
            &["gitlab.corp".into()],
        )
        .unwrap();
        assert!(
            validate(
                "https://gitlab.corp:8443/uploads/a.png",
                &gitlab,
                "https://gitlab.corp:8443/api/v4"
            )
            .is_some()
        );
        assert!(
            validate(
                "https://gitlab.com/uploads/a.png",
                &gitlab,
                "https://gitlab.corp:8443/api/v4"
            )
            .is_none()
        );
        for host in [
            "gravatar.com",
            "www.gravatar.com",
            "secure.gravatar.com",
            "0.gravatar.com",
            "seccdn.libravatar.org",
        ] {
            assert!(
                validate(
                    &format!("https://{host}/avatar/123"),
                    &gitlab,
                    "https://gitlab.corp:8443/api/v4"
                )
                .is_some()
            );
            assert!(
                validate(
                    &format!("https://{host}/profiles/123"),
                    &gitlab,
                    "https://gitlab.corp:8443/api/v4"
                )
                .is_none()
            );
        }
        assert!(validate("http://127.0.0.1:1234/a", &github, "http://127.0.0.1:1234").is_some());
        assert!(validate("http://127.0.0.1:1235/a", &github, "http://127.0.0.1:1234").is_none());
        assert!(
            validate(
                "https://avatars.githubusercontent.com/u/1",
                &github,
                "http://127.0.0.1:1234"
            )
            .is_none()
        );
    }
}
