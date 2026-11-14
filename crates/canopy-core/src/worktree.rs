//! Removing a linked working folder and forgetting those that are gone: what the removal would
//! delete, read before the user is asked and again right before git runs.

use std::path::{Path, PathBuf};

use git2::{Repository, Status, StatusOptions, Worktree, WorktreeLockStatus};

use crate::stale::worktree_index_protection;
use crate::{Error, Repo, Result, WorktreeInfo, WorktreeLoss, WorktreeRemoval, WorktreeState};

/// How many paths of each kind a [`WorktreeLoss`] keeps: the user is shown a few, the count says
/// how many more.
const SHOWN_PATHS: usize = 8;

/// Whether `a` and `b` name the same folder, however each was spelled (links, a trailing slash).
fn same_folder(a: &Path, b: &Path) -> bool {
    a == b
        || std::fs::canonicalize(a)
            .ok()
            .zip(std::fs::canonicalize(b).ok())
            .is_some_and(|(a, b)| a == b)
}

/// Git's test for a registered folder to forget: the `.git` file in it is gone. A folder that
/// merely cannot be read (permissions) is not.
pub(crate) fn folder_gone(folder: &Path) -> bool {
    folder
        .join(".git")
        .symlink_metadata()
        .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
}

/// Why `worktree` is locked (empty when no reason was given), or `None` when it is not. Git
/// stores the reason as a file, with its final newline.
pub(crate) fn lock_reason(worktree: &Worktree) -> Option<String> {
    match worktree.is_locked() {
        Ok(WorktreeLockStatus::Locked(reason)) => {
            Some(reason.unwrap_or_default().trim_end().to_owned())
        }
        _ => None,
    }
}

impl WorktreeLoss {
    fn add(&mut self, path: &[u8]) {
        self.count += 1;
        if self.first.len() < SHOWN_PATHS {
            self.first.push(String::from_utf8_lossy(path).into_owned());
        }
    }
}

impl Repo {
    /// The linked working folder at `path`, or why there is none: the main folder is not one.
    fn linked_worktree(&self, path: &Path) -> Result<(String, Worktree)> {
        for name in &self.git().worktrees()? {
            let Ok(Some(name)) = name else { continue };
            let worktree = self.git().find_worktree(name)?;
            if same_folder(worktree.path(), path) {
                return Ok((name.to_owned(), worktree));
            }
        }
        let main = Repository::open(self.git().commondir()).ok();
        if main
            .as_ref()
            .and_then(Repository::workdir)
            .is_some_and(|main| same_folder(main, path))
        {
            return Err(Error::invalid(format!(
                "{} is the main working folder, which git never removes.",
                path.display()
            )));
        }
        Err(Error::invalid(format!(
            "{} is not a linked working folder of this repository.",
            path.display()
        )))
    }

    /// What removing the linked working folder at `path` would delete, and whether Canopy will
    /// not remove it ([`WorktreeRemoval::blocked`]): it is the folder open here, locked, gone or
    /// unreadable, or its index hides edits. The status scan counts untracked and ignored
    /// files too, as stale cleanup does.
    ///
    /// # Errors
    /// `path` is not a linked working folder of this repository.
    pub fn worktree_removal(&self, path: &Path) -> Result<WorktreeRemoval> {
        let (_, worktree) = self.linked_worktree(path)?;
        let folder = worktree.path().to_path_buf();
        let mut removal = WorktreeRemoval {
            path: folder.clone(),
            branch: None,
            head: None,
            modified: WorktreeLoss::default(),
            untracked: WorktreeLoss::default(),
            ignored: WorktreeLoss::default(),
            unreachable_commits: 0,
            blocked: None,
        };
        removal.blocked = if same_folder(&folder, self.workdir()) {
            Some(
                "It is the folder open in this tab. Switch to another working folder first.".into(),
            )
        } else if let Some(reason) = lock_reason(&worktree) {
            Some(if reason.is_empty() {
                "It is locked. Unlock it with git worktree unlock first.".into()
            } else {
                format!("It is locked ({reason}). Unlock it with git worktree unlock first.")
            })
        } else if folder_gone(&folder) {
            Some("Its folder is missing. Forget it instead; git deletes nothing then.".into())
        } else if folder.to_str().is_none() {
            Some("Its folder name is not valid UTF-8, so Canopy cannot pass it to git.".into())
        } else {
            None
        };
        if removal.blocked.is_some() {
            return Ok(removal);
        }
        let repo = match worktree
            .validate()
            .map_err(Error::from)
            .and_then(|()| Repository::open_from_worktree(&worktree).map_err(Error::from))
        {
            Ok(repo) => repo,
            Err(error) => {
                removal.blocked = Some(format!("Canopy cannot read the folder: {error}"));
                return Ok(removal);
            }
        };
        let head = repo.find_reference("HEAD")?;
        removal.branch = head
            .symbolic_target()
            .ok()
            .flatten()
            .and_then(|t| t.strip_prefix("refs/heads/"))
            .map(str::to_owned);
        removal.head = head.resolve().ok().and_then(|r| r.target());
        match worktree_index_protection(&repo) {
            Ok(None) => {}
            Ok(Some(reason)) => {
                removal.blocked = Some(reason);
                return Ok(removal);
            }
            Err(error) => {
                removal.blocked = Some(format!("Cannot check the folder's index: {error}"));
                return Ok(removal);
            }
        }
        let mut options = StatusOptions::new();
        // Untracked folders are listed file by file, ignored ones as the single item git
        // reports (a `target/` is one entry, not a hundred thousand).
        options
            .include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_ignored(true);
        let statuses = match repo.statuses(Some(&mut options)) {
            Ok(statuses) => statuses,
            Err(error) => {
                removal.blocked = Some(format!("Cannot check the folder for changes: {error}"));
                return Ok(removal);
            }
        };
        let index_changes = Status::INDEX_NEW
            | Status::INDEX_MODIFIED
            | Status::INDEX_DELETED
            | Status::INDEX_RENAMED
            | Status::INDEX_TYPECHANGE;
        for entry in statuses.iter() {
            let status = entry.status();
            if status.is_empty() {
                continue;
            }
            if status.contains(Status::IGNORED) {
                removal.ignored.add(entry.path_bytes());
            } else if status.contains(Status::WT_NEW) && !status.intersects(index_changes) {
                removal.untracked.add(entry.path_bytes());
            } else {
                removal.modified.add(entry.path_bytes());
            }
        }
        // A detached `HEAD` is kept alive only by this folder's administrative files, which the
        // removal deletes. Every branch counts as keeping a commit, so a commit that only a
        // tag reaches is counted too.
        if removal.branch.is_none()
            && let Some(head) = removal.head
        {
            removal.unreachable_commits = self.stale_unique_commits(head, &[])?;
        }
        Ok(removal)
    }

    /// Removes the linked working folder `expected` describes with `git worktree remove`, after
    /// reading it again: a folder that is blocked, or that differs from `expected` in any way
    /// (another branch or commit, a file changed, added or deleted), is refused with the
    /// reason, so nothing is deleted that the user did not see. Changed and untracked files are
    /// removed only when `expected` listed them (`--force`); a locked folder never is. The
    /// branch checked out there stays.
    ///
    /// # Errors
    /// The reason above, or git's own refusal.
    pub fn remove_worktree(&self, expected: &WorktreeRemoval) -> Result<()> {
        let now = self.worktree_removal(&expected.path)?;
        let name = expected.path.display();
        if let Some(reason) = &now.blocked {
            return Err(Error::invalid(format!("Cannot remove {name}: {reason}")));
        }
        if now != *expected {
            let what = if now.branch != expected.branch || now.head != expected.head {
                "another branch or commit is checked out there"
            } else if now.unreachable_commits != expected.unreachable_commits {
                "its commits changed"
            } else {
                "its files changed"
            };
            return Err(Error::invalid(format!(
                "{name} changed since the confirmation ({what}). Open the removal again to see what it would delete."
            )));
        }
        let folder = now
            .path
            .to_str()
            .ok_or_else(|| Error::invalid("The folder's name is not valid UTF-8."))?;
        let mut args = vec!["worktree", "remove"];
        args.extend(now.needs_force().then_some("--force"));
        args.extend(["--", folder]);
        self.git_cmd(&args).map(drop)
    }

    /// The registered working folders whose folder is gone and which are not locked: the ones
    /// `git worktree prune` forgets, by folder.
    pub fn missing_worktrees(&self) -> Vec<PathBuf> {
        self.worktrees()
            .into_iter()
            .filter(|w: &WorktreeInfo| w.state == WorktreeState::Missing && w.locked.is_none())
            .map(|w| w.path)
            .collect()
    }

    /// Forgets the working folders that are gone (`git worktree prune`), provided they are
    /// still exactly `expected`, the ones the user confirmed: prune cannot name one, so another
    /// folder that went missing since is refused. Deletes no working folder. A locked one is
    /// kept.
    ///
    /// # Errors
    /// The set changed, or git's own refusal.
    pub fn forget_missing_worktrees(&self, expected: &[PathBuf]) -> Result<()> {
        let mut now = self.missing_worktrees();
        let mut expected = expected.to_vec();
        now.sort();
        expected.sort();
        if now != expected {
            return Err(Error::invalid(
                "The missing worktrees changed since the confirmation. Open Forget again to see which ones git would forget.",
            ));
        }
        if now.is_empty() {
            return Ok(());
        }
        self.git_cmd(&["worktree", "prune"]).map(drop)
    }
}
