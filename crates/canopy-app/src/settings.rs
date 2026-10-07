//! Preferences persisted between runs through eframe's storage
//! (`~/.local/share/canopy/` on Linux). Every field has a default (`#[serde(default)]`), so
//! settings written by an older version load with the new fields at their defaults. A saved
//! theme or option this build no longer has reads as its default, and a setting it no longer has
//! is ignored; the first save after launch (eframe saves every 30 seconds) writes the file again
//! without either.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use canopy_core::Whitespace;
use canopy_core::ops::PullStrategy;
use canopy_core::{Cleanup, ConflictStyle, GitOptions, PushDefault};

use crate::ui::kit::overlay::{TooltipDelay, TooltipLevel};
use crate::ui::motion::MotionPref;
use crate::ui::theme::{Curve, CustomTheme, Density, NodeShape, Style, ThemeId};
use crate::ui::toast::Topic;

/// Storage key.
pub const KEY: &str = "canopy-settings-v1";

/// How a pull integrates: Canopy's own choice, passed to git explicitly (a serialisable mirror of
/// [`PullStrategy`]). Variants are only ever added, so saved settings keep loading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PullMode {
    Merge,
    Rebase,
    /// The recommended default: a pull never makes a merge commit or rewrites commits by
    /// itself; when the branches diverged, Canopy asks how to bring them together.
    #[default]
    FastForwardOnly,
    MergeNoFastForward,
    RebaseMerges,
}

impl PullMode {
    /// In the order of [`PullStrategy::ALL`].
    pub const ALL: [Self; 5] = [
        Self::Merge,
        Self::MergeNoFastForward,
        Self::Rebase,
        Self::RebaseMerges,
        Self::FastForwardOnly,
    ];

    pub fn strategy(self) -> PullStrategy {
        match self {
            Self::Merge => PullStrategy::Merge,
            Self::MergeNoFastForward => PullStrategy::MergeNoFastForward,
            Self::Rebase => PullStrategy::Rebase,
            Self::RebaseMerges => PullStrategy::RebaseMerges,
            Self::FastForwardOnly => PullStrategy::FastForwardOnly,
        }
    }

    pub fn label(self) -> &'static str {
        self.strategy().label()
    }
}

/// How a repository pulls: Canopy's strategy, and whether a strategy the user's git configuration
/// sets wins over it (the default: Canopy starts from the user's git configuration).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PullChoice {
    pub mode: PullMode,
    /// Follow my git config: pull with what `branch.<name>.rebase`, `pull.rebase` or `pull.ff`
    /// ask for, and with `mode` only when they ask for nothing.
    pub follow_config: bool,
}

impl PullChoice {
    /// The strategy a pull uses, given what the git configuration asks for (`None`: nothing).
    pub fn resolve(self, configured: Option<PullStrategy>) -> PullStrategy {
        match configured {
            Some(s) if self.follow_config => s,
            _ => self.mode.strategy(),
        }
    }
}

impl Default for PullChoice {
    fn default() -> Self {
        Self {
            mode: PullMode::default(),
            follow_config: true,
        }
    }
}

/// How conflicts are written into files: a serialisable mirror of
/// [`canopy_core::ConflictStyle`]. Variants are only ever added, so saved settings keep loading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ConflictMode {
    #[default]
    Zdiff3,
    Diff3,
    Merge,
}

impl ConflictMode {
    pub const ALL: [Self; 3] = [Self::Zdiff3, Self::Diff3, Self::Merge];

    pub fn style(self) -> ConflictStyle {
        match self {
            Self::Zdiff3 => ConflictStyle::Zdiff3,
            Self::Diff3 => ConflictStyle::Diff3,
            Self::Merge => ConflictStyle::Merge,
        }
    }

    pub fn label(self) -> &'static str {
        self.style().label()
    }
}

/// Where a push without a destination goes: a serialisable mirror of
/// [`canopy_core::PushDefault`]. Variants are only ever added, so saved settings keep loading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PushMode {
    #[default]
    Simple,
    Upstream,
    Current,
}

impl PushMode {
    pub const ALL: [Self; 3] = [Self::Simple, Self::Upstream, Self::Current];

    pub fn push_default(self) -> PushDefault {
        match self {
            Self::Simple => PushDefault::Simple,
            Self::Upstream => PushDefault::Upstream,
            Self::Current => PushDefault::Current,
        }
    }

    pub fn label(self) -> &'static str {
        self.push_default().label()
    }
}

/// What a commit removes from its message: a serialisable mirror of [`canopy_core::Cleanup`].
/// Variants are only ever added, so saved settings keep loading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CleanupMode {
    #[default]
    Whitespace,
    Strip,
    Verbatim,
}

impl CleanupMode {
    pub const ALL: [Self; 3] = [Self::Whitespace, Self::Strip, Self::Verbatim];

    pub fn cleanup(self) -> Cleanup {
        match self {
            Self::Whitespace => Cleanup::Whitespace,
            Self::Strip => Cleanup::Strip,
            Self::Verbatim => Cleanup::Verbatim,
        }
    }

    pub fn label(self) -> &'static str {
        self.cleanup().label()
    }
}

/// The first branch of a repository Canopy creates when the setting is left empty.
pub const INITIAL_BRANCH: &str = "main";

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

/// The longest name a repository's tab can be given, in characters.
pub const MAX_TAB_NAME: usize = 60;

/// One repository's own choices. Each is `None` while the repository follows the default in
/// [`Settings`] (its "Default for repositories" value).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RepoSettings {
    #[serde(deserialize_with = "known_option")]
    pub pull_mode: Option<PullMode>,
    pub pull_follow_config: Option<bool>,
    pub prune_on_fetch: Option<bool>,
    /// Minutes between background fetches; `Some(0)` turns them off for this repository.
    pub auto_fetch_minutes: Option<u32>,
    pub signoff: Option<bool>,
    /// Protocol of remote URLs built from `owner/repo` (Add remote, Publish).
    #[serde(deserialize_with = "known_option")]
    pub protocol: Option<CloneProtocol>,
    /// The name its tab shows instead of the folder's.
    pub display_name: Option<String>,
    /// The icon its tab shows: a key of `tab::TAB_ICONS`.
    pub icon: Option<String>,
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
    #[serde(deserialize_with = "known")]
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

/// A choice this build may no longer offer: a saved theme or option that has since been removed
/// reads as `None` instead of failing the whole file. The name is read in full before it is
/// refused, so the rest of the file goes on decoding.
struct Known<T>(Option<T>);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Known<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self(T::deserialize(d).ok()))
    }
}

/// A choice, or its default when the saved one no longer exists.
fn known<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Known::deserialize(d)?.0.unwrap_or_default())
}

/// An optional choice, or none when the saved one no longer exists.
fn known_option<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Known<T>>::deserialize(d)?.and_then(|k| k.0))
}

/// Which notifications show as cards in the bottom-right corner. Every notification still goes
/// to the notification center, whatever these say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationPrefs {
    /// Cards at all; off, the types below show none either.
    pub cards: bool,
    pub errors: bool,
    pub attention: bool,
    pub updates: bool,
    pub finished: bool,
    pub confirmations: bool,
}

impl Default for NotificationPrefs {
    /// Only what needs the user: failures, questions waiting on them and new builds.
    fn default() -> Self {
        Self {
            cards: true,
            errors: true,
            attention: true,
            updates: true,
            finished: false,
            confirmations: false,
        }
    }
}

impl NotificationPrefs {
    /// The switch for notifications of `topic`.
    pub fn topic_mut(&mut self, topic: Topic) -> &mut bool {
        match topic {
            Topic::Error => &mut self.errors,
            Topic::Attention => &mut self.attention,
            Topic::Update => &mut self.updates,
            Topic::Finished => &mut self.finished,
            Topic::Confirmation => &mut self.confirmations,
        }
    }

    /// Whether a notification of `topic` shows as a card.
    pub fn shows(mut self, topic: Topic) -> bool {
        self.cards && *self.topic_mut(topic)
    }
}

/// User preferences and session state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    // Appearance.
    #[serde(deserialize_with = "known")]
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
    #[serde(deserialize_with = "known")]
    pub style: Style,
    /// Blurred translucent floating layers.
    pub translucent: bool,
    /// Which notifications show as cards in the bottom-right corner.
    pub notifications: NotificationPrefs,
    /// Which tooltips show, and after how long a rest.
    #[serde(deserialize_with = "known")]
    pub tooltips: TooltipLevel,
    #[serde(deserialize_with = "known")]
    pub tooltip_delay: TooltipDelay,
    /// The chosen theme at high contrast: stronger text, lines and focus, outlines for shadows.
    pub high_contrast: bool,
    /// The custom theme's colours (Settings, Appearance, with the Custom theme chosen).
    pub custom_theme: CustomTheme,
    /// Row heights, radii, toolbar layout and type size.
    #[serde(deserialize_with = "known")]
    pub density: Density,
    /// The history graph's commit nodes.
    #[serde(deserialize_with = "known")]
    pub graph_node: NodeShape,
    /// The history graph's lines between lanes.
    #[serde(deserialize_with = "known")]
    pub graph_curve: Curve,
    /// Match the system, always reduce, or always animate.
    #[serde(deserialize_with = "known")]
    pub motion: MotionPref,
    /// Pre-0.3 switch, read once by `migrate` and never written again.
    #[serde(skip_serializing)]
    pub reduce_motion: Option<bool>,

    // Git.
    /// Follow my git config: where the user's git configuration sets one of the git choices
    /// below (the pull strategy among them), it decides; each choice here applies where it sets
    /// none. On by default.
    pub follow_git_config: bool,
    /// Canopy's pull strategy; while `follow_git_config` is on, only where the git configuration
    /// sets none.
    #[serde(deserialize_with = "known")]
    pub pull_mode: PullMode,
    /// `--prune` on fetch: drop remote-tracking branches deleted on the remote.
    pub prune_on_fetch: bool,
    /// Fetch in the background every N minutes (0 = off).
    pub auto_fetch_minutes: u32,
    /// `git commit --signoff`.
    pub signoff: bool,
    /// Pull and rebase over uncommitted changes, stashing them around it (`--autostash`).
    pub autostash: bool,
    /// A push also sends the annotated tags on the commits it pushes (`--follow-tags`).
    pub follow_tags: bool,
    /// How conflicts are written into files (`merge.conflictStyle`).
    #[serde(deserialize_with = "known")]
    pub conflict_style: ConflictMode,
    /// Where a push without a destination goes (`push.default`).
    #[serde(deserialize_with = "known")]
    pub push_default: PushMode,
    /// What a commit removes from its message (`commit.cleanup`).
    #[serde(deserialize_with = "known")]
    pub cleanup: CleanupMode,
    /// A pruning fetch also deletes local tags the remote does not have (`--prune-tags`).
    pub prune_tags: bool,
    /// The first branch of a repository Canopy creates (`git init --initial-branch`).
    pub initial_branch: String,
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
    #[serde(deserialize_with = "known")]
    pub clone_protocol: CloneProtocol,
    /// Self-hosted GitLab hosts, comma-separated.
    pub gitlab_hosts: String,
    /// Terminal command for "Open terminal here", run in the repository folder (`%d` stands for
    /// it). Empty uses the system's terminal (`system_apps`).
    pub terminal: String,
    /// Editor command for "Open in editor"; `%f` stands for the file, which is added as the last
    /// argument otherwise. Empty uses the system's editor.
    pub editor: String,
    /// File manager command for showing a folder; `%d` stands for the folder, which is added as
    /// the last argument otherwise. Empty uses the system's file manager.
    pub file_manager: String,
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
    #[serde(deserialize_with = "known")]
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
            notifications: NotificationPrefs::default(),
            tooltips: TooltipLevel::Helpful,
            tooltip_delay: TooltipDelay::Normal,
            high_contrast: false,
            custom_theme: CustomTheme::default(),
            density: Density::Comfortable,
            graph_node: NodeShape::Dot,
            graph_curve: Curve::Smooth,
            motion: MotionPref::System,
            reduce_motion: None,
            follow_git_config: true,
            pull_mode: PullMode::FastForwardOnly,
            prune_on_fetch: true,
            auto_fetch_minutes: 0,
            signoff: false,
            autostash: true,
            follow_tags: true,
            conflict_style: ConflictMode::Zdiff3,
            push_default: PushMode::Simple,
            cleanup: CleanupMode::Whitespace,
            prune_tags: false,
            initial_branch: INITIAL_BRANCH.to_owned(),
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
            file_manager: String::new(),
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
        // Earlier builds saved a build skipped from the status bar as "build 412"; Skip this
        // version stores the number.
        if let Some(v) = self.skipped_version.as_mut() {
            let lower = v.trim().to_ascii_lowercase();
            if let Some(number) = lower
                .strip_prefix("build")
                .map(str::trim)
                .filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            {
                number.clone_into(v);
            }
        }
        self.zoom = self.zoom.clamp(0.8, 1.6);
        self.commit_page = self.commit_page.clamp(200, 50_000);
        self.diff.context = self.diff.context.min(25);
        let mut seen = Vec::new();
        self.trusted_folders
            .retain(|f| f.is_absolute() && crate::trust::add(&mut seen, f));
        for repo in self.repos.values_mut() {
            repo.display_name = repo
                .display_name
                .take()
                .map(|n| n.trim().chars().take(MAX_TAB_NAME).collect::<String>())
                .filter(|n| !n.is_empty());
            repo.icon = repo
                .icon
                .take()
                .filter(|k| crate::tab::TAB_ICONS.iter().any(|(key, _, _)| key == k));
        }
        // Only real choices are kept; a repository following every default needs no entry.
        self.repos
            .retain(|path, repo| path.is_absolute() && !repo.is_default());
    }

    /// The repository's own choices, or the defaults when it has none.
    pub fn repo(&self, path: &Path) -> RepoSettings {
        self.repos.get(path).cloned().unwrap_or_default()
    }

    /// One of the repository's own choices, without copying the rest.
    fn own<T>(&self, path: &Path, pick: impl FnOnce(&RepoSettings) -> Option<T>) -> Option<T> {
        self.repos.get(path).and_then(pick)
    }

    /// The repository's own choices, to change them.
    pub fn repo_mut(&mut self, path: &Path) -> &mut RepoSettings {
        self.repos.entry(path.to_path_buf()).or_default()
    }

    /// How a pull integrates in the repository at `path`.
    pub fn pull_mode_for(&self, path: &Path) -> PullMode {
        self.own(path, |r| r.pull_mode).unwrap_or(self.pull_mode)
    }

    /// The default for repositories: [`Settings::pull_mode`] and whether the git configuration's
    /// own strategy wins.
    pub fn pull_default(&self) -> PullChoice {
        PullChoice {
            mode: self.pull_mode,
            follow_config: self.follow_git_config,
        }
    }

    /// How the repository at `path` pulls: its own choices, else the default for repositories.
    pub fn pull_for(&self, path: &Path) -> PullChoice {
        PullChoice {
            mode: self.pull_mode_for(path),
            follow_config: self
                .own(path, |r| r.pull_follow_config)
                .unwrap_or(self.follow_git_config),
        }
    }

    /// How Canopy has git fetch, pull, push, commit, rebase and write conflicts.
    pub fn git_options(&self) -> GitOptions {
        GitOptions {
            follow_config: self.follow_git_config,
            autostash: self.autostash,
            follow_tags: self.follow_tags,
            conflict_style: self.conflict_style.style(),
            push_default: self.push_default.push_default(),
            cleanup: self.cleanup.cleanup(),
            prune_tags: self.prune_tags,
        }
    }

    /// The first branch of a repository Canopy creates: the default when none is set or the saved
    /// one is a name git refuses (from an older Canopy or an imported file).
    pub fn initial_branch(&self) -> &str {
        Some(self.initial_branch.trim())
            .filter(|b| crate::views::dialogs::valid_ref_name(b))
            .unwrap_or(INITIAL_BRANCH)
    }

    /// Whether fetching in the repository at `path` prunes.
    pub fn prune_for(&self, path: &Path) -> bool {
        self.own(path, |r| r.prune_on_fetch)
            .unwrap_or(self.prune_on_fetch)
    }

    /// Minutes between background fetches of the repository at `path` (0 = off).
    pub fn auto_fetch_for(&self, path: &Path) -> u32 {
        self.own(path, |r| r.auto_fetch_minutes)
            .unwrap_or(self.auto_fetch_minutes)
    }

    /// Whether commits in the repository at `path` get a Signed-off-by trailer.
    pub fn signoff_for(&self, path: &Path) -> bool {
        self.own(path, |r| r.signoff).unwrap_or(self.signoff)
    }

    /// The protocol of URLs built from `owner/repo` for the repository at `path`.
    pub fn protocol_for(&self, path: &Path) -> CloneProtocol {
        self.own(path, |r| r.protocol)
            .unwrap_or(self.clone_protocol)
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
    use eframe::Storage as _;

    use super::*;

    /// A saved or imported first branch that git would refuse never reaches `git init`: the
    /// default takes its place.
    #[test]
    fn an_invalid_first_branch_falls_back_to_the_default() {
        for bad in ["bad name~", "-x", "a..b", "@{-1}", "x.lock"] {
            let s = Settings {
                initial_branch: bad.to_owned(),
                ..Settings::default()
            };
            assert_eq!(s.initial_branch(), INITIAL_BRANCH, "{bad}");
        }
        let s = Settings {
            initial_branch: " trunk ".to_owned(),
            ..Settings::default()
        };
        assert_eq!(s.initial_branch(), "trunk");
    }

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

    /// Settings as eframe keeps them: RON under [`KEY`].
    #[derive(Default)]
    struct Stored(HashMap<String, String>);

    impl eframe::Storage for Stored {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }
        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }
        fn flush(&mut self) {}
    }

    #[test]
    fn choices_that_no_longer_exist_fall_back_and_keep_the_rest() {
        // Removed themes and options are not migrated: each reads as its default, a setting the
        // build no longer has is dropped, and every other setting in the file survives.
        let repo = std::env::temp_dir().join("canopy-settings-test");
        let mut s = Settings {
            zoom: 1.2,
            style: Style::Deeper,
            accepted_terms: "2026-09-01".to_owned(),
            ..Settings::default()
        };
        s.repos.insert(
            repo.clone(),
            RepoSettings {
                pull_mode: Some(PullMode::Rebase),
                signoff: Some(true),
                ..RepoSettings::default()
            },
        );
        let mut stored = Stored::default();
        eframe::set_value(&mut stored, KEY, &s);
        let saved = stored.get_string(KEY).expect("saved");
        let edited = saved
            .replacen("Midnight", "Aurora", 1)
            .replacen("Comfortable", "Spacious", 1)
            .replacen("Rebase", "Octopus", 1)
            .replacen('(', "(retired_switch: true, ", 1);
        assert_ne!(saved, edited);
        stored.set_string(KEY, edited);
        let back: Settings = eframe::get_value(&stored, KEY).expect("the rest still loads");
        assert_eq!(back.theme, ThemeId::Midnight);
        assert_eq!(back.density, Density::Comfortable);
        assert_eq!(back.style, Style::Deeper);
        assert!((back.zoom - 1.2).abs() < f32::EPSILON);
        assert_eq!(back.accepted_terms, "2026-09-01");
        let r = back.repos.get(&repo).expect("the repository's choices");
        assert_eq!((r.pull_mode, r.signoff), (None, Some(true)));
        // The next save writes only what this build knows.
        eframe::set_value(&mut stored, KEY, &back);
        let clean = stored.get_string(KEY).expect("saved");
        assert!(!clean.contains("Aurora") && !clean.contains("retired_switch"));

        // A settings file being imported reads the same way.
        let imported: Settings =
            serde_json::from_str(r#"{"theme": "Aurora", "zoom": 1.2}"#).expect("parse");
        assert_eq!(imported.theme, ThemeId::Midnight);
        assert!((imported.zoom - 1.2).abs() < f32::EPSILON);
    }

    #[test]
    fn each_choice_falls_back_to_the_default_settings_use() {
        let d = Settings::default();
        assert_eq!(d.theme, ThemeId::default());
        assert_eq!(d.style, Style::default());
        assert_eq!(d.density, Density::default());
        assert_eq!(d.graph_node, NodeShape::default());
        assert_eq!(d.graph_curve, Curve::default());
        assert_eq!(d.motion, MotionPref::default());
        assert_eq!(d.pull_mode, PullMode::default());
        assert_eq!(d.clone_protocol, CloneProtocol::default());
        assert_eq!(d.log_level, crate::logging::LogLevel::default());
        assert_eq!(d.diff.whitespace, WhitespaceMode::default());
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

    /// secfix2 F6: a build skipped from the status bar by an earlier build was saved as
    /// "build 412"; it reads as the build number Skip this version stores now.
    #[test]
    fn an_old_skipped_build_reads_as_its_number() {
        for (saved, migrated) in [("build 412", "412"), ("412", "412"), ("Build  7", "7")] {
            let mut s = Settings {
                skipped_version: Some(saved.to_owned()),
                ..Settings::default()
            };
            s.migrate();
            assert_eq!(s.skipped_version.as_deref(), Some(migrated), "{saved}");
        }
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
            ..RepoSettings::default()
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

    /// Canopy's git choices start from today's recommended practice, not git's own defaults, and
    /// the user's git configuration decides wherever it sets one of them.
    #[test]
    fn git_choices_default_to_recommended_practice() {
        let s = Settings::default();
        assert_eq!(s.pull_mode, PullMode::FastForwardOnly);
        assert_eq!(
            s.git_options(),
            GitOptions {
                follow_config: true,
                autostash: true,
                follow_tags: true,
                conflict_style: ConflictStyle::Zdiff3,
                push_default: PushDefault::Simple,
                cleanup: Cleanup::Whitespace,
                prune_tags: false,
            }
        );
        assert_eq!(s.git_options(), GitOptions::default());
        assert_eq!(s.initial_branch(), "main");
    }

    #[test]
    fn the_git_options_follow_the_settings() {
        let s = Settings {
            follow_git_config: false,
            autostash: false,
            follow_tags: false,
            conflict_style: ConflictMode::Merge,
            push_default: PushMode::Current,
            cleanup: CleanupMode::Strip,
            prune_tags: true,
            initial_branch: "  trunk ".into(),
            ..Settings::default()
        };
        assert_eq!(
            s.git_options(),
            GitOptions {
                follow_config: false,
                autostash: false,
                follow_tags: false,
                conflict_style: ConflictStyle::Merge,
                push_default: PushDefault::Current,
                cleanup: Cleanup::Strip,
                prune_tags: true,
            }
        );
        assert_eq!(s.initial_branch(), "trunk");
        let blank = Settings {
            initial_branch: " ".into(),
            ..Settings::default()
        };
        assert_eq!(blank.initial_branch(), INITIAL_BRANCH, "empty falls back");
        assert_eq!(
            ConflictMode::ALL.map(ConflictMode::style),
            ConflictStyle::ALL,
            "one ConflictMode per ConflictStyle"
        );
        assert_eq!(
            PushMode::ALL.map(PushMode::push_default),
            PushDefault::ALL,
            "one PushMode per PushDefault"
        );
        assert_eq!(
            CleanupMode::ALL.map(CleanupMode::cleanup),
            Cleanup::ALL,
            "one CleanupMode per Cleanup"
        );
    }

    /// Override O3: following the git configuration is the default, for new and old settings
    /// files; a repository can follow it or not on its own, and Canopy's strategy applies only
    /// where the configuration sets none or is not followed.
    #[test]
    fn pulls_follow_the_git_config_by_default_and_per_repository() {
        let repo = PathBuf::from("/home/ada/src/canopy");
        let other = PathBuf::from("/home/ada/src/other");
        let mut s = Settings::default();
        assert_eq!(s.pull_for(&repo), PullChoice::default());
        assert!(s.pull_for(&repo).follow_config);
        let old: Settings =
            serde_json::from_str(r#"{"pull_mode": "Rebase"}"#).expect("an old settings file");
        assert_eq!(
            old.pull_for(&repo),
            PullChoice {
                mode: PullMode::Rebase,
                follow_config: true
            },
            "pulls kept following pull.rebase before the switch existed"
        );

        s.repo_mut(&repo).pull_follow_config = Some(false);
        s.repo_mut(&repo).pull_mode = Some(PullMode::RebaseMerges);
        let own = s.pull_for(&repo);
        assert!(!own.follow_config);
        assert_eq!(
            own.resolve(Some(PullStrategy::Merge)),
            PullStrategy::RebaseMerges,
            "an explicit choice beats the git config"
        );
        let followed = s.pull_for(&other);
        assert_eq!(
            followed.resolve(Some(PullStrategy::Rebase)),
            PullStrategy::Rebase
        );
        assert_eq!(
            followed.resolve(None),
            PullStrategy::FastForwardOnly,
            "the fallback"
        );
        // Every strategy git can be asked for is offered, in the core's order.
        assert_eq!(
            PullMode::ALL.map(PullMode::strategy),
            PullStrategy::ALL,
            "one PullMode per PullStrategy"
        );
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
    fn tab_names_and_icons_are_checked_when_loaded() {
        let mut s: Settings = serde_json::from_str(
            r#"{"repos": {
                "/srv/a": {"display_name": "  Website  ", "icon": "globe"},
                "/srv/b": {"display_name": "   ", "icon": "no-such-icon"}
            }}"#,
        )
        .expect("parse");
        s.repo_mut(Path::new("/srv/c")).display_name = Some("x".repeat(500));
        s.migrate();
        let a = s.repo(Path::new("/srv/a"));
        assert_eq!(a.display_name.as_deref(), Some("Website"));
        assert_eq!(a.icon.as_deref(), Some("globe"));
        assert!(
            !s.repos.contains_key(Path::new("/srv/b")),
            "a blank name and an unknown icon are no choice"
        );
        let long = s.repo(Path::new("/srv/c")).display_name.unwrap_or_default();
        assert_eq!(long.chars().count(), MAX_TAB_NAME);
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
