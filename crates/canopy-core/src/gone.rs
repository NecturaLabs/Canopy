//! Local branches whose upstream was deleted on the remote: after a fetch that prunes, the
//! merged ones are deleted with `git branch -d`, and the rest keep their "gone" marker.

use std::collections::BTreeSet;

use git2::BranchType;

use crate::merged::LONG_LIVED;
use crate::{Repo, Result};

/// How many branch names a notice lists before it counts the rest.
const LISTED: usize = 3;

/// A local branch deleted because the remote deleted its upstream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoneBranch {
    /// Short branch name.
    pub name: String,
    /// The remote its upstream was on.
    pub remote: String,
}

/// What [`Repo::remove_gone_branches`] deleted.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GoneRemoval {
    /// The deleted branches, by name.
    pub removed: Vec<GoneBranch>,
}

impl GoneRemoval {
    /// A notice's title: how many branches went and where they were deleted ("Removed 2
    /// branches deleted on origin").
    pub fn title(&self) -> String {
        let n = self.removed.len();
        let noun = if n == 1 { "branch" } else { "branches" };
        let remotes: BTreeSet<&str> = self.removed.iter().map(|b| b.remote.as_str()).collect();
        match remotes.iter().next() {
            Some(remote) if remotes.len() == 1 => {
                format!("Removed {n} {noun} deleted on {remote}")
            }
            _ => format!("Removed {n} {noun} deleted on their remotes"),
        }
    }

    /// The branches' names, the first few of them and a count of the rest ("a, b, c and 2
    /// more").
    pub fn names(&self) -> String {
        let shown: Vec<&str> = self
            .removed
            .iter()
            .take(LISTED)
            .map(|b| b.name.as_str())
            .collect();
        let rest = self.removed.len().saturating_sub(LISTED);
        let list = shown.join(", ");
        if rest == 0 {
            list
        } else {
            format!("{list} and {rest} more")
        }
    }
}

impl Repo {
    /// After a fetch of `remote` (every remote when `None`) that asked git to prune as `prune`
    /// says, deletes each local branch whose upstream that fetch found deleted on the remote,
    /// with `git branch -d`: git refuses one that is not merged into `HEAD`, so no work is
    /// lost, and a refused branch keeps its "gone" marker. Never touches the checked-out
    /// branch, a branch checked out in another worktree, a long-lived branch (`main`, `master`,
    /// `develop`, git-flow's and each remote's default branch) or one tracking a local branch.
    ///
    /// Does nothing when [`crate::GitOptions::remove_gone_branches`] is off, or when the fetch
    /// did not prune the branch's remote: where Canopy follows the git configuration,
    /// `remote.<name>.prune` and `fetch.prune` decide that as they do for git.
    pub fn remove_gone_branches(&self, remote: Option<&str>, prune: bool) -> Result<GoneRemoval> {
        let mut out = GoneRemoval::default();
        if !self.options().remove_gone_branches {
            return Ok(out);
        }
        let candidates = self.gone_candidates(remote)?;
        if candidates.is_empty() {
            return Ok(out);
        }
        let mut protected: BTreeSet<String> = LONG_LIVED.map(str::to_owned).into();
        protected.extend(self.branch_base()?.map(|b| b.protected).unwrap_or_default());
        let remotes: BTreeSet<&str> = candidates.iter().map(|b| b.remote.as_str()).collect();
        let pruned: BTreeSet<&str> = remotes
            .into_iter()
            .filter(|r| self.fetch_prunes(r, prune))
            .collect();
        protected.extend(pruned.iter().filter_map(|r| self.remote_default_branch(r)));
        for branch in &candidates {
            if !pruned.contains(branch.remote.as_str()) || protected.contains(&branch.name) {
                continue;
            }
            match self.delete_merged_branch(&branch.name) {
                Ok(()) => out.removed.push(branch.clone()),
                Err(e) => tracing::debug!(
                    branch = %branch.name,
                    error = %e,
                    "kept a branch whose upstream is gone"
                ),
            }
        }
        Ok(out)
    }

    /// The local branches, by name, whose upstream on `remote` (any configured remote when
    /// `None`) no longer exists, leaving out every branch checked out in a worktree.
    fn gone_candidates(&self, remote: Option<&str>) -> Result<Vec<GoneBranch>> {
        let git = self.git();
        let configured = git.remotes()?;
        let configured: BTreeSet<&str> = configured.iter().flatten().flatten().collect();
        let in_use: BTreeSet<String> = self
            .worktrees()
            .into_iter()
            .filter_map(|w| w.branch)
            .collect();
        let mut out = Vec::new();
        for item in git.branches(Some(BranchType::Local))? {
            let (branch, _) = item?;
            let Some(name) = branch.name()?.map(str::to_owned) else {
                continue;
            };
            let Ok(full) = branch.get().name() else {
                continue;
            };
            if branch.is_head() || in_use.contains(&name) || !self.upstream_gone(full) {
                continue;
            }
            let Some(on) = git
                .branch_upstream_remote(full)
                .ok()
                .and_then(|r| r.as_str().ok().map(str::to_owned))
            else {
                continue;
            };
            if !configured.contains(on.as_str()) || remote.is_some_and(|r| r != on) {
                continue;
            }
            out.push(GoneBranch { name, remote: on });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Whether a fetch of `remote` that Canopy asked to prune as `prune` did prune it. While
    /// Canopy follows the git configuration, its choice only stands in for an unset
    /// `fetch.prune` ([`Repo::prune_args`]), and `remote.<name>.prune` wins over both, as git
    /// reads them; otherwise Canopy's `--prune` or `--no-prune` decided. A value git cannot read
    /// counts as not pruned.
    fn fetch_prunes(&self, remote: &str, prune: bool) -> bool {
        if !self.options().follow_config {
            return prune;
        }
        for key in [format!("remote.{remote}.prune"), "fetch.prune".to_owned()] {
            match crate::config::read_bool(self.workdir(), false, &key) {
                Ok(Some(entry)) => return entry.value.as_deref() == Some("true"),
                Ok(None) => {}
                Err(e) => {
                    tracing::debug!(%key, error = %e, "prune setting not read");
                    return false;
                }
            }
        }
        prune
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn removal(branches: &[(&str, &str)]) -> GoneRemoval {
        GoneRemoval {
            removed: branches
                .iter()
                .map(|(name, remote)| GoneBranch {
                    name: (*name).to_owned(),
                    remote: (*remote).to_owned(),
                })
                .collect(),
        }
    }

    #[test]
    fn a_notice_names_the_remote_and_the_first_few_branches() {
        let one = removal(&[("fix/a", "origin")]);
        assert_eq!(one.title(), "Removed 1 branch deleted on origin");
        assert_eq!(one.names(), "fix/a");

        let two = removal(&[("a", "origin"), ("b", "origin")]);
        assert_eq!(two.title(), "Removed 2 branches deleted on origin");
        assert_eq!(two.names(), "a, b");

        let five = removal(&[
            ("a", "origin"),
            ("b", "origin"),
            ("c", "origin"),
            ("d", "origin"),
            ("e", "origin"),
        ]);
        assert_eq!(five.names(), "a, b, c and 2 more");

        let mixed = removal(&[("a", "origin"), ("b", "upstream")]);
        assert_eq!(mixed.title(), "Removed 2 branches deleted on their remotes");
    }
}
