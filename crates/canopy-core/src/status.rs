//! Working tree status, split into the staged, unstaged and conflicted lists a staging area shows.

use git2::{Repository, Status, StatusOptions};

use crate::model::{ChangeKind, FileEntry, WorkingStatus};
use crate::{Error, Result, partial};

fn entry(path: String, old_path: Option<String>, kind: ChangeKind) -> FileEntry {
    FileEntry {
        path,
        old_path,
        kind,
        additions: 0,
        deletions: 0,
    }
}

/// Reads the status of every changed, untracked or conflicted file.
///
/// Never touches the network. Detecting a staged rename compares file contents; in a partial
/// clone whose `HEAD` blobs were not downloaded (after `git reset --soft` to an old commit, or in
/// a `--no-checkout` clone) that comparison is impossible offline, so status is read again
/// without rename detection and such a rename shows as a deletion plus an addition, where
/// `git status` would download the blobs and show the rename.
///
/// When the tree of `HEAD` itself is missing (a treeless clone after `git reset --soft` to a
/// commit whose tree was never downloaded), libgit2 would treat `HEAD` as unborn and report
/// every file as added; this returns [`Error::NotDownloaded`] instead, and
/// [`crate::Repo::status`] with [`crate::MissingObjects::Download`] fetches the tree.
pub fn collect(repo: &Repository) -> Result<WorkingStatus> {
    if partial::missing_head_tree(repo).is_some() && partial::is_partial_clone(repo) {
        return Err(partial::not_downloaded(repo, None));
    }
    match statuses(repo, true) {
        Err(Error::Git(e)) if partial::is_missing_object(&e) && partial::is_partial_clone(repo) => {
            tracing::debug!(error = e.message(), "status without rename detection");
            statuses(repo, false)
        }
        other => other,
    }
}

fn statuses(repo: &Repository, renames: bool) -> Result<WorkingStatus> {
    let mut opts = StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false)
        .renames_head_to_index(renames)
        .exclude_submodules(false);
    let statuses = repo.statuses(Some(&mut opts))?;

    let mut out = WorkingStatus::default();
    for e in statuses.iter() {
        let s = e.status();
        let path = String::from_utf8_lossy(e.path_bytes()).into_owned();

        if s.is_conflicted() {
            out.conflicted
                .push(entry(path, None, ChangeKind::Conflicted));
            continue;
        }

        let staged = if s.contains(Status::INDEX_NEW) {
            Some(ChangeKind::Added)
        } else if s.contains(Status::INDEX_RENAMED) {
            Some(ChangeKind::Renamed)
        } else if s.contains(Status::INDEX_DELETED) {
            Some(ChangeKind::Deleted)
        } else if s.contains(Status::INDEX_TYPECHANGE) {
            Some(ChangeKind::Typechange)
        } else if s.contains(Status::INDEX_MODIFIED) {
            Some(ChangeKind::Modified)
        } else {
            None
        };
        if let Some(kind) = staged {
            let (new_path, old_path) = match e.head_to_index() {
                Some(d) if kind == ChangeKind::Renamed => (
                    d.new_file()
                        .path()
                        .map(|p| p.to_string_lossy().replace('\\', "/")),
                    d.old_file()
                        .path()
                        .map(|p| p.to_string_lossy().replace('\\', "/")),
                ),
                _ => (None, None),
            };
            out.staged.push(entry(
                new_path.unwrap_or_else(|| path.clone()),
                old_path,
                kind,
            ));
        }

        let unstaged = if s.contains(Status::WT_NEW) {
            Some(ChangeKind::Untracked)
        } else if s.contains(Status::WT_DELETED) {
            Some(ChangeKind::Deleted)
        } else if s.contains(Status::WT_TYPECHANGE) {
            Some(ChangeKind::Typechange)
        } else if s.contains(Status::WT_RENAMED) {
            Some(ChangeKind::Renamed)
        } else if s.contains(Status::WT_MODIFIED) {
            Some(ChangeKind::Modified)
        } else {
            None
        };
        if let Some(kind) = unstaged {
            out.unstaged.push(entry(path, None, kind));
        }
    }

    for list in [&mut out.staged, &mut out.unstaged, &mut out.conflicted] {
        list.sort_by(|a, b| a.path.cmp(&b.path));
    }
    Ok(out)
}
