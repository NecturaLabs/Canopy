//! The application: tabs, keyboard commands, dialogs, the command palette, toasts and the
//! update check.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, unbounded};
use egui::RichText;
use egui_phosphor::regular as icon;

use canopy_core::Repo;
use canopy_core::identity::Identity;
use canopy_hosting::{NewRequest, RequestSummary};

use crate::hosting::HostJob;
use crate::identity_worker::{IdentityWorker, Job as IdentityJob};
use crate::keymap::{self, Command};
use crate::settings::{self, Settings};
use crate::tab::{RepoTab, View};
use crate::ui::cheatsheet::Cheatsheet;
use crate::ui::palette::{CommandPalette, Entry};
use crate::ui::theme::{self, ThemeId, palette};
use crate::ui::toast::{Offer, ToastKind, Toasts, Topic};
use crate::updater::{self, UpdateState, Updater};
use crate::views::dialogs::{self, Dialog, DialogStack, Outcome, Start};
use crate::views::settings_screen::{self, SettingsScreen};
use crate::views::{changes, history, requests, sidebar, statusbar, toolbar, welcome};
use crate::worker::{Job, Op};

mod background;
mod clone;
pub mod demo_requests;
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
    "credential",
    "host-key",
    "merge",
    "identity",
    "settings-git",
    "settings-git-pull",
    "settings-git-commits",
    "settings-git-config",
    "clear-identity",
    "update-installed",
    "update-running",
    "trust-prompt",
    "trust-banner",
    "trust-safety-off",
    "privacy-trust-off",
    "diff-options",
    "settings-integrations",
    "settings-accounts",
    "settings-updates",
    "settings-notifications",
    "settings-keyboard",
    "settings-backup",
    "settings-repository",
    "settings-repository-pull",
    "settings-repository-diffs",
    "settings-search",
    "settings-search-none",
    "about",
    "repository-insights",
    "file-history",
    "file-history-commit",
    "search-changes",
    "clean-stale",
    "author-card",
    "request-filters",
    "request-sort",
    "commit-expanded",
    "commit-compact",
    "amend-commit",
    "toasts",
    "notifications",
    "branch-selected",
    "long-message",
    "gallery",
    "menu-branch",
    "menu-commit",
    "menu-file",
    "blame",
    "open",
    "open-error",
    "clone",
    "clone-running",
    "clone-error",
    "init",
    "new-branch",
    "new-worktree",
    "new-worktree-error",
    "rename",
    "add-submodule",
    "git-missing",
    "remove-submodule",
    "new-tag",
    "stash",
    "export-patch",
    "apply-patch",
    "add-remote",
    "add-remote-refused",
    "publish",
    "new-request",
    "confirm",
    "confirm-danger",
    "confirm-host",
    "convert-partial-clone",
    "confirm-merge-risky",
    "diverged",
    "pull-conflicts",
    "pull-moved",
    "whats-new",
    "crash",
    "update-available",
    "update-failed",
    "terms",
    "consent-reading",
    "crash-reading",
    "username",
    "password",
    "approve",
    "notice",
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
    pub style: Option<theme::Style>,
    pub density: Option<theme::Density>,
    pub request_density: Option<crate::request_view::RequestDensity>,
    pub translucent: Option<bool>,
    pub high_contrast: Option<bool>,
    pub custom_theme: Option<crate::ui::custom_theme::CustomTheme>,
    pub motion: Option<crate::ui::motion::MotionPref>,
    /// The gallery page (window heights scrolled) for `--overlay gallery`.
    pub gallery_page: usize,
    /// `--glass-soak N` (debug builds): paint floating layers, glass when Translucent surfaces is
    /// on, for a warm-up and then N measured frames, then report and exit.
    pub glass_soak: Option<u64>,
}

/// The folders a screenshot trusts: the repositories it opens, so that a picture of the Changes
/// view is not a picture of the prompt, and `CANOPY_SCREENSHOT_TRUSTED`'s folder. Nothing for
/// `--overlay trust-banner`, which shows a repository that is not.
fn screenshot_trusted(launch: &Launch) -> Vec<PathBuf> {
    let mut folders = Vec::new();
    let opened = launch
        .paths
        .iter()
        .filter(|_| launch.overlay.as_deref() != Some("trust-banner"));
    let named = std::env::var_os("CANOPY_SCREENSHOT_TRUSTED").map(PathBuf::from);
    for folder in opened.chain(named.iter()) {
        crate::trust::add(&mut folders, folder);
    }
    folders
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
    /// Git configuration reads started for tabs just opened, answered once into the
    /// notification center.
    config_reads: Vec<(PathBuf, Receiver<crate::repo_config::GitFacts>)>,
    git_version: String,
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
    /// The theme and appearance installed in egui, so they are re-applied only when they change.
    applied: Option<(ThemeId, bool, theme::CustomTheme, theme::Appearance)>,
    /// The gallery overlay (`--overlay gallery`) and the page it shows.
    gallery: Option<usize>,
    /// The translucent-surfaces soak run (`--glass-soak`), if one was asked for.
    glass_soak: Option<crate::ui::kit::glass::Soak>,
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
                // A repository is trusted, so the picture shows the view and not the prompt that
                // asks, unless the overlay is about trust (`--overlay trust-banner`).
                // CANOPY_SCREENSHOT_TRUSTED names another folder; a picture of what a trusted
                // folder shows (a signature, say) names the folder.
                trusted_folders: screenshot_trusted(&launch),
                trust_safety: launch.overlay.as_deref() != Some("privacy-trust-off"),
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
        crate::trust::publish(&settings);
        crate::trust::install_gate();
        crate::ui::fonts::install(&cc.egui_ctx);
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        if let Some(style) = launch.style {
            settings.style = style;
        }
        if let Some(density) = launch.density {
            settings.density = density;
        }
        if let Some(density) = launch.request_density {
            settings.request_density = density;
        }
        if let Some(translucent) = launch.translucent {
            settings.translucent = translucent;
        }
        if let Some(high_contrast) = launch.high_contrast {
            settings.high_contrast = high_contrast;
        }
        if let Some(custom) = launch.custom_theme {
            settings.custom_theme = custom;
        }
        if let Some(motion) = launch.motion {
            settings.motion = motion;
        }
        // Screenshots stay deterministic: no system answer (motion counts as not reduced).
        if !shooting && settings.motion == crate::ui::motion::MotionPref::System {
            crate::ui::motion::query_system(&cc.egui_ctx);
        }
        crate::ui::kit::glass::init(cc.gl.as_deref());
        if !shooting {
            crate::ui::kit::glass::query_system(&cc.egui_ctx);
        }
        apply_zoom(&cc.egui_ctx, settings.zoom);

        let git_version = canopy_core::cli::version().unwrap_or_else(|_| "git not found".into());
        let gitlab_hosts = settings.gitlab_host_list();
        // Screenshots and tests must not delete anything of the real user's.
        if !shooting {
            crate::hosting::remove_saved_tokens();
        }
        let mut app = Self {
            trust_seen: crate::trust::changes(),
            settings,
            tabs: Vec::new(),
            active: 0,
            dialogs: DialogStack::default(),
            settings_screen: None,
            folder_picker: crate::ui::folder_picker::FolderPicker::default(),
            palette: CommandPalette::default(),
            cheatsheet: Cheatsheet::default(),
            toasts: Toasts::default(),
            screenshot: launch
                .screenshot
                .map(|path| Screenshot::new(path, launch.overlay.as_deref())),
            initial_view: launch.view,
            clone: None,
            opener: open::start(&cc.egui_ctx),
            pending_opens: 0,
            config_reads: Vec::new(),
            git_version,
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
            applied: None,
            gallery: (launch.overlay.as_deref() == Some("gallery")).then_some(launch.gallery_page),
            glass_soak: launch
                .glass_soak
                .map(|n| crate::ui::kit::glass::Soak::new(n, crate::ui::kit::glass::Soak::CYCLE)),
        };
        app.apply_look(&cc.egui_ctx);
        if !shooting
            && let Some(kept) = cc
                .storage
                .and_then(|s| s.get_string(crate::ui::toast::STORAGE_KEY))
        {
            app.toasts.restore_json(&kept);
        }
        let restore = if launch.paths.is_empty() && !shooting {
            app.settings.open_tabs.clone()
        } else {
            launch.paths.clone()
        };
        for (i, path) in restore.into_iter().enumerate() {
            let request = open::OpenRequest {
                path,
                create: None,
                focus: i == 0,
                replace: None,
                convert_legacy: false,
            };
            app.request_open(&cc.egui_ctx, request);
        }
        if !shooting && !app.settings.identity_asked {
            app.request_identity(IdentityJob::ReadGlobal);
        }
        if let Some(menu) = app.screenshot.as_ref().and_then(|s| s.menu.as_ref()) {
            // The menu's widget is found by what a screen reader hears.
            cc.egui_ctx.enable_accesskit();
            app.initial_view.get_or_insert(menu.view);
        }
        match launch.overlay.as_deref() {
            Some("settings") => app.settings_screen = Some(SettingsScreen::default()),
            Some("settings-git" | "settings-git-pull") => {
                app.settings_screen = Some(SettingsScreen::at(settings_screen::Section::Git));
            }
            Some("settings-git-commits") => {
                app.settings_screen = Some(SettingsScreen::at_group(
                    settings_screen::Section::Git,
                    "Commits",
                ));
            }
            Some("settings-git-config") => {
                app.settings_screen = Some(SettingsScreen::at_group(
                    settings_screen::Section::Git,
                    "Global Git configuration",
                ));
            }
            Some("settings-integrations") => {
                app.settings_screen =
                    Some(SettingsScreen::at(settings_screen::Section::Integrations));
            }
            Some("settings-accounts") => {
                app.settings_screen = Some(SettingsScreen::at(settings_screen::Section::Accounts));
            }
            Some("settings-updates") => {
                app.settings_screen = Some(SettingsScreen::at(settings_screen::Section::Updates));
            }
            Some("settings-notifications") => {
                app.settings_screen =
                    Some(SettingsScreen::at(settings_screen::Section::Notifications));
            }
            Some("about") => app.dialogs.open(Dialog::About),
            Some("long-message") => {
                app.initial_view.get_or_insert(View::Changes);
            }
            Some("notifications") => {
                // A notification of each kind, for the center the overlay opens.
                let ctx = &cc.egui_ctx;
                app.toasts.push(
                    ctx,
                    ToastKind::Success,
                    Topic::Finished,
                    "Pushed",
                    "main to origin",
                );
                app.toasts.push(
                    ctx,
                    ToastKind::Info,
                    Topic::Confirmation,
                    "Copied",
                    "3f9a2c1",
                );
                app.toasts.push(
                    ctx,
                    ToastKind::Error,
                    Topic::Error,
                    "Push failed",
                    "! [rejected]        main -> main (fetch first)\nerror: failed to push some refs to 'origin'",
                );
                app.toasts.push_offer(
                    ctx,
                    ToastKind::Info,
                    Topic::Update,
                    "Canopy 812 is available",
                    "",
                    Offer::ReviewUpdate,
                );
            }
            Some("toasts") => {
                // Every type, so the stack shows a finished operation beside the error.
                app.toasts.set_prefs(settings::NotificationPrefs {
                    finished: true,
                    ..Default::default()
                });
                // A known failure in a repository, so its card offers the pull.
                app.toasts.push_for(
                    &cc.egui_ctx,
                    ToastKind::Error,
                    Topic::Error,
                    "Push failed",
                    "! [rejected]        main -> main (fetch first)\nerror: failed to push some refs to 'origin'",
                    launch.paths.first().map(PathBuf::as_path),
                );
                app.toasts.push(
                    &cc.egui_ctx,
                    ToastKind::Success,
                    Topic::Finished,
                    "Committed",
                    "docs: add API reference",
                );
                app.toasts.push_offer(
                    &cc.egui_ctx,
                    ToastKind::Info,
                    Topic::Update,
                    "Canopy 812 is available",
                    "",
                    Offer::ReviewUpdate,
                );
            }
            Some("settings-keyboard") => {
                app.settings_screen = Some(SettingsScreen::at(settings_screen::Section::Keyboard));
            }
            Some("settings-backup") => {
                app.settings_screen = Some(SettingsScreen::at(settings_screen::Section::Data));
            }
            Some("settings-repository") => {
                app.settings_screen = Some(SettingsScreen::awaiting_repository());
            }
            Some("settings-repository-pull" | "settings-repository-diffs") => {
                let group = if launch.overlay.as_deref() == Some("settings-repository-pull") {
                    "Pull and fetch"
                } else {
                    "Diffs"
                };
                app.settings_screen = Some(SettingsScreen::awaiting_repository_at(group));
            }
            Some("settings-search-none") => {
                app.settings_screen = Some(SettingsScreen::searching("xyzzy"));
            }
            Some("settings-search") => {
                app.settings_screen = Some(SettingsScreen::searching("pull"));
            }
            Some("cheatsheet") => app.cheatsheet.open = true,
            Some("palette") => app.palette.open(),
            Some("privacy" | "privacy-trust-off") => {
                app.settings_screen = Some(SettingsScreen::at(settings_screen::Section::Privacy));
            }
            Some("consent") => {
                app.dialogs.open(Dialog::Consent {
                    step: crate::ui::legal::Pending::Terms,
                    updated: false,
                    reading: None,
                });
            }
            Some("crash-consent") => {
                app.dialogs.open(Dialog::Consent {
                    step: crate::ui::legal::Pending::CrashReports,
                    updated: false,
                    reading: None,
                });
            }
            Some("legal") => app
                .dialogs
                .open(Dialog::Legal(crate::ui::legal::Document::Privacy)),
            Some("credential") => {
                app.dialogs
                    .open(Dialog::credential(crate::askpass::Prompt::preview(
                        "Enter passphrase for key '/home/ada/.ssh/id_ed25519': ",
                    )));
            }
            Some("host-key") => {
                app.dialogs.open(Dialog::credential(crate::askpass::Prompt::preview(
                    "The authenticity of host 'github.com (140.82.121.4)' can't be established.\nED25519 key fingerprint is SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU.\nThis key is not known by any other names.\nAre you sure you want to continue connecting (yes/no/[fingerprint])? ",
                )));
            }
            Some("merge") => {
                app.dialogs.open(Dialog::Merge {
                    target: "feature/search".into(),
                    rebase: false,
                    mode: canopy_core::MergeMode::Squash,
                });
            }
            Some("identity") => app.dialogs.open(Dialog::Identity {
                name: String::new(),
                email: String::new(),
                pending: false,
            }),
            Some("update-installed" | "update-running") => {
                let available = updater::Available {
                    build: 1001,
                    manifest: canopy_hosting::updates::UpdateManifest {
                        version: "1001".into(),
                        notes: String::new(),
                        pub_date: None,
                        release_page: None,
                        platforms: std::collections::BTreeMap::default(),
                    },
                };
                app.updater.install = if launch.overlay.as_deref() == Some("update-installed") {
                    updater::InstallState::Done {
                        build: 1001,
                        installed: crate::selfupdate::Installed {
                            exe: Some("canopy".into()),
                            notes: Vec::new(),
                        },
                    }
                } else {
                    updater::InstallState::Running {
                        build: 1001,
                        phase: crate::selfupdate::Phase::Installing,
                    }
                };
                app.dialogs.open(Dialog::Update(Box::new(available)));
            }
            // A fixed illustrative path, so the screenshot never shows the fixture's temporary one.
            Some("trust-prompt") => app.dialogs.open(Dialog::Trust {
                repo: PathBuf::from("/home/ada/src/tidepool"),
                folders: vec![
                    PathBuf::from("/home/ada/src/tidepool"),
                    PathBuf::from("/home/ada/src/tidepool-main/.git"),
                ],
            }),
            Some("trust-safety-off") => app.dialogs.open(Dialog::ConfirmTrustSafetyOff),
            Some("clear-identity") => app.dialogs.open(Dialog::ConfirmClearIdentity {
                repo: None,
                scope: canopy_core::identity::Scope::Global,
            }),
            // A menu (screenshot::MenuShot) opens by a right click once the window has settled.
            Some(other) if !other.starts_with("menu-") => app.open_dialog_overlay(other),
            Some(_) | None => {}
        }
        if git_version_missing(&app.git_version) {
            app.dialogs.open(Dialog::GitMissing);
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
            version.clone_into(&mut app.settings.last_seen_version);
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

    /// The tab of the repository at `repo`, whether or not it is active.
    fn tab_at(&mut self, repo: &std::path::Path) -> Option<&mut RepoTab> {
        self.tabs.iter_mut().find(|t| t.path == repo)
    }

    fn set_theme(&mut self, ctx: &egui::Context, id: ThemeId) {
        self.settings.theme = id;
        self.apply_look(ctx);
    }

    /// What the settings, the system's motion and transparency answers and the graphics backend
    /// make render.
    fn look(&self) -> theme::Appearance {
        let s = &self.settings;
        theme::Appearance::resolve(
            s.theme,
            s.style,
            s.density,
            s.translucent,
            s.motion.reduces(crate::ui::motion::system()),
            crate::ui::kit::glass::available(),
            crate::ui::kit::glass::reduced_transparency(),
        )
    }

    /// Installs the theme and appearance when either changed (a setting, or the system's motion
    /// answer arriving).
    fn apply_look(&mut self, ctx: &egui::Context) {
        theme::set_high_contrast(self.settings.high_contrast);
        theme::set_custom(self.settings.custom_theme);
        let look = self.look();
        let wanted = (
            self.settings.theme,
            self.settings.high_contrast,
            self.settings.custom_theme,
            look,
        );
        if self.applied != Some(wanted) {
            theme::apply(ctx, self.settings.theme, look);
            self.applied = Some(wanted);
        }
        theme::set_graph(self.settings.graph_node, self.settings.graph_curve);
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
        let _ = writeln!(
            t,
            "High contrast: {}",
            if self.settings.high_contrast {
                "on"
            } else {
                "off"
            }
        );
        let _ = writeln!(
            t,
            "Translucent surfaces: {}, blur {}",
            if self.settings.translucent {
                "on"
            } else {
                "off"
            },
            crate::ui::kit::glass::describe()
        );
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
        let home = settings::home_dir().display().to_string();
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
    fn settings_changed(&mut self, ctx: &egui::Context, before: Watched) {
        let s = &self.settings;
        if s.motion == crate::ui::motion::MotionPref::System
            && before.motion != crate::ui::motion::MotionPref::System
        {
            crate::ui::motion::query_system(ctx);
        }
        self.apply_look(ctx);
        let s = &self.settings;
        if (ctx.zoom_factor() - s.zoom).abs() > f32::EPSILON {
            apply_zoom(ctx, s.zoom);
        }
        if Watched::hash(&s.gitlab_hosts) != before.gitlab_hosts {
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
        if s.request_density != before.request_density {
            for t in &mut self.tabs {
                t.request_density = s.request_density;
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
                t.restart_host(ctx, enabled);
            }
        }
    }
}

/// Names the window for assistive technology: the active repository and the app, as a window
/// title reads ("showcase — Canopy"), or just "Canopy" with none open.
fn name_window(ctx: &egui::Context, repository: Option<&str>) {
    let name = repository.map_or_else(|| "Canopy".to_owned(), |r| format!("{r} — Canopy"));
    ctx.accesskit_node_builder(egui::accesskit_root_id(), |node| node.set_label(name));
}

/// The settings whose change needs more than a repaint, as they were before Settings drew a
/// frame: compared after it, without copying the whole settings every frame Settings is open.
#[derive(Debug, Clone, Copy)]
struct Watched {
    motion: crate::ui::motion::MotionPref,
    diff: settings::DiffPrefs,
    split_diff: bool,
    request_density: crate::request_view::RequestDensity,
    syntax_highlight: bool,
    hosting_api: bool,
    /// A hash of the GitLab hosts text.
    gitlab_hosts: u64,
}

impl Watched {
    fn of(s: &Settings) -> Self {
        Self {
            motion: s.motion,
            diff: s.diff,
            split_diff: s.split_diff,
            request_density: s.request_density,
            syntax_highlight: s.syntax_highlight,
            hosting_api: s.hosting_api,
            gitlab_hosts: Self::hash(&s.gitlab_hosts),
        }
    }

    fn hash(text: &str) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut h);
        h.finish()
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
        name_window(&ctx, self.tabs.get(self.active).map(|t| t.name.as_str()));
        update_window_minimum(&ctx);
        self.apply_look(&ctx);
        let p = palette(&ctx);
        let compact = ctx.content_rect().width() < theme::COMPACT_WIDTH;
        self.poll(&ctx);
        if let Some(t) = self.tabs.get_mut(self.active) {
            t.diff_default = self.settings.diff;
        }
        self.apply_repo_looks();
        self.auto_fetch(&ctx);
        self.auto_refresh_requests(&ctx);
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

        let m = theme::metrics();
        let s = theme::surfaces(&ctx);
        let deeper = theme::style() == theme::Style::Deeper;
        let screen = ctx.content_rect();
        // The top deck: the tab strip and the action bar. Standard paints each as flat chrome over
        // a seam. Deeper leaves the tab strip on the ground and floats the action bar as a lifted
        // card in line with the panes below.
        let deck_height = m.tab_strip_height
            + if self.tabs.get(self.active).is_some() {
                m.action_bar_height
            } else {
                0.0
            };
        let background = ctx.layer_painter(egui::LayerId::background());
        background.rect_filled(screen, egui::CornerRadius::ZERO, s.bg);
        if deeper {
            // Deeper's gutter is open ground: the theme's ground treatment lies on it.
            crate::ui::kit::paint::ground(&background, screen, 1.0);
        }
        crate::ui::kit::surface::Deck::new(deck_height > m.tab_strip_height)
            .paint(&background, screen);
        egui::Panel::top("tabs")
            .exact_size(m.tab_strip_height)
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                toolbar::tabs(
                    ui,
                    &self.tabs,
                    self.active,
                    self.settings.theme,
                    &mut actions,
                );
            });
        if let Some(tab) = self.tabs.get_mut(self.active) {
            egui::Panel::top("actions")
                .exact_size(m.action_bar_height)
                .frame(egui::Frame::NONE)
                .show(ui, |ui| {
                    if compact {
                        ui.horizontal_centered(|ui| {
                            ui.add_space(theme::SM);
                            navigation_menu(ui, |ui| {
                                ui.set_width(m.sidebar_width);
                                ui.set_max_height(ctx.content_rect().height() * 0.7);
                                sidebar::show(ui, tab, &mut actions);
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
            .exact_size(m.status_bar_height)
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                // Standard: chrome with a top seam. Deeper: on the ground, no seam.
                let bar = ui.max_rect();
                if !deeper {
                    ui.painter()
                        .rect_filled(bar, egui::CornerRadius::ZERO, s.panel);
                    let seam = crate::ui::kit::paint::edge_seam(p, s.border);
                    let w = seam.width;
                    crate::ui::kit::paint::seam_line_h(
                        ui.painter(),
                        bar.x_range(),
                        bar.top() + w * 0.5,
                        seam,
                    );
                }
                if let Some(s) = &clone_status {
                    ui.horizontal_centered(|ui| {
                        ui.add_space(theme::SM);
                        crate::ui::kit::progress::Spinner::new()
                            .size(crate::ui::kit::Size::Xs)
                            .label("the clone")
                            .show(ui);
                        ui.label(RichText::new(s).size(m.meta_size).color(p.accent));
                    });
                } else {
                    statusbar::show(
                        ui,
                        self.tabs.get(self.active),
                        &self.git_version,
                        &self.updater.state,
                        &self.updater.install,
                        &mut self.toasts,
                        &mut actions,
                    );
                }
            });

        if let Some(tab) = self.tabs.get_mut(self.active) {
            if !compact {
                let sidebar_margin =
                    egui::Margin::symmetric(crate::ui::points_i8(m.sidebar_pad), 0);
                egui::Panel::left("sidebar")
                    .resizable(true)
                    .default_size(m.sidebar_width)
                    .min_size(190.0)
                    .max_size(460.0)
                    .frame(
                        egui::Frame::new()
                            .fill(s.panel)
                            .inner_margin(sidebar_margin),
                    )
                    .show(ui, |ui| {
                        if !deeper {
                            let r = ui.max_rect().expand2(egui::vec2(m.sidebar_pad, 0.0));
                            let seam = crate::ui::kit::paint::pane_seam(p, s.border);
                            let w = seam.width;
                            crate::ui::kit::paint::seam_line_v(
                                ui.painter(),
                                r.right() - w * 0.5,
                                r.y_range(),
                                seam,
                            );
                        }
                        sidebar::show(ui, tab, &mut actions);
                    });
            }
            tab.iso_dates = self.settings.iso_dates;
            content_pane(ui, true, |ui| match tab.view {
                View::History => {
                    let handle = history::show(ui, tab, &self.settings, &mut actions);
                    history_split(ui.ctx(), tab, handle)
                }
                View::Changes => {
                    tab.check_message = self.settings.check_message;
                    changes::show(ui, tab, &mut actions)
                        .and_then(|(list, handle)| PaneSplit::beside(list, handle))
                }
                View::Requests => requests::show(ui, tab, self.settings.hosting_api, &mut actions)
                    .and_then(|(list, handle)| PaneSplit::beside(list, handle)),
            });
        } else {
            // Nothing on the welcome screen holds the keyboard as a pane: no focused-pane edge.
            content_pane(ui, false, |ui| {
                crate::ui::art::backdrop(ui, ui.max_rect());
                welcome::show(ui, &self.settings, &mut actions);
                None
            });
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
        // Settings gives keyboard focus back to what had it when it closes, as the palette does.
        crate::ui::kit::a11y::return_focus(
            &ctx,
            "settings",
            self.settings_screen.is_some(),
            egui::LayerId::new(egui::Order::Foreground, egui::Id::new("settings")),
        );
        if let Some(screen) = &mut self.settings_screen {
            let before = Watched::of(&self.settings);
            let active_tab = self.tabs.get(self.active);
            screen.set_picture_note(active_tab.and_then(|tab| {
                crate::avatars::explain_tab(&self.settings, tab)
                    .map(settings_screen::PictureNote::from)
            }));
            let result = settings_screen::show(
                &ctx,
                screen,
                &mut self.settings,
                &settings_screen::SettingsData {
                    tabs: &self.tabs,
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
            // A CLI sign-in that changed since a tab asked for it: the tab asks again.
            let enabled = self.settings.hosting_api;
            for t in &mut self.tabs {
                let stale = t.host.as_ref().is_some_and(|h| {
                    result
                        .sign_in_changed
                        .contains(&h.forge.authority().to_lowercase())
                });
                if stale {
                    t.restart_host(&ctx, enabled);
                }
            }
            if result.close {
                self.folder_picker.cancel();
                self.settings_screen = None;
            }
            self.settings_changed(&ctx, before);
        }
        // Dialogs draw above the settings screen (the Privacy Policy opens from there).
        if let Some(d) = self.dialogs.current_mut() {
            crate::ui::widgets::dismiss_underlying_tooltips(&ctx);
            let consent = is_consent(d);
            let dialog_tab = match d {
                Dialog::Insights { repo }
                | Dialog::FileHistory { repo, .. }
                | Dialog::SearchChanges { repo, .. } => self.tabs.iter().find(|t| t.path == *repo),
                Dialog::Ignore(form) => self.tabs.iter().find(|t| t.path == form.repo),
                Dialog::CleanStale(form) => self.tabs.iter().find(|t| t.path == form.repo),
                _ => self.tabs.get(self.active),
            };
            match dialogs::show(
                &ctx,
                d,
                dialog_tab,
                &mut self.folder_picker,
                self.clone.is_some(),
                &self.updater,
                &self.git_version,
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
        }
        // Run the closed frame too: the palette restores focus to the previous widget here.
        if let Some(a) = self.palette.show(&ctx) {
            actions.push(a);
        }
        if self.cheatsheet.open {
            crate::ui::widgets::dismiss_underlying_tooltips(&ctx);
        }
        self.cheatsheet.show(&ctx);
        for offer in self.toasts.show(&ctx) {
            match offer {
                Offer::Pull(path) => {
                    if let Some(index) = self
                        .tabs
                        .iter()
                        .position(|tab| tab.path == path && tab.running.is_none())
                    {
                        actions.push(Action::SwitchTab(index));
                        actions.push(Action::Pull);
                    }
                }
                Offer::Accounts => {
                    self.open_settings(SettingsScreen::at(settings_screen::Section::Accounts));
                }
                Offer::GitSettings => {
                    self.open_settings(SettingsScreen::at(settings_screen::Section::Git));
                }
                Offer::ReviewUpdate => {
                    if let UpdateState::Available(a) = &self.updater.state {
                        self.dialogs.open(Dialog::Update(a.clone()));
                    }
                }
                Offer::Restart(exe) => actions.push(Action::RestartInto(exe)),
            }
        }
        if let Some(page) = self.gallery {
            crate::ui::kit::gallery::show(&ctx, page, crate::ui::kit::GALLERY_SECTIONS);
        }
        if let Some(soak) = &mut self.glass_soak
            && soak.frame(&ctx)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.apply(&ctx, actions);
        self.handle_screenshot(&ctx);
        crate::ui::kit::a11y::keep_focus_in_tree(&ctx);
    }

    #[expect(
        clippy::print_stdout,
        reason = "tools/glass-check.py reads the glass soak report from standard output"
    )]
    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        crate::ui::kit::glass::destroy(gl);
        // Stop every repository's threads, then wait for them all at once.
        self.tabs.clear();
        crate::worker::wait_for_threads(Duration::from_secs(1));
        if let Some(soak) = &self.glass_soak {
            println!("{}", soak.report());
        }
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.screenshot_input(raw_input);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.screenshot.is_some() || self.glass_soak.is_some() {
            return;
        }
        for tab in &self.tabs {
            if tab.sidebar_restored
                && let Some(key) = tab.sidebar_key()
            {
                // The default needs no entry, so repositories left as they opened do not pile up.
                if tab.sidebar_is_default() {
                    self.settings.repo_sidebar.remove(key);
                } else {
                    self.settings
                        .repo_sidebar
                        .insert(key.to_path_buf(), tab.collapsed.iter().cloned().collect());
                }
            }
        }
        self.settings.open_tabs = self.tabs.iter().map(|t| t.path.clone()).collect();
        eframe::set_value(storage, settings::KEY, &self.settings);
        storage.set_string(
            crate::ui::toast::STORAGE_KEY,
            saved_notifications(&self.toasts),
        );
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::from(
            self.settings
                .theme
                .palette()
                .surfaces(self.settings.style)
                .bg,
        )
        .to_array()
    }
}

/// Where Deeper splits the content pane into two cards.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Split {
    /// One card above the other, meeting at this height (History's details below its table).
    Horizontal(f32),
    /// Side by side, meeting at this x (a list beside what it shows: Changes, requests).
    Vertical(f32),
}

/// A view's answer to [`content_pane`]: where its two Deeper cards meet, and the resize handle
/// of the panel between them (its line shows in the gutter while it is hovered or dragged).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaneSplit {
    pub split: Split,
    pub handle: Option<egui::Id>,
}

impl PaneSplit {
    /// Two cards side by side: `list` is the rectangle the list's panel had this frame
    /// (`InnerResponse::response.rect`), and `handle` the id of the handle that resizes it; the
    /// cards meet at its right edge. `None` in Standard, which paints one flush pane.
    pub fn beside(list: egui::Rect, handle: egui::Id) -> Option<Self> {
        (theme::style() == theme::Style::Deeper).then(|| Self {
            split: Split::Vertical(list.right()),
            handle: Some(handle),
        })
    }
}

/// The central content pane: flush on the pane ground in Standard. In Deeper a floating card over
/// the ground's gutters, or two when the view splits (`content` returns where: History's details
/// below the table, a list beside its detail); the first card (the table or list, which holds
/// the keyboard), or the only one, carries the focused-pane edge when `focus_edge` (a repository's
/// views; the welcome screen has no pane that holds the keyboard).
fn content_pane(
    ui: &mut egui::Ui,
    focus_edge: bool,
    content: impl FnOnce(&mut egui::Ui) -> Option<PaneSplit>,
) {
    let m = theme::metrics();
    let p = palette(ui.ctx());
    let s = theme::surfaces(ui.ctx());
    // Concept 4's body padding: the gutter all round, below the action card too (Standard: none).
    let margin = egui::Margin::same(crate::ui::points_i8(m.pane_gutter));
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.inner_margin(margin))
        .show(ui, |ui| {
            let card = ui.max_rect();
            if theme::style() == theme::Style::Standard {
                crate::ui::kit::paint::pane(ui.painter(), card, p, s.pane, 1.0);
                content(ui);
                return;
            }
            // The cards' shadows and fills go under the content, once the split is known.
            let wide = ui.painter().clone();
            let below = wide.add(egui::Shape::Noop);
            let radius = egui::CornerRadius::same(m.pane_radius);
            // Content stays inside the rounded card.
            let clip = card.shrink(f32::from(radius.nw) * 0.3);
            ui.set_clip_rect(clip.intersect(ui.clip_rect()));
            let gap = m.pane_gutter;
            let split = content(ui).filter(|s| match s.split {
                Split::Horizontal(y) => y - gap > card.top() + gap && y + gap < card.bottom() - gap,
                Split::Vertical(x) => x - gap > card.left() + gap && x + gap < card.right() - gap,
            });
            let cards = match split.map(|s| s.split) {
                Some(Split::Horizontal(y)) => vec![
                    egui::Rect::from_min_max(card.min, egui::pos2(card.right(), y - gap / 2.0)),
                    egui::Rect::from_min_max(egui::pos2(card.left(), y + gap / 2.0), card.max),
                ],
                Some(Split::Vertical(x)) => vec![
                    egui::Rect::from_min_max(card.min, egui::pos2(x - gap / 2.0, card.bottom())),
                    egui::Rect::from_min_max(egui::pos2(x + gap / 2.0, card.top()), card.max),
                ],
                None => vec![card],
            };
            let mut shapes = Vec::new();
            for c in &cards {
                shapes.extend(crate::ui::kit::paint::surface_shapes(
                    &wide,
                    *c,
                    radius,
                    p,
                    theme::Style::Deeper,
                    theme::Elevation::Pane,
                    theme::Fill::flat(s.pane),
                    s.pane_edge,
                ));
            }
            wide.set(below, egui::Shape::Vec(shapes));
            if let Some(split) = split {
                split_gutter(ui, &wide, &cards, radius, split);
            }
            // The card that holds the keyboard (the table or list) gets the focus edge and a lit
            // top.
            if focus_edge {
                crate::ui::kit::paint::pane_focus(&wide, cards[0], radius, p.focus, 1.0);
            }
        });
}

/// Where History's details meet the table, when they sit below it as their own card (Deeper):
/// at `handle`, the split handle `history::show` returned, which is centred on the details' top
/// edge.
fn history_split(
    ctx: &egui::Context,
    tab: &RepoTab,
    handle: Option<egui::Id>,
) -> Option<PaneSplit> {
    let below = theme::style() == theme::Style::Deeper
        && !tab.history_layout.compact
        && !tab.history_layout.expanded
        && tab.snap.as_ref().is_some_and(|s| !s.commits.is_empty());
    let handle = handle.filter(|_| below)?;
    ctx.read_response(handle).map(|r| PaneSplit {
        split: Split::Horizontal(r.rect.center().y),
        handle: Some(handle),
    })
}

/// The gutter between two cards, painted over the content that runs under it: the ground, the
/// first card's shadow falling into it (both cards' when side by side), the cards' inner corners
/// and edges, the second card's top highlight, and the split handle's line while it is hovered or
/// dragged.
fn split_gutter(
    ui: &egui::Ui,
    painter: &egui::Painter,
    cards: &[egui::Rect],
    radius: egui::CornerRadius,
    split: PaneSplit,
) {
    let (Some(first), Some(second)) = (cards.first(), cards.get(1)) else {
        return;
    };
    let p = palette(ui.ctx());
    let s = theme::surfaces(ui.ctx());
    let beside = matches!(split.split, Split::Vertical(_));
    let band = if beside {
        egui::Rect::from_x_y_ranges(first.right()..=second.left(), first.y_range())
    } else {
        egui::Rect::from_x_y_ranges(first.x_range(), first.bottom()..=second.top())
    };
    painter.rect_filled(band, egui::CornerRadius::ZERO, s.bg);
    let material = p.material(theme::Style::Deeper, theme::Elevation::Pane);
    let shadowed = if beside { cards } else { &cards[..1] };
    for card in shadowed {
        painter
            .with_clip_rect(band)
            .extend(crate::ui::kit::paint::shadow_shapes(
                *card, radius, p, &material,
            ));
    }
    let r = f32::from(radius.nw);
    for (corner, centre, from) in inner_corners(*first, *second, r, beside) {
        corner_mask(painter, corner, centre, r, from, s.bg);
    }
    if let Some(edge) = s.pane_edge {
        crate::ui::kit::paint::rim(painter, *first, radius, edge);
        crate::ui::kit::paint::rim(painter, *second, radius, edge);
    }
    crate::ui::kit::paint::top_highlight(painter, *second, radius, material.highlight);
    if split.handle.is_some_and(|handle| {
        ui.ctx()
            .read_response(handle)
            .is_some_and(|r| r.hovered() || r.dragged() || r.has_focus())
    }) {
        let stroke = egui::Stroke::new(2.0, theme::Palette::tint(p.focus, 150));
        match split.split {
            Split::Horizontal(y) => painter.hline(band.x_range().shrink(r), y, stroke),
            Split::Vertical(x) => painter.vline(x, band.y_range().shrink(r), stroke),
        };
    }
}

/// The four rounded corners that face each other across the gutter between `first` and `second`
/// (one above the other, or `beside`), as (square corner, arc centre, arc start angle) for
/// [`corner_mask`]. Angles turn clockwise on screen from +x: a quarter turn points down.
fn inner_corners(
    first: egui::Rect,
    second: egui::Rect,
    r: f32,
    beside: bool,
) -> [(egui::Pos2, egui::Pos2, f32); 4] {
    let quarter = std::f32::consts::FRAC_PI_2;
    let corner = |at: egui::Pos2, dx: f32, dy: f32, from: f32| (at, at + egui::vec2(dx, dy), from);
    if beside {
        [
            corner(first.right_top(), -r, r, 3.0 * quarter),
            corner(first.right_bottom(), -r, -r, 0.0),
            corner(second.left_top(), r, r, 2.0 * quarter),
            corner(second.left_bottom(), r, -r, quarter),
        ]
    } else {
        [
            corner(first.left_bottom(), r, -r, quarter),
            corner(first.right_bottom(), -r, -r, 0.0),
            corner(second.left_top(), r, r, 2.0 * quarter),
            corner(second.right_top(), -r, r, 3.0 * quarter),
        ]
    }
}

/// Paints `color` over the part of a square corner outside its rounded arc: a fan from `corner`
/// over the quarter arc of radius `r` around `centre` that starts at angle `from`.
fn corner_mask(
    painter: &egui::Painter,
    corner: egui::Pos2,
    centre: egui::Pos2,
    r: f32,
    from: f32,
    color: egui::Color32,
) {
    const STEPS: u16 = 8;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(corner, color);
    for i in 0..=STEPS {
        let a = from + std::f32::consts::FRAC_PI_2 * f32::from(i) / f32::from(STEPS);
        mesh.colored_vertex(centre + r * egui::vec2(a.cos(), a.sin()), color);
    }
    for i in 1..=u32::from(STEPS) {
        mesh.add_triangle(0, i, i + 1);
    }
    painter.add(mesh);
}

/// The compact window's navigation: a button opening the sidebar in a popover. Its open state is
/// kept here, so a context menu opened from a row inside does not close it.
fn navigation_menu(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) -> egui::Response {
    let button = crate::ui::kit::IconButton::new(icon::LIST, "Branches, remotes, tags and stashes")
        .id_salt("navigation-menu")
        .show(ui);
    let id = button.id.with("navigation-open");
    let mut open = ui.data(|data| data.get_temp::<bool>(id)).unwrap_or(false);
    if button.clicked() {
        open = !open;
    }
    let shown = crate::ui::kit::overlay::Popover::new("navigation", &button)
        .open(&mut open)
        .show(content)
        .is_some();
    // Context menus use egui's single popup slot; the navigation must keep their rows alive.
    if shown && egui::Popup::is_any_open(ui.ctx()) {
        open = true;
    }
    ui.data_mut(|data| data.insert_temp(id, open));
    button
}

/// An app with no window, threads or saved state, for tests of the app's own logic.
#[cfg(test)]
pub(super) fn test_app() -> CanopyApp {
    CanopyApp {
        settings: Settings::default(),
        tabs: Vec::new(),
        active: 0,
        dialogs: DialogStack::default(),
        settings_screen: None,
        folder_picker: crate::ui::folder_picker::FolderPicker::default(),
        palette: CommandPalette::default(),
        cheatsheet: Cheatsheet::default(),
        toasts: Toasts::default(),
        screenshot: None,
        initial_view: None,
        clone: None,
        opener: None,
        pending_opens: 0,
        config_reads: Vec::new(),
        git_version: String::new(),
        updater: Updater::default(),
        identity_worker: None,
        global_identity: None,
        global_identity_error: None,
        gitlab_hosts: Vec::new(),
        update_check_due: false,
        askpass: None,
        trust_seen: 0,
        applied: None,
        gallery: None,
        glass_soak: None,
    }
}

/// A tab on no real repository (its worker answers nothing) showing `snap`.
#[cfg(test)]
pub(super) fn test_tab(snap: canopy_core::RepoSnapshot) -> RepoTab {
    let (worker, _, _) = crate::worker::Worker::detached();
    let mut tab = RepoTab::with_worker(
        PathBuf::from("fixture"),
        100,
        settings::DiffPrefs::default(),
        worker,
    );
    tab.snap = Some(std::sync::Arc::new(snap));
    tab
}

/// The notification center as it is written to disk: with no user name or password left in a
/// URL in a title or in Git's error text.
fn saved_notifications(toasts: &Toasts) -> String {
    canopy_core::cli::redact_url(&toasts.to_json())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the center keeps on disk carries no password, and keeps the rest.
    #[test]
    fn the_saved_center_has_no_passwords() {
        let ctx = egui::Context::default();
        let mut toasts = Toasts::default();
        toasts.push_for(
            &ctx,
            ToastKind::Error,
            Topic::Error,
            "Fetch from https://ada:hunter2@example.com/r.git failed",
            "fatal: unable to access 'https://ada:hunter2@example.com/r.git/': timed out",
            Some(std::path::Path::new("/work/canopy")),
        );
        let saved = saved_notifications(&toasts);
        assert!(!saved.contains("hunter2"), "{saved}");
        assert!(saved.contains("example.com/r.git"), "{saved}");
        let mut again = Toasts::default();
        again.restore_json(&saved);
        assert!(format!("{again:?}").contains("timed out"));
    }

    /// A11Y-13: the window has an accessible name, with the repository it shows.
    #[test]
    fn the_window_is_named_for_screen_readers() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let root = |repository: Option<&str>| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                name_window(ui.ctx(), repository);
            });
            out.textures_delta.clear();
            let update = out.platform_output.accesskit_update.expect("a tree");
            let root = update.tree.expect("its root").root;
            update
                .nodes
                .into_iter()
                .find(|(id, _)| *id == root)
                .and_then(|(_, n)| n.label().map(str::to_owned))
        };
        assert_eq!(root(None).as_deref(), Some("Canopy"));
        assert_eq!(root(Some("showcase")).as_deref(), Some("showcase — Canopy"));
    }

    /// review-views-9: Settings is compared by the few values whose change needs work, not by a
    /// copy of every setting each frame; a changed GitLab host list still reaches the tabs.
    #[test]
    fn a_settings_change_is_seen_without_copying_the_settings() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        let before = Watched::of(&app.settings);
        app.settings.split_diff = !app.settings.split_diff;
        "git.example.com".clone_into(&mut app.settings.gitlab_hosts);
        app.tabs
            .push(test_tab(canopy_core::RepoSnapshot::default()));
        app.settings_changed(&ctx, before);
        assert_eq!(app.gitlab_hosts, ["git.example.com"]);
        assert_eq!(app.tabs[0].split_diff, app.settings.split_diff);
    }

    /// Choosing the request list's density in Settings reaches the open tabs.
    #[test]
    fn the_request_list_density_chosen_in_settings_reaches_the_tabs() {
        use crate::request_view::RequestDensity;
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.tabs
            .push(test_tab(canopy_core::RepoSnapshot::default()));
        assert_eq!(app.tabs[0].request_density, RequestDensity::Comfortable);
        let before = Watched::of(&app.settings);
        app.settings.request_density = RequestDensity::Dense;
        app.settings_changed(&ctx, before);
        assert_eq!(app.tabs[0].request_density, RequestDensity::Dense);
    }

    /// Each mask fans over the part of a card's square corner outside its rounded arc: the arc's
    /// middle points from its centre straight at the corner, for cards above each other and side
    /// by side.
    #[test]
    fn the_gutter_masks_face_the_corners_they_round() {
        let first = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(100.0, 200.0));
        for (second, beside) in [
            (
                egui::Rect::from_min_max(egui::pos2(110.0, 0.0), egui::pos2(300.0, 200.0)),
                true,
            ),
            (
                egui::Rect::from_min_max(egui::pos2(0.0, 210.0), egui::pos2(100.0, 400.0)),
                false,
            ),
        ] {
            let r = 8.0;
            for (corner, centre, from) in inner_corners(first, second, r, beside) {
                let middle = from + std::f32::consts::FRAC_PI_4;
                let towards = (corner - centre).normalized();
                let arc = egui::vec2(middle.cos(), middle.sin());
                assert!(
                    (towards - arc).length() < 1e-4,
                    "beside {beside}: corner {corner:?}, arc middle {arc:?}"
                );
                assert!((corner - centre).length() - r * std::f32::consts::SQRT_2 < 1e-3);
                let card = if first.expand(0.5).contains(corner) {
                    first
                } else {
                    second
                };
                assert!(card.contains(centre), "the arc centre lies inside its card");
            }
        }
    }

    /// Deeper splits the content pane into two cards side by side where a view's list ends: the
    /// ground shows in a gutter of `pane_gutter` centred on the split, top to bottom.
    #[test]
    fn a_side_by_side_split_paints_a_full_height_gutter() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(
            &ctx,
            ThemeId::Midnight,
            theme::Appearance {
                style: theme::Style::Deeper,
                ..theme::Appearance::STILL
            },
        );
        let at = 500.0;
        let mut card = egui::Rect::NOTHING;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 800.0),
                )),
                ..Default::default()
            },
            |ui| {
                content_pane(ui, true, |ui| {
                    card = ui.max_rect();
                    Some(PaneSplit {
                        split: Split::Vertical(at),
                        handle: None,
                    })
                });
            },
        );
        output.textures_delta.clear();
        let ground = theme::surfaces(&ctx).bg;
        let gap = theme::metrics().pane_gutter;
        let gutter = output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(r)
                    if r.fill == ground && (r.rect.center().x - at).abs() < 0.5 =>
                {
                    Some(r.rect)
                }
                _ => None,
            });
        let gutter = gutter.expect("a gutter on the ground at the split");
        assert!((gutter.width() - gap).abs() < 0.5, "{gutter:?}");
        assert!(
            gutter.top() <= card.top() + 1.0 && gutter.bottom() >= card.bottom() - 1.0,
            "{gutter:?} spans the pane {card:?}"
        );
    }

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
                        if let Some(Some(rect)) = crate::ui::kit::menu::context(&response, |m| {
                            let item = m.item("", "Rename branch").show(m.ui);
                            (item.rect.is_positive()).then_some((item.rect, item.layer_id))
                        }) {
                            nested = Some((rect.0.center(), rect.1));
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
            modifiers: egui::Modifiers::default(),
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
                update_window_minimum(ui.ctx());
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
                    update_window_minimum(ui.ctx());
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
                update_window_minimum(ui.ctx());
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
