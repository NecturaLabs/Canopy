//! One background thread per open repository. The UI sends [`Job`]s and drains [`Event`]s; it
//! never calls git itself, so a slow disk, a large repository or the network cannot freeze it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, unbounded};

use canopy_core::ops::{HunkAction, PullStrategy, ResetMode};
use canopy_core::{
    CommitDetails, DiffSource, FileDiff, FileEntry, MissingObjects, Oid, Repo, RepoSnapshot,
    RepoState, Whitespace, WorkingStatus,
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
            Op::DeleteBranch { name, force: false } => OpKind::SafeDelete(name.clone()),
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
    /// The details of this commit could not be read. `can_download` says that it is a partial
    /// clone that has not downloaded them, which trying again can fix.
    DetailsFailed {
        oid: Oid,
        message: String,
        can_download: bool,
    },
    Diff(DiffKey, Result<Arc<FileDiff>, String>),
    /// A read needed a download from this promisor remote, which Canopy did not run because the
    /// repository is outside every trusted folder (`trust.rs`).
    DownloadsRefused {
        remote: String,
    },
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
    /// `git branch -d`: when Git refuses an unmerged branch, offer to force the delete.
    SafeDelete(String),
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

        // Whether a partial clone may download on its own here; asked only when one needs to.
        let allowed = || repo.is_in_trusted_folder(&crate::trust::folders());
        let rest = coalesce_reads(rest);
        let mut mutated = false;
        for job in rest {
            let kind = job.kind();
            let started = std::time::Instant::now();
            match job {
                Job::Details(oid) => {
                    // The file list needs no file contents, so it is shown first, offline. A
                    // treeless clone downloads trees for it (network, with a progress line).
                    match read_downloading(&emit, &allowed, |m| repo.commit_details(oid, m)) {
                        Ok(d) => {
                            let wants_counts = !d.stats_known
                                && d.files.len() <= canopy_core::repo::MAX_STAT_FILES;
                            emit(Event::Details(Arc::new(d)));
                            // Line counts need blobs: fetch them after the list is on screen,
                            // unless the user already moved on (a newer job is waiting).
                            if wants_counts && jobs.is_empty() {
                                if !allowed() {
                                    download_refused(&emit, repo.promisor_remote());
                                } else {
                                    match with_progress(&emit, repo.promisor_remote(), || {
                                        repo.commit_details(oid, MissingObjects::Download)
                                    }) {
                                        Ok(d) if d.stats_known => emit(Event::Details(Arc::new(d))),
                                        Ok(_) => {}
                                        // The list stays; only the counts are missing.
                                        Err(e) => {
                                            tracing::warn!(error = %redacted(&e), "line counts not downloaded")
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!(error = %redacted(&e), "commit details failed");
                            emit(Event::DetailsFailed {
                                oid,
                                message: e.to_string(),
                                can_download: e.can_download(),
                            });
                        }
                    }
                }
                Job::Diff(key) => {
                    let result = if key.conflicted {
                        repo.conflict_view(&key.path)
                    } else {
                        // Partial clones: download missing objects like `git show` does.
                        read_downloading(&emit, &allowed, |missing| {
                            repo.file_diff(
                                &key.source,
                                &key.path,
                                key.old_path.as_deref(),
                                key.context,
                                key.whitespace,
                                missing,
                            )
                        })
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
            match snapshot_completing(&repo, limit, &emit, &allowed) {
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
            match read_downloading(&emit, &allowed, |m| repo.status(m)) {
                Ok(s) => emit(Event::Status(s)),
                Err(e) => emit(Event::Failed(e.to_string())),
            }
        }
    }
}

/// Drops the reads a later job of the same batch makes pointless, so a burst of selections does
/// not queue one download per click on the repository's single thread: only the last
/// [`Job::Details`] runs, and only the last [`Job::Diff`] of each kind of source (a commit diff
/// supersedes earlier commit diffs of any commit; the tab ignores the replies of all but the
/// newest request anyway). Every other job keeps its place and order.
fn coalesce_reads(jobs: Vec<Job>) -> Vec<Job> {
    let mut keep = vec![true; jobs.len()];
    let mut details_seen = false;
    let mut diff_sources = Vec::new();
    for (i, job) in jobs.iter().enumerate().rev() {
        match job {
            Job::Details(_) => keep[i] = !std::mem::replace(&mut details_seen, true),
            Job::Diff(key) => {
                let source = std::mem::discriminant(&key.source);
                if diff_sources.contains(&source) {
                    keep[i] = false;
                } else {
                    diff_sources.push(source);
                }
            }
            _ => {}
        }
    }
    let dropped = keep.iter().filter(|k| !**k).count();
    if dropped > 0 {
        tracing::debug!(dropped, "superseded reads skipped");
    }
    jobs.into_iter()
        .zip(keep)
        .filter_map(|(job, keep)| keep.then_some(job))
        .collect()
}

/// `e` as it may appear in a log: git's own words, with credentials in URLs removed.
fn redacted(e: &canopy_core::Error) -> String {
    canopy_core::cli::redact_url(&e.to_string())
}

/// Runs `f` with the status bar saying where it is downloading from, then clears the line.
fn with_progress<T>(emit: &impl Fn(Event), remote: Option<String>, f: impl FnOnce() -> T) -> T {
    let from = remote.map_or_else(String::new, |r| format!(" from {r}"));
    emit(Event::Progress(format!("Downloading{from}…")));
    let result = f();
    emit(Event::Progress(String::new()));
    result
}

/// Why a download did not run in a repository outside every trusted folder.
const NOT_TRUSTED: &str = "Canopy downloads it only for repositories in a trusted folder. \
                           Choose Trust… in the status bar to allow it for this one.";

/// Reports a download that was not run because the repository is not in a trusted folder, and
/// returns the error the read ends with. `can_download()` is false for it: trying again cannot
/// help until the folder is trusted.
fn download_refused(emit: &impl Fn(Event), remote: Option<String>) -> canopy_core::Error {
    if let Some(remote) = remote {
        tracing::info!(remote = %canopy_core::cli::redact_url(&remote), "download refused: folder not trusted");
        emit(Event::DownloadsRefused { remote });
    }
    canopy_core::Error::NotDownloaded {
        remote: None,
        reason: Some(NOT_TRUSTED.into()),
    }
}

/// A read in a repository that may be a partial clone: offline first, and only when it fails
/// because objects were not downloaded and a remote can supply them, again with downloading
/// allowed (network), with a progress line while it runs. Full clones run `read` once. A
/// repository outside every trusted folder (`allowed` says) never downloads: its configuration
/// would run.
fn read_downloading<T>(
    emit: &impl Fn(Event),
    allowed: &impl Fn() -> bool,
    read: impl Fn(MissingObjects) -> canopy_core::Result<T>,
) -> canopy_core::Result<T> {
    match read(MissingObjects::Fail) {
        Err(canopy_core::Error::NotDownloaded {
            remote: Some(remote),
            ..
        }) => {
            if allowed() {
                with_progress(emit, Some(remote), || read(MissingObjects::Download))
            } else {
                Err(download_refused(emit, Some(remote)))
            }
        }
        other => other,
    }
}

/// A snapshot whose status is filled in even in a partial clone that lacks the tree of `HEAD`:
/// the history comes first and never fails because of it; if downloading the tree fails the
/// snapshot carries the reason ([`RepoSnapshot::status_unavailable`]).
fn snapshot_completing(
    repo: &Repo,
    limit: usize,
    emit: &impl Fn(Event),
    allowed: &impl Fn() -> bool,
) -> canopy_core::Result<RepoSnapshot> {
    let mut snap = repo.snapshot(limit)?;
    if snap
        .status_unavailable
        .as_ref()
        .is_some_and(|u| u.can_download)
    {
        if !allowed() {
            let error = download_refused(emit, repo.promisor_remote());
            snap.status_unavailable = Some(canopy_core::StatusUnavailable {
                message: error.to_string(),
                can_download: false,
            });
            return Ok(snap);
        }
        with_progress(emit, repo.promisor_remote(), || {
            repo.complete_status(&mut snap, MissingObjects::Download);
        });
        if let Some(u) = &snap.status_unavailable {
            tracing::warn!(reason = %canopy_core::cli::redact_url(&u.message), "status not downloaded");
        }
    }
    Ok(snap)
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
        let delete = |force| Op::DeleteBranch {
            name: "topic".into(),
            force,
        };
        assert_eq!(delete(false).kind(), OpKind::SafeDelete("topic".into()));
        assert_eq!(delete(true).kind(), OpKind::Other);
    }
}

/// Reads in partial clones: coalescing, offline first, downloads.
#[cfg(test)]
mod read_tests {
    use std::cell::{Cell, RefCell};

    use canopy_core::Error;

    use super::*;

    fn oid(n: u8) -> Oid {
        Oid::from_bytes(&[n; 20]).expect("oid")
    }

    fn diff(source: DiffSource, path: &str) -> Job {
        Job::Diff(DiffKey {
            source,
            path: path.into(),
            old_path: None,
            conflicted: false,
            context: 3,
            whitespace: Whitespace::Show,
            highlight: false,
        })
    }

    /// A short description of each job, to compare queues.
    fn describe(jobs: &[Job]) -> Vec<String> {
        jobs.iter()
            .map(|j| match j {
                Job::Details(o) => format!("details {}", canopy_core::short_id(*o)),
                Job::Diff(k) => format!("diff {:?} {}", k.source, k.path),
                Job::Op(op) => format!("op {}", op.label()),
                other => other.kind().to_owned(),
            })
            .collect()
    }

    #[test]
    fn a_burst_of_selections_keeps_only_the_last_details_and_diff() {
        let jobs = vec![
            Job::Details(oid(1)),
            diff(DiffSource::Commit(oid(1)), "a"),
            Job::Op(Op::Stage(vec!["x".into()])),
            Job::Details(oid(2)),
            diff(DiffSource::Commit(oid(2)), "b"),
            diff(DiffSource::Staged, "s"),
            Job::HeadMessage,
            diff(DiffSource::Commit(oid(3)), "c"),
            Job::Op(Op::UnstageAll),
            Job::Details(oid(3)),
            diff(DiffSource::Unstaged, "u1"),
            diff(DiffSource::Unstaged, "u2"),
        ];
        let kept = describe(&coalesce_reads(jobs));
        assert_eq!(
            kept,
            [
                "op Staging".to_owned(),
                format!("diff {:?} s", DiffSource::Staged),
                "head-message".to_owned(),
                format!("diff {:?} c", DiffSource::Commit(oid(3))),
                "op Unstaging".to_owned(),
                format!("details {}", canopy_core::short_id(oid(3))),
                format!("diff {:?} u2", DiffSource::Unstaged),
            ]
        );
    }

    #[test]
    fn jobs_without_reads_to_supersede_pass_through_unchanged() {
        let jobs = vec![
            Job::Op(Op::StageAll),
            Job::HeadMessage,
            Job::Conflict("f".into()),
            Job::Op(Op::UnstageAll),
        ];
        let before = describe(&jobs);
        assert_eq!(describe(&coalesce_reads(jobs)), before);
        assert!(coalesce_reads(Vec::new()).is_empty());
    }

    /// Runs `git` in `dir` with a fixed identity and none of the developer's configuration. (The
    /// downloads the worker runs through `canopy_core::cli` inherit this process's environment,
    /// which cannot be changed per test without racing the others.)
    fn git(dir: &Path, args: &[&str]) -> String {
        let root = dir
            .ancestors()
            .find(|a| a.join("home").join("gitconfig").is_file())
            .expect("inside the test directory");
        let out = std::process::Command::new("git")
            .current_dir(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "Ada Lovelace")
            .env("GIT_AUTHOR_EMAIL", "ada@example.com")
            .env("GIT_COMMITTER_NAME", "Ada Lovelace")
            .env("GIT_COMMITTER_EMAIL", "ada@example.com")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", root.join("home").join("gitconfig"))
            .env("HOME", root.join("home"))
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    /// A source repository with three commits, and a partial clone of it made with `filter`.
    /// Returns the root, the source, the clone and the oldest and middle commits.
    fn partial_clone(filter: &str) -> (tempfile::TempDir, PathBuf, PathBuf, Oid, Oid) {
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(root.path().join("home")).expect("home");
        std::fs::write(root.path().join("home").join("gitconfig"), "").expect("config");
        let src = root.path().join("src");
        std::fs::create_dir_all(&src).expect("src");
        git(&src, &["init", "-q", "-b", "main"]);
        for (k, v) in [
            ("commit.gpgsign", "false"),
            ("core.autocrlf", "false"),
            ("uploadpack.allowFilter", "true"),
            ("uploadpack.allowAnySHA1InWant", "true"),
        ] {
            git(&src, &["config", k, v]);
        }
        for (i, text) in ["one\n", "two\n", "three\n"].into_iter().enumerate() {
            std::fs::write(src.join("a.txt"), text).expect("write");
            git(&src, &["add", "a.txt"]);
            git(&src, &["commit", "-q", "-m", &format!("commit {i}")]);
        }
        let id = |rev: &str| Oid::from_str(&git(&src, &["rev-parse", rev])).expect("oid");
        let (oldest, middle) = (id("HEAD~2"), id("HEAD~1"));
        let url = format!("file://{}", src.to_string_lossy().replace('\\', "/"));
        let url = url.replacen("file://C:", "file:///C:", 1);
        let copy = root.path().join("copy");
        git(
            root.path(),
            &[
                "clone",
                "-q",
                "-c",
                "core.autocrlf=false",
                filter,
                &url,
                "copy",
            ],
        );
        (root, src, copy, oldest, middle)
    }

    /// Events of `worker` until `done` accepts one, without the snapshots the file watcher may
    /// trigger; fails after a timeout.
    fn events_until(worker: &Worker, done: impl Fn(&Event) -> bool) -> Vec<Event> {
        let mut seen = Vec::new();
        loop {
            let e = worker
                .events
                .recv_timeout(Duration::from_secs(60))
                .expect("the worker answers");
            if matches!(e, Event::Snapshot(_) | Event::Status(_)) {
                continue;
            }
            let last = done(&e);
            seen.push(e);
            if last {
                return seen;
            }
        }
    }

    fn progress(events: &[Event]) -> Vec<&str> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Progress(line) => Some(line.as_str()),
                _ => None,
            })
            .collect()
    }

    /// [`partial_clone`] inside a folder the user trusted, as the downloads need.
    fn trusted_partial_clone(filter: &str) -> (tempfile::TempDir, PathBuf, PathBuf, Oid, Oid) {
        let made = partial_clone(filter);
        crate::trust::add_for_tests(made.0.path().to_path_buf());
        made
    }

    #[test]
    fn an_untrusted_partial_clone_downloads_nothing_and_says_why() {
        // Blobless: the file list is offline, the line counts would need a download.
        let (_root, _src, copy, _oldest, middle) = partial_clone("--filter=blob:none");
        let worker = Worker::spawn(copy.clone(), egui::Context::default());
        worker.send(Job::Details(middle));
        let events = events_until(&worker, |e| matches!(e, Event::DownloadsRefused { .. }));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::Details(d) if !d.stats_known && d.files.len() == 1)),
            "the file list still shows: {events:?}"
        );
        assert!(
            matches!(events.last(), Some(Event::DownloadsRefused { remote }) if remote == "origin"),
            "{events:?}"
        );
        assert!(progress(&events).is_empty(), "no download ran: {events:?}");

        // Treeless: even the file list needs a download, so the read fails and explains.
        let (_root, _src, copy, oldest, _) = partial_clone("--filter=tree:0");
        let worker = Worker::spawn(copy, egui::Context::default());
        worker.send(Job::Details(oldest));
        let events = events_until(&worker, |e| matches!(e, Event::DetailsFailed { .. }));
        let Some(Event::DetailsFailed {
            message,
            can_download,
            ..
        }) = events.last()
        else {
            panic!("expected a failure, got {events:?}");
        };
        assert!(
            !can_download,
            "retrying cannot help until the folder is trusted"
        );
        assert!(message.contains("trusted folder"), "{message}");
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::DownloadsRefused { remote } if remote == "origin")),
            "{events:?}"
        );
        assert!(progress(&events).is_empty(), "no download ran: {events:?}");
    }

    #[test]
    fn details_in_a_blobless_clone_show_the_file_list_first_and_the_counts_after_downloading() {
        let (_root, _src, copy, _oldest, middle) = trusted_partial_clone("--filter=blob:none");
        let worker = Worker::spawn(copy, egui::Context::default());

        // The blobs of this commit are not local: the list comes first, offline, and the line
        // counts follow once downloaded, with a progress line around the download.
        worker.send(Job::Details(middle));
        let events = events_until(&worker, |e| {
            matches!(e, Event::Progress(l) if l.is_empty())
                || matches!(e, Event::DetailsFailed { .. })
        });
        let details: Vec<(bool, usize)> = events
            .iter()
            .filter_map(|e| match e {
                Event::Details(d) => Some((d.stats_known, d.files.len())),
                _ => None,
            })
            .collect();
        assert_eq!(details, [(false, 1)], "{events:?}");
        assert_eq!(progress(&events), ["Downloading from origin…", ""]);
        let events = events_until(&worker, |e| matches!(e, Event::Details(d) if d.stats_known));
        let Some(Event::Details(full)) = events.last() else {
            panic!("expected details with counts, got {events:?}");
        };
        assert_eq!((full.files[0].additions, full.files[0].deletions), (1, 1));

        // Everything is local now: no further download for the same commit.
        worker.send(Job::Details(middle));
        let events = events_until(&worker, |e| matches!(e, Event::Details(_)));
        assert!(progress(&events).is_empty(), "{events:?}");
    }

    #[test]
    fn a_failed_download_of_details_is_reported_with_its_oid_and_can_be_retried() {
        let (root, src, copy, oldest, _) = trusted_partial_clone("--filter=tree:0");
        let worker = Worker::spawn(copy, egui::Context::default());
        let moved = root.path().join("src-moved");
        std::fs::rename(&src, &moved).expect("hide the remote");

        worker.send(Job::Details(oldest));
        let events = events_until(&worker, |e| matches!(e, Event::DetailsFailed { .. }));
        let Some(Event::DetailsFailed {
            oid,
            message,
            can_download,
        }) = events.last()
        else {
            panic!("expected a failure, got {events:?}");
        };
        assert_eq!(*oid, oldest);
        assert!(*can_download, "{message}");
        assert!(message.contains("partial clone"), "{message}");
        assert!(message.contains("origin"), "{message}");
        // The status bar line is cleared again after the failed download.
        assert_eq!(progress(&events), ["Downloading from origin…", ""]);

        std::fs::rename(&moved, &src).expect("restore the remote");
        worker.send(Job::Details(oldest));
        let events = events_until(&worker, |e| matches!(e, Event::Details(_)));
        assert!(matches!(events.last(), Some(Event::Details(d)) if d.oid == oldest));
    }

    #[test]
    fn a_read_retries_with_downloading_only_when_a_remote_can_supply_it() {
        let lines = RefCell::new(Vec::new());
        let refused = RefCell::new(Vec::new());
        let emit = |e: Event| match e {
            Event::Progress(line) => lines.borrow_mut().push(line),
            Event::DownloadsRefused { remote } => refused.borrow_mut().push(remote),
            _ => {}
        };
        let not_downloaded = |remote: Option<&str>| Error::NotDownloaded {
            remote: remote.map(str::to_owned),
            reason: None,
        };

        // Succeeds offline: one call, no progress.
        let calls = Cell::new(0);
        let r = read_downloading(&emit, &|| true, |m| {
            calls.set(calls.get() + 1);
            assert_eq!(m, MissingObjects::Fail);
            Ok(1)
        });
        assert_eq!(r.expect("ok"), 1);
        assert_eq!(calls.get(), 1);
        assert!(lines.borrow().is_empty());

        // Missing objects and a remote: downloads, bracketed by progress lines.
        let modes = RefCell::new(Vec::new());
        let r = read_downloading(&emit, &|| true, |m| {
            modes.borrow_mut().push(m);
            match m {
                MissingObjects::Fail => Err(not_downloaded(Some("origin"))),
                MissingObjects::Download => Ok(2),
            }
        });
        assert_eq!(r.expect("downloaded"), 2);
        assert_eq!(
            *modes.borrow(),
            [MissingObjects::Fail, MissingObjects::Download]
        );
        assert_eq!(
            *lines.borrow(),
            ["Downloading from origin…".to_owned(), String::new()]
        );

        // No remote to download from: the error is returned as it is, with no network attempt.
        lines.borrow_mut().clear();
        let calls = Cell::new(0);
        let r: canopy_core::Result<u8> = read_downloading(&emit, &|| true, |_| {
            calls.set(calls.get() + 1);
            Err(not_downloaded(None))
        });
        assert!(matches!(r, Err(Error::NotDownloaded { remote: None, .. })));
        assert_eq!(calls.get(), 1);
        assert!(lines.borrow().is_empty());
        assert!(refused.borrow().is_empty());

        // A remote but an untrusted folder: nothing is downloaded, and the refusal is reported
        // with the remote, in an error that offers no retry.
        let modes = RefCell::new(Vec::new());
        let r: canopy_core::Result<u8> = read_downloading(&emit, &|| false, |m| {
            modes.borrow_mut().push(m);
            Err(not_downloaded(Some("origin")))
        });
        let Err(e) = r else {
            panic!("an untrusted folder must not download")
        };
        assert_eq!(*modes.borrow(), [MissingObjects::Fail]);
        assert!(!e.can_download(), "{e}");
        assert!(e.to_string().contains("trusted folder"), "{e}");
        assert_eq!(*refused.borrow(), ["origin".to_owned()]);
        assert!(lines.borrow().is_empty());
    }
}
