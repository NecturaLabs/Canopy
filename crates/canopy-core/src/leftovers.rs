//! What a killed Canopy leaves behind: staging files and folders whose owner is gone.
//!
//! Canopy writes some files through a staging name and a rename ([`crate::paths::write_atomic`]
//! beside a file in the working tree, the patch export's staging folder). A process that dies
//! between the two leaves the staging name behind. The names carry the process id, so a later run
//! can tell a leftover from a file another running Canopy is writing right now.
//!
//! Nothing here deletes a file in a folder that belongs to the user. [`Repo::abandoned_staging_files`]
//! only reports; [`sweep_temp_dirs`] removes folders Canopy made in the system's temporary folder.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::{Repo, Result};

/// Where the process id sits in a staging name made by [`staging_file_name`].
const FILE_MARK: &str = ".canopy-";
const FILE_SUFFIX: &str = ".tmp";

/// How long a staging file must have been untouched before it counts as abandoned where the
/// system cannot say whether a process id is in use (everywhere but Linux). Writing one takes
/// milliseconds.
const UNKNOWN_OWNER_AGE: Duration = Duration::from_mins(60);

/// The start of the name of the folder a patch export stages its files in, in the system's
/// temporary folder, followed by the process id and a count.
pub const PATCH_STAGING_PREFIX: &str = "canopy-patches-";

/// How many abandoned files one report names at most.
const MAX_REPORTED: usize = 200;

/// The name `write_atomic` stages `name` under: hidden, with the process id and the attempt.
pub(crate) fn staging_file_name(name: &str, pid: u32, attempt: u32) -> String {
    format!(".{name}{FILE_MARK}{pid}-{attempt}{FILE_SUFFIX}")
}

/// The process id in a name made by [`staging_file_name`], and `None` for any other name.
#[must_use]
pub fn staging_file_owner(file_name: &str) -> Option<u32> {
    let rest = file_name.strip_prefix('.')?.strip_suffix(FILE_SUFFIX)?;
    let (name, ids) = rest.rsplit_once(FILE_MARK)?;
    let (pid, attempt) = ids.split_once('-')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if name.is_empty() || !digits(pid) || !digits(attempt) {
        return None;
    }
    pid.parse().ok()
}

/// The process id after `prefix` in the name of a folder or file Canopy staged
/// (`<prefix><pid>-…`), and `None` for any other name.
#[must_use]
pub fn owner_after_prefix(file_name: &str, prefix: &str) -> Option<u32> {
    let rest = file_name.strip_prefix(prefix)?;
    let (pid, tail) = rest.split_once('-')?;
    if tail.is_empty() || pid.is_empty() || !pid.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    pid.parse().ok()
}

/// Whether the process that staged something is certainly gone.
///
/// It is not this process, and on Linux no `/proc/<pid>` exists; elsewhere, where a process id
/// cannot be looked up without a system call, the thing has not been touched for an hour.
#[must_use]
pub fn owner_is_gone(pid: u32, modified: Option<SystemTime>) -> bool {
    if pid == std::process::id() {
        return false;
    }
    let proc = Path::new("/proc");
    if cfg!(target_os = "linux") && proc.join("self").exists() {
        return !proc.join(pid.to_string()).exists();
    }
    untouched_for_long(modified, SystemTime::now())
}

fn untouched_for_long(modified: Option<SystemTime>, now: SystemTime) -> bool {
    modified
        .and_then(|m| now.duration_since(m).ok())
        .is_some_and(|age| age >= UNKNOWN_OWNER_AGE)
}

/// Removes the folders in `temp` named `<prefix><pid>-…` for one of `prefixes` whose process is
/// gone, and returns them. A name that does not match, a plain file and a link are left alone.
pub fn sweep_temp_dirs(temp: &Path, prefixes: &[&str]) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(temp) else {
        return Vec::new();
    };
    let mut removed = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(pid) = prefixes.iter().find_map(|p| owner_after_prefix(&name, p)) else {
            continue;
        };
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_dir() || !owner_is_gone(pid, meta.modified().ok()) {
            continue;
        }
        match std::fs::remove_dir_all(&path) {
            Ok(()) => removed.push(path),
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "could not remove a leftover staging folder");
            }
        }
    }
    removed
}

impl Repo {
    /// The staging files an interrupted write left in the working tree: files named like
    /// [`crate::paths::write_atomic`]'s whose process is gone, ignored ones included (`*.tmp` is
    /// a common rule). Only reports them; the user decides what happens to them. At most 200.
    pub fn abandoned_staging_files(&self) -> Result<Vec<PathBuf>> {
        let mut options = git2::StatusOptions::new();
        options
            .include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_ignored(true)
            .recurse_ignored_dirs(false)
            .include_unmodified(false);
        let statuses = self.git().statuses(Some(&mut options))?;
        let mut found = Vec::new();
        for entry in statuses.iter() {
            let path = crate::paths::os_path(entry.path_bytes());
            let Some(pid) = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(staging_file_owner)
            else {
                continue;
            };
            let full = self.workdir().join(&path);
            let Ok(meta) = std::fs::symlink_metadata(&full) else {
                continue;
            };
            if meta.is_file() && owner_is_gone(pid, meta.modified().ok()) {
                found.push(full);
                if found.len() == MAX_REPORTED {
                    break;
                }
            }
        }
        found.sort();
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_staging_name_gives_back_its_process() {
        let name = staging_file_name("a.rs", 4242, 3);
        assert_eq!(name, ".a.rs.canopy-4242-3.tmp");
        assert_eq!(staging_file_owner(&name), Some(4242));
        // A file name that itself holds the marker still parses from its end.
        assert_eq!(
            staging_file_owner(&staging_file_name("x.canopy-7-1.tmp", 9, 0)),
            Some(9)
        );
    }

    #[test]
    fn other_names_are_not_staging_names() {
        for name in [
            "a.rs",
            ".a.rs.canopy-1-0.bak",
            "a.rs.canopy-1-0.tmp",
            ".canopy-1-0.tmp",
            ".a.canopy-x-0.tmp",
            ".a.canopy-1-.tmp",
            ".a.canopy--0.tmp",
            ".a.canopy-1.tmp",
            ".a.canopy-99999999999-0.tmp",
            "",
        ] {
            assert_eq!(staging_file_owner(name), None, "{name:?}");
        }
    }

    #[test]
    fn a_folder_name_gives_back_its_process() {
        assert_eq!(
            owner_after_prefix("canopy-update.12-345-0", "canopy-update."),
            Some(12)
        );
        assert_eq!(
            owner_after_prefix("canopy-patches-8-1", "canopy-patches-"),
            Some(8)
        );
        for name in [
            "canopy-patches-",
            "canopy-patches-8",
            "canopy-patches-x-1",
            "other-8-1",
        ] {
            assert_eq!(
                owner_after_prefix(name, "canopy-patches-"),
                None,
                "{name:?}"
            );
        }
    }

    #[test]
    fn this_process_is_never_gone() {
        assert!(!owner_is_gone(std::process::id(), None));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_running_process_is_not_gone_and_an_unused_id_is() {
        // Process 1 always runs; no process has the largest id.
        assert!(!owner_is_gone(1, None));
        assert!(owner_is_gone(u32::MAX, None));
    }

    #[test]
    fn without_a_process_table_only_age_decides() {
        let now = SystemTime::now();
        assert!(!untouched_for_long(None, now));
        assert!(!untouched_for_long(Some(now - Duration::from_mins(5)), now));
        assert!(untouched_for_long(Some(now - Duration::from_mins(61)), now));
        assert!(!untouched_for_long(Some(now + Duration::from_mins(5)), now));
    }
}
