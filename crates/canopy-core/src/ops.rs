//! Operations that change a repository. All of them run the `git` executable (see [`crate::cli`])
//! so hooks, signing, filters and credentials behave as they do in a terminal.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use git2::{ObjectType, Oid};

use crate::cli;
use crate::diff::{DiffSource, Whitespace};
use crate::model::{ChangeKind, FileEntry, RepoState};
use crate::{Error, MissingObjects, PushDefault, Repo, Result};

/// The most of a commit template Canopy reads, in bytes.
const COMMIT_TEMPLATE_LIMIT: u64 = 256 * 1024;

/// The most commits one patch export covers.
pub const MAX_PATCH_SERIES: usize = 50;

/// Distinguishes concurrent exports in this process; exclusive creation also handles leftovers.
static PATCH_EXPORT_NEXT: AtomicU64 = AtomicU64::new(0);

/// The commits an export writes patches for, and the merge commits it skips.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PatchSeries {
    /// Newest first.
    pub commits: Vec<Oid>,
    /// Merge commits of the chain, newest first: `git format-patch` writes no patch for them.
    pub merges: Vec<Oid>,
}

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

/// Clones `url` into `dest`, reporting progress lines, with its submodules when
/// `recurse_submodules`. A URL with credentials is refused
/// ([`remote_url_problem`]): the clone would keep it as its `origin`.
pub fn clone(
    url: &str,
    dest: &Path,
    recurse_submodules: bool,
    progress: impl FnMut(&str),
) -> Result<()> {
    let url = url.trim();
    if let Some(problem) = remote_url_problem(url) {
        return Err(Error::Invalid(problem.into()));
    }
    let dest_str = dest.to_string_lossy();
    let mut args = vec!["clone", "--progress"];
    if recurse_submodules {
        args.push("--recurse-submodules");
    }
    args.extend(["--", url, &dest_str]);
    cli::run_streaming(None, &args, progress)?;
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
    let url = url.trim();
    // A git remote helper can receive an invalid one-slash URL and print the entire URL
    // (including its password) in stderr before refusing it.
    let Some((scheme, rest)) = url.split_once("://").or_else(|| url.split_once(":/")) else {
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

/// Where a force push sends the current branch ([`Repo::force_push_target`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushTarget {
    /// The checked-out branch that is pushed.
    pub branch: String,
    /// The remote it goes to.
    pub remote: String,
    /// The branch on that remote that is overwritten, without `refs/heads/`.
    pub dest: String,
}

/// Every blob and gitlink path in `tree`, without converting native names through UTF-8.
fn tree_paths(
    repo: &git2::Repository,
    tree: &git2::Tree<'_>,
    parent: &Path,
    out: &mut Vec<PathBuf>,
) -> Result<()> {
    for entry in tree {
        let path = parent.join(crate::paths::os_path(entry.name_bytes()));
        match entry.kind() {
            Some(ObjectType::Tree) => {
                let subtree = entry.to_object(repo)?.peel_to_tree()?;
                tree_paths(repo, &subtree, &path, out)?;
            }
            Some(ObjectType::Blob | ObjectType::Commit) => out.push(path),
            _ => {}
        }
    }
    Ok(())
}

/// The untracked candidate at `candidate`, or one above or below it, when it resolves to the same
/// path as `root` on this filesystem. Canonical paths catch case-insensitive and normalizing names
/// without treating unrelated files as obstructions. A dangling symlink has no target to preserve,
/// so it does not collide; other I/O errors still stop the reset.
fn resolves_to_same_path(
    dir: &Path,
    root: &Path,
    candidates: &[(PathBuf, bool)],
) -> Result<Option<(PathBuf, bool)>> {
    let actual = match std::fs::canonicalize(dir.join(root)) {
        Ok(path) => path,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    for (candidate, ignored) in candidates {
        let other = match std::fs::canonicalize(dir.join(candidate)) {
            Ok(path) => path,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        if other.starts_with(&actual) || actual.starts_with(&other) {
            return Ok(Some((candidate.clone(), *ignored)));
        }
    }
    Ok(None)
}

/// An untracked path `git reset --hard` would remove while restoring a tracked file, or an ignored
/// one it would remove in either mode. Returning it lets Discard refuse before the first mutation.
fn reset_obstruction(dir: &Path, keep_untracked: bool) -> Result<Option<(PathBuf, bool)>> {
    let repo = git2::Repository::open(dir)?;
    let head = repo.head()?.peel_to_tree()?;
    let mut tracked = Vec::new();
    tree_paths(&repo, &head, Path::new(""), &mut tracked)?;
    let mut options = git2::StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(true)
        .recurse_ignored_dirs(true);
    let statuses = repo.statuses(Some(&mut options))?;
    let mut candidates = Vec::new();
    let mut deleted = Vec::new();
    for entry in statuses.iter() {
        let status = entry.status();
        let ignored = status.contains(git2::Status::IGNORED);
        let untracked = status.contains(git2::Status::WT_NEW);
        if ignored || keep_untracked && untracked {
            candidates.push((crate::paths::os_path(entry.path_bytes()), ignored));
        }
        if status.intersects(
            git2::Status::INDEX_DELETED
                | git2::Status::WT_DELETED
                | git2::Status::INDEX_TYPECHANGE
                | git2::Status::WT_TYPECHANGE,
        ) {
            deleted.push(crate::paths::os_path(entry.path_bytes()));
        }
    }
    for (candidate, ignored) in &candidates {
        if tracked.iter().any(|tracked| {
            *candidate == *tracked
                || candidate.starts_with(tracked)
                || tracked.starts_with(candidate)
        }) {
            return Ok(Some((candidate.clone(), *ignored)));
        }
    }
    // A deleted path that exists under another spelling is visible through the filesystem's own
    // resolution, including staged deletions. A non-directory ancestor blocks a tracked child.
    for path in deleted {
        if std::fs::symlink_metadata(dir.join(&path)).is_ok()
            && let Some(found) = resolves_to_same_path(dir, &path, &candidates)?
        {
            return Ok(Some(found));
        }
    }
    for path in tracked {
        for ancestor in path.ancestors().skip(1) {
            if ancestor.as_os_str().is_empty() {
                continue;
            }
            if std::fs::symlink_metadata(dir.join(ancestor))
                .is_ok_and(|metadata| !metadata.is_dir())
                && let Some(found) = resolves_to_same_path(dir, ancestor, &candidates)?
            {
                return Ok(Some(found));
            }
        }
    }
    Ok(None)
}

impl Repo {
    pub(crate) fn git_cmd(&self, args: &[&str]) -> Result<String> {
        cli::run(self.workdir(), args)
    }

    /// Runs `git args…` with the conflict style chosen in Canopy ([`crate::GitOptions`]), for anything
    /// that can write conflicts into files.
    fn conflicting_cmd(&self, args: &[&str]) -> Result<String> {
        let o = self.conflict_override();
        let mut all: Vec<&str> = o.iter().map(String::as_str).collect();
        all.extend_from_slice(args);
        self.git_cmd(&all)
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
    /// named `*` or `[ab].txt` discards only itself. A submodule goes back to the commit the index
    /// records, with its tracked files' changes discarded ([`Repo::discard_submodule`]); every
    /// submodule is checked before anything is discarded.
    pub fn discard(&self, files: &[FileEntry]) -> Result<()> {
        let submodules = self.submodules(false)?;
        let (inside, files): (Vec<&FileEntry>, Vec<&FileEntry>) = files
            .iter()
            .partition(|f| submodules.iter().any(|s| s.path == f.path));
        let inside: Vec<&crate::SubmoduleInfo> = submodules
            .iter()
            .filter(|s| inside.iter().any(|f| f.path == s.path))
            .collect();
        for submodule in &inside {
            self.stranded_commit_check(submodule)?;
            self.reset_obstruction_check(submodule)?;
        }
        for submodule in inside {
            self.discard_submodule(submodule)?;
        }
        let (untracked, tracked): (Vec<_>, Vec<_>) = files
            .into_iter()
            .partition(|f| f.kind == ChangeKind::Untracked);
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

    /// Refuses when going back to the recorded commit would leave the commit checked out in
    /// `submodule` on no branch or tag of its own.
    fn stranded_commit_check(&self, submodule: &crate::SubmoduleInfo) -> Result<()> {
        let (Some(work), Some(recorded)) = (submodule.workdir_id, submodule.index_id) else {
            return Ok(());
        };
        if work == recorded {
            return Ok(());
        }
        let dir = self.workdir().join(&submodule.path_os);
        let refs = cli::run(
            &dir,
            &[
                "for-each-ref",
                "--format=%(refname)",
                "--contains",
                &work.to_string(),
            ],
        )?;
        if refs.trim().is_empty() {
            return Err(Error::invalid(format!(
                "{} has commits on no branch ({}). Create a branch in it first, or they are hard to find again.",
                submodule.path,
                crate::model::short_id(work)
            )));
        }
        Ok(())
    }

    /// Refuses a reset that could delete an untracked or ignored path. Every selected submodule is
    /// checked before the first reset; the check runs again immediately before that reset too.
    fn reset_obstruction_check(&self, submodule: &crate::SubmoduleInfo) -> Result<()> {
        let dir = self.workdir().join(&submodule.path_os);
        if !dir.join(".git").exists() {
            return Ok(());
        }
        let keep_untracked = !self.options().discard_untracked_in_submodules;
        if let Some((path, ignored)) = reset_obstruction(&dir, keep_untracked)? {
            let kind = if ignored { "ignored" } else { "untracked" };
            return Err(Error::invalid(format!(
                "{} has an {kind} path that obstructs a tracked path: {}. Move it aside before Discard so it is not deleted.",
                submodule.path,
                path.display()
            )));
        }
        Ok(())
    }

    /// Discards the changes to the tracked files inside `submodule` (`git reset --hard` there)
    /// and checks out the commit the index records (`git submodule update --checkout`, which
    /// ignores a configured update command). Untracked files inside stay unless
    /// [`crate::GitOptions::discard_untracked_in_submodules`] says to delete them (ignored files
    /// and nested repositories always stay), and nested submodules are left as they are.
    fn discard_submodule(&self, submodule: &crate::SubmoduleInfo) -> Result<()> {
        let dir = self.workdir().join(&submodule.path_os);
        // A deleted folder has nothing to reset: the update below checks it out again.
        if dir.join(".git").exists() {
            self.reset_obstruction_check(submodule)?;
            let keep_untracked = !self.options().discard_untracked_in_submodules;
            cli::run(
                &dir,
                &["-c", "submodule.recurse=false", "reset", "--hard", "-q"],
            )?;
            if !keep_untracked {
                cli::run(&dir, &["clean", "-f", "-d", "-q"])?;
            }
        }
        cli::run_with_path(
            self.workdir(),
            &[
                "--literal-pathspecs",
                "-c",
                "submodule.recurse=false",
                "submodule",
                "update",
                "--checkout",
                "-q",
                "--",
            ],
            &submodule.path_os,
        )
        .map(drop)
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

    /// The text of the file `commit.template` names, for the message field of a new commit, with
    /// the `#` comment lines `git commit` would strip from it left out. `None` when no template
    /// is configured. A relative path is taken from the top of the working tree, as git does.
    pub fn commit_template(&self) -> Result<Option<String>> {
        let out = cli::run_unchecked(
            self.workdir(),
            &["config", "--path", "--get", "--", "commit.template"],
        )?;
        let path = String::from_utf8_lossy(&out.stdout).trim_end().to_owned();
        if out.code != Some(0) || path.is_empty() {
            return Ok(None);
        }
        let path = self.workdir().join(path);
        let unreadable = |e: &dyn std::fmt::Display| {
            Error::invalid(format!(
                "Could not read the commit template {}: {e}",
                path.display()
            ))
        };
        let meta = std::fs::metadata(&path).map_err(|e| unreadable(&e))?;
        if !meta.is_file() {
            return Err(unreadable(&"it is not a regular file"));
        }
        let mut text = Vec::new();
        std::fs::File::open(&path)
            .and_then(|file| file.take(COMMIT_TEMPLATE_LIMIT + 1).read_to_end(&mut text))
            .map_err(|e| unreadable(&e))?;
        if u64::try_from(text.len()).is_ok_and(|len| len > COMMIT_TEMPLATE_LIMIT) {
            return Err(unreadable(&"it is too large"));
        }
        // The comment character git would strip: `core.commentChar` when it is one character
        // (`auto` and anything else mean `#`). Trailing spaces stay: "feat: " is a prefix.
        let configured =
            cli::run_unchecked(self.workdir(), &["config", "--get", "core.commentChar"])?;
        let configured = String::from_utf8_lossy(&configured.stdout);
        let mut chars = configured.trim_end().chars();
        let comment = match (chars.next(), chars.next()) {
            (Some(c), None) if !c.is_whitespace() => c,
            _ => '#',
        };
        let text = String::from_utf8_lossy(&text);
        let kept: Vec<&str> = text.lines().filter(|l| !l.starts_with(comment)).collect();
        Ok(Some(kept.join("\n").trim_end().to_owned()))
    }

    /// Commits the index with `message`, running hooks and signing as configured. `amend`
    /// replaces `HEAD`; `signoff` adds a `Signed-off-by:` trailer for the committer
    /// (`git commit --signoff`). The message is cleaned up as [`crate::GitOptions::cleanup`]
    /// says (by default only surrounding blank lines and trailing spaces, so a line starting with
    /// `#` is the user's, not a comment). Returns the new `HEAD`.
    pub fn commit(&self, message: &str, amend: bool, signoff: bool) -> Result<Oid> {
        if message.trim().is_empty() {
            return Err(Error::invalid("Write a commit message first."));
        }
        let cleanup = self.cleanup_flag();
        let mut args = vec!["commit", "-q", "-F", "-"];
        args.extend(cleanup.as_deref());
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
            vec!["-q", "--no-track", "-c", name]
        } else {
            vec!["--no-track", "--", name]
        };
        if let Some(s) = start {
            args.push(s);
        }
        if checkout {
            self.switch_cmd(&args).map(drop)
        } else {
            args.insert(0, "branch");
            self.git_cmd(&args).map(drop)
        }
    }

    /// Adds a linked working folder (`git worktree add`) at `folder`: on a new branch `branch`
    /// started at `start` (`HEAD` when `None`), or, with `create` off, on the existing local
    /// branch `branch`. A new branch never gets an upstream, as with [`Self::create_branch`].
    /// Git refuses a branch another folder has checked out and a folder that is not empty, and
    /// says so.
    pub fn add_worktree(
        &self,
        folder: &Path,
        branch: &str,
        create: bool,
        start: Option<&str>,
    ) -> Result<()> {
        if branch.is_empty() || branch.starts_with('-') {
            return Err(Error::invalid(format!(
                "'{branch}' is not a valid branch name."
            )));
        }
        if start.is_some_and(|s| s.is_empty() || s.starts_with('-')) {
            return Err(Error::invalid(format!(
                "'{}' is not a valid start point.",
                start.unwrap_or_default()
            )));
        }
        let folder = folder
            .to_str()
            .ok_or_else(|| Error::invalid("The folder's name is not valid text."))?;
        let mut args = vec!["worktree", "add", "-q"];
        if create {
            args.extend(["--no-track", "-b", branch, "--", folder]);
            args.extend(start);
        } else {
            args.extend(["--", folder, branch]);
        }
        self.git_cmd(&args).map(drop)
    }

    /// Switches to a local branch.
    pub fn checkout_branch(&self, name: &str) -> Result<()> {
        self.switch_cmd(&["-q", "--", name]).map(drop)
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
        self.switch_cmd(&["-q", "-c", branch, "--track", "--", &full])
            .map(drop)
    }

    /// Detaches `HEAD` at a commit.
    pub fn checkout_detached(&self, oid: Oid) -> Result<()> {
        self.switch_cmd(&["-q", "--detach", &oid.to_string()])
            .map(drop)
    }

    /// Renames a local branch.
    pub fn rename_branch(&self, old: &str, new: &str) -> Result<()> {
        self.git_cmd(&["branch", "-m", "--", old, new]).map(drop)
    }

    /// Deletes a local branch with `git branch -d`, which refuses (with git's "not fully merged"
    /// message in [`Error::Cli`]) unless the branch is merged into its upstream, or into `HEAD`
    /// when it has none. The default way to delete a branch.
    pub fn delete_merged_branch(&self, name: &str) -> Result<()> {
        self.git_cmd(&["branch", "-d", "--", name]).map(drop)
    }

    /// Removes selected stale items after repeating every safety check. All selections are
    /// checked before the first mutation, then each is checked again immediately before git
    /// runs. A CLI failure can leave earlier removals completed; no removal is forced.
    pub fn remove_stale_candidates(
        &self,
        candidates: &[crate::StaleCandidate],
        older_than_days: u32,
        now: i64,
    ) -> Result<()> {
        let removed_refs: Vec<_> = candidates
            .iter()
            .filter(|c| c.kind == crate::StaleKind::Branch)
            .map(|c| format!("refs/heads/{}", c.name))
            .collect();
        // One scan checks the whole selection; each removal then re-scans once, because the
        // repository may have changed since.
        let fresh = self.stale_candidates(older_than_days, now)?;
        for candidate in candidates {
            Self::validate_stale_candidate(candidate, &fresh)?;
            if self.stale_unique_commits(candidate.tip, &removed_refs)? > 0 {
                return Err(Error::invalid(format!(
                    "Cannot remove {}: the selection would leave commits without a branch",
                    candidate.name
                )));
            }
        }
        for candidate in candidates {
            let fresh = self.stale_candidates(older_than_days, now)?;
            Self::validate_stale_candidate(candidate, &fresh)?;
            match candidate.kind {
                crate::StaleKind::Branch => self.delete_merged_branch(&candidate.name)?,
                crate::StaleKind::Worktree => {
                    let path = candidate
                        .path
                        .as_ref()
                        .and_then(|p| p.to_str())
                        .ok_or_else(|| Error::invalid("Invalid worktree path"))?;
                    self.git_cmd(&["worktree", "remove", "--", path])?;
                }
            }
        }
        Ok(())
    }

    /// Deletes a local branch with `git branch -D`, even when its commits are merged nowhere.
    /// Call it only after the user confirmed losing those commits (they stay in the reflog).
    pub fn force_delete_branch(&self, name: &str) -> Result<()> {
        self.git_cmd(&["branch", "-D", "--", name]).map(drop)
    }

    /// Merges `target` into the current branch as `mode` says and returns git's standard output,
    /// stashing uncommitted changes around it when [`crate::GitOptions::autostash`] says so.
    /// [`MergeMode::Squash`] stages the changes and stops before committing; the caller commits.
    /// A squash never stashes: git leaves that stash unapplied in `.git/MERGE_AUTOSTASH`, out of
    /// `git stash list`, so the edits vanish from the tree until the next commit restores them.
    /// A squash over files it changes is refused instead, as git refuses it without autostash.
    pub fn merge(&self, target: &str, mode: MergeMode) -> Result<String> {
        let mut args = vec!["merge", "--no-edit", mode.flag()];
        if matches!(mode, MergeMode::Squash) {
            let known = cli::version_number().is_some_and(|v| v >= cli::MERGE_AUTOSTASH_SINCE);
            args.extend(known.then_some("--no-autostash"));
        } else {
            args.extend(self.autostash_flag(false));
        }
        args.extend(["--", target]);
        self.conflicting_cmd(&args)
    }

    /// Rebases the current branch onto `onto`, stashing uncommitted changes around it when
    /// [`crate::GitOptions::autostash`] says so.
    pub fn rebase(&self, onto: &str) -> Result<String> {
        let mut args = Vec::new();
        args.extend(self.autostash_flag(true));
        args.extend(["--", onto]);
        self.rebase_cmd(&args)
    }

    /// Continues or aborts the operation in progress.
    pub fn continue_or_abort(&self, state: RepoState, abort: bool) -> Result<String> {
        let Some(cmd) = state.command() else {
            return Err(Error::invalid("Nothing to continue or abort."));
        };
        let flag = if abort { "--abort" } else { "--continue" };
        if cmd == "rebase" {
            self.rebase_cmd(&[flag])
        } else {
            self.conflicting_cmd(&[cmd, flag])
        }
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
        self.conflicting_cmd(&args)
    }

    /// Commits the inverse of `oid`.
    pub fn revert(&self, oid: Oid) -> Result<String> {
        let tail = self.mainline_args(oid)?;
        let mut args = vec!["revert", "--no-edit"];
        args.extend(tail.iter().map(String::as_str));
        self.conflicting_cmd(&args)
    }

    /// `oids` without repeats, ancestors before their descendants whatever their dates (a
    /// date sort would swap commits made in the same second or rebased out of order).
    fn parents_first(&self, oids: &[Oid]) -> Result<Vec<Oid>> {
        let wanted: std::collections::HashSet<Oid> = oids.iter().copied().collect();
        let mut walk = self.git().revwalk()?;
        walk.set_sorting(git2::Sort::TOPOLOGICAL)?;
        for oid in &wanted {
            walk.push(*oid)?;
        }
        let mut found = Vec::with_capacity(wanted.len());
        for id in walk {
            let id = id?;
            if wanted.contains(&id) {
                found.push(id);
                if found.len() == wanted.len() {
                    break;
                }
            }
        }
        found.reverse();
        Ok(found)
    }

    /// `oid` and the commits before it along first parents, newest first: at most `count`
    /// (clamped to 1 to [`MAX_PATCH_SERIES`]), fewer when the history starts sooner. Merge
    /// commits stay in the chain; the walk goes on through their first parent.
    pub fn first_parent_chain(&self, oid: Oid, count: usize) -> Result<Vec<Oid>> {
        let count = count.clamp(1, MAX_PATCH_SERIES);
        let repo = self.git();
        let mut chain = Vec::with_capacity(count);
        let mut next = Some(oid);
        while let Some(id) = next.filter(|_| chain.len() < count) {
            chain.push(id);
            next = repo.find_commit(id)?.parent_ids().next();
        }
        Ok(chain)
    }

    /// What exporting `oid` and up to `count - 1` commits before it along first parents writes:
    /// the chain of [`Self::first_parent_chain`] without its merge commits, which are listed
    /// apart because `git format-patch` writes no patch for a merge (given a merge, it writes
    /// the nearest commit before it instead).
    pub fn patch_series(&self, oid: Oid, count: usize) -> Result<PatchSeries> {
        let mut series = PatchSeries::default();
        for id in self.first_parent_chain(oid, count)? {
            if self.git().find_commit(id)?.parent_count() > 1 {
                series.merges.push(id);
            } else {
                series.commits.push(id);
            }
        }
        Ok(series)
    }

    /// Writes one `git format-patch` file per commit of `oids` into `dir`, parents first, and
    /// returns their paths. `dir` must be absolute and is created when missing. A merge commit
    /// is refused: `git format-patch` has no patch for it ([`Self::patch_series`] sets merges
    /// apart).
    pub fn export_patches(&self, oids: &[Oid], dir: &Path) -> Result<Vec<PathBuf>> {
        if oids.is_empty() {
            return Ok(Vec::new());
        }
        if !dir.is_absolute() {
            return Err(Error::invalid("Choose a full folder path for the patches."));
        }
        for id in oids {
            if self.git().find_commit(*id)?.parent_count() > 1 {
                return Err(Error::invalid(format!(
                    "A merge commit cannot be exported as a patch: {}",
                    crate::short_id(*id)
                )));
            }
        }
        let ordered = self.parents_first(oids)?;
        let io = |e: std::io::Error| Error::invalid(format!("{}: {e}", dir.display()));
        std::fs::create_dir_all(dir).map_err(io)?;
        // Written beside the target and moved in only once no file there would be replaced.
        let staging = loop {
            let next = PATCH_EXPORT_NEXT.fetch_add(1, Ordering::Relaxed);
            let path = dir.join(format!(".canopy-patches-{}-{next}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => break path,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(io(error)),
            }
        };
        let staging_arg = staging.to_string_lossy();
        let mut staged = Vec::new();
        let made = ordered.iter().enumerate().try_for_each(|(n, id)| {
            let start = (n + 1).to_string();
            let out = self.git_cmd(&[
                "format-patch",
                "--no-cover-letter",
                "-1",
                "--start-number",
                &start,
                "-o",
                &staging_arg,
                &id.to_string(),
            ])?;
            staged.extend(out.lines().map(PathBuf::from));
            Ok::<(), Error>(())
        });
        let targets: Vec<PathBuf> = staged
            .iter()
            .filter_map(|file| file.file_name().map(|name| dir.join(name)))
            .collect();
        let result = made.and_then(|()| {
            if let Some(existing) = targets.iter().find(|target| target.exists()) {
                return Err(Error::invalid(format!(
                    "{} already exists; pick another folder or move it away.",
                    existing.display()
                )));
            }
            for (file, target) in staged.iter().zip(&targets) {
                let mut source = std::fs::File::open(file).map_err(io)?;
                let mut destination = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(target)
                    .map_err(io)?;
                std::io::copy(&mut source, &mut destination).map_err(io)?;
            }
            Ok(())
        });
        // Only the staging folder this call exclusively created.
        let _ = std::fs::remove_dir_all(&staging);
        result?;
        let written = targets;
        Ok(written)
    }

    /// Checks that `patch` applies to the working tree without changing anything.
    pub fn check_patch(&self, patch: &Path) -> Result<()> {
        let patch = patch.to_string_lossy();
        self.git_cmd(&["apply", "--check", "--", &patch]).map(drop)
    }

    /// Applies mailbox patch files as commits (`git am --3way`), keeping their authors. A
    /// patch that does not apply cleanly stops with the repository in the "Applying patches"
    /// state, which [`Self::continue_or_abort`] resolves.
    pub fn apply_patches(&self, patches: &[PathBuf]) -> Result<String> {
        if patches.is_empty() {
            return Ok(String::new());
        }
        let files: Vec<String> = patches
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let mut args = vec!["am", "--3way", "--"];
        args.extend(files.iter().map(String::as_str));
        self.conflicting_cmd(&args).map_err(|e| match e {
            // git prints the conflicting files on stdout, not in its error: name them.
            Error::Cli { command, stderr } => {
                let paths = self
                    .git_cmd(&["diff", "--name-only", "--diff-filter=U"])
                    .unwrap_or_default();
                let named = paths.lines().collect::<Vec<_>>().join(", ");
                let stderr = if named.is_empty() {
                    stderr
                } else {
                    format!("conflicts in {named}\n{stderr}")
                };
                Error::Cli { command, stderr }
            }
            other => other,
        })
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
        self.conflicting_cmd(&["stash", if pop { "pop" } else { "apply" }, "-q", &r])
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
        self.conflicting_cmd(&["stash", if pop { "pop" } else { "apply" }, "-q", &r])
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
    /// remote-tracking branches whose branch no longer exists on the remote; local tags are
    /// pruned only as [`crate::GitOptions::prune_tags`] says (by default never: that would delete
    /// tags not pushed yet). Where Canopy follows the git configuration and it sets either for a
    /// remote, it decides for that remote ([`Repo::prune_args`]).
    pub fn fetch(
        &self,
        remote: Option<&str>,
        prune: bool,
        progress: impl FnMut(&str),
    ) -> Result<String> {
        self.fetch_with(remote, prune, false, progress)
    }

    /// [`Self::fetch`] for a fetch nobody asked for in a repository nobody trusted: besides the
    /// settings [`crate::partial::fetch_programs`] refuses on, it switches off on the command
    /// line what a repository's own files could make a fetch run, so the list does not have to
    /// name every way: the `ext::` transport, the command that advertises an alternate's tips,
    /// and the fetch of each submodule (which reads that submodule's own configuration).
    pub fn fetch_unattended(&self, prune: bool, progress: impl FnMut(&str)) -> Result<String> {
        self.fetch_with(None, prune, true, progress)
    }

    fn fetch_with(
        &self,
        remote: Option<&str>,
        prune: bool,
        unattended: bool,
        progress: impl FnMut(&str),
    ) -> Result<String> {
        let (config, flags) = self.prune_args(prune);
        let mut args: Vec<&str> = Vec::new();
        if unattended {
            args.extend([
                "-c",
                "core.alternateRefsCommand=",
                "-c",
                "protocol.ext.allow=never",
                "-c",
                "fetch.recurseSubmodules=false",
            ]);
        }
        args.extend(config.iter().map(String::as_str));
        args.extend(["fetch", "--progress", "--tags"]);
        if unattended {
            args.push("--no-recurse-submodules");
        }
        args.extend(flags);
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
    /// them as the default. Its fetch never prunes (`--no-prune`), so `fetch.pruneTags` cannot
    /// delete local tags either. After [`Repo::pull_preview`] has fetched, which prunes as
    /// [`Repo::fetch`] does, use [`Repo::pull_fetched`] instead.
    pub fn pull(&self, strategy: PullStrategy, progress: impl FnMut(&str)) -> Result<String> {
        let rebase = matches!(strategy, PullStrategy::Rebase | PullStrategy::RebaseMerges);
        let conflicts = self.conflict_override();
        let submodules = Self::submodule_override();
        let mut args: Vec<&str> = conflicts
            .iter()
            .chain(&submodules)
            .map(String::as_str)
            .collect();
        args.extend(["pull", "--progress", "--no-prune"]);
        args.extend(self.pull_autostash_flag(rebase)?);
        args.extend(strategy.git_args());
        cli::run_streaming(Some(self.workdir()), &args, progress)
    }

    /// Brings the current branch's upstream, as last fetched, into the branch with exactly
    /// `strategy`, without fetching again: what [`Repo::pull`] does after its fetch, with a merge
    /// (`git merge @{upstream}`) or a rebase onto the upstream (`git rebase`, which finds the
    /// fork point as `git pull --rebase` does). The pull's second step once
    /// [`Repo::pull_preview`] has fetched, so the remote is asked only once.
    ///
    /// With `checked`, the commit the preview showed, that commit is brought in whatever the
    /// upstream is by now: a fetch made elsewhere in between cannot slip in commits nobody saw.
    /// A rebase then still drops the commits the upstream rewrote, as `git rebase` does.
    pub fn pull_fetched(
        &self,
        strategy: PullStrategy,
        checked: Option<Oid>,
        progress: impl FnMut(&str),
    ) -> Result<String> {
        let conflicts = self.conflict_override();
        let submodules = Self::submodule_override();
        let mut args: Vec<&str> = conflicts
            .iter()
            .chain(&submodules)
            .map(String::as_str)
            .collect();
        let tip = checked.map(|oid| oid.to_string());
        let target = tip.as_deref().unwrap_or("@{upstream}");
        let merge_message = if tip.is_some() {
            self.pull_merge_message()
        } else {
            None
        };
        let fork_point = match (strategy, &tip) {
            (PullStrategy::Rebase | PullStrategy::RebaseMerges, Some(_)) => {
                self.upstream_fork_point()
            }
            _ => None,
        };
        match strategy {
            PullStrategy::Rebase | PullStrategy::RebaseMerges => {
                args.push("rebase");
                args.extend(self.pull_autostash_flag(true)?);
                if strategy == PullStrategy::RebaseMerges {
                    args.push("--rebase-merges");
                }
                if tip.is_some() {
                    match &fork_point {
                        Some(fork) => args.extend(["--onto", target, fork]),
                        None => args.push(target),
                    }
                }
            }
            PullStrategy::Merge
            | PullStrategy::MergeNoFastForward
            | PullStrategy::FastForwardOnly => {
                args.extend(["merge", "--no-edit"]);
                args.extend(self.pull_autostash_flag(false)?);
                if let Some(message) = &merge_message {
                    args.extend(["-m", message]);
                }
                args.push(match strategy {
                    PullStrategy::MergeNoFastForward => "--no-ff",
                    PullStrategy::FastForwardOnly => "--ff-only",
                    _ => "--ff",
                });
                args.push(target);
            }
        }
        cli::run_streaming(Some(self.workdir()), &args, progress)
    }

    /// Where the branch left the upstream's earlier tips, found in the upstream's reflog as
    /// `git rebase` does without arguments; `None` where there is none to find.
    fn upstream_fork_point(&self) -> Option<String> {
        let out = cli::run_unchecked(
            self.workdir(),
            &["merge-base", "--fork-point", "@{upstream}", "HEAD"],
        )
        .ok()?;
        (out.code == Some(0))
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
            .filter(|fork| !fork.is_empty())
    }

    /// The strategy the user's git configuration asks a pull to use ([`Repo::pull_config`]),
    /// else `fallback`. A configuration that cannot be read (a git older than 2.26 has no
    /// `--show-scope`, or a broken config file) counts as asking for nothing: the pull itself then
    /// reports git's own error if it has one.
    pub fn pull_strategy(&self, fallback: PullStrategy) -> PullStrategy {
        match self.pull_config() {
            Ok(config) => config.strategy.unwrap_or(fallback),
            Err(e) => {
                tracing::warn!(error = %e, "could not read the pull configuration");
                fallback
            }
        }
    }

    /// Pushes the current branch. Without an upstream, pushes to `origin` (or the only remote)
    /// and sets it as upstream. `force` uses `--force-with-lease`; annotated tags on the pushed
    /// commits go along when [`crate::GitOptions::follow_tags`] says so.
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
        // With an upstream this is a bare push: `push.default` decides where it goes. A force
        // push names its one destination, so neither `push.default=matching` nor a remote's push
        // refspecs can force other branches too.
        let mut args = self.push_default_override();
        args.extend(["push".to_owned(), "--progress".to_owned()]);
        args.extend(self.follow_tags_flag().map(str::to_owned));
        if force {
            args.push("--force-with-lease".to_owned());
        }
        if force && has_upstream {
            let (remote, dest) = self.push_destination(&branch)?;
            args.extend([remote, format!("HEAD:{dest}")]);
        } else if !has_upstream {
            let remote = self.remote_without_upstream()?;
            args.extend(["--set-upstream".to_owned(), remote, branch]);
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        cli::run_streaming(Some(self.workdir()), &args, progress)
    }

    /// The remote a branch with no upstream is pushed to: `origin`, else the only remote.
    fn remote_without_upstream(&self) -> Result<String> {
        let remotes = self.git().remotes()?;
        let names: Vec<&str> = remotes.iter().flatten().flatten().collect();
        if names.contains(&"origin") {
            Ok("origin".to_owned())
        } else if let [only] = names.as_slice() {
            Ok((*only).to_owned())
        } else if names.is_empty() {
            Err(Error::invalid(
                "This repository has no remote. Add one first.",
            ))
        } else {
            Err(Error::invalid(
                "Several remotes and no upstream: set one with git push -u.",
            ))
        }
    }

    /// Refuses unless a force push would go where `asked` says, as [`Repo::force_push_target`]
    /// works it out now. The branch can change between the question and the push, and only
    /// what the user confirmed may be overwritten.
    pub fn ensure_force_push_target(&self, asked: &PushTarget) -> Result<()> {
        let now = self.force_push_target()?;
        if now == *asked {
            return Ok(());
        }
        Err(Error::invalid(format!(
            "Nothing was pushed: you were asked about force pushing {} to {}/{}, but the \
             checked-out branch would now go to {}/{} as {}. Force push again to confirm it.",
            asked.branch, asked.remote, asked.dest, now.remote, now.dest, now.branch
        )))
    }

    /// Where [`Repo::push`] with `force` sends the current branch: the remote and the branch on
    /// it, worked out as the push works them out. Refuses where the push would.
    pub fn force_push_target(&self) -> Result<PushTarget> {
        let Some(branch) = self.head()?.branch else {
            return Err(Error::invalid("Check out a branch to push it."));
        };
        let has_upstream = self
            .git()
            .find_branch(&branch, git2::BranchType::Local)
            .and_then(|b| b.upstream())
            .is_ok();
        let (remote, dest) = if has_upstream {
            self.push_destination(&branch)?
        } else {
            (
                self.remote_without_upstream()?,
                format!("refs/heads/{branch}"),
            )
        };
        let dest = dest.strip_prefix("refs/heads/").unwrap_or(&dest).to_owned();
        Ok(PushTarget {
            branch,
            remote,
            dest,
        })
    }

    /// Where a push without a destination sends `branch`, as `(remote, ref on the remote)`,
    /// worked out as git does: `branch.<name>.pushRemote`, `remote.pushDefault` or the upstream's
    /// remote; then the remote's push refspec for the branch, else `push.default`
    /// ([`Repo::push_default`]). Refuses where git would.
    fn push_destination(&self, branch: &str) -> Result<(String, String)> {
        let upstream_remote = self.config_value(&format!("branch.{branch}.remote"));
        let remote = self
            .config_value(&format!("branch.{branch}.pushRemote"))
            .or_else(|| self.config_value("remote.pushDefault"))
            .or_else(|| upstream_remote.clone())
            .filter(|r| !r.is_empty() && !r.starts_with('-'))
            .ok_or_else(|| Error::invalid(format!("No remote to push {branch} to.")))?;
        let local = format!("refs/heads/{branch}");
        let out = cli::run(
            self.workdir(),
            &["for-each-ref", "--format=%(push:remoteref)", "--", &local],
        )?;
        let mapped = out.trim();
        if !mapped.is_empty() {
            return Ok((remote, mapped.to_owned()));
        }
        let merge = self.config_value(&format!("branch.{branch}.merge"));
        let triangular = upstream_remote.as_deref() != Some(remote.as_str());
        let mode = self.push_default();
        let dest = match mode {
            PushDefault::Current => Some(local),
            PushDefault::Simple if triangular => Some(local),
            PushDefault::Upstream if triangular => None,
            PushDefault::Upstream => merge,
            PushDefault::Simple => merge.filter(|m| *m == local),
        };
        let dest = dest.ok_or_else(|| {
            Error::invalid(format!(
                "Git cannot tell where to push {branch} to {remote} with push.default={}: name \
                 the branch in a terminal, or change Settings → Git → Push to.",
                mode.value()
            ))
        })?;
        Ok((remote, dest))
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
        let mut args = vec!["push", "--progress"];
        args.extend(self.follow_tags_flag());
        args.extend(["--set-upstream", remote, &refspec]);
        cli::run_streaming(Some(self.workdir()), &args, progress)
    }

    /// Pushes one tag to `remote`.
    pub fn push_tag(&self, remote: &str, tag: &str, progress: impl FnMut(&str)) -> Result<String> {
        let refspec = format!("refs/tags/{tag}");
        cli::run_streaming(
            Some(self.workdir()),
            &["push", "--progress", "--", remote, &refspec],
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
                self.switch_cmd(&["-q", "--no-track", "-c", branch, &private])?;
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
            &["push", "--progress", "--delete", "--", remote, branch],
            progress,
        )
    }
}
