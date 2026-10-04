//! The application: tabs, keyboard commands, dialogs, the command palette, toasts and the
//! update check.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, unbounded};
use egui::{RichText, Stroke};
use egui_phosphor::regular as icon;

use canopy_core::Repo;
use canopy_core::identity::Identity;
use canopy_hosting::{NewRequest, RequestSummary};

use crate::hosting::{HostJob, TokenStore};
use crate::identity_worker::{IdentityWorker, Job as IdentityJob};
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
    "identity",
    "settings-git",
    "clear-identity",
    "update-installed",
    "update-running",
    "trust-folder",
    "diff-options",
];

/// Options from the command line.
#[derive(Debug, Default, Clone)]
pub struct Launch {
    pub paths: Vec<PathBuf>,
    pub theme: Option<ThemeId>,
    pub zoom: Option<f32>,
    pub screenshot: Option<PathBuf>,
    pub view: Option<View>,
    /// Open this overlay for a screenshot (see [`OVERLAYS`]).
    pub overlay: Option<String>,
    /// Side-by-side diffs.
    pub split: bool,
}

/// Rows a Page Down / Ctrl+D moves in a list.
const PAGE_ROWS: isize = 12;

fn apply_zoom(ctx: &egui::Context, zoom: f32) {
    ctx.set_zoom_factor(zoom);
}

fn update_window_minimum(ctx: &egui::Context) {
    let zoom = ctx.zoom_factor();
    let scale = (zoom, ctx.pixels_per_point());
    let id = egui::Id::new(("native-window-scale", ctx.viewport_id()));
    let changed = ctx.data_mut(|data| {
        let previous = data.get_temp::<(f32, f32)>(id);
        data.insert_temp(id, scale);
        previous != Some(scale)
    });
    if changed {
        // egui applies a requested zoom next pass; native commands use this pass's scale.
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(
            theme::MIN_WINDOW_SIZE / zoom,
        ));
    }
}

/// The Canopy application state.
pub struct CanopyApp {
    settings: Settings,
    tabs: Vec<RepoTab>,
    active: usize,
    /// The dialog on screen and those waiting behind it (start-up notices after the consent,
    /// dialogs that arrived while a credential prompt was open).
    dialogs: DialogStack,
    settings_screen: Option<SettingsScreen>,
    folder_picker: crate::ui::folder_picker::FolderPicker,
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
    identity_worker: Option<IdentityWorker>,
    global_identity: Option<Identity>,
    global_identity_error: Option<String>,
    gitlab_hosts: Vec<String>,
    /// The start-up update check waits until the user has accepted the terms.
    update_check_due: bool,
    /// Receives git's and ssh's credential prompts (`None` when it could not start).
    askpass: Option<crate::askpass::Server>,
    /// `trust::changes()` as the tabs last saw it.
    trust_seen: u64,
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
            cc.egui_ctx
                .memory_mut(|memory| *memory = egui::Memory::default());
            // Screenshots are deterministic: default settings, nothing remembered.
            settings = Settings {
                seen_intro: true,
                check_updates: false,
                hosting_api: std::env::var("CANOPY_HOSTING_API_BASE")
                    .is_ok_and(|base| canopy_hosting::is_loopback_base(base.trim())),
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
        if let Some(zoom) = launch.zoom {
            settings.zoom = zoom;
        }
        if launch.split {
            settings.split_diff = true;
        }
        crate::logging::set_level(settings.log_level);
        // Before any tab opens, so a repository thread never sees an empty list.
        crate::trust::set(&settings.trusted_folders);
        crate::trust::install_gate();
        crate::ui::fonts::install(&cc.egui_ctx);
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        theme::apply(&cc.egui_ctx, settings.theme, settings.reduce_motion);
        apply_zoom(&cc.egui_ctx, settings.zoom);

        let git_version = canopy_core::cli::version().unwrap_or_else(|_| "git not found".into());
        let gitlab_hosts = settings.gitlab_host_list();
        let mut app = Self {
            trust_seen: crate::trust::changes(),
            settings,
            tabs: Vec::new(),
            active: 0,
            dialogs: DialogStack::default(),
            settings_screen: None,
            folder_picker: Default::default(),
            palette: CommandPalette::default(),
            cheatsheet: Cheatsheet::default(),
            toasts: Toasts::default(),
            screenshot: launch.screenshot.map(|path| Screenshot {
                path,
                ready_frames: 0,
                ready_since: None,
                requested: false,
                started: 0.0,
                popup: (launch.overlay.as_deref() == Some("diff-options"))
                    .then(|| egui::Id::new(crate::views::diff::OPTIONS_POPUP)),
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
            identity_worker: IdentityWorker::start(&cc.egui_ctx),
            global_identity: None,
            global_identity_error: None,
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
        if !shooting && !app.settings.identity_asked {
            app.request_identity(IdentityJob::ReadGlobal);
        }
        match launch.overlay.as_deref() {
            Some("settings") => app.settings_screen = Some(SettingsScreen::default()),
            Some("settings-git") => {
                app.settings_screen = Some(SettingsScreen::at(settings_screen::Section::Git))
            }
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
            Some("identity") => app.dialogs.open(Dialog::Identity {
                name: String::new(),
                email: String::new(),
                pending: false,
            }),
            Some("update-installed" | "update-running") => {
                let available = crate::updater::Available {
                    build: 1001,
                    manifest: canopy_hosting::updates::UpdateManifest {
                        version: "1001".into(), notes: String::new(), pub_date: None,
                        release_page: None, platforms: Default::default(),
                    },
                };
                app.updater.install = if launch.overlay.as_deref() == Some("update-installed") {
                    crate::updater::InstallState::Done {
                        build: 1001, installed: crate::selfupdate::Installed {
                            exe: Some("canopy".into()), notes: Vec::new(),
                        },
                    }
                } else {
                    crate::updater::InstallState::Running {
                        build: 1001, phase: crate::selfupdate::Phase::Installing,
                    }
                };
                app.dialogs.open(Dialog::Update(Box::new(available)));
            }
            // A fixed illustrative path, so the screenshot never shows the fixture's temporary one.
            Some("trust-folder") => app.dialogs.open(Dialog::TrustFolder {
                repo: PathBuf::from("/home/ada/src/tidepool"),
                remote: "origin".into(),
            }),
            Some("clear-identity") => app.dialogs.open(Dialog::ConfirmClearIdentity {
                repo: None,
                scope: canopy_core::identity::Scope::Global,
            }),
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
        let version = crate::build_info::NUMBER;
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
        let _ = writeln!(t, "Canopy {}", crate::build_info::label());
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
        self.offer_identity_step();
    }

    /// Applies settings that changed in the settings screen.
    fn settings_changed(&mut self, ctx: &egui::Context, before: &Settings) {
        let s = &self.settings;
        if s.theme != before.theme || s.reduce_motion != before.reduce_motion {
            theme::apply(ctx, s.theme, s.reduce_motion);
        }
        if (ctx.zoom_factor() - s.zoom).abs() > f32::EPSILON {
            apply_zoom(ctx, s.zoom);
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

/// The "What's new" dialog for this build: the release notes the release workflow embedded, else
/// the changelog's Unreleased section (development builds).
fn whats_new() -> Option<Dialog> {
    use crate::ui::notes::{CHANGELOG, changelog_section};
    let embedded = crate::build_info::RELEASE_NOTES.trim();
    let notes = if embedded.is_empty() {
        changelog_section(CHANGELOG, "Unreleased")?
    } else {
        embedded.to_owned()
    };
    Some(Dialog::WhatsNew {
        version: crate::build_info::label(),
        notes,
    })
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
        update_window_minimum(&ctx);
        let p = palette(&ctx);
        let compact = ctx.content_rect().width() < theme::COMPACT_WIDTH;
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
            self.request_identity(IdentityJob::ReadRepo(t.path.clone()));
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
        if let Some(tab) = self.tabs.get_mut(self.active) {
            egui::Panel::top("actions")
                .exact_size(theme::XL + theme::LG + theme::XS)
                .frame(
                    egui::Frame::new()
                        .fill(p.panel)
                        .stroke(Stroke::new(1.0, p.border)),
                )
                .show(ui, |ui| {
                    if compact {
                        ui.horizontal_centered(|ui| {
                            let menu = navigation_menu(ui, |ui| {
                                ui.set_width(240.0);
                                ui.set_max_height(ctx.content_rect().height() * 0.7);
                                sidebar::show(ui, tab, &mut actions);
                            })
                            .on_hover_text("Branches, remotes, tags and stashes");
                            menu.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    true,
                                    "Branches, remotes, tags and stashes",
                                )
                            });
                            toolbar::actions_bar(ui, tab, &mut actions);
                        });
                    } else {
                        toolbar::actions_bar(ui, tab, &mut actions);
                    }
                });
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
                        ui.add(egui::Spinner::new().size(theme::META_SIZE).color(p.accent));
                        ui.label(RichText::new(s).size(theme::META_SIZE).color(p.accent));
                    });
                } else {
                    statusbar::show(
                        ui,
                        self.tabs.get(self.active),
                        &self.git_version,
                        &self.updater.state,
                        &self.updater.install,
                        &mut actions,
                    );
                }
            });

        if let Some(tab) = self.tabs.get_mut(self.active) {
            if !compact {
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
            }
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
        if self.settings_screen.is_some()
            || self.dialogs.current().is_some()
            || self.palette.open
            || self.cheatsheet.open
        {
            crate::ui::widgets::dismiss_underlying_tooltips(&ctx);
        }
        self.folder_picker.begin_frame();
        if let Some(screen) = &mut self.settings_screen {
            let before = self.settings.clone();
            let active_tab = self.tabs.get(self.active);
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
                settings_screen::SettingsData {
                    forges: &forges,
                    update: &self.updater.state,
                    identity: settings_screen::IdentityView {
                        repo: active_tab.map(|t| t.path.as_path()),
                        identity: active_tab
                            .map_or(self.global_identity.as_ref(), |t| t.identity.as_ref()),
                        error: active_tab.map_or(self.global_identity_error.as_deref(), |t| {
                            t.identity_error.as_deref()
                        }),
                    },
                },
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
                self.folder_picker.cancel();
                self.settings_screen = None;
            }
            self.settings_changed(&ctx, &before);
        }
        // Dialogs draw above the settings screen (the Privacy Policy opens from there).
        if let Some(d) = self.dialogs.current_mut() {
            crate::ui::widgets::dismiss_underlying_tooltips(&ctx);
            let consent = is_consent(d);
            match dialogs::show(
                &ctx,
                d,
                self.tabs.get(self.active),
                &mut self.folder_picker,
                self.clone.is_some(),
                &self.updater,
            ) {
                Outcome::Keep => {}
                Outcome::Close => {
                    self.folder_picker.cancel();
                    self.dialogs.close();
                }
                Outcome::RunKeep(a) => {
                    self.folder_picker.cancel();
                    actions.extend(a);
                }
                Outcome::Run(a) => {
                    // The consent dialog's actions decide what comes next themselves.
                    if !consent {
                        self.dialogs.close();
                    }
                    actions.extend(a);
                }
            }
        }
        self.folder_picker.end_frame();
        if self.palette.open {
            crate::ui::widgets::dismiss_underlying_tooltips(&ctx);
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
        if self.cheatsheet.open {
            crate::ui::widgets::dismiss_underlying_tooltips(&ctx);
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

fn navigation_menu(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) -> egui::Response {
    let button = ui.button(RichText::new(icon::LIST).size(theme::TITLE_SIZE));
    let id = button.id.with("navigation-open");
    let mut open = ui.data(|data| data.get_temp::<bool>(id)).unwrap_or(false);
    if button.clicked() {
        open = !open;
    }
    let shown = egui::Popup::menu(&button)
        .open_bool(&mut open)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(content)
        .is_some();
    // Context menus use egui's single popup slot; the navigation must keep their rows alive.
    if shown && egui::Popup::is_any_open(ui.ctx()) {
        open = true;
    }
    ui.data_mut(|data| data.insert_temp(id, open));
    button
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_navigation_stays_open_for_context_actions() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        let draw = |events| {
            let mut menu = egui::Rect::NOTHING;
            let mut branch = egui::Rect::NOTHING;
            let mut seen = false;
            let mut nested = None;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    menu = navigation_menu(ui, |ui| {
                        ui.set_width(240.0);
                        ui.set_min_height(420.0);
                        seen = true;
                        let response = ui.button("main");
                        branch = response.rect;
                        let navigation_layer = response.layer_id;
                        if let Some(menu) = crate::ui::widgets::context_menu(&response, |ui| {
                            ui.label("Rename branch");
                        }) {
                            nested = Some((menu.response.rect.center(), menu.response.layer_id));
                        }
                        ctx.move_to_top(navigation_layer);
                    })
                    .rect;
                },
            );
            output.textures_delta.clear();
            (menu, branch, seen, nested)
        };
        let (menu, _, _, _) = draw(Vec::new());
        let click = |pos, button, pressed| egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Default::default(),
        };
        draw(vec![
            egui::Event::PointerMoved(menu.center()),
            click(menu.center(), egui::PointerButton::Primary, true),
        ]);
        let (_, branch, _, _) = draw(vec![click(
            menu.center(),
            egui::PointerButton::Primary,
            false,
        )]);
        assert!(branch.is_positive(), "menu must open");
        let (_, branch, _, _) = draw(Vec::new());
        draw(vec![
            egui::Event::PointerMoved(branch.center()),
            click(branch.center(), egui::PointerButton::Secondary, true),
        ]);
        draw(vec![click(
            branch.center(),
            egui::PointerButton::Secondary,
            false,
        )]);
        for _ in 0..3 {
            let (_, _, seen, nested) = draw(Vec::new());
            assert!(
                seen,
                "a context click must preserve repository navigation across frames"
            );
            let (pos, layer) = nested.expect("branch context menu must stay open");
            assert_eq!(
                ctx.layer_id_at(pos),
                Some(layer),
                "context actions must paint above navigation"
            );
        }
    }

    #[test]
    fn zoom_transitions_use_the_current_frame_scale_for_window_constraints() {
        for (from, to) in [(1.5, 1.0), (1.0, 1.6), (1.6, 0.8)] {
            let ctx = egui::Context::default();
            ctx.set_zoom_factor(from);
            let mut warmup = ctx.run_ui(egui::RawInput::default(), |_| {});
            warmup.textures_delta.clear();
            let mut current = 0.0;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                current = ui.ctx().zoom_factor();
                update_window_minimum(ui.ctx());
                apply_zoom(ui.ctx(), to);
            });
            output.textures_delta.clear();
            let minimum = output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .find_map(|cmd| {
                    if let egui::ViewportCommand::MinInnerSize(size) = cmd {
                        Some(*size)
                    } else {
                        None
                    }
                })
                .expect("native window constraint during transition");
            assert_eq!(
                minimum * current,
                theme::MIN_WINDOW_SIZE,
                "zoom {from} -> {to}"
            );
            let mut next = ctx.run_ui(egui::RawInput::default(), |ui| {
                update_window_minimum(ui.ctx())
            });
            next.textures_delta.clear();
            assert_eq!(ctx.zoom_factor(), to);
            let next_minimum = next.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .find_map(|cmd| {
                    if let egui::ViewportCommand::MinInnerSize(size) = cmd {
                        Some(*size)
                    } else {
                        None
                    }
                })
                .expect("constraint for the newly active zoom");
            assert_eq!(next_minimum * to, theme::MIN_WINDOW_SIZE);
        }
    }

    #[test]
    fn zoom_constraints_stay_constant_in_steady_and_discarded_passes() {
        for (from, to) in [(1.1, 1.3), (1.3, 1.5), (1.5, 0.8)] {
            let ctx = egui::Context::default();
            apply_zoom(&ctx, from);
            for _ in 0..2 {
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    update_window_minimum(ui.ctx())
                });
                output.textures_delta.clear();
            }
            let mut requested = false;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                update_window_minimum(ui.ctx());
                if !requested {
                    requested = true;
                    apply_zoom(ui.ctx(), to);
                    ui.ctx().request_discard("zoom transition");
                }
            });
            output.textures_delta.clear();
            assert_eq!(ctx.zoom_factor(), to);
            let commands = &output.viewport_output[&egui::ViewportId::ROOT].commands;
            let mut constraints = 0;
            for command in commands {
                if let egui::ViewportCommand::MinInnerSize(minimum) = command {
                    constraints += 1;
                    let physical = (*minimum * ctx.zoom_factor()).round();
                    assert_eq!(physical, theme::MIN_WINDOW_SIZE, "zoom {from} -> {to}");
                }
            }
            assert!(
                constraints > 0,
                "new zoom must synchronize its native constraint"
            );
        }
    }

    #[test]
    fn interface_zoom_keeps_the_native_window_minimum_constant() {
        let ctx = egui::Context::default();
        for zoom in [0.8, 1.0, 1.6] {
            apply_zoom(&ctx, zoom);
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                update_window_minimum(ui.ctx())
            });
            output.textures_delta.clear();
            let minimum = output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .find_map(|command| match command {
                    egui::ViewportCommand::MinInnerSize(size) => Some(*size),
                    _ => None,
                })
                .expect("native window constraint");
            assert_eq!(minimum * zoom, egui::vec2(900.0, 560.0));
        }
    }
}
