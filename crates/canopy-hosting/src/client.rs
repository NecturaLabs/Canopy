//! [`Client`]: the provider-neutral entry point.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use canopy_core::forge::{Forge, ForgeKind};

use crate::error::Result;
use crate::http::{Http, redact_userinfo};
use crate::model::{MergeMethod, NewRequest, RequestDetails, RequestSummary, StateFilter};
use crate::{github, gitlab};

/// Largest page both providers serve.
pub(crate) const PER_PAGE: usize = 100;

/// RFC 3339 → unix seconds; 0 when missing or malformed.
pub(crate) fn unix(ts: Option<&str>) -> i64 {
    let Some(ts) = ts.filter(|t| !t.is_empty()) else {
        return 0;
    };
    match chrono::DateTime::parse_from_rfc3339(ts) {
        Ok(t) => t.timestamp(),
        Err(e) => {
            tracing::debug!(timestamp = ts, error = %e, "unparsable timestamp");
            0
        }
    }
}

/// The result of a secondary request in [`Client::details`]: logged and dropped on failure so
/// one forbidden or missing endpoint does not hide the rest of the request.
pub(crate) fn optional<T>(what: &str, result: Result<T>) -> Option<T> {
    match result {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!(part = what, error = %e, "skipping part of the request details");
            None
        }
    }
}

/// A blocking client for one project's pull requests (GitHub) or merge requests (GitLab).
///
/// Every call is a synchronous HTTPS round trip (10 s to connect, 30 s in total), meant for a
/// background thread. The token is sent only as an `Authorization: Bearer` header to the API of
/// the project's own host, never in a URL, a log line or this type's `Debug` output (which also
/// hides credentials embedded in a configured API root).
pub struct Client {
    http: Http,
    forge: Forge,
    kind: ForgeKind,
    /// `/repos/owner/repo` or `/projects/group%2Fproject`.
    project: String,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("kind", &self.kind)
            .field("api_base", &redact_userinfo(self.http.base()))
            .field("project", &self.project)
            .field("has_token", &self.http.has_token())
            .finish_non_exhaustive()
    }
}

/// The REST API root for `forge`: `https://api.github.com` for github.com (`/api/v3` on any
/// other GitHub host), `https://<host>[:<port>]/api/v4` for GitLab. The port is the forge's web
/// port ([`Forge::port`]), so a self-hosted GitLab at `https://gitlab.corp:8443` is called on
/// 8443.
fn default_api_base(forge: &Forge) -> String {
    match forge.kind {
        ForgeKind::GitHub if forge.host == "github.com" => "https://api.github.com".to_owned(),
        ForgeKind::GitHub => format!("https://{}/api/v3", forge.authority()),
        ForgeKind::GitLab => format!("https://{}/api/v4", forge.authority()),
    }
}

impl Client {
    /// A client for `forge`'s project on its own host. `token` is the user's personal access
    /// token; without one only public projects can be read and nothing can be changed.
    pub fn new(forge: &Forge, token: Option<String>) -> Self {
        Self::with_api_base(forge, token, &default_api_base(forge))
    }

    /// Like [`Client::new`] with an explicit API root such as `http://127.0.0.1:8080` (tests)
    /// or `https://gitlab.example.com/api/v4`.
    pub fn with_api_base(forge: &Forge, token: Option<String>, api_base: &str) -> Self {
        let project = match forge.kind {
            ForgeKind::GitHub => github::repo_path(&forge.path),
            ForgeKind::GitLab => gitlab::project_path(&forge.path),
        };
        Self {
            http: Http::new(forge.kind, api_base, token),
            forge: forge.clone(),
            kind: forge.kind,
            project,
        }
    }

    /// Sends requests only while `on` holds `true`. Once it is `false`, every call fails with
    /// [`crate::Error::Stopped`] before anything is sent, including the parts of a
    /// [`Client::details`] still to come. The hosting thread passes the switch its tab turns off
    /// when it lets the thread go (pull and merge requests turned off, the token changed, another
    /// remote), so a job already under way sends nothing more.
    #[must_use]
    pub fn with_switch(mut self, on: Arc<AtomicBool>) -> Self {
        self.http.set_switch(on);
        self
    }

    /// Which provider this client talks to.
    pub fn kind(&self) -> ForgeKind {
        self.kind
    }

    /// Whether a (non-empty) token is set.
    pub fn has_token(&self) -> bool {
        self.http.has_token()
    }

    /// Resolves a commit author's account picture on this project's host. GitHub receives the
    /// commit id; GitLab receives the author's email. Downloads never carry the hosting token.
    pub fn author_avatar(
        &self,
        commit: &str,
        email: &str,
    ) -> Result<Option<crate::avatars::Avatar>> {
        crate::avatars::lookup(&self.http, &self.forge, &self.project, commit, email)
    }

    /// Whether the host shows this repository, to tell apart why [`Client::author_avatar`] failed
    /// with [`crate::Error::NotFound`] on GitHub: `Ok` means the host lacks the commit, an error
    /// says what is wrong with the repository or the token. A separate request, so the caller can
    /// check its off switch and request budget first. GitLab sends nothing and answers
    /// [`crate::Error::NotFound`].
    pub fn author_repository(&self) -> Result<()> {
        crate::avatars::repository(&self.http, self.kind, &self.project)
    }

    /// The login (GitHub) or username (GitLab) the token belongs to. Fails with
    /// [`crate::Error::Unauthorized`] without a valid token.
    pub fn viewer(&self) -> Result<String> {
        match self.kind {
            ForgeKind::GitHub => github::viewer(&self.http),
            ForgeKind::GitLab => gitlab::viewer(&self.http),
        }
    }

    /// Up to `limit` requests matching `filter`, most recently updated first.
    pub fn list(&self, filter: StateFilter, limit: usize) -> Result<Vec<RequestSummary>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        match self.kind {
            ForgeKind::GitHub => github::list(&self.http, &self.project, filter, limit),
            ForgeKind::GitLab => gitlab::list(&self.http, &self.project, filter, limit),
        }
    }

    /// Where the checks and reviews of requests `numbers` stand, as `(number, rollup)` pairs for
    /// the requests the project has, in one GraphQL query to the GraphQL endpoint on the API's
    /// own host (`api.github.com/graphql`, `<host>/api/graphql`). It sends the project's path and
    /// the numbers (the first 100), with the token like every other request, and follows no
    /// redirect.
    ///
    /// GitHub: each pull request's status check rollup state, review decision and reviewers'
    /// latest verdicts. GitLab: each merge request's head pipeline status and whether it has an
    /// approval.
    ///
    /// Only with a token: without one it sends nothing and answers no rollups, whatever the
    /// caller checked (GitHub's GraphQL API refuses anonymous calls anyway).
    pub fn rollup(&self, numbers: &[u64]) -> Result<Vec<(u64, crate::model::Rollup)>> {
        if !self.has_token() {
            return Ok(Vec::new());
        }
        match self.kind {
            ForgeKind::GitHub => github::rollup(&self.http, &self.forge.path, numbers),
            ForgeKind::GitLab => gitlab::rollup(&self.http, &self.forge.path, numbers),
        }
    }

    /// One request with its description, reviews, checks and conversation (first 100 of each).
    /// Only the request itself failing is an error; reviews, comments or checks that cannot be
    /// read (for example a token without the checks scope) are logged and left empty.
    pub fn details(&self, number: u64) -> Result<RequestDetails> {
        match self.kind {
            ForgeKind::GitHub => github::details(&self.http, &self.project, number),
            ForgeKind::GitLab => gitlab::details(&self.http, &self.project, number),
        }
    }

    /// Opens a request from `req.source` into `req.target` (both branches of this project).
    pub fn create(&self, req: &NewRequest) -> Result<RequestSummary> {
        match self.kind {
            ForgeKind::GitHub => github::create(&self.http, &self.project, req),
            ForgeKind::GitLab => gitlab::create(&self.http, &self.project, req),
        }
    }

    /// Merges a request. On GitLab, `Merge` and `Rebase` follow the project's merge method
    /// setting, and `Rebase` rebases the source branch first (waiting up to 30 s for GitLab to
    /// finish), then merges the rebased head.
    ///
    /// `head_sha` is the head commit the user reviewed (from [`RequestDetails::head_sha`]). When
    /// given, the provider merges only while the head is still that commit, so a push after the
    /// review makes the merge fail with [`crate::Error::Api`] (status 409, "The pull request
    /// changed since you loaded it. Refresh and review before merging.", or "merge request" on
    /// GitLab) instead of merging commits nobody reviewed. A blank `head_sha` counts as `None`.
    pub fn merge(&self, number: u64, method: MergeMethod, head_sha: Option<&str>) -> Result<()> {
        let head_sha = head_sha.map(str::trim).filter(|s| !s.is_empty());
        match self.kind {
            ForgeKind::GitHub => github::merge(&self.http, &self.project, number, method, head_sha),
            ForgeKind::GitLab => gitlab::merge(&self.http, &self.project, number, method, head_sha),
        }
    }

    /// Closes (`open == false`) or reopens a request.
    pub fn set_open(&self, number: u64, open: bool) -> Result<()> {
        match self.kind {
            ForgeKind::GitHub => github::set_open(&self.http, &self.project, number, open),
            ForgeKind::GitLab => gitlab::set_open(&self.http, &self.project, number, open),
        }
    }

    /// Adds a conversation comment.
    pub fn comment(&self, number: u64, body: &str) -> Result<()> {
        match self.kind {
            ForgeKind::GitHub => github::comment(&self.http, &self.project, number, body),
            ForgeKind::GitLab => gitlab::comment(&self.http, &self.project, number, body),
        }
    }

    /// Approves a request as the token's owner.
    pub fn approve(&self, number: u64) -> Result<()> {
        match self.kind {
            ForgeKind::GitHub => github::approve(&self.http, &self.project, number),
            ForgeKind::GitLab => gitlab::approve(&self.http, &self.project, number),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forge(kind: ForgeKind, host: &str, path: &str) -> Forge {
        Forge {
            kind,
            host: host.into(),
            port: None,
            path: path.into(),
        }
    }

    #[test]
    fn api_bases() {
        assert_eq!(
            default_api_base(&forge(ForgeKind::GitHub, "github.com", "o/r")),
            "https://api.github.com"
        );
        assert_eq!(
            default_api_base(&forge(ForgeKind::GitLab, "gitlab.com", "g/r")),
            "https://gitlab.com/api/v4"
        );
        assert_eq!(
            default_api_base(&forge(ForgeKind::GitLab, "git.example.org", "g/s/r")),
            "https://git.example.org/api/v4"
        );
        let with_port =
            |url: &str| Forge::parse(url, &[]).unwrap_or_else(|| panic!("{url} not parsed"));
        assert_eq!(
            default_api_base(&with_port("https://gitlab.corp:8443/g/r.git")),
            "https://gitlab.corp:8443/api/v4"
        );
        assert_eq!(
            default_api_base(&with_port("https://gitlab.corp:443/g/r.git")),
            "https://gitlab.corp/api/v4"
        );
        // An SSH port is not the web server's port.
        assert_eq!(
            default_api_base(&with_port("ssh://git@gitlab.corp:2222/g/r.git")),
            "https://gitlab.corp/api/v4"
        );
        assert_eq!(
            Client::new(&with_port("https://gitlab.corp:8443/g/r.git"), None)
                .http
                .base(),
            "https://gitlab.corp:8443/api/v4"
        );
    }

    #[test]
    fn timestamps() {
        assert_eq!(unix(Some("2024-01-02T03:04:05Z")), 1_704_164_645);
        assert_eq!(unix(Some("2024-01-02T05:04:05.123+02:00")), 1_704_164_645);
        assert_eq!(unix(Some("yesterday")), 0);
        assert_eq!(unix(None), 0);
    }

    #[test]
    fn debug_hides_token() {
        let c = Client::new(
            &forge(ForgeKind::GitLab, "gitlab.com", "g/r"),
            Some("glpat-secret123".into()),
        );
        let dbg = format!("{c:?}");
        assert!(!dbg.contains("glpat-secret123"), "{dbg}");
        assert!(dbg.contains("has_token: true"), "{dbg}");
        assert!(dbg.contains("g%2Fr"), "{dbg}");

        // Neither the client nor its HTTP layer prints a token, even one embedded in an API
        // root the user configured, or one with surrounding whitespace.
        let c = Client::with_api_base(
            &forge(ForgeKind::GitLab, "gitlab.corp", "g/r"),
            Some("  glpat-secret456\n".into()),
            "https://oauth2:glpat-inurl789@gitlab.corp:8443/api/v4",
        );
        for dbg in [format!("{c:?}"), format!("{c:#?}"), format!("{:?}", c.http)] {
            for secret in ["glpat-secret456", "glpat-inurl789", "oauth2"] {
                assert!(!dbg.contains(secret), "{secret} in {dbg}");
            }
            assert!(dbg.contains("gitlab.corp:8443/api/v4"), "{dbg}");
        }
    }

    #[test]
    fn blank_token_is_no_token() {
        let c = Client::new(
            &forge(ForgeKind::GitHub, "github.com", "o/r"),
            Some("  ".into()),
        );
        assert!(!c.has_token());
    }
}
