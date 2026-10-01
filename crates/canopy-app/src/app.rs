//! The application: tabs, global shortcuts, dialogs, the command palette and toasts.

use std::path::PathBuf;

use crossbeam_channel::{Receiver, unbounded};
use egui::{Key, KeyboardShortcut, Modifiers, RichText, Stroke};
use egui_phosphor::regular as icon;

use canopy_core::{Oid, Repo};

use crate::settings::{self, Settings};
use crate::tab::{RepoTab, View};
use crate::ui::palette::{CommandPalette, Entry};
use crate::ui::theme::{self, palette};
use crate::ui::toast::{ToastKind, Toasts};
use crate::views::dialogs::{self, Dialog, Outcome};
use crate::views::{changes, history, sidebar, statusbar, toolbar, welcome};
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
    Pull,
    Commit,
    PrefillAmend,
    ToggleTheme,
    Palette,
    Copy(String),
    OpenPath(PathBuf),
    OpenTerminal(PathBuf),
}

/// Options from the command line.
#[derive(Debug, Default, Clone)]
pub struct Launch {
    pub paths: Vec<PathBuf>,
    pub light: bool,
    pub screenshot: Option<PathBuf>,
    pub view: Option<View>,
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

/// The Canopy application state.
pub struct CanopyApp {
    settings: Settings,
    tabs: Vec<RepoTab>,
    active: usize,
    dialog: Option<Dialog>,
    palette: CommandPalette,
    toasts: Toasts,
    screenshot: Option<Screenshot>,
    initial_view: Option<View>,
    clone: Option<(String, Receiver<CloneMsg>, String)>,
    git_version: String,
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
        if launch.light {
            settings.dark = false;
        }
        crate::ui::fonts::install(&cc.egui_ctx);
        theme::apply(&cc.egui_ctx, settings.dark);
        cc.egui_ctx.set_zoom_factor(settings.zoom);

        let git_version = canopy_core::cli::version().unwrap_or_else(|_| "git not found".into());
        let mut app = Self {
            settings,
            tabs: Vec::new(),
            active: 0,
            dialog: None,
            palette: CommandPalette::default(),
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
        };
        let restore = if launch.paths.is_empty() && app.screenshot.is_none() {
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
        if git_version_missing(&app.git_version) {
            app.toasts.push(&cc.egui_ctx, ToastKind::Error, "git was not found", "Canopy runs the git executable for changes and network operations. Install git and make sure it is on PATH.");
        }
        app
    }

    fn tab(&mut self) -> Option<&mut RepoTab> {
        self.tabs.get_mut(self.active)
    }

    fn open_repo(&mut self, ctx: &egui::Context, path: PathBuf) {
        let path = if let Some(rest) = path.to_str().and_then(|s| s.strip_prefix("~/")) {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default()
                .join(rest)
        } else {
            path
        };
        match Repo::open(&path) {
            Ok(repo) => {
                let root = repo.workdir().to_path_buf();
                let root = root.canonicalize().unwrap_or(root);
                if let Some(i) = self.tabs.iter().position(|t| t.path == root) {
                    self.active = i;
                } else {
                    let mut tab = RepoTab::new(root.clone(), ctx, self.settings.commit_page);
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

    fn shortcuts(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        if self.dialog.is_some() || self.palette.open {
            return;
        }
        let cmd = Modifiers::COMMAND;
        let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        let take = |m: Modifiers, k: Key| {
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(m, k)))
        };
        if take(cmd, Key::K) || take(cmd_shift, Key::P) {
            actions.push(Action::Palette);
        }
        if take(cmd, Key::O) {
            actions.push(Action::OpenDialog(Dialog::open()));
        }
        if take(cmd_shift, Key::N) {
            actions.push(Action::OpenDialog(Dialog::clone_default()));
        }
        if take(cmd, Key::Comma) {
            actions.push(Action::OpenDialog(Dialog::Settings));
        }
        if self.tabs.is_empty() {
            return;
        }
        if take(cmd, Key::W) {
            actions.push(Action::CloseTab(self.active));
        }
        if take(Modifiers::CTRL | Modifiers::SHIFT, Key::Tab) {
            actions.push(Action::SwitchTab(
                (self.active + self.tabs.len() - 1) % self.tabs.len(),
            ));
        } else if take(Modifiers::CTRL, Key::Tab) {
            actions.push(Action::SwitchTab((self.active + 1) % self.tabs.len()));
        }
        if take(cmd, Key::Num1) {
            actions.push(Action::SetView(View::History));
        }
        if take(cmd, Key::Num2) {
            actions.push(Action::SetView(View::Changes));
        }
        if take(Modifiers::NONE, Key::F5) || take(cmd, Key::R) {
            actions.push(Action::Refresh);
        }
        if take(cmd_shift, Key::F) {
            actions.push(Action::Op(Op::Fetch));
        }
        if take(cmd_shift, Key::L) {
            actions.push(Action::Pull);
        }
        if take(cmd_shift, Key::K) {
            actions.push(Action::Op(Op::Push { force: false }));
        }
        if take(cmd_shift, Key::S) {
            actions.push(Action::OpenDialog(Dialog::Stash {
                message: String::new(),
                untracked: true,
            }));
        }
        if take(cmd, Key::B) {
            actions.push(Action::OpenDialog(Dialog::new_branch(None)));
        }
        if take(cmd, Key::Enter)
            && self
                .tabs
                .get(self.active)
                .is_some_and(|t| t.view == View::Changes)
        {
            actions.push(Action::Commit);
        }
        if take(cmd, Key::F) {
            actions.push(Action::SetView(View::History));
            ctx.memory_mut(|m| m.request_focus(egui::Id::new("history-filter")));
        }
    }

    fn palette_entries(&self) -> Vec<(Entry, Action)> {
        let e = |icon: &'static str, label: &str, detail: &str, shortcut: &'static str| Entry {
            icon,
            label: label.to_owned(),
            detail: detail.to_owned(),
            shortcut,
        };
        let mut v = vec![
            (
                e(
                    icon::FOLDER_OPEN,
                    "Open repository…",
                    "Repository",
                    "Ctrl O",
                ),
                Action::OpenDialog(Dialog::open()),
            ),
            (
                e(
                    icon::CLOUD_ARROW_DOWN,
                    "Clone repository…",
                    "Repository",
                    "Ctrl Shift N",
                ),
                Action::OpenDialog(Dialog::clone_default()),
            ),
            (
                e(icon::PLANT, "Init new repository…", "Repository", ""),
                Action::OpenDialog(Dialog::Init {
                    path: String::new(),
                }),
            ),
            (
                e(
                    if self.settings.dark {
                        icon::SUN
                    } else {
                        icon::MOON
                    },
                    "Toggle theme",
                    "View",
                    "",
                ),
                Action::ToggleTheme,
            ),
            (
                e(icon::GEAR_SIX, "Settings", "View", "Ctrl ,"),
                Action::OpenDialog(Dialog::Settings),
            ),
            (
                e(icon::INFO, "About Canopy", "Help", ""),
                Action::OpenDialog(Dialog::About),
            ),
        ];
        if let Some(tab) = self.tabs.get(self.active) {
            v.extend([
                (e(icon::GRAPH, "Show history", "View", "Ctrl 1"), Action::SetView(View::History)),
                (e(icon::PENCIL_SIMPLE_LINE, "Show changes", "View", "Ctrl 2"), Action::SetView(View::Changes)),
                (e(icon::ARROWS_CLOCKWISE, "Refresh", "Repository", "F5"), Action::Refresh),
                (e(icon::ARROWS_CLOCKWISE, "Fetch all remotes", "Remote", "Ctrl Shift F"), Action::Op(Op::Fetch)),
                (e(icon::ARROW_FAT_LINE_DOWN, "Pull", "Remote", "Ctrl Shift L"), Action::Pull),
                (e(icon::ARROW_FAT_LINE_UP, "Push", "Remote", "Ctrl Shift K"), Action::Op(Op::Push { force: false })),
                (e(icon::WARNING, "Force push (with lease)…", "Remote", ""), Action::Confirm { title: "Force push".into(), body: "Overwrite the remote branch with your local one? --force-with-lease refuses if someone else pushed in the meantime.".into(), confirm: "Force push".into(), danger: true, op: Op::Push { force: true } }),
                (e(icon::CLOUD, "Add remote…", "Remote", ""), Action::OpenDialog(Dialog::AddRemote { name: "origin".into(), url: String::new() })),
                (e(icon::GIT_BRANCH, "New branch…", "Branch", "Ctrl B"), Action::OpenDialog(Dialog::new_branch(None))),
                (e(icon::GIT_MERGE, "Merge…", "Branch", ""), Action::OpenDialog(Dialog::Merge { target: String::new(), rebase: false })),
                (e(icon::GIT_PULL_REQUEST, "Rebase…", "Branch", ""), Action::OpenDialog(Dialog::Merge { target: String::new(), rebase: true })),
                (e(icon::ARCHIVE, "Stash changes…", "Stash", "Ctrl Shift S"), Action::OpenDialog(Dialog::Stash { message: String::new(), untracked: true })),
                (e(icon::TRAY_ARROW_UP, "Pop latest stash", "Stash", ""), Action::Op(Op::StashApply { index: 0, pop: true })),
                (e(icon::PLUS, "Stage all changes", "Changes", ""), Action::Op(Op::StageAll)),
                (e(icon::MINUS, "Unstage all changes", "Changes", ""), Action::Op(Op::UnstageAll)),
                (e(icon::TERMINAL_WINDOW, "Open terminal here", "Repository", ""), Action::OpenTerminal(tab.path.clone())),
                (e(icon::FOLDER_OPEN, "Show in file manager", "Repository", ""), Action::OpenPath(tab.path.clone())),
                (e(icon::X, "Close repository", "Repository", "Ctrl W"), Action::CloseTab(self.active)),
            ]);
            if let Some(s) = &tab.snap {
                if let Some(op) = s.state.command().map(|_| s.state) {
                    v.push((
                        e(
                            icon::PLAY,
                            &format!("Continue {}", op.label().to_lowercase()),
                            "Repository",
                            "",
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
                            "",
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
                            "",
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
                            "",
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
                            "",
                        ),
                        Action::Op(Op::Merge(b.name.clone())),
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
                    "",
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
                    "",
                ),
                Action::OpenRepo(r.clone()),
            ));
        }
        v
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
                } => {
                    self.dialog = Some(Dialog::Confirm {
                        title,
                        body,
                        confirm,
                        danger,
                        op,
                    })
                }
                Action::OpenDialog(d) => self.dialog = Some(d),
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
                    }
                }
                Action::LoadMore => {
                    let page = self.settings.commit_page;
                    if let Some(t) = self.tab() {
                        t.commit_limit += page;
                        t.refresh();
                    }
                }
                Action::Pull => {
                    let rebase = self.settings.pull_rebase;
                    if let Some(t) = self.tab() {
                        t.op(Op::Pull { rebase });
                    }
                }
                Action::Commit => {
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
                            });
                        }
                    }
                }
                Action::PrefillAmend => {
                    if let Some(t) = self.tab() {
                        t.worker.send(Job::HeadMessage);
                    }
                }
                Action::ToggleTheme => {
                    self.settings.dark = !self.settings.dark;
                    theme::apply(ctx, self.settings.dark);
                }
                Action::Palette => self.palette.open(),
                Action::Copy(text) => {
                    ctx.copy_text(text.clone());
                    self.toasts.push(ctx, ToastKind::Info, "Copied", text);
                }
                Action::OpenPath(p) => open_path(&p),
                Action::OpenTerminal(p) => open_terminal(&p),
            }
        }
    }

    fn start_clone(&mut self, ctx: &egui::Context, url: String, dest: PathBuf) {
        let (tx, rx) = unbounded();
        let thread_ctx = ctx.clone();
        let thread_url = url.clone();
        let _ = std::thread::Builder::new()
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
        self.clone = Some((url, rx, "Starting…".into()));
    }

    fn poll(&mut self, ctx: &egui::Context) {
        for tab in &mut self.tabs {
            for n in tab.poll() {
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
        let mut finished = None;
        if let Some((_, rx, status)) = &mut self.clone {
            while let Ok(m) = rx.try_recv() {
                match m {
                    CloneMsg::Progress(l) => *status = l,
                    CloneMsg::Done(r) => finished = Some(r),
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
    }

    fn settings_dialog(&mut self, ctx: &egui::Context) -> bool {
        let p = palette(ctx);
        let mut close = false;
        let mut theme_changed = false;
        let modal = egui::Modal::new(egui::Id::new("settings"))
            .frame(
                egui::Frame::new()
                    .fill(p.elevated)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(egui::CornerRadius::same(theme::RADIUS_LG))
                    .inner_margin(egui::Margin::same(22)),
            )
            .show(ctx, |ui| {
                ui.set_width(420.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(icon::GEAR_SIX).size(20.0).color(p.accent));
                    ui.label(
                        RichText::new("Settings").font(egui::FontId::new(17.0, theme::semibold())),
                    );
                });
                ui.add_space(14.0);
                egui::Grid::new("settings-grid")
                    .num_columns(2)
                    .spacing([24.0, 12.0])
                    .show(ui, |ui| {
                        ui.label("Theme");
                        ui.horizontal(|ui| {
                            theme_changed |= ui
                                .selectable_value(
                                    &mut self.settings.dark,
                                    true,
                                    format!("{}  Midnight", icon::MOON),
                                )
                                .changed();
                            theme_changed |= ui
                                .selectable_value(
                                    &mut self.settings.dark,
                                    false,
                                    format!("{}  Daylight", icon::SUN),
                                )
                                .changed();
                        });
                        ui.end_row();
                        ui.label("Zoom");
                        ui.add(
                            egui::Slider::new(&mut self.settings.zoom, 0.8..=1.6)
                                .step_by(0.05)
                                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                        );
                        ui.end_row();
                        ui.label("Pull strategy");
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut self.settings.pull_rebase, false, "Merge");
                            ui.selectable_value(&mut self.settings.pull_rebase, true, "Rebase");
                        });
                        ui.end_row();
                        ui.label("Commits per page");
                        ui.add(
                            egui::DragValue::new(&mut self.settings.commit_page)
                                .range(200..=50_000)
                                .speed(50),
                        );
                        ui.end_row();
                    });
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "The pull strategy applies only when git's pull.rebase is not set.",
                    )
                    .size(11.5)
                    .color(p.text_faint),
                );
                ui.add_space(12.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_sized([90.0, 32.0], egui::Button::new("Done"))
                        .clicked()
                    {
                        close = true;
                    }
                });
            });
        if theme_changed {
            theme::apply(ctx, self.settings.dark);
        }
        if (ctx.zoom_factor() - self.settings.zoom).abs() > f32::EPSILON {
            ctx.set_zoom_factor(self.settings.zoom);
        }
        close || modal.should_close()
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
            t.snap.is_some() && t.running.is_none() && (t.diff.is_some() || t.file.is_none())
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

fn open_path(path: &std::path::Path) {
    #[cfg(target_os = "linux")]
    let r = std::process::Command::new("xdg-open").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(path).spawn();
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    let r: std::io::Result<std::process::Child> =
        Err(std::io::Error::other("unsupported platform"));
    if let Err(e) = r {
        tracing::warn!("could not open {}: {e}", path.display());
    }
}

fn open_terminal(path: &std::path::Path) {
    #[cfg(target_os = "linux")]
    {
        let candidates: Vec<String> = std::env::var("TERMINAL")
            .ok()
            .into_iter()
            .chain(
                [
                    "x-terminal-emulator",
                    "kgx",
                    "gnome-terminal",
                    "konsole",
                    "alacritty",
                    "kitty",
                    "foot",
                    "wezterm",
                    "xterm",
                ]
                .map(String::from),
            )
            .collect();
        for term in candidates {
            if std::process::Command::new(&term)
                .current_dir(path)
                .spawn()
                .is_ok()
            {
                return;
            }
        }
        tracing::warn!("no terminal emulator found; set $TERMINAL");
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .args(["-a", "Terminal"])
            .arg(path)
            .spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "cmd"])
            .current_dir(path)
            .spawn();
    }
}

impl eframe::App for CanopyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        self.poll(&ctx);
        let mut actions = Vec::new();
        self.shortcuts(&ctx, &mut actions);

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
                toolbar::tabs(ui, &self.tabs, self.active, &mut actions)
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
                    statusbar::show(ui, self.tabs.get(self.active), &self.git_version);
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
                    View::History => history::show(ui, tab, &mut actions),
                    View::Changes => changes::show(ui, tab, &mut actions),
                });
        } else {
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(p.bg))
                .show(ui, |ui| welcome::show(ui, &self.settings, &mut actions));
        }

        // Overlays.
        if matches!(self.dialog, Some(Dialog::Settings)) {
            if self.settings_dialog(&ctx) {
                self.dialog = None;
            }
        } else if let Some(d) = &mut self.dialog {
            match dialogs::show(&ctx, d, self.tabs.get(self.active)) {
                Outcome::Keep => {}
                Outcome::Close => self.dialog = None,
                Outcome::Run(a) => {
                    self.dialog = None;
                    actions.extend(a);
                }
            }
        }
        if self.palette.open {
            let entries = self.palette_entries();
            let list: Vec<Entry> = entries.iter().map(|(e, _)| e.clone()).collect();
            if let Some(i) = self.palette.show(&ctx, &list)
                && let Some((_, a)) = entries.into_iter().nth(i)
            {
                actions.push(a);
            }
        }
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
        let c = if self.settings.dark {
            theme::DARK.bg
        } else {
            theme::LIGHT.bg
        };
        egui::Rgba::from(c).to_array()
    }
}
