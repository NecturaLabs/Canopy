//! What the user asked for during a frame.

use std::path::PathBuf;

use canopy_core::Oid;
use canopy_hosting::RequestSummary;

use crate::hosting::HostJob;
use crate::keymap::Command;
use crate::tab::View;
use crate::ui::theme::ThemeId;
use crate::views::dialogs::Dialog;
use crate::worker::Op;

/// Something the user asked for, collected during a frame and applied after it.
#[derive(Debug, Clone)]
pub enum Action {
    Op(Op),
    Confirm {
        title: String,
        body: String,
        confirm: String,
        danger: bool,
        op: Op,
    },
    OpenDialog(Dialog),
    OpenRepo(PathBuf),
    Clone {
        url: String,
        dest: PathBuf,
    },
    Init(PathBuf),
    ForgetRecent(PathBuf),
    CloseTab(usize),
    SwitchTab(usize),
    SetView(View),
    Reveal(Oid),
    Refresh,
    LoadMore,
    Fetch,
    Pull,
    Push,
    Commit,
    PrefillAmend,
    SetTheme(ThemeId),
    NextTheme,
    Palette,
    Cheatsheet,
    Settings,
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
