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

/// What the other branches are compared with, and the long-lived branches that are never
/// offered for deletion.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BranchBase {
    /// The revisions a branch may be merged into, as [`Repo::merged_branches`] accepts them: the
    /// base branch and its remote-tracking branch (`main`, `origin/main`), so a merge made here
    /// and one made on the server both count. The first is the one the user sees.
    pub references: Vec<String>,
    /// Local branches that stay: git-flow's production and development branches, the remote's
    /// default branch and `main`, `master` and `develop`.
    pub protected: Vec<String>,
}

impl BranchBase {
    /// The base as the user sees it.
    pub fn shown(&self) -> &str {
        self.references.first().map_or("", String::as_str)
    }
}

/// The names kept whatever the repository says: the usual long-lived branches.
pub(crate) const LONG_LIVED: [&str; 3] = ["main", "master", "develop"];

/// What [`Repo::remove_merged_branches`] did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MergedRemoval {
    /// Branches that were deleted.
    pub removed: Vec<String>,
    /// Branches git refused to delete, with its message.
    pub failed: Vec<(String, String)>,
}

impl MergedRemoval {
    /// A sentence for a notification: the count deleted and, for each refusal, git's own words.
    pub fn summary(&self) -> String {
        let n = self.removed.len();
        let deleted = format!("Deleted {n} {}", if n == 1 { "branch" } else { "branches" });
        let refused = self
            .failed
            .iter()
            .map(|(name, why)| format!("\nCould not delete {name}: {why}"));
        std::iter::once(deleted).chain(refused).collect()
    }
}

impl Repo {
    /// What merged-ness is measured against: git-flow's development branch when it is
    /// initialised, else the remote's default branch (`origin` first), each with its
    /// remote-tracking branch where there is one; without a remote default, a local `main` or
    /// `master`. `None` when none exists.
    pub fn branch_base(&self) -> Result<Option<BranchBase>> {
        let git = self.git();
        let flow = self
            .flow_config()?
            .filter(|f| self.local_branch_tip(&f.develop).is_some());
        let remote_default = self.default_remote_branch();
        let remote = remote_default.as_ref().map(|(r, _)| r.as_str());
        let name = match (&flow, &remote_default) {
            (Some(flow), _) => Some(flow.develop.clone()),
            (None, Some((_, branch))) => Some(branch.clone()),
            (None, None) => None,
        };
        let mut references = Vec::new();
        let local = name
            .clone()
            .filter(|n| self.local_branch_tip(n).is_some())
            .or_else(|| {
                ["main", "master"]
                    .into_iter()
                    .find(|n| self.local_branch_tip(n).is_some())
                    .map(str::to_owned)
                    .filter(|_| flow.is_none())
            });
        references.extend(local);
        if let (Some(remote), Some(name)) = (remote, &name) {
            let tracking = format!("{remote}/{name}");
            if git
                .find_reference(&format!("refs/remotes/{tracking}"))
                .is_ok()
            {
                references.push(tracking);
            }
        }
        if references.is_empty() {
            return Ok(None);
        }
        let mut protected: Vec<String> = flow
            .iter()
            .flat_map(|f| [f.master.clone(), f.develop.clone()])
            .chain(remote_default.map(|(_, branch)| branch))
            .chain(LONG_LIVED.map(str::to_owned))
            .collect();
        protected.sort();
        protected.dedup();
        Ok(Some(BranchBase {
            references,
            protected,
        }))
    }

    /// Where `branch` takes its latest changes from: for a git-flow branch the kind's base
    /// (hotfix from production, the rest from development), otherwise the default branch of
    /// the remote its upstream is on (`origin` without one), as that remote-tracking branch,
    /// else a local `main` or `master`. `None` when there is none, or when the branch is
    /// itself that base (a pull brings that in).
    pub fn update_base(&self, branch: &str) -> Result<Option<String>> {
        let git = self.git();
        let flow = self.flow_config()?;
        let from_flow = flow
            .as_ref()
            .and_then(|f| f.classify(branch).map(|(kind, _)| f.default_base(kind)))
            .filter(|base| self.local_branch_tip(base).is_some())
            .map(str::to_owned);
        let upstream = git
            .find_branch(branch, BranchType::Local)
            .ok()
            .and_then(|b| b.upstream().ok())
            .and_then(|u| u.name().ok().flatten().map(str::to_owned));
        let remote = upstream
            .as_deref()
            .and_then(|u| u.split_once('/'))
            .map(|(remote, _)| remote.to_owned());
        let from_remote = || {
            let (remote, default) = match remote {
                Some(remote) => {
                    let default = self.remote_default_branch(&remote)?;
                    (remote, default)
                }
                None => self.default_remote_branch()?,
            };
            let tracking = format!("{remote}/{default}");
            git.find_reference(&format!("refs/remotes/{tracking}"))
                .ok()
                .map(|_| tracking)
        };
        let from_local = || {
            ["main", "master"]
                .into_iter()
                .find(|n| self.local_branch_tip(n).is_some())
                .map(str::to_owned)
        };
        let base = from_flow.or_else(from_remote).or_else(from_local);
        Ok(base.filter(|b| b != branch && upstream.as_deref() != Some(b.as_str())))
    }

    /// Deletes the local branches `names`, each of which must still be merged into `base` and
    /// must not be the checked-out branch or a protected one ([`Repo::branch_base`]). Everything
    /// is checked, again, before the first deletion. A branch proven merged goes with
    /// `git branch -d`; one that only looks merged (upstream gone, squash-merged) needs
    /// `allow_unproven`, the user's confirmation, and goes with `-D`. Git refusing one branch
    /// (`-d` checks the upstream or `HEAD`, not `base`) does not stop the others: it is reported
    /// in [`MergedRemoval::failed`].
    pub fn remove_merged_branches(
        &self,
        bases: &[String],
        names: &[String],
        allow_unproven: bool,
    ) -> Result<MergedRemoval> {
        let protected = self.branch_base()?.map(|b| b.protected).unwrap_or_default();
        let shown = bases.first().map_or("the base", String::as_str);
        let fresh = self.merged_branches_among(bases, true)?;
        let mut plan = Vec::new();
        for name in names {
            let Some(found) = fresh.iter().find(|b| &b.name == name) else {
                return Err(Error::invalid(format!(
                    "'{name}' is not a local branch that can be removed here."
                )));
            };
            if found.is_head {
                return Err(Error::invalid(format!(
                    "'{name}' is checked out; switch to another branch first."
                )));
            }
            if protected.contains(name) {
                return Err(Error::invalid(format!(
                    "'{name}' is a long-lived branch and is never removed here."
                )));
            }
            let Some(reason) = found.reason else {
                return Err(Error::invalid(format!(
                    "'{name}' is no longer merged into {shown}."
                )));
            };
            if !reason.is_proof() && !allow_unproven {
                return Err(Error::invalid(format!(
                    "'{name}' only looks merged ({}); deleting it needs your confirmation.",
                    reason.label()
                )));
            }
            plan.push((name, reason.is_proof()));
        }
        let mut out = MergedRemoval::default();
        for (name, proven) in plan {
            let done = if proven {
                self.delete_merged_branch(name)
            } else {
                self.force_delete_branch(name)
            };
            match done {
                Ok(()) => out.removed.push(name.clone()),
                Err(e) => out.failed.push((name.clone(), e.to_string())),
            }
        }
        Ok(out)
    }

    /// [`Repo::merged_branches`] against several bases at once (the base branch and its
    /// remote-tracking branch): a branch counts as merged when any of them says so, and a proof
    /// wins over a hint. The first base's own branch is left out, as there. Run it off the UI
    /// thread.
    pub fn merged_branches_among(
        &self,
        bases: &[String],
        squash: bool,
    ) -> Result<Vec<MergedBranch>> {
        let Some((first, rest)) = bases.split_first() else {
            return Err(Error::invalid("There is no base to compare with."));
        };
        let mut rows = self.merged_branches(first, squash)?;
        for base in rest {
            let more = self.merged_branches(base, squash)?;
            for row in &mut rows {
                let found = more
                    .iter()
                    .find(|m| m.name == row.name)
                    .and_then(|m| m.reason);
                let better = match (row.reason, found) {
                    (None, found) => found,
                    (Some(have), Some(found)) if !have.is_proof() && found.is_proof() => {
                        Some(found)
                    }
                    (have, _) => have,
                };
                row.reason = better;
            }
        }
        Ok(rows)
    }

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
    pub(crate) fn upstream_gone(&self, full: &str) -> bool {
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
