//! One background thread per open repository. The UI sends [`Job`]s and drains [`Event`]s; it
//! never calls git itself, so a slow disk, a large repository or the network cannot freeze it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, unbounded};

use canopy_core::ops::{HunkAction, PullStrategy, ResetMode};
use canopy_core::{
    CommitDetails, DiffSource, FileDiff, FileEntry, MissingObjects, Oid, Repo, RepoSnapshot,
    RepoState, Whitespace, WorkingStatus,
};

/// Where a finished git-flow branch is pushed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowPush {
    pub remote: String,
    /// Also delete the finished branch on the remote.
    pub delete_remote: bool,
}

/// A request to change the repository. Each one is followed by a refresh.
#[derive(Debug, Clone)]
pub enum Op {
    /// `git flow init`: write the configuration and create the development branch.
    FlowInit(canopy_core::FlowConfig),
    /// Start a git-flow branch of `kind` called `name` from `base` (the kind's own when `None`).
    FlowStart {
        kind: canopy_core::FlowKind,
        name: String,
        base: Option<String>,
    },
    /// Finish a git-flow branch, then push what it changed when `push` says so.
    FlowFinish {
        kind: canopy_core::FlowKind,
        name: String,
        options: canopy_core::FlowFinish,
        push: Option<FlowPush>,
    },
    /// Delete merged branches (`Repo::remove_merged_branches`); `allow_unproven` is the user's
    /// confirmation to force-delete the ones that only look merged.
    CleanMerged {
        bases: Vec<String>,
        names: Vec<String>,
        allow_unproven: bool,
    },
    /// Bring `tip`, the commit of `base` the user was shown, into the checked-out branch.
    UpdateFromBase {
        base: String,
        strategy: PullStrategy,
        tip: Oid,
    },
    CleanStale {
        candidates: Vec<canopy_core::StaleCandidate>,
        older_than_days: u32,
    },
    AddIgnore {
        rule: canopy_core::IgnoreRule,
        target: canopy_core::IgnoreTarget,
    },
    StopTracking(String),
    /// `git lfs pull`: downloads the LFS objects of the current checkout and writes them into the
    /// working tree.
    LfsPull,
    /// `git lfs fetch`: downloads the LFS objects of the current branch into the local store.
    LfsFetch,
    /// `git lfs track`; `literal` names one file, not a glob.
    LfsTrack {
        pattern: String,
        literal: bool,
    },
    /// `git lfs untrack`, with the pattern as `.gitattributes` spells it.
    LfsUntrack(String),
    LfsLock(String),
    /// `git lfs unlock`; `force` also releases a lock another user holds.
    LfsUnlock {
        path: String,
        force: bool,
    },
    /// `git submodule update` for one submodule or, with `None`, all of them; `init` also clones
    /// those never initialised.
    SubmoduleUpdate {
        path: Option<String>,
        init: bool,
    },
    SubmoduleSync(Option<String>),
    SubmoduleAdd {
        url: String,
        path: String,
        branch: Option<String>,
    },
    SubmoduleRemove(String),
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
    /// A linked working folder (`git worktree add`) on a new branch, or on an existing one.
    AddWorktree {
        folder: PathBuf,
        branch: String,
        create: bool,
        start: Option<String>,
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
    /// Publishes a local branch (checked out or not) to `remote` and makes the remote branch its
    /// upstream.
    PublishBranch {
        name: String,
        remote: String,
    },
    /// Writes `oid` and the commits before it along first parents, `count` in all, as patch
    /// files in `dir`, oldest first; merge commits have no patch and are skipped.
    ExportPatches {
        oid: Oid,
        count: usize,
        dir: PathBuf,
    },
    /// Applies a patch file, or the `.patch` files of a folder in name order, as commits.
    ApplyPatches {
        path: PathBuf,
    },
    StashSave {
        message: Option<String>,
        untracked: bool,
    },
    /// Applies (or pops) the stash at `index` when the operation runs: the latest, `0`.
    StashApply {
        index: usize,
        pop: bool,
    },
    /// Applies (or pops) the stash whose commit is `oid`, wherever it is by then.
    StashApplyCommit {
        oid: Oid,
        pop: bool,
    },
    /// Drops the stash whose commit is `oid`, wherever it is by then.
    StashDrop(Oid),
    /// Fetch every remote. `quiet` skips the success toast (auto-fetch).
    Fetch {
        prune: bool,
        quiet: bool,
    },
    /// Pull. The strategy is always passed to git explicitly; with `follow_config` it is only the
    /// fallback for a repository whose git configuration (`branch.<name>.rebase`, `pull.rebase`,
    /// `pull.ff`) asks for none, and the configured one is used otherwise. Unless `checked`, the
    /// upstream is fetched (pruning as `prune` says) and the pull waits for the user's decision
    /// ([`Event::PullDecision`]) when the branches diverged under a fast-forward-only strategy or
    /// it would likely conflict. A `checked` pull brings in the upstream as already fetched,
    /// without asking the remote again: `checked` is the upstream commit the user was shown, and
    /// if the upstream has moved since (an auto-fetch landed while they decided) the pull asks
    /// again about the new state instead of pulling what they did not see.
    Pull {
        strategy: PullStrategy,
        follow_config: bool,
        prune: bool,
        checked: Option<Oid>,
    },
    /// A push of the current branch. A force push carries `expect`, what the user was asked
    /// about: the worker refuses if the branch or its destination is no longer that.
    Push {
        force: bool,
        expect: Option<canopy_core::PushTarget>,
    },
    AddRemote {
        name: String,
        url: String,
    },
    RemoveRemote(String),
    /// Point a remote at another URL (never one carrying credentials).
    SetRemoteUrl {
        name: String,
        url: String,
    },
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
            Self::Commit { .. } => OpKind::Commit,
            Self::Merge(target, canopy_core::MergeMode::Squash) => OpKind::Squash(target.clone()),
            Self::Fetch { .. } => OpKind::Fetch,
            Self::Pull { .. } => OpKind::Pull,
            Self::Push { .. } | Self::PublishBranch { .. } => OpKind::Push,
            Self::DeleteBranch { name, force: false } => OpKind::SafeDelete(name.clone()),
            Self::Merge(..) | Self::Rebase(_) | Self::UpdateFromBase { .. } => OpKind::Merge,
            Self::StashSave { .. } => OpKind::Stash,
            Self::StashApply { pop: true, .. } | Self::StashApplyCommit { pop: true, .. } => {
                OpKind::Pop
            }
            Self::CreateBranch { .. } => OpKind::Branch,
            Self::AddWorktree { .. } => OpKind::Worktree,
            Self::AddRemote { .. } => OpKind::Publish,
            Self::LfsPull
            | Self::LfsFetch
            | Self::LfsTrack { .. }
            | Self::LfsUntrack(_)
            | Self::LfsLock(_)
            | Self::LfsUnlock { .. } => OpKind::Lfs,
            _ => OpKind::Other,
        }
    }

    /// Present-tense label shown while the operation runs.
    pub fn label(&self) -> String {
        match self {
            Self::FlowInit(_) => "Initialising git-flow".into(),
            Self::FlowStart { kind, name, .. } => {
                format!("Starting {} {name}", kind.label().to_lowercase())
            }
            Self::FlowFinish { kind, name, .. } => {
                format!("Finishing {} {name}", kind.label().to_lowercase())
            }
            Self::CleanMerged { names, .. } => format!(
                "Deleting {} merged {}",
                names.len(),
                if names.len() == 1 {
                    "branch"
                } else {
                    "branches"
                }
            ),
            Self::UpdateFromBase { base, .. } => format!("Updating from {base}"),
            Self::CleanStale { .. } => "Cleaning up stale worktrees and branches".into(),
            Self::Stage(_) | Self::StageAll => "Staging".into(),
            Self::AddIgnore { .. } => "Adding ignore rule".into(),
            Self::StopTracking(path) => format!("Untracking {path}"),
            Self::LfsPull => "Downloading LFS files".into(),
            Self::LfsFetch => "Fetching LFS objects".into(),
            Self::LfsTrack { pattern, .. } => format!("Tracking {pattern} with Git LFS"),
            Self::LfsUntrack(pattern) => format!("Untracking {pattern} from Git LFS"),
            Self::LfsLock(path) => format!("Locking {path}"),
            Self::LfsUnlock { path, force: true } => format!("Force-unlocking {path}"),
            Self::LfsUnlock { path, .. } => format!("Unlocking {path}"),
            Self::SubmoduleUpdate { path: Some(p), .. } => format!("Updating submodule {p}"),
            Self::SubmoduleUpdate { path: None, .. } => "Updating submodules".into(),
            Self::SubmoduleSync(Some(p)) => format!("Syncing submodule {p}"),
            Self::SubmoduleSync(None) => "Syncing submodules".into(),
            Self::SubmoduleAdd { path, .. } => format!("Adding submodule {path}"),
            Self::SubmoduleRemove(p) => format!("Removing submodule {p}"),
            Self::Unstage(_) | Self::UnstageAll => "Unstaging".into(),
            Self::Discard(_) => "Discarding changes".into(),
            Self::Hunk {
                action: HunkAction::Stage,
                ..
            } => "Staging hunk".into(),
            Self::Hunk {
                action: HunkAction::Unstage,
                ..
            } => "Unstaging hunk".into(),
            Self::Hunk {
                action: HunkAction::Discard,
                ..
            } => "Discarding hunk".into(),
            Self::Lines {
                action: HunkAction::Stage,
                ..
            } => "Staging lines".into(),
            Self::Lines {
                action: HunkAction::Unstage,
                ..
            } => "Unstaging lines".into(),
            Self::Lines {
                action: HunkAction::Discard,
                ..
            } => "Discarding lines".into(),
            Self::Commit { amend: true, .. } => "Amending commit".into(),
            Self::Commit { .. } => "Committing".into(),
            Self::CreateBranch { name, .. } => format!("Creating branch {name}"),
            Self::AddWorktree { branch, .. } => format!("Adding a worktree for {branch}"),
            Self::Checkout(b) => format!("Checking out {b}"),
            Self::CheckoutRemote { remote, branch } => format!("Checking out {remote}/{branch}"),
            Self::CheckoutDetached(o) => format!("Checking out {}", canopy_core::short_id(*o)),
            Self::RenameBranch { new, .. } => format!("Renaming to {new}"),
            Self::DeleteBranch { name, .. } => format!("Deleting {name}"),
            Self::Merge(t, canopy_core::MergeMode::Squash) => format!("Squashing {t}"),
            Self::Merge(t, _) => format!("Merging {t}"),
            Self::Rebase(t) => format!("Rebasing onto {t}"),
            Self::ContinueOrAbort { state, abort } => format!(
                "{} {}",
                if *abort { "Aborting" } else { "Continuing" },
                state.label().to_lowercase()
            ),
            Self::CherryPick(o) => format!("Cherry-picking {}", canopy_core::short_id(*o)),
            Self::Revert(o) => format!("Reverting {}", canopy_core::short_id(*o)),
            Self::Reset { mode, .. } => format!("{} reset", mode.label()),
            Self::CreateTag { name, .. } => format!("Tagging {name}"),
            Self::DeleteTag(t) => format!("Deleting tag {t}"),
            Self::PushTag { tag, .. } => format!("Pushing tag {tag}"),
            Self::PublishBranch { name, .. } => format!("Publishing {name}"),
            Self::ExportPatches { .. } => "Exporting patches".into(),
            Self::ApplyPatches { .. } => "Applying patches".into(),
            Self::StashSave { .. } => "Stashing".into(),
            Self::StashApply { pop: true, .. } | Self::StashApplyCommit { pop: true, .. } => {
                "Popping stash".into()
            }
            Self::StashApply { .. } | Self::StashApplyCommit { .. } => "Applying stash".into(),
            Self::StashDrop(_) => "Dropping stash".into(),
            Self::Fetch { .. } => "Fetching".into(),
            Self::Pull { .. } => "Pulling".into(),
            Self::Push { force: true, .. } => "Force pushing".into(),
            Self::Push { .. } => "Pushing".into(),
            Self::AddRemote { name, .. } => format!("Adding remote {name}"),
            Self::RemoveRemote(r) => format!("Removing remote {r}"),
            Self::SetRemoteUrl { name, .. } => format!("Changing the URL of {name}"),
            Self::DeleteRemoteBranch { remote, branch } => format!("Deleting {remote}/{branch}"),
            Self::CheckoutRequest { branch, .. } => format!("Checking out {branch}"),
            Self::Resolve {
                path, mark: true, ..
            } => format!("Resolving {path}"),
            Self::Resolve { path, .. } => format!("Saving {path}"),
            Self::ResolveSide { path, side } => format!(
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
            Self::Stage(_)
                | Self::StageAll
                | Self::Unstage(_)
                | Self::UnstageAll
                | Self::Hunk { .. }
                | Self::Lines { .. }
                | Self::Resolve { .. }
                | Self::ResolveSide { .. }
                | Self::Fetch { quiet: true, .. }
        )
    }
}

/// Work for the repository thread.
#[derive(Debug, Clone)]
pub enum Job {
    /// Which local branches are merged into `base`; the serial ties the reply to the newest
    /// request. `squash` also runs the per-branch squash check.
    MergedBranches {
        serial: u64,
        bases: Vec<String>,
        squash: bool,
    },
    /// What bringing `base` into the checked-out branch would do (no fetch).
    BaseCompare {
        serial: u64,
        base: String,
    },
    StaleCandidates {
        serial: u64,
        older_than_days: u32,
    },
    /// Local repository and branch metrics, loaded only when requested.
    Insights(u64),
    /// Ignore management reads; the serial ties the reply to the newest form request.
    Ignore {
        serial: u64,
        query: IgnoreQuery,
    },
    /// The commits that changed one file, followed through renames; the serial ties the reply to
    /// the newest request.
    FileHistory {
        serial: u64,
        from: Option<Oid>,
        path: String,
    },
    /// The commits whose changes added or removed text (the pickaxe); the serial ties the reply to
    /// the newest search.
    SearchChanges {
        serial: u64,
        query: String,
        regex: bool,
    },
    /// Full snapshot with up to this many commits (0 keeps the current limit).
    Refresh(usize),
    /// The user's Retry after a download failed: forget the failure and read again, downloading.
    RetryDownloads,
    /// Status only (working tree changed).
    RefreshStatus,
    /// A folder of the repository could not be watched (from the file watcher): tell the user.
    WatchFailed(String),
    /// Details of one commit.
    Details(Oid),
    /// Whether one commit's signature verifies (runs gpg or ssh-keygen: trusted folders only).
    Signature(Oid),
    /// Diff of one file.
    Diff(DiffKey),
    /// Who last changed each line of a file, as it is in `rev` or, without one, in the working
    /// tree; the serial ties the reply to the newest request.
    Blame {
        serial: u64,
        path: String,
        rev: Option<Oid>,
    },
    /// `HEAD`'s message (to prefill an amend).
    HeadMessage,
    /// Git's pending merge message, read off the UI thread.
    MergeMessage,
    /// The commit template the configuration names (`Repo::commit_template`), for a new commit's
    /// empty message field.
    CommitTemplate,
    /// A commit message for the squash merge of this branch, with git's summary of the squashed
    /// commits (`.git/SQUASH_MSG`) as the body.
    SquashMessage(String),
    /// A conflicted file, parsed for the resolver.
    Conflict(String),
    /// What the configuration asks a pull to do on the current branch (`Repo::pull_config`).
    PullConfig,
    /// The programs the repository needs that the system lacks (`Repo::missing_tools`).
    Tools,
    /// The files stored with Git LFS and whether git-lfs is there. `known`: the tab has seen an
    /// LFS file, so the repository is read for them even where `.gitattributes` at the top does
    /// not say so.
    Lfs {
        known: bool,
    },
    /// The locks on the LFS server (a network read, only when the user asks); the serial ties the
    /// reply to the newest request.
    LfsLocks(u64),
    /// The patterns stored with LFS (`git lfs track`); the serial ties the reply to the newest
    /// request.
    LfsPatterns(u64),
    /// Where a force push of the current branch would go ([`Repo::force_push_target`]), for the
    /// question that asks before it.
    PushTarget,
    /// How the operations that follow have git behave (Settings → Git).
    Options(canopy_core::GitOptions),
    /// A mutation.
    Op(Op),
}

/// A read requested by the ignore management form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IgnoreQuery {
    Preview {
        rule: canopy_core::IgnoreRule,
        target: canopy_core::IgnoreTarget,
    },
    Paths,
    Why(String),
}

#[derive(Debug, Clone)]
pub enum IgnoreData {
    Preview {
        file: PathBuf,
        preview: canopy_core::IgnorePreview,
        truncated: bool,
    },
    Paths(canopy_core::IgnoredPaths),
    Why {
        matched: Option<canopy_core::IgnoreMatch>,
        tracked: bool,
    },
}

/// Repository activity and branch comparisons from one worker read.
#[derive(Debug, Clone)]
pub struct Insights {
    pub repository: canopy_core::RepositoryMetrics,
    pub branches: Vec<canopy_core::BranchMetrics>,
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
    MergedBranches {
        serial: u64,
        bases: Vec<String>,
        result: Result<Arc<[canopy_core::MergedBranch]>, String>,
    },
    BaseCompare {
        serial: u64,
        result: Result<canopy_core::PullPreview, String>,
    },
    StaleCandidates {
        serial: u64,
        result: Result<Arc<[canopy_core::StaleCandidate]>, String>,
    },
    Insights {
        serial: u64,
        result: Result<Arc<Insights>, String>,
    },
    FileHistory {
        serial: u64,
        result: Result<Arc<canopy_core::FileHistory>, String>,
    },
    ChangeSearch {
        serial: u64,
        result: Result<Arc<canopy_core::ChangeSearch>, String>,
    },
    Ignore {
        serial: u64,
        result: Result<Arc<IgnoreData>, String>,
    },
    Snapshot(Arc<RepoSnapshot>),
    Status(WorkingStatus),
    Details(Arc<CommitDetails>),
    /// How verifying the signature of this commit went; `None` when it was not tried (the
    /// repository is outside every trusted folder) or git could not run.
    Signature {
        oid: Oid,
        status: Option<canopy_core::SignatureStatus>,
    },
    /// The details of this commit could not be read. `can_download` says that it is a partial
    /// clone that has not downloaded them, which trying again can fix.
    DetailsFailed {
        oid: Oid,
        message: String,
        can_download: bool,
    },
    Diff(DiffKey, Result<Arc<FileDiff>, String>),
    Blame {
        serial: u64,
        result: Result<Arc<canopy_core::Blame>, String>,
    },
    /// The diff failed only because a partial clone has not downloaded what it needs, and a
    /// retry could fetch it: the tab remembers that until the user asks again.
    DiffNotDownloaded(DiffKey),
    /// A read needed a download from this promisor remote, which Canopy did not run because the
    /// repository is outside every trusted folder (`trust.rs`). `trust` is the version of the
    /// trusted list that said so (`trust::changes()`): a tab that has seen a newer one knows the
    /// refusal is out of date.
    DownloadsRefused {
        remote: String,
        trust: u64,
    },
    /// The working and Git folders trusting this repository covers, read off the UI thread.
    TrustFolders(Vec<PathBuf>),
    /// Whether Canopy trusts this repository, as of the version of the trusted list
    /// (`trust::changes()`) that said so; sent when either changes. An untrusted repository is
    /// read-only (`read_only_message`).
    Trust {
        trusted: bool,
        version: u64,
    },
    /// Syntax highlighting for the diff (sent before with the same key). It comes from the
    /// highlighter threads, so it may arrive after a newer diff; it applies only to this one.
    Highlight(DiffKey, Arc<FileDiff>, Arc<crate::highlight::Highlights>),
    HeadMessage(String),
    /// The answer to [`Job::PullConfig`]: git's own message when the configuration cannot be
    /// read.
    PullConfig(Result<canopy_core::PullConfig, String>),
    /// The answer to [`Job::Tools`].
    ToolsMissing(Vec<canopy_core::MissingTool>),
    /// The answer to [`Job::Lfs`].
    Lfs(Arc<LfsInfo>),
    /// The answer to [`Job::LfsLocks`]: git-lfs's own message when it cannot list them.
    LfsLocks {
        serial: u64,
        result: Result<Vec<canopy_core::LfsLock>, String>,
    },
    /// The answer to [`Job::LfsPatterns`].
    LfsPatterns {
        serial: u64,
        result: Result<Vec<canopy_core::LfsPattern>, String>,
    },
    /// The answer to [`Job::PushTarget`]: git's own message when it cannot tell.
    PushTarget(Result<canopy_core::PushTarget, String>),
    MergeMessage(String),
    /// The answer to [`Job::CommitTemplate`]: `None` without a template, git's own message when it
    /// cannot be read.
    CommitTemplate(Result<Option<String>, String>),
    /// A new commit message after a squash, never an amend prefill.
    SquashMessage(String),
    Conflict {
        path: String,
        kind: Option<canopy_core::ConflictKind>,
        file: Result<Arc<canopy_core::ConflictFile>, String>,
    },
    OpStarted {
        label: String,
        kind: OpKind,
    },
    Progress(String),
    OpFinished {
        label: String,
        kind: OpKind,
        announce: bool,
        result: Result<String, String>,
    },
    Failed(String),
    /// Changes on disk are not watched (a folder could not be watched): from [`Job::WatchFailed`].
    WatchFailed(String),
    /// A pull, rebase or merge finished, but putting the uncommitted changes back (autostash) left
    /// conflicts to resolve.
    AutostashConflict,
    /// A pull waits for the user: the branches diverged, or it would likely conflict.
    PullDecision(Box<PullDecision>),
    /// The operation `label` (a pull, rebase, merge, cherry-pick, revert or stash apply) stopped
    /// on conflicts for the user to resolve; `autostashed` when git holds the uncommitted changes
    /// aside meanwhile. `stash` when it was a stash applied or popped: nothing is in progress
    /// then (no continue or abort), and the stash is kept.
    Conflicted {
        label: String,
        autostashed: bool,
        stash: bool,
    },
}

/// What the repository stores with Git LFS ([`Job::Lfs`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LfsInfo {
    /// The repository stores files with LFS.
    pub in_use: bool,
    /// Every LFS file in the index, with whether its object is downloaded.
    pub files: Vec<canopy_core::LfsFile>,
    /// Set when `git lfs` does not work here: what to install.
    pub missing: Option<canopy_core::MissingTool>,
}

/// What a pull asks before it runs ([`Op::Pull`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullDecision {
    pub preview: canopy_core::PullPreview,
    /// The strategy the pull would use.
    pub strategy: PullStrategy,
    /// The branches diverged and the strategy only fast-forwards: ask how to combine them.
    pub diverged: bool,
    /// The git configuration chose the strategy, not Canopy's setting.
    pub configured: bool,
    /// The upstream moved after the preview the user answered: this is a new question about the
    /// new state.
    pub moved: bool,
}

/// The strategy a pull uses: what the git configuration asks for when `follow_config` and it
/// asks for one (then also `true`), else `strategy`.
fn pull_strategy(repo: &Repo, strategy: PullStrategy, follow_config: bool) -> (PullStrategy, bool) {
    if !follow_config {
        return (strategy, false);
    }
    match repo.pull_config() {
        Ok(config) => config.strategy.map_or((strategy, false), |s| (s, true)),
        Err(e) => {
            tracing::warn!(error = %e, "could not read the pull configuration");
            (strategy, false)
        }
    }
}

/// What a pull does after its preview ([`decide_pull`]).
#[derive(Debug)]
enum PullNext {
    /// Ask the user first.
    Ask(PullDecision),
    /// The upstream is fetched, and this is its commit: bring it in without fetching again.
    Fetched(Option<Oid>),
    /// No upstream to preview: `git pull` reports why.
    Pull,
}

/// Fetches and previews a pull, and says what comes next.
fn decide_pull(
    repo: &Repo,
    strategy: PullStrategy,
    follow_config: bool,
    prune: bool,
    progress: impl FnMut(&str),
) -> canopy_core::Result<PullNext> {
    let Some(preview) = repo.pull_preview(prune, progress)? else {
        return Ok(PullNext::Pull);
    };
    Ok(next_step(repo, preview, strategy, follow_config, false))
}

/// What a checked pull does once the user has answered: it goes ahead if the upstream is still
/// the commit they were shown (`seen`), and otherwise asks about the new state, whatever it
/// holds, since what they answered is not what would come in.
fn recheck_pull(
    repo: &Repo,
    seen: Oid,
    strategy: PullStrategy,
    follow_config: bool,
) -> canopy_core::Result<PullNext> {
    let Some(preview) = repo.compare_upstream()? else {
        return Ok(PullNext::Pull);
    };
    if preview.tip == Some(seen) {
        return Ok(PullNext::Fetched(preview.tip));
    }
    Ok(next_step(repo, preview, strategy, follow_config, true))
}

/// Ask about `preview` when the pull would diverge under fast-forward only or likely conflict
/// (or always when `moved` and something would come in), else bring the fetched upstream in.
fn next_step(
    repo: &Repo,
    preview: canopy_core::PullPreview,
    strategy: PullStrategy,
    follow_config: bool,
    moved: bool,
) -> PullNext {
    let (strategy, configured) = pull_strategy(repo, strategy, follow_config);
    let diverged = preview.diverged() && strategy == PullStrategy::FastForwardOnly;
    // A moved upstream that brings nothing in is not worth a question.
    let moved_for_nothing = moved && preview.behind == 0;
    if !moved_for_nothing && (diverged || preview.risky() || moved) {
        PullNext::Ask(PullDecision {
            preview,
            strategy,
            diverged,
            configured,
            moved,
        })
    } else {
        PullNext::Fetched(preview.tip)
    }
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
    /// A pull: the remote's branches moved, so the forge's requests may have too.
    Pull,
    /// A push: it can open, update or close a request.
    Push,
    /// `git branch -d`: when Git refuses an unmerged branch, offer to force the delete.
    SafeDelete(String),
    /// A merge, squash or rebase invoked from the toolbar.
    Merge,
    Stash,
    Pop,
    Branch,
    Publish,
    /// `git worktree add`: its result goes back to the New worktree dialog that asked for it.
    Worktree,
    /// A Git LFS command: its lists (patterns, locks, files) are read again.
    Lfs,
    Other,
}

impl OpKind {
    /// Whether finishing this operation means the forge's pull or merge requests may have
    /// changed (it talked to the remote).
    pub fn syncs_remote(&self) -> bool {
        matches!(self, Self::Fetch | Self::Pull | Self::Push)
    }
}

/// The UI's handle on a repository thread.
///
/// A worker made with [`Worker::lazy`] has no thread and no file watcher yet: [`Worker::start`]
/// (the first time its tab is shown) starts them. Until then jobs sent to it are dropped, so a
/// tab nobody has looked at costs nothing and never touches its repository.
#[derive(Debug)]
pub struct Worker {
    jobs: Sender<Job>,
    /// Events from the thread.
    pub events: Receiver<Event>,
    /// Dropping this ends the repository's watch on the shared watcher thread.
    stop_watcher: Sender<()>,
    /// Disconnects once the repository thread and the file watcher's have both ended.
    ended: Receiver<()>,
    /// What [`Worker::start`] needs; `None` once the worker is running.
    unstarted: Option<Unstarted>,
}

/// What a worker that has not started yet remembers.
#[derive(Debug)]
struct Unstarted {
    path: PathBuf,
    ctx: egui::Context,
    /// Whether starting spawns the threads (tests start a worker with no threads).
    threads: bool,
}

/// The channels and thread handles of a running worker.
struct Running {
    jobs: Sender<Job>,
    events: Receiver<Event>,
    stop_watcher: Sender<()>,
    ended: Receiver<()>,
}

impl Worker {
    /// Starts the thread (and a file watcher) for the repository at `path`.
    #[cfg(test)]
    pub fn spawn(path: PathBuf, ctx: &egui::Context) -> Self {
        let running = Self::spawn_threads(path, ctx);
        Self {
            jobs: running.jobs,
            events: running.events,
            stop_watcher: running.stop_watcher,
            ended: running.ended,
            unstarted: None,
        }
    }

    /// A worker that starts its thread and watcher only when [`Worker::start`] is called.
    pub fn lazy(path: PathBuf, ctx: &egui::Context) -> Self {
        Self::not_started(Unstarted {
            path,
            ctx: ctx.clone(),
            threads: true,
        })
    }

    fn not_started(unstarted: Unstarted) -> Self {
        // Channels with no other end: nothing arrives and nothing is kept.
        let (stop_watcher, _) = crossbeam_channel::bounded(0);
        let (_, ended) = crossbeam_channel::bounded(0);
        Self {
            jobs: unbounded().0,
            events: unbounded().1,
            stop_watcher,
            ended,
            unstarted: Some(unstarted),
        }
    }

    /// Whether the thread and the watcher run (a worker from [`Worker::spawn`] always does).
    pub fn is_started(&self) -> bool {
        self.unstarted.is_none()
    }

    /// Starts the thread and the watcher of a [`Worker::lazy`] worker. Returns whether it started
    /// them now: `false` when it was running already, so a caller sends the first job once.
    pub fn start(&mut self) -> bool {
        let Some(unstarted) = self.unstarted.take() else {
            return false;
        };
        if unstarted.threads {
            let running = Self::spawn_threads(unstarted.path, &unstarted.ctx);
            self.jobs = running.jobs;
            self.events = running.events;
            self.stop_watcher = running.stop_watcher;
            self.ended = running.ended;
        }
        true
    }

    fn spawn_threads(path: PathBuf, ctx: &egui::Context) -> Running {
        let (jobs, job_rx) = unbounded::<Job>();
        let (ev_tx, events) = unbounded::<Event>();
        let thread_ctx = ctx.clone();
        let thread_path = path.clone();
        let failed = ev_tx.clone();
        let (ended_tx, ended) = crossbeam_channel::bounded(0);
        let run_ended = ended_tx.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("repo:{}", path.display()))
            .spawn(move || {
                let _ended = run_ended;
                run(thread_path, job_rx, ev_tx, thread_ctx);
            });
        if let Err(e) = spawned {
            // Without a thread the tab would wait for its first snapshot forever: show why.
            tracing::error!("could not start the repository thread: {e}");
            let _ = failed.send(Event::Failed(format!(
                "Could not start the repository thread: {e}"
            )));
            ctx.request_repaint();
        }
        let (stop_watcher, stop) = crossbeam_channel::bounded(0);
        crate::watcher::watch(path, jobs.clone(), stop, ended_tx);
        Running {
            jobs,
            events,
            stop_watcher,
            ended,
        }
    }

    /// A worker with no threads: tests feed its events and read its jobs.
    #[cfg(test)]
    pub fn detached() -> (Self, Sender<Event>, Receiver<Job>) {
        let (jobs, job_rx) = unbounded();
        let (ev_tx, events) = unbounded();
        let (stop_watcher, _) = crossbeam_channel::bounded(0);
        let (_, ended) = crossbeam_channel::bounded(0);
        let worker = Self {
            jobs,
            events,
            stop_watcher,
            ended,
            unstarted: None,
        };
        (worker, ev_tx, job_rx)
    }

    /// A [`Worker::lazy`] worker with no threads either: [`Worker::start`] only marks it started,
    /// and the channels behave as [`Worker::detached`]'s.
    #[cfg(test)]
    pub fn lazy_detached(path: PathBuf) -> (Self, Sender<Event>, Receiver<Job>) {
        let (mut worker, ev_tx, job_rx) = Self::detached();
        worker.unstarted = Some(Unstarted {
            path,
            ctx: egui::Context::default(),
            threads: false,
        });
        (worker, ev_tx, job_rx)
    }

    /// Queues a job. A worker that has not started drops it, and says so (WARN): the shown tab is
    /// always loaded, and what runs for a background tab asks [`Worker::is_started`] first (its
    /// tab loads everything when it starts), so a job that gets here is a mistake in the caller.
    pub fn send(&self, job: Job) {
        if self.unstarted.is_some() {
            tracing::warn!(
                job = job.kind(),
                "a job was sent to a tab that is not loaded, and was dropped"
            );
            #[cfg(test)]
            UNSTARTED_SENDS.with(|sent| sent.borrow_mut().push(job.kind()));
            return;
        }
        let _ = self.jobs.send(job);
    }

    /// The kinds of the jobs this thread's tests sent to a worker that had not started, oldest
    /// first, and forgets them: a test of background work asserts it is empty.
    #[cfg(test)]
    pub fn take_unstarted_sends() -> Vec<&'static str> {
        UNSTARTED_SENDS.with(|sent| std::mem::take(&mut *sent.borrow_mut()))
    }
}

#[cfg(test)]
thread_local! {
    static UNSTARTED_SENDS: std::cell::RefCell<Vec<&'static str>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// The threads of dropped workers that may still be running ([`wait_for_threads`]).
static ENDING: Mutex<Vec<Receiver<()>>> = Mutex::new(Vec::new());

impl Drop for Worker {
    /// Stops the repository thread and its watch without waiting (closing a tab never stalls
    /// the UI); the process waits for them before it exits ([`wait_for_threads`]). A test binary,
    /// which has no such moment, waits here.
    fn drop(&mut self) {
        // Dropping the senders ends the watch (the shared watcher drops its copy of the jobs
        // sender), then the repository thread once that copy has gone too.
        drop(std::mem::replace(
            &mut self.stop_watcher,
            crossbeam_channel::bounded(0).0,
        ));
        drop(std::mem::replace(&mut self.jobs, unbounded().0));
        let ended = std::mem::replace(&mut self.ended, crossbeam_channel::bounded(0).1);
        if self.unstarted.is_some() {
            // No threads ever ran: nothing to wait for.
            return;
        }
        if cfg!(test) {
            let _ = ended.recv_timeout(Duration::from_secs(1));
        } else {
            ENDING
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(ended);
        }
    }
}

/// Waits, up to `within` in all (a running operation may take longer), for the threads of
/// dropped workers to end, so a process exiting right after does not tear libgit2 and OpenSSL
/// down while one of them is still using them.
pub fn wait_for_threads(within: Duration) {
    let deadline = Instant::now() + within;
    let ending = std::mem::take(&mut *ENDING.lock().unwrap_or_else(PoisonError::into_inner));
    for ended in ending {
        let _ = ended.recv_deadline(deadline);
    }
}

impl Job {
    /// A short name for logs.
    fn kind(&self) -> &'static str {
        match self {
            Self::Refresh(_) => "refresh",
            Self::RetryDownloads => "retry-downloads",
            Self::Ignore { .. } => "ignore",
            Self::StaleCandidates { .. } => "stale_candidates",
            Self::MergedBranches { .. } => "merged_branches",
            Self::BaseCompare { .. } => "base_compare",
            Self::RefreshStatus => "status",
            Self::WatchFailed(_) => "watch-failed",
            Self::Details(_) => "details",
            Self::Signature(_) => "signature",
            Self::Diff(_) => "diff",
            Self::Blame { .. } => "blame",
            Self::HeadMessage => "head-message",
            Self::MergeMessage => "merge-message",
            Self::CommitTemplate => "commit-template",
            Self::SquashMessage(_) => "squash-message",
            Self::Conflict(_) => "conflict",
            Self::PullConfig => "pull-config",
            Self::Insights(_) => "repository insights",
            Self::FileHistory { .. } => "file-history",
            Self::SearchChanges { .. } => "search-changes",
            Self::Tools => "tools",
            Self::Lfs { .. } => "lfs",
            Self::LfsLocks(_) => "lfs-locks",
            Self::LfsPatterns(_) => "lfs-patterns",
            Self::PushTarget => "push-target",
            Self::Options(_) => "options",
            Self::Op(_) => "operation",
        }
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the thread owns its channels and context; dropping them on exit tells the other side"
)]
fn run(path: PathBuf, jobs: Receiver<Job>, events: Sender<Event>, ctx: egui::Context) {
    // Every event this thread logs carries the repository.
    let _repo = tracing::info_span!("repo", path = %path.display()).entered();
    // Before this thread can start a download: core asks the trust list at every fetch.
    crate::trust::install_gate();
    let emit = |e: Event| {
        let _ = events.send(e);
        ctx.request_repaint();
    };
    let mut limit = 2000;
    // Bumped for every diff request; a highlight request made for an older one is stale.
    let diff_generation = Arc::new(AtomicU64::new(0));
    // The last diff sent. A refresh that finds the same content sends the same `Arc` again, so
    // the view keeps its layout and highlighting instead of rebuilding them on the UI thread.
    let mut last_diff: Option<(DiffKey, Arc<FileDiff>)> = None;
    // The repository is opened again for every batch; the options outlive it.
    let mut options = canopy_core::GitOptions::default();
    // Why the last download of the working tree status failed: later refreshes show it instead
    // of trying the network again, until the user retries.
    let mut status_failure: Option<canopy_core::StatusUnavailable> = None;
    // Whether `git lfs` works, asked once: installing it needs a restart of Canopy anyway.
    let mut lfs_missing: Option<Option<canopy_core::MissingTool>> = None;
    let mut trust_folders = Vec::new();
    // The last (trusted, version of the list) this thread told the tab.
    let mut told_trust: Option<(bool, u64)> = None;
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
                Job::RetryDownloads => {
                    status_failure = None;
                    want_full = Some(0);
                }
                Job::RefreshStatus => want_status = true,
                Job::WatchFailed(error) => emit(Event::WatchFailed(error)),
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
        repo.set_options(options);
        let folders = crate::trust::repository_folders(
            repo.workdir(),
            repo.git().commondir(),
            repo.git().path(),
        );
        if folders != trust_folders {
            trust_folders.clone_from(&folders);
            emit(Event::TrustFolders(folders));
        }
        let trusted = trusted_now(&repo);
        let trust = (trusted, TRUST_VERSION.get());
        if told_trust != Some(trust) {
            told_trust = Some(trust);
            emit(Event::Trust {
                trusted: trust.0,
                version: trust.1,
            });
        }

        // Whether a partial clone may download on its own here; asked only when one needs to.
        let allowed = || trusted_now(&repo);
        let rest = coalesce_reads(rest);
        let mut mutated = false;
        for job in rest {
            let kind = job.kind();
            let started = Instant::now();
            match job {
                Job::MergedBranches {
                    serial,
                    bases,
                    squash,
                } => {
                    // The squash check is `git merge-tree`, which runs the repository's merge
                    // drivers: not in a repository that is not trusted, asked now like the
                    // other jobs. The marks libgit2 gives stay.
                    let result = repo
                        .merged_branches_among(&bases, squash && allowed())
                        .map(Arc::from)
                        .map_err(|error| error.to_string());
                    emit(Event::MergedBranches {
                        serial,
                        bases,
                        result,
                    });
                }
                Job::BaseCompare { serial, base } => {
                    // Its conflict prediction is `git merge-tree`, which runs merge drivers.
                    let result = if allowed() {
                        repo.compare_base(&base).map_err(|error| error.to_string())
                    } else {
                        Err(read_only_message(&repo.programs()))
                    };
                    emit(Event::BaseCompare { serial, result });
                }
                Job::StaleCandidates {
                    serial,
                    older_than_days,
                } => {
                    emit(Event::StaleCandidates {
                        serial,
                        result: repo
                            .stale_candidates(older_than_days, chrono::Utc::now().timestamp())
                            .map(Arc::from)
                            .map_err(|error| error.to_string()),
                    });
                }
                Job::Ignore { serial, query } => {
                    emit(Event::Ignore {
                        serial,
                        result: read_ignore(&repo, &query)
                            .map(Arc::new)
                            .map_err(|e| e.to_string()),
                    });
                }
                Job::Blame { serial, path, rev } => {
                    let trusted = allowed();
                    let result = read_downloading(&emit, &allowed, |missing| {
                        repo.blame(&path, rev, missing, trusted)
                    });
                    if let Err(error) = &result {
                        tracing::warn!(error = %redacted(error), "blame failed");
                    }
                    emit(Event::Blame {
                        serial,
                        result: result.map(Arc::new).map_err(|e| e.to_string()),
                    });
                }
                Job::Signature(oid) => {
                    let status = if allowed() {
                        repo.commit_signature(oid)
                            .map_err(|e| tracing::warn!("signature check failed: {}", redacted(&e)))
                            .ok()
                    } else {
                        None
                    };
                    emit(Event::Signature { oid, status });
                }
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
                                if allowed() {
                                    match with_progress(&emit, repo.promisor_remote(), || {
                                        if allowed() {
                                            let result =
                                                repo.commit_details(oid, MissingObjects::Download);
                                            refused_after(
                                                &emit,
                                                &allowed,
                                                repo.promisor_remote(),
                                                result,
                                            )
                                        } else {
                                            Err(download_refused(&emit, repo.promisor_remote()))
                                        }
                                    }) {
                                        Ok(d) if d.stats_known => emit(Event::Details(Arc::new(d))),
                                        Ok(_) => {}
                                        // The list stays; only the counts are missing.
                                        Err(e) => {
                                            tracing::warn!(error = %redacted(&e), "line counts not downloaded");
                                        }
                                    }
                                } else {
                                    download_refused(&emit, repo.promisor_remote());
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
                    let retryable = result
                        .as_ref()
                        .err()
                        .is_some_and(canopy_core::Error::can_download);
                    let result = result.map(Arc::new).map_err(|e| e.to_string());
                    last_diff = result.as_ref().ok().map(|d| (key.clone(), Arc::clone(d)));
                    let generation = diff_generation.fetch_add(1, Ordering::Relaxed) + 1;
                    let highlighted = match &result {
                        Ok(d) if key.highlight && !key.conflicted => Some(Arc::clone(d)),
                        _ => None,
                    };
                    emit(Event::Diff(key.clone(), result));
                    if retryable {
                        emit(Event::DiffNotDownloaded(key.clone()));
                    }
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
                Job::MergeMessage => {
                    match std::fs::read_to_string(repo.git().path().join("MERGE_MSG")) {
                        Ok(message) => emit(Event::MergeMessage(
                            message
                                .lines()
                                .filter(|line| !line.starts_with('#'))
                                .collect::<Vec<_>>()
                                .join("\n"),
                        )),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => emit(Event::Failed(format!(
                            "Could not read the merge message: {error}"
                        ))),
                    }
                }
                Job::CommitTemplate => emit(Event::CommitTemplate(if allowed() {
                    repo.commit_template().map_err(|e| e.to_string())
                } else {
                    Ok(None)
                })),
                Job::SquashMessage(target) => {
                    let path = repo.git().path().join("SQUASH_MSG");
                    let body = std::fs::read_to_string(path).unwrap_or_default();
                    // Drop git's comment lines; keep the list of squashed commits.
                    let body: Vec<&str> = body.lines().filter(|l| !l.starts_with('#')).collect();
                    let body = body.join("\n");
                    emit(Event::SquashMessage(format!(
                        "Merge {target} (squashed)\n\n{}",
                        body.trim()
                    )));
                }
                Job::Options(o) => {
                    options = o;
                    repo.set_options(o);
                }
                Job::PullConfig => {
                    emit(Event::PullConfig(
                        repo.pull_config().map_err(|e| e.to_string()),
                    ));
                }
                Job::Insights(serial) => {
                    let result = repo.repository_metrics().and_then(|repository| {
                        Ok(Arc::new(Insights {
                            repository,
                            branches: repo.branch_metrics()?,
                        }))
                    });
                    if let Err(error) = &result {
                        tracing::warn!(%error, "repository insights failed");
                    }
                    emit(Event::Insights {
                        serial,
                        result: result.map_err(|e| e.to_string()),
                    });
                }
                Job::FileHistory { serial, from, path } => {
                    let result = repo.file_history(from, &path, FILE_HISTORY_LIMIT);
                    if let Err(error) = &result {
                        tracing::warn!(%error, "file history failed");
                    }
                    emit(Event::FileHistory {
                        serial,
                        result: result.map(Arc::new).map_err(|e| e.to_string()),
                    });
                }
                Job::SearchChanges {
                    serial,
                    query,
                    regex,
                } => {
                    let result = repo.search_changes(None, &query, regex, CHANGE_SEARCH_LIMIT);
                    if let Err(error) = &result {
                        tracing::warn!(%error, "change search failed");
                    }
                    emit(Event::ChangeSearch {
                        serial,
                        result: result.map(Arc::new).map_err(|e| e.to_string()),
                    });
                }
                Job::Tools => emit(Event::ToolsMissing(repo.missing_tools())),
                Job::Lfs { known } => {
                    let in_use = known || repo.uses_lfs();
                    let mut info = LfsInfo {
                        in_use,
                        ..LfsInfo::default()
                    };
                    if in_use {
                        info.missing.clone_from(
                            lfs_missing.get_or_insert_with(|| {
                                canopy_core::git_lfs_missing(repo.workdir())
                            }),
                        );
                        match repo.lfs_files() {
                            Ok(files) => info.files = files,
                            Err(error) => {
                                tracing::warn!(error = %redacted(&error), "LFS files not read");
                                continue;
                            }
                        }
                    }
                    emit(Event::Lfs(Arc::new(info)));
                }
                Job::LfsLocks(serial) => {
                    // git-lfs is a program the repository's own configuration can steer.
                    let result = if allowed() {
                        repo.lfs_locks().map_err(|e| e.to_string())
                    } else {
                        Err(read_only_message(&repo.programs()))
                    };
                    emit(Event::LfsLocks { serial, result });
                }
                Job::LfsPatterns(serial) => {
                    let result = if allowed() {
                        repo.lfs_patterns().map_err(|e| e.to_string())
                    } else {
                        Err(read_only_message(&repo.programs()))
                    };
                    emit(Event::LfsPatterns { serial, result });
                }
                Job::PushTarget => {
                    emit(Event::PushTarget(
                        repo.force_push_target().map_err(|e| e.to_string()),
                    ));
                }
                Job::Conflict(path) => {
                    let kind = repo.conflict_kind(&path).ok().flatten();
                    let file = repo
                        .conflict_file(&path)
                        .map(Arc::new)
                        .map_err(|e| e.to_string());
                    emit(Event::Conflict { path, kind, file });
                }
                Job::Op(mut op) => {
                    let label = op.label();
                    let kind = op.kind();
                    // Asked again now: trust may have been withdrawn since the batch began.
                    if !allowed() {
                        tracing::info!(operation = %label, "operation refused: repository not trusted");
                        let announce = op.announce();
                        emit(Event::OpFinished {
                            label,
                            kind,
                            announce,
                            result: Err(read_only_message(&repo.programs())),
                        });
                        continue;
                    }
                    let stage_all = matches!(op, Op::StageAll);
                    let announce = op.announce();
                    // A pull, a rebase or a merge: it can autostash, and stopping on conflicts is
                    // not a failure.
                    let integrates = matches!(
                        op,
                        Op::Pull { .. } | Op::Rebase(_) | Op::Merge(..) | Op::UpdateFromBase { .. }
                    );
                    // A leftover autostash (an external `git merge --squash --autostash`) is
                    // not this operation's: only one that appears during it is reported.
                    let held_before = integrates && repo.autostash_held();
                    // A cherry-pick, a revert and a stash apply or pop can stop on conflicts as
                    // well. Files already conflicted before they ran are not their stop: git
                    // refused them, and says why.
                    let picks = matches!(op, Op::CherryPick(_) | Op::Revert(_));
                    let applies_stash =
                        matches!(op, Op::StashApply { .. } | Op::StashApplyCommit { .. });
                    let conflicted_before =
                        (picks || applies_stash) && repo.index_conflicted().unwrap_or(true);
                    let pull = match op {
                        Op::Pull {
                            strategy,
                            follow_config,
                            ..
                        } => Some((strategy, follow_config)),
                        _ => None,
                    };
                    tracing::info!(operation = %label, "operation started");
                    emit(Event::OpStarted {
                        label: label.clone(),
                        kind: kind.clone(),
                    });
                    let progress = |line: &str| emit(Event::Progress(line.to_owned()));
                    let decided = match &op {
                        Op::Pull {
                            strategy,
                            follow_config,
                            prune,
                            checked: None,
                        } => Some(decide_pull(
                            &repo,
                            *strategy,
                            *follow_config,
                            *prune,
                            progress,
                        )),
                        Op::Pull {
                            strategy,
                            follow_config,
                            checked: Some(seen),
                            ..
                        } => Some(recheck_pull(&repo, *seen, *strategy, *follow_config)),
                        _ => None,
                    };
                    let fetch_failed = matches!(decided, Some(Err(_)));
                    let result = match decided {
                        Some(Ok(PullNext::Ask(decision))) => {
                            tracing::info!(operation = %label, "waiting for the user's decision");
                            emit(Event::PullDecision(Box::new(decision)));
                            mutated = true;
                            continue;
                        }
                        Some(Err(e)) => Err(e.to_string()),
                        Some(Ok(next)) => {
                            if let (PullNext::Fetched(tip), Op::Pull { checked, .. }) =
                                (next, &mut op)
                            {
                                *checked = tip;
                            }
                            perform(&repo, op, progress).map_err(|e| e.to_string())
                        }
                        None => perform(&repo, op, progress).map_err(|e| e.to_string()),
                    };
                    let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                    match &result {
                        Ok(_) => tracing::info!(operation = %label, ms, "operation succeeded"),
                        Err(e) => {
                            tracing::warn!(operation = %label, ms, error = %canopy_core::cli::redact_url(e), "operation failed");
                        }
                    }
                    let stash_conflict = integrates
                        && result.is_ok()
                        && repo.autostash_conflicted().unwrap_or_else(|e| {
                            tracing::warn!(error = %redacted(&e), "conflicts not checked");
                            false
                        });
                    let stopped = result.is_err()
                        && if applies_stash {
                            !conflicted_before && repo.stash_stopped_on_conflicts().unwrap_or(false)
                        } else if picks {
                            !conflicted_before && repo.stopped_on_conflicts().unwrap_or(false)
                        } else {
                            integrates && repo.stopped_on_conflicts().unwrap_or(false)
                        };
                    // Only a fast-forward was allowed and the branches diverged after all: a
                    // choice to make, not a failure (a pull checked beforehand asks first). A
                    // failed fetch brought nothing to combine: its error is the answer.
                    let refused = pull
                        .filter(|_| result.is_err() && !fetch_failed)
                        .map(|(strategy, follow)| pull_strategy(&repo, strategy, follow))
                        .filter(|&(strategy, _)| strategy == PullStrategy::FastForwardOnly)
                        .and_then(|(_, configured)| {
                            repo.compare_upstream()
                                .ok()
                                .flatten()
                                .filter(canopy_core::PullPreview::diverged)
                                .map(|preview| (preview, configured))
                        });
                    if let Some((preview, configured)) = refused {
                        tracing::info!(operation = %label, "the branches diverged: asking how to combine them");
                        emit(Event::PullDecision(Box::new(PullDecision {
                            preview,
                            strategy: PullStrategy::FastForwardOnly,
                            diverged: true,
                            configured,
                            moved: false,
                        })));
                    } else if stopped {
                        // Not a failure: the conflicts are the user's to resolve.
                        emit(Event::Conflicted {
                            autostashed: !held_before && repo.autostash_held(),
                            stash: applies_stash,
                            label,
                        });
                    } else {
                        // Frequent and quiet, unless it left folders out.
                        let announce =
                            announce || (stage_all && matches!(&result, Ok(m) if !m.is_empty()));
                        emit(Event::OpFinished {
                            label,
                            kind,
                            announce,
                            result,
                        });
                    }
                    if stash_conflict {
                        tracing::info!("putting the autostashed changes back left conflicts");
                        emit(Event::AutostashConflict);
                    }
                    mutated = true;
                }
                Job::Refresh(_)
                | Job::RetryDownloads
                | Job::RefreshStatus
                | Job::WatchFailed(_) => {}
            }
            tracing::debug!(
                job = kind,
                ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                "job done"
            );
        }

        if let Some(n) = want_full.filter(|n| *n > 0) {
            limit = n;
        }
        if want_full.is_some() || mutated {
            let started = Instant::now();
            match snapshot_completing(&repo, limit, &emit, &allowed, &mut status_failure) {
                Ok(s) => {
                    tracing::debug!(
                        commits = s.commits.len(),
                        ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
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
            let remembered = status_failure.is_some();
            let read = |m| {
                // After a failed download only what is on disk is read.
                repo.status(if remembered { MissingObjects::Fail } else { m })
            };
            match read_downloading(&emit, &allowed, read) {
                Ok(s) => emit(Event::Status(s)),
                Err(e) => emit(Event::Failed(e.to_string())),
            }
        }
    }
}

/// Drops the reads a later job of the same batch makes pointless, so a burst of selections does
/// not queue one download per click on the repository's single thread: only the last
/// [`Job::Details`] and [`Job::Blame`] run, and only the last [`Job::Diff`] of each kind of source (a commit diff
/// supersedes earlier commit diffs of any commit; the tab ignores the replies of all but the
/// newest request anyway). Every other job keeps its place and order.
fn coalesce_reads(jobs: Vec<Job>) -> Vec<Job> {
    let mut keep = vec![true; jobs.len()];
    let mut details_seen = false;
    let mut signature_seen = false;
    let mut ignore_seen = false;
    let mut file_history_seen = false;
    let mut change_search_seen = false;
    let mut blame_seen = false;
    let mut lfs_seen = false;
    let mut lfs_locks_seen = false;
    let mut lfs_patterns_seen = false;
    let mut diff_sources = Vec::new();
    for (i, job) in jobs.iter().enumerate().rev() {
        match job {
            Job::Ignore { .. } => keep[i] = !std::mem::replace(&mut ignore_seen, true),
            Job::FileHistory { .. } => {
                keep[i] = !std::mem::replace(&mut file_history_seen, true);
            }
            Job::SearchChanges { .. } => {
                keep[i] = !std::mem::replace(&mut change_search_seen, true);
            }
            Job::Details(_) => keep[i] = !std::mem::replace(&mut details_seen, true),
            Job::Signature(_) => keep[i] = !std::mem::replace(&mut signature_seen, true),
            Job::Blame { .. } => keep[i] = !std::mem::replace(&mut blame_seen, true),
            Job::Lfs { .. } => keep[i] = !std::mem::replace(&mut lfs_seen, true),
            Job::LfsLocks(_) => keep[i] = !std::mem::replace(&mut lfs_locks_seen, true),
            Job::LfsPatterns(_) => keep[i] = !std::mem::replace(&mut lfs_patterns_seen, true),
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

thread_local! {
    /// The version of the trusted list (`trust::changes()`) behind the latest answer a repository
    /// thread was given, which is the one a refusal on that thread reports.
    static TRUST_VERSION: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Whether Canopy trusts `repo` now (it is inside a trusted folder, or trust safety is off),
/// noting which version of the list said so.
fn trusted_now(repo: &Repo) -> bool {
    let trust = crate::trust::snapshot();
    TRUST_VERSION.set(trust.version);
    !trust.safety || repo.is_in_trusted_folder(&trust.folders)
}

/// The most commits of one file's history a read returns; the dialog says when there are more.
const FILE_HISTORY_LIMIT: usize = 500;

/// The most commits one change search lists.
const CHANGE_SEARCH_LIMIT: usize = 300;

/// Why a download did not run in a repository outside every trusted folder.
pub(crate) const NOT_TRUSTED: &str = "Canopy downloads it only for a trusted repository. \
                           Choose Trust… in the status bar to allow it for this one.";

/// How a refusal in a repository that is not trusted begins.
pub(crate) const READ_ONLY: &str = "This repository is not trusted, so it is read-only";

/// The most program sources a refusal names before it counts the rest.
const NAMED_PROGRAMS: usize = 4;

/// Why an action did not run in a repository that is not trusted: every action runs `git`, which
/// follows the repository's own hooks and configuration. `programs` are the ones its own files
/// name ([`Repo::programs`]); the message says which, and says nothing about them when there are
/// none.
fn read_only_message(programs: &[String]) -> String {
    let (unreadable, named): (Vec<&String>, Vec<&String>) = programs
        .iter()
        .partition(|p| p.as_str() == canopy_core::programs::UNREADABLE);
    let mut facts = Vec::new();
    if !named.is_empty() {
        let shown = named
            .iter()
            .take(NAMED_PROGRAMS)
            .map(|p| p.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let more = named.len().saturating_sub(NAMED_PROGRAMS);
        let more = if more > 0 {
            format!(" and {more} more")
        } else {
            String::new()
        };
        facts.push(format!(
            "its own files make git run programs ({shown}{more})"
        ));
    }
    if !unreadable.is_empty() {
        facts.push("its git configuration could not be read".to_owned());
    }
    let because = if facts.is_empty() {
        String::new()
    } else {
        format!(": {}", facts.join("; "))
    };
    format!("{READ_ONLY}{because}. Choose Trust… in the banner above it to allow changes.")
}

/// Reports a download that was not run because the repository is not in a trusted folder, and
/// returns the error the read ends with. `can_download()` is false for it: trying again cannot
/// help until the folder is trusted.
fn download_refused(emit: &impl Fn(Event), remote: Option<String>) -> canopy_core::Error {
    if let Some(remote) = remote {
        tracing::info!(remote = %canopy_core::cli::redact_url(&remote), "download refused: folder not trusted");
        emit(Event::DownloadsRefused {
            remote,
            trust: TRUST_VERSION.get(),
        });
    }
    canopy_core::Error::NotDownloaded {
        remote: None,
        reason: Some(NOT_TRUSTED.into()),
    }
}

/// Core asks the trusted list again right before each `git fetch`, so a download that failed may
/// have been stopped there by trust withdrawn after `allowed` last said yes. Looks again and
/// then reports a refusal like any other, which the status bar's Trust… answers.
fn refused_after<T>(
    emit: &impl Fn(Event),
    allowed: &impl Fn() -> bool,
    remote: Option<String>,
    result: canopy_core::Result<T>,
) -> canopy_core::Result<T> {
    match result {
        Err(_) if !allowed() => Err(download_refused(emit, remote)),
        other => other,
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
                with_progress(emit, Some(remote.clone()), || {
                    // Asked again as late as possible: trust may have been withdrawn meanwhile.
                    if allowed() {
                        let result = read(MissingObjects::Download);
                        refused_after(emit, allowed, Some(remote), result)
                    } else {
                        Err(download_refused(emit, Some(remote)))
                    }
                })
            } else {
                Err(download_refused(emit, Some(remote)))
            }
        }
        other => other,
    }
}

/// A snapshot whose status is filled in even in a partial clone that lacks the tree of `HEAD`:
/// the history comes first and never fails because of it; if downloading the tree fails the
/// snapshot carries the reason ([`RepoSnapshot::status_unavailable`]). A download that failed
/// is kept in `failure` and shown by the snapshots after it without another try, until it is
/// cleared (the user's Retry).
fn snapshot_completing(
    repo: &Repo,
    limit: usize,
    emit: &impl Fn(Event),
    allowed: &impl Fn() -> bool,
    failure: &mut Option<canopy_core::StatusUnavailable>,
) -> canopy_core::Result<RepoSnapshot> {
    let mut snap = repo.snapshot(limit)?;
    if snap.status_unavailable.is_none() {
        *failure = None;
    }
    if snap
        .status_unavailable
        .as_ref()
        .is_some_and(|u| u.can_download)
    {
        if let Some(remembered) = failure.as_ref() {
            snap.status_unavailable = Some(remembered.clone());
            return Ok(snap);
        }
        if !allowed() {
            let error = download_refused(emit, repo.promisor_remote());
            snap.status_unavailable = Some(canopy_core::StatusUnavailable {
                message: error.to_string(),
                can_download: false,
            });
            return Ok(snap);
        }
        with_progress(emit, repo.promisor_remote(), || {
            // Asked again as late as possible: trust may have been withdrawn meanwhile...
            if allowed() {
                repo.complete_status(&mut snap, MissingObjects::Download);
            }
            // ...even after a fetch that core's own check stopped.
            if snap.status_unavailable.is_some() && !allowed() {
                let error = download_refused(emit, repo.promisor_remote());
                snap.status_unavailable = Some(canopy_core::StatusUnavailable {
                    message: error.to_string(),
                    can_download: false,
                });
            }
        });
        if let Some(u) = &snap.status_unavailable {
            tracing::warn!(reason = %canopy_core::cli::redact_url(&u.message), "status not downloaded");
        }
        *failure = snap.status_unavailable.clone().filter(|u| u.can_download);
    }
    Ok(snap)
}

fn perform(repo: &Repo, op: Op, mut progress: impl FnMut(&str)) -> canopy_core::Result<String> {
    // A pull, rebase or checkout moves what the submodules should be at.
    let moves = matches!(
        op,
        Op::Pull { .. }
            | Op::Rebase(_)
            | Op::Checkout(_)
            | Op::CheckoutRequest { .. }
            | Op::CheckoutRemote { .. }
            | Op::CheckoutDetached(_)
            | Op::CreateBranch { checkout: true, .. }
    );
    let continuing = matches!(op, Op::ContinueOrAbort { abort: false, .. });
    let out = perform_op(repo, op, &mut progress)?;
    if moves || (continuing && repo.git().state() == git2::RepositoryState::Clean) {
        repo.follow_submodules(&mut progress)?;
    }
    Ok(out)
}

fn perform_op(repo: &Repo, op: Op, progress: impl FnMut(&str)) -> canopy_core::Result<String> {
    let done = |r: canopy_core::Result<()>| r.map(|()| String::new());
    match op {
        Op::FlowInit(config) => repo.flow_init(&config).map(|()| {
            format!(
                "git-flow is set up: {} and {}",
                config.master, config.develop
            )
        }),
        Op::FlowStart { kind, name, base } => repo
            .flow_start(kind, &name, base.as_deref())
            .map(|branch| format!("Started {branch}")),
        Op::FlowFinish {
            kind,
            name,
            options,
            push,
        } => {
            let mut message = repo.flow_finish(kind, &name, &options)?;
            if let Some(push) = push {
                repo.flow_push(kind, &name, &push.remote, push.delete_remote, progress)
                    .map_err(|e| {
                        canopy_core::Error::Invalid(format!(
                            "{message}, but pushing to {} failed: {e}",
                            push.remote
                        ))
                    })?;
                message = format!("{message} and pushed to {}", push.remote);
            }
            Ok(message)
        }
        Op::CleanMerged {
            bases,
            names,
            allow_unproven,
        } => {
            let removal = repo.remove_merged_branches(&bases, &names, allow_unproven)?;
            if removal.removed.is_empty() && !removal.failed.is_empty() {
                return Err(canopy_core::Error::Invalid(removal.summary()));
            }
            Ok(removal.summary())
        }
        Op::UpdateFromBase {
            base,
            strategy,
            tip,
        } => repo.update_from_base(strategy, tip, progress).map(|out| {
            if out.trim().is_empty() {
                format!("Updated from {base}")
            } else {
                out
            }
        }),
        Op::CleanStale {
            candidates,
            older_than_days,
        } => {
            repo.remove_stale_candidates(
                &candidates,
                older_than_days,
                chrono::Utc::now().timestamp(),
            )?;
            Ok(format!(
                "Removed {} stale worktrees and branches",
                candidates.len()
            ))
        }
        Op::AddIgnore { rule, target } => repo.add_ignore_rule(&rule, target).map(|out| {
            format!(
                "{} {} in {}.",
                if out.added { "Added" } else { "Already saved" },
                out.pattern,
                out.file
                    .strip_prefix(repo.workdir())
                    .unwrap_or(&out.file)
                    .display()
            )
        }),
        Op::SubmoduleUpdate { path, init } => {
            repo.submodule_update(path.as_deref(), init, true, progress)
        }
        Op::SubmoduleSync(path) => repo.submodule_sync(path.as_deref(), true),
        Op::SubmoduleAdd { url, path, branch } => repo
            .submodule_add(&url, &path, branch.as_deref(), progress)
            .map(|out| {
                if out.trim().is_empty() {
                    format!("Added {path}.")
                } else {
                    out
                }
            }),
        Op::SubmoduleRemove(path) => repo
            .submodule_remove(&path)
            .map(|()| format!("Removed {path}; the removal is staged.")),
        Op::LfsPull => repo
            .lfs_pull(progress)
            .map(|_| "Downloaded the LFS files of this checkout.".to_owned()),
        Op::LfsFetch => repo
            .lfs_fetch(progress)
            .map(|_| "Downloaded the LFS objects of this branch.".to_owned()),
        Op::LfsTrack { pattern, literal } => repo.lfs_track(&pattern, literal),
        Op::LfsUntrack(pattern) => repo.lfs_untrack(&pattern),
        Op::LfsLock(path) => repo.lfs_lock(&path),
        Op::LfsUnlock { path, force } => repo.lfs_unlock(&path, force),
        Op::StopTracking(path) => repo
            .stop_tracking(&path)
            .map(|()| format!("Kept {path} locally and staged its removal from Git.")),
        Op::Stage(p) => done(repo.stage(&p)),
        Op::StageAll => repo
            .stage_all_except_repositories()
            .map(|left_out| left_out_message(&left_out)),
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
        Op::AddWorktree {
            folder,
            branch,
            create,
            start,
        } => repo
            .add_worktree(&folder, &branch, create, start.as_deref())
            .map(|()| format!("Added a worktree for {branch} in {}", folder.display())),
        Op::Checkout(b) => done(repo.checkout_branch(&b)),
        Op::CheckoutRemote { remote, branch } => {
            done(repo.checkout_remote_branch(&remote, &branch))
        }
        Op::CheckoutDetached(oid) => done(repo.checkout_detached(oid)),
        Op::RenameBranch { old, new } => done(repo.rename_branch(&old, &new)),
        Op::DeleteBranch { name, force: true } => done(repo.force_delete_branch(&name)),
        Op::DeleteBranch { name, force: false } => done(repo.delete_merged_branch(&name)),
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
        Op::PublishBranch { name, remote } => repo.publish_branch(&name, &remote, progress),
        Op::ExportPatches { oid, count, dir } => {
            let series = repo.patch_series(oid, count)?;
            if series.commits.is_empty() {
                return Err(canopy_core::Error::Invalid(format!(
                    "A merge commit cannot be exported as a patch: {}. Include the commits \
                     before it, or export one of them.",
                    canopy_core::short_id(oid)
                )));
            }
            repo.export_patches(&series.commits, &dir)
                .map(|files| exported_message(&files, &dir, series.merges.len()))
        }
        Op::ApplyPatches { path } => {
            let files = patch_files(&path)?;
            repo.apply_patches(&files)
                .map(|out| applied_message(&out, files.len()))
        }
        Op::StashSave { message, untracked } => {
            done(repo.stash_save(message.as_deref(), untracked))
        }
        Op::StashApply { index, pop } => done(repo.stash_apply(index, pop)),
        Op::StashApplyCommit { oid, pop } => done(repo.stash_apply_commit(oid, pop)),
        Op::StashDrop(oid) => done(repo.stash_drop_commit(oid)),
        Op::Fetch { prune, .. } => repo.fetch(None, prune, progress),
        Op::Pull {
            strategy,
            follow_config,
            checked,
            ..
        } => {
            let (strategy, _) = pull_strategy(repo, strategy, follow_config);
            if checked.is_some() {
                // The preview fetched; what the user saw is what comes in.
                repo.pull_fetched(strategy, checked, progress)
            } else {
                repo.pull(strategy, progress)
            }
        }
        Op::Push { force, expect } => {
            if let Some(asked) = expect.filter(|_| force) {
                repo.ensure_force_push_target(&asked)?;
            }
            repo.push(force, progress)
        }
        Op::AddRemote { name, url } => done(repo.add_remote(&name, &url)),
        Op::RemoveRemote(r) => done(repo.remove_remote(&r)),
        Op::SetRemoteUrl { name, url } => done(repo.set_remote_url(&name, &url)),
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

/// What Stage all says when it left out folders that hold a repository of their own (staging
/// them would record bare gitlinks nobody can clone); empty when it left out nothing.
/// What an export tells: the files written, by name.
fn exported_message(files: &[PathBuf], dir: &Path, merges: usize) -> String {
    let names: Vec<String> = files
        .iter()
        .filter_map(|f| f.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .collect();
    let skipped = match merges {
        0 => String::new(),
        1 => " Skipped 1 merge commit: git writes no patch for a merge.".into(),
        n => format!(" Skipped {n} merge commits: git writes no patch for a merge."),
    };
    match names.as_slice() {
        [only] => format!("Wrote {only} to {}.{skipped}", dir.display()),
        [first, .., last] => format!(
            "Wrote {} patches ({first} to {last}) to {}.{skipped}",
            names.len(),
            dir.display()
        ),
        [] => format!("Wrote no patches to {}.{skipped}", dir.display()),
    }
}

/// The patch files `path` names: itself, or a folder's `.patch` files in name order.
fn patch_files(path: &Path) -> canopy_core::Result<Vec<PathBuf>> {
    if !path.is_absolute() {
        return Err(canopy_core::Error::Invalid(
            "Choose a full path to the patch file or folder.".into(),
        ));
    }
    if !path.is_dir() {
        return Ok(vec![path.to_path_buf()]);
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(path)
        .map_err(|e| canopy_core::Error::Invalid(format!("{}: {e}", path.display())))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "patch"))
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(canopy_core::Error::Invalid(format!(
            "{} has no .patch files.",
            path.display()
        )));
    }
    Ok(files)
}

/// What an apply tells: the commits made, by their subjects as `git am` printed them.
fn applied_message(out: &str, files: usize) -> String {
    let subjects: Vec<&str> = out
        .lines()
        .filter_map(|l| l.strip_prefix("Applying: "))
        .collect();
    match subjects.as_slice() {
        [] => format!(
            "Applied {files} patch file{}.",
            if files == 1 { "" } else { "s" }
        ),
        [one] => format!("Applied {one}."),
        many => format!("Applied {} commits: {}.", many.len(), many.join("; ")),
    }
}

fn left_out_message(folders: &[String]) -> String {
    if folders.is_empty() {
        return String::new();
    }
    let shown: Vec<&str> = folders.iter().take(3).map(String::as_str).collect();
    let more = folders.len() - shown.len();
    let names = if more > 0 {
        format!("{} and {more} more", shown.join(", "))
    } else {
        shown.join(", ")
    };
    format!(
        "Left out {names}: {} a repository of its own. Add {} as a submodule, or ignore {}.",
        if folders.len() == 1 {
            "it is"
        } else {
            "they are"
        },
        if folders.len() == 1 { "it" } else { "them" },
        if folders.len() == 1 { "it" } else { "them" },
    )
}

/// Candidate collection and Git's deciding-rule checks stay on the repository thread.
fn read_ignore(repo: &Repo, query: &IgnoreQuery) -> canopy_core::Result<IgnoreData> {
    match query {
        IgnoreQuery::Paths => repo.ignored_paths(1000).map(IgnoreData::Paths),
        IgnoreQuery::Why(path) => {
            let matched = repo.why_ignored(path)?;
            let needle = path.trim_end_matches('/');
            let prefix = format!("{needle}/");
            let tracked = repo.git().index()?.iter().any(|entry| {
                let path = String::from_utf8_lossy(&entry.path);
                path == needle || path.starts_with(&prefix)
            });
            Ok(IgnoreData::Why { matched, tracked })
        }
        IgnoreQuery::Preview { rule, target } => {
            let index = repo.git().index()?;
            let mut paths = index
                .iter()
                .map(|entry| String::from_utf8_lossy(&entry.path).into_owned())
                .collect::<Vec<_>>();
            let status = repo.status(MissingObjects::Fail)?;
            paths.extend(status.unstaged.into_iter().map(|entry| entry.path));
            paths.extend(status.untracked_dirs.into_iter().map(|dir| dir.path));
            let ignored = repo.ignored_paths(1000)?;
            paths.extend(ignored.paths);
            paths.sort();
            paths.dedup();
            let truncated = ignored.truncated || paths.len() > 1000;
            paths.truncate(1000);
            Ok(IgnoreData::Preview {
                file: repo.ignore_destination(*target, rule.path())?.file,
                preview: repo.preview_ignore_rule(rule, *target, &paths)?,
                truncated,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Job, Op, OpKind, PullStrategy, read_only_message};
    use crate::watcher::watch;
    use std::path::Path;
    use std::process::Command;
    use std::time::{Duration, Instant};

    /// The refusal says what is true: the program sources it found, that the configuration could
    /// not be read, or only that the repository is not trusted.
    #[test]
    fn the_read_only_refusal_names_only_what_was_found() {
        let none = read_only_message(&[]);
        assert_eq!(
            none,
            "This repository is not trusted, so it is read-only. \
             Choose Trust… in the banner above it to allow changes."
        );
        let found = read_only_message(&["hook pre-commit".into(), "core.sshcommand".into()]);
        assert!(
            found.starts_with(
                "This repository is not trusted, so it is read-only: its own files make git run \
                 programs (hook pre-commit, core.sshcommand)."
            ),
            "{found}"
        );
        let unreadable = read_only_message(&[canopy_core::programs::UNREADABLE.into()]);
        assert!(
            unreadable.contains("its git configuration could not be read"),
            "{unreadable}"
        );
        assert!(!unreadable.contains("make git run"), "{unreadable}");
        let both = read_only_message(&[
            "hook pre-commit".into(),
            canopy_core::programs::UNREADABLE.into(),
        ]);
        assert!(
            both.contains("(hook pre-commit); its git configuration"),
            "{both}"
        );
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "tests drive git directly to build and check repositories"
    )]
    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .current_dir(dir)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "git {args:?}");
    }

    /// A tab nobody has shown has no repository thread and no file watcher: a job sent to its
    /// worker goes nowhere, and starting it runs the thread, which then answers once.
    #[test]
    fn a_lazy_worker_runs_nothing_until_it_starts() {
        let dir = tempfile::tempdir().expect("tempdir");
        git(dir.path(), &["init", "-q", "-b", "main"]);
        std::fs::write(dir.path().join("a.txt"), "a\n").expect("write");
        git(dir.path(), &["add", "."]);
        git(
            dir.path(),
            &[
                "-c",
                "user.name=Ada",
                "-c",
                "user.email=ada@example.com",
                "commit",
                "-q",
                "-m",
                "first",
            ],
        );
        let ctx = egui::Context::default();
        let mut worker = super::Worker::lazy(dir.path().to_path_buf(), &ctx);
        assert!(!worker.is_started());
        worker.send(Job::Refresh(10));
        std::fs::write(dir.path().join("b.txt"), "b\n").expect("write");
        assert!(
            worker
                .events
                .recv_timeout(Duration::from_millis(400))
                .is_err(),
            "an unstarted worker answers nothing, and a file change reaches no watcher"
        );

        assert!(worker.start(), "the first start starts it");
        assert!(!worker.start(), "a second start does nothing");
        assert!(worker.is_started());
        worker.send(Job::Refresh(10));
        let end = Instant::now() + Duration::from_secs(60);
        let mut snapshots = 0;
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            match worker.events.recv_timeout(left.min(Duration::from_secs(2))) {
                Ok(super::Event::Snapshot(_)) => snapshots += 1,
                Ok(_) => {}
                Err(_) => break,
            }
        }
        assert!(snapshots >= 1, "the started worker sent its snapshot");
    }

    /// Closing a tab that never loaded ends nothing it never started, and does not wait for it.
    #[test]
    fn dropping_a_lazy_worker_does_not_wait() {
        let worker = super::Worker::lazy(
            std::path::PathBuf::from("/does/not/exist"),
            &egui::Context::default(),
        );
        let started = Instant::now();
        drop(worker);
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    /// Standard output of `git` in `dir`, without its final newline.
    #[expect(
        clippy::disallowed_methods,
        reason = "tests drive git directly to build and check repositories"
    )]
    fn git_out(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .current_dir(dir)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
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
    fn a_linked_worktree_reports_its_shared_git_folder_to_the_trust_dialog()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let main = dir.path().join("main");
        std::fs::create_dir(&main)?;
        git(&main, &["init", "-q", "-b", "main"]);
        git(&main, &["config", "user.name", "Ada"]);
        git(&main, &["config", "user.email", "ada@example.com"]);
        git(&main, &["commit", "-q", "--allow-empty", "-m", "first"]);
        let linked = dir.path().join("linked");
        git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "topic",
                &linked.to_string_lossy(),
            ],
        );
        let worker = super::Worker::spawn(linked.clone(), &egui::Context::default());
        worker.send(Job::Refresh(100));
        let deadline = Instant::now() + Duration::from_secs(5);
        let folders = loop {
            if let super::Event::TrustFolders(folders) = worker
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))?
            {
                break folders;
            }
        };
        assert_eq!(
            folders,
            [linked.canonicalize()?, main.join(".git").canonicalize()?]
        );
        Ok(())
    }

    /// The next finished operation's label and result, and nothing before it that matters.
    fn next_finished(
        worker: &super::Worker,
    ) -> Result<(String, Result<String, String>), Box<dyn std::error::Error>> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let event = worker
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))?;
            if let super::Event::OpFinished { label, result, .. } = event {
                return Ok((label, result));
            }
        }
    }

    /// git-flow through the repository thread: set up, start, finish with a push, and the
    /// snapshot that follows knows git-flow is set up and the branch is gone.
    #[test]
    fn git_flow_runs_through_the_worker_from_set_up_to_the_push()
    -> Result<(), Box<dyn std::error::Error>> {
        use canopy_core::{FlowConfig, FlowFinish, FlowKind};
        let dir = tempfile::tempdir()?;
        let main = dir.path().join("main");
        let remote = dir.path().join("remote.git");
        std::fs::create_dir(&main)?;
        std::fs::create_dir(&remote)?;
        git(&remote, &["init", "-q", "--bare", "-b", "main"]);
        git(&main, &["init", "-q", "-b", "main"]);
        for (key, value) in [
            ("user.name", "Ada"),
            ("user.email", "ada@example.com"),
            ("commit.gpgsign", "false"),
            ("core.autocrlf", "false"),
        ] {
            git(&main, &["config", key, value]);
        }
        git(&main, &["commit", "-q", "--allow-empty", "-m", "first"]);
        git(
            &main,
            &["remote", "add", "origin", &remote.to_string_lossy()],
        );
        git(&main, &["push", "-q", "-u", "origin", "main"]);

        crate::trust::add_for_tests(dir.path().to_path_buf());
        let worker = super::Worker::spawn(main.clone(), &egui::Context::default());
        let config = FlowConfig {
            master: "main".into(),
            ..FlowConfig::default()
        };
        worker.send(Job::Op(Op::FlowInit(config)));
        let (_, result) = next_finished(&worker)?;
        assert_eq!(
            result,
            Ok("git-flow is set up: main and develop".to_owned())
        );

        worker.send(Job::Op(Op::FlowStart {
            kind: FlowKind::Feature,
            name: "login".into(),
            base: None,
        }));
        let (_, result) = next_finished(&worker)?;
        assert_eq!(result, Ok("Started feature/login".to_owned()));
        std::fs::write(main.join("login.txt"), "x\n")?;
        git(&main, &["add", "login.txt"]);
        git(&main, &["commit", "-q", "-m", "feat: login"]);

        // A start that git-flow refuses comes back as its own sentence.
        worker.send(Job::Op(Op::FlowStart {
            kind: FlowKind::Feature,
            name: "login".into(),
            base: None,
        }));
        let (_, result) = next_finished(&worker)?;
        assert_eq!(
            result,
            Err("Branch 'feature/login' already exists.".to_owned())
        );

        worker.send(Job::Op(Op::FlowFinish {
            kind: FlowKind::Feature,
            name: "login".into(),
            options: FlowFinish::default(),
            push: Some(super::FlowPush {
                remote: "origin".into(),
                delete_remote: false,
            }),
        }));
        let (_, result) = next_finished(&worker)?;
        assert_eq!(
            result,
            Ok("Merged feature/login into develop and pushed to origin".to_owned())
        );
        assert_eq!(
            git_out(&main, &["rev-parse", "develop"]),
            git_out(&remote, &["rev-parse", "develop"]),
            "develop reached the remote"
        );
        assert_eq!(git_out(&main, &["branch", "--list", "feature/login"]), "");
        assert_eq!(
            git_out(&main, &["symbolic-ref", "--short", "HEAD"]),
            "develop"
        );

        // The snapshot after the operation says git-flow is set up.
        let deadline = Instant::now() + Duration::from_secs(30);
        let snapshot = loop {
            if let super::Event::Snapshot(s) = worker
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))?
                && s.head.branch.as_deref() == Some("develop")
            {
                break s;
            }
        };
        assert_eq!(
            snapshot.workflow.flow.as_ref().map(|f| f.develop.as_str()),
            Some("develop")
        );
        Ok(())
    }

    /// Cleaning up merged branches and updating from the base through the repository thread.
    #[test]
    fn merged_branches_and_the_base_update_run_through_the_worker()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let main = dir.path().join("main");
        std::fs::create_dir(&main)?;
        git(&main, &["init", "-q", "-b", "main"]);
        for (key, value) in [
            ("user.name", "Ada"),
            ("user.email", "ada@example.com"),
            ("commit.gpgsign", "false"),
            ("core.autocrlf", "false"),
        ] {
            git(&main, &["config", key, value]);
        }
        git(&main, &["commit", "-q", "--allow-empty", "-m", "first"]);
        git(&main, &["switch", "-q", "-c", "topic"]);
        std::fs::write(main.join("topic.txt"), "topic\n")?;
        git(&main, &["add", "topic.txt"]);
        git(&main, &["commit", "-q", "-m", "feat: topic"]);
        git(&main, &["switch", "-q", "main"]);
        git(&main, &["merge", "-q", "--no-ff", "--no-edit", "topic"]);
        git(&main, &["switch", "-q", "-c", "work", "main~1"]);
        std::fs::write(main.join("mine.txt"), "mine\n")?;
        git(&main, &["add", "mine.txt"]);
        git(&main, &["commit", "-q", "-m", "feat: mine"]);

        crate::trust::add_for_tests(dir.path().to_path_buf());
        let worker = super::Worker::spawn(main.clone(), &egui::Context::default());
        worker.send(Job::MergedBranches {
            serial: 1,
            bases: vec!["main".into()],
            squash: true,
        });
        let deadline = Instant::now() + Duration::from_secs(30);
        let rows = loop {
            if let super::Event::MergedBranches {
                serial: 1, result, ..
            } = worker
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))?
            {
                break result?;
            }
        };
        let reasons: Vec<(&str, bool)> = rows
            .iter()
            .map(|b| (b.name.as_str(), b.reason.is_some()))
            .collect();
        assert_eq!(reasons, [("topic", true), ("work", false)]);

        worker.send(Job::BaseCompare {
            serial: 1,
            base: "main".into(),
        });
        let preview = loop {
            if let super::Event::BaseCompare { serial: 1, result } = worker
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))?
            {
                break result?;
            }
        };
        assert_eq!((preview.ahead, preview.behind), (1, 2));

        worker.send(Job::Op(Op::UpdateFromBase {
            base: "main".into(),
            strategy: PullStrategy::Rebase,
            tip: preview.tip.ok_or("no tip")?,
        }));
        let (_, result) = next_finished(&worker)?;
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(git_out(&main, &["rev-list", "--count", "main..work"]), "1");
        assert_eq!(git_out(&main, &["rev-list", "--count", "work..main"]), "0");

        worker.send(Job::Op(Op::CleanMerged {
            bases: vec!["main".into()],
            names: vec!["topic".into()],
            allow_unproven: false,
        }));
        let (_, result) = next_finished(&worker)?;
        assert_eq!(result, Ok("Deleted 1 branch".to_owned()));
        assert_eq!(git_out(&main, &["branch", "--list", "topic"]), "");
        Ok(())
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
        let (ended, _ended) = crossbeam_channel::bounded::<()>(0);
        watch(linked.clone(), tx, stop, ended);
        // A commit in the linked worktree changes its HEAD's branch, whose ref lives in the
        // common folder, and the worktree's own index under .git/worktrees/linked. Registering
        // happens on the shared watcher thread, so commit again until one is seen.
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut seen = false;
        while !seen && Instant::now() < deadline {
            git(&linked, &["commit", "-q", "--allow-empty", "-m", "again"]);
            seen = wait_for(
                &rx,
                |j| matches!(j, Job::Refresh(_)),
                Duration::from_secs(1),
            );
        }
        assert!(seen, "a commit in the linked worktree refreshes it");
        Ok(())
    }

    #[test]
    fn a_watch_failure_reaches_the_tab() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        git(dir.path(), &["init", "-q"]);
        let worker = super::Worker::spawn(dir.path().to_path_buf(), &egui::Context::default());
        worker.send(Job::WatchFailed("limit reached".into()));
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let super::Event::WatchFailed(error) = worker
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))?
            {
                assert_eq!(error, "limit reached");
                return Ok(());
            }
        }
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
            OpKind::Merge
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
    #[test]
    fn stage_all_names_the_repositories_it_left_out() {
        assert_eq!(left_out_message(&[]), "");
        assert_eq!(
            left_out_message(&["a/".into()]),
            "Left out a/: it is a repository of its own. Add it as a submodule, or ignore it."
        );
        let many: Vec<String> = ["a/", "b/", "c/", "d/"].map(String::from).to_vec();
        assert!(left_out_message(&many).starts_with("Left out a/, b/, c/ and 1 more: they are"));
    }

    use std::cell::{Cell, RefCell};

    use canopy_core::Error;

    use super::*;
    use crate::repo_config::in_isolated_git_env;

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
                Job::Signature(o) => format!("signature {}", canopy_core::short_id(*o)),
                Job::Diff(k) => format!("diff {:?} {}", k.source, k.path),
                Job::Op(op) => format!("op {}", op.label()),
                other => other.kind().to_owned(),
            })
            .collect()
    }

    #[test]
    fn only_the_last_queued_file_history_runs() {
        let history = |serial| Job::FileHistory {
            serial,
            from: None,
            path: "a.txt".into(),
        };
        let kept = coalesce_reads(vec![history(1), Job::HeadMessage, history(2)]);
        assert!(
            matches!(
                kept.as_slice(),
                [Job::HeadMessage, Job::FileHistory { serial: 2, .. }]
            ),
            "{kept:?}"
        );
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
    fn only_the_last_blame_of_a_burst_runs() {
        let blame = |serial, path: &str| Job::Blame {
            serial,
            path: path.into(),
            rev: None,
        };
        let kept = coalesce_reads(vec![blame(1, "a"), Job::HeadMessage, blame(2, "b")]);
        assert!(matches!(
            kept.as_slice(),
            [Job::HeadMessage, Job::Blame { serial: 2, .. }]
        ));
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
    #[expect(
        clippy::disallowed_methods,
        reason = "tests drive git directly to build and check repositories"
    )]
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

    #[test]
    fn continuing_a_conflicted_pull_follows_the_incoming_submodule_pointer() {
        for (strategy, state, recurse) in [
            (PullStrategy::Rebase, RepoState::Rebase, true),
            (PullStrategy::Merge, RepoState::Merge, true),
            (PullStrategy::Rebase, RepoState::Rebase, false),
        ] {
            let root = tempfile::tempdir().expect("tempdir");
            std::fs::create_dir(root.path().join("home")).expect("home");
            std::fs::write(root.path().join("home/gitconfig"), "").expect("gitconfig");
            let library = root.path().join("library");
            let upstream = root.path().join("upstream");
            let copy = root.path().join("copy");
            for path in [&library, &upstream] {
                std::fs::create_dir(path).expect("repo folder");
                git(path, &["init", "-q", "-b", "main"]);
                git(path, &["config", "commit.gpgsign", "false"]);
                git(path, &["config", "core.autocrlf", "false"]);
                std::fs::write(path.join("a.txt"), "base\n").expect("file");
                git(path, &["add", "a.txt"]);
                git(path, &["commit", "-q", "-m", "initial"]);
            }
            let library_url = library.to_string_lossy();
            git(
                &upstream,
                &[
                    "-c",
                    "protocol.file.allow=always",
                    "submodule",
                    "add",
                    "--",
                    &library_url,
                    "deps/lib",
                ],
            );
            git(&upstream, &["commit", "-q", "-m", "add library"]);
            git(
                root.path(),
                &[
                    "-c",
                    "protocol.file.allow=always",
                    "clone",
                    "-q",
                    "--recurse-submodules",
                    &upstream.to_string_lossy(),
                    &copy.to_string_lossy(),
                ],
            );
            for (key, value) in [
                ("user.name", "Ada Lovelace"),
                ("user.email", "ada@example.com"),
                ("commit.gpgsign", "false"),
                ("core.autocrlf", "false"),
                ("core.hooksPath", "no-test-hooks"),
            ] {
                git(&copy, &["config", key, value]);
            }
            let old = Oid::from_str(&git(&library, &["rev-parse", "HEAD"])).expect("old");
            std::fs::write(library.join("a.txt"), "new library\n").expect("file");
            git(&library, &["commit", "-q", "-am", "advance library"]);
            let new = Oid::from_str(&git(&library, &["rev-parse", "HEAD"])).expect("new");
            for parent in [&upstream, &copy] {
                git(&parent.join("deps/lib"), &["fetch", "-q", "origin"]);
            }
            git(
                &upstream.join("deps/lib"),
                &["checkout", "-q", "--detach", &new.to_string()],
            );
            std::fs::write(upstream.join("a.txt"), "upstream\n").expect("file");
            git(&upstream, &["add", "a.txt", "deps/lib"]);
            git(&upstream, &["commit", "-q", "-m", "advance upstream"]);
            std::fs::write(copy.join("a.txt"), "local\n").expect("file");
            git(&copy, &["commit", "-q", "-am", "local conflict"]);
            let repo = Repo::open(&copy).expect("open");
            repo.set_options(canopy_core::GitOptions {
                follow_config: false,
                recurse_submodules: recurse,
                autostash: false,
                ..Default::default()
            });
            perform(
                &repo,
                Op::Pull {
                    strategy,
                    follow_config: false,
                    prune: false,
                    checked: None,
                },
                |_| {},
            )
            .expect_err("pull stops on conflict");
            assert!(repo.stopped_on_conflicts().expect("conflict state"));
            assert_eq!(
                repo.submodules(false).expect("submodules")[0].workdir_id,
                Some(old)
            );
            std::fs::write(copy.join("a.txt"), "resolved\n").expect("resolution");
            repo.stage(&["a.txt".into()]).expect("stage resolution");
            perform(
                &repo,
                Op::ContinueOrAbort {
                    state,
                    abort: false,
                },
                |_| {},
            )
            .expect("continue");
            let submodule = &repo.submodules(false).expect("submodules")[0];
            assert_eq!(submodule.head_id, Some(new), "{strategy:?}");
            assert_eq!(submodule.index_id, Some(new), "{strategy:?}");
            assert_eq!(
                submodule.workdir_id,
                Some(if recurse { new } else { old }),
                "{strategy:?}, recurse={recurse}"
            );
            if recurse {
                repo.stage_all_except_repositories().expect("stage all");
                assert_eq!(
                    repo.submodules(false).expect("submodules")[0].index_id,
                    Some(new),
                    "Stage all preserves the incoming pointer"
                );
            }
        }
    }

    /// A direct rebase gets the same explicit submodule follow-up as a continued pull, without
    /// letting the sequencer discard an edit inside the submodule.
    #[test]
    fn a_direct_rebase_follows_the_new_submodule_pointer_and_keeps_edits() {
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(root.path().join("home")).expect("home");
        std::fs::write(root.path().join("home/gitconfig"), "").expect("gitconfig");
        let library = root.path().join("library");
        let sup = root.path().join("super");
        for path in [&library, &sup] {
            std::fs::create_dir(path).expect("repo folder");
            git(path, &["init", "-q", "-b", "main"]);
            git(path, &["config", "commit.gpgsign", "false"]);
            git(path, &["config", "core.autocrlf", "false"]);
            std::fs::write(path.join("a.txt"), "base\n").expect("file");
            git(path, &["add", "a.txt"]);
            git(path, &["commit", "-q", "-m", "initial"]);
        }
        let library_url = library.to_string_lossy();
        git(
            &sup,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "--",
                &library_url,
                "deps/lib",
            ],
        );
        git(&sup, &["commit", "-q", "-m", "add library"]);
        git(&sup, &["switch", "-q", "-c", "topic"]);
        git(&sup, &["switch", "-q", "main"]);
        std::fs::write(library.join("b.txt"), "new library\n").expect("file");
        git(&library, &["add", "b.txt"]);
        git(&library, &["commit", "-q", "-m", "advance library"]);
        let new = Oid::from_str(&git(&library, &["rev-parse", "HEAD"])).expect("new");
        let sub = sup.join("deps/lib");
        git(&sub, &["fetch", "-q", "origin"]);
        git(&sub, &["checkout", "-q", "--detach", &new.to_string()]);
        git(&sup, &["add", "deps/lib"]);
        git(&sup, &["commit", "-q", "-m", "advance library pointer"]);
        git(&sup, &["switch", "-q", "topic"]);
        std::fs::write(sup.join("topic.txt"), "topic\n").expect("file");
        git(&sup, &["add", "topic.txt"]);
        git(&sup, &["commit", "-q", "-m", "topic"]);
        git(&sub, &["checkout", "-q", "HEAD~1"]);
        std::fs::write(sub.join("a.txt"), "edited\n").expect("edit");
        let repo = Repo::open(&sup).expect("open");
        repo.set_options(canopy_core::GitOptions {
            follow_config: false,
            recurse_submodules: true,
            autostash: false,
            ..Default::default()
        });

        perform(&repo, Op::Rebase("main".into()), |_| {}).expect("rebase");

        assert_eq!(
            repo.submodules(false).expect("submodules")[0].workdir_id,
            Some(new)
        );
        assert_eq!(
            std::fs::read_to_string(sub.join("a.txt")).expect("read edit"),
            "edited\n"
        );
    }

    /// A pull asks first when only a fast-forward is allowed and the branches diverged, or when
    /// it would conflict; a clean one runs without asking.
    #[test]
    fn a_pull_asks_before_it_diverges_or_conflicts() {
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(root.path().join("home")).expect("home");
        std::fs::write(root.path().join("home").join("gitconfig"), "").expect("config");
        let up = root.path().join("up");
        std::fs::create_dir(&up).expect("up");
        git(&up, &["init", "-q", "-b", "main"]);
        std::fs::write(up.join("a.txt"), "base\n").expect("write");
        git(&up, &["add", "."]);
        git(&up, &["commit", "-q", "-m", "base"]);
        let copy = root.path().join("copy");
        git(root.path(), &["clone", "-q", "up", "copy"]);
        std::fs::write(up.join("up.txt"), "upstream\n").expect("write");
        git(&up, &["add", "."]);
        git(&up, &["commit", "-q", "-m", "upstream"]);
        std::fs::write(copy.join("local.txt"), "local\n").expect("write");
        git(&copy, &["add", "."]);
        git(&copy, &["commit", "-q", "-m", "local"]);
        let repo = Repo::open(&copy).expect("open");
        let decide =
            |strategy| decide_pull(&repo, strategy, false, false, |_| {}).expect("checked");
        let asks = |next: PullNext| match next {
            PullNext::Ask(d) => d,
            other => panic!("no question: {other:?}"),
        };
        let d = asks(decide(PullStrategy::FastForwardOnly));
        assert!(d.diverged && !d.preview.risky(), "{d:?}");
        assert!(
            matches!(decide(PullStrategy::Rebase), PullNext::Fetched(_)),
            "clean: no question, and nothing left to fetch"
        );
        // Both sides change the same line now.
        std::fs::write(up.join("a.txt"), "theirs\n").expect("write");
        git(&up, &["commit", "-q", "-am", "theirs"]);
        std::fs::write(copy.join("a.txt"), "ours\n").expect("write");
        git(&copy, &["commit", "-q", "-am", "ours"]);
        let d = asks(decide(PullStrategy::Rebase));
        assert!(!d.diverged, "a rebase knows how to combine them");
        assert_eq!(d.preview.conflicts, Some(vec!["a.txt".to_owned()]));
        assert_eq!(d.strategy, PullStrategy::Rebase);
        // Without an upstream there is nothing to preview: `git pull` says why.
        git(&copy, &["branch", "--unset-upstream"]);
        assert!(matches!(decide(PullStrategy::Rebase), PullNext::Pull));
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

    /// A clone whose `main` and fetched `origin/main` diverged by one commit each, in `root`.
    fn diverged_clone(root: &Path) -> PathBuf {
        std::fs::create_dir_all(root.join("home")).expect("home");
        std::fs::write(root.join("home").join("gitconfig"), "").expect("config");
        let up = root.join("up");
        std::fs::create_dir(&up).expect("up");
        git(&up, &["init", "-q", "-b", "main"]);
        std::fs::write(up.join("a.txt"), "base\n").expect("write");
        git(&up, &["add", "."]);
        git(&up, &["commit", "-q", "-m", "base"]);
        let copy = root.join("copy");
        git(root, &["clone", "-q", "up", "copy"]);
        std::fs::write(up.join("up.txt"), "upstream\n").expect("write");
        git(&up, &["add", "."]);
        git(&up, &["commit", "-q", "-m", "upstream"]);
        std::fs::write(copy.join("local.txt"), "local\n").expect("write");
        git(&copy, &["add", "."]);
        git(&copy, &["commit", "-q", "-m", "local"]);
        git(&copy, &["fetch", "-q"]);
        copy
    }

    /// `folder`, trusted for the rest of the test run: an untrusted repository refuses every
    /// operation.
    fn trusted(folder: PathBuf) -> PathBuf {
        crate::trust::add_for_tests(folder.clone());
        folder
    }

    /// A worker on `repo` making Canopy's own git choices, whatever the developer's git
    /// configuration sets.
    fn worker_with_canopys_choices(repo: PathBuf) -> Worker {
        let worker = Worker::spawn(repo, &egui::Context::default());
        worker.send(Job::Options(canopy_core::GitOptions {
            follow_config: false,
            ..canopy_core::GitOptions::default()
        }));
        worker
    }

    /// A divergence says whether the git configuration asked for fast-forwards only, so the
    /// prompt offers to remember an answer for every repository only when Canopy's setting did.
    #[test]
    fn a_divergence_says_who_asked_for_fast_forwards_only() {
        if !in_isolated_git_env(
            "worker::read_tests::a_divergence_says_who_asked_for_fast_forwards_only",
        ) {
            return;
        }
        for (configured, follow, expected) in [
            (false, true, false),
            (true, true, true),
            (true, false, false),
        ] {
            let root = tempfile::tempdir().expect("tempdir");
            let copy = diverged_clone(root.path());
            if configured {
                git(&copy, &["config", "pull.ff", "only"]);
            }
            let worker = worker_with_canopys_choices(trusted(copy));
            worker.send(Job::Op(Op::Pull {
                strategy: PullStrategy::FastForwardOnly,
                follow_config: follow,
                prune: false,
                checked: None,
            }));
            let events = events_until(&worker, |e| {
                matches!(e, Event::PullDecision(_) | Event::OpFinished { .. })
            });
            let Some(Event::PullDecision(d)) = events.last() else {
                panic!("{events:?}");
            };
            assert!(d.diverged);
            assert_eq!(
                d.configured, expected,
                "configured {configured}, follow {follow}"
            );
        }
    }

    /// Dropping a worker ends its repository thread and its file watcher's: nothing of it is
    /// left running when the process exits.
    #[test]
    fn dropping_a_worker_ends_its_threads() {
        let root = tempfile::tempdir().expect("tempdir");
        let worker = worker_with_canopys_choices(trusted(diverged_clone(root.path())));
        let ended = worker.ended.clone();
        drop(worker);
        assert_eq!(
            ended.try_recv(),
            Err(crossbeam_channel::TryRecvError::Disconnected)
        );
    }

    /// The app's exit waits for the threads of the workers dropped before it, within one bound.
    #[test]
    fn exiting_waits_for_the_threads_of_dropped_workers() {
        let (ended_tx, ended) = crossbeam_channel::bounded::<()>(0);
        let (start_tx, start_rx) = crossbeam_channel::bounded::<()>(0);
        let thread = std::thread::spawn(move || {
            // The interval only starts after the test has started measuring.
            start_rx.recv().expect("start shutdown worker");
            std::thread::sleep(Duration::from_millis(200));
            drop(ended_tx);
        });
        ENDING.lock().expect("lock").push(ended);
        let started = Instant::now();
        start_tx.send(()).expect("start worker");
        wait_for_threads(Duration::from_secs(5));
        let waited = started.elapsed();
        assert!(
            waited >= Duration::from_millis(200) && waited < Duration::from_secs(5),
            "waited {waited:?}"
        );
        thread.join().expect("thread");
    }

    /// When the pull's fetch fails, git's error is the answer, even where the branches as last
    /// fetched diverged: there is nothing new to combine.
    #[test]
    fn a_pull_whose_fetch_fails_reports_git_error() {
        let root = tempfile::tempdir().expect("tempdir");
        let copy = diverged_clone(root.path());
        let gone = root.path().join("gone");
        git(
            &copy,
            &["remote", "set-url", "origin", &gone.to_string_lossy()],
        );
        let worker = worker_with_canopys_choices(trusted(copy));
        worker.send(Job::Op(Op::Pull {
            strategy: PullStrategy::FastForwardOnly,
            follow_config: false,
            prune: false,
            checked: None,
        }));
        let events = events_until(&worker, |e| {
            matches!(e, Event::PullDecision(_) | Event::OpFinished { .. })
        });
        let Some(Event::OpFinished {
            result: Err(error), ..
        }) = events.last()
        else {
            panic!("{events:?}");
        };
        assert!(error.contains("gone"), "git's own message: {error}");
    }

    /// A fast-forward-only pull that git refuses because the branches diverged after the check
    /// (they moved in between, as a Pull anyway can meet) still asks how to combine them,
    /// whatever language git speaks.
    #[test]
    fn a_refused_fast_forward_asks_how_to_combine_the_branches() {
        let root = tempfile::tempdir().expect("tempdir");
        let copy = diverged_clone(root.path());
        let seen = Oid::from_str(&git(&copy, &["rev-parse", "origin/main"])).expect("oid");
        let worker = worker_with_canopys_choices(trusted(copy));
        worker.send(Job::Op(Op::Pull {
            strategy: PullStrategy::FastForwardOnly,
            follow_config: false,
            prune: false,
            checked: Some(seen),
        }));
        let events = events_until(&worker, |e| {
            matches!(e, Event::PullDecision(_) | Event::OpFinished { .. })
        });
        let Some(Event::PullDecision(d)) = events.last() else {
            panic!("{events:?}");
        };
        assert!(d.diverged, "{d:?}");
        assert_eq!((d.preview.ahead, d.preview.behind), (1, 1));
    }

    /// The upstream moves while the user decides (an auto-fetch lands): the checked pull must
    /// not bring in the commits they were not shown, but ask about the new state.
    #[test]
    fn a_checked_pull_asks_again_when_the_upstream_moved_since_the_preview() {
        let root = tempfile::tempdir().expect("tempdir");
        let copy = diverged_clone(root.path());
        let up = root.path().join("up");
        let worker = worker_with_canopys_choices(trusted(copy.clone()));
        worker.send(Job::Op(Op::Pull {
            strategy: PullStrategy::FastForwardOnly,
            follow_config: false,
            prune: false,
            checked: None,
        }));
        let events = events_until(&worker, |e| {
            matches!(e, Event::PullDecision(_) | Event::OpFinished { .. })
        });
        let Some(Event::PullDecision(shown)) = events.last() else {
            panic!("{events:?}");
        };
        assert!(!shown.moved);
        let seen = shown.preview.tip.expect("the preview names its commit");

        std::fs::write(up.join("later.txt"), "later\n").expect("write");
        git(&up, &["add", "."]);
        git(&up, &["commit", "-q", "-m", "later"]);
        git(&copy, &["fetch", "-q"]);
        let before = git(&copy, &["rev-parse", "HEAD"]);

        let pull = |checked| {
            worker.send(Job::Op(Op::Pull {
                strategy: PullStrategy::Merge,
                follow_config: false,
                prune: false,
                checked,
            }));
            events_until(&worker, |e| {
                matches!(e, Event::PullDecision(_) | Event::OpFinished { .. })
            })
        };
        let events = pull(Some(seen));
        let Some(Event::PullDecision(again)) = events.last() else {
            panic!("a moved upstream must ask again: {events:?}");
        };
        assert!(again.moved && !again.diverged, "{again:?}");
        assert_eq!(
            again.preview.behind, 2,
            "the new state is what it asks about"
        );
        assert_ne!(again.preview.tip, Some(seen));
        assert_eq!(
            git(&copy, &["rev-parse", "HEAD"]),
            before,
            "nothing was pulled"
        );
        assert!(!copy.join("later.txt").exists());

        // Answered for what it shows now, the pull goes ahead.
        let events = pull(again.preview.tip);
        assert!(
            matches!(events.last(), Some(Event::OpFinished { result: Ok(_), .. })),
            "{events:?}"
        );
        assert!(copy.join("later.txt").exists());
    }

    /// The upstream moved while the user decided, but to a commit the branch already has (it was
    /// rewound): nothing would come in, so there is nothing to ask and the pull just finishes.
    #[test]
    fn a_moved_upstream_with_nothing_new_does_not_ask() {
        let root = tempfile::tempdir().expect("tempdir");
        let copy = diverged_clone(root.path());
        let up = root.path().join("up");
        let seen = Oid::from_str(&git(&copy, &["rev-parse", "origin/main"])).expect("oid");
        git(&up, &["reset", "-q", "--hard", "HEAD~1"]);
        git(&copy, &["fetch", "-q"]);
        let before = git(&copy, &["rev-parse", "HEAD"]);
        let worker = worker_with_canopys_choices(trusted(copy.clone()));
        worker.send(Job::Op(Op::Pull {
            strategy: PullStrategy::Merge,
            follow_config: false,
            prune: false,
            checked: Some(seen),
        }));
        let events = events_until(&worker, |e| {
            matches!(e, Event::PullDecision(_) | Event::OpFinished { .. })
        });
        assert!(
            matches!(events.last(), Some(Event::OpFinished { result: Ok(_), .. })),
            "nothing new, so nothing to ask: {events:?}"
        );
        assert_eq!(git(&copy, &["rev-parse", "HEAD"]), before);
    }

    /// Where the upstream is still the commit the user was shown, the checked pull just goes.
    #[test]
    fn a_checked_pull_goes_ahead_when_the_upstream_is_unchanged() {
        let root = tempfile::tempdir().expect("tempdir");
        let copy = diverged_clone(root.path());
        let seen = Oid::from_str(&git(&copy, &["rev-parse", "origin/main"])).expect("oid");
        let worker = worker_with_canopys_choices(trusted(copy.clone()));
        worker.send(Job::Op(Op::Pull {
            strategy: PullStrategy::Merge,
            follow_config: false,
            prune: false,
            checked: Some(seen),
        }));
        let events = events_until(&worker, |e| {
            matches!(e, Event::PullDecision(_) | Event::OpFinished { .. })
        });
        assert!(
            matches!(events.last(), Some(Event::OpFinished { result: Ok(_), .. })),
            "{events:?}"
        );
        assert!(copy.join("up.txt").exists());
    }

    /// The force push the user confirmed is the one the question named: where the checked-out
    /// branch changed after the question, nothing is pushed.
    #[test]
    fn a_force_push_is_refused_when_the_branch_changed_since_the_question() {
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(root.path().join("home")).expect("home");
        std::fs::write(root.path().join("home").join("gitconfig"), "").expect("config");
        let src = root.path().join("src");
        std::fs::create_dir(&src).expect("src");
        git(&src, &["init", "-q", "-b", "main"]);
        git(&src, &["commit", "-q", "--allow-empty", "-m", "base"]);
        git(root.path(), &["clone", "-q", "--bare", "src", "up.git"]);
        git(root.path(), &["clone", "-q", "up.git", "copy"]);
        let copy = root.path().join("copy");
        let up = root.path().join("up.git");
        git(&copy, &["commit", "-q", "--allow-empty", "-m", "local"]);
        let worker = worker_with_canopys_choices(trusted(copy.clone()));
        worker.send(Job::PushTarget);
        let events = events_until(&worker, |e| matches!(e, Event::PushTarget(_)));
        let Some(Event::PushTarget(Ok(asked))) = events.last() else {
            panic!("{events:?}");
        };
        assert_eq!(asked.branch, "main");
        let force = |expect: &canopy_core::PushTarget| {
            worker.send(Job::Op(Op::Push {
                force: true,
                expect: Some(expect.clone()),
            }));
            events_until(&worker, |e| matches!(e, Event::OpFinished { .. }))
        };

        git(&copy, &["checkout", "-q", "-b", "other"]);
        let events = force(asked);
        let Some(Event::OpFinished {
            result: Err(error), ..
        }) = events.last()
        else {
            panic!("a changed branch must be refused: {events:?}");
        };
        assert!(error.contains("main") && error.contains("other"), "{error}");
        assert_eq!(
            git(&up, &["branch", "--list", "other"]),
            "",
            "nothing pushed"
        );
        assert_ne!(
            git(&up, &["rev-parse", "main"]),
            git(&copy, &["rev-parse", "main"]),
            "main was not pushed either"
        );

        // Back on the branch the question named, the same confirmation goes through.
        git(&copy, &["checkout", "-q", "main"]);
        let events = force(asked);
        assert!(
            matches!(events.last(), Some(Event::OpFinished { result: Ok(_), .. })),
            "{events:?}"
        );
        assert_eq!(
            git(&up, &["rev-parse", "main"]),
            git(&copy, &["rev-parse", "main"])
        );
    }

    /// A rebase that stops on conflicts is not a failure: like a pull, it says so and the tab
    /// takes the user to the conflicted files.
    #[test]
    fn a_rebase_that_stops_on_conflicts_is_reported_as_stopped() {
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(root.path().join("home")).expect("home");
        std::fs::write(root.path().join("home").join("gitconfig"), "").expect("config");
        let repo = root.path().join("repo");
        std::fs::create_dir(&repo).expect("repo");
        git(&repo, &["init", "-q", "-b", "main"]);
        for (k, v) in [
            ("user.name", "Ada"),
            ("user.email", "ada@example.com"),
            ("commit.gpgsign", "false"),
        ] {
            git(&repo, &["config", k, v]);
        }
        std::fs::write(repo.join("a.txt"), "base\n").expect("write");
        std::fs::write(repo.join("d.txt"), "d\n").expect("write");
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "base"]);
        git(&repo, &["switch", "-q", "-c", "other"]);
        std::fs::write(repo.join("a.txt"), "theirs\n").expect("write");
        git(&repo, &["commit", "-q", "-am", "theirs"]);
        git(&repo, &["switch", "-q", "main"]);
        std::fs::write(repo.join("a.txt"), "ours\n").expect("write");
        git(&repo, &["commit", "-q", "-am", "ours"]);
        // Uncommitted changes, which Canopy's Autostash choice sets aside around the operation.
        std::fs::write(repo.join("d.txt"), "d\nmy edit\n").expect("write");
        let worker = worker_with_canopys_choices(trusted(repo));
        worker.send(Job::Op(Op::Rebase("other".into())));
        let events = events_until(&worker, |e| {
            matches!(e, Event::Conflicted { .. } | Event::OpFinished { .. })
        });
        assert!(
            matches!(
                events.last(),
                Some(Event::Conflicted {
                    autostashed: true,
                    ..
                })
            ),
            "{events:?}"
        );
    }

    /// #234: a merge that stops on conflicts is the user's to resolve, like a pull or a rebase
    /// that does, and not a failed operation.
    #[test]
    fn a_merge_that_stops_on_conflicts_is_reported_as_stopped() {
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(root.path().join("home")).expect("home");
        std::fs::write(root.path().join("home").join("gitconfig"), "").expect("config");
        let repo = root.path().join("repo");
        std::fs::create_dir(&repo).expect("repo");
        git(&repo, &["init", "-q", "-b", "main"]);
        for (k, v) in [
            ("user.name", "Ada"),
            ("user.email", "ada@example.com"),
            ("commit.gpgsign", "false"),
        ] {
            git(&repo, &["config", k, v]);
        }
        std::fs::write(repo.join("a.txt"), "base\n").expect("write");
        std::fs::write(repo.join("d.txt"), "d\n").expect("write");
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "base"]);
        git(&repo, &["switch", "-q", "-c", "other"]);
        std::fs::write(repo.join("a.txt"), "theirs\n").expect("write");
        git(&repo, &["commit", "-q", "-am", "theirs"]);
        git(&repo, &["switch", "-q", "main"]);
        std::fs::write(repo.join("a.txt"), "ours\n").expect("write");
        git(&repo, &["commit", "-q", "-am", "ours"]);
        // Uncommitted changes, which Canopy's Autostash choice sets aside around the operation.
        std::fs::write(repo.join("d.txt"), "d\nmy edit\n").expect("write");
        let worker = worker_with_canopys_choices(trusted(repo));
        worker.send(Job::Op(Op::Merge(
            "other".into(),
            canopy_core::MergeMode::default(),
        )));
        let events = events_until(&worker, |e| {
            matches!(e, Event::Conflicted { .. } | Event::OpFinished { .. })
        });
        assert!(
            matches!(
                events.last(),
                Some(Event::Conflicted {
                    autostashed: true,
                    ..
                })
            ),
            "{events:?}"
        );
    }

    /// A repository whose `main` and `other` both changed `a.txt`, with a stash of `a.txt` made on
    /// `other` that cannot apply on `main`. Returns the repository, the tip of `other` and the
    /// stash's commit, with `main` checked out and the working tree clean.
    fn conflicting_repo(root: &Path) -> (PathBuf, Oid, Oid) {
        std::fs::create_dir_all(root.join("home")).expect("home");
        std::fs::write(root.join("home").join("gitconfig"), "").expect("config");
        let repo = root.join("repo");
        std::fs::create_dir(&repo).expect("repo");
        git(&repo, &["init", "-q", "-b", "main"]);
        for (k, v) in [
            ("user.name", "Ada"),
            ("user.email", "ada@example.com"),
            ("commit.gpgsign", "false"),
        ] {
            git(&repo, &["config", k, v]);
        }
        std::fs::write(repo.join("a.txt"), "base\n").expect("write");
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "base"]);
        git(&repo, &["switch", "-q", "-c", "other"]);
        std::fs::write(repo.join("a.txt"), "theirs\n").expect("write");
        git(&repo, &["commit", "-q", "-am", "theirs"]);
        let other = Oid::from_str(&git(&repo, &["rev-parse", "HEAD"])).expect("oid");
        std::fs::write(repo.join("a.txt"), "stashed\n").expect("write");
        git(&repo, &["stash", "push", "-q"]);
        let stash = Oid::from_str(&git(&repo, &["rev-parse", "stash@{0}"])).expect("oid");
        git(&repo, &["switch", "-q", "main"]);
        std::fs::write(repo.join("a.txt"), "ours\n").expect("write");
        git(&repo, &["commit", "-q", "-am", "ours"]);
        (repo, other, stash)
    }

    /// Runs `op` in `repo` and returns the event that ends it.
    fn run_op(repo: &Path, op: Op) -> Event {
        let worker = worker_with_canopys_choices(trusted(repo.to_path_buf()));
        worker.send(Job::Op(op));
        events_until(&worker, |e| {
            matches!(e, Event::Conflicted { .. } | Event::OpFinished { .. })
        })
        .pop()
        .expect("an ending event")
    }

    /// #265: a cherry-pick or a revert that stops on conflicts is the user's to resolve, like a
    /// merge that does, and not a failed operation.
    #[test]
    fn a_cherry_pick_or_revert_that_stops_on_conflicts_is_reported_as_stopped() {
        for (revert, state_file) in [(false, "CHERRY_PICK_HEAD"), (true, "REVERT_HEAD")] {
            let root = tempfile::tempdir().expect("tempdir");
            let (repo, other, _) = conflicting_repo(root.path());
            let op = if revert {
                Op::Revert(other)
            } else {
                Op::CherryPick(other)
            };
            let ended = run_op(&repo, op);
            assert!(
                matches!(
                    &ended,
                    Event::Conflicted {
                        autostashed: false,
                        stash: false,
                        ..
                    }
                ),
                "revert {revert}: {ended:?}"
            );
            assert!(
                repo.join(".git").join(state_file).exists(),
                "git is left mid-operation for the user to resolve"
            );
            assert_eq!(git(&repo, &["ls-files", "-u"]).lines().count(), 3);
        }
    }

    /// #265: a stash applied or popped onto changes it conflicts with is the user's to resolve; the
    /// stash is kept and the event says it was a stash, which has nothing to continue or abort.
    #[test]
    fn a_stash_apply_that_stops_on_conflicts_is_reported_as_stopped() {
        for (by_commit, pop) in [(false, false), (false, true), (true, false), (true, true)] {
            let root = tempfile::tempdir().expect("tempdir");
            let (repo, _, stash) = conflicting_repo(root.path());
            let op = if by_commit {
                Op::StashApplyCommit { oid: stash, pop }
            } else {
                Op::StashApply { index: 0, pop }
            };
            let ended = run_op(&repo, op);
            assert!(
                matches!(
                    &ended,
                    Event::Conflicted {
                        autostashed: false,
                        stash: true,
                        ..
                    }
                ),
                "by commit {by_commit}, pop {pop}: {ended:?}"
            );
            assert_eq!(git(&repo, &["ls-files", "-u"]).lines().count(), 3);
            assert_eq!(
                git(&repo, &["rev-parse", "stash@{0}"]),
                stash.to_string(),
                "git keeps the stash after a conflicted apply or pop"
            );
        }
    }

    /// Only a stop on conflicts is a stop: a stash that git refuses without conflicts, an apply
    /// onto files that are already conflicted and a cherry-pick refused for the same reason stay
    /// failures with git's own message.
    #[test]
    fn a_refusal_that_is_not_a_new_conflict_stays_a_failure() {
        let root = tempfile::tempdir().expect("tempdir");
        let (repo, other, stash) = conflicting_repo(root.path());
        // Uncommitted edits to the stash's file: git refuses before touching anything.
        std::fs::write(repo.join("a.txt"), "mine\n").expect("write");
        let ended = run_op(
            &repo,
            Op::StashApply {
                index: 0,
                pop: false,
            },
        );
        assert!(
            matches!(&ended, Event::OpFinished { result: Err(e), .. } if e.contains("a.txt")),
            "{ended:?}"
        );
        assert_eq!(git(&repo, &["ls-files", "-u"]), "");
        git(&repo, &["checkout", "-q", "--", "a.txt"]);
        // Conflicted already (a cherry-pick stopped): another operation stops nothing new.
        let _ = run_op(&repo, Op::CherryPick(other));
        assert!(repo.join(".git").join("CHERRY_PICK_HEAD").exists());
        for op in [
            Op::CherryPick(other),
            Op::Revert(other),
            Op::StashApplyCommit {
                oid: stash,
                pop: false,
            },
        ] {
            let ended = run_op(&repo, op);
            assert!(
                matches!(&ended, Event::OpFinished { result: Err(_), .. }),
                "{ended:?}"
            );
        }
    }

    /// A leftover `.git/MERGE_AUTOSTASH` (an earlier `git merge --squash --autostash` outside
    /// Canopy) is not this merge's: a merge that stops on conflicts without stashing anything
    /// does not claim to hold the user's changes.
    #[test]
    fn a_leftover_autostash_is_not_reported_as_held_by_a_stopped_merge() {
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(root.path().join("home")).expect("home");
        std::fs::write(root.path().join("home").join("gitconfig"), "").expect("config");
        let repo = root.path().join("repo");
        std::fs::create_dir(&repo).expect("repo");
        git(&repo, &["init", "-q", "-b", "main"]);
        for (k, v) in [
            ("user.name", "Ada"),
            ("user.email", "ada@example.com"),
            ("commit.gpgsign", "false"),
        ] {
            git(&repo, &["config", k, v]);
        }
        std::fs::write(repo.join("a.txt"), "base\n").expect("write");
        std::fs::write(repo.join("d.txt"), "d\n").expect("write");
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "base"]);
        git(&repo, &["switch", "-q", "-c", "other"]);
        std::fs::write(repo.join("a.txt"), "theirs\n").expect("write");
        git(&repo, &["commit", "-q", "-am", "theirs"]);
        git(&repo, &["switch", "-q", "-c", "side", "main"]);
        std::fs::write(repo.join("s.txt"), "s\n").expect("write");
        git(&repo, &["add", "s.txt"]);
        git(&repo, &["commit", "-q", "-m", "side"]);
        git(&repo, &["switch", "-q", "main"]);
        std::fs::write(repo.join("a.txt"), "ours\n").expect("write");
        git(&repo, &["commit", "-q", "-am", "ours"]);
        // Git's own squash with autostash leaves the stash unapplied in `.git/MERGE_AUTOSTASH`.
        std::fs::write(repo.join("d.txt"), "d\nold edit\n").expect("write");
        git(
            &repo,
            &[
                "merge",
                "--no-edit",
                "--squash",
                "--autostash",
                "--",
                "side",
            ],
        );
        // Drop the squash's result without `git reset`, which would clear the leftover too.
        git(&repo, &["rm", "-q", "-f", "--", "s.txt"]);
        let leftover = repo.join(".git").join("MERGE_AUTOSTASH");
        assert!(leftover.exists(), "the premise: a stale autostash");
        let worker = worker_with_canopys_choices(trusted(repo));
        worker.send(Job::Op(Op::Merge(
            "other".into(),
            canopy_core::MergeMode::default(),
        )));
        let events = events_until(&worker, |e| {
            matches!(e, Event::Conflicted { .. } | Event::OpFinished { .. })
        });
        assert!(
            matches!(
                events.last(),
                Some(Event::Conflicted {
                    autostashed: false,
                    ..
                })
            ),
            "{events:?}"
        );
        assert!(leftover.exists(), "the leftover is not Canopy's to remove");
    }

    /// #92: the repository thread reads the commit template; a missing file is reported, not
    /// swallowed.
    #[test]
    fn the_worker_reads_the_commit_template() {
        let root = tempfile::tempdir().expect("tempdir");
        let copy = diverged_clone(root.path());
        let ask = |copy: &Path| {
            let worker = worker_with_canopys_choices(copy.to_path_buf());
            worker.send(Job::CommitTemplate);
            let events = events_until(&worker, |e| matches!(e, Event::CommitTemplate(_)));
            match events.into_iter().last() {
                Some(Event::CommitTemplate(result)) => result,
                other => panic!("{other:?}"),
            }
        };
        crate::trust::add_for_tests(copy.clone());
        assert_eq!(ask(&copy), Ok(None));
        std::fs::write(copy.join("msg.txt"), "feat: \n# comment\nWhy:\n").expect("write");
        git(&copy, &["config", "commit.template", "msg.txt"]);
        assert_eq!(ask(&copy), Ok(Some("feat: \nWhy:".into())));
        git(&copy, &["config", "commit.template", "gone.txt"]);
        let error = ask(&copy).expect_err("no such file");
        assert!(error.contains("gone.txt"), "{error}");
    }

    /// A repository outside every trusted folder names the template file itself, so its
    /// configuration is not followed there: no file is read.
    #[test]
    fn an_untrusted_repository_reads_no_commit_template() {
        let root = tempfile::tempdir().expect("tempdir");
        let copy = diverged_clone(root.path());
        std::fs::write(copy.join("msg.txt"), "feat: \n").expect("write");
        git(&copy, &["config", "commit.template", "msg.txt"]);
        let worker = worker_with_canopys_choices(copy);
        worker.send(Job::CommitTemplate);
        let events = events_until(&worker, |e| matches!(e, Event::CommitTemplate(_)));
        assert!(
            matches!(events.last(), Some(Event::CommitTemplate(Ok(None)))),
            "{events:?}"
        );
    }

    /// PS-F1: the Pull menu's default comes from the repository thread, never the UI thread.
    #[test]
    fn the_worker_reads_what_the_configuration_asks_a_pull_to_do() {
        let (_root, _src, copy, _oldest, _middle) = partial_clone("--no-local");
        // The repository's own values are read last, so the developer's configuration cannot
        // change the answer.
        git(&copy, &["config", "pull.ff", "true"]);
        git(&copy, &["config", "branch.main.rebase", "true"]);
        let worker = worker_with_canopys_choices(copy);
        worker.send(Job::PullConfig);
        let events = events_until(&worker, |e| matches!(e, Event::PullConfig(_)));
        let Some(Event::PullConfig(Ok(config))) = events.last() else {
            panic!("expected the pull settings, got {events:?}");
        };
        assert_eq!(config.strategy, Some(PullStrategy::Rebase));
        assert_eq!(
            config.describe().as_deref(),
            Some("branch.main.rebase=true (repository)")
        );
    }

    /// A repository whose own files would run programs: a `pre-commit` hook and a
    /// `core.fsmonitor` command that each leave a marker file, and a change to commit.
    #[cfg(unix)]
    fn hostile_repository(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;

        fn script(path: &Path, marker: &Path) {
            std::fs::write(path, format!("#!/bin/sh\ntouch '{}'\n", marker.display()))
                .expect("script");
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("mode");
        }
        std::fs::create_dir_all(root.join("home")).expect("home");
        std::fs::write(root.join("home").join("gitconfig"), "").expect("config");
        let repo = root.join("repo");
        std::fs::create_dir(&repo).expect("repo");
        git(&repo, &["init", "-q", "-b", "main"]);
        for (k, v) in [
            ("user.name", "Ada"),
            ("user.email", "ada@example.com"),
            ("commit.gpgsign", "false"),
        ] {
            git(&repo, &["config", k, v]);
        }
        std::fs::write(repo.join("a.txt"), "base\n").expect("write");
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "base"]);
        let hook = root.join("hook-ran");
        let fsmonitor = root.join("fsmonitor-ran");
        script(&repo.join(".git").join("hooks").join("pre-commit"), &hook);
        let program = root.join("fsmonitor");
        script(&program, &fsmonitor);
        git(
            &repo,
            &["config", "core.fsmonitor", &program.to_string_lossy()],
        );
        std::fs::write(repo.join("a.txt"), "changed\n").expect("write");
        (repo, hook, fsmonitor)
    }

    /// An operation sent to a repository that is not trusted ends in this refusal.
    fn refusal_of(worker: &Worker, op: Op) -> Option<String> {
        worker.send(Job::Op(op));
        let events = events_until(worker, |e| matches!(e, Event::OpFinished { .. }));
        match events.last() {
            Some(Event::OpFinished { result, .. }) => result.as_ref().err().cloned(),
            other => panic!("{other:?}"),
        }
    }

    /// #228: opening a repository whose own files name programs starts none of them, and every
    /// action in it is refused until its folder is trusted; trusted, the hook runs as in a
    /// terminal and `core.fsmonitor` stays off.
    #[cfg(unix)]
    #[test]
    fn an_untrusted_repository_is_read_only_and_starts_no_program() {
        let root = tempfile::tempdir().expect("tempdir");
        let (repo, hook, fsmonitor) = hostile_repository(root.path());
        let worker = Worker::spawn(repo.clone(), &egui::Context::default());
        worker.send(Job::Refresh(100));
        let events = events_until(&worker, |e| matches!(e, Event::Trust { .. }));
        assert!(
            matches!(events.last(), Some(Event::Trust { trusted: false, .. })),
            "the tab is told it is read-only: {events:?}"
        );
        for op in [
            Op::StageAll,
            Op::Commit {
                message: "no".into(),
                amend: false,
                signoff: false,
            },
            Op::Fetch {
                prune: false,
                quiet: true,
            },
            Op::CreateBranch {
                name: "topic".into(),
                start: None,
                checkout: true,
            },
            // git-lfs is a program the repository's own configuration steers.
            Op::LfsPull,
            Op::LfsFetch,
            Op::LfsLock("a.txt".into()),
            Op::LfsTrack {
                pattern: "*.bin".into(),
                literal: false,
            },
        ] {
            let label = op.label();
            let refusal = refusal_of(&worker, op).unwrap_or_default();
            assert!(
                refusal.starts_with(READ_ONLY),
                "{label} is refused and says why: {refusal}"
            );
            // The refusal names the program sources, not only that there are some.
            for source in ["core.fsmonitor", "hook pre-commit"] {
                assert!(
                    refusal.contains(source),
                    "{label} names {source}: {refusal}"
                );
            }
        }
        assert!(!hook.exists(), "the pre-commit hook did not run");
        assert!(!fsmonitor.exists(), "core.fsmonitor did not run");
        assert!(
            git(&repo, &["branch", "--list", "topic"]).is_empty(),
            "nothing was created"
        );

        // Trusted: the same commit goes through, and git runs the hook as it would in a
        // terminal. The fsmonitor program is off however the repository names it.
        trusted(repo);
        assert_eq!(refusal_of(&worker, Op::StageAll), None);
        let commit = Op::Commit {
            message: "yes".into(),
            amend: false,
            signoff: false,
        };
        assert_eq!(refusal_of(&worker, commit), None);
        assert!(hook.exists(), "the hook is the user's once they trust it");
        assert!(!fsmonitor.exists(), "core.fsmonitor is never run");
    }

    /// Git LFS reads that run git-lfs (the patterns, the locks) are refused in a repository that
    /// is not trusted, like its commands; the files it stores are read without the program and
    /// are not.
    #[cfg(unix)]
    #[test]
    fn git_lfs_programs_do_not_run_for_an_untrusted_repository() {
        let root = tempfile::tempdir().expect("tempdir");
        let (repo, _hook, _fsmonitor) = hostile_repository(root.path());
        let worker = Worker::spawn(repo, &egui::Context::default());
        worker.send(Job::Refresh(100));
        events_until(&worker, |e| matches!(e, Event::Trust { .. }));
        worker.send(Job::LfsLocks(7));
        worker.send(Job::LfsPatterns(8));
        worker.send(Job::Lfs { known: false });
        let events = events_until(&worker, |e| matches!(e, Event::Lfs(_)));
        assert!(
            events.iter().any(|e| matches!(
                e,
                Event::LfsLocks { serial: 7, result: Err(m) }
                    if m.starts_with(READ_ONLY) && m.contains("core.fsmonitor")
            )),
            "{events:?}"
        );
        assert!(
            events.iter().any(|e| matches!(
                e,
                Event::LfsPatterns { serial: 8, result: Err(m) }
                    if m.starts_with(READ_ONLY) && m.contains("hook pre-commit")
            )),
            "{events:?}"
        );
        assert!(
            matches!(events.last(), Some(Event::Lfs(info)) if !info.in_use && info.files.is_empty()),
            "a repository without LFS attributes has nothing to read: {events:?}"
        );
    }

    /// #228: with trust safety off nothing is refused or told to be read-only, the trusted list
    /// is kept meanwhile, and turning it on restores every gate. Runs in a child process: the
    /// switch and the list are process-wide, and other tests rely on them.
    #[cfg(unix)]
    #[test]
    fn trust_safety_off_trusts_everything_and_on_again_restores_the_gates() {
        if !in_isolated_git_env(
            "worker::read_tests::trust_safety_off_trusts_everything_and_on_again_restores_the_gates",
        ) {
            return;
        }
        let root = tempfile::tempdir().expect("tempdir");
        let (repo, _hook, _fsmonitor) = hostile_repository(root.path());
        let settings = |safety: bool, folders: &[&Path]| crate::settings::Settings {
            trust_safety: safety,
            trusted_folders: folders.iter().map(|folder| folder.to_path_buf()).collect(),
            ..crate::settings::Settings::default()
        };
        let told = |worker: &Worker| {
            worker.send(Job::RefreshStatus);
            let events = events_until(worker, |e| matches!(e, Event::Trust { .. }));
            matches!(events.last(), Some(Event::Trust { trusted: true, .. }))
        };
        crate::trust::publish(&settings(true, &[]));
        let worker = Worker::spawn(repo.clone(), &egui::Context::default());
        assert!(!told(&worker), "untrusted to begin with");
        assert!(
            refusal_of(&worker, Op::StageAll).is_some_and(|m| m.starts_with(READ_ONLY)),
            "refused"
        );

        crate::trust::publish(&settings(false, &[]));
        assert!(told(&worker), "off: the tab is told it is trusted");
        assert_eq!(
            refusal_of(&worker, Op::StageAll),
            None,
            "off: nothing refused"
        );

        crate::trust::publish(&settings(true, &[]));
        assert!(!told(&worker), "on again: read-only again");
        assert!(
            refusal_of(&worker, Op::UnstageAll).is_some_and(|m| m.starts_with(READ_ONLY)),
            "on again: refused again"
        );

        // The list was kept while the switch was off; here it is put back with the switch.
        crate::trust::publish(&settings(false, &[&repo]));
        crate::trust::publish(&settings(true, &[&repo]));
        assert!(told(&worker), "the folder is trusted, not merely unchecked");
        assert_eq!(refusal_of(&worker, Op::UnstageAll), None);
    }

    #[test]
    fn an_untrusted_partial_clone_downloads_nothing_and_says_why() {
        // Blobless: the file list is offline, the line counts would need a download.
        let (_root, _src, copy, _oldest, middle) = partial_clone("--filter=blob:none");
        let worker = worker_with_canopys_choices(copy);
        worker.send(Job::Details(middle));
        let events = events_until(&worker, |e| matches!(e, Event::DownloadsRefused { .. }));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::Details(d) if !d.stats_known && d.files.len() == 1)),
            "the file list still shows: {events:?}"
        );
        assert!(
            matches!(events.last(), Some(Event::DownloadsRefused { remote, .. }) if remote == "origin"),
            "{events:?}"
        );
        assert!(progress(&events).is_empty(), "no download ran: {events:?}");

        // Treeless: even the file list needs a download, so the read fails and explains.
        let (_root, _src, copy, oldest, _) = partial_clone("--filter=tree:0");
        let worker = worker_with_canopys_choices(copy);
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
        assert!(message.contains(NOT_TRUSTED), "{message}");
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::DownloadsRefused { remote, .. } if remote == "origin")),
            "{events:?}"
        );
        assert!(progress(&events).is_empty(), "no download ran: {events:?}");
    }

    #[test]
    fn details_in_a_blobless_clone_show_the_file_list_first_and_the_counts_after_downloading() {
        let (_root, _src, copy, _oldest, middle) = trusted_partial_clone("--filter=blob:none");
        let worker = worker_with_canopys_choices(copy);

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
        let worker = worker_with_canopys_choices(copy);
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

    /// #36: a diff that failed because a partial clone could not download its blobs is reported
    /// with the retryable marker, so the tab offers Retry rather than asking on every refresh.
    #[test]
    fn a_failed_diff_download_is_reported_as_retryable() {
        let (root, src, copy, _, middle) = trusted_partial_clone("--filter=blob:none");
        let worker = worker_with_canopys_choices(copy);
        let moved = root.path().join("src-moved");
        std::fs::rename(&src, &moved).expect("hide the remote");
        let key = DiffKey {
            source: DiffSource::Commit(middle),
            path: "a.txt".into(),
            old_path: None,
            conflicted: false,
            context: 3,
            whitespace: Whitespace::Show,
            highlight: false,
        };
        worker.send(Job::Diff(key.clone()));
        let events = events_until(&worker, |e| matches!(e, Event::DiffNotDownloaded(_)));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::Diff(k, Err(m)) if *k == key && m.contains("origin"))),
            "{events:?}"
        );
        assert!(matches!(events.last(), Some(Event::DiffNotDownloaded(k)) if *k == key));

        std::fs::rename(&moved, &src).expect("restore the remote");
        worker.send(Job::Diff(key));
        let events = events_until(&worker, |e| matches!(e, Event::Diff(..)));
        assert!(
            matches!(events.last(), Some(Event::Diff(_, Ok(_)))),
            "{events:?}"
        );
    }

    /// #43: Publish works on any local branch, not only the checked-out one, and makes the remote
    /// branch its upstream.
    #[test]
    fn a_branch_that_is_not_checked_out_can_be_published() {
        let root = tempfile::tempdir().expect("tempdir");
        let copy = diverged_clone(root.path());
        git(&copy, &["branch", "topic"]);
        let repo = Repo::open(&copy).expect("open");
        let op = Op::PublishBranch {
            name: "topic".into(),
            remote: "origin".into(),
        };
        assert_eq!(op.kind(), OpKind::Push);
        assert_eq!(op.label(), "Publishing topic");
        perform(&repo, op, |_| {}).expect("publishes");
        let up = root.path().join("up");
        assert_eq!(
            git(&up, &["rev-parse", "refs/heads/topic"]),
            git(&copy, &["rev-parse", "refs/heads/topic"]),
            "the remote has the branch"
        );
        assert_eq!(git(&copy, &["config", "branch.topic.remote"]), "origin");
        assert_eq!(
            git(&copy, &["config", "branch.topic.merge"]),
            "refs/heads/topic"
        );
        let refused = perform(
            &repo,
            Op::PublishBranch {
                name: "nothing".into(),
                remote: "origin".into(),
            },
            |_| {},
        )
        .expect_err("not a branch");
        assert!(refused.to_string().contains("not a local branch"));
    }

    /// #99: a commit is exported to a folder and applied again, by its file or by the folder; a
    /// patch that does not fit stops in the "Applying patches" state and names its file.
    #[test]
    fn exported_patches_apply_again_and_a_misfit_stops_naming_its_file() {
        let root = tempfile::tempdir().expect("tempdir");
        let copy = diverged_clone(root.path());
        // The fixture helper ignores system configuration; Canopy's git calls respect it.
        git(&copy, &["config", "core.autocrlf", "false"]);
        let repo = Repo::open(&copy).expect("open");
        let local = repo.snapshot(5).expect("snapshot").head.oid.expect("head");
        let out = root.path().join("out");
        let exported = Op::ExportPatches {
            oid: local,
            count: 1,
            dir: out.clone(),
        };
        assert_eq!(exported.kind(), OpKind::Other);
        let message = perform(&repo, exported, |_| {}).expect("exports");
        assert!(
            message.starts_with("Wrote 0001-local.patch to "),
            "{message}"
        );
        assert!(out.join("0001-local.patch").is_file());

        git(&copy, &["checkout", "-q", "-B", "fresh", "origin/main"]);
        let applied = perform(&repo, Op::ApplyPatches { path: out.clone() }, |_| {})
            .expect("applies the folder");
        assert_eq!(applied, "Applied local.");
        assert_eq!(
            git(&copy, &["log", "-1", "--format=%an: %s"]),
            "Ada Lovelace: local"
        );

        // The same patch again does not fit: it stops, says where, and leaves the state to resolve.
        git(&copy, &["checkout", "-q", "-B", "second", "origin/main"]);
        std::fs::write(copy.join("local.txt"), "other\n").expect("write");
        git(&copy, &["add", "."]);
        git(&copy, &["commit", "-q", "-m", "other"]);
        let stopped = perform(
            &repo,
            Op::ApplyPatches {
                path: out.join("0001-local.patch"),
            },
            |_| {},
        )
        .expect_err("does not fit");
        assert!(stopped.to_string().contains("local.txt"), "{stopped}");
        assert_eq!(
            repo.snapshot(5).expect("snapshot").state,
            RepoState::ApplyMailbox
        );
    }

    /// #99: several commits are exported in one go, oldest first, counting back along first
    /// parents (and no further than the root); they apply again in order, and a merge commit is
    /// skipped with a word about it instead of writing its parent's patch.
    #[test]
    fn several_commits_export_oldest_first_and_apply_again() {
        let root = tempfile::tempdir().expect("tempdir");
        let copy = diverged_clone(root.path());
        git(&copy, &["config", "core.autocrlf", "false"]);
        for name in ["second", "third"] {
            std::fs::write(copy.join(format!("{name}.txt")), format!("{name}\n")).expect("write");
            git(&copy, &["add", "."]);
            git(&copy, &["commit", "-q", "-m", name]);
        }
        let repo = Repo::open(&copy).expect("open");
        let head = repo.snapshot(5).expect("snapshot").head.oid.expect("head");
        let out = root.path().join("out");
        let message = perform(
            &repo,
            Op::ExportPatches {
                oid: head,
                count: 3,
                dir: out.clone(),
            },
            |_| {},
        )
        .expect("exports three");
        assert!(
            message.starts_with("Wrote 3 patches (0001-local.patch to 0003-third.patch) to "),
            "{message}"
        );
        let mut files: Vec<_> = std::fs::read_dir(&out)
            .expect("list")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        files.sort();
        assert_eq!(
            files,
            ["0001-local.patch", "0002-second.patch", "0003-third.patch"]
        );

        git(&copy, &["checkout", "-q", "-B", "fresh", "origin/main"]);
        let applied = perform(&repo, Op::ApplyPatches { path: out }, |_| {}).expect("applies");
        assert_eq!(applied, "Applied 3 commits: local; second; third.");
        assert_eq!(
            git(&copy, &["log", "--format=%s", "-3"]),
            "third\nsecond\nlocal"
        );

        // More than the history holds stops at the root commit (no patch for "base" is lost).
        let all = perform(
            &repo,
            Op::ExportPatches {
                oid: head,
                count: 50,
                dir: root.path().join("all"),
            },
            |_| {},
        )
        .expect("exports the whole line");
        assert!(
            all.starts_with("Wrote 4 patches (0001-base.patch to 0004-third.patch)"),
            "{all}"
        );

        // A merge in the line is skipped and named; a merge alone is refused.
        git(&copy, &["checkout", "-q", "main"]);
        git(&copy, &["merge", "-q", "--no-ff", "-m", "join", "fresh"]);
        let merge = repo.snapshot(5).expect("snapshot").head.oid.expect("head");
        let skipped = perform(
            &repo,
            Op::ExportPatches {
                oid: merge,
                count: 2,
                dir: root.path().join("merge"),
            },
            |_| {},
        )
        .expect("exports the commit before the merge");
        assert!(skipped.contains("Skipped 1 merge commit"), "{skipped}");
        let refused = perform(
            &repo,
            Op::ExportPatches {
                oid: merge,
                count: 1,
                dir: root.path().join("alone"),
            },
            |_| {},
        )
        .expect_err("a merge alone has no patch");
        assert!(
            refused
                .to_string()
                .contains("A merge commit cannot be exported as a patch"),
            "{refused}"
        );
        assert!(!root.path().join("alone").exists());
    }

    #[test]
    fn a_folder_without_patches_says_so() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("notes.txt"), "x").expect("write");
        let err = patch_files(dir.path()).expect_err("nothing to apply");
        assert!(err.to_string().contains("has no .patch files"), "{err}");
        std::fs::write(dir.path().join("0002-b.patch"), "x").expect("write");
        std::fs::write(dir.path().join("0001-a.patch"), "x").expect("write");
        let names: Vec<_> = patch_files(dir.path())
            .expect("patches")
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        assert_eq!(names, ["0001-a.patch", "0002-b.patch"]);
    }

    #[test]
    fn a_failed_status_download_is_remembered_until_retry() {
        let (root, src, copy, oldest, _) = trusted_partial_clone("--filter=tree:0");
        git(&copy, &["reset", "-q", "--soft", &oldest.to_string()]);
        let moved = root.path().join("src-moved");
        std::fs::rename(&src, &moved).expect("hide the remote");
        let worker = Worker::spawn(copy, &egui::Context::default());
        // The snapshots from the next `Snapshot` on, and whether a download ran before it.
        let next = |worker: &Worker| -> (Arc<RepoSnapshot>, bool) {
            let mut downloaded = false;
            loop {
                match worker
                    .events
                    .recv_timeout(Duration::from_secs(60))
                    .expect("the worker answers")
                {
                    Event::Progress(line) if line.starts_with("Downloading") => downloaded = true,
                    Event::Snapshot(snap) => return (snap, downloaded),
                    _ => {}
                }
            }
        };

        worker.send(Job::Refresh(100));
        let (snap, downloaded) = next(&worker);
        assert!(downloaded, "the first read tries the download");
        let unavailable = snap.status_unavailable.as_ref().expect("no tree");
        assert!(unavailable.can_download, "{unavailable:?}");

        worker.send(Job::Refresh(0));
        let (again, downloaded) = next(&worker);
        assert!(!downloaded, "a refresh does not download again");
        assert_eq!(again.status_unavailable, snap.status_unavailable);

        std::fs::rename(&moved, &src).expect("restore the remote");
        worker.send(Job::RetryDownloads);
        let (healed, downloaded) = next(&worker);
        assert!(downloaded, "Retry downloads");
        assert_eq!(healed.status_unavailable, None);
    }

    #[test]
    fn a_refusal_names_the_version_of_the_trusted_list_that_made_it()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let repo = Repo::init(dir.path(), "main", false)?;
        let before = crate::trust::changes();
        assert!(!trusted_now(&repo), "nothing is trusted here");
        let after = crate::trust::changes();
        // The list may change meanwhile (other tests publish one); the count only grows.
        assert!((before..=after).contains(&TRUST_VERSION.get()));

        let sent = RefCell::new(Vec::new());
        let emit = |e: Event| {
            if let Event::DownloadsRefused { trust, .. } = e {
                sent.borrow_mut().push(trust);
            }
        };
        TRUST_VERSION.set(7);
        let error = download_refused(&emit, Some("origin".into()));
        assert!(matches!(error, Error::NotDownloaded { .. }));
        assert_eq!(*sent.borrow(), [7]);
        Ok(())
    }

    #[test]
    fn a_read_retries_with_downloading_only_when_a_remote_can_supply_it() {
        let lines = RefCell::new(Vec::new());
        let refused = RefCell::new(Vec::new());
        let emit = |e: Event| match e {
            Event::Progress(line) => lines.borrow_mut().push(line),
            Event::DownloadsRefused { remote, .. } => refused.borrow_mut().push(remote),
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
        assert!(e.to_string().contains(NOT_TRUSTED), "{e}");
        assert_eq!(*refused.borrow(), ["origin".to_owned()]);
        assert!(lines.borrow().is_empty());

        // Trust withdrawn between the first check and the moment the fetch would start: it
        // does not start.
        let asked = Cell::new(0);
        let modes = RefCell::new(Vec::new());
        let r: canopy_core::Result<u8> = read_downloading(
            &emit,
            &|| {
                asked.set(asked.get() + 1);
                asked.get() == 1
            },
            |m| {
                modes.borrow_mut().push(m);
                Err(not_downloaded(Some("origin")))
            },
        );
        r.unwrap_err();
        assert_eq!(*modes.borrow(), [MissingObjects::Fail]);
        assert_eq!(asked.get(), 2, "asked again right before the download");

        // Core's own check, right before `git fetch`, stopped a download that both of ours let
        // through: trust was withdrawn in between. That is a refusal like any other.
        refused.borrow_mut().clear();
        let asked = Cell::new(0);
        let r: canopy_core::Result<u8> = read_downloading(
            &emit,
            &|| {
                asked.set(asked.get() + 1);
                asked.get() <= 2
            },
            |m| match m {
                MissingObjects::Fail => Err(not_downloaded(Some("origin"))),
                MissingObjects::Download => Err(Error::NotDownloaded {
                    remote: None,
                    reason: Some("The repository is not in a folder trusted for downloads.".into()),
                }),
            },
        );
        let Err(e) = r else {
            panic!("the stopped download is an error")
        };
        assert!(e.to_string().contains("Choose Trust…"), "{e}");
        assert_eq!(*refused.borrow(), ["origin".to_owned()]);
        assert_eq!(asked.get(), 3, "asked once more after the failure");

        // A download that fails while trust stands keeps its own error and reports no refusal.
        refused.borrow_mut().clear();
        let r: canopy_core::Result<u8> = read_downloading(&emit, &|| true, |m| match m {
            MissingObjects::Fail => Err(not_downloaded(Some("origin"))),
            MissingObjects::Download => Err(Error::NotDownloaded {
                remote: Some("origin".into()),
                reason: Some("network down".into()),
            }),
        });
        let Err(e) = r else {
            panic!("the failed download is an error")
        };
        assert!(e.to_string().contains("network down"), "{e}");
        assert!(refused.borrow().is_empty());
    }
}
