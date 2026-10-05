//! The folders whose repositories may download missing objects without being asked.
//!
//! In a partial clone a download runs `git fetch`, which obeys the repository's own git
//! configuration (programs to run, servers to contact). Canopy does it on its own only for a
//! repository inside a folder the user trusted in Settings (`Settings::trusted_folders`).
//! Repository threads read the list from here, so a change applies to every open tab at once.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{PoisonError, RwLock};

/// The list and how often it has changed. A change is counted under the same lock that replaces
/// the list, so a reader never pairs a list with another list's count.
struct Trust {
    folders: RwLock<Vec<PathBuf>>,
    changes: AtomicU64,
}

static TRUST: Trust = Trust::new();

impl Trust {
    const fn new() -> Self {
        Self {
            folders: RwLock::new(Vec::new()),
            changes: AtomicU64::new(0),
        }
    }

    fn set(&self, folders: &[PathBuf]) -> bool {
        let mut list = self.folders.write().unwrap_or_else(PoisonError::into_inner);
        if *list == folders {
            return false;
        }
        *list = folders.to_vec();
        self.changes.fetch_add(1, Ordering::SeqCst);
        true
    }

    fn changes(&self) -> u64 {
        self.changes.load(Ordering::SeqCst)
    }

    /// The list together with its count.
    fn snapshot(&self) -> (u64, Vec<PathBuf>) {
        let list = self.folders.read().unwrap_or_else(PoisonError::into_inner);
        (self.changes(), list.clone())
    }

    /// What `check` says about the list as it is when the answer is given. Resolving paths takes
    /// time with no lock held (the UI thread must never wait for it), so a list that changed
    /// meanwhile makes the answer void and `check` runs again on the new one.
    fn decide(&self, check: impl Fn(&[PathBuf]) -> bool) -> bool {
        loop {
            let (seen, list) = self.snapshot();
            let answer = check(&list);
            if self.changes() == seen {
                return answer;
            }
        }
    }
}

/// Replaces the trusted folders, for every repository thread at once: call it the moment the
/// list changes, so a download that has not started yet sees a removal. Returns whether the
/// list changed.
pub fn set(folders: &[PathBuf]) -> bool {
    TRUST.set(folders)
}

/// How many times the list has changed, for whoever must react to a change once.
pub fn changes() -> u64 {
    TRUST.changes()
}

/// Trusts `folder` for the rest of the test run. Unlike [`set`] it adds to the list under one
/// lock, so tests running in parallel cannot lose each other's folders.
#[cfg(test)]
pub fn add_for_tests(folder: PathBuf) {
    let mut list = TRUST
        .folders
        .write()
        .unwrap_or_else(PoisonError::into_inner);
    list.push(folder);
}

/// The trusted folders and the version of that list (`changes()` as it stood when it was read).
pub fn snapshot() -> (u64, Vec<PathBuf>) {
    TRUST.snapshot()
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
        TRUST.decide(|folders| {
            canopy_core::partial::paths_in_trusted_folder(common_dir, workdir, folders)
        })
    });
}

/// Adds `folder`, made absolute, to `list` unless it is already there: a relative entry would
/// mean another folder from another working directory, so settings never keep one. A filesystem
/// root is refused, whether spelled `/`, `C:\` or `/a/../..`. This is a reading of the path,
/// without touching the repository's disk (it runs on the UI thread); core resolves links when it
/// checks, and a link that leads to a root trusts nothing there.
pub fn add(list: &mut Vec<PathBuf>, folder: &PathBuf) -> bool {
    let Ok(folder) = std::path::absolute(folder) else {
        return false;
    };
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
        assert!(add(&mut list, &PathBuf::from("/home/ada/src")));
        assert!(!add(&mut list, &PathBuf::from("/home/ada/src")));
        assert!(!add(&mut list, &PathBuf::from("/")));
        assert_eq!(list, [PathBuf::from("/home/ada/src")]);
    }

    #[test]
    fn a_relative_folder_is_kept_as_an_absolute_one() {
        let mut list = Vec::new();
        assert!(add(&mut list, &PathBuf::from("repo")));
        assert!(list.iter().all(|folder| folder.is_absolute()));
        assert!(list[0].ends_with("repo"));
        let kept = list[0].clone();
        assert!(!add(&mut list, &kept), "already there");
        assert!(!add(&mut list, &PathBuf::new()), "nothing to trust");
    }

    #[test]
    fn only_a_real_change_is_counted() {
        let trust = Trust::new();
        let src = PathBuf::from("src");
        assert!(trust.set(std::slice::from_ref(&src)));
        assert!(!trust.set(std::slice::from_ref(&src)));
        assert_eq!(trust.changes(), 1);
        assert!(trust.set(&[]));
        assert_eq!(trust.snapshot(), (2, Vec::new()));
    }

    #[test]
    fn a_folder_removed_while_the_gate_decides_is_not_trusted() {
        let trust = Trust::new();
        trust.set(&[PathBuf::from("src")]);
        let asked = std::cell::Cell::new(0);
        let answer = trust.decide(|folders| {
            asked.set(asked.get() + 1);
            if asked.get() == 1 {
                // The user removes the folder while the paths are being resolved.
                trust.set(&[]);
            }
            !folders.is_empty()
        });
        assert!(!answer, "the answer of a list that no longer holds");
        assert_eq!(asked.get(), 2);
    }

    #[test]
    fn a_list_nobody_changes_is_asked_once() {
        let trust = Trust::new();
        trust.set(&[PathBuf::from("src")]);
        let asked = std::cell::Cell::new(0);
        assert!(trust.decide(|folders| {
            asked.set(asked.get() + 1);
            !folders.is_empty()
        }));
        assert_eq!(asked.get(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn a_path_that_reads_as_the_root_is_refused_like_the_root() {
        let mut list = Vec::new();
        assert!(!add(&mut list, &PathBuf::from("/..")));
        assert!(!add(&mut list, &PathBuf::from("/a/../..")));
        assert!(!add(&mut list, &PathBuf::from("/./")));
        assert!(list.is_empty(), "{list:?}");
        assert!(add(&mut list, &PathBuf::from("/a/../b")));
    }
}
