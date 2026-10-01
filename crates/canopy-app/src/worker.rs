//! One background thread per open repository. The UI sends [`Job`]s and drains [`Event`]s; it
//! never calls git itself, so a slow disk, a large repository or the network cannot freeze it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, unbounded};

use canopy_core::ops::{HunkAction, PullStrategy, ResetMode};
use canopy_core::{
    CommitDetails, DiffSource, FileDiff, FileEntry, Oid, Repo, RepoSnapshot, RepoState, Whitespace,
    WorkingStatus,
};

/// A request to change the repository. Each one is followed by a refresh.
#[derive(Debug, Clone)]
pub enum Op {
    Stage(Vec<String>),
    StageAll,
    Unstage(Vec<String>),
    UnstageAll,
    Discard(Vec<FileEntry>),
    Hunk {
        action: HunkAction,
        path: String,
        index: usize,
        key: (u32, u32, u32, u32),
        /// Context lines of the diff the hunk came from.
        context: u32,
    },
    /// Stage, unstage or discard only some lines of a hunk.
    Lines {
        action: HunkAction,
        path: String,
        index: usize,
        key: (u32, u32, u32, u32),
        /// Indices into the hunk's lines.
        lines: Vec<usize>,
        context: u32,
    },
    Commit {
        message: String,
        amend: bool,
        signoff: bool,
    },
    CreateBranch {
        name: String,
        start: Option<String>,
        checkout: bool,
    },
    Checkout(String),
    CheckoutRemote {
        remote: String,
        branch: String,
    },
    CheckoutDetached(Oid),
    RenameBranch {
        old: String,
        new: String,
    },
    DeleteBranch {
        name: String,
        force: bool,
    },
    Merge(String, canopy_core::MergeMode),
    Rebase(String),
    ContinueOrAbort {
        state: RepoState,
        abort: bool,
    },
    CherryPick(Oid),
    Revert(Oid),
    Reset {
        oid: Oid,
        mode: ResetMode,
    },
    CreateTag {
        name: String,
        target: Oid,
        message: Option<String>,
    },
    DeleteTag(String),
    PushTag {
        remote: String,
        tag: String,
    },
    StashSave {
        message: Option<String>,
        untracked: bool,
    },
    StashApply {
        index: usize,
        pop: bool,
    },
    StashDrop(usize),
    /// Fetch every remote. `quiet` skips the success toast (auto-fetch).
    Fetch {
        prune: bool,
        quiet: bool,
    },
    Pull {
        strategy: PullStrategy,
    },
    Push {
        force: bool,
    },
    AddRemote {
        name: String,
        url: String,
    },
    RemoveRemote(String),
    DeleteRemoteBranch {
        remote: String,
        branch: String,
    },
    /// Write a conflicted file's resolution; `mark` also stages it (marks it resolved).
    /// `expected` is the fingerprint of the parse `content` came from; the write is refused if
    /// the file changed on disk since.
    Resolve {
        path: String,
        content: String,
        mark: bool,
        expected: u64,
    },
    /// Resolve a conflicted file by taking one side whole (deleting the file if that side did).
    ResolveSide {
        path: String,
        side: canopy_core::Side,
    },
    /// Fetch a pull/merge request's head into a local branch and check it out.
    CheckoutRequest {
        remote: String,
        remote_ref: String,
        branch: String,
    },
}

impl Op {
    /// What finishing this operation means for the tab, whatever its label says.
    pub fn kind(&self) -> OpKind {
        match self {
            Op::Commit { .. } => OpKind::Commit,
            Op::Merge(target, canopy_core::MergeMode::Squash) => OpKind::Squash(target.clone()),
            Op::Fetch { .. } => OpKind::Fetch,
            _ => OpKind::Other,
        }
    }

    /// Present-tense label shown while the operation runs.
    pub fn label(&self) -> String {
        match self {
            Op::Stage(_) | Op::StageAll => "Staging".into(),
            Op::Unstage(_) | Op::UnstageAll => "Unstaging".into(),
            Op::Discard(_) => "Discarding changes".into(),
            Op::Hunk {
                action: HunkAction::Stage,
                ..
            } => "Staging hunk".into(),
            Op::Hunk {
                action: HunkAction::Unstage,
                ..
            } => "Unstaging hunk".into(),
            Op::Hunk {
                action: HunkAction::Discard,
                ..
            } => "Discarding hunk".into(),
            Op::Lines {
                action: HunkAction::Stage,
                ..
            } => "Staging lines".into(),
            Op::Lines {
                action: HunkAction::Unstage,
                ..
            } => "Unstaging lines".into(),
            Op::Lines {
                action: HunkAction::Discard,
                ..
            } => "Discarding lines".into(),
            Op::Commit { amend: true, .. } => "Amending commit".into(),
            Op::Commit { .. } => "Committing".into(),
            Op::CreateBranch { name, .. } => format!("Creating branch {name}"),
            Op::Checkout(b) => format!("Checking out {b}"),
            Op::CheckoutRemote { remote, branch } => format!("Checking out {remote}/{branch}"),
            Op::CheckoutDetached(o) => format!("Checking out {}", canopy_core::short_id(*o)),
            Op::RenameBranch { new, .. } => format!("Renaming to {new}"),
            Op::DeleteBranch { name, .. } => format!("Deleting {name}"),
            Op::Merge(t, canopy_core::MergeMode::Squash) => format!("Squashing {t}"),
            Op::Merge(t, _) => format!("Merging {t}"),
            Op::Rebase(t) => format!("Rebasing onto {t}"),
            Op::ContinueOrAbort { state, abort } => format!(
                "{} {}",
                if *abort { "Aborting" } else { "Continuing" },
                state.label().to_lowercase()
            ),
            Op::CherryPick(o) => format!("Cherry-picking {}", canopy_core::short_id(*o)),
            Op::Revert(o) => format!("Reverting {}", canopy_core::short_id(*o)),
            Op::Reset { mode, .. } => format!("{} reset", mode.label()),
            Op::CreateTag { name, .. } => format!("Tagging {name}"),
            Op::DeleteTag(t) => format!("Deleting tag {t}"),
            Op::PushTag { tag, .. } => format!("Pushing tag {tag}"),
            Op::StashSave { .. } => "Stashing".into(),
            Op::StashApply { pop: true, .. } => "Popping stash".into(),
            Op::StashApply { .. } => "Applying stash".into(),
            Op::StashDrop(_) => "Dropping stash".into(),
            Op::Fetch { .. } => "Fetching".into(),
            Op::Pull { .. } => "Pulling".into(),
            Op::Push { force: true } => "Force pushing".into(),
            Op::Push { .. } => "Pushing".into(),
            Op::AddRemote { name, .. } => format!("Adding remote {name}"),
            Op::RemoveRemote(r) => format!("Removing remote {r}"),
            Op::DeleteRemoteBranch { remote, branch } => format!("Deleting {remote}/{branch}"),
            Op::CheckoutRequest { branch, .. } => format!("Checking out {branch}"),
            Op::Resolve {
                path, mark: true, ..
            } => format!("Resolving {path}"),
            Op::Resolve { path, .. } => format!("Saving {path}"),
            Op::ResolveSide { path, side } => format!(
                "Taking {} for {path}",
                match side {
                    canopy_core::Side::Ours => "ours",
                    canopy_core::Side::Theirs => "theirs",
                }
            ),
        }
    }

    /// Whether a toast should confirm success (quiet for frequent staging actions).
    pub fn announce(&self) -> bool {
        !matches!(
            self,
            Op::Stage(_)
                | Op::StageAll
                | Op::Unstage(_)
                | Op::UnstageAll
                | Op::Hunk { .. }
                | Op::Lines { .. }
                | Op::Resolve { .. }
                | Op::ResolveSide { .. }
                | Op::Fetch { quiet: true, .. }
        )
    }
}

/// Work for the repository thread.
#[derive(Debug, Clone)]
pub enum Job {
    /// Full snapshot with up to this many commits (0 keeps the current limit).
    Refresh(usize),
    /// Status only (working tree changed).
    RefreshStatus,
    /// Details of one commit.
    Details(Oid),
    /// Diff of one file.
    Diff(DiffKey),
    /// `HEAD`'s message (to prefill an amend).
    HeadMessage,
    /// A commit message for the squash merge of this branch, with git's summary of the squashed
    /// commits (`.git/SQUASH_MSG`) as the body.
    SquashMessage(String),
    /// A conflicted file, parsed for the resolver.
    Conflict(String),
    /// A mutation.
    Op(Op),
}

/// Identifies a file diff request; the response carries it back so stale replies can be dropped.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DiffKey {
    pub source: DiffSource,
    pub path: String,
    pub old_path: Option<String>,
    pub conflicted: bool,
    pub context: u32,
    pub whitespace: Whitespace,
    /// Also send syntax highlighting.
    pub highlight: bool,
}

/// What the repository thread reports back. Large results are shared (`Arc`), so the UI can
/// hold them across a frame without copying.
#[derive(Debug)]
pub enum Event {
    Snapshot(Arc<RepoSnapshot>),
    Status(WorkingStatus),
    Details(Arc<CommitDetails>),
    Diff(DiffKey, Result<Arc<FileDiff>, String>),
    /// Syntax highlighting for the diff (sent before with the same key). It comes from the
    /// highlighter threads, so it may arrive after a newer diff; it applies only to this one.
    Highlight(DiffKey, Arc<FileDiff>, Arc<crate::highlight::Highlights>),
    HeadMessage(String),
    Conflict {
        path: String,
        kind: Option<canopy_core::ConflictKind>,
        file: Result<Arc<canopy_core::ConflictFile>, String>,
    },
    OpStarted(String),
    Progress(String),
    OpFinished {
        label: String,
        kind: OpKind,
        announce: bool,
        result: Result<String, String>,
    },
    Failed(String),
}

/// What a finished operation means for the tab, independent of its label's wording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpKind {
    /// A commit or an amend: clear the composer.
    Commit,
    /// A squash merge staged onto this branch: prefill the message and review it.
    Squash(String),
    /// A fetch: restart the auto-fetch clock.
    Fetch,
    Other,
}

/// The UI's handle on a repository thread.
#[derive(Debug)]
pub struct Worker {
    jobs: Sender<Job>,
    /// Events from the thread.
    pub events: Receiver<Event>,
    /// Dropping this stops the file watcher's thread.
    _stop_watcher: Sender<()>,
}

impl Worker {
    /// Starts the thread (and a file watcher) for the repository at `path`.
    pub fn spawn(path: PathBuf, ctx: egui::Context) -> Self {
        let (jobs, job_rx) = unbounded::<Job>();
        let (ev_tx, events) = unbounded::<Event>();
        let thread_ctx = ctx.clone();
        let thread_path = path.clone();
        let failed = ev_tx.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("repo:{}", path.display()))
            .spawn(move || run(thread_path, job_rx, ev_tx, thread_ctx));
        if let Err(e) = spawned {
            // Without a thread the tab would wait for its first snapshot forever: show why.
            tracing::error!("could not start the repository thread: {e}");
            let _ = failed.send(Event::Failed(format!(
                "Could not start the repository thread: {e}"
            )));
            ctx.request_repaint();
        }
        let (stop_watcher, stop) = crossbeam_channel::bounded(0);
        watch(path, jobs.clone(), stop);
        Self {
            jobs,
            events,
            _stop_watcher: stop_watcher,
        }
    }

    /// A worker with no threads: tests feed its events and read its jobs.
    #[cfg(test)]
    pub fn detached() -> (Self, Sender<Event>, Receiver<Job>) {
        let (jobs, job_rx) = unbounded();
        let (ev_tx, events) = unbounded();
        let (stop_watcher, _) = crossbeam_channel::bounded(0);
        let worker = Self {
            jobs,
            events,
            _stop_watcher: stop_watcher,
        };
        (worker, ev_tx, job_rx)
    }

    /// Queues a job.
    pub fn send(&self, job: Job) {
        let _ = self.jobs.send(job);
    }
}

impl Job {
    /// A short name for logs.
    fn kind(&self) -> &'static str {
        match self {
            Job::Refresh(_) => "refresh",
            Job::RefreshStatus => "status",
            Job::Details(_) => "details",
            Job::Diff(_) => "diff",
            Job::HeadMessage => "head-message",
            Job::SquashMessage(_) => "squash-message",
            Job::Conflict(_) => "conflict",
            Job::Op(_) => "operation",
        }
    }
}

fn run(path: PathBuf, jobs: Receiver<Job>, events: Sender<Event>, ctx: egui::Context) {
    // Every event this thread logs carries the repository.
    let _repo = tracing::info_span!("repo", path = %path.display()).entered();
    let emit = |e: Event| {
        let _ = events.send(e);
        ctx.request_repaint();
    };
    let mut limit = 2000;
    // Bumped for every diff request; a highlight request made for an older one is stale.
    let diff_generation = Arc::new(AtomicU64::new(0));
    // The last diff sent. A refresh that finds the same content sends the same `Arc` again, so
    // the view keeps its layout and highlighting instead of rebuilding them on the UI thread.
    let mut last_diff: Option<(DiffKey, Arc<canopy_core::FileDiff>)> = None;
    while let Ok(first) = jobs.recv() {
        // Coalesce queued refreshes: only the latest matters.
        let mut batch = vec![first];
        batch.extend(jobs.try_iter());
        let mut want_full = None;
        let mut want_status = false;
        let mut rest = Vec::new();
        for job in batch {
            match job {
                Job::Refresh(n) => want_full = Some(n),
                Job::RefreshStatus => want_status = true,
                other => rest.push(other),
            }
        }

        let repo = match Repo::open(&path) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "cannot open the repository");
                emit(Event::Failed(e.to_string()));
                continue;
            }
        };

        let mut mutated = false;
        for job in rest {
            let kind = job.kind();
            let started = std::time::Instant::now();
            match job {
                Job::Details(oid) => match repo.commit_details(oid) {
                    Ok(d) => emit(Event::Details(Arc::new(d))),
                    Err(e) => emit(Event::Failed(e.to_string())),
                },
                Job::Diff(key) => {
                    let result = if key.conflicted {
                        repo.conflict_view(&key.path)
                    } else {
                        repo.file_diff(
                            &key.source,
                            &key.path,
                            key.old_path.as_deref(),
                            key.context,
                            key.whitespace,
                        )
                    };
                    let unchanged = match (&result, &last_diff) {
                        (Ok(d), Some((k, last))) if *k == key && **last == *d => {
                            Some(Arc::clone(last))
                        }
                        _ => None,
                    };
                    if let Some(same) = unchanged {
                        emit(Event::Diff(key, Ok(same)));
                        continue;
                    }
                    let result = result.map(Arc::new).map_err(|e| e.to_string());
                    last_diff = result.as_ref().ok().map(|d| (key.clone(), Arc::clone(d)));
                    let generation = diff_generation.fetch_add(1, Ordering::Relaxed) + 1;
                    let highlighted = match &result {
                        Ok(d) if key.highlight && !key.conflicted => Some(Arc::clone(d)),
                        _ => None,
                    };
                    emit(Event::Diff(key.clone(), result));
                    // After the diff, so it shows at once as plain text; the highlighter threads
                    // colour it later (cached per file revision).
                    if let Some(diff) = highlighted {
                        let latest = Arc::clone(&diff_generation);
                        let events = events.clone();
                        let ctx = ctx.clone();
                        let shown = Arc::clone(&diff);
                        crate::highlight::request(crate::highlight::Request {
                            diff,
                            stale: Box::new(move || latest.load(Ordering::Relaxed) != generation),
                            deliver: Box::new(move |h| {
                                let _ = events.send(Event::Highlight(key, shown, h));
                                ctx.request_repaint();
                            }),
                        });
                    }
                }
                Job::HeadMessage => {
                    if let Ok(m) = repo.head_message() {
                        emit(Event::HeadMessage(m));
                    }
                }
                Job::SquashMessage(target) => {
                    let path = repo.git().path().join("SQUASH_MSG");
                    let body = std::fs::read_to_string(path).unwrap_or_default();
                    // Drop git's comment lines; keep the list of squashed commits.
                    let body: Vec<&str> = body.lines().filter(|l| !l.starts_with('#')).collect();
                    let body = body.join("\n");
                    emit(Event::HeadMessage(format!(
                        "Merge {target} (squashed)\n\n{}",
                        body.trim()
                    )));
                }
                Job::Conflict(path) => {
                    let kind = repo.conflict_kind(&path).ok().flatten();
                    let file = repo
                        .conflict_file(&path)
                        .map(Arc::new)
                        .map_err(|e| e.to_string());
                    emit(Event::Conflict { path, kind, file });
                }
                Job::Op(op) => {
                    let label = op.label();
                    let kind = op.kind();
                    let announce = op.announce();
                    tracing::info!(operation = %label, "operation started");
                    emit(Event::OpStarted(label.clone()));
                    let progress = |line: &str| emit(Event::Progress(line.to_owned()));
                    let result = perform(&repo, op, progress).map_err(|e| e.to_string());
                    let ms = started.elapsed().as_millis() as u64;
                    match &result {
                        Ok(_) => tracing::info!(operation = %label, ms, "operation succeeded"),
                        Err(e) => {
                            tracing::warn!(operation = %label, ms, error = %canopy_core::cli::redact_url(e), "operation failed")
                        }
                    }
                    emit(Event::OpFinished {
                        label,
                        kind,
                        announce,
                        result,
                    });
                    mutated = true;
                }
                Job::Refresh(_) | Job::RefreshStatus => {}
            }
            tracing::debug!(
                job = kind,
                ms = started.elapsed().as_millis() as u64,
                "job done"
            );
        }

        if let Some(n) = want_full.filter(|n| *n > 0) {
            limit = n;
        }
        if want_full.is_some() || mutated {
            let started = std::time::Instant::now();
            match repo.snapshot(limit) {
                Ok(s) => {
                    tracing::debug!(
                        commits = s.commits.len(),
                        ms = started.elapsed().as_millis() as u64,
                        "snapshot"
                    );
                    emit(Event::Snapshot(Arc::new(s)));
                }
                Err(e) => {
                    tracing::warn!(error = %e, "snapshot failed");
                    emit(Event::Failed(e.to_string()));
                }
            }
        } else if want_status {
            match canopy_core::status::collect(repo.git()) {
                Ok(s) => emit(Event::Status(s)),
                Err(e) => emit(Event::Failed(e.to_string())),
            }
        }
    }
}

fn perform(repo: &Repo, op: Op, progress: impl FnMut(&str)) -> canopy_core::Result<String> {
    let done = |r: canopy_core::Result<()>| r.map(|()| String::new());
    match op {
        Op::Stage(p) => done(repo.stage(&p)),
        Op::StageAll => done(repo.stage_all()),
        Op::Unstage(p) => done(repo.unstage(&p)),
        Op::UnstageAll => done(repo.unstage_all()),
        Op::Discard(f) => done(repo.discard(&f)),
        Op::Hunk {
            action,
            path,
            index,
            key,
            context,
        } => done(repo.apply_hunk(action, &path, index, key, context)),
        Op::Lines {
            action,
            path,
            index,
            key,
            lines,
            context,
        } => done(repo.apply_lines(action, &path, index, key, &lines, context)),
        Op::Commit {
            message,
            amend,
            signoff,
        } => repo
            .commit(&message, amend, signoff)
            .map(canopy_core::short_id),
        Op::CreateBranch {
            name,
            start,
            checkout,
        } => done(repo.create_branch(&name, start.as_deref(), checkout)),
        Op::Checkout(b) => done(repo.checkout_branch(&b)),
        Op::CheckoutRemote { remote, branch } => {
            done(repo.checkout_remote_branch(&remote, &branch))
        }
        Op::CheckoutDetached(oid) => done(repo.checkout_detached(oid)),
        Op::RenameBranch { old, new } => done(repo.rename_branch(&old, &new)),
        Op::DeleteBranch { name, force } => done(repo.delete_branch(&name, force)),
        Op::Merge(t, mode) => repo.merge(&t, mode),
        Op::Rebase(t) => repo.rebase(&t),
        Op::ContinueOrAbort { state, abort } => repo.continue_or_abort(state, abort),
        Op::CherryPick(o) => repo.cherry_pick(o),
        Op::Revert(o) => repo.revert(o),
        Op::Reset { oid, mode } => done(repo.reset(oid, mode)),
        Op::CreateTag {
            name,
            target,
            message,
        } => done(repo.create_tag(&name, target, message.as_deref())),
        Op::DeleteTag(t) => done(repo.delete_tag(&t)),
        Op::PushTag { remote, tag } => repo.push_tag(&remote, &tag, progress),
        Op::StashSave { message, untracked } => {
            done(repo.stash_save(message.as_deref(), untracked))
        }
        Op::StashApply { index, pop } => done(repo.stash_apply(index, pop)),
        Op::StashDrop(i) => done(repo.stash_drop(i)),
        Op::Fetch { prune, .. } => repo.fetch(None, prune, progress),
        Op::Pull { strategy } => repo.pull(strategy, progress),
        Op::Push { force } => repo.push(force, progress),
        Op::AddRemote { name, url } => done(repo.add_remote(&name, &url)),
        Op::RemoveRemote(r) => done(repo.remove_remote(&r)),
        Op::DeleteRemoteBranch { remote, branch } => {
            repo.delete_remote_branch(&remote, &branch, progress)
        }
        Op::CheckoutRequest {
            remote,
            remote_ref,
            branch,
        } => repo.checkout_request(&remote, &remote_ref, &branch, progress),
        Op::Resolve {
            path,
            content,
            mark,
            expected,
        } => done(repo.write_resolution(&path, &content, mark, expected)),
        Op::ResolveSide { path, side } => done(repo.resolve_with_side(&path, side)),
    }
}

/// Whether a changed path inside `.git` affects what Canopy shows.
fn relevant_git_path(rel: &Path) -> bool {
    let s = rel.to_string_lossy();
    !(s.starts_with("objects")
        || s.starts_with("logs")
        || s.ends_with(".lock")
        || s.contains("fsmonitor"))
}

/// Watches the working tree and the repository's own folders on a thread of its own, so a large
/// tree never stalls the UI while the watches are registered. Changes in a git folder trigger a
/// full refresh, other changes a status refresh, both debounced; ignored files are skipped.
/// Symbolic links are not followed. A linked worktree's or submodule's git folder (and the
/// common folder it shares) lives outside the working tree, so it gets watches of its own. The
/// thread ends when `stop`'s sender (held by [`Worker`]) is dropped.
fn watch(path: PathBuf, jobs: Sender<Job>, stop: Receiver<()>) {
    let spawned = std::thread::Builder::new()
        .name("watcher".into())
        .spawn(move || {
            use notify::{Config, RecursiveMode, Watcher};
            let repo = match git2::Repository::open(&path) {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(error = %e.message(), "no file watcher: cannot open the repository");
                    return;
                }
            };
            let Some(root) = repo.workdir().map(Path::to_path_buf) else {
                return;
            };
            let mut git_dirs = vec![repo.path().to_path_buf()];
            if !git_dirs.iter().any(|d| d == repo.commondir()) {
                git_dirs.push(repo.commondir().to_path_buf());
            }
            let (tx, rx) = unbounded::<notify::Result<notify::Event>>();
            let config = Config::default().with_follow_symlinks(false);
            let mut watcher = match notify::RecommendedWatcher::new(
                move |e| {
                    let _ = tx.send(e);
                },
                config,
            ) {
                Ok(w) => w,
                Err(e) => {
                    tracing::warn!(error = %e, "no file watcher");
                    return;
                }
            };
            if let Err(e) = watcher.watch(&root, RecursiveMode::Recursive) {
                tracing::warn!(error = %e, "no file watcher for the working tree");
                return;
            }
            for dir in git_dirs.iter().filter(|d| !d.starts_with(&root)) {
                if let Err(e) = watcher.watch(dir, RecursiveMode::Recursive) {
                    tracing::warn!(error = %e, "no file watcher for the git folder");
                }
            }
            loop {
                let first = crossbeam_channel::select! {
                    recv(rx) -> e => match e {
                        Ok(e) => e,
                        Err(_) => break,
                    },
                    recv(stop) -> _ => break,
                };
                let mut full = false;
                let mut status = false;
                let mut classify = |e: notify::Result<notify::Event>| {
                    let Ok(e) = e else { return };
                    if matches!(e.kind, notify::EventKind::Access(_)) {
                        return;
                    }
                    for p in e.paths {
                        if let Some(rel) = git_dirs.iter().find_map(|d| p.strip_prefix(d).ok()) {
                            full |= relevant_git_path(rel);
                        } else if let Ok(rel) = p.strip_prefix(&root) {
                            status |= !repo.is_path_ignored(rel).unwrap_or(false);
                        }
                    }
                };
                classify(first);
                // Debounce: wait for a quiet period.
                while let Ok(e) = rx.recv_timeout(Duration::from_millis(350)) {
                    classify(e);
                }
                if full {
                    if jobs.send(Job::Refresh(0)).is_err() {
                        break;
                    }
                } else if status && jobs.send(Job::RefreshStatus).is_err() {
                    break;
                }
            }
            drop(watcher);
        });
    if let Err(e) = spawned {
        tracing::warn!("no file watcher: cannot start its thread: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::{Job, Op, OpKind, watch};
    use std::path::Path;
    use std::process::Command;
    use std::time::{Duration, Instant};

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .current_dir(dir)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "git {args:?}");
    }

    /// Waits up to `limit` for a job matching `want`.
    fn wait_for(
        jobs: &crossbeam_channel::Receiver<Job>,
        want: fn(&Job) -> bool,
        limit: Duration,
    ) -> bool {
        let end = Instant::now() + limit;
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            match jobs.recv_timeout(left) {
                Ok(j) if want(&j) => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
        false
    }

    #[test]
    fn a_linked_worktrees_git_folder_is_watched() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let main = dir.path().join("main");
        std::fs::create_dir(&main)?;
        git(&main, &["init", "-q", "-b", "main"]);
        git(&main, &["config", "user.name", "Ada"]);
        git(&main, &["config", "user.email", "ada@example.com"]);
        git(&main, &["config", "core.autocrlf", "false"]);
        git(&main, &["commit", "-q", "--allow-empty", "-m", "first"]);
        let linked = dir.path().join("linked");
        let linked_arg = linked.to_string_lossy().into_owned();
        git(
            &main,
            &["worktree", "add", "-q", "-b", "topic", &linked_arg],
        );

        let (tx, rx) = crossbeam_channel::unbounded();
        let (_stop, stop) = crossbeam_channel::bounded::<()>(0);
        watch(linked.clone(), tx, stop);
        // Registering the watches happens on the watcher's thread; give it a moment.
        std::thread::sleep(Duration::from_millis(500));
        // A commit in the linked worktree changes its HEAD's branch, whose ref lives in the
        // common folder, and the worktree's own index under .git/worktrees/linked.
        git(&linked, &["commit", "-q", "--allow-empty", "-m", "second"]);
        assert!(wait_for(
            &rx,
            |j| matches!(j, Job::Refresh(_)),
            Duration::from_secs(10)
        ));
        Ok(())
    }

    #[test]
    fn finished_operations_are_typed_not_matched_by_label() {
        let commit = |amend| Op::Commit {
            message: "feat: x".into(),
            amend,
            signoff: false,
        };
        assert_eq!(commit(false).kind(), OpKind::Commit);
        assert_eq!(commit(true).kind(), OpKind::Commit);
        assert_eq!(
            Op::Merge("topic".into(), canopy_core::MergeMode::Squash).kind(),
            OpKind::Squash("topic".into())
        );
        assert_eq!(
            Op::Merge("topic".into(), canopy_core::MergeMode::Default).kind(),
            OpKind::Other
        );
        let fetch = Op::Fetch {
            prune: true,
            quiet: false,
        };
        assert_eq!(fetch.kind(), OpKind::Fetch);
    }
}
