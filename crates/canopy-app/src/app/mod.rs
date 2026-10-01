//! The application: tabs, keyboard commands, dialogs, the command palette, toasts and the
//! update check.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, unbounded};
use egui::{RichText, Stroke};
use egui_phosphor::regular as icon;

use canopy_core::Repo;
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
use crate::views::dialogs::{self, Dialog, DialogStack, Outcome, Start};
use crate::views::settings_screen::{self, SettingsScreen};
use crate::views::{changes, history, requests, sidebar, statusbar, toolbar, welcome};
use crate::worker::{Job, Op};

mod background;
mod clone;
mod dispatch;
mod input;
mod open;
mod palette;
mod request_flow;
mod screenshot;

pub use crate::action::Action;
use clone::CloneMsg;
use screenshot::Screenshot;

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
    /// The thread that opens and creates repositories (`None` when it could not start).
    opener: Option<(
        crossbeam_channel::Sender<open::OpenRequest>,
        Receiver<open::Opened>,
    )>,
    /// Repositories asked for and not opened yet (a screenshot waits for them).
    pending_opens: usize,
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
            opener: open::start(&cc.egui_ctx),
            pending_opens: 0,
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
        for (i, path) in restore.into_iter().enumerate() {
            let request = open::OpenRequest {
                path,
                create: false,
                focus: i == 0,
            };
            app.request_open(&cc.egui_ctx, request);
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

    fn tab(&mut self) -> Option<&mut RepoTab> {
        self.tabs.get_mut(self.active)
    }

    fn set_theme(&mut self, ctx: &egui::Context, id: ThemeId) {
        self.settings.theme = id;
        theme::apply(ctx, id, self.settings.reduce_motion);
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

fn git_version_missing(v: &str) -> bool {
    !v.starts_with("git version")
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
