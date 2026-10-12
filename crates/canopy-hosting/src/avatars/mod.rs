//! Commit identity pictures, restricted to the repository's forge and GitHub's avatar CDN.

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

/// Preserve the transport's cause without revealing an author's private picture URL.
fn picture_error(context: &str, detail: &str, url: &str) -> Error {
    let detail = detail.replace(url, "[picture URL]");
    Error::Network(crate::http::shorten(&format!("{context}: {detail}")))
}

impl Avatar {
    /// Downloads this picture without credentials or redirects, on a background thread.
    pub fn download(&self) -> Result<Vec<u8>> {
        let mut config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .https_only(!self.loopback)
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_global(Some(TOTAL_TIMEOUT))
            .user_agent(user_agent())
            .tls_config(
                TlsConfig::builder()
                    .root_certs(RootCerts::PlatformVerifier)
                    .build(),
            );
        if self.loopback {
            config = config.proxy(None);
        }
        let agent = crate::transport::agent(config.build());
        let mut response = agent
            .get(&self.url)
            .header("Accept", "image/png, image/jpeg")
            .call()
            .map_err(|e| {
                picture_error(
                    "Could not download the profile picture",
                    &e.to_string(),
                    &self.url,
                )
            })?;
        let status = response.status().as_u16();
        if status == crate::http::status::TOO_MANY_REQUESTS {
            return Err(Error::RateLimited {
                reset: crate::http::rate_limit_reset(response.headers()),
            });
        }
        if !crate::http::status::SUCCESS.contains(&status) {
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
            .map_err(|e| {
                picture_error(
                    "Profile picture is unreadable or exceeds 2 MiB",
                    &e.to_string(),
                    &self.url,
                )
            })
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
    let picture_service = !loopback
        && forge.kind == ForgeKind::GitLab
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
        );
    let allowed = if loopback {
        let base: Uri = api_base.parse().ok()?;
        uri.scheme_str() == Some("http") && uri.authority() == base.authority()
    } else {
        uri.scheme_str() == Some("https")
            && (authority == forge.authority()
                || (forge.kind == ForgeKind::GitHub
                    && forge.host == "github.com"
                    && authority == "avatars.githubusercontent.com")
                || picture_service)
    };
    allowed.then(|| Avatar {
        url: if picture_service {
            without_stand_in(authority, &uri)
        } else {
            url.to_owned()
        },
        loopback,
    })
}

/// Gravatar and Libravatar draw a stand-in for an email without a picture (GitLab asks for a
/// generated pattern, `d=identicon`; without `d`, their logo) unless asked for a 404. With the 404
/// the author keeps Canopy's initials and is remembered as having no picture.
fn without_stand_in(authority: &str, uri: &Uri) -> String {
    let mut query: Vec<&str> = uri
        .query()
        .unwrap_or_default()
        .split('&')
        .filter(|pair| {
            let key = pair.split_once('=').map_or(*pair, |(key, _)| key);
            !pair.is_empty() && !matches!(key, "d" | "default")
        })
        .collect();
    query.push("d=404");
    format!("https://{authority}{}?{}", uri.path(), query.join("&"))
}

#[derive(Deserialize)]
struct Account {
    avatar_url: Option<String>,
}

#[derive(Deserialize)]
struct Commit {
    sha: String,
    author: Option<Account>,
    committer: Option<Account>,
    #[serde(rename = "commit")]
    people: Option<CommitPeople>,
}

#[derive(Deserialize)]
struct CommitPeople {
    author: Option<CommitPerson>,
    committer: Option<CommitPerson>,
}

#[derive(Deserialize)]
struct CommitPerson {
    email: Option<String>,
}

impl Commit {
    fn account(self, email: &str) -> Option<Account> {
        let people = self.people?;
        let matches = |person: &Option<CommitPerson>| {
            person.as_ref().and_then(|p| p.email.as_deref()) == Some(email)
        };
        if matches(&people.author) {
            self.author
        } else if matches(&people.committer) {
            self.committer
        } else {
            None
        }
    }
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
            if !matches!(commit.len(), SHA1_HEX_LEN | SHA256_HEX_LEN)
                || !commit.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(Error::Parse("Invalid commit id for profile lookup".into()));
            }
            // The list endpoint, not `commits/<sha>`: that one carries the changed files and, for a
            // large commit, outgrows the lookup's size limit.
            let commits: Vec<Commit> =
                http.avatar_json(&format!("{project}/commits?sha={commit}&per_page=1"), &[])?;
            commits
                .into_iter()
                .next()
                .filter(|c| c.sha.eq_ignore_ascii_case(commit))
                .and_then(|c| c.account(email))
                .and_then(|a| a.avatar_url)
        }
        ForgeKind::GitLab => {
            if email.is_empty()
                || email.len() > MAX_EMAIL_LEN
                || email.chars().any(char::is_control)
            {
                return Ok(None);
            }
            let encoded = encode(email);
            let account: Account = http.avatar_json(
                &format!("/avatar?email={encoded}&size=64"),
                &[email, &encoded],
            )?;
            account.avatar_url
        }
    };
    Ok(url.and_then(|u| validate(&u, forge, http.base())))
}

/// The status [`is_unknown_commit`] recognises, as GitHub's own `commits/<sha>` endpoint answers.
const UNKNOWN_COMMIT: u16 = 422;
/// Connecting to the picture's host gives up after this long, and the whole download after the
/// longer one.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(10);
/// The lengths of a Git object id: SHA-1 and SHA-256.
const SHA1_HEX_LEN: usize = 40;
const SHA256_HEX_LEN: usize = 64;
/// The longest email address looked up: a 64-character local part, the @, a 255-character domain.
const MAX_EMAIL_LEN: usize = 320;

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
            .avatar_json::<serde::de::IgnoredAny>(project, &[])
            .map(|_| ()),
        ForgeKind::GitLab => Err(Error::NotFound(
            "GitLab picture lookups name no repository".into(),
        )),
    }
}

#[cfg(test)]
mod network_failures;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picture_errors_preserve_the_cause_without_exposing_the_url() {
        let url = "https://avatars.githubusercontent.com/u/17?size=64&private=value";
        let error = picture_error(
            "Could not download the profile picture",
            &format!("connection refused for {url}"),
            url,
        );
        let message = error.to_string();
        assert!(message.contains("connection refused"), "{message}");
        assert!(!message.contains("private=value"), "{message}");
        assert!(!message.contains(url), "{message}");
    }

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

    #[test]
    fn gitlab_picture_links_ask_for_no_generated_stand_in() {
        let gitlab = Forge::parse("https://gitlab.com/group/repo", &[]).unwrap();
        let link = |url: &str| validate(url, &gitlab, "https://gitlab.com/api/v4").map(|a| a.url);
        // GitLab links Gravatar with `d=identicon`: an author without a picture would get a
        // generated pattern instead of Canopy's initials. Asking for 404 keeps the initials.
        assert_eq!(
            link("https://secure.gravatar.com/avatar/0a1b?s=128&d=identicon").as_deref(),
            Some("https://secure.gravatar.com/avatar/0a1b?s=128&d=404")
        );
        assert_eq!(
            link("https://seccdn.libravatar.org/avatar/0a1b?default=retro&s=80").as_deref(),
            Some("https://seccdn.libravatar.org/avatar/0a1b?s=80&d=404")
        );
        assert_eq!(
            link("https://www.gravatar.com/avatar/0a1b?d=https%3A%2F%2Fexample.com%2Fa.png")
                .as_deref(),
            Some("https://www.gravatar.com/avatar/0a1b?d=404")
        );
        assert_eq!(
            link("https://gravatar.com/avatar/0a1b").as_deref(),
            Some("https://gravatar.com/avatar/0a1b?d=404"),
            "without a default Gravatar draws its own logo"
        );
        assert_eq!(
            link("https://0.gravatar.com/avatar/0a1b?d=404&s=64").as_deref(),
            Some("https://0.gravatar.com/avatar/0a1b?s=64&d=404")
        );
        // The same host and path; pictures on GitLab itself are left as they are.
        assert_eq!(
            link("https://gitlab.com/uploads/-/system/user/avatar/1/a.png?width=64").as_deref(),
            Some("https://gitlab.com/uploads/-/system/user/avatar/1/a.png?width=64")
        );
    }
}
