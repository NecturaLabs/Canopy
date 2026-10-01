//! The application: tabs, keyboard commands, dialogs, the command palette, toasts and the
//! update check.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, unbounded};
use egui::{RichText, Stroke};
use egui_phosphor::regular as icon;

use canopy_core::{Oid, Repo};
use canopy_hosting::{NewRequest, RequestSummary};

use crate::hosting::{HostJob, TokenStore};
use crate::keymap::{self, Command};
use crate::settings::{self, Settings};
use crate::tab::{RepoTab, View};
use crate::ui::cheatsheet::Cheatsheet;
use crate::ui::palette::{CommandPalette, Entry};
use crate::ui::theme::{self, ThemeId, palette};
use crate::ui::toast::{ToastKind, Toasts};
use crate::updater::{self, UpdateState, Updater};
use crate::views::dialogs::{self, Dialog, DialogStack, Outcome};
use crate::views::settings_screen::{self, SettingsScreen};
use crate::views::{changes, history, requests, sidebar, statusbar, toolbar, welcome};
use crate::worker::{Job, Op};

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

/// Every `--overlay` name [`CanopyApp::new`] opens for screenshots.
pub const OVERLAYS: &[&str] = &[
    "settings",
    "privacy",
    "cheatsheet",
    "palette",
    "consent",
    "crash-consent",
    "legal",
    "licences",
    "credential",
    "host-key",
    "merge",
];

/// Options from the command line.
#[derive(Debug, Default, Clone)]
pub struct Launch {
    pub paths: Vec<PathBuf>,
    pub theme: Option<ThemeId>,
    pub screenshot: Option<PathBuf>,
    pub view: Option<View>,
    /// Open this overlay for a screenshot: settings, privacy, cheatsheet, palette, consent,
    /// crash-consent, legal, licences, credential, host-key or merge (see [`OVERLAYS`]).
    pub overlay: Option<String>,
    /// Side-by-side diffs.
    pub split: bool,
}

#[derive(Debug)]
struct Screenshot {
    path: PathBuf,
    ready_frames: u32,
    requested: bool,
    started: f64,
}

#[derive(Debug)]
enum CloneMsg {
    Progress(String),
    Done(Result<PathBuf, String>),
}

/// Rows a Page Down / Ctrl+D moves in a list.
const PAGE_ROWS: isize = 12;

/// The Canopy application state.
pub struct CanopyApp {
    settings: Settings,
    tabs: Vec<RepoTab>,
    active: usize,
    /// The dialog on screen and those waiting behind it (start-up notices after the consent,
    /// dialogs that arrived while a credential prompt was open).
    dialogs: DialogStack,
    settings_screen: Option<SettingsScreen>,
    palette: CommandPalette<Action>,
    cheatsheet: Cheatsheet,
    toasts: Toasts,
    screenshot: Option<Screenshot>,
    initial_view: Option<View>,
    clone: Option<(String, Receiver<CloneMsg>, String)>,
    git_version: String,
    tokens: TokenStore,
    updater: Updater,
    gitlab_hosts: Vec<String>,
    /// The start-up update check waits until the user has accepted the terms.
    update_check_due: bool,
    /// Receives git's and ssh's credential prompts (`None` when it could not start).
    askpass: Option<crate::askpass::Server>,
}

impl std::fmt::Debug for CanopyApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CanopyApp")
            .field("tabs", &self.tabs.len())
            .finish_non_exhaustive()
    }
}

impl CanopyApp {
    /// Builds the app, restoring settings and the previous session.
    pub fn new(cc: &eframe::CreationContext<'_>, launch: Launch) -> Self {
        let mut settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, settings::KEY))
            .unwrap_or_default();
        settings.migrate();
        let shooting = launch.screenshot.is_some();
        if shooting {
            // Screenshots are deterministic: default settings, nothing remembered.
            settings = Settings {
                seen_intro: true,
                check_updates: false,
                hosting_api: std::env::var_os("CANOPY_HOSTING_API_BASE").is_some(),
                // Answered, so the mock API loads; the consent overlays open the dialog directly.
                accepted_terms: crate::ui::legal::TERMS_VERSION.to_owned(),
                accepted_privacy: crate::ui::legal::PRIVACY_VERSION.to_owned(),
                crash_reports: Some(false),
                crash_reports_version: crate::crash_reports::DATA_VERSION,
                ..Settings::default()
            };
        }
        if let Some(t) = launch.theme {
            settings.theme = t;
        }
        if launch.split {
            settings.split_diff = true;
        }
        crate::logging::set_level(settings.log_level);
        crate::ui::fonts::install(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        theme::apply(&cc.egui_ctx, settings.theme, settings.reduce_motion);
        cc.egui_ctx.set_zoom_factor(settings.zoom);

        let git_version = canopy_core::cli::version().unwrap_or_else(|_| "git not found".into());
        let gitlab_hosts = settings.gitlab_host_list();
        let mut app = Self {
            settings,
            tabs: Vec::new(),
            active: 0,
            dialogs: DialogStack::default(),
            settings_screen: None,
            palette: CommandPalette::default(),
            cheatsheet: Cheatsheet::default(),
            toasts: Toasts::default(),
            screenshot: launch.screenshot.map(|path| Screenshot {
                path,
                ready_frames: 0,
                requested: false,
                started: 0.0,
            }),
            initial_view: launch.view,
            clone: None,
            git_version,
            tokens: if shooting {
                TokenStore::default()
            } else {
                TokenStore::load()
            },
            updater: Updater::default(),
            gitlab_hosts,
            update_check_due: false,
            askpass: if shooting {
                None
            } else {
                start_askpass(&cc.egui_ctx)
            },
        };
        let restore = if launch.paths.is_empty() && !shooting {
            app.settings.open_tabs.clone()
        } else {
            launch.paths
        };
        for p in restore {
            app.open_repo(&cc.egui_ctx, p);
        }
        if !app.tabs.is_empty() {
            app.active = 0;
        }
        match launch.overlay.as_deref() {
            Some("settings") => app.settings_screen = Some(SettingsScreen::default()),
            Some("cheatsheet") => app.cheatsheet.open = true,
            Some("palette") => app.palette.open(),
            Some("privacy") => {
                app.settings_screen = Some(SettingsScreen::at(settings_screen::Section::Privacy))
            }
            Some("consent") => {
                app.dialogs.open(Dialog::Consent {
                    step: crate::ui::legal::Pending::Terms,
                    updated: false,
                    reading: None,
                })
            }
            Some("crash-consent") => {
                app.dialogs.open(Dialog::Consent {
                    step: crate::ui::legal::Pending::CrashReports,
                    updated: false,
                    reading: None,
                })
            }
            Some("legal") => app.dialogs.open(Dialog::Legal(crate::ui::legal::Document::Privacy)),
            Some("credential") => {
                app.dialogs.open(Dialog::credential(crate::askpass::Prompt::preview(
                    "Enter passphrase for key '/home/ada/.ssh/id_ed25519': ",
                )))
            }
            Some("host-key") => {
                app.dialogs.open(Dialog::credential(crate::askpass::Prompt::preview(
                    "The authenticity of host 'github.com (140.82.121.4)' can't be established.\nED25519 key fingerprint is SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU.\nThis key is not known by any other names.\nAre you sure you want to continue connecting (yes/no/[fingerprint])? ",
                )))
            }
            Some("licences") => {
                app.dialogs.open(Dialog::Legal(crate::ui::legal::Document::Notices))
            }
            Some("merge") => {
                app.dialogs.open(Dialog::Merge {
                    target: "feature/search".into(),
                    rebase: false,
                    mode: canopy_core::MergeMode::Squash,
                })
            }
            _ => {}
        }
        if git_version_missing(&app.git_version) {
            app.toasts.push(&cc.egui_ctx, ToastKind::Error, "git was not found", "Canopy runs the git executable for changes and network operations. Install git and make sure it is on PATH.");
        }
        let mut notices = Vec::new();
        // A crash report from last time, offered once.
        if !shooting
            && let Some(report) = crate::logging::unseen_crash(&app.settings.last_crash_seen)
        {
            app.settings.last_crash_seen = report
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            notices.push(Dialog::Crash(report));
        }
        // Release notes once after an upgrade (not on a fresh install).
        let version = env!("CARGO_PKG_VERSION");
        if !shooting && app.settings.last_seen_version != version {
            if !app.settings.last_seen_version.is_empty()
                && let Some(d) = whats_new()
            {
                notices.push(d);
            }
            app.settings.last_seen_version = version.to_owned();
        }
        let now = chrono::Utc::now().timestamp();
        app.update_check_due = app.settings.check_updates
            && now - app.settings.last_update_check >= updater::CHECK_INTERVAL_SECS;
        // Nothing leaves the computer until the user has accepted the terms and answered about
        // crash reports.
        match (shooting, crate::ui::legal::pending(&app.settings)) {
            (false, Some(step)) => {
                for n in notices {
                    app.dialogs.open(n);
                }
                app.dialogs.open(Dialog::Consent {
                    step,
                    updated: !app.settings.accepted_terms.is_empty(),
                    reading: None,
                });
            }
            (true, _) => {}
            (false, None) => {
                crate::crash_reports::set_enabled(app.settings.crash_reports == Some(true));
                for n in notices {
                    app.dialogs.open(n);
                }
                app.start_due_update_check(&cc.egui_ctx);
            }
        }
        app
    }

    /// Runs the start-up update check if one is due.
    fn start_due_update_check(&mut self, ctx: &egui::Context) {
        if std::mem::take(&mut self.update_check_due) {
            self.settings.last_update_check = chrono::Utc::now().timestamp();
            self.updater
                .start(ctx, self.settings.include_prereleases, false);
        }
    }

    fn tab(&mut self) -> Option<&mut RepoTab> {
        self.tabs.get_mut(self.active)
    }

    fn open_repo(&mut self, ctx: &egui::Context, path: PathBuf) {
        let path = match path.to_str() {
            Some(typed) if typed.trim_start().starts_with('~') => settings::expand_home(typed),
            _ => path,
        };
        match Repo::open(&path) {
            Ok(repo) => {
                let root = repo.workdir().to_path_buf();
                let root = root.canonicalize().unwrap_or(root);
                if let Some(i) = self.tabs.iter().position(|t| t.path == root) {
                    self.active = i;
                } else {
                    let prefs = self.settings.diff_for(&root);
                    let mut tab = RepoTab::new(root.clone(), ctx, self.settings.commit_page, prefs);
                    tab.split_diff = self.settings.split_diff;
                    tab.highlight_diff = self.settings.syntax_highlight;
                    if let Some(v) = self.initial_view {
                        tab.view = v;
                    }
                    self.tabs.push(tab);
                    self.active = self.tabs.len() - 1;
                }
                self.settings.touch_recent(root);
            }
            Err(e) => self.toasts.push(
                ctx,
                ToastKind::Error,
                "Could not open repository",
                e.to_string(),
            ),
        }
    }

    /// Runs the keyboard table for this frame.
    fn keyboard(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let modal =
            self.dialogs.current().is_some() || self.settings_screen.is_some() || self.palette.open;
        let overlay = modal || self.cheatsheet.open;
        let typing = ctx.egui_wants_keyboard_input();
        // A button or checkbox reached with Tab keeps Space, Enter and the arrows for itself.
        let widget_focused = !typing && ctx.memory(|m| m.focused().is_some());
        let fired = keymap::dispatch(ctx, |b, chord| {
            if b.command == Command::Dismiss {
                return overlay;
            }
            if modal {
                return false;
            }
            if b.command == Command::CloseTab {
                // Ctrl+W while typing a commit message must not throw the draft away.
                return !typing;
            }
            if !b.section.is_bare() {
                return true;
            }
            !typing && !(widget_focused && keymap::widget_key(chord.key))
        });
        for f in fired {
            self.command(ctx, f.command, f.chord, actions);
        }
    }

    /// Turns a command into actions. `chord` is the matched chord's index (for tab jumps).
    fn command(
        &mut self,
        ctx: &egui::Context,
        command: Command,
        chord: usize,
        actions: &mut Vec<Action>,
    ) {
        use Command as C;
        let has_tab = !self.tabs.is_empty();
        let view = self.tabs.get(self.active).map(|t| t.view);
        match command {
            C::Palette => actions.push(Action::Palette),
            C::Cheatsheet => self.cheatsheet.toggle(),
            C::Dismiss => {
                if self.cheatsheet.open {
                    self.cheatsheet.open = false;
                } else if self.palette.open {
                    self.palette.open = false;
                } else if self.dialogs.current().is_some() {
                    // The consent stays (it must be answered); a credential prompt is declined.
                    self.dialogs.dismiss();
                } else {
                    self.settings_screen = None;
                }
            }
            C::Open => actions.push(Action::OpenDialog(Dialog::open())),
            C::Clone => actions.push(Action::OpenDialog(Dialog::clone_default(&self.settings))),
            C::Settings => actions.push(Action::Settings),
            C::ZoomIn | C::ZoomOut | C::ZoomReset => {
                self.settings.zoom = match command {
                    C::ZoomIn => (self.settings.zoom + 0.1).min(1.6),
                    C::ZoomOut => (self.settings.zoom - 0.1).max(0.8),
                    _ => 1.0,
                };
                ctx.set_zoom_factor(self.settings.zoom);
            }
            C::Quit => actions.push(Action::Quit),
            _ if !has_tab => {}
            C::CloseTab => actions.push(Action::CloseTab(self.active)),
            C::NextTab => actions.push(Action::SwitchTab((self.active + 1) % self.tabs.len())),
            C::PrevTab => actions.push(Action::SwitchTab(
                (self.active + self.tabs.len() - 1) % self.tabs.len(),
            )),
            C::JumpTab => {
                if chord < self.tabs.len() {
                    actions.push(Action::SwitchTab(chord));
                }
            }
            C::Refresh => actions.push(Action::Refresh),
            C::Fetch => actions.push(Action::Fetch),
            C::Pull => actions.push(Action::Pull),
            C::Push => actions.push(Action::Push),
            C::NewBranch => actions.push(Action::OpenDialog(Dialog::new_branch(None))),
            C::Stash => actions.push(Action::OpenDialog(Dialog::Stash {
                message: String::new(),
                untracked: true,
            })),
            C::PopStash => actions.push(Action::Op(Op::StashApply {
                index: 0,
                pop: true,
            })),
            C::MergeDialog => actions.push(Action::OpenDialog(Dialog::Merge {
                target: String::new(),
                rebase: false,
                mode: canopy_core::MergeMode::Default,
            })),
            C::Commit => {
                if view == Some(View::Changes) {
                    actions.push(Action::Commit);
                }
            }
            C::Filter => {
                actions.push(Action::SetView(View::History));
                ctx.memory_mut(|m| m.request_focus(egui::Id::new("history-filter")));
            }
            C::History => actions.push(Action::SetView(View::History)),
            C::Changes => actions.push(Action::SetView(View::Changes)),
            C::Requests => actions.push(Action::SetView(View::Requests)),
            C::Down | C::Up | C::PageDown | C::PageUp | C::First | C::Last => {
                let delta = match command {
                    C::Down => 1,
                    C::Up => -1,
                    C::PageDown => PAGE_ROWS,
                    C::PageUp => -PAGE_ROWS,
                    C::First => isize::MIN,
                    _ => isize::MAX,
                };
                if let Some(t) = self.tab() {
                    match t.view {
                        View::History => t.move_commit(delta),
                        View::Changes => t.move_file(delta),
                        View::Requests => t.move_request(delta),
                    }
                }
            }
            C::Tag => {
                if let Some(oid) = self.tabs.get(self.active).and_then(|t| t.selected_commit) {
                    actions.push(Action::OpenDialog(Dialog::NewTag {
                        name: String::new(),
                        target: oid,
                        message: String::new(),
                    }));
                }
            }
            C::CopySha => {
                if let Some(oid) = self.tabs.get(self.active).and_then(|t| t.selected_commit) {
                    actions.push(Action::Copy(oid.to_string()));
                }
            }
            C::OpenOnForge => {
                if let Some(t) = self.tabs.get(self.active)
                    && let Some((_, forge)) = t.forge()
                {
                    let url = match (t.view, t.selected_commit, &t.requests.list) {
                        (View::Requests, _, Some(Ok(list))) => t
                            .requests
                            .selected
                            .and_then(|n| list.iter().find(|r| r.number == n))
                            .map(|r| r.web_url.clone())
                            .unwrap_or_else(|| forge.web_url()),
                        (View::History, Some(oid), _) => forge.commit_url(&oid.to_string()),
                        _ => forge.web_url(),
                    };
                    actions.push(Action::OpenUrl(url));
                }
            }
            C::FocusCommit => {
                if let Some(t) = self.tab() {
                    t.view = View::Changes;
                    t.focus_summary = true;
                }
            }
            C::ToggleStage | C::Discard => {
                let Some(t) = self.tabs.get(self.active) else {
                    return;
                };
                let Some(sel) = t.file.clone().filter(|_| t.view == View::Changes) else {
                    return;
                };
                let path = sel.entry.path.clone();
                if command == C::Discard {
                    if sel.source == canopy_core::DiffSource::Unstaged && !sel.conflicted {
                        actions.push(Action::Confirm {
                            title: "Discard changes".into(),
                            body: format!(
                                "Throw away the unstaged changes to {path}? This cannot be undone."
                            ),
                            confirm: "Discard".into(),
                            danger: true,
                            op: Op::Discard(vec![sel.entry]),
                        });
                    }
                } else if sel.source == canopy_core::DiffSource::Staged {
                    actions.push(Action::Op(Op::Unstage(vec![path])));
                } else {
                    actions.push(Action::Op(Op::Stage(vec![path])));
                }
            }
            C::NextConflict
            | C::PrevConflict
            | C::TakeOurs
            | C::TakeTheirs
            | C::TakeBoth
            | C::TakeBothTheirsFirst
            | C::TakeBase
            | C::EditConflict
            | C::ClearChoice => {
                if let Some(r) = self
                    .tabs
                    .get_mut(self.active)
                    .filter(|t| t.view == View::Changes)
                    .and_then(|t| t.resolver.as_mut())
                {
                    match command {
                        C::NextConflict => r.move_focus(1),
                        C::PrevConflict => r.move_focus(-1),
                        C::TakeOurs => r.choose(canopy_core::Choice::Ours),
                        C::TakeTheirs => r.choose(canopy_core::Choice::Theirs),
                        C::TakeBoth => r.choose(canopy_core::Choice::OursThenTheirs),
                        C::TakeBothTheirsFirst => r.choose(canopy_core::Choice::TheirsThenOurs),
                        C::TakeBase => r.choose(canopy_core::Choice::Base),
                        C::EditConflict => r.edit_focused(),
                        _ => r.clear_focused(),
                    }
                }
            }
            C::StageAll => {
                if view == Some(View::Changes) {
                    actions.push(Action::Op(Op::StageAll));
                }
            }
            C::UnstageAll => {
                if view == Some(View::Changes) {
                    actions.push(Action::Op(Op::UnstageAll));
                }
            }
        }
    }

    /// A digest of what [`Self::palette_entries`] depends on that can change while the palette
    /// is open (it is rebuilt on every opening anyway).
    fn palette_inputs(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.active, self.tabs.len()).hash(&mut h);
        for t in &self.tabs {
            (&t.name, t.generation).hash(&mut h);
        }
        self.settings.recent.hash(&mut h);
        h.finish()
    }

    fn palette_entries(&self) -> Vec<(Entry, Action)> {
        let e = |icon: &'static str, label: &str, detail: &str, cmd: Option<Command>| Entry {
            icon,
            label: label.to_owned(),
            detail: detail.to_owned(),
            shortcut: cmd.map(keymap::hint).unwrap_or_default(),
        };
        use Command as C;
        let mut v = vec![
            (
                e(
                    icon::FOLDER_OPEN,
                    "Open repository…",
                    "Repository",
                    Some(C::Open),
                ),
                Action::OpenDialog(Dialog::open()),
            ),
            (
                e(
                    icon::CLOUD_ARROW_DOWN,
                    "Clone repository…",
                    "Repository",
                    Some(C::Clone),
                ),
                Action::OpenDialog(Dialog::clone_default(&self.settings)),
            ),
            (
                e(icon::PLANT, "Init new repository…", "Repository", None),
                Action::OpenDialog(Dialog::Init {
                    path: String::new(),
                }),
            ),
            (
                e(icon::PALETTE, "Next theme", "Appearance", None),
                Action::NextTheme,
            ),
            (
                e(icon::GEAR_SIX, "Settings", "Canopy", Some(C::Settings)),
                Action::Settings,
            ),
            (
                e(
                    icon::KEYBOARD,
                    "Keyboard shortcuts",
                    "Help",
                    Some(C::Cheatsheet),
                ),
                Action::Cheatsheet,
            ),
            (
                e(icon::ARROW_CIRCLE_UP, "Check for updates", "Canopy", None),
                Action::CheckUpdates,
            ),
            (
                e(icon::SPARKLE, "What's new in Canopy", "Help", None),
                Action::WhatsNew,
            ),
            (
                e(icon::INFO, "About Canopy", "Help", None),
                Action::OpenDialog(Dialog::About),
            ),
            (
                e(icon::FILE_TEXT, "Open log folder", "Help", None),
                Action::OpenLogs,
            ),
            (
                e(icon::SIGN_OUT, "Quit Canopy", "Canopy", Some(C::Quit)),
                Action::Quit,
            ),
            (
                e(
                    icon::MAGNIFYING_GLASS_PLUS,
                    "Zoom in",
                    "Appearance",
                    Some(C::ZoomIn),
                ),
                Action::Command(C::ZoomIn),
            ),
            (
                e(
                    icon::MAGNIFYING_GLASS_MINUS,
                    "Zoom out",
                    "Appearance",
                    Some(C::ZoomOut),
                ),
                Action::Command(C::ZoomOut),
            ),
        ];
        for t in ThemeId::ALL {
            v.push((
                e(
                    icon::PALETTE,
                    &format!("Theme: {}", t.name()),
                    "Appearance",
                    None,
                ),
                Action::SetTheme(t),
            ));
        }
        if let Some(tab) = self.tabs.get(self.active) {
            let local_only = tab.is_local_only();
            v.extend([
                (
                    e(icon::GRAPH, "Show history", "View", Some(C::History)),
                    Action::SetView(View::History),
                ),
                (
                    e(
                        icon::PENCIL_SIMPLE_LINE,
                        "Show changes",
                        "View",
                        Some(C::Changes),
                    ),
                    Action::SetView(View::Changes),
                ),
                (
                    e(
                        icon::ARROWS_CLOCKWISE,
                        "Refresh",
                        "Repository",
                        Some(C::Refresh),
                    ),
                    Action::Refresh,
                ),
                (
                    e(
                        icon::GIT_BRANCH,
                        "New branch…",
                        "Branch",
                        Some(C::NewBranch),
                    ),
                    Action::OpenDialog(Dialog::new_branch(None)),
                ),
                (
                    e(icon::GIT_MERGE, "Merge…", "Branch", Some(C::MergeDialog)),
                    Action::OpenDialog(Dialog::Merge {
                        target: String::new(),
                        rebase: false,
                        mode: canopy_core::MergeMode::Default,
                    }),
                ),
                (
                    e(icon::GIT_PULL_REQUEST, "Rebase…", "Branch", None),
                    Action::OpenDialog(Dialog::Merge {
                        target: String::new(),
                        rebase: true,
                        mode: canopy_core::MergeMode::Default,
                    }),
                ),
                (
                    e(icon::ARCHIVE, "Stash changes…", "Stash", Some(C::Stash)),
                    Action::OpenDialog(Dialog::Stash {
                        message: String::new(),
                        untracked: true,
                    }),
                ),
                (
                    e(
                        icon::TRAY_ARROW_UP,
                        "Pop latest stash",
                        "Stash",
                        Some(C::PopStash),
                    ),
                    Action::Op(Op::StashApply {
                        index: 0,
                        pop: true,
                    }),
                ),
                (
                    e(
                        icon::PLUS,
                        "Stage all changes",
                        "Changes",
                        Some(C::StageAll),
                    ),
                    Action::Op(Op::StageAll),
                ),
                (
                    e(
                        icon::MINUS,
                        "Unstage all changes",
                        "Changes",
                        Some(C::UnstageAll),
                    ),
                    Action::Op(Op::UnstageAll),
                ),
                (
                    e(icon::CLOUD, "Add remote…", "Remote", None),
                    Action::OpenDialog(Dialog::add_remote(&self.settings)),
                ),
                (
                    e(
                        icon::TERMINAL_WINDOW,
                        "Open terminal here",
                        "Repository",
                        None,
                    ),
                    Action::OpenTerminal(tab.path.clone()),
                ),
                (
                    e(
                        icon::FOLDER_OPEN,
                        "Show in file manager",
                        "Repository",
                        None,
                    ),
                    Action::OpenPath(tab.path.clone()),
                ),
                (
                    e(icon::X, "Close repository", "Repository", Some(C::CloseTab)),
                    Action::CloseTab(self.active),
                ),
            ]);
            if local_only {
                v.push((
                    e(icon::CLOUD_ARROW_UP, "Publish repository…", "Remote", None),
                    Action::Publish,
                ));
            } else {
                v.extend([
                    (e(icon::ARROWS_CLOCKWISE, "Fetch all remotes", "Remote", Some(C::Fetch)), Action::Fetch),
                    (e(icon::ARROW_FAT_LINE_DOWN, "Pull", "Remote", Some(C::Pull)), Action::Pull),
                    (e(icon::ARROW_FAT_LINE_UP, "Push", "Remote", Some(C::Push)), Action::Push),
                    (e(icon::WARNING, "Force push (with lease)…", "Remote", None), Action::Confirm { title: "Force push".into(), body: "Overwrite the remote branch with your local one? --force-with-lease refuses if someone else pushed in the meantime.".into(), confirm: "Force push".into(), danger: true, op: Op::Push { force: true } }),
                ]);
            }
            if let Some((_, forge)) = tab.forge() {
                let kind = forge.kind;
                v.push((
                    e(
                        forge_icon(kind),
                        &format!("Open repository on {}", kind.name()),
                        "Remote",
                        None,
                    ),
                    Action::OpenUrl(forge.web_url()),
                ));
                v.push((
                    e(
                        icon::GIT_PULL_REQUEST,
                        &format!("Show {}s", kind.request_noun()),
                        "View",
                        Some(C::Requests),
                    ),
                    Action::SetView(View::Requests),
                ));
                if let Some(b) = tab.snap.as_ref().and_then(|s| s.current_branch())
                    && b.upstream.is_some()
                {
                    v.push((
                        e(
                            icon::GIT_PULL_REQUEST,
                            &format!("Create {} for {}…", kind.request_noun(), b.name),
                            "Remote",
                            None,
                        ),
                        Action::CreateRequest {
                            source: b.name.clone(),
                        },
                    ));
                }
            }
            if let Some(s) = &tab.snap {
                if let Some(op) = s.state.command().map(|_| s.state) {
                    v.push((
                        e(
                            icon::PLAY,
                            &format!("Continue {}", op.label().to_lowercase()),
                            "Repository",
                            None,
                        ),
                        Action::Op(Op::ContinueOrAbort {
                            state: op,
                            abort: false,
                        }),
                    ));
                    v.push((
                        e(
                            icon::X_CIRCLE,
                            &format!("Abort {}", op.label().to_lowercase()),
                            "Repository",
                            None,
                        ),
                        Action::Op(Op::ContinueOrAbort {
                            state: op,
                            abort: true,
                        }),
                    ));
                }
                for b in s.branches.iter().filter(|b| !b.is_head) {
                    v.push((
                        e(
                            icon::GIT_BRANCH,
                            &format!("Check out {}", b.name),
                            "Branch",
                            None,
                        ),
                        Action::Op(Op::Checkout(b.name.clone())),
                    ));
                }
                for b in &s.remote_branches {
                    v.push((
                        e(
                            icon::CLOUD,
                            &format!("Check out {}", b.full_name()),
                            "Remote branch",
                            None,
                        ),
                        Action::Op(Op::CheckoutRemote {
                            remote: b.remote.clone(),
                            branch: b.branch.clone(),
                        }),
                    ));
                }
                for b in s.branches.iter().filter(|b| !b.is_head) {
                    v.push((
                        e(
                            icon::GIT_MERGE,
                            &format!("Merge {} into current", b.name),
                            "Branch",
                            None,
                        ),
                        Action::Op(Op::Merge(b.name.clone(), canopy_core::MergeMode::Default)),
                    ));
                }
            }
            if let Some(Ok(list)) = &tab.requests.list
                && let Some((_, forge)) = tab.forge()
            {
                for r in list.iter() {
                    v.push((
                        e(
                            icon::GIT_PULL_REQUEST,
                            &format!("#{} {}", r.number, r.title),
                            forge.kind.request_noun(),
                            None,
                        ),
                        Action::Command(C::Requests),
                    ));
                }
            }
        }
        for (i, t) in self.tabs.iter().enumerate() {
            v.push((
                e(
                    icon::FOLDER_SIMPLE,
                    &format!("Switch to {}", t.name),
                    "Tab",
                    None,
                ),
                Action::SwitchTab(i),
            ));
        }
        for r in &self.settings.recent {
            let name = r
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            v.push((
                e(
                    icon::CLOCK_COUNTER_CLOCKWISE,
                    &format!("Open {name}"),
                    &r.display().to_string(),
                    None,
                ),
                Action::OpenRepo(r.clone()),
            ));
        }
        v
    }

    fn set_theme(&mut self, ctx: &egui::Context, id: ThemeId) {
        self.settings.theme = id;
        theme::apply(ctx, id, self.settings.reduce_motion);
    }

    fn apply(&mut self, ctx: &egui::Context, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::Op(op) => {
                    if let Some(t) = self.tab() {
                        t.op(op);
                    }
                }
                Action::Confirm {
                    title,
                    body,
                    confirm,
                    danger,
                    op,
                } => self.dialogs.open(Dialog::Confirm {
                    title,
                    body,
                    confirm,
                    danger,
                    op,
                }),
                Action::OpenDialog(d) => self.dialogs.open(d),
                Action::OpenRepo(p) => self.open_repo(ctx, p),
                Action::Clone { url, dest } => self.start_clone(ctx, url, dest),
                Action::Init(p) => match Repo::init(&p) {
                    Ok(_) => {
                        self.toasts.push(
                            ctx,
                            ToastKind::Success,
                            "Repository created",
                            p.display().to_string(),
                        );
                        self.open_repo(ctx, p);
                    }
                    Err(e) => self.toasts.push(
                        ctx,
                        ToastKind::Error,
                        "Could not create repository",
                        e.to_string(),
                    ),
                },
                Action::ForgetRecent(p) => self.settings.recent.retain(|r| r != &p),
                Action::CloseTab(i) => {
                    if i < self.tabs.len() {
                        self.tabs.remove(i);
                        if self.active >= self.tabs.len() {
                            self.active = self.tabs.len().saturating_sub(1);
                        }
                    }
                }
                Action::SwitchTab(i) => {
                    if i < self.tabs.len() {
                        self.active = i;
                        self.tabs[i].refresh();
                    }
                }
                Action::SetView(v) => {
                    if let Some(t) = self.tab()
                        && t.view != v
                    {
                        t.view = v;
                        t.file = None;
                        t.diff = None;
                        if v == View::History
                            && let Some(oid) = t.selected_commit.take()
                        {
                            t.select_commit(oid);
                            t.reveal_selected = true;
                        }
                        if v == View::Requests && t.requests.list.is_none() {
                            t.load_requests();
                        }
                    }
                }
                Action::Reveal(oid) => {
                    if let Some(t) = self.tab() {
                        t.view = View::History;
                        t.filter.clear();
                        t.select_commit(oid);
                        t.scroll_to_selected = true;
                    }
                }
                Action::Refresh => {
                    if let Some(t) = self.tab() {
                        t.refresh();
                        if t.view == View::Requests {
                            t.load_requests();
                        }
                    }
                }
                Action::LoadMore => {
                    let page = self.settings.commit_page;
                    if let Some(t) = self.tab() {
                        t.commit_limit += page;
                        t.refresh();
                    }
                }
                Action::Fetch => {
                    let prune = self.settings.prune_on_fetch;
                    if let Some(t) = self.tab() {
                        t.op(Op::Fetch {
                            prune,
                            quiet: false,
                        });
                    }
                }
                Action::Pull => {
                    let strategy = self.settings.pull_mode.strategy();
                    if let Some(t) = self.tab() {
                        t.op(Op::Pull { strategy });
                    }
                }
                Action::Push => {
                    if self
                        .tabs
                        .get(self.active)
                        .is_some_and(RepoTab::is_local_only)
                    {
                        self.dialogs
                            .open(Dialog::add_remote(&self.settings).publishing());
                    } else if let Some(t) = self.tab() {
                        t.op(Op::Push { force: false });
                    }
                }
                Action::WhatsNew => {
                    if let Some(d) = whats_new() {
                        self.dialogs.open(d);
                    }
                }
                Action::CopyDiagnostics => {
                    ctx.copy_text(self.diagnostics());
                    self.toasts.push(
                        ctx,
                        ToastKind::Info,
                        "Diagnostics copied",
                        "Check the text before you share it: it can name files and branches.",
                    );
                }
                Action::OpenLogs => match crate::logging::log_dir() {
                    Some(dir) => {
                        let _ = std::fs::create_dir_all(&dir);
                        if let Err(e) = crate::platform::open_folder(&dir) {
                            self.toasts.push(ctx, ToastKind::Error, "Could not open", e);
                        }
                    }
                    None => self.toasts.push(
                        ctx,
                        ToastKind::Error,
                        "No log folder",
                        "This platform has no data folder for Canopy.",
                    ),
                },
                Action::ShowUpdate => {
                    if let UpdateState::Available(a) = &self.updater.state {
                        self.dialogs.open(Dialog::Update(a.clone()));
                    }
                }
                Action::AddRemoteDialog => {
                    self.dialogs.open(Dialog::add_remote(&self.settings));
                }
                Action::Publish => {
                    self.dialogs
                        .open(Dialog::add_remote(&self.settings).publishing());
                }
                Action::Commit => {
                    let signoff = self.settings.signoff;
                    if let Some(t) = self.tab() {
                        let mut message = t.commit_summary.trim().to_owned();
                        if !t.commit_body.trim().is_empty() {
                            message.push_str("\n\n");
                            message.push_str(t.commit_body.trim());
                        }
                        if !message.is_empty() {
                            t.op(Op::Commit {
                                message,
                                amend: t.amend,
                                signoff,
                            });
                        }
                    }
                }
                Action::PrefillAmend => {
                    if let Some(t) = self.tab() {
                        t.worker.send(Job::HeadMessage);
                    }
                }
                Action::SetTheme(id) => self.set_theme(ctx, id),
                Action::NextTheme => {
                    let next = self.settings.theme.next();
                    self.set_theme(ctx, next);
                    self.toasts
                        .push(ctx, ToastKind::Info, format!("Theme: {}", next.name()), "");
                }
                Action::Palette => self.palette.open(),
                Action::Cheatsheet => self.cheatsheet.toggle(),
                Action::Settings => self.settings_screen = Some(SettingsScreen::default()),
                Action::Copy(text) => {
                    ctx.copy_text(text.clone());
                    self.toasts.push(ctx, ToastKind::Info, "Copied", text);
                }
                Action::OpenUrl(url) => {
                    if let Err(e) = crate::platform::open_url(&url) {
                        self.toasts
                            .push(ctx, ToastKind::Error, "Could not open the page", e);
                    }
                }
                Action::OpenPath(dir) => {
                    if let Err(e) = crate::platform::open_folder(&dir) {
                        self.toasts
                            .push(ctx, ToastKind::Error, "Could not open the folder", e);
                    }
                }
                Action::OpenInEditor(file) => {
                    if let Err(e) = crate::platform::open_in_editor(&file, &self.settings.editor) {
                        self.toasts
                            .push(ctx, ToastKind::Error, "Could not open the file", e);
                    }
                }
                Action::OpenTerminal(p) => {
                    if let Err(e) = crate::platform::open_terminal(&p, &self.settings.terminal) {
                        self.toasts
                            .push(ctx, ToastKind::Error, "Could not open a terminal", e);
                    }
                }
                Action::Command(c) => {
                    let mut more = Vec::new();
                    self.command(ctx, c, 0, &mut more);
                    self.apply(ctx, more);
                }
                Action::CheckUpdates => {
                    self.settings.last_update_check = chrono::Utc::now().timestamp();
                    self.updater
                        .start(ctx, self.settings.include_prereleases, true);
                }
                Action::SkipVersion(v) => {
                    self.settings.skipped_version = Some(v);
                    self.updater.state = UpdateState::UpToDate;
                }
                Action::CreateRequest { source } => self.create_request(ctx, source),
                Action::Host(job) => {
                    if let Some(t) = self.tab() {
                        let kind = t.host.as_ref().map(|h| h.forge.kind);
                        if let Some(kind) = kind {
                            t.requests.busy = Some(match &job {
                                HostJob::Create(_) => format!("Creating {}", kind.request_noun()),
                                _ => "Working…".into(),
                            });
                        }
                        t.host_job(job);
                    }
                }
                Action::CheckoutRequest(r) => self.checkout_request(r),
                Action::DiffPrefsChanged => {
                    if let Some(t) = self.tabs.get_mut(self.active) {
                        let prefs = t.diff_prefs;
                        let path = t.path.clone();
                        t.reload_diff();
                        if prefs == self.settings.diff {
                            self.settings.repo_diff.remove(&path);
                        } else {
                            self.settings.repo_diff.insert(path, prefs);
                        }
                    }
                }
                Action::DismissIntro => self.settings.seen_intro = true,
                Action::AcceptTerms => {
                    self.settings.accepted_terms = crate::ui::legal::TERMS_VERSION.to_owned();
                    self.settings.accepted_privacy = crate::ui::legal::PRIVACY_VERSION.to_owned();
                    // The consent dialog stays open until its actions are applied; this replaces
                    // it with the next question, or ends it.
                    self.dialogs.close_where(is_consent);
                    match crate::ui::legal::pending(&self.settings) {
                        Some(step) => self.dialogs.open(Dialog::Consent {
                            step,
                            updated: false,
                            reading: None,
                        }),
                        None => self.consent_done(ctx),
                    }
                }
                Action::SetCrashReports(on) => {
                    self.settings.crash_reports = Some(on);
                    self.settings.crash_reports_version = crate::crash_reports::DATA_VERSION;
                    crate::crash_reports::set_enabled(on);
                    if self.dialogs.any(is_consent) {
                        self.dialogs.close_where(is_consent);
                        self.consent_done(ctx);
                    }
                }
                Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Action::SetSplitDiff(split) => {
                    self.settings.split_diff = split;
                    for t in &mut self.tabs {
                        t.split_diff = split;
                    }
                }
            }
        }
    }

    /// Shows git's and ssh's credential questions, one at a time, above any other dialog.
    fn poll_prompts(&mut self, ctx: &egui::Context) {
        // A prompt git or ssh gave up on (killed, timed out) closes by itself.
        self.dialogs.drop_withdrawn();
        let Some(server) = &self.askpass else { return };
        while let Ok(prompt) = server.prompts.try_recv() {
            if prompt.kind == crate::askpass::PromptKind::Notice {
                // ssh shows it while it waits (e.g. for a security key touch); nothing to answer.
                self.toasts
                    .push(ctx, ToastKind::Info, "SSH", prompt.text.clone());
                prompt.answer(Some(String::new()));
                continue;
            }
            // In front of any other dialog; behind a prompt already open.
            self.dialogs.open(Dialog::credential(prompt));
        }
    }

    /// The text "Copy diagnostics" puts on the clipboard, with the home folder shown as `~`.
    fn diagnostics(&self) -> String {
        use std::fmt::Write as _;
        let mut t = String::new();
        let run = crate::logging::run();
        let _ = writeln!(t, "Canopy {}", env!("CARGO_PKG_VERSION"));
        let _ = writeln!(
            t,
            "Platform: {} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        );
        let _ = writeln!(t, "Git: {}", self.git_version);
        let _ = writeln!(t, "Run: {}", run.map_or("-", |r| r.id.as_str()));
        let _ = writeln!(
            t,
            "Log: {}",
            run.and_then(|r| r.file.as_ref())
                .map_or("(not written)".to_owned(), |f| f.display().to_string())
        );
        let _ = writeln!(t, "Log detail: {}", self.settings.log_level.label());
        let _ = writeln!(t, "Theme: {}", self.settings.theme.name());
        let _ = writeln!(t, "Open repositories: {}", self.tabs.len());
        let _ = writeln!(
            t,
            "Crash reports: {}",
            match self.settings.crash_reports {
                Some(true) => "on",
                Some(false) => "off",
                None => "not answered",
            }
        );
        let problems = crate::logging::recent_problems();
        let _ = writeln!(t, "\nRecent warnings and errors ({}):", problems.len());
        for line in problems {
            let _ = writeln!(t, "{line}");
        }
        let home = crate::settings::home_dir().display().to_string();
        crate::crash_reports::scrub_text(&t, &home)
    }

    /// The consent dialog is answered: start what waited for it (the start-up notices queued
    /// behind it now show).
    fn consent_done(&mut self, ctx: &egui::Context) {
        crate::crash_reports::set_enabled(self.settings.crash_reports == Some(true));
        self.start_due_update_check(ctx);
    }

    /// Opens the new pull/merge request dialog, or the provider's page when the API is off or
    /// has no token.
    fn create_request(&mut self, ctx: &egui::Context, source: String) {
        let Some(t) = self.tabs.get(self.active) else {
            return;
        };
        let Some((remote, forge)) = t.forge().cloned() else {
            return;
        };
        let target = t
            .snap
            .as_ref()
            .and_then(|s| s.remotes.iter().find(|r| r.name == remote))
            .and_then(|r| r.default_branch.clone());
        let local = t
            .snap
            .as_ref()
            .and_then(|s| s.branches.iter().find(|b| b.name == source));
        // The request's head is the branch on the remote: a local `topic` tracking
        // `origin/feature/topic` opens a request from `feature/topic`.
        let prefix = format!("{remote}/");
        let source = local
            .and_then(|b| b.upstream.as_deref())
            .and_then(|u| u.strip_prefix(&prefix))
            .map_or(source.clone(), str::to_owned);
        let head_commit = local.and_then(|b| b.target);
        let has_token = matches!(t.requests.token, Some(Some(_)));
        if !self.settings.hosting_api || !has_token {
            let url = forge.new_request_url(&source, target.as_deref());
            if let Err(e) = crate::platform::open_url(&url) {
                self.toasts
                    .push(ctx, ToastKind::Error, "Could not open the browser", e);
            }
            return;
        }
        // Prefill from the branch's newest commit: its summary and body.
        let (title, body) = head_commit
            .and_then(|oid| t.snap.as_ref()?.commits.iter().find(|c| c.oid == oid))
            .map(|c| (c.summary.clone(), String::new()))
            .unwrap_or_default();
        let branches: Vec<String> = t
            .snap
            .as_ref()
            .map(|s| {
                s.remote_branches
                    .iter()
                    .filter(|b| b.remote == remote)
                    .map(|b| b.branch.clone())
                    .collect()
            })
            .unwrap_or_default();
        self.dialogs.open(Dialog::NewRequest {
            kind: forge.kind,
            request: NewRequest {
                title,
                body,
                source,
                target: target.unwrap_or_else(|| "main".into()),
                draft: false,
            },
            branches,
        });
    }

    fn checkout_request(&mut self, r: RequestSummary) {
        let Some(t) = self.tabs.get(self.active) else {
            return;
        };
        let Some((remote, forge)) = t.forge().cloned() else {
            return;
        };
        let same_repo_branch = !r.from_fork
            && t.snap.as_ref().is_some_and(|s| {
                s.remote_branches
                    .iter()
                    .any(|b| b.remote == remote && b.branch == r.source_branch)
            });
        let local_exists = t
            .snap
            .as_ref()
            .is_some_and(|s| s.branches.iter().any(|b| b.name == r.source_branch));
        let op = if same_repo_branch && local_exists {
            Op::Checkout(r.source_branch.clone())
        } else if same_repo_branch {
            Op::CheckoutRemote {
                remote,
                branch: r.source_branch.clone(),
            }
        } else {
            let prefix = match forge.kind {
                canopy_core::forge::ForgeKind::GitHub => "pr",
                canopy_core::forge::ForgeKind::GitLab => "mr",
            };
            Op::CheckoutRequest {
                remote,
                remote_ref: canopy_hosting::checkout_ref(forge.kind, r.number),
                branch: format!("{prefix}/{}", r.number),
            }
        };
        if let Some(t) = self.tab() {
            t.op(op);
        }
    }

    fn start_clone(&mut self, ctx: &egui::Context, url: String, dest: PathBuf) {
        let (tx, rx) = unbounded();
        let thread_ctx = ctx.clone();
        let thread_url = url.clone();
        let spawned = std::thread::Builder::new()
            .name("clone".into())
            .spawn(move || {
                let send = |m: CloneMsg| {
                    let _ = tx.send(m);
                    thread_ctx.request_repaint();
                };
                let result = canopy_core::ops::clone(&thread_url, &dest, |l| {
                    send(CloneMsg::Progress(l.to_owned()))
                });
                send(CloneMsg::Done(
                    result.map(|()| dest).map_err(|e| e.to_string()),
                ));
            });
        match spawned {
            Ok(_) => self.clone = Some((url, rx, "Starting…".into())),
            // The status bar never shows a clone that is not running.
            Err(e) => {
                tracing::error!("could not start the clone thread: {e}");
                self.toasts.push(
                    ctx,
                    ToastKind::Error,
                    "Clone failed",
                    format!("Could not start the clone: {e}"),
                );
            }
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        for tab in &mut self.tabs {
            for n in tab.poll(&self.gitlab_hosts) {
                self.toasts.push(
                    ctx,
                    if n.ok {
                        ToastKind::Success
                    } else {
                        ToastKind::Error
                    },
                    n.title,
                    n.body,
                );
            }
        }
        // Start the pull/merge request thread for the active tab once its remotes are known, and
        // not before the start-up questions are answered: until then Canopy contacts no service
        // (the Privacy Policy promises this).
        let answered = crate::ui::legal::pending(&self.settings).is_none();
        if let Some(t) = self.tabs.get_mut(self.active).filter(|_| answered) {
            let saved = t
                .forge()
                .and_then(|(_, f)| self.tokens.get_for(f))
                .map(str::to_owned);
            t.ensure_host(ctx, saved, self.settings.hosting_api);
        }
        let mut finished = None;
        if let Some((_, rx, status)) = &mut self.clone {
            loop {
                match rx.try_recv() {
                    Ok(CloneMsg::Progress(l)) => *status = l,
                    Ok(CloneMsg::Done(r)) => finished = Some(r),
                    Err(crossbeam_channel::TryRecvError::Empty) => break,
                    // The thread ended without reporting (it panicked).
                    Err(crossbeam_channel::TryRecvError::Disconnected) => {
                        finished.get_or_insert(Err("The clone stopped unexpectedly.".into()));
                        break;
                    }
                }
            }
        }
        if let Some(r) = finished {
            self.clone = None;
            match r {
                Ok(p) => {
                    self.toasts.push(
                        ctx,
                        ToastKind::Success,
                        "Clone complete",
                        p.display().to_string(),
                    );
                    self.open_repo(ctx, p);
                }
                Err(e) => self.toasts.push(ctx, ToastKind::Error, "Clone failed", e),
            }
        }
        self.poll_prompts(ctx);
        if self.updater.poll() {
            let manual = self.updater.manual;
            match &self.updater.state {
                UpdateState::Available(a) => {
                    let v = &a.version;
                    let skipped = self.settings.skipped_version.as_deref() == Some(&v.to_string());
                    if manual {
                        self.dialogs.open(Dialog::Update(a.clone()));
                    } else if !skipped {
                        self.toasts.push(
                            ctx,
                            ToastKind::Info,
                            format!("Canopy {v} is available"),
                            "Click the update badge in the status bar for what's new.",
                        );
                    } else {
                        self.updater.state = UpdateState::UpToDate;
                    }
                }
                UpdateState::UpToDate if manual => self.toasts.push(
                    ctx,
                    ToastKind::Success,
                    "Canopy is up to date",
                    format!("Version {}", env!("CARGO_PKG_VERSION")),
                ),
                UpdateState::Failed(e) if manual => self.toasts.push(
                    ctx,
                    ToastKind::Error,
                    "Could not check for updates",
                    e.clone(),
                ),
                _ => {}
            }
        }
    }

    /// Queues background fetches for tabs whose auto-fetch interval has passed.
    fn auto_fetch(&mut self, ctx: &egui::Context) {
        let minutes = self.settings.auto_fetch_minutes;
        if minutes == 0 || self.screenshot.is_some() {
            return;
        }
        let every = Duration::from_secs(u64::from(minutes) * 60);
        let prune = self.settings.prune_on_fetch;
        for t in &mut self.tabs {
            let idle = t.running.is_none();
            let has_remote = t.snap.as_ref().is_some_and(|s| !s.remotes.is_empty());
            if idle && has_remote && t.last_fetch.elapsed() >= every {
                t.last_fetch = Instant::now();
                t.op(Op::Fetch { prune, quiet: true });
            }
        }
        ctx.request_repaint_after(Duration::from_secs(30));
    }

    /// Applies settings that changed in the settings screen.
    fn settings_changed(&mut self, ctx: &egui::Context, before: &Settings) {
        let s = &self.settings;
        if s.theme != before.theme || s.reduce_motion != before.reduce_motion {
            theme::apply(ctx, s.theme, s.reduce_motion);
        }
        if (ctx.zoom_factor() - s.zoom).abs() > f32::EPSILON {
            ctx.set_zoom_factor(s.zoom);
        }
        if s.gitlab_hosts != before.gitlab_hosts {
            self.gitlab_hosts = s.gitlab_host_list();
            for t in &mut self.tabs {
                t.update_forges(&self.gitlab_hosts);
            }
        }
        if s.diff != before.diff {
            for t in &mut self.tabs {
                if !self.settings.repo_diff.contains_key(&t.path) {
                    t.diff_prefs = self.settings.diff;
                    t.reload_diff();
                }
            }
        }
        if s.split_diff != before.split_diff || s.syntax_highlight != before.syntax_highlight {
            for t in &mut self.tabs {
                t.split_diff = s.split_diff;
                if t.highlight_diff != s.syntax_highlight {
                    t.highlight_diff = s.syntax_highlight;
                    t.reload_diff();
                }
            }
        }
        if s.hosting_api != before.hosting_api {
            let enabled = s.hosting_api;
            for t in &mut self.tabs {
                let saved = t
                    .forge()
                    .and_then(|(_, f)| self.tokens.get_for(f))
                    .map(str::to_owned);
                t.restart_host(ctx, saved, enabled);
            }
        }
    }

    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let Some(shot) = &mut self.screenshot else {
            return;
        };
        let now = ctx.input(|i| i.time);
        if shot.started == 0.0 {
            shot.started = now;
        }
        let ready = self.tabs.iter().all(|t| {
            let requests_ready = t.view != View::Requests
                || t.host.is_none()
                || (!t.requests.loading
                    && !t.requests.details_loading
                    && t.requests.list.is_some());
            t.snap.is_some()
                && t.running.is_none()
                && (t.diff.is_some() || t.file.is_none())
                && requests_ready
        });
        if ready {
            shot.ready_frames += 1;
        }
        if !shot.requested && (shot.ready_frames > 30 || now - shot.started > 12.0) {
            shot.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        ctx.request_repaint();
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            match save_png(&shot.path, &image) {
                Ok(()) => tracing::info!(path = %shot.path.display(), "screenshot saved"),
                Err(e) => tracing::error!("screenshot failed: {e}"),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

/// The "What's new" dialog for this version, from the bundled changelog (the Unreleased
/// section in development builds).
fn whats_new() -> Option<Dialog> {
    use crate::ui::notes::{CHANGELOG, changelog_section};
    let version = env!("CARGO_PKG_VERSION");
    changelog_section(CHANGELOG, version)
        .map(|notes| (version.to_owned(), notes))
        .or_else(|| changelog_section(CHANGELOG, "Unreleased").map(|n| (format!("{version}+"), n)))
        .map(|(version, notes)| Dialog::WhatsNew { version, notes })
}

fn is_consent(d: &Dialog) -> bool {
    matches!(d, Dialog::Consent { .. })
}

/// The Phosphor icon of a forge.
pub fn forge_icon(kind: canopy_core::forge::ForgeKind) -> &'static str {
    match kind {
        canopy_core::forge::ForgeKind::GitHub => icon::GITHUB_LOGO,
        canopy_core::forge::ForgeKind::GitLab => icon::GITLAB_LOGO_SIMPLE,
    }
}

fn git_version_missing(v: &str) -> bool {
    !v.starts_with("git version")
}

fn save_png(path: &std::path::Path, image: &egui::ColorImage) -> anyhow::Result<()> {
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(
        std::io::BufWriter::new(file),
        u32::try_from(image.size[0])?,
        u32::try_from(image.size[1])?,
    );
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header()?;
    let bytes: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(|c| c.to_srgba_unmultiplied())
        .collect();
    writer.write_image_data(&bytes)?;
    Ok(())
}

/// Starts the credential prompt listener and registers Canopy as git's and ssh's askpass program.
fn start_askpass(ctx: &egui::Context) -> Option<crate::askpass::Server> {
    let program = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("no credential prompts: cannot locate the Canopy executable: {e}");
            return None;
        }
    };
    let ctx = ctx.clone();
    match crate::askpass::Server::start(move || ctx.request_repaint()) {
        Ok(server) => {
            canopy_core::cli::set_askpass(Some(canopy_core::cli::Askpass {
                program,
                env: server.env(),
            }));
            Some(server)
        }
        Err(e) => {
            tracing::warn!("no credential prompts: {e}");
            None
        }
    }
}

impl eframe::App for CanopyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        self.poll(&ctx);
        self.auto_fetch(&ctx);
        let mut actions = Vec::new();
        self.keyboard(&ctx, &mut actions);

        // Refresh when the window regains focus: files may have changed in an editor.
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        let was = ctx.data_mut(|d| {
            std::mem::replace(
                d.get_temp_mut_or_insert_with(egui::Id::new("was-focused"), || true),
                focused,
            )
        });
        if focused
            && !was
            && let Some(t) = self.tabs.get(self.active)
        {
            t.refresh();
        }

        egui::Panel::top("tabs")
            .exact_size(40.0)
            .frame(egui::Frame::new().fill(p.bg))
            .show(ui, |ui| {
                toolbar::tabs(
                    ui,
                    &self.tabs,
                    self.active,
                    self.settings.theme,
                    &mut actions,
                )
            });
        if let Some(tab) = self.tabs.get(self.active) {
            egui::Panel::top("actions")
                .exact_size(56.0)
                .frame(
                    egui::Frame::new()
                        .fill(p.panel)
                        .stroke(Stroke::new(1.0, p.border)),
                )
                .show(ui, |ui| toolbar::actions_bar(ui, tab, &mut actions));
        }
        let clone_status = self
            .clone
            .as_ref()
            .map(|(url, _, s)| format!("Cloning {url}  ·  {s}"));
        egui::Panel::bottom("status")
            .exact_size(26.0)
            .frame(
                egui::Frame::new()
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0, p.border)),
            )
            .show(ui, |ui| {
                if let Some(s) = &clone_status {
                    ui.horizontal_centered(|ui| {
                        ui.add_space(10.0);
                        ui.add(egui::Spinner::new().size(12.0).color(p.accent));
                        ui.label(RichText::new(s).size(12.0).color(p.accent));
                    });
                } else {
                    statusbar::show(
                        ui,
                        self.tabs.get(self.active),
                        &self.git_version,
                        &self.updater.state,
                        &mut actions,
                    );
                }
            });

        if let Some(tab) = self.tabs.get_mut(self.active) {
            egui::Panel::left("sidebar")
                .resizable(true)
                .default_size(250.0)
                .min_size(190.0)
                .max_size(460.0)
                .frame(
                    egui::Frame::new()
                        .fill(p.panel)
                        .stroke(Stroke::new(1.0, p.border))
                        .inner_margin(egui::Margin::symmetric(6, 0)),
                )
                .show(ui, |ui| sidebar::show(ui, tab, &mut actions));
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(p.bg))
                .show(ui, |ui| match tab.view {
                    View::History => history::show(ui, tab, &self.settings, &mut actions),
                    View::Changes => changes::show(ui, tab, &mut actions),
                    View::Requests => {
                        requests::show(ui, tab, self.settings.hosting_api, &mut actions)
                    }
                });
        } else {
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(p.bg))
                .show(ui, |ui| welcome::show(ui, &self.settings, &mut actions));
        }

        // Overlays.
        if let Some(screen) = &mut self.settings_screen {
            let before = self.settings.clone();
            let forges: Vec<canopy_core::forge::Forge> = self
                .tabs
                .iter()
                .flat_map(|t| t.forges.iter().map(|(_, f)| f.clone()))
                .collect();
            let result = settings_screen::show(
                &ctx,
                screen,
                &mut self.settings,
                &mut self.tokens,
                &forges,
                &self.updater.state,
                &mut actions,
            );
            if result.tokens_changed {
                let enabled = self.settings.hosting_api;
                for t in &mut self.tabs {
                    let saved = t
                        .forge()
                        .and_then(|(_, f)| self.tokens.get_for(f))
                        .map(str::to_owned);
                    t.restart_host(&ctx, saved, enabled);
                }
            }
            if result.close {
                self.settings_screen = None;
            }
            self.settings_changed(&ctx, &before);
        }
        // Dialogs draw above the settings screen (the Privacy Policy opens from there).
        if let Some(d) = self.dialogs.current_mut() {
            let consent = is_consent(d);
            match dialogs::show(&ctx, d, self.tabs.get(self.active)) {
                Outcome::Keep => {}
                Outcome::Close => self.dialogs.close(),
                Outcome::Run(a) => {
                    // The consent dialog's actions decide what comes next themselves.
                    if !consent {
                        self.dialogs.close();
                    }
                    actions.extend(a);
                }
            }
        }
        if self.palette.open {
            // Built when the palette opens or its inputs change, not every frame.
            let inputs = self.palette_inputs();
            if self.palette.needs_entries(inputs) {
                let entries = self.palette_entries();
                self.palette.set_entries(inputs, entries);
            }
            if let Some(a) = self.palette.show(&ctx) {
                actions.push(a);
            }
        }
        self.cheatsheet.show(&ctx);
        self.toasts.show(&ctx);
        self.apply(&ctx, actions);
        self.handle_screenshot(&ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.screenshot.is_some() {
            return;
        }
        self.settings.open_tabs = self.tabs.iter().map(|t| t.path.clone()).collect();
        eframe::set_value(storage, settings::KEY, &self.settings);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::from(self.settings.theme.palette().bg).to_array()
    }
}
