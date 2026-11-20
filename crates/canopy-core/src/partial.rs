//! Partial clones (`git clone --filter=blob:none`, `--filter=tree:0`): recognising them and
//! downloading the objects they left on the server.
//!
//! A partial clone omits objects (blobs, or trees too) and records a *promisor remote* that
//! promises to send them later. The `git` command fetches a missing object on demand; libgit2
//! cannot (it has no promisor support, and Canopy builds it without network transports), so a
//! libgit2 read that reaches a missing object fails with "object not found".
//!
//! Reads in [`crate::Repo`] therefore run libgit2 first, unchanged: a full clone never pays for
//! anything here. Only when libgit2 reports a missing object in a repository that is a partial
//! clone do they work out which objects the read needs and, if the caller allows it with
//! [`MissingObjects::Download`], fetch exactly those through the `git` executable with the same
//! command git itself uses for a lazy fetch (`fetch_objects` in git's `promisor-remote.c`):
//!
//! ```text
//! git -c fetch.negotiationAlgorithm=noop fetch <remote> --no-tags --no-write-fetch-head \
//!     --recurse-submodules=no --filter=blob:none --stdin      # object IDs on standard input
//! ```
//!
//! It runs through [`crate::cli`], so like any other network operation it never waits on a
//! terminal (`GIT_TERMINAL_PROMPT=0`), asks for credentials through the app's askpass helper when
//! one is set ([`crate::cli::set_askpass`]), and uses the user's credential helpers and SSH setup. libgit2 rescans the pack directory
//! when an object is not found, so the retried read sees the new pack.

use std::fmt::Write as _;
use std::path::Path;

use git2::{ErrorClass, ErrorCode, OdbLookupFlags, Oid, Repository};

use crate::{Error, Result, cli};

/// What a read does when a partial clone lacks an object it needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingObjects {
    /// Stay offline: return [`Error::NotDownloaded`] without contacting the remote.
    Fail,
    /// Download the missing objects from the promisor remote with the `git` executable, as
    /// `git show` would. This is a network operation: it can take as long as the fetch does and
    /// fails when the remote is unreachable or needs credentials Canopy cannot supply.
    Download,
}

/// How long one download may take before git is killed and the read fails, so a server that
/// never answers cannot hold the repository's thread for good.
const DOWNLOAD_LIMIT: std::time::Duration = std::time::Duration::from_secs(DOWNLOAD_MINUTES * 60);

/// [`DOWNLOAD_LIMIT`] in minutes, the unit the message names.
const DOWNLOAD_MINUTES: u64 = 10;

/// How many fetch-and-retry rounds a read may take: trees (treeless clones), then blobs, then a
/// last round for anything the read needed that the first estimate missed.
const MAX_ROUNDS: usize = 4;
/// The length of a SHA-1 and of a SHA-256 object id in hex.
const SHA1_HEX_LEN: usize = 40;
const SHA256_HEX_LEN: usize = 64;

/// Whether `e` is libgit2 failing to find an object in the object database.
pub(crate) fn is_missing_object(e: &git2::Error) -> bool {
    e.code() == ErrorCode::NotFound && e.class() == ErrorClass::Odb
}

/// The promisor remotes, in the order git tries them.
///
/// Every `remote.<name>.promisor = true` in configuration order, then the one
/// `extensions.partialClone` names (git moves it to the end, `promisor_remote_init` in
/// `promisor-remote.c`).
pub fn promisor_remotes(repo: &Repository) -> Vec<String> {
    let Ok(config) = repo.config().and_then(|mut c| c.snapshot()) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    if let Ok(mut entries) = config.entries(Some(r"remote\..*\.promisor")) {
        while let Some(Ok(entry)) = entries.next() {
            let Ok(key) = entry.name() else { continue };
            let Some(name) = key
                .strip_prefix("remote.")
                .and_then(|k| k.strip_suffix(".promisor"))
            else {
                continue;
            };
            // Later entries override earlier ones, so look the final value up.
            if config.get_bool(key).unwrap_or(false) && !out.iter().any(|n| n == name) {
                out.push(name.to_owned());
            }
        }
    }
    if let Ok(name) = config.get_string("extensions.partialclone")
        && !name.is_empty()
    {
        out.retain(|n| *n != name);
        out.push(name);
    }
    out
}

/// A remote name that would be read as an option if passed to `git fetch` (for example
/// `--upload-pack=<command>`, which runs the command). Such a remote is never fetched from.
fn is_refused(name: &str) -> bool {
    name.starts_with('-')
}

/// The remotes of `remotes` that may be fetched from.
fn usable(remotes: &[String]) -> impl Iterator<Item = &String> {
    remotes.iter().filter(|n| !is_refused(n))
}

/// The first promisor remote that may be fetched from.
pub(crate) fn usable_promisor_remote(repo: &Repository) -> Option<String> {
    usable(&promisor_remotes(repo)).next().cloned()
}

/// Why the promisor remotes `remotes` (all of them refused) cannot be used.
fn refused_reason(remotes: &[String]) -> String {
    let names: Vec<String> = remotes.iter().map(|n| format!("{n:?}")).collect();
    format!(
        "The promisor remote name {} was refused because it starts with \"-\" and git would \
         read it as an option. Rename the remote.",
        names.join(", ")
    )
}

/// Whether the repository's working folder and all Git configuration directories lie inside one of the
/// `trusted` folders.
///
/// A download runs `git fetch` in the repository, and git then obeys the repository's own
/// configuration, which can name programs to run (`core.sshCommand`, a credential helper) and
/// servers to contact. Canopy therefore downloads without being asked only for a repository the
/// user placed in a folder they trust. The git directory counts too: a repository whose `.git`
/// file points elsewhere takes its configuration from there. Linked worktrees also read a
/// private git directory's `config.worktree` when `extensions.worktreeConfig` is enabled.
///
/// Every path is resolved first (symbolic links, `..`), so a link or a relative spelling can
/// neither enter nor leave a trusted folder, and `/a/bc` is not inside `/a/b`. A trusted folder
/// that cannot be resolved (removed, unreadable) trusts nothing, and a filesystem root is never
/// accepted as a trusted folder. Folders are compared by identity, not spelling (see
/// [`paths_in_trusted_folder`]).
pub fn is_in_trusted_folder(repo: &Repository, trusted: &[std::path::PathBuf]) -> bool {
    paths_in_trusted_folder(repo.commondir(), repo.path(), repo.workdir(), trusted)
}

/// [`is_in_trusted_folder`] for the common git directory, this worktree's own git directory
/// (which may hold `config.worktree`), and its working folder when present.
///
/// A folder is known by what it is, not by how it is spelled: a path is inside a trusted folder
/// when one of its resolved ancestors is that same folder on disk (the same file on the same
/// device). So `~/SRC` is `~/src` on a volume that ignores capitalization, and is a different
/// folder, which trusts nothing, on one that does not; no spelling is ever folded or guessed.
pub fn paths_in_trusted_folder(
    common_dir: &Path,
    git_dir: &Path,
    workdir: Option<&Path>,
    trusted: &[std::path::PathBuf],
) -> bool {
    let trusted: Vec<same_file::Handle> = trusted
        .iter()
        .filter_map(|t| std::fs::canonicalize(t).ok())
        .filter(|t| t.parent().is_some())
        .filter_map(|t| same_file::Handle::from_path(t).ok())
        .collect();
    let inside = |path: &Path| {
        std::fs::canonicalize(path).is_ok_and(|resolved| {
            resolved.ancestors().any(|folder| {
                same_file::Handle::from_path(folder).is_ok_and(|folder| trusted.contains(&folder))
            })
        })
    };
    inside(common_dir) && inside(git_dir) && workdir.is_none_or(inside)
}

/// What decides, right before each `git fetch` starts, whether the repository (its common git
/// directory, worktree-private git directory and working folder) may download. The
/// application installs it.
type DownloadGate = fn(common_dir: &Path, git_dir: &Path, workdir: Option<&Path>) -> bool;

static DOWNLOAD_GATE: std::sync::OnceLock<DownloadGate> = std::sync::OnceLock::new();

/// Installs the rule every download passes at the very point where `git fetch` would start, so that
/// no read path can reach a fetch around it, however many remotes or rounds one read takes.
///
/// Only the application knows what the user trusts. Until a rule is installed downloads are not
/// restricted; the first call wins.
pub fn set_download_gate(gate: DownloadGate) {
    let _ = DOWNLOAD_GATE.set(gate);
}

fn download_allowed(repo: &Repository) -> bool {
    DOWNLOAD_GATE
        .get()
        .is_none_or(|gate| gate(repo.commondir(), repo.path(), repo.workdir()))
}

/// Whether `repo` is a partial clone: it has a promisor remote, or objects that came from one
/// (`objects/pack/*.promisor`), which is still true after that remote was removed.
pub fn is_partial_clone(repo: &Repository) -> bool {
    !promisor_remotes(repo).is_empty() || has_promisor_packs(repo)
}

fn has_promisor_packs(repo: &Repository) -> bool {
    let Ok(entries) = std::fs::read_dir(repo.commondir().join("objects").join("pack")) else {
        return false;
    };
    entries
        .flatten()
        .any(|e| Path::new(&e.file_name()).extension() == Some("promisor".as_ref()))
}

/// The value of `extensions.partialClone` when the repository at `path` uses that setting, which
/// Git before 2.24 wrote and libgit2 refuses to open. Read from the common directory, which holds
/// the configuration of a linked worktree too.
pub(crate) fn legacy_extension(path: &Path) -> Option<String> {
    extension(path, "extensions.partialclone")
}

/// The value of repository extension `key` in the repository at `path`, read from its common
/// configuration file without opening it (libgit2 refuses to open one with an unknown extension).
pub(crate) fn extension(path: &Path, key: &str) -> Option<String> {
    let gitdir = Repository::discover_path(path, std::iter::empty::<&Path>()).ok()?;
    let common = std::fs::read_to_string(gitdir.join("commondir"))
        .ok()
        .map(|rel| gitdir.join(rel.trim()))
        .unwrap_or(gitdir);
    let config = git2::Config::open(&common.join("config")).ok()?;
    config.get_string(key).ok().filter(|v| !v.is_empty())
}

/// Converts a partial clone made by Git before 2.24 to the current form and returns its remote.
///
/// The remote that `extensions.partialClone` names gets `promisor = true` and the old setting is
/// removed, the two commands [`Error::LegacyPartialClone`] suggests, run through `git` in the
/// repository at `path` (so for a linked worktree they change the shared configuration). Refuses a repository that does not use the old setting, and a remote name no key can hold.
pub fn convert_legacy_partial_clone(path: &Path) -> Result<String> {
    let remote = legacy_extension(path).ok_or_else(|| {
        Error::Invalid(
            "This repository does not use the old extensions.partialClone setting.".into(),
        )
    })?;
    if remote.chars().any(char::is_control) {
        return Err(Error::Invalid(
            "The remote that extensions.partialClone names has a name Canopy will not write; edit the repository's .git/config by hand.".into(),
        ));
    }
    cli::run(
        path,
        &["config", &format!("remote.{remote}.promisor"), "true"],
    )?;
    cli::run(path, &["config", "--unset", "extensions.partialClone"])?;
    Ok(remote)
}

/// Whether the object database has `oid`, without rescanning the pack directory: for checking
/// many objects after one [`refresh`].
pub(crate) fn has_object(repo: &Repository, oid: Oid) -> bool {
    repo.odb()
        .is_ok_and(|odb| odb.exists_ext(oid, OdbLookupFlags::NO_REFRESH))
}

/// Whether the object database has `oid`, rescanning the pack directory when it seems missing,
/// in case `git` added a pack after this handle listed them.
fn has_object_fresh(repo: &Repository, oid: Oid) -> bool {
    repo.odb().is_ok_and(|odb| odb.exists(oid))
}

/// Makes libgit2 see packs `git` wrote since this handle listed them.
fn refresh(repo: &Repository) {
    if let Ok(odb) = repo.odb() {
        let _ = odb.refresh();
    }
}

/// The blob IDs a diff refers to that are not on disk. `sides` picks which sides are stored
/// objects: a working-tree side holds hashes of files, never objects to download.
pub(crate) fn missing_blobs(
    repo: &Repository,
    diff: &git2::Diff<'_>,
    old_side: bool,
    new_side: bool,
) -> Vec<Oid> {
    refresh(repo);
    let mut out = Vec::new();
    for delta in diff.deltas() {
        let files = [(old_side, delta.old_file()), (new_side, delta.new_file())];
        for (wanted, file) in files {
            let id = file.id();
            // Submodule entries (gitlinks) name commits of another repository.
            if !wanted || id.is_zero() || file.mode() == git2::FileMode::Commit {
                continue;
            }
            if !out.contains(&id) && !has_object(repo, id) {
                out.push(id);
            }
        }
    }
    out
}

/// The object ID libgit2 names in a "not found" message, as a last resort when the needed
/// objects could not be worked out in advance. Only a full ID counts: `Oid::from_str` also
/// accepts shorter hex strings, which are not objects to fetch.
fn oid_in_message(e: &git2::Error) -> Option<Oid> {
    let msg = e.message();
    let (_, after) = msg.rsplit_once('(')?;
    let (id, _) = after.split_once(')')?;
    let full = matches!(id.len(), SHA1_HEX_LEN | SHA256_HEX_LEN)
        && id.bytes().all(|b| b.is_ascii_hexdigit());
    if full { Oid::from_str(id).ok() } else { None }
}

/// The error for a read that needs objects this partial clone does not have. `remote` is the
/// first remote that may be fetched from; when every promisor remote was refused it is `None` and
/// the reason says so.
pub(crate) fn not_downloaded(repo: &Repository, reason: Option<String>) -> Error {
    let all = promisor_remotes(repo);
    let remote = usable(&all).next().cloned();
    let reason =
        reason.or_else(|| (remote.is_none() && !all.is_empty()).then(|| refused_reason(&all)));
    Error::NotDownloaded { remote, reason }
}

/// `e` as the failure of a download: one that outlasted [`DOWNLOAD_LIMIT`] is
/// [`Error::NotDownloaded`] naming the limit, so it can be retried; any other error is itself.
fn timed_out(repo: &Repository, e: Error) -> Error {
    match e {
        Error::Invalid(message) if message.contains("did not answer within") => not_downloaded(
            repo,
            Some(format!(
                "The download stopped after {DOWNLOAD_MINUTES} minutes without finishing. {message}"
            )),
        ),
        other => other,
    }
}

/// Fetches `oids` from the promisor remotes, trying each in turn until every object is present.
pub(crate) fn download(repo: &Repository, workdir: &Path, oids: &[Oid]) -> Result<()> {
    let remotes = promisor_remotes(repo);
    if remotes.is_empty() {
        return Err(Error::NotDownloaded {
            remote: None,
            reason: Some("No promisor remote is configured to download it from.".into()),
        });
    }
    if usable(&remotes).next().is_none() {
        return Err(not_downloaded(repo, None));
    }
    let mut wanted: Vec<Oid> = oids.to_vec();
    let mut last_error = None;
    for remote in usable(&remotes) {
        // Asked again for every remote: trust may be withdrawn while an earlier fetch runs.
        if !download_allowed(repo) {
            return Err(Error::NotDownloaded {
                remote: None,
                reason: Some("The repository is not in a folder trusted for downloads.".into()),
            });
        }
        let input = wanted.iter().fold(String::new(), |mut out, o| {
            let _ = writeln!(out, "{o}");
            out
        });
        let args = [
            "-c",
            "fetch.negotiationAlgorithm=noop",
            "fetch",
            remote.as_str(),
            "--no-tags",
            "--no-write-fetch-head",
            "--recurse-submodules=no",
            "--filter=blob:none",
            "--stdin",
        ];
        tracing::debug!(
            remote = %cli::redact_url(remote),
            objects = wanted.len(),
            "downloading missing objects"
        );
        match cli::run_with_input_within(workdir, &args, input.as_bytes(), DOWNLOAD_LIMIT) {
            Ok(_) => {}
            Err(Error::Cli { stderr, .. }) => {
                last_error = Some(stderr);
                continue;
            }
            // A download that took too long is a failed download (Retry asks again, and the
            // app remembers it), not a reason to start the next round for another ten minutes.
            Err(e) => return Err(timed_out(repo, e)),
        }
        // A successful fetch wrote a new pack; let libgit2 see it.
        refresh(repo);
        wanted.retain(|o| !has_object(repo, *o));
        if wanted.is_empty() {
            return Ok(());
        }
    }
    Err(not_downloaded(
        repo,
        Some(
            last_error.unwrap_or_else(|| {
                "The remote did not send every object that is needed.".to_owned()
            }),
        ),
    ))
}

/// Runs `read`; if it fails on an object a partial clone left on the server, downloads what
/// `needed` reports (when `missing` allows it) and runs `read` again.
///
/// `needed` returns the objects to fetch next, or `Ok(vec![])` when it cannot tell; it runs only
/// on the slow path and may itself fail on a missing object (such as a tree of a treeless clone
/// it has not seen yet), in which case the object named by that error is fetched.
pub(crate) fn read_with_download<T>(
    repo: &Repository,
    workdir: &Path,
    missing: MissingObjects,
    mut read: impl FnMut() -> Result<T>,
    mut needed: impl FnMut() -> Result<Vec<Oid>>,
) -> Result<T> {
    let first = match read() {
        Err(Error::Git(e)) if is_missing_object(&e) => e,
        other => return other,
    };
    if !is_partial_clone(repo) {
        // A missing object in a full clone is corruption, not something to download.
        return Err(Error::Git(first));
    }
    if missing == MissingObjects::Fail {
        return Err(not_downloaded(repo, None));
    }
    let mut last = first;
    for _ in 0..MAX_ROUNDS {
        let mut oids = match needed() {
            Ok(oids) => oids,
            Err(Error::Git(e)) if is_missing_object(&e) => oid_in_message(&e).into_iter().collect(),
            Err(e) => return Err(e),
        };
        if oids.is_empty() {
            oids.extend(oid_in_message(&last).filter(|o| !has_object_fresh(repo, *o)));
        }
        if oids.is_empty() {
            break;
        }
        download(repo, workdir, &oids)?;
        match read() {
            Err(Error::Git(e)) if is_missing_object(&e) => last = e,
            other => return other,
        }
    }
    tracing::debug!(
        error = last.message(),
        "objects still missing after download"
    );
    Err(not_downloaded(
        repo,
        Some("Some objects are still missing after downloading from the remote.".into()),
    ))
}

/// The trees of `commit` and its first parent that are not on disk (a treeless clone).
pub(crate) fn missing_commit_trees(repo: &Repository, commit: &git2::Commit<'_>) -> Vec<Oid> {
    let mut ids = vec![commit.tree_id()];
    if let Some(parent) = commit.parent_ids().next()
        && let Ok(p) = repo.find_commit(parent)
    {
        ids.push(p.tree_id());
    }
    ids.retain(|id| !has_object_fresh(repo, *id));
    ids
}

/// The tree `HEAD` points to when it is missing (a treeless clone after a reset or checkout).
pub(crate) fn missing_head_tree(repo: &Repository) -> Option<Oid> {
    let tree = repo.head().ok()?.peel_to_commit().ok()?.tree_id();
    (!has_object_fresh(repo, tree)).then_some(tree)
}

#[cfg(test)]
mod tests {
    use crate::test_support::isolate_git;
    /// A download that outlasted its limit can be retried; other errors pass through.
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "the test builds its repository with libgit2"
    )]
    fn a_download_that_timed_out_can_be_retried() {
        isolate_git!(a_download_that_timed_out_can_be_retried);
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = Repository::init(dir.path()).expect("init");
        repo.remote("origin", "https://example.com/r.git")
            .expect("remote");
        repo.config()
            .and_then(|mut c| c.set_bool("remote.origin.promisor", true))
            .expect("promisor");
        let timeout = Error::Invalid("git fetch did not answer within 600 seconds.".into());
        let err = timed_out(&repo, timeout);
        assert!(err.can_download(), "{err}");
        assert!(err.to_string().contains("10 minutes"), "{err}");
        let other = timed_out(&repo, Error::Invalid("something else".into()));
        assert!(matches!(other, Error::Invalid(m) if m == "something else"));
    }

    use super::*;

    fn not_found(msg: &str) -> git2::Error {
        git2::Error::new(ErrorCode::NotFound, ErrorClass::Odb, msg)
    }

    #[test]
    #[expect(clippy::string_slice, reason = "object IDs are ASCII hex")]
    fn only_a_full_object_id_in_a_message_is_taken() {
        isolate_git!(only_a_full_object_id_in_a_message_is_taken);
        let full = "0123456789abcdef0123456789abcdef01234567";
        let named = |id: &str| {
            oid_in_message(&not_found(&format!(
                "object not found - no match for id ({id})"
            )))
        };
        assert_eq!(named(full), Some(Oid::from_str(full).unwrap()));
        // `Oid::from_str` accepts prefixes, which are not objects to fetch.
        assert_eq!(named("0123abcd"), None);
        assert_eq!(named(&full[..39]), None);
        assert_eq!(named("0123456789abcdef0123456789abcdef0123456z"), None);
        assert_eq!(named(""), None);
        assert_eq!(oid_in_message(&not_found("no parentheses")), None);
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "the test builds its repository with libgit2"
    )]
    fn promisor_remotes_follow_gits_order_with_the_extension_last() {
        isolate_git!(promisor_remotes_follow_gits_order_with_the_extension_last);
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = Repository::init(dir.path()).expect("init");
        let mut config = repo.config().expect("config");
        config.set_bool("remote.first.promisor", true).unwrap();
        config.set_bool("remote.off.promisor", false).unwrap();
        config.set_bool("remote.second.promisor", true).unwrap();
        assert_eq!(promisor_remotes(&repo), ["first", "second"]);

        // `extensions.partialClone` is tried last, whether or not it also has the flag.
        config.set_str("extensions.partialclone", "first").unwrap();
        assert_eq!(promisor_remotes(&repo), ["second", "first"]);
        config.set_str("extensions.partialclone", "legacy").unwrap();
        assert_eq!(promisor_remotes(&repo), ["first", "second", "legacy"]);
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "the test builds its repository with libgit2"
    )]
    fn a_repository_is_trusted_only_inside_a_trusted_folder() {
        isolate_git!(a_repository_is_trusted_only_inside_a_trusted_folder);
        let root = tempfile::tempdir().expect("tempdir");
        let work = root.path().join("work");
        let repo_dir = work.join("repo");
        std::fs::create_dir_all(&repo_dir).unwrap();
        let repo = Repository::init(&repo_dir).expect("init");
        let trust = |folders: &[&Path]| {
            let folders: Vec<_> = folders.iter().map(|p| p.to_path_buf()).collect();
            is_in_trusted_folder(&repo, &folders)
        };

        assert!(!trust(&[]), "nothing is trusted by default");
        assert!(trust(&[&repo_dir]), "the repository's own folder");
        assert!(trust(&[&work]), "its parent");
        assert!(
            trust(&[&work.join("..").join("work")]),
            "an unresolved spelling"
        );

        // A sibling whose name merely starts the same is a different folder.
        let sibling = root.path().join("wor");
        std::fs::create_dir_all(&sibling).unwrap();
        assert!(!trust(&[&sibling]));
        let child = repo_dir.join("sub");
        std::fs::create_dir_all(&child).unwrap();
        assert!(
            !trust(&[&child]),
            "a folder inside the repository does not trust it"
        );

        // A folder that is gone trusts nothing, even if the entry is still listed.
        assert!(!trust(&[&root.path().join("removed")]));
        assert!(!trust(&[&root.path().join("removed"), &sibling]));
        assert!(trust(&[&sibling, &work]));

        // A filesystem root is never accepted, whatever it contains.
        let mut anchor = root.path().to_path_buf();
        while anchor.pop() {}
        assert!(!trust(&[&anchor]));
    }

    /// A folder written with other capitalization is the same folder on a volume that ignores
    /// capitalization and another folder (here: none) on one that does not. The test asks the
    /// volume, so it holds on Linux, macOS and Windows alike.
    #[test]
    fn a_folder_spelled_in_other_capitals_is_trusted_only_where_the_volume_says_it_is_the_same() {
        isolate_git!(
            a_folder_spelled_in_other_capitals_is_trusted_only_where_the_volume_says_it_is_the_same
        );
        let root = tempfile::tempdir().expect("tempdir");
        let work = root.path().join("Work");
        let repo = work.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let spelled = root.path().join("wORK");
        let folds_case = spelled.exists();
        assert_eq!(
            paths_in_trusted_folder(&repo, &repo, Some(&repo), &[spelled]),
            folds_case
        );
        assert!(paths_in_trusted_folder(&repo, &repo, Some(&repo), &[work]));
    }

    /// A trailing separator, `.` and a `..` that returns into the folder all name the same
    /// trusted folder, and one that leaves it does not.
    #[test]
    fn spellings_of_a_trusted_folder_that_resolve_to_it_trust_the_same() {
        isolate_git!(spellings_of_a_trusted_folder_that_resolve_to_it_trust_the_same);
        let root = tempfile::tempdir().expect("tempdir");
        let work = root.path().join("work");
        let repo = work.join("repo");
        let other = root.path().join("other");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let trust = |folder: std::path::PathBuf| {
            paths_in_trusted_folder(&repo, &repo, Some(&repo), &[folder])
        };
        assert!(trust(std::path::PathBuf::from(format!(
            "{}/",
            work.display()
        ))));
        assert!(trust(work.join(".")));
        assert!(trust(other.join("..").join("work")));
        assert!(!trust(work.join("..").join("other")));
    }

    #[test]
    fn a_private_git_directory_outside_trust_refuses_downloads() {
        isolate_git!(a_private_git_directory_outside_trust_refuses_downloads);
        let root = tempfile::tempdir().expect("tempdir");
        let trusted = root.path().join("trusted");
        let common = trusted.join("main.git");
        let work = trusted.join("worktree");
        let private = root
            .path()
            .join("outside")
            .join("worktrees")
            .join("feature");
        for path in [&common, &work, &private] {
            std::fs::create_dir_all(path).expect("test folder");
        }
        let folders = [trusted];
        assert!(
            paths_in_trusted_folder(&common, &common, Some(&work), &folders),
            "all Git configuration and worktree folders are inside the trusted root"
        );
        assert!(
            !paths_in_trusted_folder(&common, &private, Some(&work), &folders),
            "the worktree's private config cannot run outside trust"
        );
    }

    #[cfg(unix)]
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "the test builds its repository with libgit2"
    )]
    fn a_trusted_folder_that_is_a_link_to_the_root_trusts_nothing() {
        isolate_git!(a_trusted_folder_that_is_a_link_to_the_root_trusts_nothing);
        let dir = tempfile::tempdir().expect("tempdir");
        let repo_dir = dir.path().join("repo");
        std::fs::create_dir_all(&repo_dir).unwrap();
        let repo = Repository::init(&repo_dir).expect("init");
        let link = dir.path().join("everything");
        std::os::unix::fs::symlink("/", &link).unwrap();
        assert!(!is_in_trusted_folder(&repo, &[link]));
    }

    #[cfg(unix)]
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "the test builds its repository with libgit2"
    )]
    fn trust_follows_where_the_repository_really_lives() {
        isolate_git!(trust_follows_where_the_repository_really_lives);
        let root = tempfile::tempdir().expect("tempdir");
        let trusted = root.path().join("trusted");
        let elsewhere = root.path().join("elsewhere");
        std::fs::create_dir_all(&trusted).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();

        // A repository outside the trusted folder, reached through a link inside it.
        let outside = elsewhere.join("repo");
        std::fs::create_dir_all(&outside).unwrap();
        Repository::init(&outside).expect("init");
        std::os::unix::fs::symlink(&outside, trusted.join("link")).unwrap();
        let through_link = Repository::open(trusted.join("link")).expect("open through link");
        assert!(!is_in_trusted_folder(
            &through_link,
            std::slice::from_ref(&trusted)
        ));

        // A trusted folder that is itself a link counts as the folder it leads to.
        std::os::unix::fs::symlink(&elsewhere, root.path().join("alias")).unwrap();
        assert!(is_in_trusted_folder(
            &through_link,
            &[root.path().join("alias")]
        ));

        // A link elsewhere that leads into the trusted folder: the repository (and the
        // configuration git would follow) really is inside it, however it was reached.
        let inside = trusted.join("real");
        std::fs::create_dir_all(&inside).unwrap();
        Repository::init(&inside).expect("init");
        std::os::unix::fs::symlink(&inside, elsewhere.join("shortcut")).unwrap();
        let through_shortcut = Repository::open(elsewhere.join("shortcut")).expect("open");
        assert!(is_in_trusted_folder(
            &through_shortcut,
            std::slice::from_ref(&trusted)
        ));
    }

    #[test]
    fn option_like_remote_names_are_not_usable() {
        isolate_git!(option_like_remote_names_are_not_usable);
        let names = [
            "--upload-pack=x".to_owned(),
            "origin".to_owned(),
            "-x".to_owned(),
        ];
        assert_eq!(usable(&names).collect::<Vec<_>>(), ["origin"]);
        let reason = refused_reason(&[names[0].clone()]);
        assert!(
            reason.contains("refused") && reason.contains("--upload-pack=x"),
            "{reason}"
        );
    }
}
