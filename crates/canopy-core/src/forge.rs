//! Recognising GitHub and GitLab remotes and building their web and clone URLs.
//!
//! Everything here is pure string handling: nothing touches the network. A remote URL is parsed
//! into a [`Forge`] (which provider, which host, which project), and the web pages Canopy opens in
//! the user's browser are derived from it. Credentials embedded in a remote URL
//! (`https://user:token@host/…`) are dropped while parsing and never appear in any URL built
//! here.

/// A supported hosting provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ForgeKind {
    /// github.com.
    GitHub,
    /// gitlab.com or a self-hosted GitLab instance.
    GitLab,
}

impl ForgeKind {
    /// Display name: "GitHub" or "GitLab".
    pub fn name(self) -> &'static str {
        match self {
            Self::GitHub => "GitHub",
            Self::GitLab => "GitLab",
        }
    }

    /// What the provider calls a review request: "pull request" or "merge request".
    pub fn request_noun(self) -> &'static str {
        match self {
            Self::GitHub => "pull request",
            Self::GitLab => "merge request",
        }
    }

    /// Short form of [`Self::request_noun`]: "PR" or "MR".
    pub fn request_abbrev(self) -> &'static str {
        match self {
            Self::GitHub => "PR",
            Self::GitLab => "MR",
        }
    }

    /// The public host: `github.com` or `gitlab.com`.
    pub fn default_host(self) -> &'static str {
        match self {
            Self::GitHub => "github.com",
            Self::GitLab => "gitlab.com",
        }
    }
}

/// A project on a forge, as recognised from a remote URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Forge {
    /// Which provider.
    pub kind: ForgeKind,
    /// Lower-case host name without port or credentials, e.g. `github.com`.
    pub host: String,
    /// Project path without `.git` and without leading or trailing slashes: `owner/repo` on
    /// GitHub, `group/subgroup/repo` on GitLab.
    pub path: String,
}

/// Schemes a forge remote can use. `file://` and anything unknown are rejected.
const SCHEMES: [&str; 6] = ["https", "http", "ssh", "git", "git+ssh", "ssh+git"];

/// Host aliases that mean the canonical public host.
fn canonical_host(host: &str) -> &str {
    match host {
        "www.github.com" | "ssh.github.com" => "github.com",
        "www.gitlab.com" | "altssh.gitlab.com" => "gitlab.com",
        other => other,
    }
}

/// Characters allowed in a host name (DNS names and IPv4 addresses).
fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && !host.starts_with(['.', '-'])
        && !host.ends_with(['.', '-'])
        && host
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
}

/// One path segment of a GitHub or GitLab project: letters, digits, `.`, `_` and `-`.
fn valid_segment(seg: &str) -> bool {
    !seg.is_empty()
        && seg != "."
        && seg != ".."
        && seg
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Strips surrounding slashes and one trailing `.git` from a project path and validates it for
/// `kind`: exactly two segments on GitHub, at least two on GitLab.
fn normalise_path(kind: ForgeKind, raw: &str) -> Option<String> {
    let trimmed = raw.trim_matches('/');
    let trimmed = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    let trimmed = trimmed.trim_end_matches('/');
    let segments: Vec<&str> = trimmed.split('/').collect();
    if !segments.iter().all(|s| valid_segment(s)) {
        return None;
    }
    let ok = match kind {
        ForgeKind::GitHub => segments.len() == 2,
        ForgeKind::GitLab => segments.len() >= 2,
    };
    ok.then(|| segments.join("/"))
}

/// Normalises a user-entered host: drops surrounding whitespace, a scheme, credentials, a path
/// and a port, and lower-cases the rest. Returns `None` when nothing valid remains (including
/// IPv6 literals and names with characters outside `a-z 0-9 . -`).
///
/// `"HTTPS://user@GitLab.Corp:8443/x"` becomes `gitlab.corp`, the same host [`Forge::parse`]
/// stores, so the result can be compared with [`Forge::host`].
pub fn normalise_host(raw: &str) -> Option<String> {
    let mut s = raw.trim();
    if let Some((_, rest)) = s.split_once("://") {
        s = rest;
    }
    let s = s.split('/').next().unwrap_or_default();
    let s = s.rsplit_once('@').map_or(s, |(_, host)| host);
    let s = s.split_once(':').map_or(s, |(host, _)| host);
    let host = s.to_ascii_lowercase();
    valid_host(&host).then_some(host)
}

/// Which forge `host` belongs to, given the user's self-hosted GitLab hosts.
fn kind_of(host: &str, gitlab_hosts: &[String]) -> Option<ForgeKind> {
    if host == "github.com" {
        return Some(ForgeKind::GitHub);
    }
    let first_label = host.split('.').next().unwrap_or_default();
    if host == "gitlab.com"
        || first_label.contains("gitlab")
        || gitlab_hosts
            .iter()
            .filter_map(|h| normalise_host(h))
            .any(|h| h == host)
    {
        return Some(ForgeKind::GitLab);
    }
    None
}

/// Splits a remote URL into `(host, path)`, or `None` for local paths, `file://` and unknown
/// schemes. The host keeps its case; credentials and port are removed.
fn split_url(url: &str) -> Option<(&str, &str)> {
    if let Some((scheme, rest)) = url.split_once("://") {
        let scheme = scheme.to_ascii_lowercase();
        if !SCHEMES.contains(&scheme.as_str()) {
            return None;
        }
        let (authority, path) = rest.split_once('/')?;
        let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        let host = match host_port.split_once(':') {
            Some((host, port)) => {
                if !port.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                host
            }
            None => host_port,
        };
        return Some((host, path));
    }
    // scp-like syntax `[user@]host:path`. As in git, it only counts when the colon comes before
    // any slash; otherwise the string is a local path.
    let colon = url.find(':')?;
    if url[..colon].contains('/') {
        return None;
    }
    let (user_host, path) = (&url[..colon], &url[colon + 1..]);
    let host = user_host.rsplit_once('@').map_or(user_host, |(_, h)| h);
    // A single letter before the colon is a Windows drive (`C:/repo`), which git treats as local.
    if host.len() == 1 {
        return None;
    }
    Some((host, path))
}

/// Percent-encodes `s`, keeping RFC 3986 unreserved characters and, when `keep_slash`, `/`.
fn encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'.' | b'_' | b'~')
            || (keep_slash && b == b'/')
        {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Encodes a branch, tag, revision or file path for use in a URL path (keeps `/`).
fn path_part(s: &str) -> String {
    encode(s, true)
}

/// Encodes a query parameter value (encodes `/` too).
fn query_value(s: &str) -> String {
    encode(s, false)
}

impl Forge {
    /// Recognises a GitHub or GitLab remote URL: `https://`, `http://`, `ssh://` (with or without
    /// a port), `git://` and scp-like `git@host:path`, with or without credentials, a trailing
    /// `.git` or a trailing slash. `gitlab_hosts` lists self-hosted GitLab instances; any host
    /// whose first label contains `gitlab` is recognised without being listed. Returns `None`
    /// for local paths, `file://` URLs, other providers and malformed project paths.
    pub fn parse(url: &str, gitlab_hosts: &[String]) -> Option<Forge> {
        let url = url.trim();
        if url.is_empty() || url.chars().any(char::is_whitespace) || url.contains('\\') {
            return None;
        }
        let (host, path) = split_url(url)?;
        let host = host.to_ascii_lowercase();
        if !valid_host(&host) {
            return None;
        }
        let host = canonical_host(&host).to_owned();
        let kind = kind_of(&host, gitlab_hosts)?;
        let path = normalise_path(kind, path)?;
        Some(Forge { kind, host, path })
    }

    /// The project's home page: `https://host/path`.
    pub fn web_url(&self) -> String {
        format!("https://{}/{}", self.host, self.path)
    }

    /// GitLab puts project sub-pages under `/-/`.
    fn page(&self, rest: &str) -> String {
        match self.kind {
            ForgeKind::GitHub => format!("{}/{rest}", self.web_url()),
            ForgeKind::GitLab => format!("{}/-/{rest}", self.web_url()),
        }
    }

    /// The page of one commit.
    pub fn commit_url(&self, sha: &str) -> String {
        self.page(&format!("commit/{}", query_value(sha)))
    }

    /// The file tree of a branch.
    pub fn branch_url(&self, branch: &str) -> String {
        self.page(&format!("tree/{}", path_part(branch)))
    }

    /// A tag: the release page on GitHub, the tag page on GitLab.
    pub fn tag_url(&self, tag: &str) -> String {
        match self.kind {
            ForgeKind::GitHub => self.page(&format!("releases/tag/{}", path_part(tag))),
            ForgeKind::GitLab => self.page(&format!("tags/{}", path_part(tag))),
        }
    }

    /// One file at a revision (commit, branch or tag).
    pub fn file_url(&self, rev: &str, path: &str) -> String {
        let path = path.trim_start_matches('/');
        self.page(&format!("blob/{}/{}", path_part(rev), path_part(path)))
    }

    /// The page that opens a new pull or merge request from `source`, against `target` or the
    /// project's default branch.
    pub fn new_request_url(&self, source: &str, target: Option<&str>) -> String {
        match self.kind {
            ForgeKind::GitHub => {
                let range = match target {
                    Some(t) => format!("{}...{}", path_part(t), path_part(source)),
                    None => path_part(source),
                };
                self.page(&format!("compare/{range}?expand=1"))
            }
            ForgeKind::GitLab => {
                let mut q = format!(
                    "merge_requests/new?merge_request%5Bsource_branch%5D={}",
                    query_value(source)
                );
                if let Some(t) = target {
                    q.push_str("&merge_request%5Btarget_branch%5D=");
                    q.push_str(&query_value(t));
                }
                self.page(&q)
            }
        }
    }

    /// `https://host/path.git`.
    pub fn https_clone_url(&self) -> String {
        format!("https://{}/{}.git", self.host, self.path)
    }

    /// `git@host:path.git`.
    pub fn ssh_clone_url(&self) -> String {
        format!("git@{}:{}.git", self.host, self.path)
    }
}

/// Builds a clone URL for `project` (`owner/repo`, or `group/sub/repo` on GitLab) on `host`
/// (the provider's public host when `None` or empty). Surrounding slashes and a trailing `.git`
/// in `project` are tolerated. Returns `None` when the host or the project path is invalid.
pub fn clone_url(kind: ForgeKind, host: Option<&str>, project: &str, ssh: bool) -> Option<String> {
    let host = match host.map(str::trim).filter(|h| !h.is_empty()) {
        Some(h) => canonical_host(&normalise_host(h)?).to_owned(),
        None => kind.default_host().to_owned(),
    };
    let project = project.trim();
    if project.chars().any(char::is_whitespace) {
        return None;
    }
    let path = normalise_path(kind, project)?;
    let forge = Forge { kind, host, path };
    Some(if ssh {
        forge.ssh_clone_url()
    } else {
        forge.https_clone_url()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use ForgeKind::{GitHub, GitLab};

    fn hosts(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn kind_names() {
        assert_eq!(GitHub.name(), "GitHub");
        assert_eq!(GitLab.name(), "GitLab");
        assert_eq!(GitHub.request_noun(), "pull request");
        assert_eq!(GitLab.request_noun(), "merge request");
        assert_eq!(GitHub.request_abbrev(), "PR");
        assert_eq!(GitLab.request_abbrev(), "MR");
        assert_eq!(GitHub.default_host(), "github.com");
        assert_eq!(GitLab.default_host(), "gitlab.com");
    }

    #[test]
    fn parses_supported_urls() {
        let custom = hosts(&["  HTTPS://Code.Example.NET:8443/  ", "git.corp"]);
        let cases: &[(&str, ForgeKind, &str, &str)] = &[
            // GitHub, every transport.
            (
                "https://github.com/owner/repo.git",
                GitHub,
                "github.com",
                "owner/repo",
            ),
            (
                "https://github.com/owner/repo",
                GitHub,
                "github.com",
                "owner/repo",
            ),
            (
                "https://github.com/owner/repo/",
                GitHub,
                "github.com",
                "owner/repo",
            ),
            (
                "https://github.com/owner/repo.git/",
                GitHub,
                "github.com",
                "owner/repo",
            ),
            (
                "http://github.com/owner/repo",
                GitHub,
                "github.com",
                "owner/repo",
            ),
            (
                "HTTPS://GitHub.COM/Owner/Repo.git",
                GitHub,
                "github.com",
                "Owner/Repo",
            ),
            (
                "https://user@github.com/owner/repo.git",
                GitHub,
                "github.com",
                "owner/repo",
            ),
            (
                "https://user:ghp_secret@github.com/owner/repo.git",
                GitHub,
                "github.com",
                "owner/repo",
            ),
            (
                "https://www.github.com/owner/repo",
                GitHub,
                "github.com",
                "owner/repo",
            ),
            (
                "git@github.com:owner/repo.git",
                GitHub,
                "github.com",
                "owner/repo",
            ),
            (
                "git@github.com:owner/repo",
                GitHub,
                "github.com",
                "owner/repo",
            ),
            (
                "git@github.com:/owner/repo.git",
                GitHub,
                "github.com",
                "owner/repo",
            ),
            (
                "github.com:owner/repo.git",
                GitHub,
                "github.com",
                "owner/repo",
            ),
            (
                "ssh://git@github.com/owner/repo.git",
                GitHub,
                "github.com",
                "owner/repo",
            ),
            (
                "ssh://git@ssh.github.com:443/owner/repo.git",
                GitHub,
                "github.com",
                "owner/repo",
            ),
            (
                "git+ssh://git@github.com/owner/repo.git",
                GitHub,
                "github.com",
                "owner/repo",
            ),
            (
                "git://github.com/owner/repo.git",
                GitHub,
                "github.com",
                "owner/repo",
            ),
            (
                "  https://github.com/owner/repo.git\n",
                GitHub,
                "github.com",
                "owner/repo",
            ),
            (
                "https://github.com/o-w_n.er/.github",
                GitHub,
                "github.com",
                "o-w_n.er/.github",
            ),
            // GitLab.com with subgroups.
            (
                "https://gitlab.com/group/repo.git",
                GitLab,
                "gitlab.com",
                "group/repo",
            ),
            (
                "https://gitlab.com/group/sub/deeper/repo.git",
                GitLab,
                "gitlab.com",
                "group/sub/deeper/repo",
            ),
            (
                "git@gitlab.com:group/sub/repo.git",
                GitLab,
                "gitlab.com",
                "group/sub/repo",
            ),
            (
                "ssh://git@altssh.gitlab.com:443/group/repo.git",
                GitLab,
                "gitlab.com",
                "group/repo",
            ),
            (
                "https://oauth2:glpat-xyz@gitlab.com/group/repo",
                GitLab,
                "gitlab.com",
                "group/repo",
            ),
            // Self-hosted GitLab recognised by its first label.
            (
                "https://gitlab.example.org/team/app.git",
                GitLab,
                "gitlab.example.org",
                "team/app",
            ),
            (
                "git@my-gitlab.corp:team/app.git",
                GitLab,
                "my-gitlab.corp",
                "team/app",
            ),
            (
                "ssh://git@GITLAB.Example.org:2222/team/sub/app.git",
                GitLab,
                "gitlab.example.org",
                "team/sub/app",
            ),
            ("https://gitlab/team/app", GitLab, "gitlab", "team/app"),
            // Self-hosted GitLab from settings, entries normalised.
            (
                "https://code.example.net/team/app.git",
                GitLab,
                "code.example.net",
                "team/app",
            ),
            (
                "git@CODE.example.net:team/app.git",
                GitLab,
                "code.example.net",
                "team/app",
            ),
            (
                "ssh://git@git.corp:7999/a/b/c.git",
                GitLab,
                "git.corp",
                "a/b/c",
            ),
        ];
        for (url, kind, host, path) in cases {
            let forge = Forge::parse(url, &custom).unwrap_or_else(|| panic!("{url} not parsed"));
            assert_eq!(
                forge,
                Forge {
                    kind: *kind,
                    host: (*host).to_owned(),
                    path: (*path).to_owned()
                },
                "{url}"
            );
        }
    }

    #[test]
    fn rejects_everything_else() {
        let custom = hosts(&["code.example.net"]);
        let cases = [
            "",
            "   ",
            "/home/me/repo",
            "/home/me/repo.git",
            "./repo",
            "../repo",
            "repo",
            "C:/Users/me/repo",
            "C:\\Users\\me\\repo",
            "\\\\server\\share\\repo",
            "file:///home/me/repo.git",
            "file://github.com/owner/repo.git",
            "ftp://github.com/owner/repo.git",
            "https://bitbucket.org/owner/repo.git",
            "git@bitbucket.org:owner/repo.git",
            "https://codeberg.org/owner/repo.git",
            "https://github.example.com/owner/repo.git",
            "https://notgithub.com/owner/repo.git",
            "https://example.com/gitlab/repo.git",
            // Malformed GitHub paths: exactly owner/repo.
            "https://github.com/owner",
            "https://github.com/owner/",
            "https://github.com/",
            "https://github.com",
            "https://github.com/owner/repo/tree/main",
            "https://github.com/owner//repo",
            "https://github.com/owner/re po",
            "https://github.com/owner/repo?tab=readme",
            "https://github.com/owner/repo#readme",
            "https://github.com/owner/%72epo",
            "https://github.com/../repo",
            "git@github.com:owner",
            "git@github.com:",
            "ssh://git@github.com:notaport/owner/repo.git",
            "ssh://git@github.com/~owner/repo.git",
            // Malformed GitLab paths: at least two segments.
            "https://gitlab.com/group",
            "https://gitlab.com/group//repo",
            "https://code.example.net/onlyone.git",
            // Bad hosts.
            "https://-gitlab.com/group/repo",
            "https://git_lab.com/group/repo",
            "https://[::1]/group/repo",
        ];
        for url in cases {
            assert_eq!(Forge::parse(url, &custom), None, "{url}");
        }
    }

    #[test]
    fn never_keeps_credentials() {
        let f = Forge::parse("https://me:s3cr3t@github.com/owner/repo.git", &[]).expect("parsed");
        for url in [
            f.web_url(),
            f.commit_url("abc"),
            f.branch_url("main"),
            f.tag_url("v1"),
            f.file_url("main", "a.txt"),
            f.new_request_url("topic", Some("main")),
            f.https_clone_url(),
            f.ssh_clone_url(),
        ] {
            assert!(!url.contains("s3cr3t") && !url.contains("me@"), "{url}");
        }
    }

    fn gh() -> Forge {
        Forge {
            kind: GitHub,
            host: "github.com".into(),
            path: "owner/repo".into(),
        }
    }

    fn gl() -> Forge {
        Forge {
            kind: GitLab,
            host: "gitlab.example.org".into(),
            path: "group/sub/repo".into(),
        }
    }

    #[test]
    fn builds_web_urls() {
        let (gh, gl) = (gh(), gl());
        let cases: Vec<(String, &str)> = vec![
            (gh.web_url(), "https://github.com/owner/repo"),
            (gl.web_url(), "https://gitlab.example.org/group/sub/repo"),
            (
                gh.commit_url("0123abc"),
                "https://github.com/owner/repo/commit/0123abc",
            ),
            (
                gl.commit_url("0123abc"),
                "https://gitlab.example.org/group/sub/repo/-/commit/0123abc",
            ),
            (
                gh.branch_url("main"),
                "https://github.com/owner/repo/tree/main",
            ),
            (
                gh.branch_url("feature/login"),
                "https://github.com/owner/repo/tree/feature/login",
            ),
            (
                gl.branch_url("feature/login"),
                "https://gitlab.example.org/group/sub/repo/-/tree/feature/login",
            ),
            (
                gh.branch_url("a b#c?d%e"),
                "https://github.com/owner/repo/tree/a%20b%23c%3Fd%25e",
            ),
            (
                gh.branch_url("fix-1.2_x~y"),
                "https://github.com/owner/repo/tree/fix-1.2_x~y",
            ),
            (
                gh.branch_url("café"),
                "https://github.com/owner/repo/tree/caf%C3%A9",
            ),
            (
                gh.branch_url("a+b@c"),
                "https://github.com/owner/repo/tree/a%2Bb%40c",
            ),
            (
                gh.tag_url("v1.0.0"),
                "https://github.com/owner/repo/releases/tag/v1.0.0",
            ),
            (
                gl.tag_url("v1.0.0"),
                "https://gitlab.example.org/group/sub/repo/-/tags/v1.0.0",
            ),
            (
                gh.tag_url("release/2024 q1"),
                "https://github.com/owner/repo/releases/tag/release/2024%20q1",
            ),
            (
                gh.file_url("main", "src/lib.rs"),
                "https://github.com/owner/repo/blob/main/src/lib.rs",
            ),
            (
                gl.file_url("0123abc", "docs/read me.md"),
                "https://gitlab.example.org/group/sub/repo/-/blob/0123abc/docs/read%20me.md",
            ),
            (
                gh.file_url("feature/x", "/a#b.txt"),
                "https://github.com/owner/repo/blob/feature/x/a%23b.txt",
            ),
            (gh.https_clone_url(), "https://github.com/owner/repo.git"),
            (gh.ssh_clone_url(), "git@github.com:owner/repo.git"),
            (
                gl.https_clone_url(),
                "https://gitlab.example.org/group/sub/repo.git",
            ),
            (
                gl.ssh_clone_url(),
                "git@gitlab.example.org:group/sub/repo.git",
            ),
        ];
        for (got, want) in cases {
            assert_eq!(got, want);
        }
    }

    #[test]
    fn builds_request_urls() {
        let (gh, gl) = (gh(), gl());
        let cases: Vec<(String, &str)> = vec![
            (
                gh.new_request_url("topic", Some("main")),
                "https://github.com/owner/repo/compare/main...topic?expand=1",
            ),
            (
                gh.new_request_url("feature/x y", None),
                "https://github.com/owner/repo/compare/feature/x%20y?expand=1",
            ),
            (
                gh.new_request_url("feat/a", Some("release/1.x")),
                "https://github.com/owner/repo/compare/release/1.x...feat/a?expand=1",
            ),
            (
                gl.new_request_url("topic", None),
                "https://gitlab.example.org/group/sub/repo/-/merge_requests/new?merge_request%5Bsource_branch%5D=topic",
            ),
            (
                gl.new_request_url("feature/x&y=1", Some("main")),
                "https://gitlab.example.org/group/sub/repo/-/merge_requests/new?merge_request%5Bsource_branch%5D=feature%2Fx%26y%3D1&merge_request%5Btarget_branch%5D=main",
            ),
        ];
        for (got, want) in cases {
            assert_eq!(got, want);
        }
    }

    /// `(kind, host, project, ssh, expected URL)`.
    type CloneCase<'a> = (ForgeKind, Option<&'a str>, &'a str, bool, Option<&'a str>);

    #[test]
    fn builds_clone_urls() {
        let cases: &[CloneCase<'_>] = &[
            (
                GitHub,
                None,
                "owner/repo",
                false,
                Some("https://github.com/owner/repo.git"),
            ),
            (
                GitHub,
                None,
                "owner/repo",
                true,
                Some("git@github.com:owner/repo.git"),
            ),
            (
                GitHub,
                Some(""),
                " /owner/repo.git/ ",
                false,
                Some("https://github.com/owner/repo.git"),
            ),
            (
                GitHub,
                Some("  "),
                "owner/repo/",
                true,
                Some("git@github.com:owner/repo.git"),
            ),
            (
                GitHub,
                Some("WWW.GitHub.com"),
                "owner/repo",
                false,
                Some("https://github.com/owner/repo.git"),
            ),
            (
                GitLab,
                None,
                "group/sub/repo",
                false,
                Some("https://gitlab.com/group/sub/repo.git"),
            ),
            (
                GitLab,
                None,
                "group/repo",
                true,
                Some("git@gitlab.com:group/repo.git"),
            ),
            (
                GitLab,
                Some("GitLab.Example.org"),
                "team/app",
                true,
                Some("git@gitlab.example.org:team/app.git"),
            ),
            (
                GitLab,
                Some("https://user:pw@git.corp:8443/x"),
                "team/app",
                false,
                Some("https://git.corp/team/app.git"),
            ),
            // Invalid projects.
            (GitHub, None, "", false, None),
            (GitHub, None, "owner", false, None),
            (GitHub, None, "owner/repo/extra", false, None),
            (GitHub, None, "owner//repo", false, None),
            (GitHub, None, "owner/re po", false, None),
            (GitHub, None, "owner /repo", false, None),
            (GitHub, None, "https://github.com/owner/repo", false, None),
            (GitHub, None, "git@github.com:owner/repo.git", true, None),
            (GitLab, None, "group", false, None),
            (GitLab, None, "/group/", false, None),
            (GitLab, None, "group/../repo", false, None),
            // Invalid hosts.
            (GitLab, Some("bad host"), "team/app", false, None),
            (GitLab, Some("https://"), "team/app", false, None),
        ];
        for (kind, host, project, ssh, want) in cases {
            assert_eq!(
                clone_url(*kind, *host, project, *ssh).as_deref(),
                *want,
                "{kind:?} {host:?} {project:?} ssh={ssh}"
            );
        }
    }

    #[test]
    fn normalises_hosts() {
        let cases: &[(&str, Option<&str>)] = &[
            ("gitlab.com", Some("gitlab.com")),
            ("  GitLab.Corp  ", Some("gitlab.corp")),
            ("https://gitlab.corp", Some("gitlab.corp")),
            ("HTTPS://user:pw@GitLab.Corp:8443/x/y", Some("gitlab.corp")),
            ("gitlab.corp:8443", Some("gitlab.corp")),
            ("", None),
            ("https://", None),
            ("bad host", None),
            ("[::1]", None),
        ];
        for (raw, want) in cases {
            assert_eq!(normalise_host(raw).as_deref(), *want, "{raw:?}");
        }
    }

    #[test]
    fn clone_urls_round_trip_through_parse() {
        for ssh in [false, true] {
            for (kind, project) in [(GitHub, "owner/repo"), (GitLab, "a/b/c")] {
                let url = clone_url(kind, None, project, ssh).expect("url");
                let forge = Forge::parse(&url, &[]).expect("parsed");
                assert_eq!((forge.kind, forge.path.as_str()), (kind, project), "{url}");
            }
        }
    }
}
