//! Plain data describing a repository. Everything here is cheap to clone and free of libgit2
//! handles, so it can cross threads and live in UI state.

use std::collections::HashMap;
use std::path::PathBuf;

use git2::Oid;

use crate::graph::GraphRow;

/// What `HEAD` points at.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeadInfo {
    /// Short branch name when `HEAD` is a symbolic reference to a branch.
    pub branch: Option<String>,
    /// The commit `HEAD` resolves to, if any (`None` on an unborn branch).
    pub oid: Option<Oid>,
    /// `HEAD` points directly at a commit.
    pub detached: bool,
}

impl HeadInfo {
    /// A short human label: the branch name, or the abbreviated commit when detached.
    pub fn label(&self) -> String {
        match (&self.branch, self.oid) {
            (Some(b), _) => b.clone(),
            (None, Some(oid)) => format!("detached at {}", short_id(oid)),
            (None, None) => "no commits yet".to_owned(),
        }
    }
}

/// An operation that is in progress and needs to be continued or aborted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RepoState {
    /// Nothing in progress.
    #[default]
    Clean,
    /// A merge stopped (usually on conflicts).
    Merge,
    /// A rebase (interactive or not) stopped.
    Rebase,
    /// A cherry-pick stopped.
    CherryPick,
    /// A revert stopped.
    Revert,
    /// `git bisect` is running.
    Bisect,
    /// `git am` / apply-mailbox is running.
    ApplyMailbox,
}

impl RepoState {
    /// Display name, e.g. "Merging".
    pub fn label(self) -> &'static str {
        match self {
            Self::Clean => "Clean",
            Self::Merge => "Merging",
            Self::Rebase => "Rebasing",
            Self::CherryPick => "Cherry-picking",
            Self::Revert => "Reverting",
            Self::Bisect => "Bisecting",
            Self::ApplyMailbox => "Applying patches",
        }
    }

    /// The `git` sub-command that owns this state and accepts `--continue` / `--abort`.
    pub fn command(self) -> Option<&'static str> {
        match self {
            Self::Merge => Some("merge"),
            Self::Rebase => Some("rebase"),
            Self::CherryPick => Some("cherry-pick"),
            Self::Revert => Some("revert"),
            Self::ApplyMailbox => Some("am"),
            Self::Clean | Self::Bisect => None,
        }
    }
}

/// A local branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchInfo {
    /// Short name, e.g. `feature/login`.
    pub name: String,
    /// The commit the branch points at.
    pub target: Option<Oid>,
    /// This is the checked-out branch.
    pub is_head: bool,
    /// Short name of the upstream branch, e.g. `origin/main`.
    pub upstream: Option<String>,
    /// Commits on this branch that the upstream lacks.
    pub ahead: usize,
    /// Commits on the upstream that this branch lacks.
    pub behind: usize,
    /// The other working folder (`git worktree`) the branch is checked out in, where git
    /// refuses to check it out here too.
    pub worktree: Option<PathBuf>,
}

/// One of the repository's working folders (`git worktree list`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInfo {
    /// The folder.
    pub path: PathBuf,
    /// The branch checked out there; `None` when its `HEAD` is detached.
    pub branch: Option<String>,
    /// The commit its `HEAD` is at (`None` on an unborn branch).
    pub head: Option<Oid>,
    /// It is the folder this repository was opened at.
    pub current: bool,
    /// It is the main working folder, the one the others were added from.
    pub main: bool,
}

/// A remote-tracking branch, e.g. `origin/main`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteBranchInfo {
    /// Remote name, e.g. `origin`.
    pub remote: String,
    /// Branch name on the remote, e.g. `main`.
    pub branch: String,
    /// The commit it points at.
    pub target: Option<Oid>,
}

impl RemoteBranchInfo {
    /// `remote/branch`.
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.remote, self.branch)
    }
}

/// A configured remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteInfo {
    /// Remote name.
    pub name: String,
    /// Fetch URL, if set.
    pub url: Option<String>,
    /// The branch `refs/remotes/<name>/HEAD` points at (e.g. `main`), the remote's default
    /// branch as last recorded by `git clone` or `git remote set-head`; `None` when unset.
    pub default_branch: Option<String>,
}

/// A tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagInfo {
    /// Tag name.
    pub name: String,
    /// The commit the tag (eventually) points at.
    pub target: Option<Oid>,
    /// Annotated tags carry a message.
    pub message: Option<String>,
}

/// A stash entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StashInfo {
    /// Position in the stash list (`stash@{index}`).
    pub index: usize,
    /// The stash message.
    pub message: String,
    /// The stash commit.
    pub oid: Oid,
}

/// How a file changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    /// New file.
    Added,
    /// Content changed.
    Modified,
    /// File removed.
    Deleted,
    /// Moved, possibly with edits.
    Renamed,
    /// File type changed (e.g. file to symlink).
    Typechange,
    /// Not tracked by Git.
    Untracked,
    /// Both sides changed it during a merge.
    Conflicted,
}

impl ChangeKind {
    /// One-letter code, as `git status --short` shows it except `U` for untracked (git's `?`).
    pub fn letter(self) -> &'static str {
        match self {
            Self::Added => "A",
            Self::Modified => "M",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::Typechange => "T",
            Self::Untracked => "U",
            Self::Conflicted => "!",
        }
    }

    /// Word used in tooltips.
    pub fn label(self) -> &'static str {
        match self {
            Self::Added => "Added",
            Self::Modified => "Modified",
            Self::Deleted => "Deleted",
            Self::Renamed => "Renamed",
            Self::Typechange => "Type changed",
            Self::Untracked => "Untracked",
            Self::Conflicted => "Conflicted",
        }
    }
}

/// One file in a status list or a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Path relative to the repository root, `/`-separated as Git stores it.
    pub path: String,
    /// Previous path for renames.
    pub old_path: Option<String>,
    /// How it changed.
    pub kind: ChangeKind,
    /// Lines added, when known.
    pub additions: usize,
    /// Lines removed, when known.
    pub deletions: usize,
    /// The path has the `filter=lfs` attribute: Git LFS stores its content.
    pub lfs: bool,
    /// Git file mode on the old side (`0o100644`, `0o100755`, `0o120000` for a symbolic link,
    /// `0o160000` for a submodule); `None` when the old side does not exist.
    pub old_mode: Option<u32>,
    /// Git file mode on the new side; `None` when the new side does not exist.
    pub new_mode: Option<u32>,
}

impl FileEntry {
    /// The old and new mode when both sides exist and differ: an executable-bit change
    /// (`0o100644` to `0o100755`) or, with [`ChangeKind::Typechange`], a file/symbolic link swap.
    pub fn mode_change(&self) -> Option<(u32, u32)> {
        match (self.old_mode, self.new_mode) {
            (Some(old), Some(new)) if old != new => Some((old, new)),
            _ => None,
        }
    }
}

/// The working tree split the way the staging area shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkingStatus {
    /// Changes in the index relative to `HEAD`.
    pub staged: Vec<FileEntry>,
    /// Changes in the working tree relative to the index, including untracked files.
    pub unstaged: Vec<FileEntry>,
    /// Files with unresolved conflicts.
    pub conflicted: Vec<FileEntry>,
    /// Untracked folders listed as one entry each instead of file by file: every folder that
    /// holds a repository of its own, and, when there are more untracked files than
    /// [`crate::status::UNTRACKED_LIMIT`], every top-most untracked folder. Their files are not
    /// in [`Self::unstaged`].
    pub untracked_dirs: Vec<UntrackedDir>,
}

/// An untracked folder shown as a single entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UntrackedDir {
    /// Path relative to the repository root, `/`-separated, ending in `/`.
    pub path: String,
    /// Untracked, unignored files below it; 0 for a repository of its own.
    pub files: usize,
    /// The folder is a Git repository of its own (it contains `.git`) and not a submodule.
    /// `git add` would record it as a gitlink without a `.gitmodules` entry, so
    /// [`crate::Repo::stage_all_except_repositories`] leaves it out.
    pub repository: bool,
    /// A collapsed folder with such a repository somewhere below it. Staging the folder as a
    /// whole (`git add -- folder/`) would add that repository as a gitlink.
    pub contains_repository: bool,
}

/// One level of an untracked folder, from [`crate::status::untracked_children`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UntrackedListing {
    /// Untracked files directly in the folder.
    pub files: Vec<FileEntry>,
    /// Its subfolders that hold untracked files or a repository of their own.
    pub dirs: Vec<UntrackedDir>,
}

impl WorkingStatus {
    /// No changes at all.
    pub fn is_clean(&self) -> bool {
        self.staged.is_empty()
            && self.unstaged.is_empty()
            && self.conflicted.is_empty()
            && self.untracked_dirs.is_empty()
    }

    /// Total number of entries across the four lists.
    pub fn len(&self) -> usize {
        self.staged.len() + self.unstaged.len() + self.conflicted.len() + self.untracked_dirs.len()
    }

    /// Same as [`Self::is_clean`].
    pub fn is_empty(&self) -> bool {
        self.is_clean()
    }
}

/// A person and a moment, as recorded in a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    /// Name.
    pub name: String,
    /// E-mail address.
    pub email: String,
    /// Seconds since the Unix epoch.
    pub time: i64,
    /// Offset from UTC in minutes.
    pub offset_minutes: i32,
}

/// The fields of a commit the history list needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitSummary {
    /// Commit id.
    pub oid: Oid,
    /// First line of the message.
    pub summary: String,
    /// Author.
    pub author: Signature,
    /// Parent ids, first parent first.
    pub parents: Vec<Oid>,
}

/// Everything shown in the commit details pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitDetails {
    /// Commit id.
    pub oid: Oid,
    /// The whole message.
    pub message: String,
    /// Author.
    pub author: Signature,
    /// Committer.
    pub committer: Signature,
    /// Parent ids.
    pub parents: Vec<Oid>,
    /// Files changed relative to the first parent (or the empty tree for a root commit).
    pub files: Vec<FileEntry>,
    /// Whether `files` carry line counts and rename detection. True unless the commit lives in
    /// a partial clone that has not downloaded the file contents: then the list names the
    /// changed files (exact renames only) and every count is 0. See
    /// [`crate::Repo::commit_details`].
    pub stats_known: bool,
}

/// What kind of reference a label on a commit row stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RefKind {
    /// `HEAD` when detached.
    Head,
    /// The checked-out local branch.
    CurrentBranch,
    /// Another local branch.
    LocalBranch,
    /// A remote-tracking branch.
    RemoteBranch,
    /// A tag.
    Tag,
    /// A stash entry.
    Stash,
}

/// A reference decorating a commit in the graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefLabel {
    /// Display name.
    pub name: String,
    /// Kind, which decides the colour and icon.
    pub kind: RefKind,
}

/// A consistent picture of a repository, produced in one go by [`crate::Repo::snapshot`].
#[derive(Debug, Clone, Default)]
pub struct RepoSnapshot {
    /// Working directory.
    pub workdir: PathBuf,
    /// Folder name, used as the tab title.
    pub name: String,
    /// `HEAD`.
    pub head: HeadInfo,
    /// In-progress operation.
    pub state: RepoState,
    /// Local branches, current first, then by name.
    pub branches: Vec<BranchInfo>,
    /// Remote-tracking branches by remote then name.
    pub remote_branches: Vec<RemoteBranchInfo>,
    /// Remotes.
    pub remotes: Vec<RemoteInfo>,
    /// Tags, newest version-looking names first.
    pub tags: Vec<TagInfo>,
    /// Stash entries, newest first.
    pub stashes: Vec<StashInfo>,
    /// The working folders, the main one first, then by folder; only this one when no
    /// `git worktree` was added.
    pub worktrees: Vec<WorktreeInfo>,
    /// Submodules by path, from the cheap read ([`crate::Repo::submodules`] without the dirty
    /// check); empty when the repository has none.
    pub submodules: Vec<crate::SubmoduleInfo>,
    /// Working tree status.
    pub status: WorkingStatus,
    /// Commits in topological order, newest first.
    pub commits: Vec<CommitSummary>,
    /// One graph row per entry in `commits`.
    pub graph: Vec<GraphRow>,
    /// Labels per commit.
    pub refs: HashMap<Oid, Vec<RefLabel>>,
    /// More commits exist beyond `commits`.
    pub truncated: bool,
    /// Set when `status` is empty only because it could not be read: a partial clone that has
    /// not downloaded the tree of `HEAD`. History never depends on it.
    pub status_unavailable: Option<StatusUnavailable>,
}

/// Why a [`RepoSnapshot`] has no working tree status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusUnavailable {
    /// What to tell the user (git's own explanation when a download failed).
    pub message: String,
    /// Downloading the missing tree from the promisor remote is possible, so trying again
    /// ([`crate::Repo::complete_status`]) may help.
    pub can_download: bool,
}

impl StatusUnavailable {
    pub(crate) fn from_error(e: &crate::Error) -> Self {
        Self {
            message: e.to_string(),
            can_download: e.can_download(),
        }
    }
}

impl RepoSnapshot {
    /// The checked-out branch, if any.
    pub fn current_branch(&self) -> Option<&BranchInfo> {
        self.branches.iter().find(|b| b.is_head)
    }
}

/// The hex digits of an abbreviated commit id.
const SHORT_ID_LEN: usize = 7;

/// Abbreviates a commit id to seven hex digits.
pub fn short_id(oid: Oid) -> String {
    let mut s = oid.to_string();
    s.truncate(SHORT_ID_LEN);
    s
}
