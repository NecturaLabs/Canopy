//! Reading a repository: snapshots, commit details and diffs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use git2::{BranchType, Oid, Repository, Sort};

use crate::diff::{self, DiffSource, FileDiff, Whitespace};
use crate::graph;
use crate::model::*;
use crate::{Error, Result, status};

/// An open repository with a working directory.
pub struct Repo {
    git: Repository,
    workdir: PathBuf,
}

impl std::fmt::Debug for Repo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Repo")
            .field("workdir", &self.workdir)
            .finish()
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
            _ => Error::Git(e),
        })?;
        let workdir = git.workdir().ok_or(Error::Bare)?.to_path_buf();
        Ok(Self { git, workdir })
    }

    /// Creates a new repository in `path` (creating the directory) and opens it.
    pub fn init(path: &Path) -> Result<Self> {
        std::fs::create_dir_all(path)?;
        crate::cli::run(path, &["init", "-q"])?;
        Self::open(path)
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
        self.workdir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.workdir.display().to_string())
    }

    /// Where `HEAD` points.
    pub fn head(&self) -> Result<HeadInfo> {
        match self.git.head() {
            Ok(r) => Ok(HeadInfo {
                branch: if r.is_branch() {
                    Some(String::from_utf8_lossy(r.shorthand_bytes()).into_owned())
                } else {
                    None
                },
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

    fn remote_branches(&self) -> Result<Vec<RemoteBranchInfo>> {
        let mut out = Vec::new();
        for item in self.git.branches(Some(BranchType::Remote))? {
            let (branch, _) = item?;
            let reference = branch.get();
            if reference.symbolic_target_bytes().is_some() {
                continue; // origin/HEAD
            }
            let Ok(full) = reference.name() else { continue };
            let remote = match self.git.branch_remote_name(full) {
                Ok(buf) => buf.as_str().unwrap_or_default().to_owned(),
                Err(_) => continue,
            };
            let short = full.strip_prefix("refs/remotes/").unwrap_or(full);
            let name = short
                .strip_prefix(&format!("{remote}/"))
                .unwrap_or(short)
                .to_owned();
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
            let time = commit.as_ref().map(|c| c.time().seconds()).unwrap_or(0);
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

    fn stashes(&self) -> Result<Vec<StashInfo>> {
        let reflog = match self.git.reflog("refs/stash") {
            Ok(r) => r,
            Err(_) => return Ok(Vec::new()),
        };
        Ok(reflog
            .iter()
            .enumerate()
            .map(|(index, entry)| StashInfo {
                index,
                message: entry.message().ok().flatten().unwrap_or("").to_owned(),
                oid: entry.id_new(),
            })
            .collect())
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
        let mut out = Vec::with_capacity(max_commits.min(4096));
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

    /// Gathers everything the main window shows, in one consistent read.
    pub fn snapshot(&self, max_commits: usize) -> Result<RepoSnapshot> {
        let head = self.head()?;
        let branches = self.branches()?;
        let remote_branches = self.remote_branches()?;
        let tags = self.tags()?;
        let stashes = self.stashes()?;
        let (commits, truncated) = self.commits(max_commits)?;
        let graph = graph::layout(commits.iter().map(|c| (c.oid, c.parents.as_slice())));

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
            status: status::collect(&self.git)?,
            commits,
            graph,
            refs,
            truncated,
        })
    }

    /// Full message, signatures and changed files of one commit.
    pub fn commit_details(&self, oid: Oid) -> Result<CommitDetails> {
        let commit = self.git.find_commit(oid)?;
        let tree = commit.tree()?;
        let parent_tree = commit.parents().next().map(|p| p.tree()).transpose()?;
        let mut opts = git2::DiffOptions::new();
        opts.context_lines(0).include_typechange(true);
        let mut d =
            self.git
                .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut opts))?;
        let mut find = git2::DiffFindOptions::new();
        find.renames(true);
        d.find_similar(Some(&mut find))?;

        let mut files = Vec::with_capacity(d.deltas().len());
        for idx in 0..d.deltas().len() {
            let Some(delta) = d.get_delta(idx) else {
                continue;
            };
            let Some(kind) = diff::delta_kind(delta.status()) else {
                continue;
            };
            let path = diff::path_of(delta.new_file()).or_else(|| diff::path_of(delta.old_file()));
            let Some(path) = path else { continue };
            let old_path = (kind == ChangeKind::Renamed)
                .then(|| diff::path_of(delta.old_file()))
                .flatten();
            let (additions, deletions) = match git2::Patch::from_diff(&d, idx)? {
                Some(p) => p.line_stats().map(|(_, a, r)| (a, r)).unwrap_or((0, 0)),
                None => (0, 0),
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
        let mut opts = diff::options(path, old_path, context, whitespace);
        let d = match source {
            DiffSource::Unstaged => self.git.diff_index_to_workdir(None, Some(&mut opts))?,
            DiffSource::Staged => {
                let head = self.git.head().ok().and_then(|h| h.peel_to_tree().ok());
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

    /// The diff of one file with `context` lines around each hunk. `whitespace` hides
    /// whitespace-only changes; such a diff is for reading only, since its hunks cannot be applied
    /// as patches.
    pub fn file_diff(
        &self,
        source: &DiffSource,
        path: &str,
        old_path: Option<&str>,
        context: u32,
        whitespace: Whitespace,
    ) -> Result<FileDiff> {
        let d = self.raw_diff(source, path, old_path, context, whitespace)?;
        let mut out = diff::to_file_diff(&d, path)?;
        out.submodule = crate::submodule::change_in(self, &d);
        Ok(out)
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
