//! Pull and merge requests: a background thread per repository tab that talks to the GitHub or
//! GitLab API through `canopy-hosting`, plus the lookup of the credential.
//!
//! The only credential is the sign-in of the provider's CLI (`gh`, `glab`; ADR 0014): its token is
//! read when the thread starts, held in memory and sent only to the host it belongs to. Canopy
//! saves no token and takes none from the environment or git's credential helpers.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, unbounded};

use canopy_core::forge::{Forge, ForgeKind};
use canopy_hosting::{
    CliAccount, CliError, Client, MergeMethod, NewRequest, RequestDetails, RequestQuery,
    RequestSummary, StateFilter,
};

/// Deletes what builds before ADR 0014 saved: a `tokens.json` of plaintext tokens (and a leftover
/// temporary copy). Best effort and off the calling thread; nothing reads those files any more.
pub fn remove_saved_tokens() {
    let Some(dir) = eframe::storage_dir("canopy") else {
        return;
    };
    let spawned = std::thread::Builder::new()
        .name("tokens-cleanup".into())
        .spawn(move || remove_saved_tokens_in(&dir));
    if let Err(e) = spawned {
        tracing::warn!("could not start the removal of the old saved tokens: {e}");
    }
}

fn remove_saved_tokens_in(dir: &std::path::Path) {
    for name in ["tokens.json", "tokens.json.tmp"] {
        let path = dir.join(name);
        match std::fs::remove_file(&path) {
            Ok(()) => tracing::info!("removed {}: Canopy no longer saves tokens", path.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!("could not remove {}: {e}", path.display()),
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

/// What the provider's CLI says about its sign-in to one host: the account and scopes it is
/// signed in with, or why that sign-in cannot be used (not installed, signed out, a scope missing,
/// a failure). The error's words say what to do.
pub type CliState = Result<CliAccount, CliError>;

/// The CLI's token, if it gave one, and what it said.
pub struct Resolved {
    pub token: Option<String>,
    pub cli: CliState,
}

impl std::fmt::Debug for Resolved {
    // Whether there is a token only: a token must never reach a log line.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resolved")
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .field("cli", &self.cli)
            .finish()
    }
}

impl From<Result<canopy_hosting::CliLogin, CliError>> for Resolved {
    fn from(login: Result<canopy_hosting::CliLogin, CliError>) -> Self {
        match login {
            Ok(login) => {
                let (token, account) = login.into_parts();
                Self {
                    token: Some(token),
                    cli: Ok(account),
                }
            }
            Err(e) => Self {
                token: None,
                cli: Err(e),
            },
        }
    }
}

/// The account `--demo-requests` and tests show a CLI signed in as, as they have no real CLI.
pub fn demo_account() -> CliAccount {
    CliAccount {
        user: Some("octocat".into()),
        scopes: None,
    }
}

/// A CLI that is installed and not signed in to `host`, for the same uses.
pub fn signed_out(kind: ForgeKind, host: &str) -> CliState {
    Err(CliError::SignedOut {
        kind,
        host: host.into(),
    })
}

/// Asks the provider's CLI for its sign-in to `forge`'s host and port (blocking: runs the CLI, up
/// to ten seconds). This is the only credential source: nothing saved in Canopy, in the
/// environment or in git's credential helpers is used.
pub fn resolve_token(forge: &Forge) -> Resolved {
    canopy_hosting::login_from_cli(forge.kind, &forge.authority()).into()
}

/// [`resolve_token`] on its own thread, for the Accounts page, which has no hosting thread for
/// a host without an open tab. The token is dropped unread; the result arrives on the channel.
pub fn check_cli(forge: Forge, ctx: egui::Context) -> Receiver<CliState> {
    let (tx, rx) = unbounded();
    let spawned = std::thread::Builder::new()
        .name(format!("cli-check:{}", forge.host))
        .spawn(move || {
            let _ = tx.send(resolve_token(&forge).cli);
            ctx.request_repaint();
        });
    if let Err(e) = spawned {
        tracing::error!("could not start the sign-in check: {e}");
    }
    rx
}

/// Which list a job or an event is about: the state tab and what the host is asked to search for.
/// A result for another key than the tab's is out of date and dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListKey {
    pub state: StateFilter,
    pub query: RequestQuery,
}

impl From<StateFilter> for ListKey {
    fn from(state: StateFilter) -> Self {
        Self {
            state,
            query: RequestQuery::default(),
        }
    }
}

/// Work for the hosting thread.
#[derive(Debug, Clone)]
pub enum HostJob {
    Avatar(canopy_core::Oid, String),
    List(ListKey),
    /// Page (2 or later) of the list for this key: the pages already loaded stay as they are.
    /// GitHub's search continues from the cursor the page before reported.
    ListMore(ListKey, usize, Option<String>),
    /// Request number and the tab's monotonically changing details generation.
    Details(u64, u64),
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
            Self::ListMore(..) => format!("Loading more {}s", kind.request_noun()),
            Self::Details(i, _) => format!("Loading {n} #{i}"),
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
    /// What the provider's CLI said about its sign-in, sent once when the thread starts.
    Cli(CliState),
    /// The host answered a list with a rate limit; sent just before that list's failure. `reset`
    /// is when the limit lifts (unix seconds), when the host said.
    RateLimited {
        reset: Option<i64>,
    },
    List(ListKey, Result<Vec<RequestSummary>, String>),
    /// Whether the host holds requests beyond the first page (and where a search goes on from);
    /// sent just before that list.
    More {
        key: ListKey,
        more: bool,
        cursor: Option<String>,
    },
    /// A page after the first, and whether the host holds more beyond it. Sent again with the
    /// page's checks and reviews, which replace its rows.
    Page {
        key: ListKey,
        page: usize,
        result: Result<(Vec<RequestSummary>, bool), String>,
        cursor: Option<String>,
    },
    Details(u64, u64, Box<Result<RequestDetails, String>>),
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
    /// On while this handle lives: the thread's requests check it right before they are sent, so
    /// a job under way when the tab lets the thread go sends nothing more.
    on: Arc<AtomicBool>,
    ctx: egui::Context,
}

impl HostWorker {
    /// Starts a thread for `forge`.
    pub fn spawn(forge: Forge, ctx: egui::Context) -> Self {
        let (jobs, job_rx) = unbounded::<HostJob>();
        let (tx, events) = unbounded::<HostEvent>();
        let thread_forge = forge.clone();
        let avatars_enabled = Arc::new(AtomicBool::new(false));
        let on = Arc::new(AtomicBool::new(true));
        let switches = Switches {
            avatars: Arc::clone(&avatars_enabled),
            on: Arc::clone(&on),
        };
        let thread_ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("hosting:{}", forge.host))
            .spawn(move || {
                run(thread_forge, job_rx, tx, thread_ctx, switches);
            });
        if let Err(e) = &spawned {
            tracing::error!("could not start the hosting thread: {e}");
        }
        Self {
            forge,
            jobs,
            events,
            avatars_enabled,
            on,
            ctx,
        }
    }

    /// A handle with no thread: tests feed its events and read its jobs.
    #[cfg(test)]
    pub fn detached(forge: Forge) -> (Self, Sender<HostEvent>, Receiver<HostJob>) {
        let (jobs, job_rx) = unbounded::<HostJob>();
        let (tx, events) = unbounded::<HostEvent>();
        let worker = Self {
            forge,
            jobs,
            events,
            avatars_enabled: Arc::new(AtomicBool::new(false)),
            on: Arc::new(AtomicBool::new(true)),
            ctx: egui::Context::default(),
        };
        (worker, tx, job_rx)
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

    /// A handle for `forge` with no thread behind it, so it cannot reach the network: a job sent
    /// to it is dropped and no event ever comes back. `--demo-requests` (screenshots only) shows
    /// invented requests through it.
    pub fn offline(forge: Forge, ctx: egui::Context) -> Self {
        // Both far ends are dropped at once: `send` fails quietly and `events` reads as closed.
        let (jobs, _) = unbounded::<HostJob>();
        let (_, events) = unbounded::<HostEvent>();
        Self {
            forge,
            jobs,
            events,
            avatars_enabled: Arc::new(AtomicBool::new(false)),
            on: Arc::new(AtomicBool::new(false)),
            ctx,
        }
    }
}

impl Drop for HostWorker {
    fn drop(&mut self) {
        self.set_avatars_enabled(false);
        self.on.store(false, Ordering::SeqCst);
    }
}

/// The flags a [`HostWorker`] shares with its thread.
struct Switches {
    /// Author pictures may be looked up ([`HostWorker::set_avatars_enabled`]).
    avatars: Arc<AtomicBool>,
    /// The tab still holds the handle; off once it lets the thread go.
    on: Arc<AtomicBool>,
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the thread owns its channels and context; dropping them on exit tells the other side"
)]
fn run(
    forge: Forge,
    jobs: Receiver<HostJob>,
    events: Sender<HostEvent>,
    ctx: egui::Context,
    switches: Switches,
) {
    let Switches {
        avatars: avatars_enabled,
        on,
    } = switches;
    let emit = |e: HostEvent| {
        let delivered = events.send(e).is_ok();
        ctx.request_repaint();
        delivered
    };
    // The CLI can be slow; ask it once, on this thread.
    let Resolved { token, cli } = resolve_token(&forge);
    emit(HostEvent::Cli(cli));
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
    let client = build_client(token.clone()).with_switch(Arc::clone(&on));
    // Author pictures have a thread of their own: a slow or black-holed picture host (a request
    // can take tens of seconds) must not hold up a pull or merge request action queued behind it.
    // Its client stops with the same switch, besides its own `avatars_enabled`.
    let avatar_jobs = {
        let (jobs, job_rx) = unbounded::<(canopy_core::Oid, String)>();
        let (client, forge) = (build_client(token).with_switch(on), forge.clone());
        let (events, ctx) = (events.clone(), ctx.clone());
        let spawned = std::thread::Builder::new()
            .name(format!("avatars:{}", forge.host))
            .spawn(move || {
                while let Ok((oid, email)) = job_rx.recv() {
                    let result =
                        crate::avatars::load(&client, &forge, oid, &email, &avatars_enabled);
                    if let Some(crate::avatars::Note::Blocked(why)) = &result.note {
                        log_blocked(&forge.host, why);
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
    serve(&client, &forge, &jobs, &avatar_jobs, &emit);
}

/// Logs that author pictures stopped for the repository on `host`, and the kind of answer that
/// stopped them. The author's email stays out of the log, and so do a refusal's words: the server
/// chose them, and could echo the token it was sent.
fn log_blocked(host: &str, why: &crate::avatars::Blocked) {
    use crate::avatars::Blocked;
    let reason = match why {
        Blocked::NotFound => "not found",
        Blocked::Refused(_) => "refused",
        Blocked::Moved => "moved",
    };
    tracing::info!(host, reason, "author pictures stopped for this repository");
}

/// A list shown without its checks and reviews: the list job it answers (see [`serve`]), its
/// filter and its rows.
struct Shown {
    list: u64,
    key: ListKey,
    /// The page the rows are (1 for the first).
    page: usize,
    /// Whether the host holds more after this page.
    more: bool,
    /// Where a search continues after this page.
    cursor: Option<String>,
    rows: Vec<RequestSummary>,
}

/// Which list job's rows the tab was sent last. The lock is held while a list is sent, so a
/// list's checks are sent only while its rows are still the ones shown.
type LastList = std::sync::Mutex<u64>;

fn last_list(last: &LastList) -> std::sync::MutexGuard<'_, u64> {
    last.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Runs `jobs` against `client` until the tab drops its end of the queue. `emit` reports an event
/// to the tab and says whether it got there; author pictures go to `avatar_jobs`.
///
/// A loaded list's checks and reviews (one query that can take seconds) wait for
/// [`ROLLUP_DELAY`] and for every job queued by then, such as the details of the request the list
/// selected or an action the user started. Then they load on a lane of their own
/// ([`add_rollups`]), so a request opened or an action started while the query runs does not
/// wait for it either. A newer list replaces one still waiting, and the checks of a list the tab
/// no longer shows are dropped.
fn serve(
    client: &Client,
    forge: &Forge,
    jobs: &Receiver<HostJob>,
    avatar_jobs: &Sender<(canopy_core::Oid, String)>,
    emit: &(dyn Fn(HostEvent) -> bool + Sync),
) {
    let last = LastList::new(0);
    let (rollups, shown) = unbounded::<Shown>();
    std::thread::scope(|scope| {
        let lane_last = &last;
        // The lane owns the receiver: if it cannot start, the receiver goes with it and lists
        // queue nothing for a lane that will never read them.
        let lane = std::thread::Builder::new()
            .name(format!("request-checks:{}", forge.host))
            .spawn_scoped(scope, move || add_rollups(client, &shown, lane_last, emit));
        if let Err(e) = &lane {
            tracing::error!("could not start the request-checks thread: {e}");
        }
        serve_jobs(client, forge, jobs, avatar_jobs, &rollups, &last, emit);
        // Ends the lane once its query, if any, is done.
        drop(rollups);
    });
}

/// The lane that gives shown lists their checks and reviews ([`add_rollup`]), newest first, and
/// sends a list again with them only while it is still the one the tab shows.
fn add_rollups(
    client: &Client,
    shown: &Receiver<Shown>,
    last: &LastList,
    emit: &(dyn Fn(HostEvent) -> bool + Sync),
) {
    // Until when (unix seconds) the list goes without checks and reviews after a rate limit.
    let mut paused = 0;
    while let Ok(first) = shown.recv() {
        // Only the newest list is worth a query: pages of an older list are dropped, the pages of
        // the newest one are all kept.
        let mut batch = vec![first];
        while let Ok(more) = shown.try_recv() {
            batch.push(more);
        }
        for mut next in batch {
            if *last_list(last) != next.list {
                continue;
            }
            let now = chrono::Utc::now().timestamp();
            if add_rollup(&mut next.rows, client.has_token(), &mut paused, now, |n| {
                client.rollup(n)
            }) {
                let current = last_list(last);
                if *current == next.list {
                    emit(if next.page <= 1 {
                        HostEvent::List(next.key, Ok(next.rows))
                    } else {
                        HostEvent::Page {
                            key: next.key,
                            page: next.page,
                            result: Ok((next.rows, next.more)),
                            cursor: next.cursor,
                        }
                    });
                }
            }
        }
    }
}

/// The job loop of [`serve`]; `rollups` takes each list due for its checks and reviews.
fn serve_jobs(
    client: &Client,
    forge: &Forge,
    jobs: &Receiver<HostJob>,
    avatar_jobs: &Sender<(canopy_core::Oid, String)>,
    rollups: &Sender<Shown>,
    last: &LastList,
    emit: &(dyn Fn(HostEvent) -> bool + Sync),
) {
    let send = |e: HostEvent| {
        emit(e);
    };
    // A list shown without its checks and reviews, and when they may be asked for.
    let mut waiting: Option<(Shown, Instant)> = None;
    loop {
        let next = match &waiting {
            None => jobs.recv().map_err(RecvTimeoutError::from),
            Some((_, due)) => jobs.recv_deadline(*due),
        };
        let job = match next {
            Ok(job) => job,
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                if let Some((shown, _)) = waiting.take() {
                    let _ = rollups.send(shown);
                }
                continue;
            }
        };
        let label = job.label(forge.kind);
        tracing::debug!(host = %forge.host, job = %label, "hosting job");
        let s = |e: canopy_hosting::Error| e.to_string();
        match job {
            HostJob::Avatar(oid, email) => {
                if let Err(unsent) = avatar_jobs.send((oid, email)) {
                    // No picture thread: end the request so the tab does not wait for it.
                    let (oid, email) = unsent.into_inner();
                    let result = crate::avatars::unavailable(chrono::Utc::now().timestamp());
                    send(HostEvent::Avatar { oid, email, result });
                }
            }
            HostJob::List(key) => {
                waiting = None;
                let result = client.search_page(key.state, &key.query, 1, None);
                // Held while the list is sent, so no older list's checks can follow it.
                let mut sent = last_list(last);
                *sent += 1;
                let list = *sent;
                // The list shows at once; its checks and reviews follow in a second event for
                // the same filter, which replaces it (the selection stays). A list the tab no
                // longer takes was let go with its thread: nothing more is asked.
                let mut more = false;
                let mut cursor = None;
                let delivered = match result {
                    Ok(page) => {
                        more = page.more;
                        cursor.clone_from(&page.cursor);
                        let rows = page.rows;
                        emit(HostEvent::More {
                            key: key.clone(),
                            more,
                            cursor: page.cursor,
                        });
                        emit(HostEvent::List(key.clone(), Ok(rows.clone()))).then_some(rows)
                    }
                    Err(e) => {
                        if let canopy_hosting::Error::RateLimited { reset } = &e {
                            send(HostEvent::RateLimited { reset: *reset });
                        }
                        send(HostEvent::List(key.clone(), Err(s(e))));
                        None
                    }
                };
                drop(sent);
                if let Some(rows) = delivered {
                    let due = Instant::now() + ROLLUP_DELAY;
                    let shown = Shown {
                        list,
                        key,
                        page: 1,
                        more,
                        cursor,
                        rows,
                    };
                    waiting = Some((shown, due));
                }
            }
            HostJob::ListMore(key, page, after) => {
                // The list still counts as the one the tab shows: its checks and these pages'
                // are asked for in the same lane.
                let result = client.search_page(key.state, &key.query, page, after.as_deref());
                let list = *last_list(last);
                let delivered = match result {
                    Ok(got) => {
                        let (rows, more, cursor) = (got.rows, got.more, got.cursor);
                        let sent = emit(HostEvent::Page {
                            key: key.clone(),
                            page,
                            result: Ok((rows.clone(), more)),
                            cursor: cursor.clone(),
                        });
                        sent.then_some((rows, more, cursor))
                    }
                    Err(e) => {
                        if let canopy_hosting::Error::RateLimited { reset } = &e {
                            send(HostEvent::RateLimited { reset: *reset });
                        }
                        send(HostEvent::Page {
                            key: key.clone(),
                            page,
                            result: Err(s(e)),
                            cursor: None,
                        });
                        None
                    }
                };
                if let Some((rows, more, cursor)) = delivered {
                    let _ = rollups.send(Shown {
                        list,
                        key,
                        page,
                        more,
                        cursor,
                        rows,
                    });
                }
            }
            HostJob::Details(n, generation) => send(HostEvent::Details(
                n,
                generation,
                Box::new(client.details(n).map_err(s)),
            )),
            HostJob::Create(req) => send(HostEvent::Created(client.create(&req).map_err(s))),
            HostJob::Merge(n, method, head) => send(HostEvent::Done {
                label,
                action: HostAction::Merge,
                number: Some(n),
                result: client.merge(n, method, head.as_deref()).map_err(s),
            }),
            HostJob::SetOpen(n, open) => send(HostEvent::Done {
                label,
                action: HostAction::SetOpen,
                number: Some(n),
                result: client.set_open(n, open).map_err(s),
            }),
            HostJob::Comment(n, body) => send(HostEvent::Done {
                label,
                action: HostAction::Comment,
                number: Some(n),
                result: client.comment(n, &body).map_err(s),
            }),
            HostJob::Approve(n) => send(HostEvent::Done {
                label,
                action: HostAction::Approve,
                number: Some(n),
                result: client.approve(n).map_err(s),
            }),
        }
    }
}

/// How long a list's checks and reviews wait for jobs the list itself sets off (the tab opens the
/// request it selects as soon as the list arrives).
const ROLLUP_DELAY: Duration = Duration::from_millis(250);

/// How long the list goes without checks and reviews after a rate limit that named no reset.
const ROLLUP_PAUSE_SECS: i64 = 60;
/// The longest a rate-limited host is left alone, however far off it puts its reset (an hour,
/// GitHub's longest window): a wrong or hostile reset cannot switch a feature off for the session.
const RATE_LIMIT_PAUSE_MAX_SECS: i64 = 3_600;

/// Until when (unix seconds) to leave a host alone after a rate limit: its `reset` when it named
/// one still ahead (an hour at most), else `default_secs` from `now`.
pub fn rate_limit_pause(reset: Option<i64>, now: i64, default_secs: i64) -> i64 {
    reset.filter(|&r| r > now).map_or(now + default_secs, |r| {
        r.min(now + RATE_LIMIT_PAUSE_MAX_SECS)
    })
}

/// Gives each row of a freshly loaded `list` its checks and reviews ([`Client::rollup`]: one
/// query for the page), only when a token is set and no rate limit asked to wait (`paused`, unix
/// seconds, moved to the limit's reset when one answers, an hour ahead at most). Without them a
/// row shows what the request's details said when they were opened, as before. A failure is
/// logged like any skipped part of a request and leaves the rows as they are. Returns whether any
/// row got a rollup.
fn add_rollup(
    list: &mut [RequestSummary],
    has_token: bool,
    paused: &mut i64,
    now: i64,
    fetch: impl FnOnce(&[u64]) -> canopy_hosting::Result<Vec<(u64, canopy_hosting::Rollup)>>,
) -> bool {
    if !has_token || list.is_empty() || now < *paused {
        return false;
    }
    let numbers: Vec<u64> = list.iter().map(|r| r.number).collect();
    match fetch(&numbers) {
        Ok(found) => {
            let mut added = false;
            for (number, rollup) in found {
                if let Some(row) = list.iter_mut().find(|r| r.number == number) {
                    row.rollup = Some(rollup);
                    added = true;
                }
            }
            added
        }
        Err(e) => {
            if let canopy_hosting::Error::RateLimited { reset } = &e {
                *paused = rate_limit_pause(*reset, now, ROLLUP_PAUSE_SECS);
            }
            tracing::warn!(error = %e, "skipping the checks and reviews of the request list");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cli_that_cannot_be_used_gives_no_token_and_says_why() {
        let kind = ForgeKind::GitLab;
        let errors = [
            CliError::NotInstalled { kind },
            CliError::SignedOut {
                kind,
                host: "gitlab.corp".into(),
            },
            CliError::MissingScope {
                kind,
                host: "gitlab.corp".into(),
                user: Some("tanuki".into()),
                scope: "api",
            },
            CliError::Failed {
                kind,
                message: "it took longer than 10 seconds".into(),
            },
        ];
        for error in errors {
            let resolved = Resolved::from(Err(error.clone()));
            assert!(resolved.token.is_none());
            assert_eq!(resolved.cli, Err(error));
        }
    }

    #[test]
    fn debug_output_never_shows_the_token() {
        let resolved = Resolved {
            token: Some("ghp_secretvalue".into()),
            cli: Ok(CliAccount {
                user: Some("octocat".into()),
                scopes: None,
            }),
        };
        let shown = format!("{resolved:?} {resolved:#?}");
        assert!(shown.contains("octocat"), "{shown}");
        assert!(!shown.contains("ghp_secretvalue"), "{shown}");
    }

    /// What builds before ADR 0014 saved is deleted, and nothing else in the folder is touched.
    #[test]
    fn the_old_saved_tokens_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["tokens.json", "tokens.json.tmp", "settings.json"] {
            std::fs::write(
                dir.path().join(name),
                r#"{"tokens":{"github.com":"ghp_a"}}"#,
            )
            .unwrap();
        }
        remove_saved_tokens_in(dir.path());
        assert!(!dir.path().join("tokens.json").exists());
        assert!(!dir.path().join("tokens.json.tmp").exists());
        assert!(dir.path().join("settings.json").exists());
        // A folder without them is left alone.
        remove_saved_tokens_in(dir.path());
        remove_saved_tokens_in(&dir.path().join("missing"));
    }

    const CHILD: &str = "CANOPY_TEST_HOSTING_CHILD";

    /// The half of the two tests below that runs in a copy of this test binary they start (the
    /// environment cannot be changed in-process): a hosting thread for github.com/o/r that lists
    /// the open pull requests, against the stub whose address the environment names. Does nothing
    /// in an ordinary run.
    #[test]
    fn the_child_of_the_environment_tests() {
        if std::env::var_os(CHILD).is_none() {
            return;
        }
        let forge = Forge {
            kind: ForgeKind::GitHub,
            host: "github.com".into(),
            port: None,
            path: "o/r".into(),
        };
        let worker = HostWorker::spawn(forge, egui::Context::default());
        worker.send(HostJob::List(StateFilter::Open.into()));
        loop {
            match worker.events.recv_timeout(Duration::from_secs(20)) {
                Ok(HostEvent::List(..)) => break,
                Ok(_) => {}
                Err(e) => panic!("no list answer: {e}"),
            }
        }
    }

    /// Runs the child above with every token variable set to `LEAK`, a `gh` on `PATH` that runs
    /// `fake_gh`, and the API at a stub; returns the `Authorization` header of each request the
    /// stub got.
    #[cfg(unix)]
    #[expect(
        clippy::disallowed_methods,
        reason = "starts this test binary again with its own environment"
    )]
    fn authorizations_with_tokens_in_the_environment(fake_gh: &str) -> Vec<String> {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let gh = dir.path().join("gh");
        std::fs::write(&gh, format!("#!/bin/sh\n{fake_gh}\n")).unwrap();
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
        let (base, _, authorizations) = stub_with_authorizations(Box::new(|target| {
            if target.starts_with("/repos/o/r/pulls?") {
                pulls(&[1])
            } else {
                "{}".into()
            }
        }));
        let path = format!(
            "{}:{}",
            dir.path().display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "hosting::tests::the_child_of_the_environment_tests",
                "--nocapture",
            ])
            .env("PATH", path)
            .env(CHILD, "1")
            .env("CANOPY_HOSTING_API_BASE", &base);
        for name in [
            "GH_TOKEN",
            "GITHUB_TOKEN",
            "GH_ENTERPRISE_TOKEN",
            "GITLAB_TOKEN",
            "GL_TOKEN",
        ] {
            child.env(name, LEAK);
        }
        let out = child.output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && text.contains("1 passed"),
            "{text}{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let seen = authorizations.lock().unwrap().clone();
        assert!(!seen.is_empty(), "no request reached the stub");
        seen
    }

    const LEAK: &str = "ghp_FROMTHEENVIRONMENT";

    /// With `gh` signed in, every request carries its token, and the token variables in Canopy's
    /// environment neither reach the request nor the CLI (a fake `gh` that answers with
    /// `$GH_TOKEN` when it sees one).
    #[cfg(unix)]
    #[test]
    fn the_cli_token_is_used_and_the_environment_is_not() {
        let seen = authorizations_with_tokens_in_the_environment(
            r#"case "$2" in
status) echo "  ✓ Logged in to github.com account octocat (keyring)"; echo "  - Token scopes: 'repo'";;
token) if [ -n "$GH_TOKEN" ]; then echo "$GH_TOKEN"; else echo gho_FROMTHECLI; fi;;
esac"#,
        );
        assert!(
            seen.iter().all(|a| a == "Bearer gho_FROMTHECLI"),
            "{seen:?}"
        );
    }

    /// With `gh` signed out, requests carry no credential at all, whatever the environment holds.
    #[cfg(unix)]
    #[test]
    fn the_environment_alone_sends_no_credential() {
        let seen = authorizations_with_tokens_in_the_environment(
            r#"echo "You are not logged into any accounts on github.com" >&2; exit 1"#,
        );
        assert!(seen.iter().all(|a| a == "none"), "{seen:?}");
    }

    fn rows(numbers: &[u64]) -> Vec<RequestSummary> {
        numbers
            .iter()
            .map(|&number| RequestSummary {
                number,
                title: format!("PR {number}"),
                author: "ann".into(),
                state: canopy_hosting::RequestState::Open,
                draft: false,
                source_branch: "topic".into(),
                target_branch: "main".into(),
                from_fork: false,
                created_at: 0,
                updated_at: 0,
                web_url: String::new(),
                labels: Vec::new(),
                description: String::new(),
                rollup: None,
            })
            .collect()
    }

    const PASSING: canopy_hosting::Rollup = canopy_hosting::Rollup {
        checks: Some(canopy_hosting::CheckState::Success),
        review: Some(canopy_hosting::ReviewDecision::Approved),
    };

    #[test]
    fn a_list_gets_its_rollup_in_one_query_only_with_a_token() {
        let mut list = rows(&[3, 2, 1]);
        let mut paused = 0;
        let mut asked = Vec::new();
        let added = add_rollup(&mut list, false, &mut paused, 1_000, |n| {
            asked.push(n.to_vec());
            Ok(vec![(1, PASSING)])
        });
        assert!(!added && asked.is_empty(), "no token, no query");
        let added = add_rollup(&mut list, true, &mut paused, 1_000, |n| {
            asked.push(n.to_vec());
            Ok(vec![(1, PASSING), (3, canopy_hosting::Rollup::default())])
        });
        assert!(added);
        assert_eq!(asked, [vec![3, 2, 1]], "one query for the whole page");
        let got: Vec<_> = list.iter().map(|r| r.rollup).collect();
        assert_eq!(
            got,
            [Some(canopy_hosting::Rollup::default()), None, Some(PASSING)],
            "a request the query did not answer keeps no rollup"
        );
        let mut empty = rows(&[]);
        assert!(!add_rollup(&mut empty, true, &mut paused, 1_000, |_| {
            panic!("an empty list needs no query")
        }));
        // Answers for none of the rows add nothing, so the list is not sent again.
        let mut unanswered = rows(&[5]);
        assert!(!add_rollup(
            &mut unanswered,
            true,
            &mut paused,
            1_000,
            |_| { Ok(vec![(6, PASSING)]) }
        ));
    }

    #[test]
    fn a_rate_limited_rollup_waits_for_the_reset() {
        let mut list = rows(&[1]);
        let mut paused = 0;
        let limited = |reset| {
            move |_: &[u64]| -> canopy_hosting::Result<Vec<(u64, canopy_hosting::Rollup)>> {
                Err(canopy_hosting::Error::RateLimited { reset })
            }
        };
        assert!(!add_rollup(
            &mut list,
            true,
            &mut paused,
            1_000,
            limited(Some(1_900))
        ));
        assert_eq!(paused, 1_900);
        assert_eq!(list[0].rollup, None, "the rows stay as they were");
        let mut calls = 0;
        add_rollup(&mut list, true, &mut paused, 1_899, |_| {
            calls += 1;
            Ok(vec![(1, PASSING)])
        });
        assert_eq!(calls, 0, "nothing is sent before the reset");
        add_rollup(&mut list, true, &mut paused, 1_900, |_| {
            calls += 1;
            Ok(vec![(1, PASSING)])
        });
        assert_eq!((calls, list[0].rollup), (1, Some(PASSING)));
        // No reset named (or one already past): a minute's pause.
        add_rollup(&mut list, true, &mut paused, 2_000, limited(None));
        assert_eq!(paused, 2_000 + ROLLUP_PAUSE_SECS);
        add_rollup(&mut list, true, &mut paused, 3_000, limited(Some(10)));
        assert_eq!(paused, 3_000 + ROLLUP_PAUSE_SECS);
        // Any other failure sends again with the next list.
        let mut other = 0;
        for _ in 0..2 {
            add_rollup(&mut list, true, &mut paused, 4_000, |_| {
                other += 1;
                Err(canopy_hosting::Error::Network("refused".into()))
            });
        }
        assert_eq!(other, 2);
        // However far off the server puts its reset, the list waits an hour at most.
        add_rollup(
            &mut list,
            true,
            &mut paused,
            5_000,
            limited(Some(32_503_680_000)),
        );
        assert_eq!(paused, 5_000 + RATE_LIMIT_PAUSE_MAX_SECS);
    }

    type Answer = Box<dyn Fn(&str) -> String + Send + Sync>;

    /// A loopback stand-in for GitHub's API: one request per connection, each connection on a
    /// thread of its own (an answer may wait), answered with 200 and `answer(target)`. Returns its
    /// base URL and the `METHOD target` lines it received, in order of arrival.
    fn stub(answer: Answer) -> (String, Log) {
        let (base, log, _) = stub_with_authorizations(answer);
        (base, log)
    }

    type Log = Arc<std::sync::Mutex<Vec<String>>>;

    /// [`stub`], and the `Authorization` header of each request (`none` without one), in the same
    /// order as the requests.
    fn stub_with_authorizations(answer: Answer) -> (String, Log, Log) {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("address"));
        let log = Log::default();
        let seen = Arc::clone(&log);
        let authorizations = Log::default();
        let authorized = Arc::clone(&authorizations);
        let answer: Arc<Answer> = Arc::new(answer);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let (seen, answer) = (Arc::clone(&seen), Arc::clone(&answer));
                let authorized = Arc::clone(&authorized);
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let mut length = 0;
                    let mut authorization = "none".to_owned();
                    loop {
                        let mut header = String::new();
                        if reader.read_line(&mut header).unwrap_or(0) == 0
                            || header.trim().is_empty()
                        {
                            break;
                        }
                        if let Some((name, value)) = header.split_once(':') {
                            if name.eq_ignore_ascii_case("content-length") {
                                length = value.trim().parse().unwrap_or(0);
                            } else if name.eq_ignore_ascii_case("authorization") {
                                value.trim().clone_into(&mut authorization);
                            }
                        }
                    }
                    let mut body = vec![0; length];
                    let _ = reader.read_exact(&mut body);
                    let mut parts = line.split_whitespace();
                    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
                    authorized.lock().expect("log").push(authorization);
                    seen.lock().expect("log").push(format!("{method} {target}"));
                    let reply = answer(target);
                    let mut stream = stream;
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                        reply.len()
                    );
                });
            }
        });
        (base, log, authorizations)
    }

    fn pull(n: u64) -> serde_json::Value {
        serde_json::json!({
            "number": n,
            "title": format!("PR {n}"),
            "user": { "login": "ann" },
            "state": "open",
            "head": { "ref": "topic", "sha": "abc", "repo": { "full_name": "o/r" } },
            "base": { "ref": "main", "sha": "def", "repo": { "full_name": "o/r" } },
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "html_url": format!("https://github.com/o/r/pull/{n}"),
        })
    }

    fn pulls(numbers: &[u64]) -> String {
        serde_json::Value::Array(numbers.iter().map(|&n| pull(n)).collect()).to_string()
    }

    const ROLLUP_ANSWER: &str = r#"{"data":{"repository":{"p1":{"number":1,"statusCheckRollup":{"state":"SUCCESS"},"reviewDecision":"APPROVED"}}}}"#;

    /// A hosting thread for `github.com/o/r` with a token, served by the stub at `base`, as the
    /// tab holds it.
    fn worker_for(base: &str) -> (HostWorker, std::thread::JoinHandle<()>) {
        let forge = Forge {
            kind: ForgeKind::GitHub,
            host: "github.com".into(),
            port: None,
            path: "o/r".into(),
        };
        let (jobs, job_rx) = unbounded::<HostJob>();
        let (tx, events) = unbounded::<HostEvent>();
        let on = Arc::new(AtomicBool::new(true));
        let client = Client::with_api_base(&forge, Some("tok-secret".into()), base)
            .with_switch(Arc::clone(&on));
        let thread_forge = forge.clone();
        let thread = std::thread::spawn(move || {
            let (avatar_jobs, _) = unbounded();
            serve(&client, &thread_forge, &job_rx, &avatar_jobs, &|e| {
                tx.send(e).is_ok()
            });
        });
        let worker = HostWorker {
            forge,
            jobs,
            events,
            avatars_enabled: Arc::new(AtomicBool::new(false)),
            on,
            ctx: egui::Context::default(),
        };
        (worker, thread)
    }

    #[test]
    fn a_rate_limited_list_says_when_the_limit_lifts_before_it_fails() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("address"));
        let reset = chrono::Utc::now().timestamp() + 600;
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let _ = stream.read(&mut [0; 4096]);
                let _ = write!(
                    stream,
                    "HTTP/1.1 429 Too Many Requests\r\nX-RateLimit-Reset: {reset}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
            }
        });
        let (worker, thread) = worker_for(&base);
        worker.send(HostJob::List(StateFilter::Open.into()));
        let mut seen = Vec::new();
        while !matches!(seen.last(), Some(HostEvent::List(..))) {
            match worker.events.recv_timeout(Duration::from_secs(20)) {
                Ok(event) => seen.push(event),
                Err(e) => panic!("no list answer: {e}"),
            }
        }
        drop(worker);
        thread.join().expect("hosting thread");
        let limited = seen
            .iter()
            .position(|e| matches!(e, HostEvent::RateLimited { reset: Some(r) } if *r == reset));
        assert_eq!(limited, Some(seen.len() - 2), "{seen:?}");
        assert!(matches!(seen.last(), Some(HostEvent::List(_, Err(_)))));
    }

    #[test]
    fn a_list_sends_no_query_once_its_worker_is_gone() {
        // The tab drops its worker (integration switched off, token changed, other remote) while
        // the list is on its way: the thread finishes that request but sends nothing more.
        let held: Arc<std::sync::Mutex<Option<HostWorker>>> = Arc::default();
        let dropped = Arc::clone(&held);
        let (base, log) = stub(Box::new(move |target| {
            if target.starts_with("/repos/o/r/pulls?") {
                drop(dropped.lock().expect("worker").take());
                pulls(&[1])
            } else {
                ROLLUP_ANSWER.to_owned()
            }
        }));
        let (worker, thread) = worker_for(&base);
        {
            let mut slot = held.lock().expect("worker");
            slot.insert(worker)
                .send(HostJob::List(StateFilter::Open.into()));
        }
        thread.join().expect("hosting thread");
        let log = log.lock().expect("log").clone();
        assert_eq!(log.len(), 1, "only the list was sent: {log:?}");
        assert!(log[0].starts_with("GET /repos/o/r/pulls?"), "{log:?}");
    }

    #[test]
    fn a_lists_checks_wait_for_the_jobs_queued_behind_it() {
        // The tab opens the request the list selected, and the user may act on it at once: the
        // list's checks and reviews (one query that can take seconds) go after those jobs.
        let (base, log) = stub(Box::new(|target| {
            if target.starts_with("/repos/o/r/pulls?") {
                pulls(&[1])
            } else if target == "/repos/o/r/pulls/1" {
                pull(1).to_string()
            } else if target == "/graphql" {
                ROLLUP_ANSWER.to_owned()
            } else {
                "[]".to_owned()
            }
        }));
        let (worker, thread) = worker_for(&base);
        worker.send(HostJob::List(StateFilter::Open.into()));
        worker.send(HostJob::Details(1, 1));
        worker.send(HostJob::Comment(1, "Looks good".into()));
        let mut lists = 0;
        while lists < 2 {
            match worker.events.recv_timeout(Duration::from_secs(20)) {
                Ok(HostEvent::List(..)) => lists += 1,
                Ok(_) => {}
                Err(e) => panic!("no second list: {e}"),
            }
        }
        drop(worker);
        thread.join().expect("hosting thread");
        let log = log.lock().expect("log").clone();
        let query = log.iter().position(|l| l == "POST /graphql");
        assert_eq!(query, Some(log.len() - 1), "the query goes last: {log:?}");
        assert!(log.iter().any(|l| l == "GET /repos/o/r/pulls/1"), "{log:?}");
        assert!(
            log.iter()
                .any(|l| l.starts_with("POST /repos/o/r/issues/1/comments")),
            "{log:?}"
        );
    }

    #[test]
    fn a_request_opened_while_the_checks_load_does_not_wait_for_them() {
        // The list's checks and reviews are under way (a query that can take seconds) when the
        // user opens a request: its details come without waiting for the query to finish.
        let (arrived, query_arrived) = unbounded::<()>();
        let (release, released) = unbounded::<()>();
        let (base, _) = stub(Box::new(move |target| {
            if target.starts_with("/repos/o/r/pulls?") {
                pulls(&[1])
            } else if target == "/graphql" {
                let _ = arrived.send(());
                let _ = released.recv_timeout(Duration::from_secs(10));
                ROLLUP_ANSWER.to_owned()
            } else if target == "/repos/o/r/pulls/1" {
                pull(1).to_string()
            } else {
                "[]".to_owned()
            }
        }));
        let (worker, thread) = worker_for(&base);
        worker.send(HostJob::List(StateFilter::Open.into()));
        query_arrived
            .recv_timeout(Duration::from_secs(10))
            .expect("the list's query was sent");
        worker.send(HostJob::Details(1, 1));
        let opened = loop {
            match worker.events.recv_timeout(Duration::from_secs(5)) {
                Ok(HostEvent::Details(n, _, details)) => break (n, details.is_ok()),
                Ok(_) => {}
                Err(e) => panic!("the request waited for the list's checks: {e}"),
            }
        };
        assert_eq!(opened, (1, true));
        let _ = release.send(());
        let with_checks = loop {
            match worker.events.recv_timeout(Duration::from_secs(10)) {
                Ok(HostEvent::List(_, Ok(list))) => break list,
                Ok(_) => {}
                Err(e) => panic!("no list with its checks: {e}"),
            }
        };
        assert_eq!(with_checks[0].rollup, Some(PASSING));
        drop(worker);
        thread.join().expect("hosting thread");
    }

    #[test]
    fn a_newer_list_is_never_replaced_by_an_older_ones_checks() {
        // The checks of the first list arrive after a second list was shown: they belong to rows
        // that may have changed, so they are dropped rather than shown over the newer list.
        let (arrived, query_arrived) = unbounded::<()>();
        let (release, released) = unbounded::<()>();
        let lists = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let served = Arc::clone(&lists);
        let (base, log) = stub(Box::new(move |target| {
            if target.starts_with("/repos/o/r/pulls?") {
                // The first list has request 1, the second requests 1 and 2.
                if served.fetch_add(1, Ordering::SeqCst) == 0 {
                    pulls(&[1])
                } else {
                    pulls(&[2, 1])
                }
            } else if target == "/graphql" {
                let _ = arrived.send(());
                let _ = released.recv_timeout(Duration::from_secs(10));
                ROLLUP_ANSWER.to_owned()
            } else {
                "[]".to_owned()
            }
        }));
        let (worker, thread) = worker_for(&base);
        worker.send(HostJob::List(StateFilter::Open.into()));
        query_arrived
            .recv_timeout(Duration::from_secs(10))
            .expect("the first list's query was sent");
        worker.send(HostJob::List(StateFilter::Open.into()));
        let mut shown = Vec::new();
        while shown.len() < 2 {
            match worker.events.recv_timeout(Duration::from_secs(5)) {
                Ok(HostEvent::List(_, Ok(list))) => shown.push(list),
                Ok(_) => {}
                Err(e) => panic!("the second list waited for the first one's checks: {e}"),
            }
        }
        assert_eq!(shown[1].len(), 2, "the second list is shown");
        // The first query ends; the second list's own query follows.
        let _ = release.send(());
        query_arrived
            .recv_timeout(Duration::from_secs(10))
            .expect("the second list's query was sent");
        let _ = release.send(());
        let last = loop {
            match worker.events.recv_timeout(Duration::from_secs(10)) {
                Ok(HostEvent::List(_, Ok(list))) => break list,
                Ok(_) => {}
                Err(e) => panic!("no list with its checks: {e}"),
            }
        };
        assert_eq!(last.len(), 2, "only the newer list's checks are shown");
        assert_eq!(last[1].rollup, Some(PASSING));
        drop(worker);
        thread.join().expect("hosting thread");
        let queries = log
            .lock()
            .expect("log")
            .iter()
            .filter(|l| l.as_str() == "POST /graphql")
            .count();
        assert_eq!(queries, 2);
    }

    /// A log destination for one test.
    #[derive(Clone, Default)]
    struct Captured(Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Captured {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("log").extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_refused_repository_is_logged_without_the_servers_words() {
        use crate::avatars::Blocked;
        let captured = Captured::default();
        let writer = captured.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            log_blocked(
                "gitlab.example.com",
                &Blocked::Refused("echoed glpat-secret-token".into()),
            );
            log_blocked("gitlab.example.com", &Blocked::NotFound);
            log_blocked("gitlab.example.com", &Blocked::Moved);
        });
        let log = String::from_utf8(captured.0.lock().expect("log").clone()).expect("UTF-8");
        assert_eq!(log.lines().count(), 3, "{log}");
        assert!(!log.contains("glpat") && !log.contains("echoed"), "{log}");
        for reason in ["refused", "not found", "moved"] {
            assert!(log.contains(&format!("reason=\"{reason}\"")), "{log}");
        }
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
