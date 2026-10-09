//! Reading a repository: snapshots, commit details and diffs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use git2::{BranchType, Oid, Repository, Sort};

use crate::diff::{self, DiffSource, FileDiff, Whitespace};
use crate::graph;
use crate::model::{
    BranchInfo, ChangeKind, CommitDetails, CommitSummary, FileEntry, HeadInfo, RefKind, RefLabel,
    RemoteBranchInfo, RemoteInfo, RepoSnapshot, RepoState, Signature, StashInfo, StatusUnavailable,
    TagInfo, UntrackedListing, WorkingStatus, WorktreeInfo,
};
use crate::options::GitOptions;
use crate::partial::{self, MissingObjects};
use crate::{Error, Result, status};

/// A commit that changes more files than this is not worth downloading every blob for in a
/// partial clone: [`Repo::commit_details`] returns its file list without line counts.
pub const MAX_STAT_FILES: usize = 200;
/// The most commits a history read reserves room for up front, however many it may return.
const MAX_PREALLOCATED_COMMITS: usize = 4096;

/// An open repository with a working directory.
pub struct Repo {
    git: Repository,
    workdir: PathBuf,
    /// How operations have git behave ([`Repo::set_options`]).
    options: std::cell::Cell<GitOptions>,
}

impl std::fmt::Debug for Repo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Repo")
            .field("workdir", &self.workdir)
            .finish_non_exhaustive()
    }
}

fn signature(s: &git2::Signature<'_>) -> Signature {
    Signature {
        name: String::from_utf8_lossy(s.name_bytes()).into_owned(),
        email: String::from_utf8_lossy(s.email_bytes()).into_owned(),
        time: s.when().seconds(),
        offset_minutes: s.when().offset_minutes(),
    }
}

impl Repo {
    /// Opens the repository containing `path`, searching parent directories like `git` does.
    pub fn open(path: &Path) -> Result<Self> {
        let git = Repository::discover(path).map_err(|e| match e.code() {
            git2::ErrorCode::NotFound => Error::NotARepository(path.to_path_buf()),
            _ => match partial::legacy_extension(path) {
                Some(remote) => Error::LegacyPartialClone { remote },
                None => Error::Git(e),
            },
        })?;
        let workdir = git.workdir().ok_or(Error::Bare)?.to_path_buf();
        Ok(Self {
            git,
            workdir,
            options: std::cell::Cell::default(),
        })
    }

    /// Creates a new repository in `path` (creating the directory) and opens it. Its first
    /// branch is `initial_branch`, unless `follow_config` and `init.defaultBranch` names one. A
    /// name git would not accept as a branch is refused before anything is created.
    pub fn init(path: &Path, initial_branch: &str, follow_config: bool) -> Result<Self> {
        // Where git can run before `path` exists: its nearest folder that does.
        let existing = path
            .ancestors()
            .find(|p| p.is_dir())
            .unwrap_or_else(|| Path::new("."));
        let configured = follow_config
            && crate::cli::run_unchecked(existing, &["config", "--get", "init.defaultBranch"])
                .is_ok_and(|o| o.code == Some(0));
        if !configured {
            // Not `--branch`: that expands `@{-1}` and `@{upstream}` against whichever repository
            // `existing` is in, and `git init` refuses such a name only after creating the folder.
            let ref_name = format!("refs/heads/{initial_branch}");
            let valid = !initial_branch.starts_with('-')
                && initial_branch != "HEAD"
                && crate::cli::run_unchecked(existing, &["check-ref-format", &ref_name])
                    .is_ok_and(|o| o.code == Some(0));
            if !valid {
                return Err(Error::invalid(format!(
                    "'{initial_branch}' is not a valid branch name"
                )));
            }
        }
        std::fs::create_dir_all(path)?;
        if configured {
            crate::cli::run(path, &["init", "-q"])?;
        } else if crate::cli::version_number()
            .is_some_and(|v| v >= crate::cli::INITIAL_BRANCH_SINCE)
        {
            let branch = format!("--initial-branch={initial_branch}");
            crate::cli::run(path, &["init", "-q", &branch])?;
        } else {
            // `--initial-branch` is git 2.28's; an older git names the unborn branch afterwards.
            crate::cli::run(path, &["init", "-q"])?;
            let head = format!("refs/heads/{initial_branch}");
            crate::cli::run(path, &["symbolic-ref", "HEAD", &head])?;
        }
        Self::open(path)
    }

    /// Sets how the operations that follow have git behave.
    pub fn set_options(&self, options: GitOptions) {
        self.options.set(options);
    }

    /// How operations have git behave.
    pub fn options(&self) -> GitOptions {
        self.options.get()
    }

    /// The working directory.
    pub fn workdir(&self) -> &Path {
        &self.workdir
    }

    /// The underlying libgit2 handle, for reads not covered here.
    pub fn git(&self) -> &Repository {
        &self.git
    }

    /// The folder name of the working directory.
    pub fn name(&self) -> String {
        self.workdir.file_name().map_or_else(
            || self.workdir.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        )
    }

    /// Where `HEAD` points.
    pub fn head(&self) -> Result<HeadInfo> {
        match self.git.head() {
            Ok(r) => Ok(HeadInfo {
                branch: r
                    .is_branch()
                    .then(|| String::from_utf8_lossy(r.shorthand_bytes()).into_owned()),
                oid: r.peel_to_commit().ok().map(|c| c.id()),
                detached: self.git.head_detached().unwrap_or(false),
            }),
            Err(e) if e.code() == git2::ErrorCode::UnbornBranch => {
                let head = self.git.find_reference("HEAD")?;
                let branch = head.symbolic_target_bytes().map(|t| {
                    let t = String::from_utf8_lossy(t);
                    t.strip_prefix("refs/heads/").unwrap_or(&t).to_owned()
                });
                Ok(HeadInfo {
                    branch,
                    oid: None,
                    detached: false,
                })
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Whether putting back the changes stashed around a pull or rebase (`--autostash`) ran into
    /// conflicts: files are conflicted while no merge, rebase or other operation is in progress.
    /// Git keeps those changes in the stash list as well until the stash is dropped.
    pub fn autostash_conflicted(&self) -> Result<bool> {
        Ok(self.state() == RepoState::Clean && self.index_conflicted()?)
    }

    /// Whether a merge or rebase (a pull's, say) stopped with conflicts to resolve.
    pub fn stopped_on_conflicts(&self) -> Result<bool> {
        Ok(matches!(self.state(), RepoState::Merge | RepoState::Rebase)
            && self.index_conflicted()?)
    }

    fn index_conflicted(&self) -> Result<bool> {
        // git, not libgit2, wrote the index: read it from disk again.
        let mut index = self.git.index()?;
        index.read(true)?;
        Ok(index.has_conflicts())
    }

    fn state(&self) -> RepoState {
        use git2::RepositoryState as S;
        match self.git.state() {
            S::Clean => RepoState::Clean,
            S::Merge => RepoState::Merge,
            S::Revert | S::RevertSequence => RepoState::Revert,
            S::CherryPick | S::CherryPickSequence => RepoState::CherryPick,
            S::Bisect => RepoState::Bisect,
            S::Rebase | S::RebaseInteractive | S::RebaseMerge => RepoState::Rebase,
            S::ApplyMailbox | S::ApplyMailboxOrRebase => RepoState::ApplyMailbox,
        }
    }

    /// Local branches, the checked-out one first.
    pub fn branches(&self) -> Result<Vec<BranchInfo>> {
        let mut elsewhere: HashMap<String, PathBuf> = self
            .worktrees()
            .into_iter()
            .filter(|w| !w.current)
            .filter_map(|w| Some((w.branch?, w.path)))
            .collect();
        let mut out = Vec::new();
        for item in self.git.branches(Some(BranchType::Local))? {
            let (branch, _) = item?;
            let Some(name) = branch.name()?.map(str::to_owned) else {
                continue;
            };
            let target = branch.get().target();
            let upstream_branch = branch.upstream().ok();
            let upstream = upstream_branch
                .as_ref()
                .and_then(|u| u.name().ok().flatten().map(str::to_owned));
            let (ahead, behind) = match (target, upstream_branch.and_then(|u| u.get().target())) {
                (Some(local), Some(up)) => self.git.graph_ahead_behind(local, up).unwrap_or((0, 0)),
                _ => (0, 0),
            };
            out.push(BranchInfo {
                worktree: elsewhere.remove(&name),
                name,
                target,
                is_head: branch.is_head(),
                upstream,
                ahead,
                behind,
            });
        }
        out.sort_by(|a, b| b.is_head.cmp(&a.is_head).then_with(|| a.name.cmp(&b.name)));
        Ok(out)
    }

    /// The repository's working folders: the main one first, then those `git worktree add`
    /// made, by folder. A folder that cannot be read (moved, deleted, owned by someone else) is
    /// left out; a bare repository has none.
    pub fn worktrees(&self) -> Vec<WorktreeInfo> {
        let read = |repo: &Repository, main: bool| {
            let path = repo.workdir()?.to_path_buf();
            let head = repo.find_reference("HEAD").ok()?;
            let branch = head
                .symbolic_target()
                .ok()
                .flatten()
                .and_then(|t| t.strip_prefix("refs/heads/"))
                .map(str::to_owned);
            Some(WorktreeInfo {
                path,
                branch,
                head: head.resolve().ok().and_then(|r| r.target()),
                current: repo.path() == self.git.path(),
                main,
            })
        };
        let mut out = Vec::new();
        let main = if self.git.is_worktree() {
            Repository::open(self.git.commondir()).ok()
        } else {
            None
        };
        let main = if self.git.is_worktree() {
            main.as_ref()
        } else {
            Some(&self.git)
        };
        if let Some(w) = main.and_then(|r| read(r, true)) {
            out.push(w);
        }
        let mut linked = Vec::new();
        if let Ok(names) = self.git.worktrees() {
            for name in names.iter().flatten().flatten() {
                if let Ok(worktree) = self.git.find_worktree(name)
                    && worktree.validate().is_ok()
                    && let Ok(repo) = Repository::open_from_worktree(&worktree)
                    && let Some(w) = read(&repo, false)
                {
                    linked.push(w);
                }
            }
        }
        linked.sort_by(|a, b| a.path.cmp(&b.path));
        out.extend(linked);
        out
    }

    fn remote_branches(&self) -> Result<Vec<RemoteBranchInfo>> {
        let mut out = Vec::new();
        // The remote is read off the ref name, not looked up through libgit2, which refuses a
        // remote whose configuration it cannot parse (for example `remote.<name>.prune = x`).
        let remotes = self.git.remotes()?;
        for item in self.git.branches(Some(BranchType::Remote))? {
            let (branch, _) = item?;
            let reference = branch.get();
            if reference.symbolic_target_bytes().is_some() {
                continue; // origin/HEAD
            }
            let Ok(full) = reference.name() else { continue };
            let short = full.strip_prefix("refs/remotes/").unwrap_or(full);
            let Some((remote, name)) = remotes
                .iter()
                .flatten()
                .flatten()
                .filter_map(|r| Some((r, short.strip_prefix(r)?.strip_prefix('/')?)))
                .max_by_key(|(r, _)| r.len())
                .map(|(r, n)| (r.to_owned(), n.to_owned()))
            else {
                continue;
            };
            if name == "HEAD" {
                continue;
            }
            out.push(RemoteBranchInfo {
                remote,
                branch: name,
                target: reference.target(),
            });
        }
        out.sort_by(|a, b| {
            a.remote
                .cmp(&b.remote)
                .then_with(|| a.branch.cmp(&b.branch))
        });
        Ok(out)
    }

    fn remotes(&self) -> Result<Vec<RemoteInfo>> {
        let mut out = Vec::new();
        for name in self.git.remotes()?.iter().flatten().flatten() {
            let url = self
                .git
                .find_remote(name)
                .ok()
                .and_then(|r| r.url().ok().map(str::to_owned));
            out.push(RemoteInfo {
                name: name.to_owned(),
                url,
                default_branch: self.remote_default_branch(name),
            });
        }
        Ok(out)
    }

    /// The branch `refs/remotes/<remote>/HEAD` points at (set by `git clone` and
    /// `git remote set-head`), without the `<remote>/` prefix.
    fn remote_default_branch(&self, remote: &str) -> Option<String> {
        let head = self
            .git
            .find_reference(&format!("refs/remotes/{remote}/HEAD"))
            .ok()?;
        let target = String::from_utf8_lossy(head.symbolic_target_bytes()?).into_owned();
        target
            .strip_prefix(&format!("refs/remotes/{remote}/"))
            .filter(|b| !b.is_empty())
            .map(str::to_owned)
    }

    fn tags(&self) -> Result<Vec<TagInfo>> {
        let mut raw = Vec::new();
        self.git.tag_foreach(|oid, name| {
            raw.push((oid, String::from_utf8_lossy(name).into_owned()));
            true
        })?;
        let mut out = Vec::with_capacity(raw.len());
        for (oid, full) in raw {
            let name = full.strip_prefix("refs/tags/").unwrap_or(&full).to_owned();
            let object = self.git.find_object(oid, None).ok();
            let message = object
                .as_ref()
                .and_then(|o| o.as_tag())
                .and_then(|t| t.message().ok().flatten().map(|m| m.trim().to_owned()));
            let commit = object.and_then(|o| o.peel_to_commit().ok());
            let time = commit.as_ref().map_or(0, |c| c.time().seconds());
            out.push((
                time,
                TagInfo {
                    name,
                    target: commit.map(|c| c.id()),
                    message,
                },
            ));
        }
        out.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.name.cmp(&a.1.name)));
        Ok(out.into_iter().map(|(_, t)| t).collect())
    }

    fn stashes(&self) -> Vec<StashInfo> {
        let Ok(reflog) = self.git.reflog("refs/stash") else {
            return Vec::new();
        };
        reflog
            .iter()
            .enumerate()
            .map(|(index, entry)| StashInfo {
                index,
                message: entry.message().ok().flatten().unwrap_or("").to_owned(),
                oid: entry.id_new(),
            })
            .collect()
    }

    /// Reads up to `max_commits` commits reachable from `HEAD`, branches, remote branches and
    /// tags, newest first in topological order. Returns whether more exist.
    pub fn commits(&self, max_commits: usize) -> Result<(Vec<CommitSummary>, bool)> {
        let mut walk = self.git.revwalk()?;
        walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME)?;
        let _ = walk.push_head();
        walk.push_glob("refs/heads")?;
        walk.push_glob("refs/remotes")?;
        walk.push_glob("refs/tags")?;
        let mut out = Vec::with_capacity(max_commits.min(MAX_PREALLOCATED_COMMITS));
        let mut truncated = false;
        for oid in walk {
            let oid = oid?;
            if out.len() >= max_commits {
                truncated = true;
                break;
            }
            let commit = self.git.find_commit(oid)?;
            out.push(CommitSummary {
                oid,
                summary: String::from_utf8_lossy(commit.summary_bytes().unwrap_or_default())
                    .into_owned(),
                author: signature(&commit.author()),
                parents: commit.parent_ids().collect(),
            });
        }
        Ok((out, truncated))
    }

    /// Gathers everything the main window shows, in one consistent read. Offline: in a partial
    /// clone that lacks the tree of `HEAD` (a treeless clone after a soft reset) the status is
    /// left out and [`RepoSnapshot::status_unavailable`] says so; see [`Repo::complete_status`].
    pub fn snapshot(&self, max_commits: usize) -> Result<RepoSnapshot> {
        let head = self.head()?;
        let branches = self.branches()?;
        let remote_branches = self.remote_branches()?;
        let tags = self.tags()?;
        let stashes = self.stashes();
        let worktrees = self.worktrees();
        // A broken `.gitmodules` must not hide the rest of the repository.
        let submodules = self.submodules(false).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "could not read the submodules");
            Vec::new()
        });
        let (commits, truncated) = self.commits(max_commits)?;
        let graph = graph::layout(commits.iter().map(|c| (c.oid, c.parents.as_slice())));
        // History never depends on status: a partial clone that lacks the tree of `HEAD` still
        // shows its commits, with the reason the changes are missing.
        let (status, status_unavailable) = match status::collect(&self.git) {
            Ok(s) => (s, None),
            Err(e @ Error::NotDownloaded { .. }) => (
                WorkingStatus::default(),
                Some(StatusUnavailable::from_error(&e)),
            ),
            Err(e) => return Err(e),
        };

        let mut refs: HashMap<Oid, Vec<RefLabel>> = HashMap::new();
        if head.detached
            && let Some(oid) = head.oid
        {
            refs.entry(oid).or_default().push(RefLabel {
                name: "HEAD".into(),
                kind: RefKind::Head,
            });
        }
        for b in &branches {
            if let Some(t) = b.target {
                let kind = if b.is_head {
                    RefKind::CurrentBranch
                } else {
                    RefKind::LocalBranch
                };
                refs.entry(t).or_default().push(RefLabel {
                    name: b.name.clone(),
                    kind,
                });
            }
        }
        for b in &remote_branches {
            if let Some(t) = b.target {
                refs.entry(t).or_default().push(RefLabel {
                    name: b.full_name(),
                    kind: RefKind::RemoteBranch,
                });
            }
        }
        for t in &tags {
            if let Some(target) = t.target {
                refs.entry(target).or_default().push(RefLabel {
                    name: t.name.clone(),
                    kind: RefKind::Tag,
                });
            }
        }
        for labels in refs.values_mut() {
            labels.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.name.cmp(&b.name)));
        }

        Ok(RepoSnapshot {
            workdir: self.workdir.clone(),
            name: self.name(),
            state: self.state(),
            head,
            branches,
            remote_branches,
            remotes: self.remotes()?,
            tags,
            stashes,
            worktrees,
            submodules,
            status,
            commits,
            graph,
            refs,
            truncated,
            status_unavailable,
        })
    }

    /// Fills in the status of a snapshot that has none ([`RepoSnapshot::status_unavailable`]),
    /// downloading the tree of `HEAD` from the promisor remote when `missing` allows it (network,
    /// like [`Repo::status`]). When that fails the snapshot keeps its history and carries the
    /// reason instead.
    pub fn complete_status(&self, snap: &mut RepoSnapshot, missing: MissingObjects) {
        if snap.status_unavailable.is_none() {
            return;
        }
        match self.status(missing) {
            Ok(status) => {
                snap.status = status;
                snap.status_unavailable = None;
            }
            Err(e) => snap.status_unavailable = Some(StatusUnavailable::from_error(&e)),
        }
    }

    /// Whether this repository is a partial clone (`git clone --filter=...`), whose reads may need
    /// objects that are still on the server. Reads the configuration; not cached.
    pub fn is_partial_clone(&self) -> bool {
        partial::is_partial_clone(&self.git)
    }

    /// Whether the repository lies inside one of the `trusted` folders, so that a download may
    /// run its own git configuration (see [`partial::is_in_trusted_folder`]).
    pub fn is_in_trusted_folder(&self, trusted: &[PathBuf]) -> bool {
        partial::is_in_trusted_folder(&self.git, trusted)
    }

    /// The configuration keys and hooks of the repository itself that make a fetch run a
    /// program (see [`partial::fetch_programs`]); empty when there are none.
    pub fn fetch_programs(&self) -> Vec<String> {
        partial::fetch_programs(&self.git)
    }

    /// The promisor remote a download would use first, to name in a progress message; `None` for
    /// a full clone or when no remote can be fetched from.
    pub fn promisor_remote(&self) -> Option<String> {
        partial::usable_promisor_remote(&self.git)
    }

    /// The working-tree status, as [`status::collect`] reads it (and [`Repo::snapshot`] includes).
    ///
    /// The only object status ever needs from a partial clone's remote is the tree of `HEAD`,
    /// missing in a treeless clone after a soft reset or similar. With [`MissingObjects::Fail`]
    /// that is [`Error::NotDownloaded`]; with [`MissingObjects::Download`] the tree (without
    /// blobs) is fetched through `git` (network) and status read again.
    pub fn status(&self, missing: MissingObjects) -> Result<WorkingStatus> {
        match status::collect(&self.git) {
            Err(Error::NotDownloaded { .. }) if missing == MissingObjects::Download => {
                if let Some(tree) = partial::missing_head_tree(&self.git) {
                    partial::download(&self.git, &self.workdir, &[tree])?;
                }
                status::collect(&self.git)
            }
            other => other,
        }
    }

    /// Full message, signatures and changed files of one commit.
    ///
    /// A partial clone may lack what the line counts and rename detection need. The file list
    /// itself needs only trees, so it never waits for blobs:
    ///
    /// - [`MissingObjects::Fail`] stays offline. If blobs are missing the result is the file list
    ///   (exact renames only, counts 0) with [`CommitDetails::stats_known`] false; if trees are
    ///   missing (a treeless clone) it is [`Error::NotDownloaded`].
    /// - [`MissingObjects::Download`] fetches the missing trees, then the missing blobs, from the
    ///   promisor remote through `git` (network) and returns the complete details. A commit that
    ///   changes more than [`MAX_STAT_FILES`] files is not worth that many blobs: it returns the
    ///   file list with `stats_known` false after fetching trees only.
    ///
    /// Full clones never touch the network and always have `stats_known` true.
    pub fn commit_details(&self, oid: Oid, missing: MissingObjects) -> Result<CommitDetails> {
        match self.commit_details_local(oid, true) {
            Err(Error::Git(e))
                if partial::is_missing_object(&e) && partial::is_partial_clone(&self.git) => {}
            other => return other,
        }
        let list = partial::read_with_download(
            &self.git,
            &self.workdir,
            missing,
            || self.commit_details_local(oid, false),
            || {
                let commit = self.git.find_commit(oid)?;
                Ok(partial::missing_commit_trees(&self.git, &commit))
            },
        )?;
        if missing == MissingObjects::Fail || list.files.len() > MAX_STAT_FILES {
            return Ok(list);
        }
        partial::read_with_download(
            &self.git,
            &self.workdir,
            missing,
            || self.commit_details_local(oid, true),
            || {
                let (_, d) = self.commit_tree_diff(oid, None)?;
                Ok(partial::missing_blobs(&self.git, &d, true, true))
            },
        )
    }

    /// The commit, and the raw tree-to-tree diff between its first parent (or nothing) and it.
    fn commit_tree_diff(
        &self,
        oid: Oid,
        opts: Option<&mut git2::DiffOptions>,
    ) -> Result<(git2::Commit<'_>, git2::Diff<'_>)> {
        let commit = self.git.find_commit(oid)?;
        let tree = commit.tree()?;
        let parent_tree = commit.parents().next().map(|p| p.tree()).transpose()?;
        let d = self
            .git
            .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), opts)?;
        Ok((commit, d))
    }

    /// The details of `oid` from what is on disk. With `full` the diff gets rename detection and
    /// line counts, which read blobs; without it only trees are read, and only exact renames
    /// (same blob) are detected.
    fn commit_details_local(&self, oid: Oid, full: bool) -> Result<CommitDetails> {
        let mut opts = git2::DiffOptions::new();
        opts.context_lines(0).include_typechange(true);
        let (commit, mut d) = self.commit_tree_diff(oid, Some(&mut opts))?;
        let mut find = git2::DiffFindOptions::new();
        find.renames(true);
        if full {
            d.find_similar(Some(&mut find))?;
        } else {
            find.exact_match_only(true);
            // Exact matching compares IDs and should read no blob; if libgit2 wants one anyway,
            // list the files without renames.
            if let Err(e) = d.find_similar(Some(&mut find)) {
                if !partial::is_missing_object(&e) {
                    return Err(e.into());
                }
                d = self.commit_tree_diff(oid, Some(&mut opts))?.1;
            }
        }

        let mut files = Vec::with_capacity(d.deltas().len());
        for idx in 0..d.deltas().len() {
            let Some(delta) = d.get_delta(idx) else {
                continue;
            };
            let Some(kind) = diff::delta_kind(delta.status()) else {
                continue;
            };
            let path =
                diff::path_of(&delta.new_file()).or_else(|| diff::path_of(&delta.old_file()));
            let Some(path) = path else { continue };
            let old_path = (kind == ChangeKind::Renamed)
                .then(|| diff::path_of(&delta.old_file()))
                .flatten();
            let (additions, deletions) = if full {
                match git2::Patch::from_diff(&d, idx)? {
                    Some(p) => p.line_stats().map_or((0, 0), |(_, a, r)| (a, r)),
                    None => (0, 0),
                }
            } else {
                (0, 0)
            };
            files.push(FileEntry {
                lfs: crate::lfs::is_lfs(&self.git, Path::new(&path)),
                path,
                old_path,
                kind,
                additions,
                deletions,
                old_mode: diff::mode_of(&delta.old_file()),
                new_mode: diff::mode_of(&delta.new_file()),
            });
        }

        Ok(CommitDetails {
            oid,
            message: String::from_utf8_lossy(commit.message_bytes()).into_owned(),
            author: signature(&commit.author()),
            committer: signature(&commit.committer()),
            parents: commit.parent_ids().collect(),
            files,
            stats_known: full,
        })
    }

    /// The raw libgit2 diff of one file for `source`.
    pub(crate) fn raw_diff(
        &self,
        source: &DiffSource,
        path: &str,
        old_path: Option<&str>,
        context: u32,
        whitespace: Whitespace,
    ) -> Result<git2::Diff<'_>> {
        // With core.symlinks=false, libgit2 reads symlink-mode worktree entries as plain files,
        // even when the path is a real link. Refuse those before it can read the target; the
        // ordinary files Git uses to emulate links still pass. Hunk and line staging share this.
        if matches!(source, DiffSource::Unstaged)
            && self
                .git
                .config()?
                .get_bool("core.symlinks")
                .is_ok_and(|symlinks| !symlinks)
        {
            crate::paths::safe_join(&self.workdir, path)?;
            if let Some(old) = old_path {
                crate::paths::safe_join(&self.workdir, old)?;
            }
        }
        let mut opts = diff::options(path, old_path, context, whitespace);
        let d = match source {
            DiffSource::Unstaged => self.git.diff_index_to_workdir(None, Some(&mut opts))?,
            DiffSource::Staged => {
                // No HEAD (unborn branch) diffs against nothing; a HEAD whose tree cannot be
                // read is an error, never an empty tree. (`peel_to_tree` would report a missing
                // tree as a peel failure rather than as a missing object.)
                let head = match self.git.head() {
                    Ok(h) => Some(h.peel_to_commit()?.tree()?),
                    Err(_) => None,
                };
                self.git
                    .diff_tree_to_index(head.as_ref(), None, Some(&mut opts))?
            }
            DiffSource::Commit(oid) => {
                let commit = self.git.find_commit(*oid)?;
                let tree = commit.tree()?;
                let parent = commit.parents().next().map(|p| p.tree()).transpose()?;
                let mut d =
                    self.git
                        .diff_tree_to_tree(parent.as_ref(), Some(&tree), Some(&mut opts))?;
                let mut find = git2::DiffFindOptions::new();
                find.renames(true);
                d.find_similar(Some(&mut find))?;
                d
            }
        };
        Ok(d)
    }

    /// Runs `read` (which reads the diff of `path` for `source`) and, if it fails on objects a
    /// partial clone has not downloaded, fetches them when `missing` allows it and runs `read`
    /// again (see [`Repo::file_diff`]).
    pub(crate) fn read_diff<T>(
        &self,
        source: &DiffSource,
        path: &str,
        old_path: Option<&str>,
        missing: MissingObjects,
        read: impl FnMut() -> Result<T>,
    ) -> Result<T> {
        partial::read_with_download(&self.git, &self.workdir, missing, read, || {
            self.missing_for_diff(source, path, old_path)
        })
    }

    /// The objects a diff of one file needs that a partial clone has not downloaded: first any
    /// missing tree (treeless clones), then the missing blobs of the file's old and new sides.
    fn missing_for_diff(
        &self,
        source: &DiffSource,
        path: &str,
        old_path: Option<&str>,
    ) -> Result<Vec<Oid>> {
        let mut opts = diff::options(path, old_path, 0, Whitespace::Show);
        match source {
            DiffSource::Commit(oid) => {
                let commit = self.git.find_commit(*oid)?;
                let trees = partial::missing_commit_trees(&self.git, &commit);
                if !trees.is_empty() {
                    return Ok(trees);
                }
                let tree = commit.tree()?;
                let parent = commit.parents().next().map(|p| p.tree()).transpose()?;
                let d =
                    self.git
                        .diff_tree_to_tree(parent.as_ref(), Some(&tree), Some(&mut opts))?;
                Ok(partial::missing_blobs(&self.git, &d, true, true))
            }
            DiffSource::Staged => {
                if let Some(tree) = partial::missing_head_tree(&self.git) {
                    return Ok(vec![tree]);
                }
                let head = self.git.head().ok().and_then(|h| h.peel_to_tree().ok());
                let d = self
                    .git
                    .diff_tree_to_index(head.as_ref(), None, Some(&mut opts))?;
                Ok(partial::missing_blobs(&self.git, &d, true, true))
            }
            DiffSource::Unstaged => {
                let d = self.git.diff_index_to_workdir(None, Some(&mut opts))?;
                // The new side is the working tree: file hashes, not objects to download.
                Ok(partial::missing_blobs(&self.git, &d, true, false))
            }
        }
    }

    /// The diff of one file with `context` lines around each hunk. `whitespace` hides
    /// whitespace-only changes; such a diff is for reading only, since its hunks cannot be applied
    /// as patches.
    ///
    /// In a partial clone the diff may need blobs (or, in a treeless clone, trees) that were not
    /// downloaded. With [`MissingObjects::Download`] they are fetched from the promisor remote
    /// through `git`, which is a network operation; with [`MissingObjects::Fail`] the read
    /// returns [`Error::NotDownloaded`] instead. Full clones never touch the network.
    pub fn file_diff(
        &self,
        source: &DiffSource,
        path: &str,
        old_path: Option<&str>,
        context: u32,
        whitespace: Whitespace,
        missing: MissingObjects,
    ) -> Result<FileDiff> {
        self.read_diff(source, path, old_path, missing, || {
            let d = self.raw_diff(source, path, old_path, context, whitespace)?;
            let mut out = diff::to_file_diff(&d, path)?;
            if out.kind == Some(ChangeKind::Typechange) && out.hunks.is_empty() {
                diff::typechange_hunks(self, source, &d, &mut out)?;
            }
            out.submodule = crate::submodule::change_in(self, &d);
            Ok(out)
        })
    }

    /// The working copy of a conflicted file, every line as context, so conflict markers show.
    pub fn conflict_view(&self, path: &str) -> Result<FileDiff> {
        let bytes = std::fs::read(crate::paths::safe_join(&self.workdir, path)?)?;
        let text = String::from_utf8_lossy(&bytes);
        let lines: Vec<diff::DiffLine> = text
            .lines()
            .enumerate()
            .map(|(i, l)| diff::DiffLine {
                kind: diff::LineKind::Context,
                old_no: None,
                new_no: u32::try_from(i + 1).ok(),
                text: l.to_owned(),
            })
            .collect();
        let n = u32::try_from(lines.len()).unwrap_or(u32::MAX);
        Ok(FileDiff {
            path: path.to_owned(),
            kind: Some(ChangeKind::Conflicted),
            hunks: vec![diff::Hunk {
                header: "Conflicted — resolve the markers, then stage the file".into(),
                old_start: 0,
                old_lines: 0,
                new_start: 1,
                new_lines: n,
                lines,
            }],
            ..FileDiff::default()
        })
    }

    /// One level of the untracked folder `dir`; see [`status::untracked_children`].
    pub fn untracked_children(&self, dir: &str) -> Result<UntrackedListing> {
        status::untracked_children(&self.git, dir)
    }

    /// Message of the `HEAD` commit, for amending.
    pub fn head_message(&self) -> Result<String> {
        let commit = self.git.head()?.peel_to_commit()?;
        Ok(String::from_utf8_lossy(commit.message_bytes()).into_owned())
    }

    /// Reads a configuration value (e.g. `pull.rebase`).
    pub fn config_string(&self, key: &str) -> Option<String> {
        self.git.config().ok()?.get_string(key).ok()
    }
}
