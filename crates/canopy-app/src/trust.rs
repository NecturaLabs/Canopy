//! The folders whose repositories may download missing objects without being asked.
//!
//! In a partial clone a download runs `git fetch`, which obeys the repository's own git
//! configuration (programs to run, servers to contact). Canopy does it on its own only for a
//! repository inside a folder the user trusted in Settings (`Settings::trusted_folders`).
//! Repository threads read the list from here, so a change applies to every open tab at once.

use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

static FOLDERS: RwLock<Vec<PathBuf>> = RwLock::new(Vec::new());
static CHANGES: AtomicU64 = AtomicU64::new(0);

/// Replaces the trusted folders, for every repository thread at once: call it the moment the
/// list changes, so a download that has not started yet sees a removal. Returns whether the
/// list changed.
pub fn set(folders: &[PathBuf]) -> bool {
    if *read() == folders {
        return false;
    }
    *FOLDERS
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = folders.to_vec();
    CHANGES.fetch_add(1, Ordering::SeqCst);
    true
}

/// How many times the list has changed, for whoever must react to a change once.
pub fn changes() -> u64 {
    CHANGES.load(Ordering::SeqCst)
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
        .filter(|parent| !is_root(parent))
        .map(Path::to_path_buf)
}

/// Whether `path` names a filesystem root once `.` and `..` are read lexically.
fn is_root(path: &Path) -> bool {
    use std::path::Component;
    let mut depth = 0usize;
    for part in path.components() {
        match part {
            Component::Normal(_) => depth += 1,
            Component::ParentDir => depth = depth.saturating_sub(1),
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
        }
    }
    depth == 0
}

/// Gives core the rule every download passes right where `git fetch` would start, so that no
/// read path reaches a fetch around the trust list. Safe to call again.
pub fn install_gate() {
    canopy_core::partial::set_download_gate(|common_dir, workdir| {
        canopy_core::partial::paths_in_trusted_folder(common_dir, workdir, &folders())
    });
}

/// Adds `folder` to `list` unless it is already there. A filesystem root is refused, whether
/// spelled `/`, `C:\` or `/a/../..`. This is a reading of the path, without touching the disk
/// (it runs on the UI thread); core resolves links when it checks, and a link that leads to a
/// root trusts nothing there.
pub fn add(list: &mut Vec<PathBuf>, folder: PathBuf) -> bool {
    if is_root(&folder) || list.contains(&folder) {
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

    #[cfg(unix)]
    #[test]
    fn a_path_that_reads_as_the_root_is_refused_like_the_root() {
        let mut list = Vec::new();
        assert!(!add(&mut list, PathBuf::from("/..")));
        assert!(!add(&mut list, PathBuf::from("/a/../..")));
        assert!(!add(&mut list, PathBuf::from("/./")));
        assert!(list.is_empty());
        assert!(add(&mut list, PathBuf::from("/a/../b")));
    }
}
