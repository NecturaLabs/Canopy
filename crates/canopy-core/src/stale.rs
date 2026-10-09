//! Stale branch and linked worktree previews with conservative removal checks.

use std::collections::HashSet;
use std::path::PathBuf;

use git2::{BranchType, Oid, Repository, StatusOptions, WorktreeLockStatus};

use crate::{Error, Repo, Result, Signature, StaleCandidate, StaleKind};

/// UTC seconds in an inactivity day.
const SECONDS_PER_DAY: i64 = 86_400;

struct LinkedWorktree {
    name: String,
    path: PathBuf,
    repo: Repository,
}

impl Repo {
    /// Previews items whose last commit predates the inactivity period. Main, current and
    /// locked worktrees, and checked-out, default and unpublished-ahead branches are excluded.
    /// Safety uncertainty protects an item or fails the read; it never permits removal.
    pub fn stale_candidates(&self, older_than_days: u32, now: i64) -> Result<Vec<StaleCandidate>> {
        let cutoff = now.saturating_sub(i64::from(older_than_days) * SECONDS_PER_DAY);
        let mut checked_out = HashSet::new();
        let main = Repository::open(self.git().commondir())?;
        let linked = self.stale_worktrees(&main, &mut checked_out)?;
        if let Some(branch) = checked_out_branch(&main)? {
            checked_out.insert(branch);
        }
        for worktree in &linked {
            if let Some(branch) = checked_out_branch(&worktree.repo)? {
                checked_out.insert(branch);
            }
        }
        let defaults = self.stale_default_branches()?;
        let mut candidates = Vec::new();
        for worktree in linked {
            if worktree.repo.path() == self.git().path() {
                continue;
            }
            let Some(commit) = head_commit(&worktree.repo)? else {
                continue;
            };
            if commit.time().seconds() >= cutoff {
                continue;
            }
            // Excluding the worktree's own branch asks whether its work is merged elsewhere.
            let exclude: Vec<_> = checked_out_branch(&worktree.repo)?
                .map(|name| format!("refs/heads/{name}"))
                .into_iter()
                .collect();
            let unmerged = self.stale_unique_commits(commit.id(), &exclude)?;
            let mut options = StatusOptions::new();
            options
                .include_untracked(true)
                .recurse_untracked_dirs(true)
                .include_ignored(true)
                .recurse_ignored_dirs(true);
            let statuses = worktree.repo.statuses(Some(&mut options));
            let dirty = statuses.as_ref().is_ok_and(|s| !s.is_empty());
            let index_protection =
                worktree_index_protection(&worktree.repo).unwrap_or_else(|error| {
                    Some(format!("Cannot check working folder index: {error}"))
                });
            let protected = if let Err(error) = statuses {
                Some(format!("Cannot check working folder: {error}"))
            } else if dirty {
                Some("Contains uncommitted or untracked files".to_owned())
            } else if index_protection.is_some() {
                index_protection
            } else if worktree.path.to_str().is_none() {
                Some("Working folder path is not UTF-8".to_owned())
            } else {
                stale_protection(unmerged, defaults.is_empty())
            };
            candidates.push(candidate(
                StaleKind::Worktree,
                worktree.name,
                Some(worktree.path),
                &commit,
                dirty,
                unmerged,
                protected,
            ));
        }
        for item in self.git().branches(Some(BranchType::Local))? {
            let (branch, _) = item?;
            let Some(name) = branch.name()?.map(str::to_owned) else {
                continue;
            };
            if branch.is_head() || defaults.contains(&name) || checked_out.contains(&name) {
                continue;
            }
            let commit = branch.get().peel_to_commit()?;
            if commit.time().seconds() >= cutoff {
                continue;
            }
            let upstream = match branch.upstream() {
                Ok(upstream) => Some(upstream.get().peel_to_commit()?.id()),
                Err(e) if e.code() == git2::ErrorCode::NotFound => None,
                Err(e) => return Err(e.into()),
            };
            if let Some(upstream) = upstream
                && self.git().graph_ahead_behind(commit.id(), upstream)?.0 > 0
            {
                continue;
            }
            let full = format!("refs/heads/{name}");
            let unmerged = self.stale_unique_commits(commit.id(), &[full])?;
            let mut protected = stale_protection(unmerged, defaults.is_empty());
            // branch -d uses upstream or HEAD, even when another branch retains every commit.
            let deletion_base = upstream.or(head_commit(self.git())?.map(|c| c.id()));
            if protected.is_none()
                && !deletion_base.is_some_and(|base| {
                    base == commit.id()
                        || self
                            .git()
                            .graph_descendant_of(base, commit.id())
                            .unwrap_or(false)
                })
            {
                protected = Some("Not merged into its upstream or the current branch".to_owned());
            }
            candidates.push(candidate(
                StaleKind::Branch,
                name,
                None,
                &commit,
                false,
                unmerged,
                protected,
            ));
        }
        candidates.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.name.cmp(&b.name)));
        Ok(candidates)
    }

    fn stale_worktrees(
        &self,
        main: &Repository,
        checked_out: &mut HashSet<String>,
    ) -> Result<Vec<LinkedWorktree>> {
        let mut linked = Vec::new();
        let names = self.git().worktrees()?;
        for name in &names {
            let name = name?.ok_or_else(|| Error::invalid("Worktree name is not UTF-8"))?;
            let worktree = self.git().find_worktree(name)?;
            if !matches!(worktree.is_locked()?, WorktreeLockStatus::Unlocked) {
                // Locked folders may be on an unplugged drive. Their administrative HEAD
                // still reserves the branch, without needing to open the working folder.
                let head = main.find_reference(&format!("worktrees/{name}/HEAD"))?;
                if let Some(branch) = head
                    .symbolic_target()?
                    .and_then(|target| target.strip_prefix("refs/heads/"))
                {
                    checked_out.insert(branch.to_owned());
                }
                continue;
            }
            linked.push(LinkedWorktree {
                name: name.to_owned(),
                path: worktree.path().to_path_buf(),
                repo: Repository::open_from_worktree(&worktree)?,
            });
        }
        Ok(linked)
    }

    fn stale_default_branches(&self) -> Result<HashSet<String>> {
        let mut defaults = HashSet::new();
        for remote in &self.git().remotes()? {
            let Some(remote) = remote? else { continue };
            match self
                .git()
                .find_reference(&format!("refs/remotes/{remote}/HEAD"))
            {
                Ok(reference) => {
                    if let Some(target) = reference.symbolic_target()? {
                        let prefix = format!("refs/remotes/{remote}/");
                        if let Some(name) = target.strip_prefix(&prefix) {
                            defaults.insert(name.to_owned());
                        }
                    }
                }
                Err(e) if e.code() == git2::ErrorCode::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        if defaults.is_empty() {
            for name in ["main", "master"] {
                if self.git().find_branch(name, BranchType::Local).is_ok() {
                    defaults.insert(name.to_owned());
                    break;
                }
            }
        }
        Ok(defaults)
    }

    /// Counts commits that would lose every branch reference after excluding `excluded`.
    pub(crate) fn stale_unique_commits(&self, tip: Oid, excluded: &[String]) -> Result<usize> {
        let mut walk = self.git().revwalk()?;
        walk.push(tip)?;
        for item in self.git().branches(None)? {
            let (branch, _) = item?;
            let reference = branch.get();
            if excluded
                .iter()
                .any(|name| reference.name().is_ok_and(|n| n == name))
            {
                continue;
            }
            walk.hide(reference.peel_to_commit()?.id())?;
        }
        walk.try_fold(0, |count, commit| commit.map(|_| count + 1))
            .map_err(Into::into)
    }

    /// Finds `preview` among the `fresh` candidates: still removable, at the same commit and
    /// identity.
    pub(crate) fn validate_stale_candidate(
        preview: &StaleCandidate,
        fresh: &[StaleCandidate],
    ) -> Result<()> {
        let current = fresh
            .iter()
            .find(|c| c.kind == preview.kind && c.name == preview.name && c.path == preview.path)
            .ok_or_else(|| {
                Error::invalid(format!(
                    "{} is no longer eligible; refresh the cleanup preview",
                    preview.name
                ))
            })?;
        if preview.tip != current.tip {
            return Err(Error::invalid(format!(
                "{} changed since the cleanup preview; refresh it",
                preview.name
            )));
        }
        if let Some(reason) = &current.protected {
            return Err(Error::invalid(format!(
                "Cannot remove {}: {reason}",
                preview.name
            )));
        }
        Ok(())
    }
}

/// Git status and even non-force worktree removal trust these flags. Refuse the worktree
/// rather than treating skipped file checks as proof that all tracked files are unchanged.
fn worktree_index_protection(repo: &Repository) -> Result<Option<String>> {
    let mut index = repo.index()?;
    index.read(true)?;
    for entry in index.iter() {
        if entry.flags & git2::IndexEntryFlag::VALID.bits() != 0 {
            return Ok(Some(
                "Assume-unchanged index flags can hide edits; clear them before removal".to_owned(),
            ));
        }
        if entry.flags_extended & git2::IndexEntryExtendedFlag::SKIP_WORKTREE.bits() != 0 {
            return Ok(Some(
                "Sparse checkout or skip-worktree flags can hide edits; restore a full checkout before removal"
                    .to_owned(),
            ));
        }
    }
    Ok(None)
}

fn checked_out_branch(repo: &Repository) -> Result<Option<String>> {
    let head = repo.find_reference("HEAD")?;
    Ok(head
        .symbolic_target()?
        .and_then(|name| name.strip_prefix("refs/heads/"))
        .map(str::to_owned))
}

fn head_commit(repo: &Repository) -> Result<Option<git2::Commit<'_>>> {
    match repo.head().and_then(|head| head.peel_to_commit()) {
        Ok(commit) => Ok(Some(commit)),
        Err(e)
            if matches!(
                e.code(),
                git2::ErrorCode::UnbornBranch | git2::ErrorCode::NotFound
            ) =>
        {
            Ok(None)
        }
        Err(e) => Err(e.into()),
    }
}

fn stale_protection(unmerged: usize, unknown_default: bool) -> Option<String> {
    if unmerged > 0 {
        let phrase = if unmerged == 1 {
            "commit is"
        } else {
            "commits are"
        };
        Some(format!(
            "{unmerged} {phrase} not reachable from another branch"
        ))
    } else if unknown_default {
        Some("Cannot determine the default branch".to_owned())
    } else {
        None
    }
}

fn candidate(
    kind: StaleKind,
    name: String,
    path: Option<PathBuf>,
    commit: &git2::Commit<'_>,
    dirty: bool,
    unmerged: usize,
    protected: Option<String>,
) -> StaleCandidate {
    let author = commit.author();
    StaleCandidate {
        kind,
        name,
        path,
        tip: commit.id(),
        last_commit_time: commit.time().seconds(),
        author: Signature {
            name: String::from_utf8_lossy(author.name_bytes()).into_owned(),
            email: String::from_utf8_lossy(author.email_bytes()).into_owned(),
            time: author.when().seconds(),
            offset_minutes: author.when().offset_minutes(),
        },
        dirty,
        unmerged,
        protected,
    }
}
