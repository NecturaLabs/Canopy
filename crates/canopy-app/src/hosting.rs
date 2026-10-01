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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenStore {
    tokens: HashMap<String, String>,
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
        std::fs::write(&tmp, json)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, &path)
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

/// Finds a token for `forge` (blocking: may run the git credential helper).
pub fn resolve_token(
    forge: &Forge,
    saved: Option<String>,
    use_git_credentials: bool,
) -> Option<(String, TokenSource)> {
    if let Some(t) = saved.filter(|t| !t.trim().is_empty()) {
        return Some((t, TokenSource::Saved));
    }
    if let Some(t) = canopy_hosting::token_from_env(forge.kind) {
        return Some((t, TokenSource::Environment));
    }
    if use_git_credentials && let Some(t) = canopy_hosting::token_from_git_credentials(&forge.host)
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
    Merge(u64, MergeMethod),
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
            HostJob::Merge(i, _) => format!("Merging {n} #{i}"),
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
    pub fn spawn(forge: Forge, saved: Option<String>, ctx: egui::Context) -> Self {
        let (jobs, job_rx) = unbounded::<HostJob>();
        let (tx, events) = unbounded::<HostEvent>();
        let thread_forge = forge.clone();
        let _ = std::thread::Builder::new()
            .name(format!("hosting:{}", forge.host))
            .spawn(move || run(thread_forge, saved, job_rx, tx, ctx));
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
    jobs: Receiver<HostJob>,
    events: Sender<HostEvent>,
    ctx: egui::Context,
) {
    let emit = |e: HostEvent| {
        let _ = events.send(e);
        ctx.request_repaint();
    };
    // The credential helper can be slow; resolve once, lazily, on this thread.
    let token = resolve_token(&forge, saved, true);
    emit(HostEvent::Token(token.as_ref().map(|t| t.1)));
    let token = token.map(|t| t.0);
    // Development and screenshots: talk to a local stand-in (tools/mock-hosting.py). Loopback
    // only, so the variable can never send a token anywhere else.
    let client = match std::env::var("CANOPY_HOSTING_API_BASE") {
        Ok(base)
            if base.starts_with("http://127.0.0.1:") || base.starts_with("http://localhost:") =>
        {
            Client::with_api_base(&forge, token, base.trim())
        }
        _ => Client::new(&forge, token),
    };
    while let Ok(job) = jobs.recv() {
        let label = job.label(forge.kind);
        tracing::debug!(host = %forge.host, "{label}");
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
            HostJob::Merge(n, method) => emit(HostEvent::Done {
                label,
                number: Some(n),
                result: client.merge(n, method).map_err(s),
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
