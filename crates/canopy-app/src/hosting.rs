//! Pull and merge requests: a background thread per repository tab that talks to the GitHub or
//! GitLab API through `canopy-hosting`, plus token lookup and storage.
//!
//! Tokens come from, in order: the token saved in Canopy's settings for that host (and port, see
//! [`TokenStore`]), the provider's environment variable (`GH_TOKEN`, `GITLAB_TOKEN`, …), and the
//! user's git credential helper. Saved tokens live in `tokens.json` next to Canopy's other data,
//! created readable only by the user (mode 0600) on Unix; they are not in the operating system's
//! keychain. They are sent only to the host they belong to.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crossbeam_channel::{Receiver, Sender, unbounded};
use serde::{Deserialize, Serialize};

use canopy_core::forge::{Forge, ForgeKind};
use canopy_hosting::{
    Client, MergeMethod, NewRequest, RequestDetails, RequestSummary, StateFilter,
};

/// Where a token came from, shown in settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenSource {
    Saved,
    Environment,
    GitCredentials,
}

impl TokenSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Saved => "saved in Canopy",
            Self::Environment => "from the environment",
            Self::GitCredentials => "from your git credential helper",
        }
    }
}

/// Tokens the user saved in Canopy, by URL authority: the host, plus the port when it is not the
/// default (`gitlab.example.com:8443`), so two services on one host never share a token.
///
/// Files written before version 2 keyed tokens by host alone, and such a token was used for
/// every port of its host. Those entries still load and keep that meaning (`legacy_hosts`) until
/// the token for that host is saved or removed again.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct TokenStore {
    tokens: HashMap<String, String>,
    #[serde(default)]
    version: u32,
    /// Hosts whose token was saved before version 2 and still applies to their other ports.
    #[serde(default)]
    legacy_hosts: Vec<String>,
}

/// The current `tokens.json` format.
const TOKENS_VERSION: u32 = 2;

impl std::fmt::Debug for TokenStore {
    // Hosts only: a token must never reach a log line.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenStore")
            .field("hosts", &self.tokens.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

fn tokens_path() -> Option<PathBuf> {
    eframe::storage_dir("canopy").map(|d| d.join("tokens.json"))
}

impl TokenStore {
    /// Loads saved tokens; an unreadable file counts as empty.
    pub fn load() -> Self {
        let Some(path) = tokens_path() else {
            return Self::default();
        };
        match std::fs::read(&path) {
            Ok(bytes) => Self::parse(&bytes).unwrap_or_else(|e| {
                tracing::warn!("ignoring unreadable {}: {e}", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// Reads `tokens.json`, upgrading an older format.
    fn parse(bytes: &[u8]) -> serde_json::Result<Self> {
        let mut store: Self = serde_json::from_slice(bytes)?;
        if store.version < TOKENS_VERSION {
            store.legacy_hosts = store
                .tokens
                .keys()
                .filter(|k| !k.contains(':'))
                .cloned()
                .collect();
            store.version = TOKENS_VERSION;
        }
        Ok(store)
    }

    /// The saved token for `forge`: the one saved for its authority, else, for a forge on a
    /// non-default port, a token its host had before tokens were kept per port.
    pub fn get_for(&self, forge: &Forge) -> Option<&str> {
        let exact = self.get(&forge.authority());
        if exact.is_some() || forge.port.is_none() {
            return exact;
        }
        let host = forge.host.to_lowercase();
        if self.legacy_hosts.contains(&host) {
            return self.get(&host);
        }
        None
    }

    fn save(&self) -> std::io::Result<()> {
        let Some(path) = tokens_path() else {
            return Err(std::io::Error::other("no data directory"));
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        let written = write_private(&tmp, &json).and_then(|()| std::fs::rename(&tmp, &path));
        if written.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        written
    }

    /// The token saved for exactly `authority` (`host` or `host:port`).
    pub fn get(&self, authority: &str) -> Option<&str> {
        self.tokens
            .get(&authority.to_lowercase())
            .map(String::as_str)
    }

    /// Saves (or with an empty `token`, removes) the token for `authority` (`host` or
    /// `host:port`). From then on a host's token applies to that host's default port only.
    pub fn set(&mut self, authority: &str, token: &str) -> std::io::Result<()> {
        self.put(authority, token);
        self.save()
    }

    fn put(&mut self, authority: &str, token: &str) {
        let key = authority.to_lowercase();
        let token = token.trim();
        self.legacy_hosts.retain(|h| *h != key);
        self.version = TOKENS_VERSION;
        if token.is_empty() {
            self.tokens.remove(&key);
        } else {
            self.tokens.insert(key, token.to_owned());
        }
    }
}

/// Writes `bytes` to a new file that only the user can read, from the moment it exists (Unix).
pub(crate) fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let _ = std::fs::remove_file(path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Finds a token for `forge` (blocking: may run the git credential helper). Environment tokens
/// are used only for github.com, gitlab.com, `GITLAB_HOST` and the self-hosted GitLab hosts the
/// user listed (`trusted_gitlab_hosts`), never for a host recognised as GitLab by its name alone.
pub fn resolve_token(
    forge: &Forge,
    saved: Option<String>,
    trusted_gitlab_hosts: &[String],
    use_git_credentials: bool,
) -> Option<(String, TokenSource)> {
    if let Some(t) = saved.filter(|t| !t.trim().is_empty()) {
        return Some((t, TokenSource::Saved));
    }
    if let Some(t) =
        canopy_hosting::token_from_env_for(forge.kind, &forge.host, trusted_gitlab_hosts)
    {
        return Some((t, TokenSource::Environment));
    }
    if use_git_credentials
        && let Some(t) = canopy_hosting::token_from_git_credentials(&forge.authority())
    {
        return Some((t, TokenSource::GitCredentials));
    }
    None
}

/// Where to create a token for `forge`, prefilled with a name and the scopes Canopy needs.
pub fn token_page(kind: ForgeKind, host: &str) -> String {
    match kind {
        ForgeKind::GitHub => {
            "https://github.com/settings/tokens/new?description=Canopy&scopes=repo".into()
        }
        ForgeKind::GitLab => {
            format!("https://{host}/-/user_settings/personal_access_tokens?name=Canopy&scopes=api")
        }
    }
}

/// Work for the hosting thread.
#[derive(Debug, Clone)]
pub enum HostJob {
    Avatar(canopy_core::Oid, String),
    List(StateFilter),
    Details(u64),
    Create(NewRequest),
    /// Merge with this method, only if the head is still this commit.
    Merge(u64, MergeMethod, Option<String>),
    SetOpen(u64, bool),
    Comment(u64, String),
    Approve(u64),
}

impl HostJob {
    fn label(&self, kind: ForgeKind) -> String {
        let n = kind.request_abbrev();
        match self {
            Self::Avatar(..) => "Loading an author picture".into(),
            Self::List(_) => format!("Loading {}s", kind.request_noun()),
            Self::Details(i) => format!("Loading {n} #{i}"),
            Self::Create(_) => format!("Creating {}", kind.request_noun()),
            Self::Merge(i, _, _) => format!("Merging {n} #{i}"),
            Self::SetOpen(i, true) => format!("Reopening {n} #{i}"),
            Self::SetOpen(i, false) => format!("Closing {n} #{i}"),
            Self::Comment(i, _) => format!("Commenting on {n} #{i}"),
            Self::Approve(i) => format!("Approving {n} #{i}"),
        }
    }
}

/// What the hosting thread reports back.
#[derive(Debug)]
pub enum HostEvent {
    Avatar {
        oid: canopy_core::Oid,
        email: String,
        result: crate::avatars::Cached,
    },
    Token(Option<TokenSource>),
    List(StateFilter, Result<Vec<RequestSummary>, String>),
    Details(u64, Box<Result<RequestDetails, String>>),
    Created(Result<RequestSummary, String>),
    /// An action finished; the list and the request should be reloaded.
    Done {
        label: String,
        action: HostAction,
        number: Option<u64>,
        result: Result<(), String>,
    },
}

/// Which pull or merge request action finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostAction {
    Merge,
    SetOpen,
    Comment,
    Approve,
}

/// The UI's handle on a hosting thread for one forge.
#[derive(Debug)]
pub struct HostWorker {
    pub forge: Forge,
    jobs: Sender<HostJob>,
    pub events: Receiver<HostEvent>,
    avatars_enabled: Arc<AtomicBool>,
    ctx: egui::Context,
}

impl HostWorker {
    /// Starts a thread for `forge`. `saved` is the token saved in settings for its host.
    pub fn spawn(
        forge: Forge,
        saved: Option<String>,
        trusted_gitlab_hosts: Vec<String>,
        ctx: egui::Context,
    ) -> Self {
        let (jobs, job_rx) = unbounded::<HostJob>();
        let (tx, events) = unbounded::<HostEvent>();
        let thread_forge = forge.clone();
        let avatars_enabled = Arc::new(AtomicBool::new(false));
        let thread_avatars = Arc::clone(&avatars_enabled);
        let thread_ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("hosting:{}", forge.host))
            .spawn(move || {
                run(
                    thread_forge,
                    saved,
                    &trusted_gitlab_hosts,
                    job_rx,
                    tx,
                    thread_ctx,
                    thread_avatars,
                );
            });
        if let Err(e) = &spawned {
            tracing::error!("could not start the hosting thread: {e}");
        }
        Self {
            forge,
            jobs,
            events,
            avatars_enabled,
            ctx,
        }
    }

    pub fn send(&self, job: HostJob) {
        let _ = self.jobs.send(job);
    }

    pub fn set_avatars_enabled(&self, enabled: bool) {
        self.avatars_enabled.store(enabled, Ordering::SeqCst);
    }

    pub fn avatars_enabled(&self) -> bool {
        self.avatars_enabled.load(Ordering::SeqCst)
    }

    pub fn context(&self) -> &egui::Context {
        &self.ctx
    }
}

impl Drop for HostWorker {
    fn drop(&mut self) {
        self.set_avatars_enabled(false);
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the thread owns its channels and context; dropping them on exit tells the other side"
)]
fn run(
    forge: Forge,
    saved: Option<String>,
    trusted_gitlab_hosts: &[String],
    jobs: Receiver<HostJob>,
    events: Sender<HostEvent>,
    ctx: egui::Context,
    avatars_enabled: Arc<AtomicBool>,
) {
    let emit = |e: HostEvent| {
        let _ = events.send(e);
        ctx.request_repaint();
    };
    // The credential helper can be slow; resolve once, lazily, on this thread.
    let token = resolve_token(&forge, saved, trusted_gitlab_hosts, true);
    emit(HostEvent::Token(token.as_ref().map(|t| t.1)));
    let token = token.map(|t| t.0);
    // Development and screenshots: talk to a local stand-in (tools/mock-hosting.py). Only a
    // plain loopback address is accepted (no user info, no look-alike host names), so the
    // variable cannot send a token anywhere else.
    let build_client = |token: Option<String>| match std::env::var("CANOPY_HOSTING_API_BASE") {
        Ok(base) if canopy_hosting::is_loopback_base(base.trim()) => {
            Client::with_api_base(&forge, token, base.trim())
        }
        Ok(base) => {
            tracing::warn!(
                base = %canopy_core::cli::redact_url(&base),
                "ignoring CANOPY_HOSTING_API_BASE: not a loopback http URL"
            );
            Client::new(&forge, token)
        }
        Err(_) => Client::new(&forge, token),
    };
    let client = build_client(token.clone());
    // Author pictures have a thread of their own: a slow or black-holed picture host (a request
    // can take tens of seconds) must not hold up a pull or merge request action queued behind it.
    let avatar_jobs = {
        let (jobs, job_rx) = unbounded::<(canopy_core::Oid, String)>();
        let (client, forge) = (build_client(token), forge.clone());
        let (events, ctx) = (events.clone(), ctx.clone());
        let spawned = std::thread::Builder::new()
            .name(format!("avatars:{}", forge.host))
            .spawn(move || {
                while let Ok((oid, email)) = job_rx.recv() {
                    let result =
                        crate::avatars::load(&client, &forge, oid, &email, &avatars_enabled);
                    // The author's email stays out of the log; the reason is about the repository.
                    if let Some(crate::avatars::Note::Blocked(why)) = &result.note {
                        tracing::info!(host = %forge.host, reason = ?why, "author pictures stopped for this repository");
                    }
                    let _ = events.send(HostEvent::Avatar { oid, email, result });
                    ctx.request_repaint();
                }
            });
        if let Err(e) = &spawned {
            tracing::error!("could not start the author-picture thread: {e}");
        }
        jobs
    };
    while let Ok(job) = jobs.recv() {
        let label = job.label(forge.kind);
        tracing::debug!(host = %forge.host, job = %label, "hosting job");
        let s = |e: canopy_hosting::Error| e.to_string();
        match job {
            HostJob::Avatar(oid, email) => {
                if let Err(unsent) = avatar_jobs.send((oid, email)) {
                    // No picture thread: end the request so the tab does not wait for it.
                    let (oid, email) = unsent.into_inner();
                    let result = crate::avatars::unavailable(chrono::Utc::now().timestamp());
                    emit(HostEvent::Avatar { oid, email, result });
                }
            }
            HostJob::List(filter) => {
                emit(HostEvent::List(filter, client.list(filter, 100).map_err(s)));
            }
            HostJob::Details(n) => emit(HostEvent::Details(
                n,
                Box::new(client.details(n).map_err(s)),
            )),
            HostJob::Create(req) => emit(HostEvent::Created(client.create(&req).map_err(s))),
            HostJob::Merge(n, method, head) => emit(HostEvent::Done {
                label,
                action: HostAction::Merge,
                number: Some(n),
                result: client.merge(n, method, head.as_deref()).map_err(s),
            }),
            HostJob::SetOpen(n, open) => emit(HostEvent::Done {
                label,
                action: HostAction::SetOpen,
                number: Some(n),
                result: client.set_open(n, open).map_err(s),
            }),
            HostJob::Comment(n, body) => emit(HostEvent::Done {
                label,
                action: HostAction::Comment,
                number: Some(n),
                result: client.comment(n, &body).map_err(s),
            }),
            HostJob::Approve(n) => emit(HostEvent::Done {
                label,
                action: HostAction::Approve,
                number: Some(n),
                result: client.approve(n).map_err(s),
            }),
        }
    }
}

/// Tests a token for `forge` off the UI thread; the result arrives on the returned channel.
pub fn test_token(
    forge: Forge,
    token: String,
    ctx: egui::Context,
) -> Receiver<Result<String, String>> {
    let (tx, rx) = unbounded();
    let _ = std::thread::Builder::new()
        .name("token-test".into())
        .spawn(move || {
            let r = Client::new(&forge, Some(token))
                .viewer()
                .map_err(|e| e.to_string());
            let _ = tx.send(r);
            ctx.request_repaint();
        });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_never_shows_tokens() {
        let mut store = TokenStore::default();
        store
            .tokens
            .insert("github.com".into(), "ghp_secretvalue".into());
        let shown = format!("{store:?}");
        assert!(shown.contains("github.com"));
        assert!(!shown.contains("ghp_secretvalue"));
    }

    fn gitlab(host: &str, port: Option<u16>) -> Forge {
        Forge {
            kind: ForgeKind::GitLab,
            host: host.into(),
            port,
            path: "group/app".into(),
        }
    }

    #[test]
    fn tokens_are_kept_per_port() {
        let mut store = TokenStore::default();
        store.put("gitlab.example.com", "glpat-default");
        store.put("gitlab.example.com:8443", "glpat-other");
        let default_port = gitlab("gitlab.example.com", None);
        let other_port = gitlab("gitlab.example.com", Some(8443));
        assert_eq!(store.get_for(&default_port), Some("glpat-default"));
        assert_eq!(store.get_for(&other_port), Some("glpat-other"));
        // A token for the default port never goes to another port.
        store.put("gitlab.example.com:8443", "");
        assert_eq!(store.get_for(&other_port), None);
        assert_eq!(store.get_for(&default_port), Some("glpat-default"));
        assert_eq!(
            store.get_for(&gitlab("GitLab.Example.com", None)),
            Some("glpat-default")
        );
    }

    #[test]
    fn host_only_tokens_from_older_versions_still_load() -> serde_json::Result<()> {
        let old = br#"{"tokens":{"github.com":"ghp_a","gitlab.example.com":"glpat-b"}}"#;
        let mut store = TokenStore::parse(old)?;
        let github = Forge {
            kind: ForgeKind::GitHub,
            host: "github.com".into(),
            port: None,
            path: "o/r".into(),
        };
        assert_eq!(store.get_for(&github), Some("ghp_a"));
        let other_port = gitlab("gitlab.example.com", Some(8443));
        // Saved when tokens were kept per host: it served every port, and still does…
        assert_eq!(store.get_for(&other_port), Some("glpat-b"));
        // …until the host's token is saved again, which makes it the default port's only.
        store.put("gitlab.example.com", "glpat-new");
        assert_eq!(store.get_for(&other_port), None);
        assert_eq!(
            store.get_for(&gitlab("gitlab.example.com", None)),
            Some("glpat-new")
        );
        // The upgraded format round-trips without turning new keys into legacy ones.
        let saved = serde_json::to_vec(&store)?;
        let reloaded = TokenStore::parse(&saved)?;
        assert_eq!(reloaded.get_for(&other_port), None);
        assert_eq!(reloaded.get_for(&github), Some("ghp_a"));
        Ok(())
    }

    #[test]
    fn private_files_replace_leftovers_and_are_owner_only() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("tokens.json.tmp");
        std::fs::write(&path, b"stale")?;
        write_private(&path, b"{}")?;
        assert_eq!(std::fs::read(&path)?, b"{}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path)?.permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        Ok(())
    }
}
