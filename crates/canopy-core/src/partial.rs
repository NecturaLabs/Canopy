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

/// How many fetch-and-retry rounds a read may take: trees (treeless clones), then blobs, then a
/// last round for anything the read needed that the first estimate missed.
const MAX_ROUNDS: usize = 4;

/// Whether `e` is libgit2 failing to find an object in the object database.
pub(crate) fn is_missing_object(e: &git2::Error) -> bool {
    e.code() == ErrorCode::NotFound && e.class() == ErrorClass::Odb
}

/// The promisor remotes, in the order git tries them: `extensions.partialClone` first, then
/// every `remote.<name>.promisor = true`.
pub fn promisor_remotes(repo: &Repository) -> Vec<String> {
    let Ok(config) = repo.config().and_then(|mut c| c.snapshot()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Ok(name) = config.get_string("extensions.partialclone")
        && !name.is_empty()
    {
        out.push(name);
    }
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
    out
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
/// Git before 2.24 wrote and libgit2 refuses to open.
pub(crate) fn legacy_extension(path: &Path) -> Option<String> {
    let gitdir = Repository::discover_path(path, std::iter::empty::<&Path>()).ok()?;
    let config = git2::Config::open(&gitdir.join("config")).ok()?;
    config
        .get_string("extensions.partialclone")
        .ok()
        .filter(|v| !v.is_empty())
}

/// Whether the object database has `oid`, without rescanning the pack directory: for checking
/// many objects after one [`refresh`].
pub(crate) fn has_object(repo: &Repository, oid: Oid) -> bool {
    repo.odb()
        .map(|odb| odb.exists_ext(oid, OdbLookupFlags::NO_REFRESH))
        .unwrap_or(false)
}

/// Whether the object database has `oid`, rescanning the pack directory when it seems missing,
/// in case `git` added a pack after this handle listed them.
fn has_object_fresh(repo: &Repository, oid: Oid) -> bool {
    repo.odb().map(|odb| odb.exists(oid)).unwrap_or(false)
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
/// objects could not be worked out in advance.
fn oid_in_message(e: &git2::Error) -> Option<Oid> {
    let msg = e.message();
    let start = msg.rfind('(')? + 1;
    let end = start + msg[start..].find(')')?;
    Oid::from_str(&msg[start..end]).ok()
}

/// The error for a read that needs objects this partial clone does not have.
pub(crate) fn not_downloaded(repo: &Repository, reason: Option<String>) -> Error {
    Error::NotDownloaded {
        remote: promisor_remotes(repo).into_iter().next(),
        reason,
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
    let mut wanted: Vec<Oid> = oids.to_vec();
    let mut last_error = None;
    for remote in &remotes {
        if remote.starts_with('-') {
            continue;
        }
        let input: String = wanted.iter().map(|o| format!("{o}\n")).collect();
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
            remote,
            objects = wanted.len(),
            "downloading missing objects"
        );
        match cli::run_with_input(workdir, &args, input.as_bytes()) {
            Ok(_) => {}
            Err(Error::Cli { stderr, .. }) => {
                last_error = Some(stderr);
                continue;
            }
            Err(e) => return Err(e),
        }
        // A successful fetch wrote a new pack; let libgit2 see it.
        refresh(repo);
        wanted.retain(|o| !has_object(repo, *o));
        if wanted.is_empty() {
            return Ok(());
        }
    }
    Err(Error::NotDownloaded {
        remote: remotes.into_iter().next(),
        reason: Some(
            last_error.unwrap_or_else(|| {
                "The remote did not send every object that is needed.".to_owned()
            }),
        ),
    })
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
