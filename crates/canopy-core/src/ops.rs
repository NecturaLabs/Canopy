//! Operations that change a repository. All of them run the `git` executable (see [`crate::cli`])
//! so hooks, signing, filters and credentials behave as they do in a terminal.

use std::path::Path;

use git2::Oid;

use crate::cli;
use crate::diff::DiffSource;
use crate::model::{ChangeKind, FileEntry, RepoState};
use crate::{Error, Repo, Result};

/// How far `git reset` goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetMode {
    /// Move the branch only; keep index and working tree.
    Soft,
    /// Move the branch and reset the index; keep the working tree.
    Mixed,
    /// Move the branch and discard every uncommitted change.
    Hard,
}

impl ResetMode {
    fn flag(self) -> &'static str {
        match self {
            Self::Soft => "--soft",
            Self::Mixed => "--mixed",
            Self::Hard => "--hard",
        }
    }

    /// Display name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Soft => "Soft",
            Self::Mixed => "Mixed",
            Self::Hard => "Hard",
        }
    }
}

/// What a hunk action does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunkAction {
    /// Add an unstaged hunk to the index.
    Stage,
    /// Remove a staged hunk from the index.
    Unstage,
    /// Revert an unstaged hunk in the working tree.
    Discard,
}

/// Clones `url` into `dest`, reporting progress lines.
pub fn clone(url: &str, dest: &Path, progress: impl FnMut(&str)) -> Result<()> {
    let dest_str = dest.to_string_lossy();
    cli::run_streaming(
        None,
        &["clone", "--progress", "--", url, &dest_str],
        progress,
    )?;
    Ok(())
}

fn with_paths<'a>(args: &[&'a str], paths: &'a [String]) -> Vec<&'a str> {
    let mut v: Vec<&str> = args.to_vec();
    v.push("--");
    v.extend(paths.iter().map(String::as_str));
    v
}

impl Repo {
    fn git_cmd(&self, args: &[&str]) -> Result<String> {
        cli::run(self.workdir(), args)
    }

    fn is_unborn(&self) -> bool {
        self.git().head().is_err()
    }

    /// Stages files (additions, modifications and deletions).
    pub fn stage(&self, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        self.git_cmd(&with_paths(&["add", "-A"], paths)).map(drop)
    }

    /// Stages every change, including untracked files.
    pub fn stage_all(&self) -> Result<()> {
        self.git_cmd(&["add", "-A"]).map(drop)
    }

    /// Removes files from the index, keeping the working tree.
    pub fn unstage(&self, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        if self.is_unborn() {
            self.git_cmd(&with_paths(&["rm", "--cached", "-r", "-q"], paths))
                .map(drop)
        } else {
            self.git_cmd(&with_paths(&["reset", "-q", "HEAD"], paths))
                .map(drop)
        }
    }

    /// Unstages everything.
    pub fn unstage_all(&self) -> Result<()> {
        if self.is_unborn() {
            self.git_cmd(&["rm", "--cached", "-r", "-q", "."]).map(drop)
        } else {
            self.git_cmd(&["reset", "-q"]).map(drop)
        }
    }

    /// Throws away working-tree changes to `files`: tracked files go back to their staged (or
    /// committed) content, untracked files are deleted.
    pub fn discard(&self, files: &[FileEntry]) -> Result<()> {
        let (untracked, tracked): (Vec<_>, Vec<_>) =
            files.iter().partition(|f| f.kind == ChangeKind::Untracked);
        let tracked: Vec<String> = tracked.into_iter().map(|f| f.path.clone()).collect();
        let untracked: Vec<String> = untracked.into_iter().map(|f| f.path.clone()).collect();
        if !tracked.is_empty() {
            self.git_cmd(&with_paths(&["checkout", "-q"], &tracked))?;
        }
        if !untracked.is_empty() {
            self.git_cmd(&with_paths(&["clean", "-f", "-q"], &untracked))?;
        }
        Ok(())
    }

    /// Stages, unstages or discards hunk number `index` of `path`. `expected` is the hunk's
    /// [`crate::Hunk::key`] as the user saw it; if the file changed since, nothing happens and an
    /// error asks for a refresh.
    pub fn apply_hunk(
        &self,
        action: HunkAction,
        path: &str,
        index: usize,
        expected: crate::HunkKey,
        context: u32,
    ) -> Result<()> {
        let source = match action {
            HunkAction::Stage | HunkAction::Discard => DiffSource::Unstaged,
            HunkAction::Unstage => DiffSource::Staged,
        };
        let diff = self.raw_diff(&source, path, None, context)?;
        let Some((patch, key)) = crate::diff::hunk_patch(&diff, index)? else {
            return Err(Error::invalid(
                "That change no longer exists. Refresh and try again.",
            ));
        };
        if key != expected {
            return Err(Error::invalid(
                "The file changed since it was shown. Refresh and try again.",
            ));
        }
        let args: &[&str] = match action {
            HunkAction::Stage => &["apply", "--cached", "--whitespace=nowarn", "-"],
            HunkAction::Unstage => &["apply", "--cached", "--reverse", "--whitespace=nowarn", "-"],
            HunkAction::Discard => &["apply", "--reverse", "--whitespace=nowarn", "-"],
        };
        cli::run_with_input(self.workdir(), args, &patch).map(drop)
    }

    /// Commits the index with `message`, running hooks and signing as configured. Returns the new
    /// `HEAD`.
    pub fn commit(&self, message: &str, amend: bool) -> Result<Oid> {
        if message.trim().is_empty() {
            return Err(Error::invalid("Write a commit message first."));
        }
        let mut args = vec!["commit", "-q", "--cleanup=strip", "-F", "-"];
        if amend {
            args.push("--amend");
        }
        cli::run_with_input(self.workdir(), &args, message.as_bytes())?;
        Ok(self.git().head()?.peel_to_commit()?.id())
    }

    /// Creates a branch at `start` (a commit, branch or tag; `HEAD` when `None`), optionally
    /// switching to it.
    pub fn create_branch(&self, name: &str, start: Option<&str>, checkout: bool) -> Result<()> {
        let mut args = if checkout {
            vec!["switch", "-q", "-c", name]
        } else {
            vec!["branch", name]
        };
        if let Some(s) = start {
            args.push(s);
        }
        self.git_cmd(&args).map(drop)
    }

    /// Switches to a local branch.
    pub fn checkout_branch(&self, name: &str) -> Result<()> {
        self.git_cmd(&["switch", "-q", name]).map(drop)
    }

    /// Checks out a remote branch: switches to the local branch of the same name if it exists,
    /// otherwise creates one tracking `remote/branch`.
    pub fn checkout_remote_branch(&self, remote: &str, branch: &str) -> Result<()> {
        if self
            .git()
            .find_branch(branch, git2::BranchType::Local)
            .is_ok()
        {
            return self.checkout_branch(branch);
        }
        let full = format!("{remote}/{branch}");
        self.git_cmd(&["switch", "-q", "-c", branch, "--track", &full])
            .map(drop)
    }

    /// Detaches `HEAD` at a commit.
    pub fn checkout_detached(&self, oid: Oid) -> Result<()> {
        self.git_cmd(&["switch", "-q", "--detach", &oid.to_string()])
            .map(drop)
    }

    /// Renames a local branch.
    pub fn rename_branch(&self, old: &str, new: &str) -> Result<()> {
        self.git_cmd(&["branch", "-m", old, new]).map(drop)
    }

    /// Deletes a local branch; `force` also deletes it when unmerged.
    pub fn delete_branch(&self, name: &str, force: bool) -> Result<()> {
        self.git_cmd(&["branch", if force { "-D" } else { "-d" }, name])
            .map(drop)
    }

    /// Merges `target` into the current branch.
    pub fn merge(&self, target: &str, no_ff: bool) -> Result<String> {
        let mut args = vec!["merge", "--no-edit"];
        if no_ff {
            args.push("--no-ff");
        }
        args.push(target);
        self.git_cmd(&args)
    }

    /// Rebases the current branch onto `onto`.
    pub fn rebase(&self, onto: &str) -> Result<String> {
        self.git_cmd(&["rebase", onto])
    }

    /// Continues or aborts the operation in progress.
    pub fn continue_or_abort(&self, state: RepoState, abort: bool) -> Result<String> {
        let Some(cmd) = state.command() else {
            return Err(Error::invalid("Nothing to continue or abort."));
        };
        self.git_cmd(&[cmd, if abort { "--abort" } else { "--continue" }])
    }

    fn mainline_args(&self, oid: Oid) -> Result<Vec<String>> {
        let parents = self.git().find_commit(oid)?.parent_count();
        let mut v = Vec::new();
        if parents > 1 {
            v.push("-m".to_owned());
            v.push("1".to_owned());
        }
        v.push(oid.to_string());
        Ok(v)
    }

    /// Applies the changes of `oid` on top of `HEAD`.
    pub fn cherry_pick(&self, oid: Oid) -> Result<String> {
        let tail = self.mainline_args(oid)?;
        let mut args = vec!["cherry-pick"];
        args.extend(tail.iter().map(String::as_str));
        self.git_cmd(&args)
    }

    /// Commits the inverse of `oid`.
    pub fn revert(&self, oid: Oid) -> Result<String> {
        let tail = self.mainline_args(oid)?;
        let mut args = vec!["revert", "--no-edit"];
        args.extend(tail.iter().map(String::as_str));
        self.git_cmd(&args)
    }

    /// Moves the current branch to `oid`.
    pub fn reset(&self, oid: Oid, mode: ResetMode) -> Result<()> {
        self.git_cmd(&["reset", "-q", mode.flag(), &oid.to_string()])
            .map(drop)
    }

    /// Creates a tag at `target`; annotated when `message` is given.
    pub fn create_tag(&self, name: &str, target: Oid, message: Option<&str>) -> Result<()> {
        let target = target.to_string();
        match message.filter(|m| !m.trim().is_empty()) {
            Some(m) => self
                .git_cmd(&["tag", "-a", name, "-m", m, &target])
                .map(drop),
            None => self.git_cmd(&["tag", name, &target]).map(drop),
        }
    }

    /// Deletes a local tag.
    pub fn delete_tag(&self, name: &str) -> Result<()> {
        self.git_cmd(&["tag", "-d", name]).map(drop)
    }

    /// Stashes local changes.
    pub fn stash_save(&self, message: Option<&str>, include_untracked: bool) -> Result<()> {
        let mut args = vec!["stash", "push", "-q"];
        if include_untracked {
            args.push("--include-untracked");
        }
        if let Some(m) = message.filter(|m| !m.trim().is_empty()) {
            args.push("-m");
            args.push(m);
        }
        self.git_cmd(&args).map(drop)
    }

    /// Applies a stash; `pop` also drops it when it applied cleanly.
    pub fn stash_apply(&self, index: usize, pop: bool) -> Result<()> {
        let r = format!("stash@{{{index}}}");
        self.git_cmd(&["stash", if pop { "pop" } else { "apply" }, "-q", &r])
            .map(drop)
    }

    /// Deletes a stash entry.
    pub fn stash_drop(&self, index: usize) -> Result<()> {
        let r = format!("stash@{{{index}}}");
        self.git_cmd(&["stash", "drop", "-q", &r]).map(drop)
    }

    /// Adds a remote.
    pub fn add_remote(&self, name: &str, url: &str) -> Result<()> {
        self.git_cmd(&["remote", "add", name, url]).map(drop)
    }

    /// Removes a remote and its remote-tracking branches.
    pub fn remove_remote(&self, name: &str) -> Result<()> {
        self.git_cmd(&["remote", "remove", name]).map(drop)
    }

    /// Fetches `remote`, or every remote when `None`, pruning deleted branches.
    pub fn fetch(&self, remote: Option<&str>, progress: impl FnMut(&str)) -> Result<String> {
        let mut args = vec!["fetch", "--progress", "--prune", "--tags"];
        match remote {
            Some(r) => args.push(r),
            None => args.push("--all"),
        }
        cli::run_streaming(Some(self.workdir()), &args, progress)
    }

    /// Pulls into the current branch. Uses `pull.rebase` from the configuration when set,
    /// otherwise `rebase` decides between rebasing and merging.
    pub fn pull(&self, rebase: bool, progress: impl FnMut(&str)) -> Result<String> {
        let mut args = vec!["pull", "--progress"];
        if self.config_string("pull.rebase").is_none() {
            args.push(if rebase { "--rebase" } else { "--no-rebase" });
        }
        cli::run_streaming(Some(self.workdir()), &args, progress)
    }

    /// Pushes the current branch. Without an upstream, pushes to `origin` (or the only remote)
    /// and sets it as upstream. `force` uses `--force-with-lease`.
    pub fn push(&self, force: bool, progress: impl FnMut(&str)) -> Result<String> {
        let head = self.head()?;
        let Some(branch) = head.branch else {
            return Err(Error::invalid("Check out a branch to push it."));
        };
        let has_upstream = self
            .git()
            .find_branch(&branch, git2::BranchType::Local)
            .and_then(|b| b.upstream())
            .is_ok();
        let mut args = vec!["push".to_owned(), "--progress".to_owned()];
        if force {
            args.push("--force-with-lease".to_owned());
        }
        if !has_upstream {
            let remotes = self.git().remotes()?;
            let names: Vec<&str> = remotes.iter().flatten().flatten().collect();
            let remote = if names.contains(&"origin") {
                "origin".to_owned()
            } else if let [only] = names.as_slice() {
                (*only).to_owned()
            } else if names.is_empty() {
                return Err(Error::invalid(
                    "This repository has no remote. Add one first.",
                ));
            } else {
                return Err(Error::invalid(
                    "Several remotes and no upstream: set one with git push -u.",
                ));
            };
            args.extend(["--set-upstream".to_owned(), remote, branch]);
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        cli::run_streaming(Some(self.workdir()), &args, progress)
    }

    /// Pushes one tag to `remote`.
    pub fn push_tag(&self, remote: &str, tag: &str, progress: impl FnMut(&str)) -> Result<String> {
        let refspec = format!("refs/tags/{tag}");
        cli::run_streaming(
            Some(self.workdir()),
            &["push", "--progress", remote, &refspec],
            progress,
        )
    }

    /// Deletes a branch on a remote.
    pub fn delete_remote_branch(
        &self,
        remote: &str,
        branch: &str,
        progress: impl FnMut(&str),
    ) -> Result<String> {
        cli::run_streaming(
            Some(self.workdir()),
            &["push", "--progress", remote, "--delete", branch],
            progress,
        )
    }
}
