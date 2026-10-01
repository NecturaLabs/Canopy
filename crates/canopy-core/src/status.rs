//! Working tree status, split into the staged, unstaged and conflicted lists a staging area shows.
//!
//! Untracked folders are listed file by file up to [`UNTRACKED_LIMIT`] untracked files in total;
//! above it each top-most untracked folder becomes one [`UntrackedDir`] (with its file count), so
//! an unignored `node_modules` cannot flood the list. [`untracked_children`] lists one such folder
//! a level at a time. A folder holding a Git repository of its own (not a submodule) is always an
//! [`UntrackedDir`] with `repository` set.

use std::collections::BTreeMap;
use std::path::Path;

use git2::{Repository, Status, StatusOptions, StatusShow};

use crate::diff::mode_of;
use crate::model::{ChangeKind, FileEntry, UntrackedDir, UntrackedListing, WorkingStatus};
use crate::paths::os_path;
use crate::{Error, Result, lfs};

/// The number of untracked files above which [`collect`] lists untracked folders collapsed.
pub const UNTRACKED_LIMIT: usize = 2000;

fn entry(path: String, old_path: Option<String>, kind: ChangeKind) -> FileEntry {
    FileEntry {
        path,
        old_path,
        kind,
        additions: 0,
        deletions: 0,
        lfs: false,
        old_mode: None,
        new_mode: None,
    }
}

fn with_modes(mut e: FileEntry, delta: Option<git2::DiffDelta<'_>>) -> FileEntry {
    if let Some(d) = delta {
        e.old_mode = mode_of(&d.old_file());
        e.new_mode = mode_of(&d.new_file());
    }
    e
}

/// Whether the untracked folder `dir` (relative, as raw bytes, possibly ending in `/`) holds a
/// repository of its own.
fn is_repository(workdir: Option<&Path>, dir: &[u8]) -> bool {
    let Some(root) = workdir else {
        return false;
    };
    let dir = dir.strip_suffix(b"/").unwrap_or(dir);
    std::fs::symlink_metadata(root.join(os_path(dir)).join(".git")).is_ok()
}

/// Reads the status of every changed, untracked or conflicted file, collapsing untracked folders
/// above [`UNTRACKED_LIMIT`] untracked files.
pub fn collect(repo: &Repository) -> Result<WorkingStatus> {
    collect_with(repo, UNTRACKED_LIMIT)
}

/// [`collect`] with an explicit limit: untracked folders are expanded into their files while
/// the total number of untracked files stays at or below `untracked_limit`.
pub fn collect_with(repo: &Repository, untracked_limit: usize) -> Result<WorkingStatus> {
    let mut opts = StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(false)
        .include_ignored(false)
        .renames_head_to_index(true)
        .exclude_submodules(false);
    let statuses = repo.statuses(Some(&mut opts))?;

    let mut out = WorkingStatus::default();
    let mut dirs = Vec::new();
    for e in statuses.iter() {
        let s = e.status();
        let path = String::from_utf8_lossy(e.path_bytes()).into_owned();

        if s.is_conflicted() {
            out.conflicted
                .push(entry(path, None, ChangeKind::Conflicted));
            continue;
        }
        if s.contains(Status::WT_NEW) && path.ends_with('/') {
            dirs.push((path, e.path_bytes().to_vec()));
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
            out.staged.push(with_modes(
                entry(new_path.unwrap_or_else(|| path.clone()), old_path, kind),
                e.head_to_index(),
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
            out.unstaged
                .push(with_modes(entry(path, None, kind), e.index_to_workdir()));
        }
    }

    let workdir = repo.workdir();
    let mut plain = Vec::new();
    for (dir, raw) in dirs {
        if is_repository(workdir, &raw) {
            out.untracked_dirs.push(UntrackedDir {
                path: dir,
                files: 0,
                repository: true,
                contains_repository: false,
            });
        } else {
            plain.push((dir, raw));
        }
    }
    if !plain.is_empty() {
        let raw: Vec<&[u8]> = plain.iter().map(|(_, r)| r.as_slice()).collect();
        let inside = untracked_under(repo, &raw)?;
        let untracked_files = out
            .unstaged
            .iter()
            .filter(|e| e.kind == ChangeKind::Untracked)
            .count()
            + inside.files.len();
        if untracked_files <= untracked_limit {
            out.unstaged.extend(inside.files);
            out.untracked_dirs.extend(inside.repositories);
        } else {
            for (dir, _) in plain {
                let files = inside
                    .files
                    .iter()
                    .filter(|f| f.path.starts_with(&dir))
                    .count();
                let contains_repository =
                    inside.repositories.iter().any(|r| r.path.starts_with(&dir));
                out.untracked_dirs.push(UntrackedDir {
                    path: dir,
                    files,
                    repository: false,
                    contains_repository,
                });
            }
        }
    }

    for list in [&mut out.staged, &mut out.unstaged, &mut out.conflicted] {
        list.sort_by(|a, b| a.path.cmp(&b.path));
    }
    out.untracked_dirs.sort_by(|a, b| a.path.cmp(&b.path));
    lfs::mark(repo, &mut out);
    Ok(out)
}

/// Untracked files below some folders, and the repositories of their own found among them.
struct Inside {
    files: Vec<FileEntry>,
    repositories: Vec<UntrackedDir>,
}

/// Lists every untracked, unignored file below the untracked folders `dirs` (each relative and
/// ending in `/`). Folders holding a repository of their own are returned as such, not entered.
fn untracked_under(repo: &Repository, dirs: &[&[u8]]) -> Result<Inside> {
    let mut opts = StatusOptions::new();
    opts.show(StatusShow::Workdir)
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false)
        .exclude_submodules(true)
        .disable_pathspec_match(true);
    for d in dirs {
        opts.pathspec(d.strip_suffix(b"/").unwrap_or(d));
    }
    let statuses = repo.statuses(Some(&mut opts))?;
    let mut inside = Inside {
        files: Vec::new(),
        repositories: Vec::new(),
    };
    let workdir = repo.workdir();
    for e in statuses.iter() {
        if !e.status().contains(Status::WT_NEW) {
            continue;
        }
        let path = String::from_utf8_lossy(e.path_bytes()).into_owned();
        if path.ends_with('/') {
            inside.repositories.push(UntrackedDir {
                repository: is_repository(workdir, e.path_bytes()),
                path,
                files: 0,
                contains_repository: false,
            });
        } else {
            inside.files.push(with_modes(
                entry(path, None, ChangeKind::Untracked),
                e.index_to_workdir(),
            ));
        }
    }
    Ok(inside)
}

/// Lists the untracked folder `dir` (relative to the working tree, with or without a trailing
/// `/`) one level deep: the untracked files directly in it, and each subfolder as an
/// [`UntrackedDir`] with its untracked file count. Ignored files are left out.
pub fn untracked_children(repo: &Repository, dir: &str) -> Result<UntrackedListing> {
    crate::paths::check_relative(dir)?;
    let base = format!("{}/", dir.trim_end_matches('/'));
    if is_repository(repo.workdir(), base.as_bytes()) {
        return Err(Error::invalid(format!(
            "{base} is a repository of its own. Open it in a tab to see its files."
        )));
    }
    let inside = untracked_under(repo, &[base.as_bytes()])?;
    let mut listing = UntrackedListing::default();
    let mut subdirs: BTreeMap<String, UntrackedDir> = BTreeMap::new();
    for f in inside.files {
        let Some(rest) = f.path.strip_prefix(&base) else {
            continue;
        };
        match rest.split_once('/') {
            None => listing.files.push(f),
            Some((first, _)) => {
                let path = format!("{base}{first}/");
                subdirs
                    .entry(path.clone())
                    .or_insert(UntrackedDir {
                        path,
                        files: 0,
                        repository: false,
                        contains_repository: false,
                    })
                    .files += 1;
            }
        }
    }
    for r in inside.repositories {
        let Some(rest) = r.path.strip_prefix(&base) else {
            continue;
        };
        match rest.trim_end_matches('/').split_once('/') {
            None => {
                subdirs.insert(r.path.clone(), r);
            }
            Some((first, _)) => {
                let path = format!("{base}{first}/");
                subdirs
                    .entry(path.clone())
                    .or_insert(UntrackedDir {
                        path,
                        files: 0,
                        repository: false,
                        contains_repository: false,
                    })
                    .contains_repository = true;
            }
        }
    }
    listing.dirs = subdirs.into_values().collect();
    listing.files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(listing)
}
