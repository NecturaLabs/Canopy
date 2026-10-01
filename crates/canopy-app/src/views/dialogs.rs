//! Modal dialogs. Each returns the actions to run when the user confirms.

use egui::{FontId, Id, Key, RichText, Ui};
use egui_phosphor::regular as icon;

use canopy_core::forge::{self, ForgeKind};
use canopy_core::{Oid, short_id};
use canopy_hosting::NewRequest;

use crate::app::Action;
use crate::hosting::HostJob;
use crate::settings::{CloneProtocol, Settings};
use crate::tab::RepoTab;
use crate::ui::theme::{self, palette};
use crate::ui::{art, widgets};
use crate::worker::Op;

/// Where a remote URL comes from in the clone and add-remote dialogs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlSource {
    GitHub,
    GitLab,
    Url,
}

/// Builds a remote URL from `owner/repo` (GitHub, GitLab) or takes it verbatim.
#[derive(Debug, Clone)]
pub struct UrlBuilder {
    pub source: UrlSource,
    pub url: String,
    pub project: String,
    /// GitLab host; empty means gitlab.com.
    pub host: String,
    pub ssh: bool,
}

impl UrlBuilder {
    fn new(settings: &Settings) -> Self {
        Self {
            source: UrlSource::GitHub,
            url: String::new(),
            project: String::new(),
            host: String::new(),
            ssh: settings.clone_protocol == CloneProtocol::Ssh,
        }
    }

    /// The URL to use, if complete.
    pub fn resolve(&self) -> Option<String> {
        let kind = match self.source {
            UrlSource::Url => {
                let u = self.url.trim();
                return (!u.is_empty()).then(|| u.to_owned());
            }
            UrlSource::GitHub => ForgeKind::GitHub,
            UrlSource::GitLab => ForgeKind::GitLab,
        };
        let host = (kind == ForgeKind::GitLab)
            .then(|| self.host.trim())
            .filter(|h| !h.is_empty());
        forge::clone_url(kind, host, self.project.trim(), self.ssh)
    }

    fn ui(&mut self, ui: &mut Ui, focus: bool) {
        let p = palette(ui.ctx());
        ui.horizontal(|ui| {
            for (s, glyph, label) in [
                (UrlSource::GitHub, icon::GITHUB_LOGO, "GitHub"),
                (UrlSource::GitLab, icon::GITLAB_LOGO_SIMPLE, "GitLab"),
                (UrlSource::Url, icon::LINK, "URL"),
            ] {
                ui.selectable_value(&mut self.source, s, format!("{glyph}  {label}"));
            }
        });
        ui.add_space(8.0);
        match self.source {
            UrlSource::Url => {
                field(
                    ui,
                    "Repository URL",
                    &mut self.url,
                    "https://host/owner/repo.git or git@host:owner/repo.git",
                    focus,
                );
            }
            UrlSource::GitHub | UrlSource::GitLab => {
                if self.source == UrlSource::GitLab {
                    field(
                        ui,
                        "Host (empty for gitlab.com)",
                        &mut self.host,
                        "gitlab.example.com",
                        false,
                    );
                }
                let hint = if self.source == UrlSource::GitHub {
                    "owner/repository"
                } else {
                    "group/subgroup/project"
                };
                field(ui, "Project", &mut self.project, hint, focus);
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.ssh, false, "HTTPS");
                    ui.selectable_value(&mut self.ssh, true, "SSH");
                });
                ui.add_space(4.0);
                match self.resolve() {
                    Some(u) => {
                        ui.label(RichText::new(u).monospace().size(11.5).color(p.text_faint))
                    }
                    None if !self.project.trim().is_empty() => ui.label(
                        RichText::new("Use the form owner/repository")
                            .size(12.0)
                            .color(p.danger),
                    ),
                    None => ui.label(""),
                };
                ui.add_space(4.0);
            }
        }
    }
}

/// Every dialog Canopy shows.
#[derive(Debug, Clone)]
pub enum Dialog {
    Open {
        path: String,
    },
    Clone {
        builder: UrlBuilder,
        parent: String,
    },
    Init {
        path: String,
    },
    NewBranch {
        name: String,
        start: Option<String>,
        checkout: bool,
    },
    Rename {
        old: String,
        new: String,
    },
    NewTag {
        name: String,
        target: Oid,
        message: String,
    },
    Stash {
        message: String,
        untracked: bool,
    },
    Merge {
        target: String,
        rebase: bool,
        mode: canopy_core::MergeMode,
    },
    AddRemote {
        name: String,
        builder: UrlBuilder,
        /// Push the current branch after adding (publishing a local-only repository).
        publish: bool,
    },
    NewRequest {
        kind: ForgeKind,
        request: NewRequest,
        /// Branches on the remote, for the target picker.
        branches: Vec<String>,
    },
    Confirm {
        title: String,
        body: String,
        confirm: String,
        danger: bool,
        op: Op,
    },
    About,
    /// What's new in an available update.
    Update(Box<crate::updater::Available>),
    /// What's new in this version (after an upgrade, or from the palette).
    WhatsNew {
        version: String,
        notes: String,
    },
    /// Canopy crashed last time; offer the local report.
    Crash(crate::logging::CrashReport),
}

/// What a dialog wants after this frame.
pub enum Outcome {
    Keep,
    Close,
    Run(Vec<Action>),
}

fn home() -> String {
    crate::settings::home_dir().display().to_string()
}

impl Dialog {
    /// The open-repository dialog, prefilled with the home directory.
    pub fn open() -> Self {
        Dialog::Open { path: home() }
    }

    /// The clone dialog with the default clone folder as the destination.
    pub fn clone_default(settings: &Settings) -> Self {
        Dialog::Clone {
            builder: UrlBuilder::new(settings),
            parent: settings.clone_parent(),
        }
    }

    /// The add-remote dialog.
    pub fn add_remote(settings: &Settings) -> Self {
        Dialog::AddRemote {
            name: "origin".into(),
            builder: UrlBuilder::new(settings),
            publish: false,
        }
    }

    /// Turns an add-remote dialog into "Publish": add, then push the current branch.
    pub fn publishing(mut self) -> Self {
        if let Dialog::AddRemote { publish, .. } = &mut self {
            *publish = true;
        }
        self
    }

    /// New branch starting at `start` (HEAD when `None`).
    pub fn new_branch(start: Option<String>) -> Self {
        Dialog::NewBranch {
            name: String::new(),
            start,
            checkout: true,
        }
    }
}

fn valid_ref_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !name.starts_with('/')
        && !name.ends_with('/')
        && !name.ends_with(".lock")
        && !name.ends_with('.')
        && !name.contains("..")
        && !name.contains("@{")
        && !name
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c))
}

fn pick_folder(start: &str) -> Option<String> {
    rfd::FileDialog::new()
        .set_directory(start)
        .pick_folder()
        .map(|p| p.display().to_string())
}

fn field(ui: &mut Ui, label: &str, value: &mut String, hint: &str, focus: bool) -> egui::Response {
    let p = palette(ui.ctx());
    ui.label(RichText::new(label).size(12.0).color(p.text_dim));
    let r = ui.add(
        egui::TextEdit::singleline(value)
            .hint_text(hint)
            .desired_width(f32::INFINITY)
            .margin(egui::Margin::symmetric(8, 6)),
    );
    if focus && ui.memory(|m| m.focused().is_none()) {
        r.request_focus();
    }
    ui.add_space(8.0);
    r
}

/// Draws the dialog. `tab` is the active repository, for branch lists and defaults.
pub fn show(ctx: &egui::Context, dialog: &mut Dialog, tab: Option<&RepoTab>) -> Outcome {
    let p = palette(ctx);
    let mut outcome = Outcome::Keep;
    let modal = egui::Modal::new(Id::new("dialog"))
        .frame(egui::Frame::new().fill(p.elevated).stroke(egui::Stroke::new(1.0, p.border)).corner_radius(egui::CornerRadius::same(theme::RADIUS_LG)).shadow(ctx.global_style().visuals.window_shadow).inner_margin(egui::Margin::same(22)))
        .backdrop_color(egui::Color32::from_black_alpha(if p.dark { 150 } else { 70 }))
        .show(ctx, |ui| {
            ui.set_width(460.0);
            let enter = ui.input(|i| i.key_pressed(Key::Enter) && !i.modifiers.shift);
            let mut primary = |ui: &mut Ui, label: &str, enabled: bool| -> bool {
                ui.add_space(6.0);
                let mut go = false;
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        go = widgets::primary_button(ui, label, enabled, 150.0).clicked() || (enter && enabled);
                        if ui.add_sized([90.0, 34.0], egui::Button::new("Cancel")).clicked() {
                            outcome = Outcome::Close;
                        }
                    });
                });
                go
            };
            let title = |ui: &mut Ui, glyph: &str, text: &str| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(glyph).size(20.0).color(p.accent));
                    ui.label(RichText::new(text).font(FontId::new(17.0, theme::semibold())));
                });
                ui.add_space(12.0);
            };
            match dialog {
                Dialog::Open { path } => {
                    title(ui, icon::FOLDER_OPEN, "Open repository");
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Folder").size(12.0).color(p.text_dim));
                    });
                    ui.horizontal(|ui| {
                        let r = ui.add(egui::TextEdit::singleline(path).desired_width(340.0).margin(egui::Margin::symmetric(8, 6)));
                        if ui.memory(|m| m.focused().is_none()) {
                            r.request_focus();
                        }
                        if ui.button(format!("{}  Browse…", icon::FOLDER)).clicked()
                            && let Some(picked) = pick_folder(path) {
                                *path = picked;
                            }
                    });
                    ui.add_space(8.0);
                    let ok = !path.trim().is_empty();
                    if primary(ui, "Open", ok) {
                        outcome = Outcome::Run(vec![Action::OpenRepo(path.trim().into())]);
                    }
                }
                Dialog::Clone { builder, parent } => {
                    title(ui, icon::CLOUD_ARROW_DOWN, "Clone repository");
                    builder.ui(ui, true);
                    let url = builder.resolve().unwrap_or_default();
                    ui.label(RichText::new("Clone into").size(12.0).color(p.text_dim));
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(parent).desired_width(340.0).margin(egui::Margin::symmetric(8, 6)));
                        if ui.button(format!("{}  Browse…", icon::FOLDER)).clicked()
                            && let Some(picked) = pick_folder(parent) {
                                *parent = picked;
                            }
                    });
                    let name = url.trim().trim_end_matches('/').trim_end_matches(".git").rsplit(['/', ':']).next().unwrap_or("").to_owned();
                    let dest = std::path::Path::new(parent.trim()).join(&name);
                    ui.add_space(4.0);
                    ui.label(RichText::new(format!("→ {}", dest.display())).size(12.0).color(p.text_faint));
                    ui.add_space(8.0);
                    if dest.exists() && !name.is_empty() {
                        ui.label(RichText::new("That folder already exists").size(12.0).color(p.danger));
                    }
                    let ok = !url.trim().is_empty() && !name.is_empty() && !dest.exists();
                    if primary(ui, "Clone", ok) {
                        outcome = Outcome::Run(vec![Action::Clone { url: url.trim().to_owned(), dest }]);
                    }
                }
                Dialog::Init { path } => {
                    title(ui, icon::PLANT, "New repository");
                    ui.label(RichText::new("Folder (created if missing)").size(12.0).color(p.text_dim));
                    ui.horizontal(|ui| {
                        let r = ui.add(egui::TextEdit::singleline(path).desired_width(340.0).hint_text("~/src/my-project").margin(egui::Margin::symmetric(8, 6)));
                        if ui.memory(|m| m.focused().is_none()) {
                            r.request_focus();
                        }
                        if ui.button(format!("{}  Browse…", icon::FOLDER)).clicked()
                            && let Some(picked) = pick_folder(&home()) {
                                *path = picked;
                            }
                    });
                    ui.add_space(8.0);
                    if primary(ui, "Create", !path.trim().is_empty()) {
                        let expanded = if let Some(rest) = path.trim().strip_prefix("~/") { std::path::Path::new(&home()).join(rest) } else { path.trim().into() };
                        outcome = Outcome::Run(vec![Action::Init(expanded)]);
                    }
                }
                Dialog::NewBranch { name, start, checkout } => {
                    title(ui, icon::GIT_BRANCH, "New branch");
                    let r = field(ui, "Name", name, "feature/something", true);
                    let _ = r;
                    let from = match start.as_deref() {
                        Some(s) if s.len() == 40 => format!("commit {}", &s[..7]),
                        Some(s) => s.to_owned(),
                        None => tab.and_then(|t| t.snap.as_ref()).map(|s| s.head.label()).unwrap_or_else(|| "HEAD".into()),
                    };
                    ui.label(RichText::new(format!("{}  Starts at {from}", icon::GIT_COMMIT)).size(12.0).color(p.text_faint));
                    ui.add_space(6.0);
                    ui.checkbox(checkout, "Check out after creating");
                    ui.add_space(8.0);
                    let ok = valid_ref_name(name.trim());
                    if !name.is_empty() && !ok {
                        ui.label(RichText::new("Not a valid branch name").size(12.0).color(p.danger));
                    }
                    if primary(ui, "Create branch", ok) {
                        outcome = Outcome::Run(vec![Action::Op(Op::CreateBranch { name: name.trim().to_owned(), start: start.clone(), checkout: *checkout })]);
                    }
                }
                Dialog::Rename { old, new } => {
                    title(ui, icon::PENCIL_SIMPLE, &format!("Rename {old}"));
                    field(ui, "New name", new, "", true);
                    let ok = valid_ref_name(new.trim()) && new.trim() != old;
                    if primary(ui, "Rename", ok) {
                        outcome = Outcome::Run(vec![Action::Op(Op::RenameBranch { old: old.clone(), new: new.trim().to_owned() })]);
                    }
                }
                Dialog::NewTag { name, target, message } => {
                    title(ui, icon::TAG, &format!("Tag {}", short_id(*target)));
                    field(ui, "Name", name, "v1.2.0", true);
                    ui.label(RichText::new("Message (makes an annotated tag)").size(12.0).color(p.text_dim));
                    ui.add(egui::TextEdit::multiline(message).desired_rows(3).desired_width(f32::INFINITY).margin(egui::Margin::symmetric(8, 6)));
                    ui.add_space(8.0);
                    let ok = valid_ref_name(name.trim());
                    if primary(ui, "Create tag", ok) {
                        let m = (!message.trim().is_empty()).then(|| message.trim().to_owned());
                        outcome = Outcome::Run(vec![Action::Op(Op::CreateTag { name: name.trim().to_owned(), target: *target, message: m })]);
                    }
                }
                Dialog::Stash { message, untracked } => {
                    title(ui, icon::ARCHIVE, "Stash changes");
                    field(ui, "Message (optional)", message, "WIP: what you were doing", true);
                    ui.checkbox(untracked, "Include untracked files");
                    ui.add_space(8.0);
                    if primary(ui, "Stash", true) {
                        let m = (!message.trim().is_empty()).then(|| message.trim().to_owned());
                        outcome = Outcome::Run(vec![Action::Op(Op::StashSave { message: m, untracked: *untracked })]);
                    }
                }
                Dialog::Merge { target, rebase, mode } => {
                    let current = tab.and_then(|t| t.snap.as_ref()).map(|s| s.head.label()).unwrap_or_default();
                    title(ui, icon::GIT_MERGE, if *rebase { "Rebase" } else { "Merge" });
                    ui.horizontal(|ui| {
                        ui.selectable_value(rebase, false, "Merge into current");
                        ui.selectable_value(rebase, true, "Rebase current onto");
                    });
                    ui.add_space(8.0);
                    let branches: Vec<String> = tab
                        .and_then(|t| t.snap.as_ref())
                        .map(|s| s.branches.iter().filter(|b| !b.is_head).map(|b| b.name.clone()).chain(s.remote_branches.iter().map(|b| b.full_name())).collect())
                        .unwrap_or_default();
                    egui::ComboBox::from_id_salt("merge-target").width(410.0).selected_text(if target.is_empty() { "Choose a branch…".to_owned() } else { target.clone() }).show_ui(ui, |ui| {
                        for b in branches {
                            ui.selectable_value(target, b.clone(), b);
                        }
                    });
                    ui.add_space(8.0);
                    if !*rebase {
                        ui.label(RichText::new("How").size(12.0).color(p.text_dim));
                        for m in canopy_core::MergeMode::ALL {
                            ui.radio_value(mode, m, m.label());
                        }
                        ui.add_space(6.0);
                    }
                    if !target.is_empty() {
                        use canopy_core::MergeMode as M;
                        let text = match (*rebase, *mode) {
                            (true, _) => format!("Replay {current}'s commits on top of {target}."),
                            (false, M::Default) => format!("Merge {target} into {current}, fast-forwarding when {current} has no commits of its own."),
                            (false, M::NoFastForward) => format!("Merge {target} into {current} with a merge commit, even when a fast-forward is possible."),
                            (false, M::FastForwardOnly) => format!("Move {current} forward to {target}; stops without changes if they have diverged."),
                            (false, M::Squash) => format!("Stage all of {target}'s changes on {current} as one change, without committing. Review them in Changes, then commit."),
                        };
                        ui.label(RichText::new(text).size(12.0).color(p.text_faint));
                    }
                    ui.add_space(8.0);
                    let verb = match (*rebase, *mode) {
                        (true, _) => "Rebase",
                        (false, canopy_core::MergeMode::Squash) => "Squash",
                        _ => "Merge",
                    };
                    if primary(ui, verb, !target.is_empty()) {
                        let op = if *rebase { Op::Rebase(target.clone()) } else { Op::Merge(target.clone(), *mode) };
                        outcome = Outcome::Run(vec![Action::Op(op)]);
                    }
                }
                Dialog::AddRemote { name, builder, publish } => {
                    if *publish {
                        title(ui, icon::CLOUD_ARROW_UP, "Publish repository");
                        ui.label(RichText::new("This repository is local only. Create an empty repository on GitHub or GitLab, enter it here, and Canopy adds it as a remote and pushes the current branch.").size(12.5).color(p.text_dim));
                        ui.add_space(10.0);
                    } else {
                        title(ui, icon::CLOUD, "Add remote");
                    }
                    field(ui, "Name", name, "origin", false);
                    builder.ui(ui, true);
                    let url = builder.resolve();
                    let ok = valid_ref_name(name.trim()) && url.is_some();
                    if primary(ui, if *publish { "Add and push" } else { "Add remote" }, ok)
                        && let Some(url) = url
                    {
                        let mut run = vec![Action::Op(Op::AddRemote { name: name.trim().to_owned(), url })];
                        if *publish {
                            run.push(Action::Op(Op::Push { force: false }));
                        }
                        outcome = Outcome::Run(run);
                    }
                }
                Dialog::NewRequest { kind, request, branches } => {
                    title(ui, icon::GIT_PULL_REQUEST, &format!("New {}", kind.request_noun()));
                    ui.label(RichText::new(format!("{}  {}  →", icon::GIT_BRANCH, request.source)).size(12.5).color(p.text_dim));
                    ui.add_space(4.0);
                    egui::ComboBox::from_id_salt("request-target").width(410.0).selected_text(format!("{}  {}", icon::GIT_BRANCH, request.target)).show_ui(ui, |ui| {
                        for b in branches.iter().filter(|b| **b != request.source) {
                            ui.selectable_value(&mut request.target, b.clone(), b);
                        }
                    });
                    ui.add_space(10.0);
                    field(ui, "Title", &mut request.title, "What this change does", true);
                    ui.label(RichText::new("Description").size(12.0).color(p.text_dim));
                    ui.add(egui::TextEdit::multiline(&mut request.body).desired_rows(6).desired_width(f32::INFINITY).hint_text("Why, how it was tested, anything reviewers should know (Markdown)").margin(egui::Margin::symmetric(8, 6)));
                    ui.add_space(6.0);
                    ui.checkbox(&mut request.draft, "Create as draft");
                    ui.add_space(8.0);
                    let ok = !request.title.trim().is_empty() && !request.target.is_empty() && request.target != request.source;
                    if primary(ui, &format!("Create {}", kind.request_abbrev()), ok) {
                        let mut r = request.clone();
                        r.title = r.title.trim().to_owned();
                        outcome = Outcome::Run(vec![Action::Host(HostJob::Create(r))]);
                    }
                }
                Dialog::Confirm { title: t, body, confirm, danger, op } => {
                    let glyph = if *danger { icon::WARNING } else { icon::QUESTION };
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(glyph).size(20.0).color(if *danger { p.danger } else { p.accent }));
                        ui.label(RichText::new(t.as_str()).font(FontId::new(17.0, theme::semibold())));
                    });
                    ui.add_space(10.0);
                    ui.label(RichText::new(body.as_str()).color(p.text_dim));
                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let size = egui::vec2(120.0, 34.0);
                            let b = if *danger { widgets::danger_button(ui, confirm, size) } else { widgets::accent_button(ui, confirm, size) };
                            if b.clicked() {
                                outcome = Outcome::Run(vec![Action::Op(op.clone())]);
                            }
                            if ui.add_sized([90.0, 34.0], egui::Button::new("Cancel")).clicked() {
                                outcome = Outcome::Close;
                            }
                        });
                    });
                }
                Dialog::Update(a) => {
                    title(ui, icon::ARROW_CIRCLE_UP, &format!("Canopy {} is available", a.version));
                    ui.label(RichText::new(format!("You have {}.{}", env!("CARGO_PKG_VERSION"), a.manifest.pub_date.as_deref().and_then(|d| d.get(..10)).map(|d| format!(" Released {d}.")).unwrap_or_default())).size(12.5).color(p.text_dim));
                    ui.add_space(8.0);
                    egui::Frame::new().fill(p.panel).stroke(egui::Stroke::new(1.0, p.border)).corner_radius(egui::CornerRadius::same(theme::RADIUS)).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                            let notes = a.manifest.notes.trim();
                            if notes.is_empty() {
                                ui.label(RichText::new("No release notes.").color(p.text_faint));
                            } else {
                                crate::ui::notes::markdown(ui, notes);
                            }
                        });
                    });
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let download = a.manifest.artifact().is_some();
                            if widgets::primary_button(ui, if download { "Download" } else { "Open release" }, true, 150.0).clicked() {
                                outcome = Outcome::Run(vec![Action::OpenExternal(a.download())]);
                            }
                            if ui.add_sized([90.0, 34.0], egui::Button::new("Later")).clicked() {
                                outcome = Outcome::Close;
                            }
                            if ui.add_sized([130.0, 34.0], egui::Button::new("Skip this version")).clicked() {
                                outcome = Outcome::Run(vec![Action::SkipVersion(a.version.to_string())]);
                            }
                        });
                    });
                }
                Dialog::WhatsNew { version, notes } => {
                    title(ui, icon::SPARKLE, &format!("What's new in Canopy {version}"));
                    egui::Frame::new().fill(p.panel).stroke(egui::Stroke::new(1.0, p.border)).corner_radius(egui::CornerRadius::same(theme::RADIUS)).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| crate::ui::notes::markdown(ui, notes));
                    });
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::primary_button(ui, "Continue", true, 120.0).clicked() || enter {
                                outcome = Outcome::Close;
                            }
                            if ui.add_sized([150.0, 34.0], egui::Button::new(format!("{}  All releases", icon::ARROW_SQUARE_OUT))).clicked() {
                                outcome = Outcome::Run(vec![Action::OpenExternal(crate::updater::RELEASES_PAGE.to_owned())]);
                            }
                        });
                    });
                }
                Dialog::Crash(report) => {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(icon::FIRST_AID_KIT).size(20.0).color(p.warning));
                        ui.label(RichText::new("Canopy closed unexpectedly").font(FontId::new(17.0, theme::semibold())));
                    });
                    ui.add_space(10.0);
                    ui.label(RichText::new("Sorry about that. A crash report was saved on this computer; nothing was sent anywhere. Sharing it with the developers helps fix the problem.").color(p.text_dim));
                    if !report.message.is_empty() {
                        ui.add_space(8.0);
                        egui::Frame::new().fill(p.panel).stroke(egui::Stroke::new(1.0, p.border)).corner_radius(egui::CornerRadius::same(theme::RADIUS)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(RichText::new(&report.message).font(FontId::monospace(12.0)).color(p.text));
                        });
                    }
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::primary_button(ui, "Open report", true, 130.0).clicked() {
                                outcome = Outcome::Run(vec![Action::OpenExternal(report.path.display().to_string())]);
                            }
                            if ui.add_sized([90.0, 34.0], egui::Button::new("Dismiss")).clicked() {
                                outcome = Outcome::Close;
                            }
                            if let Some(dir) = report.path.parent()
                                && ui.add_sized([120.0, 34.0], egui::Button::new(format!("{}  Show folder", icon::FOLDER))).clicked()
                            {
                                outcome = Outcome::Run(vec![Action::OpenExternal(dir.display().to_string())]);
                            }
                        });
                    });
                }
                Dialog::About => {
                    ui.vertical_centered(|ui| {
                        art::show(ui, art::logo(), egui::vec2(64.0, 64.0));
                        ui.label(RichText::new("Canopy").font(FontId::new(24.0, theme::semibold())));
                        ui.label(RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).color(p.text_dim));
                        ui.add_space(8.0);
                        ui.label(RichText::new("A fast, beautiful, Linux-first Git client.").color(p.text_dim));
                        ui.label(RichText::new("© 2026 NecturaLabs. Built with Rust, egui and libgit2.").size(12.0).color(p.text_faint));
                        ui.label(RichText::new("Fonts: Inter and JetBrains Mono (SIL OFL 1.1). Icons: Phosphor (MIT).").size(12.0).color(p.text_faint));
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            ui.add_space((ui.available_width() - 400.0).max(0.0) / 2.0);
                            if ui.button(format!("{}  Check for updates", icon::ARROW_CIRCLE_UP)).clicked() {
                                outcome = Outcome::Run(vec![Action::CheckUpdates]);
                            }
                            if ui.button(format!("{}  What's new", icon::SPARKLE)).clicked() {
                                outcome = Outcome::Run(vec![Action::WhatsNew]);
                            }
                            if ui.button(format!("{}  Releases", icon::ARROW_SQUARE_OUT)).clicked() {
                                outcome = Outcome::Run(vec![Action::OpenExternal(crate::updater::RELEASES_PAGE.to_owned())]);
                            }
                            if ui.button("Close").clicked() {
                                outcome = Outcome::Close;
                            }
                        });
                    });
                }
            }
        });
    if modal.should_close() && matches!(outcome, Outcome::Keep) {
        outcome = Outcome::Close;
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::valid_ref_name;

    #[test]
    fn ref_names() {
        assert!(valid_ref_name("feature/login"));
        assert!(valid_ref_name("v1.2.0"));
        assert!(!valid_ref_name(""));
        assert!(!valid_ref_name("has space"));
        assert!(!valid_ref_name("a..b"));
        assert!(!valid_ref_name("-x"));
        assert!(!valid_ref_name("x.lock"));
        assert!(!valid_ref_name("x~1"));
    }
}
