//! One file watcher for every open repository. A single thread owns one `notify` watcher (one
//! inotify instance on Linux), watches each tab's working tree and git folders, and routes every
//! change to the tabs it belongs to: a change in a git folder asks for a full refresh, any other
//! change for a status refresh (ignored files skipped), both debounced per tab.
//!
//! Recursive watches never overlap. A path inside another watched path (a submodule's or nested
//! repository's tree, a second tab on the same repository) is already covered by the outer
//! watch, and `notify` shares one inotify watch per folder between overlapping watches: removing
//! the inner one would remove the outer one's folders too. The hub therefore watches only the
//! outermost of the wanted paths, and when that set changes it refreshes the tabs whose folders
//! were briefly unwatched, so no change goes unseen.

use std::collections::{BTreeSet, HashMap};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Select, Sender, TryRecvError, unbounded};
use notify::Watcher as _;

use crate::worker::Job;

/// How long a tab's changes must stay quiet before it is told to refresh.
const DEBOUNCE: Duration = Duration::from_millis(350);

/// A tab whose watches were registered this long after it asked may have read the repository
/// before they existed: it reads everything again.
const LATE_REGISTRATION: Duration = Duration::from_secs(1);

/// When the system dropped queued changes, tabs refresh this far apart, not all at once.
const RESCAN_STEP: Duration = Duration::from_millis(50);

/// The reason a tab is given when the watcher itself failed.
const WATCHER_STOPPED: &str = "Canopy's file watcher stopped.";

/// What a tab says after a watch failure, behind the reason.
pub const WATCH_FAILED_HINT: &str = "Canopy shows changes on disk after a Refresh.";

/// Whether a change at `rel` inside a git folder is worth a full refresh. Object writes, reflogs,
/// lock files and fsmonitor state change constantly during git commands and say nothing new.
pub fn relevant_git_path(rel: &Path) -> bool {
    let s = rel.to_string_lossy();
    !(s.starts_with("objects")
        || s.starts_with("logs")
        || s.ends_with(".lock")
        || s.contains("fsmonitor"))
}

/// Watches the repository at `path` for as long as `stop`'s sender lives, sending
/// [`Job::Refresh`] and [`Job::RefreshStatus`] to `jobs`, and [`Job::WatchFailed`] once if a
/// folder cannot be watched. `ended` is dropped when the watch is gone. Opening the repository
/// and registering the watches happen on the shared watcher thread, never the caller's.
/// Symbolic links are not followed. A linked worktree's or submodule's git folder (and the
/// common folder it shares) lives outside the working tree, so it is watched too.
pub fn watch(path: PathBuf, jobs: Sender<Job>, stop: Receiver<()>, ended: Sender<()>) {
    let command = Command {
        path,
        jobs,
        stop,
        ended,
        queued: Instant::now(),
    };
    let refused = match hub() {
        Some(hub) => match hub.send(command) {
            Ok(()) => return,
            Err(refused) => refused.0,
        },
        None => command,
    };
    tracing::warn!("no file watcher: its thread is not running");
    let _ = refused.jobs.send(Job::WatchFailed(
        "Canopy's file watcher is not running.".into(),
    ));
}

/// A repository to watch, as [`watch`] hands it to the watcher thread.
struct Command {
    path: PathBuf,
    jobs: Sender<Job>,
    stop: Receiver<()>,
    ended: Sender<()>,
    queued: Instant,
}

/// The watcher thread's inbox, started with the first watch.
fn hub() -> Option<&'static Sender<Command>> {
    static HUB: OnceLock<Option<Sender<Command>>> = OnceLock::new();
    HUB.get_or_init(|| {
        let (tx, rx) = unbounded::<Command>();
        match std::thread::Builder::new()
            .name("watcher".into())
            .spawn(move || {
                let (event_tx, events) = unbounded();
                run(&rx, &events, move || {
                    let tx = event_tx.clone();
                    let watcher = notify::RecommendedWatcher::new(
                        move |e| {
                            let _ = tx.send(e);
                        },
                        notify::Config::default().with_follow_symlinks(false),
                    )
                    .map_err(|e| describe(&e))?;
                    Ok(Notify(watcher))
                });
            }) {
            Ok(_) => Some(tx),
            Err(e) => {
                tracing::warn!("no file watcher: cannot start its thread: {e}");
                None
            }
        }
    })
    .as_ref()
}

/// The watcher thread: takes new repositories, routes changes, ends watches whose tab is gone and
/// tells each tab once its changes have settled. Returns when `commands` disconnects and no tab
/// is left. A panic inside (the backend's) is contained: the tabs watched then are told, and
/// the next repository starts a fresh backend.
fn run<B: Backend>(
    commands: &Receiver<Command>,
    events: &Receiver<notify::Result<notify::Event>>,
    make: impl FnMut() -> Result<B, String>,
) {
    let mut hub = Hub::new(make);
    let mut closed = false;
    // Commands not yet taken in, and the one being taken in: after a panic each is told.
    let mut pending: std::collections::VecDeque<Command> = std::collections::VecDeque::new();
    let opening: std::cell::RefCell<Option<Sender<Job>>> = std::cell::RefCell::new(None);
    loop {
        let stops: Vec<Receiver<()>> = hub.tabs.iter().map(|t| t.stop.clone()).collect();
        let mut select = Select::new();
        if !closed {
            select.recv(commands);
        }
        select.recv(events);
        for stop in &stops {
            select.recv(stop);
        }
        match hub.next_due() {
            Some(due) => {
                let _ = select.ready_timeout(due.saturating_duration_since(Instant::now()));
            }
            None => {
                select.ready();
            }
        }
        drop(select);
        while !closed {
            match commands.try_recv() {
                Ok(command) => pending.push_back(command),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    closed = true;
                    break;
                }
            }
        }
        let ok = hub.guarded(|hub| {
            let now = Instant::now();
            for event in events.try_iter() {
                hub.route(event, now);
            }
            // Every tab gone since the last wake-up leaves at once, in one update.
            let gone: Vec<u64> = hub
                .tabs
                .iter()
                .filter(|t| !matches!(t.stop.try_recv(), Err(TryRecvError::Empty)))
                .map(|t| t.id)
                .collect();
            hub.remove(&gone);
            while let Some(command) = pending.pop_front() {
                // A panic inside `Tab::open` (git2) is told through `opening`; one inside `add`
                // through the tab it already holds.
                *opening.borrow_mut() = Some(command.jobs.clone());
                let tab = Tab::open(command);
                *opening.borrow_mut() = None;
                if let Some(tab) = tab {
                    hub.add(tab);
                }
            }
            hub.flush(Instant::now());
        });
        if !ok {
            for jobs in opening
                .take()
                .into_iter()
                .chain(std::mem::take(&mut pending).into_iter().map(|c| c.jobs))
            {
                let _ = jobs.send(Job::WatchFailed(WATCHER_STOPPED.into()));
            }
        }
        if closed && hub.tabs.is_empty() {
            return;
        }
    }
}

/// Watches folders recursively. `notify` in production, a recorder in tests.
trait Backend {
    fn watch(&mut self, path: &Path) -> Result<(), String>;
    fn unwatch(&mut self, path: &Path);
}

struct Notify(notify::RecommendedWatcher);

impl Backend for Notify {
    fn watch(&mut self, path: &Path) -> Result<(), String> {
        self.0
            .watch(path, notify::RecursiveMode::Recursive)
            .map_err(|e| describe(&e))
    }

    fn unwatch(&mut self, path: &Path) {
        // A folder deleted meanwhile has lost its watch already.
        if let Err(e) = self.0.unwatch(path) {
            tracing::debug!(error = %e, "file watch already gone");
        }
    }
}

/// A watcher error as one plain sentence, without `notify`'s list of paths.
fn describe(e: &notify::Error) -> String {
    match &e.kind {
        notify::ErrorKind::MaxFilesWatch => {
            "The system's limit on watched folders is reached.".into()
        }
        notify::ErrorKind::PathNotFound => "A folder of the repository is gone.".into(),
        notify::ErrorKind::Io(io) => sentence(&io.to_string()),
        notify::ErrorKind::Generic(text) => sentence(text),
        notify::ErrorKind::WatchNotFound => "A folder's watch is gone.".into(),
        notify::ErrorKind::InvalidConfig(_) => "The watcher's settings are not valid.".into(),
    }
}

/// `text` with a capital first letter and a closing period.
fn sentence(text: &str) -> String {
    let text = text.trim().trim_end_matches('.');
    let mut chars = text.chars();
    let mut out: String = chars
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default();
    out.push_str(chars.as_str());
    out.push('.');
    out
}

/// One watched repository.
struct Tab {
    id: u64,
    root: PathBuf,
    /// The repository's git folder and, for a linked worktree, the common folder.
    git_dirs: Vec<PathBuf>,
    /// Whether a path relative to `root` is ignored by git.
    ignored: Box<dyn Fn(&Path) -> bool + Send>,
    jobs: Sender<Job>,
    stop: Receiver<()>,
    /// Dropped with the tab, which tells the worker the watch has ended.
    _ended: Sender<()>,
    /// When the tab asked to be watched.
    queued: Instant,
    full: bool,
    status: bool,
    due: Option<Instant>,
    /// A failure to watch one of its folders was reported; once is enough.
    told_failure: bool,
}

impl Tab {
    /// Opens the repository of `command` for its folders and ignore rules. A bare repository has
    /// no working tree to watch; a repository that cannot be opened is reported by its own
    /// thread, which fails the same way. A tab closed before its turn is not watched at all.
    fn open(command: Command) -> Option<Self> {
        if !matches!(command.stop.try_recv(), Err(TryRecvError::Empty)) {
            return None;
        }
        let _span = tracing::info_span!("repo", path = %command.path.display()).entered();
        let repo = match git2::Repository::open(&command.path) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e.message(), "no file watcher: cannot open the repository");
                return None;
            }
        };
        let root = repo.workdir()?.to_path_buf();
        let mut git_dirs = vec![repo.path().to_path_buf()];
        if !git_dirs.iter().any(|d| d == repo.commondir()) {
            git_dirs.push(repo.commondir().to_path_buf());
        }
        let mut tab = Self::new(
            root,
            git_dirs,
            Box::new(move |rel| repo.is_path_ignored(rel).unwrap_or(false)),
            command.jobs,
            command.stop,
            command.ended,
        );
        tab.queued = command.queued;
        Some(tab)
    }

    fn new(
        root: PathBuf,
        git_dirs: Vec<PathBuf>,
        ignored: Box<dyn Fn(&Path) -> bool + Send>,
        jobs: Sender<Job>,
        stop: Receiver<()>,
        ended: Sender<()>,
    ) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        Self {
            id: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            root,
            git_dirs,
            ignored,
            jobs,
            stop,
            _ended: ended,
            queued: Instant::now(),
            full: false,
            status: false,
            due: None,
            told_failure: false,
        }
    }

    /// The folders this tab needs watched: the working tree and the git folders outside it.
    fn wanted(&self) -> impl Iterator<Item = &Path> {
        std::iter::once(self.root.as_path()).chain(
            self.git_dirs
                .iter()
                .filter(|d| !d.starts_with(&self.root))
                .map(PathBuf::as_path),
        )
    }

    /// Tells the tab once; whether this was the first time.
    fn report(&mut self, error: &str) -> bool {
        if self.told_failure {
            return false;
        }
        self.told_failure = true;
        let _ = self.jobs.send(Job::WatchFailed(error.to_owned()));
        true
    }
}

/// The outermost of `paths`: those not inside another one. Inside means by whole components, so
/// `/a/bc` is not inside `/a/b`.
fn outermost<'a>(paths: impl IntoIterator<Item = &'a Path>) -> BTreeSet<PathBuf> {
    let all: BTreeSet<&Path> = paths.into_iter().collect();
    all.iter()
        .filter(|p| !all.iter().any(|q| q != *p && p.starts_with(q)))
        .map(|p| p.to_path_buf())
        .collect()
}

/// What the watcher thread knows: the backend, the tabs and the folders actually watched.
struct Hub<B, F> {
    make: F,
    backend: Option<B>,
    tabs: Vec<Tab>,
    watched: BTreeSet<PathBuf>,
    /// Every tab folder, to the tabs it belongs to and whether it is a git folder.
    index: HashMap<PathBuf, Vec<(u64, bool)>>,
    /// Folders that could not be watched, with why. They are tried again only after a tab
    /// leaves (which frees watches), not at every update: retrying means unwatching the folders
    /// inside them first, and a refresh of their tabs each time.
    failed: std::collections::BTreeMap<PathBuf, String>,
}

impl<B: Backend, F: FnMut() -> Result<B, String>> Hub<B, F> {
    fn new(make: F) -> Self {
        Self {
            make,
            backend: None,
            tabs: Vec::new(),
            watched: BTreeSet::new(),
            index: HashMap::new(),
            failed: std::collections::BTreeMap::new(),
        }
    }

    /// Runs `step`; if it panics (inside the backend, say), the tabs watched until then are told
    /// their changes are no longer picked up and leave, and the backend is dropped so the next
    /// repository starts a fresh one.
    ///
    /// Returns whether `step` finished without a panic.
    fn guarded(&mut self, step: impl FnOnce(&mut Self)) -> bool {
        if std::panic::catch_unwind(AssertUnwindSafe(|| step(self))).is_ok() {
            return true;
        }
        tracing::warn!("no file watcher: the watcher failed");
        for tab in &mut self.tabs {
            tab.report(WATCHER_STOPPED);
        }
        self.tabs.clear();
        self.index.clear();
        self.watched.clear();
        self.failed.clear();
        self.backend = None;
        false
    }

    fn add(&mut self, tab: Tab) {
        let id = tab.id;
        let queued = tab.queued;
        self.tabs.push(tab);
        self.update(Some(id));
        // Waiting behind other repositories' registrations, it may have read the repository
        // before its watches existed.
        let now = Instant::now();
        if now.saturating_duration_since(queued) > LATE_REGISTRATION
            && let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id)
        {
            tab.full = true;
            tab.due.get_or_insert(now);
        }
    }

    fn remove(&mut self, ids: &[u64]) {
        if ids.is_empty() {
            return;
        }
        self.tabs.retain(|t| !ids.contains(&t.id));
        self.failed.clear();
        self.update(None);
    }

    /// Brings the watched folders in line with the tabs: unwatches what is no longer outermost
    /// first, then watches what now is, so an inner watch never removes an outer one's folders.
    /// A folder that cannot be watched is unwatched again (its partial watches would hold the
    /// limit that failed it) and the wanted folders inside it are tried on their own. Tabs
    /// (other than `newcomer`, which reads everything anyway) whose folders were unwatched for a
    /// moment refresh. A failed folder is tried again after a tab leaves (see `failed`).
    fn update(&mut self, newcomer: Option<u64>) {
        self.index.clear();
        for tab in &self.tabs {
            self.index
                .entry(tab.root.clone())
                .or_default()
                .push((tab.id, false));
            for dir in &tab.git_dirs {
                self.index
                    .entry(dir.clone())
                    .or_default()
                    .push((tab.id, true));
            }
        }
        if self.backend.is_none() && !self.tabs.is_empty() {
            match (self.make)() {
                Ok(backend) => self.backend = Some(backend),
                Err(e) => {
                    tracing::warn!(error = %e, "no file watcher");
                    for tab in &mut self.tabs {
                        tab.report(&e);
                    }
                    return;
                }
            }
        }
        let Some(backend) = self.backend.as_mut() else {
            return;
        };
        let wanted: BTreeSet<PathBuf> = self
            .tabs
            .iter()
            .flat_map(Tab::wanted)
            .map(Path::to_path_buf)
            .collect();
        self.failed.retain(|f, _| wanted.contains(f));
        let mut gone: Vec<PathBuf> = Vec::new();
        loop {
            let desired = outermost(
                wanted
                    .iter()
                    .filter(|w| !self.failed.contains_key(*w))
                    .map(PathBuf::as_path),
            );
            let stale: BTreeSet<PathBuf> = self.watched.difference(&desired).cloned().collect();
            self.watched.retain(|p| desired.contains(p));
            for path in stale {
                backend.unwatch(&path);
                gone.push(path);
            }
            let new: Vec<PathBuf> = desired.difference(&self.watched).cloned().collect();
            if new.is_empty() {
                break;
            }
            let mut any_failed = false;
            for path in new {
                match backend.watch(&path) {
                    Ok(()) => {
                        self.watched.insert(path);
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, folder = %path.display(), "no file watcher for a folder");
                        backend.unwatch(&path);
                        self.failed.insert(path, e);
                        any_failed = true;
                    }
                }
            }
            // Without a failure everything desired is watched; with one, the folders inside the
            // failed one are now outermost and get their own try.
            if !any_failed {
                break;
            }
        }
        let now = Instant::now();
        for tab in &mut self.tabs {
            let reason = tab.wanted().find_map(|w| self.failed.get(w)).cloned();
            if let Some(e) = reason {
                tab.report(&e);
            }
            let was_unwatched = gone.iter().any(|g| tab.wanted().any(|w| w.starts_with(g)));
            if was_unwatched && Some(tab.id) != newcomer {
                tab.full = true;
                tab.due.get_or_insert(now);
            }
        }
    }

    /// The tabs whose folders hold `path`, each with whether its deepest such folder is a git
    /// folder and `path` relative to it.
    fn owners<'p>(&self, path: &'p Path) -> Vec<(u64, bool, &'p Path)> {
        let mut found: Vec<(u64, bool, &'p Path)> = Vec::new();
        for folder in path.ancestors() {
            let (Some(owners), Ok(rel)) = (self.index.get(folder), path.strip_prefix(folder))
            else {
                continue;
            };
            for &(id, git) in owners {
                if !found.iter().any(|(seen, _, _)| *seen == id) {
                    found.push((id, git, rel));
                }
            }
        }
        found
    }

    /// Notes which tabs a change concerns and pushes back their quiet deadline. A watch error
    /// (a folder created later that cannot be watched) is logged and told to its tabs, or to
    /// every tab when it names no folder.
    fn route(&mut self, event: notify::Result<notify::Event>, now: Instant) {
        let event = match event {
            Ok(event) => event,
            Err(e) => {
                let text = describe(&e);
                let ids: Vec<u64> = e
                    .paths
                    .iter()
                    .flat_map(|p| self.owners(p))
                    .map(|(id, _, _)| id)
                    .collect();
                let mut told = false;
                for tab in &mut self.tabs {
                    if e.paths.is_empty() || ids.contains(&tab.id) {
                        told |= tab.report(&text);
                    }
                }
                // One line per tab told, not one per folder an install creates.
                if told {
                    tracing::warn!(error = %text, "no file watcher for a new folder");
                } else {
                    tracing::debug!(error = %text, "no file watcher for a new folder");
                }
                return;
            }
        };
        if event.need_rescan() {
            // The system dropped changes: every tab reads everything again, a step apart.
            let mut at = now + DEBOUNCE;
            for tab in &mut self.tabs {
                tab.full = true;
                tab.due = Some(at);
                at += RESCAN_STEP;
            }
            return;
        }
        if matches!(event.kind, notify::EventKind::Access(_)) {
            return;
        }
        for path in &event.paths {
            // The deepest folder a tab owns decides: a git folder inside the working tree wins.
            for (id, git, rel) in self.owners(path) {
                let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else {
                    continue;
                };
                if git {
                    tab.full |= relevant_git_path(rel);
                } else {
                    tab.status |= !(tab.ignored)(rel);
                }
                tab.due = Some(tab.due.map_or(now + DEBOUNCE, |d| d.max(now + DEBOUNCE)));
            }
        }
    }

    /// The soonest moment a tab's changes have settled.
    fn next_due(&self) -> Option<Instant> {
        self.tabs.iter().filter_map(|t| t.due).min()
    }

    /// Tells every tab whose changes have settled to refresh; a tab whose worker is gone stops
    /// being watched.
    fn flush(&mut self, now: Instant) {
        let mut ended = Vec::new();
        for tab in &mut self.tabs {
            if tab.due.is_none_or(|due| due > now) {
                continue;
            }
            let job = if tab.full {
                Some(Job::Refresh(0))
            } else if tab.status {
                Some(Job::RefreshStatus)
            } else {
                None
            };
            tab.full = false;
            tab.status = false;
            tab.due = None;
            if let Some(job) = job
                && tab.jobs.send(job).is_err()
            {
                ended.push(tab.id);
            }
        }
        self.remove(&ended);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Backend, Command, DEBOUNCE, Hub, LATE_REGISTRATION, RESCAN_STEP, Tab, outermost, sentence,
    };
    use crate::worker::Job;
    use crossbeam_channel::{Receiver, Sender, unbounded};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// Records watch calls; `fail` names folders that cannot be watched, `panic` one whose watch
    /// panics.
    #[derive(Default)]
    struct Recorder {
        log: Arc<Mutex<Vec<String>>>,
        fail: Vec<PathBuf>,
        panic: Option<PathBuf>,
    }

    impl Backend for Recorder {
        fn watch(&mut self, path: &Path) -> Result<(), String> {
            assert!(self.panic.as_deref() != Some(path), "backend failure");
            if self.fail.iter().any(|f| f == path) {
                return Err(format!("Cannot watch {}.", path.display()));
            }
            self.log
                .lock()
                .unwrap()
                .push(format!("+{}", path.display()));
            Ok(())
        }

        fn unwatch(&mut self, path: &Path) {
            self.log
                .lock()
                .unwrap()
                .push(format!("-{}", path.display()));
        }
    }

    /// A tab on `root` with these git folders, nothing ignored but `*.log`.
    fn tab_with(root: &str, git_dirs: &[&str]) -> (Tab, Receiver<Job>, Sender<()>) {
        let (jobs, rx) = unbounded();
        let (stop_tx, stop) = crossbeam_channel::bounded(0);
        let (ended, _) = crossbeam_channel::bounded(0);
        let tab = Tab::new(
            PathBuf::from(root),
            git_dirs.iter().map(PathBuf::from).collect(),
            Box::new(|rel: &Path| rel.extension().is_some_and(|e| e == "log")),
            jobs,
            stop,
            ended,
        );
        (tab, rx, stop_tx)
    }

    /// A tab on `root` with its git folder at `root/.git`.
    fn tab(root: &str) -> (Tab, Receiver<Job>, Sender<()>) {
        tab_with(root, &[&format!("{root}/.git")])
    }

    fn hub(
        log: &Arc<Mutex<Vec<String>>>,
        fail: &[&str],
    ) -> Hub<Recorder, impl FnMut() -> Result<Recorder, String>> {
        let log = Arc::clone(log);
        let fail: Vec<PathBuf> = fail.iter().map(PathBuf::from).collect();
        Hub::new(move || {
            Ok(Recorder {
                log: Arc::clone(&log),
                fail: fail.clone(),
                panic: None,
            })
        })
    }

    fn change(path: &str) -> notify::Event {
        notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Any))
            .add_path(PathBuf::from(path))
    }

    fn jobs(rx: &Receiver<Job>) -> Vec<String> {
        rx.try_iter().map(|j| format!("{j:?}")).collect()
    }

    fn logged(log: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
        log.lock().unwrap().clone()
    }

    #[test]
    fn only_the_outermost_folders_are_watched() {
        let paths = [
            "/a",
            "/a/sub",
            "/a/sub/deeper",
            "/ab",
            "/b/.git",
            "/b",
            "/a",
        ];
        let got = outermost(paths.iter().map(Path::new));
        let want: Vec<PathBuf> = ["/a", "/ab", "/b"].iter().map(PathBuf::from).collect();
        assert_eq!(got.into_iter().collect::<Vec<_>>(), want);
    }

    #[test]
    fn an_inner_repository_never_removes_the_outer_watch() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut hub = hub(&log, &[]);
        let (inner, _inner_rx, _s1) = tab("/p/sub");
        let inner_id = inner.id;
        hub.add(inner);
        let (outer, outer_rx, _s2) = tab("/p");
        hub.add(outer);
        // The outer folder replaces the inner watch: unwatch first, then watch.
        assert_eq!(logged(&log), ["+/p/sub", "-/p/sub", "+/p"]);
        hub.remove(&[inner_id]);
        // Closing the inner tab leaves the outer watch alone.
        assert_eq!(logged(&log), ["+/p/sub", "-/p/sub", "+/p"]);
        hub.route(Ok(change("/p/file.rs")), Instant::now());
        hub.flush(Instant::now() + DEBOUNCE);
        assert_eq!(jobs(&outer_rx), ["RefreshStatus"]);
    }

    #[test]
    fn closing_the_outer_repository_watches_the_inner_one_again_and_refreshes_it() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut hub = hub(&log, &[]);
        let (outer, _outer_rx, _s1) = tab("/p");
        let outer_id = outer.id;
        hub.add(outer);
        let (inner, inner_rx, _s2) = tab("/p/sub");
        hub.add(inner);
        assert_eq!(logged(&log), ["+/p"], "the inner one is covered");
        hub.remove(&[outer_id]);
        assert_eq!(logged(&log), ["+/p", "-/p", "+/p/sub"]);
        // Its folders went unwatched for a moment: it reads everything again.
        hub.flush(Instant::now());
        assert_eq!(jobs(&inner_rx), ["Refresh(0)"]);
    }

    #[test]
    fn two_tabs_on_one_repository_share_its_watch() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut hub = hub(&log, &[]);
        let (first, _rx1, _s1) = tab("/p");
        let first_id = first.id;
        hub.add(first);
        let (second, second_rx, _s2) = tab("/p");
        hub.add(second);
        hub.remove(&[first_id]);
        assert_eq!(logged(&log), ["+/p"], "the watch stays for the other tab");
        hub.route(Ok(change("/p/.git/HEAD")), Instant::now());
        hub.flush(Instant::now() + DEBOUNCE);
        assert_eq!(jobs(&second_rx), ["Refresh(0)"]);
    }

    #[test]
    fn a_linked_worktree_sees_its_common_folder_inside_another_tab() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut hub = hub(&log, &[]);
        let (main, main_rx, _s1) = tab("/main");
        let main_id = main.id;
        hub.add(main);
        let (linked, linked_rx, _s2) = tab_with("/wt", &["/main/.git/worktrees/wt", "/main/.git"]);
        hub.add(linked);
        assert_eq!(logged(&log), ["+/main", "+/wt"]);
        let now = Instant::now();
        // A branch moved in the shared folder concerns both; the worktree's own HEAD only it.
        hub.route(Ok(change("/main/.git/refs/heads/topic")), now);
        hub.flush(now + DEBOUNCE);
        assert_eq!(jobs(&main_rx), ["Refresh(0)"]);
        assert_eq!(jobs(&linked_rx), ["Refresh(0)"]);
        hub.route(Ok(change("/main/.git/worktrees/wt/HEAD")), now);
        hub.flush(now + DEBOUNCE);
        assert_eq!(jobs(&linked_rx), ["Refresh(0)"]);
        // The main tab's tree is not the worktree's: closing it keeps the git folders watched.
        hub.remove(&[main_id]);
        assert_eq!(logged(&log), ["+/main", "+/wt", "-/main", "+/main/.git"]);
    }

    #[test]
    fn changes_reach_every_tab_they_belong_to_debounced() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut hub = hub(&log, &[]);
        let (outer, outer_rx, _s1) = tab("/p");
        hub.add(outer);
        let (inner, inner_rx, _s2) = tab("/p/sub");
        hub.add(inner);
        let (other, other_rx, _s3) = tab("/q");
        hub.add(other);
        let now = Instant::now();
        hub.route(Ok(change("/p/sub/src/lib.rs")), now);
        hub.route(Ok(change("/p/sub/.git/HEAD")), now);
        hub.route(Ok(change("/q/.git/objects/ab/cdef")), now);
        hub.flush(now);
        assert_eq!(
            jobs(&outer_rx),
            Vec::<String>::new(),
            "nothing before the quiet period"
        );
        hub.flush(now + DEBOUNCE);
        // The outer tree sees a working-tree change, the inner repository a new HEAD; an object
        // write says nothing.
        assert_eq!(jobs(&outer_rx), ["RefreshStatus"]);
        assert_eq!(jobs(&inner_rx), ["Refresh(0)"]);
        assert_eq!(jobs(&other_rx), Vec::<String>::new());
    }

    #[test]
    fn ignored_files_and_reads_change_nothing() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut hub = hub(&log, &[]);
        let (t, rx, _s) = tab("/p");
        hub.add(t);
        let now = Instant::now();
        hub.route(Ok(change("/p/build.log")), now);
        hub.route(
            Ok(
                notify::Event::new(notify::EventKind::Access(notify::event::AccessKind::Any))
                    .add_path(PathBuf::from("/p/src/lib.rs")),
            ),
            now,
        );
        hub.flush(now + DEBOUNCE);
        assert_eq!(jobs(&rx), Vec::<String>::new());
    }

    #[test]
    fn a_folder_that_cannot_be_watched_is_reported_once_and_tried_again() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut hub = hub(&log, &["/p"]);
        let (t, rx, _s) = tab("/p");
        hub.add(t);
        let (t2, rx2, _s2) = tab("/q");
        hub.add(t2);
        assert_eq!(jobs(&rx), ["WatchFailed(\"Cannot watch /p.\")"]);
        assert_eq!(jobs(&rx2), Vec::<String>::new());
        // Not retried while tabs only arrive; tried again (its partial watches dropped again)
        // once a tab leaves and frees watches, and not reported twice.
        let (t3, _rx3, _s3) = tab("/r");
        let r = t3.id;
        hub.add(t3);
        assert_eq!(logged(&log), ["-/p", "+/q", "+/r"]);
        hub.remove(&[r]);
        assert_eq!(jobs(&rx), Vec::<String>::new());
        assert_eq!(logged(&log), ["-/p", "+/q", "+/r", "-/r", "-/p"]);
    }

    #[test]
    fn a_repository_inside_one_that_cannot_be_watched_is_watched_on_its_own() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut hub = hub(&log, &["/big"]);
        let (outer, outer_rx, _s1) = tab("/big");
        hub.add(outer);
        let (inner, inner_rx, _s2) = tab("/big/sub");
        hub.add(inner);
        assert_eq!(jobs(&outer_rx), ["WatchFailed(\"Cannot watch /big.\")"]);
        assert_eq!(
            jobs(&inner_rx),
            Vec::<String>::new(),
            "the inner one is watched"
        );
        assert_eq!(logged(&log), ["-/big", "+/big/sub"]);
        // Another tab arriving leaves both alone: no unwatching, no refresh of the inner one.
        let (other, _other_rx, _s3) = tab("/q");
        hub.add(other);
        hub.flush(Instant::now());
        assert_eq!(logged(&log), ["-/big", "+/big/sub", "+/q"]);
        assert_eq!(jobs(&inner_rx), Vec::<String>::new());
    }

    #[test]
    fn failures_nested_two_deep_are_each_reported_and_the_rest_watched() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut hub = hub(&log, &["/big", "/big/a"]);
        let (big, big_rx, _s1) = tab("/big");
        let (a, a_rx, _s2) = tab("/big/a");
        let (b, b_rx, _s3) = tab("/big/b");
        hub.tabs.extend([big, a, b]);
        hub.update(None);
        assert_eq!(jobs(&big_rx), ["WatchFailed(\"Cannot watch /big.\")"]);
        assert_eq!(jobs(&a_rx), ["WatchFailed(\"Cannot watch /big/a.\")"]);
        assert_eq!(jobs(&b_rx), Vec::<String>::new());
        assert_eq!(logged(&log), ["-/big", "-/big/a", "+/big/b"]);
    }

    #[test]
    fn a_watcher_that_cannot_start_is_reported_and_created_later() {
        let mut attempts = 0;
        let mut hub = Hub::new(move || {
            attempts += 1;
            if attempts == 1 {
                Err("Too many inotify instances.".to_owned())
            } else {
                Ok(Recorder::default())
            }
        });
        let (t, rx, _s) = tab("/p");
        hub.add(t);
        assert_eq!(jobs(&rx), ["WatchFailed(\"Too many inotify instances.\")"]);
        let (t2, _rx2, _s2) = tab("/q");
        hub.add(t2);
        assert!(hub.backend.is_some());
        assert!(hub.watched.contains(Path::new("/p")));
    }

    #[test]
    fn a_watch_error_later_is_told_to_the_tabs_it_concerns() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut hub = hub(&log, &[]);
        let (p, p_rx, _s1) = tab("/p");
        hub.add(p);
        let (q, q_rx, _s2) = tab("/q");
        hub.add(q);
        let error = notify::Error::new(notify::ErrorKind::MaxFilesWatch)
            .add_path(PathBuf::from("/p/target/new"));
        hub.route(Err(error), Instant::now());
        assert_eq!(
            jobs(&p_rx),
            ["WatchFailed(\"The system's limit on watched folders is reached.\")"]
        );
        assert_eq!(jobs(&q_rx), Vec::<String>::new());
        // One with no folder concerns every tab.
        hub.route(
            Err(notify::Error::generic("inotify read failed")),
            Instant::now(),
        );
        assert_eq!(jobs(&q_rx), ["WatchFailed(\"Inotify read failed.\")"]);
    }

    #[test]
    fn a_tab_registered_late_reads_everything_again() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut hub = hub(&log, &[]);
        let (mut t, rx, _s) = tab("/p");
        t.queued = Instant::now()
            .checked_sub(LATE_REGISTRATION * 2)
            .unwrap_or_else(Instant::now);
        hub.add(t);
        hub.flush(Instant::now());
        assert_eq!(jobs(&rx), ["Refresh(0)"]);
        let (on_time, rx2, _s2) = tab("/q");
        hub.add(on_time);
        hub.flush(Instant::now());
        assert_eq!(jobs(&rx2), Vec::<String>::new());
    }

    #[test]
    fn a_tab_whose_worker_is_gone_stops_being_watched() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut hub = hub(&log, &[]);
        let (t, rx, _s) = tab("/p");
        hub.add(t);
        drop(rx);
        let now = Instant::now();
        hub.route(Ok(change("/p/a.rs")), now);
        hub.flush(now + DEBOUNCE);
        assert!(hub.tabs.is_empty());
        assert_eq!(logged(&log), ["+/p", "-/p"]);
    }

    #[test]
    fn a_dropped_change_queue_refreshes_every_tab_a_step_apart() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut hub = hub(&log, &[]);
        let (t, rx, _s) = tab("/p");
        hub.add(t);
        let (t2, rx2, _s2) = tab("/q");
        hub.add(t2);
        let now = Instant::now();
        hub.route(
            Ok(notify::Event::new(notify::EventKind::Other).set_flag(notify::event::Flag::Rescan)),
            now,
        );
        hub.flush(now + DEBOUNCE);
        assert_eq!(jobs(&rx), ["Refresh(0)"]);
        assert_eq!(jobs(&rx2), Vec::<String>::new(), "not in the same moment");
        hub.flush(now + DEBOUNCE + RESCAN_STEP);
        assert_eq!(jobs(&rx2), ["Refresh(0)"]);
    }

    #[test]
    fn a_failing_backend_tells_its_tabs_and_the_next_repository_starts_afresh() {
        let mut made = 0;
        let mut hub = Hub::new(move || {
            made += 1;
            Ok(Recorder {
                panic: (made == 1).then(|| PathBuf::from("/boom")),
                ..Recorder::default()
            })
        });
        let (p, p_rx, _s1) = tab("/p");
        hub.guarded(|hub| hub.add(p));
        let (boom, _boom_rx, _s2) = tab("/boom");
        hub.guarded(|hub| hub.add(boom));
        assert_eq!(
            jobs(&p_rx),
            ["WatchFailed(\"Canopy's file watcher stopped.\")"]
        );
        assert!(hub.tabs.is_empty());
        let (q, _q_rx, _s3) = tab("/q");
        hub.guarded(|hub| hub.add(q));
        assert!(hub.watched.contains(Path::new("/q")));
    }

    #[test]
    fn errors_read_as_sentences() {
        assert_eq!(sentence("inotify read failed"), "Inotify read failed.");
        assert_eq!(sentence("Too many open files."), "Too many open files.");
        assert_eq!(
            super::describe(
                &notify::Error::new(notify::ErrorKind::MaxFilesWatch).add_path(PathBuf::from("/p"))
            ),
            "The system's limit on watched folders is reached."
        );
    }

    /// The loop itself: a tab closed before its turn is never watched, and the thread ends when
    /// nothing can reach it any more.
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "tests drive git directly to build repositories"
    )]
    fn a_tab_closed_before_its_turn_is_not_watched() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let made = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()?;
        assert!(made.success(), "git init");
        let log = Arc::new(Mutex::new(Vec::new()));
        let (commands_tx, commands) = unbounded::<Command>();
        let (_events_tx, events) = unbounded();
        let thread_log = Arc::clone(&log);
        let thread = std::thread::spawn(move || {
            super::run(&commands, &events, move || {
                Ok(Recorder {
                    log: Arc::clone(&thread_log),
                    ..Recorder::default()
                })
            });
        });
        let (jobs, _jobs_rx) = unbounded();
        let (stop_tx, stop) = crossbeam_channel::bounded::<()>(0);
        let (ended, ended_rx) = crossbeam_channel::bounded::<()>(0);
        drop(stop_tx);
        commands_tx.send(Command {
            path: dir.path().to_path_buf(),
            jobs,
            stop,
            ended,
            queued: Instant::now(),
        })?;
        assert!(matches!(
            ended_rx.recv_timeout(Duration::from_secs(10)),
            Err(crossbeam_channel::RecvTimeoutError::Disconnected)
        ));
        drop(commands_tx);
        thread
            .join()
            .map_err(|_panic| "the watcher thread panicked")?;
        assert_eq!(logged(&log), Vec::<String>::new());
        Ok(())
    }

    /// A panic while repositories wait their turn: the one being taken in and every one still
    /// queued are told, none silently left unwatched.
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "tests drive git directly to build repositories"
    )]
    fn a_panic_tells_every_repository_still_waiting() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let (commands_tx, commands) = unbounded::<Command>();
        let mut receivers = Vec::new();
        let mut keep = Vec::new();
        for name in ["a", "b", "c"] {
            let repo = dir.path().join(name);
            std::fs::create_dir_all(&repo)?;
            let made = std::process::Command::new("git")
                .args(["init", "-q"])
                .current_dir(&repo)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .status()?;
            assert!(made.success(), "git init");
            let (jobs, rx) = unbounded();
            let (stop_tx, stop) = crossbeam_channel::bounded::<()>(0);
            let (ended, ended_rx) = crossbeam_channel::bounded::<()>(0);
            keep.push((stop_tx, ended_rx));
            receivers.push(rx);
            commands_tx.send(Command {
                path: repo,
                jobs,
                stop,
                ended,
                queued: Instant::now(),
            })?;
        }
        // All three wait before the loop starts; the first one's watch panics.
        let first = dir.path().join("a");
        let (_events_tx, events) = unbounded();
        let thread = std::thread::spawn(move || {
            super::run(&commands, &events, move || {
                Ok(Recorder {
                    panic: Some(first.clone()),
                    ..Recorder::default()
                })
            });
        });
        for rx in &receivers {
            let job = rx.recv_timeout(Duration::from_secs(10))?;
            assert_eq!(
                format!("{job:?}"),
                "WatchFailed(\"Canopy's file watcher stopped.\")"
            );
        }
        // Each is told once, the first one (which panicked) included.
        for rx in &receivers {
            assert!(rx.try_recv().is_err(), "told twice");
        }
        drop(commands_tx);
        drop(keep);
        thread
            .join()
            .map_err(|_panic| "the watcher thread panicked")?;
        Ok(())
    }

    /// The real watcher: closing the outer of two nested repositories must not take the inner
    /// one's watches with it (`notify` shares one inotify watch per folder), and both share one
    /// inotify instance.
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "tests drive git directly to build repositories"
    )]
    fn closing_an_outer_repository_keeps_the_inner_one_watched()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let outer = dir.path().join("outer");
        let inner = outer.join("inner");
        for repo in [&outer, &inner] {
            std::fs::create_dir_all(repo)?;
            let made = std::process::Command::new("git")
                .args(["init", "-q"])
                .current_dir(repo)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .status()?;
            assert!(made.success(), "git init");
        }
        let (outer_jobs, _outer_rx) = unbounded();
        let (outer_stop, stop) = crossbeam_channel::bounded::<()>(0);
        let (ended, outer_ended) = crossbeam_channel::bounded::<()>(0);
        super::watch(outer, outer_jobs, stop, ended);
        let (inner_jobs, inner_rx) = unbounded();
        let (_inner_stop, stop) = crossbeam_channel::bounded::<()>(0);
        let (ended, _inner_ended) = crossbeam_channel::bounded::<()>(0);
        super::watch(inner.clone(), inner_jobs, stop, ended);
        // Both are registered, in order, once a change in the inner one reaches it.
        // A change made before the inner repository is registered reaches no one, so keep
        // making them until one arrives (or ten seconds pass).
        let status_after = |name: &str| -> Result<bool, Box<dyn std::error::Error>> {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut attempt = 0;
            while Instant::now() < deadline {
                attempt += 1;
                std::fs::write(inner.join(format!("{name}-{attempt}.txt")), "x")?;
                let wait = (Instant::now() + DEBOUNCE * 3).min(deadline);
                while let Ok(job) = inner_rx.recv_deadline(wait) {
                    if matches!(job, Job::RefreshStatus) {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        };
        assert!(status_after("probe")?, "the inner repository is watched");
        #[cfg(target_os = "linux")]
        {
            let inotify = std::fs::read_dir("/proc/self/fd")?
                .filter_map(|e| std::fs::read_link(e.ok()?.path()).ok())
                .filter(|l| l.to_string_lossy().contains("inotify"))
                .count();
            assert_eq!(inotify, 1, "one inotify instance serves both repositories");
        }
        drop(outer_stop);
        // The outer watch is gone once its `ended` disconnects.
        assert!(matches!(
            outer_ended.recv_timeout(Duration::from_secs(10)),
            Err(crossbeam_channel::RecvTimeoutError::Disconnected)
        ));
        // The inner one refreshes for the moment it went unwatched; let that pass.
        std::thread::sleep(DEBOUNCE * 2);
        let _ = inner_rx.try_iter().count();
        assert!(
            status_after("new")?,
            "a change in the inner repository still reaches it"
        );
        Ok(())
    }
}
