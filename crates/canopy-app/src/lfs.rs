//! What a repository tab knows about Git LFS: the files stored with it and whether their objects
//! are downloaded, whether `git lfs` works, and the lists the Git LFS dialog reads.

use std::collections::HashSet;
use std::sync::Arc;

use canopy_core::{LfsLock, LfsPattern};

use crate::worker::LfsInfo;

/// The most paths the Git LFS dialog lists as not downloaded.
pub const LISTED_FILES: usize = 100;

/// Bytes as a person reads them: `12 B`, `3.4 KB`, `560 MB`.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    const STEP: f64 = 1024.0;
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "a size shown with one decimal needs no more precision than f64's"
    )]
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= STEP && unit + 1 < UNITS.len() {
        value /= STEP;
        unit += 1;
    }
    if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// An object id as the card shows it: `sha256:` and its first twelve digits.
pub fn short_oid(oid: &str) -> String {
    format!("sha256:{}", oid.get(..12).unwrap_or(oid))
}

/// The tab's view of Git LFS, from the repository thread's answers.
#[derive(Debug, Default)]
pub struct LfsState {
    info: Arc<LfsInfo>,
    pointer_only: HashSet<String>,
    /// Bumped for every request of the patterns and the locks; an older answer is dropped.
    patterns_serial: u64,
    locks_serial: u64,
    /// The newest answer of [`Self::request_patterns`], `None` while it is on its way.
    pub patterns: Option<Result<Vec<LfsPattern>, String>>,
    /// The newest answer of [`Self::request_locks`], `None` while it is on its way.
    pub locks: Option<Result<Vec<LfsLock>, String>>,
    /// The Git LFS dialog was opened: its lists are read again after an LFS command.
    pub dialog_seen: bool,
    /// The diff card shows the pointer text.
    pub show_pointer: bool,
}

impl LfsState {
    pub fn set(&mut self, info: Arc<LfsInfo>) {
        self.pointer_only = info
            .files
            .iter()
            .filter(|f| !f.downloaded)
            .map(|f| f.path.clone())
            .collect();
        self.info = info;
    }

    /// The repository stores files with LFS.
    pub fn in_use(&self) -> bool {
        self.info.in_use
    }

    /// Every LFS file of the index.
    pub fn files(&self) -> &[canopy_core::LfsFile] {
        &self.info.files
    }

    /// Whether the working tree holds only the pointer of `path`, its object not being in the
    /// local store.
    pub fn is_pointer_only(&self, path: &str) -> bool {
        self.pointer_only.contains(path)
    }

    /// How many files have no object downloaded.
    pub fn not_downloaded(&self) -> usize {
        self.pointer_only.len()
    }

    /// Why Git LFS commands cannot run, with the steps that fix it; `None` when they can.
    pub fn unavailable(&self) -> Option<String> {
        self.info.missing.as_ref().map(|tool| {
            format!(
                "git-lfs is not installed, so downloading, tracking and locking are off. {}",
                tool.install.join(" ")
            )
        })
    }

    pub fn next_patterns_serial(&mut self) -> u64 {
        self.patterns_serial = self.patterns_serial.wrapping_add(1);
        self.patterns = None;
        self.patterns_serial
    }

    pub fn next_locks_serial(&mut self) -> u64 {
        self.locks_serial = self.locks_serial.wrapping_add(1);
        self.locks = None;
        self.locks_serial
    }

    pub fn accept_patterns(&mut self, serial: u64, result: Result<Vec<LfsPattern>, String>) {
        if serial == self.patterns_serial {
            self.patterns = Some(result);
        }
    }

    pub fn accept_locks(&mut self, serial: u64, result: Result<Vec<LfsLock>, String>) {
        if serial == self.locks_serial {
            self.locks = Some(result);
        }
    }
}

#[cfg(test)]
mod tests {
    use canopy_core::{LfsFile, LfsPointer, MissingTool};

    use super::*;

    fn file(path: &str, downloaded: bool) -> LfsFile {
        LfsFile {
            path: path.into(),
            pointer: LfsPointer {
                oid: "a".repeat(64),
                size: 10,
                extensions: false,
            },
            downloaded,
        }
    }

    #[test]
    fn sizes_read_as_a_person_would() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(human_size(560 * 1024 * 1024), "560 MB");
        assert_eq!(human_size(u64::MAX), "16777216 TB");
    }

    #[test]
    fn an_object_id_is_cut_to_twelve_digits() {
        assert_eq!(short_oid(&"ab".repeat(32)), "sha256:abababababab");
        assert_eq!(short_oid("abc"), "sha256:abc");
    }

    #[test]
    fn pointer_only_files_are_the_ones_without_an_object() {
        let mut lfs = LfsState::default();
        assert!(!lfs.in_use() && lfs.not_downloaded() == 0);
        lfs.set(Arc::new(LfsInfo {
            in_use: true,
            files: vec![file("a.bin", true), file("b.bin", false)],
            missing: None,
        }));
        assert!(lfs.in_use());
        assert!(!lfs.is_pointer_only("a.bin") && lfs.is_pointer_only("b.bin"));
        assert_eq!(lfs.not_downloaded(), 1);
        assert_eq!(lfs.unavailable(), None);
    }

    #[test]
    fn a_missing_git_lfs_names_the_install_steps() {
        let mut lfs = LfsState::default();
        lfs.set(Arc::new(LfsInfo {
            in_use: true,
            files: Vec::new(),
            missing: Some(MissingTool {
                program: "git-lfs".into(),
                needed_for: "Git LFS files",
                install: vec!["Install git-lfs.".into(), "Restart.".into()],
            }),
        }));
        let reason = lfs.unavailable().expect("a reason");
        assert!(reason.contains("not installed") && reason.contains("Install git-lfs. Restart."));
    }

    #[test]
    fn an_answer_for_an_older_request_is_dropped() {
        let mut lfs = LfsState::default();
        let old = lfs.next_locks_serial();
        let new = lfs.next_locks_serial();
        lfs.accept_locks(old, Err("old".into()));
        assert!(lfs.locks.is_none());
        lfs.accept_locks(new, Ok(Vec::new()));
        assert!(matches!(lfs.locks, Some(Ok(_))));
        let old = lfs.next_patterns_serial();
        let _ = lfs.next_patterns_serial();
        lfs.accept_patterns(old, Ok(Vec::new()));
        assert!(lfs.patterns.is_none());
    }
}
