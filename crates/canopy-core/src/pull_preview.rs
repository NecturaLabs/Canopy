//! What a pull would do, worked out before it changes anything: the branch's upstream is fetched,
//! then the two are compared in memory, so the user can decide before a pull stops on conflicts.

use std::collections::BTreeSet;

use git2::{BranchType, Oid, StatusOptions};

use crate::flow::{MergeTree, merge_tree};
use crate::{Repo, Result, cli};

/// What pulling the current branch would bring and where it would conflict.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PullPreview {
    /// The upstream, as `origin/main`.
    pub upstream: String,
    /// Commits on the branch that the upstream does not have.
    pub ahead: usize,
    /// Commits on the upstream that the branch does not have.
    pub behind: usize,
    /// Files where bringing the two together would conflict, worked out only when both moved on;
    /// `None` when this git cannot tell (`merge-tree --write-tree` needs git 2.38).
    pub conflicts: Option<Vec<String>>,
    /// Files with uncommitted changes (untracked ones included) that the incoming commits change
    /// too: putting those changes back after the pull may conflict, or git may refuse to start.
    pub overlap: Vec<String>,
}

impl PullPreview {
    /// Both sides have commits of their own: only a merge or a rebase brings them together.
    pub fn diverged(&self) -> bool {
        self.ahead > 0 && self.behind > 0
    }

    /// Whether the pull is likely to stop on conflicts.
    pub fn risky(&self) -> bool {
        !self.overlap.is_empty() || self.conflicts.as_ref().is_some_and(|c| !c.is_empty())
    }
}

impl Repo {
    /// Fetches the current branch's upstream (pruning when `prune` says so) and works out what a
    /// pull would bring ([`PullPreview`]) without touching the branch, the index or any file.
    /// `None` when the branch has no upstream (or `HEAD` is detached): a pull then only reports
    /// git's own error.
    pub fn pull_preview(
        &self,
        prune: bool,
        progress: impl FnMut(&str),
    ) -> Result<Option<PullPreview>> {
        let git = self.git();
        let Ok(head) = git.head() else {
            return Ok(None);
        };
        let (Some(name), Some(head_ref)) = (head.shorthand().ok(), head.name().ok()) else {
            return Ok(None);
        };
        if !head.is_branch() {
            return Ok(None);
        }
        let Ok(upstream) = git
            .find_branch(name, BranchType::Local)
            .and_then(|b| b.upstream())
        else {
            return Ok(None);
        };
        let Some(upstream_ref) = upstream.get().name().ok().map(str::to_owned) else {
            return Ok(None);
        };
        let shown = upstream.name().ok().flatten().unwrap_or(name).to_owned();
        let remote = git
            .branch_upstream_remote(head_ref)
            .ok()
            .and_then(|r| r.as_str().ok().map(str::to_owned));
        // An upstream in this repository (remote ".") has nothing to fetch.
        if let Some(remote) = remote.filter(|r| r != ".") {
            self.fetch(Some(&remote), prune, progress)?;
        }
        let ours = git.head()?.peel_to_commit()?.id();
        let theirs = git.refname_to_id(&upstream_ref)?;
        let (ahead, behind) = git.graph_ahead_behind(ours, theirs)?;
        let mut preview = PullPreview {
            upstream: shown,
            ahead,
            behind,
            ..PullPreview::default()
        };
        if behind == 0 {
            return Ok(Some(preview));
        }
        let base = git.merge_base(ours, theirs)?;
        let incoming = self.changed_between(base, theirs)?;
        preview.overlap = self
            .uncommitted_paths()?
            .intersection(&incoming)
            .cloned()
            .collect();
        if ahead > 0 {
            preview.conflicts = if cli::has_merge_tree_write_tree() {
                match merge_tree(self, &ours.to_string(), &theirs.to_string())? {
                    MergeTree::Clean(_) => Some(Vec::new()),
                    MergeTree::Conflicts(paths) => Some(paths),
                }
            } else {
                None
            };
        }
        Ok(Some(preview))
    }

    /// Every path `to` changes since `from` (both sides of a rename).
    fn changed_between(&self, from: Oid, to: Oid) -> Result<BTreeSet<String>> {
        let git = self.git();
        let old = git.find_commit(from)?.tree()?;
        let new = git.find_commit(to)?.tree()?;
        let diff = git.diff_tree_to_tree(Some(&old), Some(&new), None)?;
        Ok(diff
            .deltas()
            .flat_map(|d| [d.old_file().path(), d.new_file().path()])
            .flatten()
            .map(|p| p.to_string_lossy().into_owned())
            .collect())
    }

    /// Every path with uncommitted changes, staged or not, untracked files included.
    fn uncommitted_paths(&self) -> Result<BTreeSet<String>> {
        let mut options = StatusOptions::new();
        options
            .include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_ignored(false);
        Ok(self
            .git()
            .statuses(Some(&mut options))?
            .iter()
            .map(|e| String::from_utf8_lossy(e.path_bytes()).into_owned())
            .collect())
    }
}
