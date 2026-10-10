//! Which local branches are merged into a base branch, locally and without the network, and
//! which check proved it.

use git2::BranchType;

use crate::flow::{MergeTree, merge_tree};
use crate::{Error, Repo, Result, cli};

/// Why a branch counts as merged, in the order the checks run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MergedReason {
    /// The branch's tip is reachable from the base: `git branch -d` deletes it safely.
    Reachable,
    /// The branch has an upstream configured, but its remote-tracking branch is gone (deleted on
    /// the remote and pruned). A hint, not proof: deleting it needs `-D` and a confirmation.
    UpstreamGone,
    /// A squash/rebase merge hint: either `merge-tree` leaves the base tree unchanged, or
    /// repository metrics find every changed topic path already matches the base. Neither
    /// proves a merge occurred; deleting it needs `-D` and a confirmation.
    SquashMerged,
}

impl MergedReason {
    /// Tooltip text naming the check.
    pub fn label(self) -> &'static str {
        match self {
            Self::Reachable => "Merged: its last commit is part of the base",
            Self::UpstreamGone => "Its upstream branch was deleted on the remote",
            Self::SquashMerged => "Squash- or rebase-merged: merging it would change nothing",
        }
    }

    /// Whether the branch is provably merged, so `git branch -d` is enough.
    pub fn is_proof(self) -> bool {
        self == Self::Reachable
    }
}

/// One local branch compared with the base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedBranch {
    /// Short branch name.
    pub name: String,
    /// The branch is checked out.
    pub is_head: bool,
    /// The first check that matched; `None` when the branch is not merged.
    pub reason: Option<MergedReason>,
}

impl Repo {
    /// Compares every local branch except `base` itself with `base` (a branch, remote-tracking
    /// branch or any revision): first whether its tip is reachable from the base, then whether
    /// its upstream is gone, then, when `squash` is set and git is 2.38 or later, whether merging
    /// it would leave the base's tree unchanged. The squash check runs `git merge-tree` once per
    /// remaining branch, so run this off the UI thread. Branches come sorted by name.
    pub fn merged_branches(&self, base: &str, squash: bool) -> Result<Vec<MergedBranch>> {
        if base.starts_with('-') {
            return Err(Error::invalid(format!("'{base}' is not a valid base.")));
        }
        let git = self.git();
        let base_commit = git
            .revparse_single(base)
            .and_then(|o| o.peel_to_commit())
            .map_err(|e| Error::invalid(format!("Could not resolve the base '{base}': {e}")))?;
        let base_id = base_commit.id();
        let base_tree = base_commit.tree_id();
        let base_ref = git
            .resolve_reference_from_short_name(base)
            .ok()
            .and_then(|r| r.name().ok().map(str::to_owned));
        let squash = squash && cli::has_merge_tree_write_tree();

        let mut out = Vec::new();
        for item in git.branches(Some(BranchType::Local))? {
            let (branch, _) = item?;
            let Some(name) = branch.name()?.map(str::to_owned) else {
                continue;
            };
            let Ok(full) = branch.get().name().map(str::to_owned) else {
                continue;
            };
            if base_ref.as_deref() == Some(full.as_str()) {
                continue;
            }
            let Some(tip) = branch.get().target() else {
                continue;
            };
            let reachable = tip == base_id || git.graph_descendant_of(base_id, tip)?;
            let reason = if reachable {
                Some(MergedReason::Reachable)
            } else if self.upstream_gone(&full) {
                Some(MergedReason::UpstreamGone)
            } else if squash && self.squash_merged(base_id, tip, base_tree) {
                Some(MergedReason::SquashMerged)
            } else {
                None
            };
            out.push(MergedBranch {
                name,
                is_head: branch.is_head(),
                reason,
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Merging `tip` into `base` would leave `base_tree` unchanged. A merge git refuses (for
    /// example unrelated histories) counts as not merged.
    fn squash_merged(&self, base: git2::Oid, tip: git2::Oid, base_tree: git2::Oid) -> bool {
        match merge_tree(self, &base.to_string(), &tip.to_string()) {
            Ok(MergeTree::Clean(tree)) => tree == base_tree,
            Ok(MergeTree::Conflicts(_)) => false,
            Err(e) => {
                tracing::debug!(%tip, error = %e, "merge-tree check skipped");
                false
            }
        }
    }

    /// The branch `full` (`refs/heads/…`) has `branch.<name>.merge` configured but the ref it
    /// tracks does not exist.
    fn upstream_gone(&self, full: &str) -> bool {
        let git = self.git();
        let Some(short) = full.strip_prefix("refs/heads/") else {
            return false;
        };
        let configured = git
            .config()
            .ok()
            .and_then(|c| c.get_string(&format!("branch.{short}.merge")).ok())
            .is_some();
        if !configured {
            return false;
        }
        match git.branch_upstream_name(full) {
            Ok(upstream) => upstream
                .as_str()
                .is_ok_and(|u| git.find_reference(u).is_err()),
            Err(_) => false,
        }
    }
}
