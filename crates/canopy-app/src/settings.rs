//! Preferences persisted between runs through eframe's storage
//! (`~/.local/share/canopy/` on Linux).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Storage key.
pub const KEY: &str = "canopy-settings-v1";

/// User preferences and session state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Dark ("Midnight") or light ("Daylight") theme.
    pub dark: bool,
    /// Recently opened repositories, most recent first.
    pub recent: Vec<PathBuf>,
    /// Repositories open when Canopy last closed, restored on start.
    pub open_tabs: Vec<PathBuf>,
    /// Pull with `--rebase` when `pull.rebase` is not configured.
    pub pull_rebase: bool,
    /// Commits loaded per page in the history.
    pub commit_page: usize,
    /// Interface zoom.
    pub zoom: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            dark: true,
            recent: Vec::new(),
            open_tabs: Vec::new(),
            pull_rebase: false,
            commit_page: 2000,
            zoom: 1.0,
        }
    }
}

impl Settings {
    /// Moves `path` to the front of the recent list.
    pub fn touch_recent(&mut self, path: PathBuf) {
        self.recent.retain(|p| p != &path);
        self.recent.insert(0, path);
        self.recent.truncate(12);
    }
}
