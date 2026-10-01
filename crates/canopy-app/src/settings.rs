//! Preferences persisted between runs through eframe's storage
//! (`~/.local/share/canopy/` on Linux). Every field has a default (`#[serde(default)]`), so
//! settings written by an older version load with the new fields at their defaults.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use canopy_core::Whitespace;
use canopy_core::ops::PullStrategy;

use crate::ui::theme::ThemeId;

/// Storage key.
pub const KEY: &str = "canopy-settings-v1";

/// How `git pull` integrates when `pull.rebase` is not configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PullMode {
    #[default]
    Merge,
    Rebase,
    FastForwardOnly,
}

impl PullMode {
    pub const ALL: [PullMode; 3] = [PullMode::Merge, PullMode::Rebase, PullMode::FastForwardOnly];

    pub fn strategy(self) -> PullStrategy {
        match self {
            PullMode::Merge => PullStrategy::Merge,
            PullMode::Rebase => PullStrategy::Rebase,
            PullMode::FastForwardOnly => PullStrategy::FastForwardOnly,
        }
    }

    pub fn label(self) -> &'static str {
        self.strategy().label()
    }
}

/// Protocol of clone URLs built from `owner/repo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CloneProtocol {
    #[default]
    Https,
    Ssh,
}

/// Whitespace handling in diffs (a serialisable mirror of [`Whitespace`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum WhitespaceMode {
    #[default]
    Show,
    IgnoreEol,
    IgnoreAll,
}

impl WhitespaceMode {
    pub const ALL: [WhitespaceMode; 3] = [
        WhitespaceMode::Show,
        WhitespaceMode::IgnoreEol,
        WhitespaceMode::IgnoreAll,
    ];

    pub fn core(self) -> Whitespace {
        match self {
            WhitespaceMode::Show => Whitespace::Show,
            WhitespaceMode::IgnoreEol => Whitespace::IgnoreEol,
            WhitespaceMode::IgnoreAll => Whitespace::IgnoreAll,
        }
    }

    pub fn label(self) -> &'static str {
        self.core().label()
    }
}

/// Diff options. Global defaults live in [`Settings::diff`]; a repository can override them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiffPrefs {
    /// Unchanged lines around each change (0–25).
    pub context: u32,
    pub whitespace: WhitespaceMode,
}

impl Default for DiffPrefs {
    fn default() -> Self {
        Self {
            context: 3,
            whitespace: WhitespaceMode::Show,
        }
    }
}

/// User preferences and session state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    // Appearance.
    pub theme: ThemeId,
    /// Interface zoom (0.8–1.6).
    pub zoom: f32,
    /// "3 hours ago" rather than "Sep 21, 2026" in the history.
    pub relative_dates: bool,
    /// Initials avatars next to authors.
    pub show_avatars: bool,
    /// Turn off animations (hover fades, the welcome screen's drifting glow).
    pub reduce_motion: bool,

    // Git.
    pub pull_mode: PullMode,
    /// `--prune` on fetch: drop remote-tracking branches deleted on the remote.
    pub prune_on_fetch: bool,
    /// Fetch in the background every N minutes (0 = off).
    pub auto_fetch_minutes: u32,
    /// `git commit --signoff`.
    pub signoff: bool,
    /// Commits loaded per page in the history.
    pub commit_page: usize,
    /// Default diff options.
    pub diff: DiffPrefs,
    /// Per-repository diff options, keyed by working directory.
    pub repo_diff: HashMap<PathBuf, DiffPrefs>,
    /// Show diffs side by side (old left, new right) instead of unified.
    pub split_diff: bool,
    /// Colour code in diffs by language.
    pub syntax_highlight: bool,

    // Integrations.
    /// Where clones go by default.
    pub clone_dir: String,
    pub clone_protocol: CloneProtocol,
    /// Self-hosted GitLab hosts, comma-separated.
    pub gitlab_hosts: String,
    /// Terminal command for "Open terminal here", run in the repository folder.
    pub terminal: String,
    /// Load pull/merge requests from GitHub and GitLab.
    pub hosting_api: bool,

    // Updates.
    pub check_updates: bool,
    pub include_prereleases: bool,
    /// Unix time of the last update check.
    pub last_update_check: i64,
    /// A version the user chose to skip.
    pub skipped_version: Option<String>,
    /// The version that last ran, to show "What's new" once after an upgrade.
    pub last_seen_version: String,

    // Privacy.
    /// Version (date) of the Terms of Service the user accepted.
    pub accepted_terms: String,
    /// Version (date) of the Privacy Policy the user accepted.
    pub accepted_privacy: String,
    /// Send crash reports to NecturaLabs (Sentry). `None` until the user answers; nothing is
    /// sent before that.
    pub crash_reports: Option<bool>,

    // Session.
    /// Recently opened repositories, most recent first.
    pub recent: Vec<PathBuf>,
    /// Repositories open when Canopy last closed, restored on start.
    pub open_tabs: Vec<PathBuf>,
    /// Shown the first-run tips.
    pub seen_intro: bool,
    /// File name of the last crash report offered, so each is offered once.
    pub last_crash_seen: String,

    /// Pre-0.2 theme flag, read once to migrate light-theme users to Daylight.
    #[serde(skip_serializing)]
    pub dark: Option<bool>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeId::Midnight,
            zoom: 1.0,
            relative_dates: true,
            show_avatars: true,
            reduce_motion: false,
            pull_mode: PullMode::Merge,
            prune_on_fetch: true,
            auto_fetch_minutes: 0,
            signoff: false,
            commit_page: 2000,
            diff: DiffPrefs::default(),
            repo_diff: HashMap::new(),
            split_diff: false,
            syntax_highlight: true,
            clone_dir: String::new(),
            clone_protocol: CloneProtocol::Https,
            gitlab_hosts: String::new(),
            terminal: String::new(),
            hosting_api: true,
            check_updates: true,
            include_prereleases: false,
            last_update_check: 0,
            skipped_version: None,
            last_seen_version: String::new(),
            accepted_terms: String::new(),
            accepted_privacy: String::new(),
            crash_reports: None,
            recent: Vec::new(),
            open_tabs: Vec::new(),
            seen_intro: false,
            last_crash_seen: String::new(),
            dark: None,
        }
    }
}

impl Settings {
    /// Fixes up values from older versions and out-of-range edits.
    pub fn migrate(&mut self) {
        if self.dark.take() == Some(false) && self.theme == ThemeId::Midnight {
            self.theme = ThemeId::Daylight;
        }
        self.zoom = self.zoom.clamp(0.8, 1.6);
        self.commit_page = self.commit_page.clamp(200, 50_000);
        self.diff.context = self.diff.context.min(25);
    }

    /// Moves `path` to the front of the recent list.
    pub fn touch_recent(&mut self, path: PathBuf) {
        self.recent.retain(|p| p != &path);
        self.recent.insert(0, path);
        self.recent.truncate(12);
    }

    /// Self-hosted GitLab hosts as a list.
    pub fn gitlab_host_list(&self) -> Vec<String> {
        self.gitlab_hosts
            .split([',', ' ', '\n'])
            .map(str::trim)
            .filter(|h| !h.is_empty())
            .map(str::to_owned)
            .collect()
    }

    /// Diff options for the repository at `path`.
    pub fn diff_for(&self, path: &Path) -> DiffPrefs {
        self.repo_diff.get(path).copied().unwrap_or(self.diff)
    }

    /// The default clone folder: the setting, else `~/src` when it exists, else home.
    pub fn clone_parent(&self) -> String {
        if !self.clone_dir.trim().is_empty() {
            return self.clone_dir.trim().to_owned();
        }
        let home = home_dir();
        let src = home.join("src");
        if src.is_dir() {
            src.display().to_string()
        } else {
            home.display().to_string()
        }
    }
}

/// The user's home directory (empty when unknown).
pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_light_setting_migrates_to_daylight() {
        let mut s: Settings =
            serde_json::from_str(r#"{"dark": false, "zoom": 1.2}"#).expect("parse");
        s.migrate();
        assert_eq!(s.theme, ThemeId::Daylight);
        assert!((s.zoom - 1.2).abs() < f32::EPSILON);
        assert!(s.check_updates, "new fields take their defaults");
    }

    #[test]
    fn gitlab_hosts_split() {
        let s = Settings {
            gitlab_hosts: " git.example.com, code.corp.io\nlab.x ".into(),
            ..Settings::default()
        };
        assert_eq!(
            s.gitlab_host_list(),
            vec!["git.example.com", "code.corp.io", "lab.x"]
        );
    }
}
