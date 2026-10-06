//! Preferences persisted between runs through eframe's storage
//! (`~/.local/share/canopy/` on Linux). Every field has a default (`#[serde(default)]`), so
//! settings written by an older version load with the new fields at their defaults.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use canopy_core::Whitespace;
use canopy_core::ops::PullStrategy;

use crate::ui::motion::MotionPref;
use crate::ui::theme::{Density, Style, ThemeId};

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
    pub const ALL: [Self; 3] = [Self::Merge, Self::Rebase, Self::FastForwardOnly];

    pub fn strategy(self) -> PullStrategy {
        match self {
            Self::Merge => PullStrategy::Merge,
            Self::Rebase => PullStrategy::Rebase,
            Self::FastForwardOnly => PullStrategy::FastForwardOnly,
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

impl CloneProtocol {
    pub const ALL: [Self; 2] = [Self::Https, Self::Ssh];

    pub fn label(self) -> &'static str {
        match self {
            Self::Https => "HTTPS",
            Self::Ssh => "SSH",
        }
    }
}

/// One repository's own choices. Each is `None` while the repository follows the default in
/// [`Settings`] (its "Default for repositories" value).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RepoSettings {
    pub pull_mode: Option<PullMode>,
    pub prune_on_fetch: Option<bool>,
    /// Minutes between background fetches; `Some(0)` turns them off for this repository.
    pub auto_fetch_minutes: Option<u32>,
    pub signoff: Option<bool>,
    /// Protocol of remote URLs built from `owner/repo` (Add remote, Publish).
    pub protocol: Option<CloneProtocol>,
}

impl RepoSettings {
    /// Whether the repository follows every default.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
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
    pub const ALL: [Self; 3] = [Self::Show, Self::IgnoreEol, Self::IgnoreAll];

    pub fn core(self) -> Whitespace {
        match self {
            Self::Show => Whitespace::Show,
            Self::IgnoreEol => Whitespace::IgnoreEol,
            Self::IgnoreAll => Whitespace::IgnoreAll,
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
    /// ISO calendar dates in History and the commit inspector.
    pub iso_dates: bool,
    /// Avatars next to authors.
    pub show_avatars: bool,
    /// Fetch account pictures after consent; initials remain when disabled.
    pub download_avatars: bool,
    /// Standard (concept 1) or Deeper (concept 4) surfaces and geometry.
    pub style: Style,
    /// Blurred translucent floating layers.
    pub translucent: bool,
    /// Row heights, radii, toolbar layout and type size.
    pub density: Density,
    /// Match the system, always reduce, or always animate.
    pub motion: MotionPref,
    /// Pre-0.3 switch, read once by `migrate` and never written again.
    #[serde(skip_serializing)]
    pub reduce_motion: Option<bool>,

    // Git.
    pub pull_mode: PullMode,
    /// `--prune` on fetch: drop remote-tracking branches deleted on the remote.
    pub prune_on_fetch: bool,
    /// Fetch in the background every N minutes (0 = off).
    pub auto_fetch_minutes: u32,
    /// `git commit --signoff`.
    pub signoff: bool,
    /// The one-time Git identity question has been shown.
    pub identity_asked: bool,
    /// Commits loaded per page in the history.
    pub commit_page: usize,
    /// Default diff options.
    pub diff: DiffPrefs,
    /// Per-repository diff options, keyed by working directory.
    pub repo_diff: HashMap<PathBuf, DiffPrefs>,
    /// Per-repository choices that differ from the defaults above, keyed by working directory.
    pub repos: HashMap<PathBuf, RepoSettings>,
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
    /// Editor command for "Open in editor"; the file is added as the last argument. Empty uses
    /// the platform's text editor.
    pub editor: String,
    /// Load pull/merge requests from GitHub and GitLab.
    pub hosting_api: bool,
    /// Folders whose repositories may download missing objects in a partial clone without being
    /// asked, because git then runs the repository's own configuration (see `trust.rs`).
    pub trusted_folders: Vec<PathBuf>,

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
    /// The `crash_reports::DATA_VERSION` the answer was given for.
    pub crash_reports_version: u32,
    /// How much the local log records.
    pub log_level: crate::logging::LogLevel,

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
            iso_dates: false,
            show_avatars: true,
            download_avatars: true,
            style: Style::Standard,
            translucent: false,
            density: Density::Comfortable,
            motion: MotionPref::System,
            reduce_motion: None,
            pull_mode: PullMode::Merge,
            prune_on_fetch: true,
            auto_fetch_minutes: 0,
            signoff: false,
            identity_asked: false,
            commit_page: 2000,
            diff: DiffPrefs::default(),
            repo_diff: HashMap::new(),
            repos: HashMap::new(),
            split_diff: false,
            syntax_highlight: true,
            clone_dir: String::new(),
            clone_protocol: CloneProtocol::Https,
            gitlab_hosts: String::new(),
            terminal: String::new(),
            editor: String::new(),
            hosting_api: true,
            trusted_folders: Vec::new(),
            check_updates: true,
            include_prereleases: false,
            last_update_check: 0,
            skipped_version: None,
            last_seen_version: String::new(),
            accepted_terms: String::new(),
            accepted_privacy: String::new(),
            crash_reports: None,
            crash_reports_version: 0,
            log_level: crate::logging::LogLevel::Normal,
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
        // A file with `reduce_motion` predates `motion`, so this overwrites nothing chosen.
        if self.reduce_motion.take() == Some(true) {
            self.motion = MotionPref::Reduced;
        }
        self.zoom = self.zoom.clamp(0.8, 1.6);
        self.commit_page = self.commit_page.clamp(200, 50_000);
        self.diff.context = self.diff.context.min(25);
        let mut seen = Vec::new();
        self.trusted_folders
            .retain(|f| f.is_absolute() && crate::trust::add(&mut seen, f));
        // Only real choices are kept; a repository following every default needs no entry.
        self.repos
            .retain(|path, repo| path.is_absolute() && !repo.is_default());
    }

    /// The repository's own choices, or the defaults when it has none.
    pub fn repo(&self, path: &Path) -> RepoSettings {
        self.repos.get(path).cloned().unwrap_or_default()
    }

    /// The repository's own choices, to change them.
    pub fn repo_mut(&mut self, path: &Path) -> &mut RepoSettings {
        self.repos.entry(path.to_path_buf()).or_default()
    }

    /// How a pull integrates in the repository at `path`.
    pub fn pull_mode_for(&self, path: &Path) -> PullMode {
        self.repo(path).pull_mode.unwrap_or(self.pull_mode)
    }

    /// Whether fetching in the repository at `path` prunes.
    pub fn prune_for(&self, path: &Path) -> bool {
        self.repo(path)
            .prune_on_fetch
            .unwrap_or(self.prune_on_fetch)
    }

    /// Minutes between background fetches of the repository at `path` (0 = off).
    pub fn auto_fetch_for(&self, path: &Path) -> u32 {
        self.repo(path)
            .auto_fetch_minutes
            .unwrap_or(self.auto_fetch_minutes)
    }

    /// Whether commits in the repository at `path` get a Signed-off-by trailer.
    pub fn signoff_for(&self, path: &Path) -> bool {
        self.repo(path).signoff.unwrap_or(self.signoff)
    }

    /// The protocol of URLs built from `owner/repo` for the repository at `path`.
    pub fn protocol_for(&self, path: &Path) -> CloneProtocol {
        self.repo(path).protocol.unwrap_or(self.clone_protocol)
    }

    /// Repositories with a choice of their own (settings or diff options).
    pub fn repos_with_overrides(&self) -> usize {
        let mut paths: Vec<&PathBuf> = self
            .repos
            .iter()
            .filter(|(_, r)| !r.is_default())
            .map(|(p, _)| p)
            .chain(self.repo_diff.keys())
            .collect();
        paths.sort();
        paths.dedup();
        paths.len()
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

/// A path typed by the user, with a leading `~` (alone, or followed by `/`, or `\` on Windows)
/// standing for the home folder. `~user` is left alone: Canopy does not look up other users'
/// folders. Surrounding whitespace is trimmed.
pub fn expand_home(typed: &str) -> PathBuf {
    expand_home_in(typed, &home_dir())
}

fn expand_home_in(typed: &str, home: &Path) -> PathBuf {
    let typed = typed.trim();
    let Some(rest) = typed.strip_prefix('~') else {
        return PathBuf::from(typed);
    };
    if home.as_os_str().is_empty() {
        return PathBuf::from(typed);
    }
    let separator = |c: char| c == '/' || (cfg!(windows) && c == '\\');
    if rest.is_empty() {
        return home.to_path_buf();
    }
    match rest.strip_prefix(separator) {
        Some(below) => {
            let below = below.trim_start_matches(separator);
            if below.is_empty() {
                home.to_path_buf()
            } else {
                home.join(below)
            }
        }
        None => PathBuf::from(typed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midnight_is_the_default_theme() {
        // The owner chose Midnight as Canopy's look: a fresh install, an empty settings file and
        // settings saved before themes existed all start on it.
        assert_eq!(Settings::default().theme, ThemeId::Midnight);
        assert_eq!(ThemeId::default(), ThemeId::Midnight);
        let mut empty: Settings = serde_json::from_str("{}").expect("parse");
        empty.migrate();
        assert_eq!(empty.theme, ThemeId::Midnight);
        let mut old_dark: Settings = serde_json::from_str(r#"{"dark": true}"#).expect("parse");
        old_dark.migrate();
        assert_eq!(old_dark.theme, ThemeId::Midnight);
    }

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
    fn old_reduce_motion_true_maps_to_reduced() {
        let mut s: Settings = serde_json::from_str(r#"{"reduce_motion": true}"#).expect("parse");
        s.migrate();
        assert_eq!(s.motion, MotionPref::Reduced);
        assert_eq!(s.reduce_motion, None);
    }

    #[test]
    fn old_reduce_motion_false_keeps_system() {
        let mut s: Settings = serde_json::from_str(r#"{"reduce_motion": false}"#).expect("parse");
        s.migrate();
        assert_eq!(s.motion, MotionPref::System);
    }

    #[test]
    fn new_fields_default() {
        let mut s: Settings = serde_json::from_str("{}").expect("parse");
        s.migrate();
        assert_eq!(s.style, Style::Standard);
        assert!(!s.translucent);
        assert_eq!(s.density, Density::Comfortable);
        assert_eq!(s.motion, MotionPref::System);
        let d = Settings::default();
        assert_eq!(
            (d.style, d.translucent, d.density, d.motion),
            (
                Style::Standard,
                false,
                Density::Comfortable,
                MotionPref::System
            )
        );
    }

    #[test]
    fn reduce_motion_is_never_written() {
        let s = Settings {
            reduce_motion: Some(true),
            style: Style::Deeper,
            density: Density::Dense,
            translucent: true,
            motion: MotionPref::Full,
            ..Settings::default()
        };
        let json = serde_json::to_value(&s).expect("serialise");
        assert!(json.get("reduce_motion").is_none(), "{json}");
        let back: Settings = serde_json::from_value(json).expect("parse");
        assert_eq!(
            (back.style, back.density, back.translucent, back.motion),
            (Style::Deeper, Density::Dense, true, MotionPref::Full)
        );
    }

    #[test]
    fn a_repository_follows_the_defaults_until_it_chooses() {
        let repo = PathBuf::from("/home/ada/src/canopy");
        let other = PathBuf::from("/home/ada/src/other");
        let mut s = Settings {
            pull_mode: PullMode::Rebase,
            prune_on_fetch: false,
            auto_fetch_minutes: 15,
            signoff: true,
            clone_protocol: CloneProtocol::Ssh,
            ..Settings::default()
        };
        // Nothing chosen: every value is the default.
        assert_eq!(s.pull_mode_for(&repo), PullMode::Rebase);
        assert!(!s.prune_for(&repo));
        assert_eq!(s.auto_fetch_for(&repo), 15);
        assert!(s.signoff_for(&repo));
        assert_eq!(s.protocol_for(&repo), CloneProtocol::Ssh);
        // A repository's choice wins for it alone.
        *s.repo_mut(&repo) = RepoSettings {
            pull_mode: Some(PullMode::FastForwardOnly),
            prune_on_fetch: Some(true),
            auto_fetch_minutes: Some(0),
            signoff: Some(false),
            protocol: Some(CloneProtocol::Https),
        };
        assert_eq!(s.pull_mode_for(&repo), PullMode::FastForwardOnly);
        assert!(s.prune_for(&repo));
        assert_eq!(
            s.auto_fetch_for(&repo),
            0,
            "Some(0) turns background fetch off"
        );
        assert!(!s.signoff_for(&repo));
        assert_eq!(s.protocol_for(&repo), CloneProtocol::Https);
        assert_eq!(s.pull_mode_for(&other), PullMode::Rebase);
        assert_eq!(s.protocol_for(&other), CloneProtocol::Ssh);
        // A changed default reaches the repositories that follow it, not the others.
        s.pull_mode = PullMode::Merge;
        assert_eq!(s.pull_mode_for(&other), PullMode::Merge);
        assert_eq!(s.pull_mode_for(&repo), PullMode::FastForwardOnly);
    }

    #[cfg(unix)]
    #[test]
    fn repository_choices_survive_a_round_trip_and_old_files_load() {
        let repo = PathBuf::from("/srv/git/app");
        let mut s = Settings::default();
        s.repo_mut(&repo).protocol = Some(CloneProtocol::Ssh);
        s.repo_mut(&repo).pull_mode = Some(PullMode::Rebase);
        let json = serde_json::to_string(&s).expect("serialise");
        let mut back: Settings = serde_json::from_str(&json).expect("parse");
        back.migrate();
        assert_eq!(back.repo(&repo), s.repo(&repo));
        // A file from before per-repository settings: every repository follows the defaults.
        let mut old: Settings =
            serde_json::from_str(r#"{"clone_protocol": "Ssh", "pull_mode": "Rebase"}"#)
                .expect("parse");
        old.migrate();
        assert!(old.repos.is_empty());
        assert_eq!(old.protocol_for(&repo), CloneProtocol::Ssh);
        assert_eq!(old.pull_mode_for(&repo), PullMode::Rebase);
        // An entry with a field this version does not know still loads.
        let mut newer: Settings = serde_json::from_str(
            r#"{"repos": {"/srv/git/app": {"signoff": true, "from_the_future": 1}}}"#,
        )
        .expect("parse");
        newer.migrate();
        assert!(newer.signoff_for(&repo));
    }

    #[cfg(unix)]
    #[test]
    fn empty_and_relative_repository_entries_are_dropped() {
        let mut s = Settings::default();
        s.repo_mut(Path::new("/srv/git/kept")).signoff = Some(true);
        s.repo_mut(Path::new("/srv/git/reset"));
        s.repo_mut(Path::new("relative/repo")).signoff = Some(true);
        s.migrate();
        let kept: Vec<&PathBuf> = s.repos.keys().collect();
        assert_eq!(kept, [&PathBuf::from("/srv/git/kept")]);
        s.repo_diff
            .insert(PathBuf::from("/srv/git/kept"), DiffPrefs::default());
        s.repo_diff
            .insert(PathBuf::from("/srv/git/diff"), DiffPrefs::default());
        assert_eq!(s.repos_with_overrides(), 2);
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

    #[cfg(unix)]
    #[test]
    fn trusted_folders_keep_only_distinct_absolute_non_root_paths() {
        let mut s: Settings = serde_json::from_str(
            r#"{"trusted_folders": ["/home/ada/src", "relative/dir", "/", "/home/ada/src", "/srv/git"]}"#,
        )
        .expect("parse");
        s.migrate();
        assert_eq!(
            s.trusted_folders,
            [PathBuf::from("/home/ada/src"), PathBuf::from("/srv/git")]
        );
        assert!(
            Settings::default().trusted_folders.is_empty(),
            "nothing is trusted until the user says so"
        );
    }

    #[test]
    fn tilde_expands_to_the_home_folder_only() {
        let home = Path::new("/home/ada");
        let e = |typed: &str| expand_home_in(typed, home);
        assert_eq!(e("~"), PathBuf::from("/home/ada"));
        assert_eq!(e("~/"), PathBuf::from("/home/ada"));
        assert_eq!(e("~/src/canopy"), PathBuf::from("/home/ada/src/canopy"));
        assert_eq!(e("  ~/src  "), PathBuf::from("/home/ada/src"));
        assert_eq!(e("~//src"), PathBuf::from("/home/ada/src"));
        // Another user's folder and names that merely start with a tilde stay as typed.
        assert_eq!(e("~bob/src"), PathBuf::from("~bob/src"));
        assert_eq!(e("~backup"), PathBuf::from("~backup"));
        assert_eq!(e("/srv/~/x"), PathBuf::from("/srv/~/x"));
        assert_eq!(e("relative/dir"), PathBuf::from("relative/dir"));
        // Without a known home folder nothing is expanded.
        assert_eq!(
            expand_home_in("~/src", Path::new("")),
            PathBuf::from("~/src")
        );
    }
}
