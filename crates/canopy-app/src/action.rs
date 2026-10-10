//! What the user asked for during a frame.

use std::path::PathBuf;

use canopy_core::Oid;
use canopy_core::identity::Scope;
use canopy_hosting::RequestSummary;

use crate::hosting::HostJob;
use crate::keymap::Command;
use crate::settings::DiffPrefs;
use crate::tab::View;
use crate::ui::motion::MotionPref;
use crate::ui::theme::{Density, Style, ThemeId};
use crate::views::dialogs::Dialog;
use crate::views::settings_screen::Section;
use crate::worker::Op;

/// An Appearance option the command palette can set, as Settings → Appearance sets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppearanceChoice {
    Style(Style),
    Density(Density),
    Translucent(bool),
    HighContrast(bool),
    Motion(MotionPref),
}

/// Something the user asked for, collected during a frame and applied after it.
#[derive(Debug, Clone)]
pub enum Action {
    CleanStale,
    RememberSidebar {
        repo: PathBuf,
        closed: std::collections::BTreeSet<String>,
    },
    /// The request list's filters and sort for the repository identified like the sidebar's.
    RememberRequestView {
        repo: PathBuf,
        view: crate::request_view::RequestView,
    },
    RepositoryInsights(PathBuf),
    ReadIgnore {
        repo: PathBuf,
        query: crate::worker::IgnoreQuery,
    },
    Op(Op),
    /// `op` in the repository at `repo`, whichever tab is active (a confirmation answered after
    /// the user moved to another tab); nothing when that repository's tab is closed.
    OpIn {
        repo: PathBuf,
        op: Op,
    },
    Confirm {
        title: String,
        body: String,
        confirm: String,
        danger: bool,
        op: Op,
    },
    OpenDialog(Dialog),
    OpenRepo(PathBuf),
    /// Convert the old-style partial clone at the path, then open it.
    ConvertPartialClone {
        path: PathBuf,
    },
    /// Navigate to another linked worktree in the current tab rather than opening a second tab.
    SwitchWorktree(PathBuf),
    Clone {
        url: String,
        dest: PathBuf,
    },
    Init(PathBuf, canopy_core::IgnoreTemplate),
    ForgetRecent(PathBuf),
    CloseTab(usize),
    SwitchTab(usize),
    SetView(View),
    Reveal(Oid),
    Refresh,
    /// Ask before force pushing the current branch: the repository thread first works out where
    /// the push would go, so the question can say.
    ForcePush,
    LoadMore,
    Fetch,
    Pull,
    /// Pull the repository at `repo` with `mode`, chosen when a fast-forward was not possible;
    /// `remember` keeps it as the pull strategy of that repository or of every repository.
    PullAs {
        repo: PathBuf,
        mode: crate::settings::PullMode,
        remember: Remember,
        /// The upstream commit the question showed ([`canopy_core::PullPreview::tip`]).
        seen: Option<Oid>,
    },
    /// Pull the repository at `repo` with `strategy` although it will likely conflict.
    PullAnyway {
        repo: PathBuf,
        strategy: canopy_core::ops::PullStrategy,
        /// The upstream commit the question showed ([`canopy_core::PullPreview::tip`]).
        seen: Option<Oid>,
    },
    Push,
    Commit,
    PrefillAmend,
    SetTheme(ThemeId),
    NextTheme,
    /// One of Settings → Appearance's options, from the command palette.
    SetAppearance(AppearanceChoice),
    Palette,
    Cheatsheet,
    Settings,
    /// Settings, opened on one section.
    SettingsAt(Section),
    /// Settings, opened on one section scrolled to the group with this heading.
    SettingsGroup(Section, &'static str),
    /// Settings → This repository for the open repository at this path (made the active tab).
    RepositorySettings(PathBuf),
    /// A repository's own diff options (`None`: follow the defaults in Settings again).
    SetRepoDiff {
        path: PathBuf,
        prefs: Option<DiffPrefs>,
    },
    /// Every repository back to the defaults in Settings.
    ResetRepoSettings,
    /// Settings were replaced as a whole (imported, or reset): apply what is not read live.
    SettingsReplaced,
    IdentitySettings,
    SaveIdentity {
        repo: Option<PathBuf>,
        scope: Scope,
        name: String,
        email: String,
    },
    ClearIdentity {
        repo: Option<PathBuf>,
        scope: Scope,
    },
    /// Let repositories in this folder download missing objects in partial clones (Settings,
    /// `trusted_folders`).
    TrustFolder(PathBuf),
    Copy(String),
    /// Open a file, folder or web page with the system's default handler.
    /// A web page (http or https only: API data and the update feed reach this).
    OpenUrl(String),
    /// A folder in the file manager.
    OpenPath(PathBuf),
    /// A file in the editor from Settings, or the platform's text editor; never run.
    OpenInEditor(PathBuf),
    OpenTerminal(PathBuf),
    /// Run a keyboard command (from the palette).
    Command(Command),
    CheckUpdates,
    SkipVersion(String),
    /// Install the update the dialog shows (in place, or through the installer).
    InstallUpdate(Box<crate::updater::Available>),
    /// Close Canopy and start this program, the updated one.
    RestartInto(PathBuf),
    /// Show what's new in an available update.
    ShowUpdate,
    /// Show this version's release notes.
    WhatsNew,
    /// Add a remote to a repository without one, then push.
    Publish,
    AddRemoteDialog,
    /// Start a pull/merge request for `source` (dialog, or the browser without an API token).
    CreateRequest {
        source: String,
    },
    /// Run a pull/merge request API action.
    Host(HostJob),
    CheckoutRequest(RequestSummary),
    /// Repository diff options changed: re-render the open diff.
    DiffPrefsChanged,
    /// Make the active repository's diff context and whitespace every repository's default.
    DiffPrefsForEveryRepository,
    /// The user saw the first-run tips.
    DismissIntro,
    /// Open the folder with Canopy's logs and crash reports.
    OpenLogs,
    /// Versions, this run's log file and recent warnings, to the clipboard.
    CopyDiagnostics,
    /// Unified or side-by-side diffs.
    SetSplitDiff(bool),
    /// The user accepted the current Terms of Service and Privacy Policy.
    AcceptTerms,
    /// The user's answer about crash reports.
    SetCrashReports(bool),
    /// Close Canopy.
    Quit,
}

/// How far a choice made in a prompt reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Remember {
    /// Ask again next time.
    #[default]
    No,
    Repository,
    Everywhere,
}
