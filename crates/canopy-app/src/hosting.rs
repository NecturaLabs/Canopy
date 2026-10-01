//! Pull and merge requests: a background thread per repository tab that talks to the GitHub or
//! GitLab API through `canopy-hosting`, plus token lookup and storage.
//!
//! Tokens come from, in order: the token saved in Canopy's settings for that host, the
//! provider's environment variable (`GH_TOKEN`, `GITLAB_TOKEN`, …), and the user's git
//! credential helper. Saved tokens live in `tokens.json` next to Canopy's other data, readable
//! only by the user on Unix. They are sent only to the host they belong to.

use std::collections::HashMap;
use std::path::PathBuf;

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
            TokenSource::Saved => "saved in Canopy",
            TokenSource::Environment => "from the environment",
            TokenSource::GitCredentials => "from your git credential helper",
        }
    }
}

/// Tokens the user saved in Canopy, by host.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct TokenStore {
    tokens: HashMap<String, String>,
}

impl std::fmt::Debug for TokenStore {
    // Hosts only: a token must never reach a log line.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenStore")
            .field("hosts", &self.tokens.keys().collect::<Vec<_>>())
            .finish()
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
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                tracing::warn!("ignoring unreadable {}: {e}", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
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

    /// The saved token for `host`.
    pub fn get(&self, host: &str) -> Option<&str> {
        self.tokens.get(&host.to_lowercase()).map(String::as_str)
    }

    /// Saves (or with an empty `token`, removes) the token for `host`.
    pub fn set(&mut self, host: &str, token: &str) -> std::io::Result<()> {
        let token = token.trim();
        if token.is_empty() {
            self.tokens.remove(&host.to_lowercase());
        } else {
            self.tokens.insert(host.to_lowercase(), token.to_owned());
        }
        self.save()
    }
}

/// Writes `bytes` to a new file that only the user can read, from the moment it exists (Unix).
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
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
            HostJob::List(_) => format!("Loading {}s", kind.request_noun()),
            HostJob::Details(i) => format!("Loading {n} #{i}"),
            HostJob::Create(_) => format!("Creating {}", kind.request_noun()),
            HostJob::Merge(i, _, _) => format!("Merging {n} #{i}"),
            HostJob::SetOpen(i, true) => format!("Reopening {n} #{i}"),
            HostJob::SetOpen(i, false) => format!("Closing {n} #{i}"),
            HostJob::Comment(i, _) => format!("Commenting on {n} #{i}"),
            HostJob::Approve(i) => format!("Approving {n} #{i}"),
        }
    }
}

/// What the hosting thread reports back.
#[derive(Debug)]
pub enum HostEvent {
    Token(Option<TokenSource>),
    List(StateFilter, Result<Vec<RequestSummary>, String>),
    Details(u64, Box<Result<RequestDetails, String>>),
    Created(Result<RequestSummary, String>),
    /// An action finished; the list and the request should be reloaded.
    Done {
        label: String,
        number: Option<u64>,
        result: Result<(), String>,
    },
}

/// The UI's handle on a hosting thread for one forge.
#[derive(Debug)]
pub struct HostWorker {
    pub forge: Forge,
    jobs: Sender<HostJob>,
    pub events: Receiver<HostEvent>,
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
        let spawned = std::thread::Builder::new()
            .name(format!("hosting:{}", forge.host))
            .spawn(move || run(thread_forge, saved, &trusted_gitlab_hosts, job_rx, tx, ctx));
        if let Err(e) = &spawned {
            tracing::error!("could not start the hosting thread: {e}");
        }
        Self {
            forge,
            jobs,
            events,
        }
    }

    pub fn send(&self, job: HostJob) {
        let _ = self.jobs.send(job);
    }
}

fn run(
    forge: Forge,
    saved: Option<String>,
    trusted_gitlab_hosts: &[String],
    jobs: Receiver<HostJob>,
    events: Sender<HostEvent>,
    ctx: egui::Context,
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
    let client = match std::env::var("CANOPY_HOSTING_API_BASE") {
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
    while let Ok(job) = jobs.recv() {
        let label = job.label(forge.kind);
        tracing::debug!(host = %forge.host, job = %label, "hosting job");
        let s = |e: canopy_hosting::Error| e.to_string();
        match job {
            HostJob::List(filter) => {
                emit(HostEvent::List(filter, client.list(filter, 100).map_err(s)))
            }
            HostJob::Details(n) => emit(HostEvent::Details(
                n,
                Box::new(client.details(n).map_err(s)),
            )),
            HostJob::Create(req) => emit(HostEvent::Created(client.create(&req).map_err(s))),
            HostJob::Merge(n, method, head) => emit(HostEvent::Done {
                label,
                number: Some(n),
                result: client.merge(n, method, head.as_deref()).map_err(s),
            }),
            HostJob::SetOpen(n, open) => emit(HostEvent::Done {
                label,
                number: Some(n),
                result: client.set_open(n, open).map_err(s),
            }),
            HostJob::Comment(n, body) => emit(HostEvent::Done {
                label,
                number: Some(n),
                result: client.comment(n, &body).map_err(s),
            }),
            HostJob::Approve(n) => emit(HostEvent::Done {
                label,
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
