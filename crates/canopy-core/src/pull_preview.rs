//! What a pull would do, worked out before it changes anything: the branch's upstream is fetched,
//! then the two are compared in memory, so the user can decide before a pull stops on conflicts.

use std::collections::BTreeSet;

use git2::{BranchType, Oid, StatusOptions};

use crate::flow::{MergeTree, merge_tree};
use crate::{Error, Repo, Result, cli};

/// The checked-out branch's upstream.
struct Upstream {
    /// As `origin/main`.
    shown: String,
    /// As `refs/remotes/origin/main`.
    reference: String,
    /// `branch.<name>.remote`: `.` for a branch of this repository.
    remote: Option<String>,
}

/// What pulling the current branch would bring and where it would conflict.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PullPreview {
    /// The upstream, as `origin/main`.
    pub upstream: String,
    /// The upstream's commit this preview compared against. `None` only in a placeholder that
    /// did not come from a repository.
    pub tip: Option<Oid>,
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
        let Some(upstream) = self.head_upstream() else {
            return Ok(None);
        };
        // An upstream in this repository (remote ".") has nothing to fetch.
        if let Some(remote) = upstream.remote.filter(|r| r != ".") {
            self.fetch(Some(&remote), prune, progress)?;
        }
        self.compare_upstream()
    }

    /// [`Repo::pull_preview`] against the upstream as last fetched, without fetching.
    pub fn compare_upstream(&self) -> Result<Option<PullPreview>> {
        let Some(upstream) = self.head_upstream() else {
            return Ok(None);
        };
        self.compare_head_with(&upstream.reference, upstream.shown)
            .map(Some)
    }

    /// What bringing `base` (a branch, remote-tracking branch or tag) into the checked-out
    /// branch would do, as [`Repo::compare_upstream`] works it out for the upstream; the
    /// preview's `upstream` is `base` as given. Does not fetch.
    pub fn compare_base(&self, base: &str) -> Result<PullPreview> {
        if base.starts_with('-') {
            return Err(Error::invalid(format!("'{base}' is not a valid base.")));
        }
        let reference = self
            .git()
            .resolve_reference_from_short_name(base)
            .map_err(|e| Error::invalid(format!("Could not resolve the base '{base}': {e}")))?;
        let full = reference
            .name()
            .ok()
            .map(str::to_owned)
            .ok_or_else(|| Error::invalid(format!("The base '{base}' has no usable name.")))?;
        self.compare_head_with(&full, base.to_owned())
    }

    fn compare_head_with(&self, reference: &str, shown: String) -> Result<PullPreview> {
        let git = self.git();
        let ours = git.head()?.peel_to_commit()?.id();
        let theirs = git.refname_to_id(reference)?;
        let (ahead, behind) = git.graph_ahead_behind(ours, theirs)?;
        let mut preview = PullPreview {
            upstream: shown,
            tip: Some(theirs),
            ahead,
            behind,
            ..PullPreview::default()
        };
        if behind == 0 {
            return Ok(preview);
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
        Ok(preview)
    }

    /// The merge-commit subject for a checked pull, naming the upstream branch and its remote
    /// without using the remote's URL. `None` when the upstream's remote cannot be identified.
    pub(crate) fn pull_merge_message(&self) -> Option<String> {
        let upstream = self.head_upstream()?;
        match upstream.remote.as_deref() {
            Some(".") => Some(format!("Merge branch '{}'", upstream.shown)),
            Some(remote) => {
                let branch = upstream.shown.strip_prefix(remote)?.strip_prefix('/')?;
                Some(format!("Merge branch '{branch}' of {remote}"))
            }
            None => None,
        }
    }

    /// The checked-out branch's upstream; `None` without one or on a detached `HEAD`.
    fn head_upstream(&self) -> Option<Upstream> {
        let git = self.git();
        let head = git.head().ok()?;
        if !head.is_branch() {
            return None;
        }
        let (name, head_ref) = (head.shorthand().ok()?, head.name().ok()?);
        let upstream = git
            .find_branch(name, BranchType::Local)
            .and_then(|b| b.upstream())
            .ok()?;
        let reference = upstream.get().name().ok()?.to_owned();
        let shown = upstream.name().ok().flatten().unwrap_or(name).to_owned();
        let remote = git
            .branch_upstream_remote(head_ref)
            .ok()
            .and_then(|r| r.as_str().ok().map(str::to_owned));
        Some(Upstream {
            shown,
            reference,
            remote,
        })
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
