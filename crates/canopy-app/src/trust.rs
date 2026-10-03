//! The folders whose repositories may download missing objects without being asked.
//!
//! In a partial clone a download runs `git fetch`, which obeys the repository's own git
//! configuration (programs to run, servers to contact). Canopy does it on its own only for a
//! repository inside a folder the user trusted in Settings (`Settings::trusted_folders`).
//! Repository threads read the list from here, so a change applies to every open tab at once.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

static FOLDERS: RwLock<Vec<PathBuf>> = RwLock::new(Vec::new());

/// Replaces the trusted folders. Returns whether the list changed.
pub fn set(folders: &[PathBuf]) -> bool {
    if *read() == folders {
        return false;
    }
    *FOLDERS
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = folders.to_vec();
    true
}

/// Trusts `folder` for the rest of the test run. Unlike [`set`] it adds to the list under one
/// lock, so tests running in parallel cannot lose each other's folders.
#[cfg(test)]
pub fn add_for_tests(folder: PathBuf) {
    let mut list = FOLDERS
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    list.push(folder);
}

/// The trusted folders.
pub fn folders() -> Vec<PathBuf> {
    read().clone()
}

fn read() -> std::sync::RwLockReadGuard<'static, Vec<PathBuf>> {
    FOLDERS
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The folder to offer next to `repo`'s own: its parent, unless that is a filesystem root, which
/// can never be trusted.
pub fn parent_offer(repo: &Path) -> Option<PathBuf> {
    repo.parent()
        .filter(|parent| parent.parent().is_some())
        .map(Path::to_path_buf)
}

/// Adds `folder` to `list` unless it is already there. A filesystem root is refused.
pub fn add(list: &mut Vec<PathBuf>, folder: PathBuf) -> bool {
    if folder.parent().is_none() || list.contains(&folder) {
        return false;
    }
    list.push(folder);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_repository_is_offered_its_parent_but_never_a_root() {
        assert_eq!(
            parent_offer(Path::new("/home/ada/src/repo")),
            Some(PathBuf::from("/home/ada/src"))
        );
        assert_eq!(parent_offer(Path::new("/repo")), None);
        assert_eq!(parent_offer(Path::new("/")), None);
    }

    #[cfg(unix)]
    #[test]
    fn folders_are_added_once_and_a_root_is_refused() {
        let mut list = Vec::new();
        assert!(add(&mut list, PathBuf::from("/home/ada/src")));
        assert!(!add(&mut list, PathBuf::from("/home/ada/src")));
        assert!(!add(&mut list, PathBuf::from("/")));
        assert_eq!(list, [PathBuf::from("/home/ada/src")]);
    }
}
