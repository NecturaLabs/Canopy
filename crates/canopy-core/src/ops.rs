//! Operations that change a repository. All of them run the `git` executable (see [`crate::cli`])
//! so hooks, signing, filters and credentials behave as they do in a terminal.

use std::path::Path;

use git2::Oid;

use crate::cli;
use crate::diff::{DiffSource, Whitespace};
use crate::model::{ChangeKind, FileEntry, RepoState};
use crate::{Error, MissingObjects, Repo, Result};

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

/// How `git pull` integrates the upstream branch.
///
/// Canopy always passes the chosen strategy to git (see [`PullStrategy::git_args`]), so `pull.rebase`, `branch.<name>.rebase`, `pull.ff` and
/// `merge.ff` never decide what a pull does; [`Repo::pull_config`] reads them as the default a
/// choice starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PullStrategy {
    /// Merge the upstream branch, fast-forwarding when possible (`--no-rebase --ff`).
    #[default]
    Merge,
    /// Merge the upstream branch with a merge commit even when it could fast-forward
    /// (`--no-rebase --no-ff`), as `pull.ff=false` does.
    MergeNoFastForward,
    /// Rebase local commits onto the upstream branch (`--rebase`).
    Rebase,
    /// Rebase local commits and keep the merge commits among them (`--rebase=merges`).
    RebaseMerges,
    /// Only fast-forward; fail when the branches have diverged (`--no-rebase --ff-only`).
    FastForwardOnly,
}

impl PullStrategy {
    /// Every strategy, in menu order.
    pub const ALL: [Self; 5] = [
        Self::Merge,
        Self::MergeNoFastForward,
        Self::Rebase,
        Self::RebaseMerges,
        Self::FastForwardOnly,
    ];

    /// Display name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Merge => "Merge",
            Self::MergeNoFastForward => "Merge, always with a merge commit",
            Self::Rebase => "Rebase",
            Self::RebaseMerges => "Rebase, keeping merges",
            Self::FastForwardOnly => "Fast-forward only",
        }
    }

    /// The options `git pull` gets for this strategy. Each leaves no configuration value to
    /// decide how to integrate or whether to fast-forward.
    pub fn git_args(self) -> &'static [&'static str] {
        match self {
            Self::Merge => &["--no-rebase", "--ff"],
            Self::MergeNoFastForward => &["--no-rebase", "--no-ff"],
            Self::Rebase => &["--rebase"],
            Self::RebaseMerges => &["--rebase=merges"],
            Self::FastForwardOnly => &["--no-rebase", "--ff-only"],
        }
    }
}

/// How `git merge` combines the target into the current branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MergeMode {
    /// `git merge --ff`: fast-forward when the current branch is an ancestor of the target,
    /// otherwise create a merge commit. Passed explicitly so a `merge.ff` setting cannot make the
    /// label wrong.
    #[default]
    Default,
    /// `git merge --no-ff`: always create a merge commit, even when a fast-forward is possible.
    NoFastForward,
    /// `git merge --ff-only`: fast-forward, or fail without changing anything.
    FastForwardOnly,
    /// `git merge --squash`: apply the target's combined changes to the index and working tree
    /// without committing and without recording a merge (no `MERGE_HEAD`); the next commit is an
    /// ordinary single-parent commit, and git prepares its message in `.git/SQUASH_MSG`.
    /// Conflicts leave conflicted files but no merge in progress, so there is nothing to
    /// `--abort`: resolve and commit, or reset.
    Squash,
}

impl MergeMode {
    /// Every mode, in the order a picker shows them.
    pub const ALL: [Self; 4] = [
        Self::Default,
        Self::NoFastForward,
        Self::FastForwardOnly,
        Self::Squash,
    ];

    fn flag(self) -> &'static str {
        match self {
            Self::Default => "--ff",
            Self::NoFastForward => "--no-ff",
            Self::FastForwardOnly => "--ff-only",
            Self::Squash => "--squash",
        }
    }

    /// Display name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default (fast-forward if possible)",
            Self::NoFastForward => "Always create a merge commit",
            Self::FastForwardOnly => "Fast-forward only",
            Self::Squash => "Squash into one change",
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

impl HunkAction {
    /// The `git apply` arguments that apply a single-hunk patch read from standard input for this
    /// action. `context` is the number of context lines the patch was made with: with none, git
    /// apply's checks that a hunk is anchored by context (or by the start or end of the file)
    /// must be turned off with `--unidiff-zero`, as git-apply(1) describes for `diff -U0`
    /// output; otherwise a hunk in the middle of a file fails with "patch failed".
    pub(crate) fn apply_args(self, context: u32) -> Vec<&'static str> {
        let mut args = vec!["apply"];
        match self {
            Self::Stage => args.push("--cached"),
            Self::Unstage => args.extend(["--cached", "--reverse"]),
            Self::Discard => args.push("--reverse"),
        }
        if context == 0 {
            args.push("--unidiff-zero");
        }
        args.extend(["--whitespace=nowarn", "-"]);
        args
    }
}

/// Clones `url` into `dest`, reporting progress lines. A URL with credentials is refused
/// ([`remote_url_problem`]): the clone would keep it as its `origin`.
pub fn clone(url: &str, dest: &Path, progress: impl FnMut(&str)) -> Result<()> {
    let url = url.trim();
    if let Some(problem) = remote_url_problem(url) {
        return Err(Error::Invalid(problem.into()));
    }
    let dest_str = dest.to_string_lossy();
    cli::run_streaming(
        None,
        &["clone", "--progress", "--", url, &dest_str],
        progress,
    )?;
    Ok(())
}

/// Whether a remote URL carries credentials.
///
/// Any user name in an `http(s)://` URL counts (it is usually a token), and a password
/// (`user:secret@`) in any other URL with a scheme. An ssh user such as `git@github.com:o/r.git`
/// or `ssh://git@host/r` is not a credential. Every spelling git accepts counts: extra slashes
/// after `://` (`https:///token@host`), a `?` or `#` in the password, and a remote helper prefix
/// (`http::https://token@host`).
#[expect(
    clippy::string_slice,
    reason = "the range comes from cli::userinfo over the same text"
)]
pub fn url_has_credentials(url: &str) -> bool {
    let Some((scheme, rest)) = url.trim().split_once("://") else {
        return false;
    };
    let Some(user) = cli::userinfo(rest, char::is_whitespace) else {
        return false;
    };
    let scheme = scheme.rsplit("::").next().unwrap_or(scheme);
    let web = matches!(
        scheme.to_ascii_lowercase().as_str(),
        "http" | "https" | "ftp" | "ftps"
    );
    web || rest[user].contains(':')
}

/// Why `url` (trimmed) cannot be a remote's URL, or `None`.
///
/// A URL carrying credentials
/// ([`url_has_credentials`]) would be stored in `.git/config` and shown wherever the URL is,
/// while git asks for credentials through the credential helper anyway; one starting with `-`
/// could be read as an option.
pub fn remote_url_problem(url: &str) -> Option<&'static str> {
    let url = url.trim();
    if url.is_empty() {
        Some("Enter the remote's URL.")
    } else if url.starts_with('-') {
        Some("A remote's URL cannot start with \"-\".")
    } else if url_has_credentials(url) {
        Some(
            "Remove the user name and password from the URL: git asks for them when it needs them.",
        )
    } else {
        None
    }
}

/// The name's or else the URL's problem as an error.
fn check_remote(name: &str, url: &str) -> Result<()> {
    match remote_name_problem(name).or_else(|| remote_url_problem(url)) {
        Some(problem) => Err(Error::Invalid(problem.into())),
        None => Ok(()),
    }
}

/// Why `name` cannot name a remote, or `None`: empty, or starting with `-` like an option.
pub fn remote_name_problem(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        Some("Enter a name for the remote.")
    } else if name.starts_with('-') {
        Some("A remote's name cannot start with \"-\".")
    } else {
        None
    }
}

/// Where [`Repo::checkout_request`] keeps the head of `remote_ref` (a full ref such as
/// `refs/pull/12/head`) fetched from `remote`: `refs/canopy/requests/<remote>/pull/12/head`.
fn request_ref(remote: &str, remote_ref: &str) -> String {
    let rest = remote_ref.strip_prefix("refs/").unwrap_or(remote_ref);
    format!("refs/canopy/requests/{remote}/{rest}")
}

fn with_paths<'a>(args: &[&'a str], paths: &'a [String]) -> Vec<&'a str> {
    let mut v: Vec<&str> = args.to_vec();
    v.push("--");
    v.extend(paths.iter().map(String::as_str));
    v
}

impl Repo {
    pub(crate) fn git_cmd(&self, args: &[&str]) -> Result<String> {
        cli::run(self.workdir(), args)
    }

    fn is_unborn(&self) -> bool {
        self.git().head().is_err()
    }

    /// Stages files (additions, modifications and deletions). Paths are literal, never globs.
    pub fn stage(&self, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        self.git_cmd(&with_paths(&["--literal-pathspecs", "add", "-A"], paths))
            .map(drop)
    }

    /// Stages every change, including untracked files, except folders holding a repository of
    /// their own (see [`Self::stage_all_except_repositories`]).
    pub fn stage_all(&self) -> Result<()> {
        self.stage_all_except_repositories().map(drop)
    }

    /// Stages every change, including untracked files (`git add -A`), but leaves out untracked
    /// folders that hold a Git repository of their own and are not submodules: `git add` would
    /// record each as a gitlink with no `.gitmodules` entry, which nobody can clone. Returns those
    /// folders (relative, ending in `/`) so the caller can ask what to do with them; staging one
    /// on purpose is [`Self::stage`] with its path.
    pub fn stage_all_except_repositories(&self) -> Result<Vec<String>> {
        let repositories: Vec<String> = crate::status::collect_with(self.git(), usize::MAX)?
            .untracked_dirs
            .into_iter()
            .filter(|d| d.repository)
            .map(|d| d.path)
            .collect();
        // A name that is not UTF-8 cannot be passed as an exclusion, and staging it would add a
        // gitlink, so refuse instead.
        if let Some(bad) = repositories.iter().find(|r| r.contains('\u{FFFD}')) {
            return Err(Error::invalid(format!(
                "The folder {bad} holds a repository of its own and its name is not UTF-8, so \
                 Stage all cannot leave it out. Stage files one by one, or rename the folder."
            )));
        }
        let excludes: Vec<String> = repositories
            .iter()
            .map(|d| format!(":(top,exclude,literal){}", d.trim_end_matches('/')))
            .collect();
        let mut args = vec!["add", "-A", "--", ":/"];
        args.extend(excludes.iter().map(String::as_str));
        self.git_cmd(&args)?;
        Ok(repositories)
    }

    /// Removes files from the index, keeping the working tree. Paths are literal, never globs.
    pub fn unstage(&self, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        if self.is_unborn() {
            self.git_cmd(&with_paths(
                &["--literal-pathspecs", "rm", "--cached", "-r", "-q"],
                paths,
            ))
            .map(drop)
        } else {
            self.git_cmd(&with_paths(
                &["--literal-pathspecs", "reset", "-q", "HEAD"],
                paths,
            ))
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
    /// committed) content, untracked files are deleted. Paths are literal, never globs, so a file
    /// named `*` or `[ab].txt` discards only itself.
    pub fn discard(&self, files: &[FileEntry]) -> Result<()> {
        let (untracked, tracked): (Vec<_>, Vec<_>) =
            files.iter().partition(|f| f.kind == ChangeKind::Untracked);
        let tracked: Vec<String> = tracked.into_iter().map(|f| f.path.clone()).collect();
        let untracked: Vec<String> = untracked.into_iter().map(|f| f.path.clone()).collect();
        if !tracked.is_empty() {
            self.git_cmd(&with_paths(
                &["--literal-pathspecs", "checkout", "-q"],
                &tracked,
            ))?;
        }
        if !untracked.is_empty() {
            self.git_cmd(&with_paths(
                &["--literal-pathspecs", "clean", "-f", "-q"],
                &untracked,
            ))?;
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
        // Offline: the hunk was shown from this same diff, so a partial clone already has its
        // objects; if not, the caller gets Error::NotDownloaded rather than a hidden fetch.
        let found = self.read_diff(&source, path, None, MissingObjects::Fail, || {
            let diff = self.raw_diff(&source, path, None, context, Whitespace::Show)?;
            crate::diff::hunk_patch(&diff, index)
        })?;
        let Some((patch, key)) = found else {
            return Err(Error::invalid(
                "That change no longer exists. Refresh and try again.",
            ));
        };
        if key != expected {
            return Err(Error::invalid(
                "The file changed since it was shown. Refresh and try again.",
            ));
        }
        cli::run_with_input(self.workdir(), &action.apply_args(context), &patch).map(drop)
    }

    /// Commits the index with `message`, running hooks and signing as configured. `amend`
    /// replaces `HEAD`; `signoff` adds a `Signed-off-by:` trailer for the committer
    /// (`git commit --signoff`). Returns the new `HEAD`.
    pub fn commit(&self, message: &str, amend: bool, signoff: bool) -> Result<Oid> {
        if message.trim().is_empty() {
            return Err(Error::invalid("Write a commit message first."));
        }
        let mut args = vec!["commit", "-q", "--cleanup=strip", "-F", "-"];
        if amend {
            args.push("--amend");
        }
        if signoff {
            args.push("--signoff");
        }
        cli::run_with_input(self.workdir(), &args, message.as_bytes())?;
        Ok(self.git().head()?.peel_to_commit()?.id())
    }

    /// Creates a branch at `start` (a commit, branch, tag or remote-tracking branch such as
    /// `origin/main`; `HEAD` when `None`), optionally switching to it.
    ///
    /// The new branch never gets an upstream (`--no-track`), whatever `branch.autoSetupMerge`
    /// says: a branch started from `origin/main` would otherwise track `origin/main`, and a plain
    /// `git push` would then fail under `push.default=simple` or push to `main` under `upstream`.
    /// Use [`Self::publish_branch`] to give it its own upstream.
    pub fn create_branch(&self, name: &str, start: Option<&str>, checkout: bool) -> Result<()> {
        // Git refuses such names anyway; without this check they would be read as options.
        if name.is_empty() || name.starts_with('-') {
            return Err(Error::invalid(format!(
                "'{name}' is not a valid branch name."
            )));
        }
        if start.is_some_and(|s| s.is_empty() || s.starts_with('-')) {
            return Err(Error::invalid(format!(
                "'{}' is not a valid start point.",
                start.unwrap_or_default()
            )));
        }
        let mut args = if checkout {
            vec!["switch", "-q", "--no-track", "-c", name]
        } else {
            vec!["branch", "--no-track", "--", name]
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

    /// Deletes a local branch: [`Self::force_delete_branch`] when `force` is set, otherwise
    /// [`Self::delete_merged_branch`]. Kept for existing callers; new code calls those two.
    pub fn delete_branch(&self, name: &str, force: bool) -> Result<()> {
        if force {
            self.force_delete_branch(name)
        } else {
            self.delete_merged_branch(name)
        }
    }

    /// Deletes a local branch with `git branch -d`, which refuses (with git's "not fully merged"
    /// message in [`Error::Cli`]) unless the branch is merged into its upstream, or into `HEAD`
    /// when it has none. The default way to delete a branch.
    pub fn delete_merged_branch(&self, name: &str) -> Result<()> {
        self.git_cmd(&["branch", "-d", "--", name]).map(drop)
    }

    /// Deletes a local branch with `git branch -D`, even when its commits are merged nowhere.
    /// Call it only after the user confirmed losing those commits (they stay in the reflog).
    pub fn force_delete_branch(&self, name: &str) -> Result<()> {
        self.git_cmd(&["branch", "-D", "--", name]).map(drop)
    }

    /// Merges `target` into the current branch as `mode` says and returns git's standard output.
    /// [`MergeMode::Squash`] stages the changes and stops before committing; the caller commits.
    pub fn merge(&self, target: &str, mode: MergeMode) -> Result<String> {
        self.git_cmd(&["merge", "--no-edit", mode.flag(), target])
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

    /// Applies the stash whose commit is `oid`, wherever it now is in the list; `pop` also drops
    /// it when it applied cleanly. A stash pushed or dropped since the user chose it cannot
    /// shift the choice onto another.
    pub fn stash_apply_commit(&self, oid: Oid, pop: bool) -> Result<()> {
        let r = self.stash_ref(oid)?;
        self.git_cmd(&["stash", if pop { "pop" } else { "apply" }, "-q", &r])
            .map(drop)
    }

    /// Deletes the stash whose commit is `oid`, wherever it now is in the list.
    pub fn stash_drop_commit(&self, oid: Oid) -> Result<()> {
        let r = self.stash_ref(oid)?;
        self.git_cmd(&["stash", "drop", "-q", &r]).map(drop)
    }

    /// `stash@{n}` for the stash whose commit is `oid` now, or an error when it is gone.
    fn stash_ref(&self, oid: Oid) -> Result<String> {
        let wanted = oid.to_string();
        self.git_cmd(&["stash", "list", "--format=%H"])?
            .lines()
            .position(|line| line.trim() == wanted)
            .map(|index| format!("stash@{{{index}}}"))
            .ok_or_else(|| {
                Error::invalid("That stash is no longer in the stash list. Refresh and try again.")
            })
    }

    /// Adds the remote `name` fetching from `url` (trimmed). A name or URL that
    /// [`remote_name_problem`] or [`remote_url_problem`] refuses, such as a URL with credentials,
    /// is refused before git runs.
    pub fn add_remote(&self, name: &str, url: &str) -> Result<()> {
        let url = url.trim();
        check_remote(name, url)?;
        self.git_cmd(&["remote", "add", "--", name, url]).map(drop)
    }

    /// Removes a remote and its remote-tracking branches.
    pub fn remove_remote(&self, name: &str) -> Result<()> {
        self.git_cmd(&["remote", "remove", "--", name]).map(drop)
    }

    /// Points the remote `name` at `url` (`git remote set-url`), refused as for
    /// [`Repo::add_remote`].
    pub fn set_remote_url(&self, name: &str, url: &str) -> Result<()> {
        let url = url.trim();
        check_remote(name, url)?;
        self.git_cmd(&["remote", "set-url", "--", name, url])
            .map(drop)
    }

    /// Fetches `remote`, or every remote when `None`, with tags. `prune` also deletes
    /// remote-tracking branches whose branch no longer exists on the remote (`--prune`); without
    /// it `--no-prune` is passed, so `fetch.prune` / `remote.<name>.prune` in the configuration
    /// do not prune either.
    pub fn fetch(
        &self,
        remote: Option<&str>,
        prune: bool,
        progress: impl FnMut(&str),
    ) -> Result<String> {
        let mut args = vec!["fetch", "--progress", "--tags"];
        args.push(if prune { "--prune" } else { "--no-prune" });
        match remote {
            Some(r) => args.extend(["--", r]),
            None => args.push("--all"),
        }
        cli::run_streaming(Some(self.workdir()), &args, progress)
    }

    /// Pulls into the current branch with exactly `strategy`: its options are always passed to
    /// git, so the user's `pull.rebase`, `branch.<name>.rebase`, `pull.ff` and `merge.ff` cannot
    /// change what happens (a bare `git pull` also refuses diverged branches when none of the
    /// first three is set). [`Repo::pull_config`] reads those settings for a caller that wants
    /// them as the default.
    pub fn pull(&self, strategy: PullStrategy, progress: impl FnMut(&str)) -> Result<String> {
        let mut args = vec!["pull", "--progress"];
        args.extend(strategy.git_args());
        cli::run_streaming(Some(self.workdir()), &args, progress)
    }

    /// Pulls with the strategy the user's git configuration asks for ([`Repo::pull_config`]),
    /// else with `fallback`; either way the strategy is passed to git explicitly. A configuration
    /// that cannot be read (a git older than 2.26 has no `--show-scope`, or a broken config file)
    /// counts as asking for nothing: the pull itself then reports git's own error if it has one.
    pub fn pull_default(
        &self,
        fallback: PullStrategy,
        progress: impl FnMut(&str),
    ) -> Result<String> {
        let strategy = match self.pull_config() {
            Ok(config) => config.strategy.unwrap_or(fallback),
            Err(e) => {
                tracing::warn!(error = %e, "could not read the pull configuration");
                fallback
            }
        };
        self.pull(strategy, progress)
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

    /// Publishes the local branch `name` (checked out or not) to `remote` and makes the remote
    /// branch of the same name its upstream: `git push -u <remote> refs/heads/<name>:refs/heads/<name>`.
    /// Returns git's summary.
    pub fn publish_branch(
        &self,
        name: &str,
        remote: &str,
        progress: impl FnMut(&str),
    ) -> Result<String> {
        if remote.is_empty() || remote.starts_with('-') || self.git().find_remote(remote).is_err() {
            return Err(Error::invalid(format!("'{remote}' is not a remote.")));
        }
        if self
            .git()
            .find_branch(name, git2::BranchType::Local)
            .is_err()
        {
            return Err(Error::invalid(format!("'{name}' is not a local branch.")));
        }
        let refspec = format!("refs/heads/{name}:refs/heads/{name}");
        cli::run_streaming(
            Some(self.workdir()),
            &["push", "--progress", "--set-upstream", remote, &refspec],
            progress,
        )
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

    /// Checks out a pull or merge request head such as `refs/pull/12/head` (GitHub) or
    /// `refs/merge-requests/12/head` (GitLab) from `remote` as the local branch `branch` (e.g.
    /// `pr/12`). Returns fetch's summary.
    ///
    /// The head is force-fetched into Canopy's private namespace,
    /// `refs/canopy/requests/<remote>/<remote_ref without "refs/">` (for example
    /// `refs/canopy/requests/origin/pull/12/head`), never into `refs/remotes/`: a real branch
    /// called `pr/12` on the remote keeps its remote-tracking ref, and `git fetch --prune` does
    /// not delete the request head.
    ///
    /// A new `branch` is created at the head without an upstream (`--no-track`), since the
    /// request ref is not a branch on the remote. An existing local `branch` is switched to and
    /// fast-forwarded (`merge --ff-only`) when the head contains it, or just switched to when it
    /// already contains the head. When the two have diverged (the branch has commits of its own,
    /// the request was rewritten, or the branch is an unrelated branch of the same name) nothing
    /// is switched or changed and [`Error::Invalid`] says so.
    ///
    /// The private ref stays after the call (it lets a later call fast-forward); deleting the
    /// remote does not remove it (`git update-ref -d` does).
    pub fn checkout_request(
        &self,
        remote: &str,
        remote_ref: &str,
        branch: &str,
        progress: impl FnMut(&str),
    ) -> Result<String> {
        if remote.is_empty() || remote.starts_with('-') {
            return Err(Error::invalid(format!("'{remote}' is not a valid remote.")));
        }
        // Refs handed to check-ref-format start with `refs/`, so they cannot be read as options
        // (it takes no `--`).
        if !remote_ref.starts_with("refs/")
            || self.git_cmd(&["check-ref-format", remote_ref]).is_err()
        {
            return Err(Error::invalid(format!(
                "'{remote_ref}' is not a valid ref to fetch."
            )));
        }
        // `--branch` also expands `@{-1}` and friends, so insist the name comes back unchanged.
        let valid_branch = !branch.starts_with('-')
            && self
                .git_cmd(&["check-ref-format", "--branch", branch])
                .is_ok_and(|out| out.trim_end_matches('\n') == branch);
        if !valid_branch {
            return Err(Error::invalid(format!(
                "'{branch}' is not a valid branch name."
            )));
        }
        let private = request_ref(remote, remote_ref);
        if self.git_cmd(&["check-ref-format", &private]).is_err() {
            return Err(Error::invalid(format!(
                "'{private}' is not a valid ref name."
            )));
        }

        // Forced: request heads are rewritten by force-pushes. The destination is outside
        // `refs/remotes/`, so no remote-tracking branch moves and no prune removes it.
        let refspec = format!("+{remote_ref}:{private}");
        let summary = cli::run_streaming(
            Some(self.workdir()),
            &["fetch", "--progress", "--", remote, &refspec],
            progress,
        )?;
        let head = self.git().find_reference(&private)?.peel_to_commit()?.id();

        let local = self
            .git()
            .find_branch(branch, git2::BranchType::Local)
            .ok()
            .and_then(|b| b.get().target());
        match local {
            None => {
                self.git_cmd(&["switch", "-q", "--no-track", "-c", branch, &private])?;
            }
            Some(tip) => {
                let git = self.git();
                let behind = tip == head || git.graph_descendant_of(head, tip)?;
                let ahead = git.graph_descendant_of(tip, head)?;
                if !behind && !ahead {
                    return Err(Error::invalid(format!(
                        "The local branch '{branch}' and the {remote} request head have \
                         diverged (the branch has commits of its own, the request was \
                         rewritten, or '{branch}' is another branch). Nothing was changed; \
                         rename or delete '{branch}' to check the request out again."
                    )));
                }
                self.checkout_branch(branch)?;
                if !ahead {
                    self.git_cmd(&["merge", "--ff-only", "-q", &private])?;
                }
            }
        }
        Ok(summary)
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
