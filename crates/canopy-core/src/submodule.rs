//! Submodules: a cheap status read kept off the main status path, the `git submodule` operations,
//! and the commit pair a submodule's diff shows.

use std::path::{Component, Path};

use git2::{FileMode, Oid, Repository, SubmoduleIgnore, SubmoduleStatus};

use crate::{Error, Repo, Result, cli};

/// The one state a submodule list shows, most urgent first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubmoduleState {
    /// The index holds a merge conflict for the submodule's commit.
    Conflicted,
    /// Not checked out: `git submodule update --init` has not run (or failed).
    NotInitialized,
    /// The submodule's checked-out commit differs from the one the superproject's index records.
    NewCommits,
    /// The submodule has uncommitted changes or untracked files.
    Dirty,
    /// Checked out at the recorded commit, with no changes (as far as was checked).
    Clean,
}

impl SubmoduleState {
    /// Display name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Conflicted => "Conflicted",
            Self::NotInitialized => "Not initialized",
            Self::NewCommits => "New commits",
            Self::Dirty => "Modified content",
            Self::Clean => "Up to date",
        }
    }
}

/// One submodule of the repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmoduleInfo {
    /// Name in `.gitmodules` (often the path).
    pub name: String,
    /// Path relative to the superproject's root, `/`-separated.
    pub path: String,
    /// URL from `.gitmodules`, when set.
    pub url: Option<String>,
    /// The commit `HEAD`'s tree records.
    pub head_id: Option<Oid>,
    /// The commit the index records.
    pub index_id: Option<Oid>,
    /// The commit checked out in the submodule (`None` when it is not checked out).
    pub workdir_id: Option<Oid>,
    /// The index has a conflict at the submodule's path.
    pub conflicted: bool,
    /// Uncommitted changes or untracked files inside the submodule; `None` until checked with
    /// [`Repo::submodule_dirty`] (or [`Repo::submodules`] with `check_dirty`).
    pub dirty: Option<bool>,
}

impl SubmoduleInfo {
    /// The submodule is checked out.
    pub fn is_initialized(&self) -> bool {
        self.workdir_id.is_some()
    }

    /// The checked-out commit differs from the one in the index: the change to stage.
    pub fn has_new_commits(&self) -> bool {
        self.is_initialized() && self.workdir_id != self.index_id
    }

    /// The index records a different commit than `HEAD`: a staged pointer change.
    pub fn is_staged(&self) -> bool {
        self.index_id != self.head_id
    }

    /// The state a list shows.
    pub fn state(&self) -> SubmoduleState {
        if self.conflicted {
            SubmoduleState::Conflicted
        } else if !self.is_initialized() {
            SubmoduleState::NotInitialized
        } else if self.has_new_commits() {
            SubmoduleState::NewCommits
        } else if self.dirty == Some(true) {
            SubmoduleState::Dirty
        } else {
            SubmoduleState::Clean
        }
    }
}

/// One commit in a submodule's log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmoduleCommit {
    /// Commit id.
    pub oid: Oid,
    /// First line of the message.
    pub summary: String,
    /// Author name.
    pub author: String,
}

/// Most commits a submodule's diff lists on each side.
pub const SUBMODULE_LOG_LIMIT: usize = 50;

/// The commits between the two sides of a submodule's diff, read from the submodule's own
/// repository.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubmoduleLog {
    /// Commits reachable from the new side but not the old one, newest first.
    pub added: Vec<SubmoduleCommit>,
    /// Commits reachable from the old side but not the new one: what moving back leaves out.
    pub removed: Vec<SubmoduleCommit>,
    /// A side has more commits than [`SUBMODULE_LOG_LIMIT`].
    pub truncated: bool,
}

/// The commits on each side of a submodule's diff (mode 160000).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmoduleChange {
    /// The commit before; `None` when the submodule was added.
    pub old: Option<Oid>,
    /// The commit after; `None` when the submodule was removed.
    pub new: Option<Oid>,
    /// `old` exists in the submodule's local repository, so its log can be shown.
    pub old_present: bool,
    /// `new` exists in the submodule's local repository.
    pub new_present: bool,
    /// What lies between the two commits; empty unless both exist locally.
    pub log: SubmoduleLog,
}

/// Commits reachable from `from` but not from `hidden`, newest first, at most
/// [`SUBMODULE_LOG_LIMIT`]; also whether more exist.
fn commits_between(repo: &Repository, from: Oid, hidden: Oid) -> (Vec<SubmoduleCommit>, bool) {
    let walk = || -> std::result::Result<_, git2::Error> {
        let mut walk = repo.revwalk()?;
        walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;
        walk.push(from)?;
        walk.hide(hidden)?;
        let mut out = Vec::new();
        for oid in walk {
            let oid = oid?;
            if out.len() == SUBMODULE_LOG_LIMIT {
                return Ok((out, true));
            }
            let commit = repo.find_commit(oid)?;
            out.push(SubmoduleCommit {
                oid,
                summary: String::from_utf8_lossy(commit.summary_bytes().unwrap_or_default())
                    .into_owned(),
                author: String::from_utf8_lossy(commit.author().name_bytes()).into_owned(),
            });
        }
        Ok((out, false))
    };
    walk().unwrap_or_default()
}

/// The submodule part of a single-path `diff`, if that path is a submodule on either side.
pub(crate) fn change_in(repo: &Repo, diff: &git2::Diff<'_>) -> Option<SubmoduleChange> {
    let delta = diff.deltas().find(|d| {
        d.old_file().mode() == FileMode::Commit || d.new_file().mode() == FileMode::Commit
    })?;
    let side = |file: git2::DiffFile<'_>| {
        (file.mode() == FileMode::Commit && !file.id().is_zero()).then(|| file.id())
    };
    let old = side(delta.old_file());
    let new = side(delta.new_file());
    let path = delta
        .new_file()
        .path()
        .or_else(|| delta.old_file().path())?;
    // The submodule's own repository, when it is checked out.
    let sub = Repository::open(repo.workdir().join(path)).ok();
    let present = |id: Option<Oid>| match (&sub, id) {
        (Some(sub), Some(id)) => sub.find_commit(id).is_ok(),
        _ => false,
    };
    let (old_present, new_present) = (present(old), present(new));
    let log = match (&sub, old, new) {
        (Some(sub), Some(old), Some(new)) if old_present && new_present => {
            let (added, more_added) = commits_between(sub, new, old);
            let (removed, more_removed) = commits_between(sub, old, new);
            SubmoduleLog {
                added,
                removed,
                truncated: more_added || more_removed,
            }
        }
        _ => SubmoduleLog::default(),
    };
    Some(SubmoduleChange {
        old,
        new,
        old_present,
        new_present,
        log,
    })
}

/// Why `url` and `path` cannot make a new submodule, or `None`.
///
/// The path is relative to the working folder and stays inside it; the URL follows the rules of
/// a remote's ([`crate::ops::remote_url_problem`]) and cannot name git's `ext::` helper, which
/// runs commands.
pub fn submodule_add_problem(url: &str, path: &str) -> Option<&'static str> {
    let (url, path) = (url.trim(), path.trim());
    if url.is_empty() {
        return Some("Enter the submodule's URL.");
    }
    if url
        .get(..5)
        .is_some_and(|p| p.eq_ignore_ascii_case("ext::"))
    {
        return Some("This kind of URL is not supported for submodules.");
    }
    if let Some(problem) = crate::ops::remote_url_problem(url) {
        return Some(problem);
    }
    if path.is_empty() {
        return Some("Enter the folder for the submodule.");
    }
    if path.starts_with('-') {
        return Some("A submodule's folder cannot start with \"-\".");
    }
    let plain = Path::new(path)
        .components()
        .all(|c| matches!(c, Component::Normal(_)));
    if !plain || path.contains('\\') || path.contains(':') {
        return Some("The folder must be a relative path inside the repository.");
    }
    if path
        .split('/')
        .any(|part| part.eq_ignore_ascii_case(".git"))
    {
        return Some("A submodule's folder cannot be inside .git.");
    }
    None
}

impl Repo {
    /// Every submodule in `.gitmodules` and the index, sorted by path. Reads only the recorded
    /// and checked-out commit ids and index conflicts, which is cheap; `check_dirty` also looks
    /// inside every checked-out submodule for changes, which can be slow (see
    /// [`Self::submodule_dirty`] to check one at a time).
    pub fn submodules(&self, check_dirty: bool) -> Result<Vec<SubmoduleInfo>> {
        let git = self.git();
        let mut index = git.index()?;
        // libgit2 keeps the index loaded; the git CLI may have rewritten it since.
        index.read(false)?;
        let mut out = Vec::new();
        for sm in git.submodules()? {
            let name = String::from_utf8_lossy(sm.name_bytes()).into_owned();
            let path = sm.path().to_string_lossy().replace('\\', "/");
            let conflicted = (1..=3).any(|stage| index.get_path(sm.path(), stage).is_some());
            let mut info = SubmoduleInfo {
                url: sm
                    .opt_url_bytes()
                    .map(|u| String::from_utf8_lossy(u).into_owned()),
                head_id: sm.head_id(),
                index_id: sm.index_id(),
                workdir_id: sm.workdir_id(),
                conflicted,
                dirty: None,
                name,
                path,
            };
            if check_dirty && info.is_initialized() {
                info.dirty = Some(self.dirty_by_name(&info.name)?);
            }
            out.push(info);
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }

    /// Whether the submodule at `path` has uncommitted changes or untracked files. `false` when
    /// it is not checked out.
    pub fn submodule_dirty(&self, path: &str) -> Result<bool> {
        let (name, _) = self.submodule_at(path)?;
        self.dirty_by_name(&name)
    }

    /// The `.gitmodules` name and path of the submodule at `path`. (libgit2's lookup by path
    /// reports the path as the name when the two differ.)
    fn submodule_at(&self, path: &str) -> Result<(String, String)> {
        let wanted = path.trim_end_matches('/');
        for sm in self.git().submodules()? {
            let sm_path = sm.path().to_string_lossy().replace('\\', "/");
            if sm_path == wanted {
                let name = String::from_utf8_lossy(sm.name_bytes()).into_owned();
                return Ok((name, sm.path().to_string_lossy().into_owned()));
            }
        }
        Err(Error::invalid(format!("'{path}' is not a submodule.")))
    }

    fn dirty_by_name(&self, name: &str) -> Result<bool> {
        let status = self.git().submodule_status(name, SubmoduleIgnore::None)?;
        Ok(status.intersects(
            SubmoduleStatus::WD_INDEX_MODIFIED
                | SubmoduleStatus::WD_WD_MODIFIED
                | SubmoduleStatus::WD_UNTRACKED,
        ))
    }

    /// `git submodule update [--init] [--recursive] -- [<path>]`: checks out the recorded commit
    /// in the submodule at `path`, or in every submodule when `None`, cloning or fetching when
    /// needed. `init` also registers and clones submodules that were never initialised.
    pub fn submodule_update(
        &self,
        path: Option<&str>,
        init: bool,
        recursive: bool,
        progress: impl FnMut(&str),
    ) -> Result<String> {
        let mut args = vec!["submodule", "update", "--progress"];
        if init {
            args.push("--init");
        }
        if recursive {
            args.push("--recursive");
        }
        args.push("--");
        args.extend(path);
        cli::run_streaming(Some(self.workdir()), &args, progress)
    }

    /// `git submodule sync [--recursive] -- [<path>]`: copies the URLs in `.gitmodules` into the
    /// configuration (after a URL changed upstream).
    pub fn submodule_sync(&self, path: Option<&str>, recursive: bool) -> Result<String> {
        let mut args = vec!["submodule", "sync"];
        if recursive {
            args.push("--recursive");
        }
        args.push("--");
        args.extend(path);
        self.git_cmd(&args)
    }

    /// `git submodule add [-b <branch>] -- <url> <path>`: clones `url` into `path` (or adopts a
    /// repository already there) and stages it with `.gitmodules`.
    pub fn submodule_add(
        &self,
        url: &str,
        path: &str,
        branch: Option<&str>,
        progress: impl FnMut(&str),
    ) -> Result<String> {
        if let Some(problem) = submodule_add_problem(url, path) {
            return Err(Error::invalid(problem));
        }
        let (url, path) = (url.trim(), path.trim());
        let mut args = vec!["submodule", "add", "--progress"];
        if let Some(b) = branch.filter(|b| !b.trim().is_empty()) {
            args.extend(["-b", b]);
        }
        args.extend(["--", url, path]);
        cli::run_streaming(Some(self.workdir()), &args, progress)
    }

    /// Removes the submodule at `path` completely: `git submodule deinit -f`, `git rm -f` (which
    /// also drops its `.gitmodules` section and stages both), then deletes its repository under
    /// the git directory's `modules/`. Local commits that were never pushed are lost, so callers
    /// confirm first.
    pub fn submodule_remove(&self, path: &str) -> Result<()> {
        let (name, path) = self.submodule_at(path)?;
        // Validate before changing anything, so a bad name cannot leave a half-removed submodule.
        let modules = self.submodule_git_dir(&name)?;
        // `git rm` refuses to edit a `.gitmodules` with unstaged changes, which would leave the
        // submodule deinitialised but not removed.
        let check = ["diff", "--quiet", "--", ".gitmodules"];
        match cli::run_unchecked(self.workdir(), &check)? {
            out if out.code == Some(0) => {}
            out if out.code == Some(1) => {
                return Err(Error::invalid(
                    "Stage or discard your changes to .gitmodules before removing a submodule.",
                ));
            }
            out => return Err(out.error(&check)),
        }

        self.git_cmd(&["submodule", "deinit", "-q", "-f", "--", &path])?;
        self.git_cmd(&["rm", "-q", "-f", "--", &path])?;
        if let Some(dir) = modules {
            std::fs::remove_dir_all(&dir)?;
        }
        Ok(())
    }

    /// Where git keeps the repository of submodule `name` (`git rev-parse --git-path
    /// modules/<name>`), if it exists. Refuses a name that could point outside the `modules`
    /// directory or a location reached through a symbolic link.
    fn submodule_git_dir(&self, name: &str) -> Result<Option<std::path::PathBuf>> {
        let unsafe_name = name.is_empty()
            || name.contains('\\')
            || Path::new(name)
                .components()
                .any(|c| !matches!(c, Component::Normal(_)));
        if unsafe_name {
            return Err(Error::invalid(format!(
                "Refusing to remove submodule '{name}': its name is not a plain relative path."
            )));
        }
        // Deleting `modules/<name>` would also delete the repository of a submodule named
        // `<name>/…`.
        let nested = format!("{name}/");
        for other in self.git().submodules()? {
            let other_name = String::from_utf8_lossy(other.name_bytes()).into_owned();
            if other_name.starts_with(&nested) {
                return Err(Error::invalid(format!(
                    "Refusing to remove submodule '{name}': the repository of submodule \
                     '{other_name}' is stored inside its own."
                )));
            }
        }
        let modules = self.git_path("modules")?;
        let dir = self.git_path(&format!("modules/{name}"))?;
        let Ok(meta) = std::fs::symlink_metadata(&dir) else {
            return Ok(None);
        };
        #[expect(
            clippy::suspicious_operation_groupings,
            reason = "the last test is that the folder is not `modules` itself, not a third prefix check"
        )]
        let inside = match (modules.canonicalize(), dir.canonicalize()) {
            (Ok(canonical), Ok(resolved)) => {
                dir.starts_with(&modules)
                    && resolved.starts_with(&canonical)
                    && resolved != canonical
            }
            _ => false,
        };
        if !meta.is_dir() || !inside {
            return Err(Error::invalid(format!(
                "Refusing to delete {}: it is not a directory inside the git directory's modules.",
                dir.display()
            )));
        }
        // Every component below `modules` must be a real directory, not a link.
        let mut below = dir.as_path();
        while below != modules {
            if std::fs::symlink_metadata(below)?.file_type().is_symlink() {
                return Err(Error::invalid(format!(
                    "Refusing to delete {}: it goes through a symbolic link.",
                    dir.display()
                )));
            }
            match below.parent() {
                Some(parent) => below = parent,
                None => break,
            }
        }
        Ok(Some(dir))
    }

    /// `git rev-parse --git-path <rel>`, made absolute.
    pub(crate) fn git_path(&self, rel: &str) -> Result<std::path::PathBuf> {
        let out = self.git_cmd(&["rev-parse", "--git-path", rel])?;
        let p = Path::new(out.trim_end_matches(['\n', '\r']));
        Ok(if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.workdir().join(p)
        })
    }
}
