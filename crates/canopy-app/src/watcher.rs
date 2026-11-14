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
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Select, Sender, unbounded};

use crate::worker::Job;

/// How long a tab's changes must stay quiet before it is told to refresh.
const DEBOUNCE: Duration = Duration::from_millis(350);

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
    let Some(hub) = hub() else {
        tracing::warn!("no file watcher: its thread could not start");
        let _ = jobs.send(Job::WatchFailed(
            "Canopy could not start its file watcher".into(),
        ));
        return;
    };
    if hub
        .send(Command {
            path,
            jobs,
            stop,
            ended,
        })
        .is_err()
    {
        tracing::warn!("no file watcher: its thread has ended");
    }
}

/// A repository to watch, as [`watch`] hands it to the watcher thread.
struct Command {
    path: PathBuf,
    jobs: Sender<Job>,
    stop: Receiver<()>,
    ended: Sender<()>,
}

/// The watcher thread's inbox, started with the first watch.
fn hub() -> Option<&'static Sender<Command>> {
    static HUB: OnceLock<Option<Sender<Command>>> = OnceLock::new();
    HUB.get_or_init(|| {
        let (tx, rx) = unbounded::<Command>();
        match std::thread::Builder::new()
            .name("watcher".into())
            .spawn(move || run(&rx))
        {
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
/// tells each tab once its changes have settled.
fn run(commands: &Receiver<Command>) {
    let (event_tx, events) = unbounded::<notify::Result<notify::Event>>();
    let mut hub = Hub::new(move || {
        use notify::Watcher as _;
        let tx = event_tx.clone();
        let config = notify::Config::default().with_follow_symlinks(false);
        let mut watcher = notify::RecommendedWatcher::new(
            move |e| {
                let _ = tx.send(e);
            },
            config,
        )
        .map_err(|e| e.to_string())?;
        // Settings a watcher could not apply leave the defaults, which still work.
        let _ = watcher.configure(config);
        Ok(Notify(watcher))
    });
    loop {
        let stops: Vec<(u64, Receiver<()>)> =
            hub.tabs.iter().map(|t| (t.id, t.stop.clone())).collect();
        let mut select = Select::new();
        let command_index = select.recv(commands);
        let event_index = select.recv(&events);
        let stop_indices: Vec<usize> = stops.iter().map(|(_, s)| select.recv(s)).collect();
        let picked = match hub.next_due() {
            Some(due) => select
                .select_timeout(due.saturating_duration_since(Instant::now()))
                .ok(),
            None => Some(select.select()),
        };
        if let Some(op) = picked {
            let index = op.index();
            if index == command_index {
                match op.recv(commands) {
                    Ok(command) => {
                        if let Some(tab) = Tab::open(command) {
                            hub.add(tab);
                        }
                    }
                    Err(_) => return,
                }
            } else if index == event_index {
                if let Ok(event) = op.recv(&events) {
                    hub.route(event, Instant::now());
                }
            } else if let Some(at) = stop_indices.iter().position(|&i| i == index) {
                let (id, stop) = &stops[at];
                let _ = op.recv(stop);
                hub.remove(*id);
            }
        }
        hub.flush(Instant::now());
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
        use notify::Watcher as _;
        self.0
            .watch(path, notify::RecursiveMode::Recursive)
            .map_err(|e| e.to_string())
    }

    fn unwatch(&mut self, path: &Path) {
        use notify::Watcher as _;
        // A folder deleted meanwhile has lost its watch already.
        if let Err(e) = self.0.unwatch(path) {
            tracing::debug!(error = %e, "file watch already gone");
        }
    }
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
    full: bool,
    status: bool,
    due: Option<Instant>,
    /// A failure to watch one of its folders was reported; once is enough.
    told_failure: bool,
}

impl Tab {
    /// Opens the repository of `command` for its folders and ignore rules. A bare repository has
    /// no working tree to watch; a repository that cannot be opened is reported by its own
    /// thread, which fails the same way.
    fn open(command: Command) -> Option<Self> {
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
        Some(Self::new(
            root,
            git_dirs,
            Box::new(move |rel| repo.is_path_ignored(rel).unwrap_or(false)),
            command.jobs,
            command.stop,
            command.ended,
        ))
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

    fn report(&mut self, error: &str) {
        if !self.told_failure {
            self.told_failure = true;
            let _ = self.jobs.send(Job::WatchFailed(error.to_owned()));
        }
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
}

impl<B: Backend, F: FnMut() -> Result<B, String>> Hub<B, F> {
    fn new(make: F) -> Self {
        Self {
            make,
            backend: None,
            tabs: Vec::new(),
            watched: BTreeSet::new(),
            index: HashMap::new(),
        }
    }

    fn add(&mut self, tab: Tab) {
        let id = tab.id;
        self.tabs.push(tab);
        self.update(Some(id));
    }

    fn remove(&mut self, id: u64) {
        self.tabs.retain(|t| t.id != id);
        self.update(None);
    }

    /// Brings the watched folders in line with the tabs: unwatches what is no longer outermost
    /// first, then watches what now is, so an inner watch never removes an outer one's folders.
    /// Tabs (other than `newcomer`, which reads everything anyway) whose folders were unwatched
    /// for a moment refresh. A folder that could not be watched is tried again next time.
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
        let desired = outermost(self.tabs.iter().flat_map(Tab::wanted));
        let gone: Vec<PathBuf> = self.watched.difference(&desired).cloned().collect();
        let new: Vec<PathBuf> = desired.difference(&self.watched).cloned().collect();
        for path in &gone {
            backend.unwatch(path);
            self.watched.remove(path);
        }
        let mut failed = Vec::new();
        for path in new {
            match backend.watch(&path) {
                Ok(()) => {
                    self.watched.insert(path);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "no file watcher for a repository folder");
                    failed.push((path, e));
                }
            }
        }
        let now = Instant::now();
        for tab in &mut self.tabs {
            for (path, e) in &failed {
                if tab.wanted().any(|w| w.starts_with(path)) {
                    tab.report(e);
                }
            }
            let was_unwatched = gone.iter().any(|g| tab.wanted().any(|w| w.starts_with(g)));
            if was_unwatched && Some(tab.id) != newcomer {
                tab.full = true;
                tab.due.get_or_insert(now);
            }
        }
    }

    /// Notes which tabs a change concerns and pushes back their quiet deadline.
    fn route(&mut self, event: notify::Result<notify::Event>, now: Instant) {
        let Ok(event) = event else { return };
        if event.need_rescan() {
            // The system dropped changes: every tab reads everything again.
            for tab in &mut self.tabs {
                tab.full = true;
                tab.due = Some(now + DEBOUNCE);
            }
            return;
        }
        if matches!(event.kind, notify::EventKind::Access(_)) {
            return;
        }
        for path in &event.paths {
            // The deepest folder a tab owns decides: a git folder inside the working tree wins.
            let mut seen: Vec<u64> = Vec::new();
            for folder in path.ancestors() {
                let Some(owners) = self.index.get(folder) else {
                    continue;
                };
                let Ok(rel) = path.strip_prefix(folder) else {
                    continue;
                };
                for &(id, git) in owners {
                    if seen.contains(&id) {
                        continue;
                    }
                    seen.push(id);
                    let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else {
                        continue;
                    };
                    if git {
                        tab.full |= relevant_git_path(rel);
                    } else {
                        tab.status |= !(tab.ignored)(rel);
                    }
                    tab.due = Some(now + DEBOUNCE);
                }
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
        for id in ended {
            self.remove(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Backend, DEBOUNCE, Hub, Tab, outermost};
    use crate::worker::Job;
    use crossbeam_channel::{Receiver, unbounded};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    /// Records watch calls; `fail` names folders that cannot be watched.
    #[derive(Default)]
    struct Recorder {
        log: Arc<Mutex<Vec<String>>>,
        fail: Vec<PathBuf>,
    }

    impl Backend for Recorder {
        fn watch(&mut self, path: &Path) -> Result<(), String> {
            if self.fail.iter().any(|f| f == path) {
                return Err(format!("cannot watch {}", path.display()));
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

    /// A tab on `root` with its git folder at `root/.git`, nothing ignored but `*.log`.
    fn tab(root: &str) -> (Tab, Receiver<Job>, crossbeam_channel::Sender<()>) {
        let (jobs, rx) = unbounded();
        let (stop_tx, stop) = crossbeam_channel::bounded(0);
        let (ended, _) = crossbeam_channel::bounded(0);
        let root = PathBuf::from(root);
        let tab = Tab::new(
            root.clone(),
            vec![root.join(".git")],
            Box::new(|rel: &Path| rel.extension().is_some_and(|e| e == "log")),
            jobs,
            stop,
            ended,
        );
        (tab, rx, stop_tx)
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
        assert_eq!(*log.lock().unwrap(), ["+/p/sub", "-/p/sub", "+/p"]);
        hub.remove(inner_id);
        // Closing the inner tab leaves the outer watch alone.
        assert_eq!(log.lock().unwrap().len(), 3, "{:?}", log.lock().unwrap());
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
        assert_eq!(*log.lock().unwrap(), ["+/p"], "the inner one is covered");
        hub.remove(outer_id);
        assert_eq!(*log.lock().unwrap(), ["+/p", "-/p", "+/p/sub"]);
        // Its folders went unwatched for a moment: it reads everything again.
        hub.flush(Instant::now());
        assert_eq!(jobs(&inner_rx), ["Refresh(0)"]);
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
        assert!(
            jobs(&outer_rx).is_empty(),
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
        let (t2, _rx2, _s2) = tab("/q");
        hub.add(t2);
        let reported: Vec<String> = jobs(&rx);
        assert_eq!(reported, ["WatchFailed(\"cannot watch /p\")"]);
        // Tried again with the next change of tabs, and not reported twice.
        let (t3, _rx3, _s3) = tab("/r");
        hub.add(t3);
        assert_eq!(jobs(&rx), Vec::<String>::new());
        assert_eq!(*log.lock().unwrap(), ["+/q", "+/r"]);
    }

    #[test]
    fn a_watcher_that_cannot_start_is_reported_and_created_later() {
        let mut attempts = 0;
        let mut hub = Hub::new(move || {
            attempts += 1;
            if attempts == 1 {
                Err("too many inotify instances".to_owned())
            } else {
                Ok(Recorder::default())
            }
        });
        let (t, rx, _s) = tab("/p");
        hub.add(t);
        assert_eq!(jobs(&rx), ["WatchFailed(\"too many inotify instances\")"]);
        let (t2, _rx2, _s2) = tab("/q");
        hub.add(t2);
        assert!(hub.backend.is_some());
        assert!(hub.watched.contains(Path::new("/p")));
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
        assert_eq!(*log.lock().unwrap(), ["+/p", "-/p"]);
    }

    #[test]
    fn a_dropped_change_queue_refreshes_every_tab() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut hub = hub(&log, &[]);
        let (t, rx, _s) = tab("/p");
        hub.add(t);
        let now = Instant::now();
        hub.route(
            Ok(notify::Event::new(notify::EventKind::Other).set_flag(notify::event::Flag::Rescan)),
            now,
        );
        hub.flush(now + DEBOUNCE);
        assert_eq!(jobs(&rx), ["Refresh(0)"]);
    }

    /// The real watcher: closing the outer of two nested repositories must not take the inner
    /// one's watches with it (`notify` shares one inotify watch per folder), and every
    /// repository shares one inotify instance.
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "tests drive git directly to build repositories"
    )]
    fn closing_an_outer_repository_keeps_the_inner_one_watched()
    -> Result<(), Box<dyn std::error::Error>> {
        use std::time::Duration;
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
        // Registering happens on the watcher's thread: let both land before closing the outer.
        std::thread::sleep(Duration::from_millis(500));
        drop(outer_stop);
        // The outer watch is gone once its `ended` disconnects.
        assert!(matches!(
            outer_ended.recv_timeout(Duration::from_secs(10)),
            Err(crossbeam_channel::RecvTimeoutError::Disconnected)
        ));
        // The inner one refreshes for the moment it went unwatched; wait for that to pass.
        std::thread::sleep(DEBOUNCE * 2);
        let _ = inner_rx.try_iter().count();
        std::fs::write(inner.join("new.txt"), "x")?;
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut seen = false;
        while let Ok(job) = inner_rx.recv_deadline(deadline) {
            if matches!(job, Job::RefreshStatus) {
                seen = true;
                break;
            }
        }
        assert!(seen, "a change in the inner repository still reaches it");
        #[cfg(target_os = "linux")]
        {
            let inotify = std::fs::read_dir("/proc/self/fd")?
                .filter_map(|e| std::fs::read_link(e.ok()?.path()).ok())
                .filter(|l| l.to_string_lossy().contains("inotify"))
                .count();
            assert_eq!(inotify, 1, "one inotify instance serves every repository");
        }
        Ok(())
    }
}
