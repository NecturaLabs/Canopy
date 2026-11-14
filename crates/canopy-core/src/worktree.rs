//! Removing a linked working folder and forgetting those that are gone: what the removal would
//! delete, read before the user is asked and again right before git runs.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};

use git2::{Repository, Status, StatusOptions, Worktree, WorktreeLockStatus};

use crate::stale::worktree_index_protection;
use crate::{
    Error, ForgottenWorktree, OpenFolder, Repo, Result, WorktreeForget, WorktreeLoss,
    WorktreeRemoval, cli,
};

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

/// Adds to `hasher` the name and size of everything under `folder`, by name within each folder,
/// without following links: what git deletes with an ignored folder, which `git status` lists as
/// the folder alone.
fn hash_folder(folder: &Path, hasher: &mut DefaultHasher) {
    let mut pending = vec![folder.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            dir.hash(hasher);
            continue;
        };
        let mut entries: Vec<_> = read.flatten().collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let Ok(meta) = entry.path().symlink_metadata() else {
                continue;
            };
            entry.file_name().hash(hasher);
            meta.len().hash(hasher);
            if meta.is_dir() {
                pending.push(entry.path());
            }
        }
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
    pub fn worktree_removal(&self, path: &Path, open_in: &[OpenFolder]) -> Result<WorktreeRemoval> {
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
            operation: None,
            fingerprint: 0,
        };
        removal.blocked = if same_folder(&folder, self.workdir()) {
            Some(
                "It is the folder open in this tab. Switch to another working folder first.".into(),
            )
        } else if let Some(open) = open_in.iter().find(|o| same_folder(&o.path, &folder)) {
            Some(format!(
                "It is open in the tab {}. Close that tab first.",
                open.name
            ))
        } else if let Some(reason) = lock_reason(&worktree) {
            Some(if reason.is_empty() {
                "It is locked. Unlock it with git worktree unlock first.".into()
            } else {
                format!("It is locked ({reason}). Unlock it with git worktree unlock first.")
            })
        } else if folder_gone(&folder) {
            Some("Its folder is missing. Forget it instead (Forget missing worktrees…).".into())
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
        removal.operation = admin_operation(repo.path());
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
        if let Some(reason) = initialized_submodules(&repo, &folder) {
            removal.blocked = Some(reason);
            return Ok(removal);
        }
        let inside = self.worktrees_inside(&folder);
        if !inside.is_empty() {
            removal.blocked = Some(format!(
                "It contains another worktree of this repository ({}). Removing the folder would delete it and leave it registered; remove that one first.",
                inside.join(", ")
            ));
            return Ok(removal);
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
        let mut nested = Vec::new();
        let mut hasher = DefaultHasher::new();
        for entry in statuses.iter() {
            let status = entry.status();
            if status.is_empty() {
                continue;
            }
            // A folder git lists whole, untracked or ignored, that is a repository of its own.
            if entry.path_bytes().ends_with(b"/")
                && let Ok(path) = std::str::from_utf8(entry.path_bytes())
                && folder.join(path).join(".git").symlink_metadata().is_ok()
            {
                nested.push(path.to_owned());
                continue;
            }
            entry.path_bytes().hash(&mut hasher);
            status.bits().hash(&mut hasher);
            if status.contains(Status::IGNORED) {
                if entry.path_bytes().ends_with(b"/")
                    && let Ok(path) = std::str::from_utf8(entry.path_bytes())
                {
                    hash_folder(&folder.join(path), &mut hasher);
                }
                removal.ignored.add(entry.path_bytes());
            } else if status.contains(Status::WT_NEW) && !status.intersects(index_changes) {
                removal.untracked.add(entry.path_bytes());
            } else {
                removal.modified.add(entry.path_bytes());
            }
        }
        removal.fingerprint = hasher.finish();
        if !nested.is_empty() {
            removal.blocked = Some(format!(
                "It contains another repository ({}). Removing the folder, which git does only with --force, would delete it with any work that is not pushed.",
                nested.join(", ")
            ));
            return Ok(removal);
        }
        // A detached `HEAD` is kept alive only by this folder's administrative files, which the
        // removal deletes; a branch, remote branch or tag containing a commit keeps it.
        if removal.branch.is_none()
            && let Some(head) = removal.head
        {
            removal.unreachable_commits = self.commits_only_reached_by(head)?;
        }
        Ok(removal)
    }

    /// The folders of the other working folders of this repository (the main one and the linked
    /// ones) that lie inside `folder`, relative to it.
    fn worktrees_inside(&self, folder: &Path) -> Vec<String> {
        let mut others: Vec<PathBuf> = Vec::new();
        if let Ok(main) = Repository::open(self.git().commondir())
            && let Some(workdir) = main.workdir()
        {
            others.push(workdir.to_path_buf());
        }
        if let Ok(names) = self.git().worktrees() {
            for name in names.iter().flatten().flatten() {
                if let Ok(worktree) = self.git().find_worktree(name) {
                    others.push(worktree.path().to_path_buf());
                }
            }
        }
        let home = std::fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf());
        others
            .into_iter()
            .filter(|other| !same_folder(other, folder))
            .filter_map(|other| {
                let real = std::fs::canonicalize(&other).unwrap_or_else(|_| other.clone());
                let inside = real
                    .strip_prefix(&home)
                    .or_else(|_| other.strip_prefix(folder));
                inside.ok().map(|rel| rel.display().to_string())
            })
            .collect()
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
    pub fn remove_worktree(
        &self,
        expected: &WorktreeRemoval,
        open_in: &[OpenFolder],
    ) -> Result<()> {
        let now = self.worktree_removal(&expected.path, open_in)?;
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

    /// What `git worktree prune` would forget, as git itself says (`prune --dry-run`), with what
    /// forgetting each deletes. Canopy's own view of a missing folder is checked against it: a
    /// folder git forgets that Canopy does not see as gone (it exists but cannot be read, or its
    /// administrative folder is broken) blocks the forgetting, because the user would not have
    /// been told.
    ///
    /// # Errors
    /// git's own failure.
    pub fn forget_preview(&self) -> Result<WorktreeForget> {
        let args = ["worktree", "prune", "--dry-run", "--verbose"];
        let out = cli::run_unchecked(self.workdir(), &args)?;
        if out.code != Some(0) {
            return Err(out.error(&args));
        }
        let common = Repository::open(self.git().commondir())?;
        let mut entries = Vec::new();
        let mut unknown = Vec::new();
        // git words the line in the user's language; the folder name and the reason come after
        // the fixed `worktrees/` it is given.
        for line in String::from_utf8_lossy(&out.stdout)
            .lines()
            .chain(String::from_utf8_lossy(&out.stderr).lines())
        {
            let Some((_, rest)) = line.split_once("worktrees/") else {
                continue;
            };
            let (id, reason) = rest.split_once(": ").unwrap_or((rest, ""));
            let Ok(worktree) = self.git().find_worktree(id) else {
                unknown.push(id.to_owned());
                continue;
            };
            let head = common.find_reference(&format!("worktrees/{id}/HEAD")).ok();
            let branch = head
                .as_ref()
                .and_then(|h| h.symbolic_target().ok().flatten())
                .and_then(|t| t.strip_prefix("refs/heads/"))
                .map(str::to_owned);
            let head = head.and_then(|h| h.resolve().ok()).and_then(|r| r.target());
            let admin = self.git().commondir().join("worktrees").join(id);
            let path = worktree.path().to_path_buf();
            entries.push(ForgottenWorktree {
                id: id.to_owned(),
                reason: reason.to_owned(),
                gone: folder_gone(&path) && lock_reason(&worktree).is_none(),
                unreachable_commits: match (&branch, head) {
                    (None, Some(head)) => self.commits_only_reached_by(head)?,
                    _ => 0,
                },
                branch,
                head,
                submodule_repositories: admin.join("modules").is_dir(),
                operation: admin_operation(&admin),
                path,
            });
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.id.cmp(&b.id)));
        let blocked = if unknown.is_empty() {
            let outside: Vec<String> = entries
                .iter()
                .filter(|e| !e.gone)
                .map(|e| e.path.display().to_string())
                .collect();
            (!outside.is_empty()).then(|| {
                format!(
                    "Git would also forget {}, which Canopy does not see as missing: the folder exists but cannot be read, or its entry in the repository is broken. Fix that, or run git worktree prune yourself.",
                    outside.join(", ")
                )
            })
        } else {
            Some(format!(
                "Git would also forget {} in the repository's worktrees folder, which Canopy cannot read as a working folder. Run git worktree prune -n -v to see them.",
                unknown.join(", ")
            ))
        };
        Ok(WorktreeForget { entries, blocked })
    }

    /// Forgets the working folders `expected` lists (`git worktree prune`), provided git's set
    /// is still exactly that, down to what each deletes: prune cannot name one folder, so
    /// another that went missing since, or a commit made since, is refused. Deletes no working
    /// folder, but deletes each forgotten folder's administrative files in the repository (see
    /// [`ForgottenWorktree`]). A locked one is kept.
    ///
    /// # Errors
    /// The set changed or is blocked, or git's own refusal.
    pub fn forget_worktrees(&self, expected: &WorktreeForget) -> Result<()> {
        let now = self.forget_preview()?;
        if let Some(reason) = &now.blocked {
            return Err(Error::invalid(reason.clone()));
        }
        if now != *expected {
            return Err(Error::invalid(
                "The missing worktrees changed since the confirmation. Open Forget again to see which ones git would forget.",
            ));
        }
        if now.entries.is_empty() {
            return Ok(());
        }
        self.git_cmd(&["worktree", "prune"]).map(drop)
    }

    /// Adds the way out to git's refusal of a branch that is checked out in a worktree whose
    /// folder is gone: git refuses it as if the folder were there.
    pub(crate) fn explain_missing_worktree(&self, error: Error) -> Error {
        let Error::Cli { command, stderr } = error else {
            return error;
        };
        let refused = stderr.contains("already used by worktree")
            || stderr.contains("already checked out at");
        let missing = refused
            && self.worktrees().iter().any(|w| {
                w.state == crate::WorktreeState::Missing
                    && stderr.contains(w.path.to_string_lossy().trim_end_matches('/'))
            });
        Error::Cli {
            stderr: if missing {
                format!(
                    "{stderr}\nThat folder is missing. Forget it (Forget missing worktrees… in the Worktrees section or the palette), then check the branch out."
                )
            } else {
                stderr
            },
            command,
        }
    }

    /// Commits reachable from `head` that no branch, remote branch or tag reaches.
    fn commits_only_reached_by(&self, head: git2::Oid) -> Result<usize> {
        let mut walk = self.git().revwalk()?;
        walk.push(head)?;
        for reference in self.git().references()? {
            let reference = reference?;
            let Ok(name) = reference.name() else {
                continue;
            };
            if ["refs/heads/", "refs/remotes/", "refs/tags/"]
                .iter()
                .any(|prefix| name.starts_with(prefix))
                && let Ok(commit) = reference.peel_to_commit()
            {
                walk.hide(commit.id())?;
            }
        }
        walk.try_fold(0, |count, commit| commit.map(|_| count + 1))
            .map_err(Into::into)
    }
}

/// Why a worktree whose index holds initialized submodules is not removed, or `None` when it has
/// none. A submodule's repository lives under `.git/worktrees/<id>/modules`, which the removal
/// deletes with every commit in it that was never pushed. Git refuses such a worktree unless
/// forced, and `--force` is exactly what loses that work, so Canopy does not run it.
fn initialized_submodules(repo: &Repository, folder: &Path) -> Option<String> {
    /// The file mode of a submodule entry in the index.
    const GITLINK: u32 = 0o160_000;
    let index = repo.index().ok()?;
    let names: Vec<String> = index
        .iter()
        .filter(|entry| entry.mode == GITLINK)
        .filter_map(|entry| {
            let path = std::str::from_utf8(&entry.path).ok()?;
            folder
                .join(path)
                .join(".git")
                .symlink_metadata()
                .ok()
                .map(|_| path.to_owned())
        })
        .collect();
    (!names.is_empty()).then(|| {
        format!(
            "It has initialized {} ({}). Git refuses to remove such a worktree without --force, and --force deletes the submodule repositories with any commits that were never pushed. Deinitialize them with git submodule deinit, or remove the folder yourself.",
            if names.len() == 1 { "submodule" } else { "submodules" },
            names.join(", ")
        )
    })
}

/// The operation in progress in the administrative folder `admin` of a linked worktree, which
/// holds that folder's own rebase, bisect and merge state.
fn admin_operation(admin: &Path) -> Option<String> {
    [
        ("rebase-merge", "rebase"),
        ("rebase-apply", "rebase"),
        ("BISECT_LOG", "bisect"),
        ("MERGE_HEAD", "merge"),
        ("CHERRY_PICK_HEAD", "cherry-pick"),
        ("REVERT_HEAD", "revert"),
    ]
    .into_iter()
    .find(|(file, _)| admin.join(file).symlink_metadata().is_ok())
    .map(|(_, what)| what.to_owned())
}
