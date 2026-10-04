//! Modal dialogs. Each returns the actions to run when the user confirms.

use egui::{FontId, Id, Key, RichText, Ui};
use egui_phosphor::regular as icon;

use canopy_core::forge::{self, ForgeKind};
use canopy_core::{Oid, short_id};
use canopy_hosting::NewRequest;

use crate::action::Action;
use crate::askpass::{Prompt, PromptKind};
use crate::hosting::HostJob;
use crate::settings::{CloneProtocol, Settings};
use crate::tab::RepoTab;
use crate::ui::legal::{Document, Pending};
use crate::ui::theme::{self, palette};
use crate::ui::{art, widgets};
use crate::worker::Op;

mod common;
mod credential;
mod stack;
mod start;
mod update;
mod url;

use common::*;
use credential::credential;
pub use stack::DialogStack;
pub use start::Start;
pub use url::UrlBuilder;

/// Every dialog Canopy shows.
#[derive(Debug, Clone)]
pub enum Dialog {
    Open {
        path: String,
    },
    Clone {
        builder: UrlBuilder,
        parent: String,
        pending: bool,
    },
    Init {
        path: String,
    },
    NewBranch {
        name: String,
        start: Start,
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
    Identity {
        name: String,
        email: String,
        pending: bool,
    },
    /// Removing the identity from one Git configuration file.
    ConfirmClearIdentity {
        repo: Option<std::path::PathBuf>,
        scope: canopy_core::identity::Scope,
    },
    /// Allow downloads in the partial clones of a folder: the repository's own, or its parent.
    TrustFolder {
        repo: std::path::PathBuf,
        /// The promisor remote the downloads would come from.
        remote: String,
    },
    /// What's new in an available update.
    Update(Box<crate::updater::Available>),
    /// What's new in this version (after an upgrade, or from the palette).
    WhatsNew {
        version: String,
        notes: String,
    },
    /// Canopy crashed last time; offer the local report.
    Crash(crate::logging::CrashReport),
    /// First start (or changed terms): accept the terms, then answer about crash reports. Cannot
    /// be dismissed.
    Consent {
        step: Pending,
        /// The terms changed since the user last accepted them.
        updated: bool,
        /// A document expanded inline.
        reading: Option<Document>,
    },
    /// The Terms of Service or the Privacy Policy.
    Legal(Document),
    /// Confirm a pull/merge request action (merge, close).
    ConfirmHost {
        title: String,
        body: String,
        confirm: String,
        danger: bool,
        job: HostJob,
    },
    /// A question from git or ssh: a user name, password, passphrase or host key.
    Credential {
        prompt: Prompt,
        answer: String,
        /// Show the secret being typed.
        reveal: bool,
    },
}

/// What a dialog wants after this frame.
pub enum Outcome {
    Keep,
    Close,
    Run(Vec<Action>),
    RunKeep(Vec<Action>),
}

impl Dialog {
    /// The dialog answering a credential prompt.
    pub fn credential(prompt: Prompt) -> Self {
        Dialog::Credential {
            prompt,
            answer: String::new(),
            reveal: false,
        }
    }

    /// The open-repository dialog, prefilled with the home directory.
    pub fn open() -> Self {
        Dialog::Open { path: home() }
    }

    /// The clone dialog with the default clone folder as the destination.
    pub fn clone_default(settings: &Settings) -> Self {
        Dialog::Clone {
            builder: UrlBuilder::new(settings),
            parent: settings.clone_parent(),
            pending: false,
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

    /// New branch starting at `start`.
    pub fn new_branch_at(start: Start) -> Self {
        Dialog::NewBranch {
            name: String::new(),
            start,
            checkout: true,
        }
    }
}

/// Draws the dialog. `tab` is the active repository, for branch lists and defaults.
pub fn show(
    ctx: &egui::Context,
    dialog: &mut Dialog,
    tab: Option<&RepoTab>,
    picker: &mut crate::ui::folder_picker::FolderPicker,
    clone_running: bool,
    updater: &crate::updater::Updater,
    git_version: &str,
) -> Outcome {
    let p = palette(ctx);
    let mut outcome = Outcome::Keep;
    let modal = egui::Modal::new(Id::new("dialog"))
        .frame(egui::Frame::new().fill(p.elevated).stroke(egui::Stroke::new(1.0, p.border)).corner_radius(egui::CornerRadius::same(theme::RADIUS_LG)).shadow(ctx.global_style().visuals.window_shadow).inner_margin(egui::Margin::same(22)))
        .backdrop_color(p.backdrop())
        .show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - theme::XL * 4.0).clamp(0.0, 460.0));
            ui.set_max_height((ctx.content_rect().height() - theme::XL * 4.0).max(0.0));
            ui.spacing_mut().scroll = theme::solid_scroll();
            let document_actions = matches!(dialog, Dialog::Consent { .. } | Dialog::Legal(_));
            let pinned_actions = document_actions && ctx.content_rect().height() < 600.0;
            if pinned_actions {
                ui.set_height((ctx.content_rect().height() - theme::XL * 4.0).max(0.0));
                egui::Panel::bottom("dialog-actions")
                    .frame(egui::Frame::new())
                    .show(ui, |ui| { outcome = common::document_actions(ui, dialog); });
            }
            egui::ScrollArea::vertical()
                .id_salt(("dialog-body", std::mem::discriminant(dialog)))
                .max_height(if pinned_actions { ui.available_height() } else { (ctx.content_rect().height() - theme::XL * 4.0).max(0.0) })
                .min_scrolled_height(0.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
            // Enter submits, except in a multi-line field, where it starts a new line and
            // Ctrl+Enter submits instead.
            let in_multiline = ctx.data(|d| d.get_temp::<Id>(multiline_key())).is_some_and(|id| ctx.memory(|m| m.has_focus(id)));
            let enter = ui.input(|i| i.key_pressed(Key::Enter) && !i.modifiers.shift && (!in_multiline || i.modifiers.command));
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
                    ui.label(RichText::new(glyph).size(theme::ICON_TITLE).color(p.accent));
                    ui.label(RichText::new(text).font(FontId::new(theme::TITLE_SIZE, theme::semibold())));
                });
                ui.add_space(12.0);
            };
            match dialog {
                Dialog::Identity { name, email, pending } => {
                    title(ui, icon::IDENTIFICATION_CARD, "Set your Git identity");
                    ui.label("Git stores an author name and email in every new commit. Enter the details you want to use. Canopy will not guess them, and commits already made keep their authors.");
                    ui.add_space(theme::MD);
                    ui.label("Name");
                    ui.add(egui::TextEdit::singleline(name).hint_text("e.g. Ada Lovelace").desired_width(ui.available_width()).margin(theme::FIELD_MARGIN));
                    ui.label("Email");
                    ui.add(egui::TextEdit::singleline(email).hint_text("e.g. ada@example.com").desired_width(ui.available_width()).margin(theme::FIELD_MARGIN));
                    ui.add_space(theme::XS);
                    ui.label(RichText::new("Saved to your global Git configuration, so every repository without its own identity uses it. You can change it later in Settings → Git.").size(theme::META_SIZE).color(p.text_dim));
                    let ok = !name.trim().is_empty() && !email.trim().is_empty() && !*pending;
                    if !ok && !*pending {
                        ui.add_space(theme::XS);
                        ui.label(RichText::new(if name.trim().is_empty() && email.trim().is_empty() { "Enter a name and an email to save." } else if name.trim().is_empty() { "Enter a name to save." } else { "Enter an email to save." }).size(theme::META_SIZE).color(p.text_faint));
                    }
                    if primary(ui, "Save globally", ok) {
                        outcome = Outcome::RunKeep(vec![Action::SaveIdentity {
                            repo: None,
                            scope: canopy_core::identity::Scope::Global,
                            name: name.clone(),
                            email: email.clone(),
                        }]);
                    }
                }
                Dialog::Open { path } => {
                    title(ui, icon::FOLDER_OPEN, "Open repository");
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Folder").size(theme::META_SIZE).color(p.text_dim));
                    });
                    ui.horizontal(|ui| {
                        let r = ui.add(egui::TextEdit::singleline(path).desired_width((ui.available_width() - 110.0).max(80.0)).margin(theme::FIELD_MARGIN));
                        if ui.memory(|m| m.focused().is_none()) {
                            r.request_focus();
                        }
                        if let Some(picked) = picker.show(ui, "open", &crate::settings::expand_home(path)) {
                                *path = picked.display().to_string();
                            }
                    });
                    ui.add_space(8.0);
                    let ok = !path.trim().is_empty();
                    if primary(ui, "Open", ok) {
                        outcome = Outcome::Run(vec![Action::OpenRepo(path.trim().into())]);
                    }
                }
                Dialog::Clone { builder, parent, pending } => {
                    title(ui, icon::CLOUD_ARROW_DOWN, "Clone repository");
                    let (url, dest, ok) = ui.add_enabled_ui(!*pending, |ui| {
                        builder.ui(ui, true);
                        let url = builder.resolve().unwrap_or_default();
                        ui.label(RichText::new("Clone into").size(theme::META_SIZE).color(p.text_dim));
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(parent).desired_width((ui.available_width() - 110.0).max(80.0)).margin(theme::FIELD_MARGIN));
                            if let Some(picked) = picker.show(ui, "clone", &crate::settings::expand_home(parent)) {
                                *parent = picked.display().to_string();
                            }
                        });
                        let name = url.trim().trim_end_matches('/').trim_end_matches(".git").rsplit(['/', ':']).next().unwrap_or("").to_owned();
                        let dest = crate::settings::expand_home(parent).join(&name);
                        ui.add_space(4.0);
                        ui.label(RichText::new(format!("→ {}", dest.display())).size(theme::META_SIZE).color(p.text_faint));
                        let ok = !url.trim().is_empty() && !name.is_empty();
                        (url, dest, ok)
                    }).inner;
                    ui.add_space(8.0);
                    if *pending {
                        ui.horizontal(|ui| { ui.spinner(); ui.label("Cloning repository…"); });
                        if ui.button("Continue in background").clicked() { outcome = Outcome::Close; }
                    } else if primary(ui, "Clone", ok && !clone_running) {
                        *pending = true;
                        outcome = Outcome::RunKeep(vec![Action::Clone { url: url.trim().to_owned(), dest }]);
                    }
                }
                Dialog::Init { path } => {
                    title(ui, icon::PLANT, "New repository");
                    ui.label(RichText::new("Folder (created if missing)").size(theme::META_SIZE).color(p.text_dim));
                    ui.horizontal(|ui| {
                        let r = ui.add(egui::TextEdit::singleline(path).desired_width((ui.available_width() - 110.0).max(80.0)).hint_text("~/src/my-project").margin(theme::FIELD_MARGIN));
                        if ui.memory(|m| m.focused().is_none()) {
                            r.request_focus();
                        }
                        if let Some(picked) = picker.show(ui, "init", &crate::settings::expand_home(&home())) {
                                *path = picked.display().to_string();
                            }
                    });
                    ui.add_space(8.0);
                    if primary(ui, "Create", !path.trim().is_empty()) {
                        outcome = Outcome::Run(vec![Action::Init(crate::settings::expand_home(path))]);
                    }
                }
                Dialog::NewBranch { name, start, checkout } => {
                    title(ui, icon::GIT_BRANCH, "New branch");
                    let r = field(ui, "Name", name, "feature/something", true);
                    let _ = r;
                    let from = start.label(|| tab.and_then(|t| t.snap.as_ref()).map(|s| s.head.label()).unwrap_or_else(|| "HEAD".into()));
                    ui.label(RichText::new(format!("{}  Starts at {from}", icon::GIT_COMMIT)).size(theme::META_SIZE).color(p.text_faint));
                    ui.add_space(6.0);
                    widgets::checkbox(ui, checkout, "Check out after creating");
                    ui.add_space(8.0);
                    let ok = valid_ref_name(name.trim());
                    if !name.is_empty() && !ok {
                        ui.label(RichText::new("Not a valid branch name").size(theme::META_SIZE).color(p.danger));
                    }
                    if primary(ui, "Create branch", ok) {
                        outcome = Outcome::Run(vec![Action::Op(Op::CreateBranch { name: name.trim().to_owned(), start: start.rev(), checkout: *checkout })]);
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
                    ui.label(RichText::new("Message (makes an annotated tag)").size(theme::META_SIZE).color(p.text_dim));
                    let r = ui.add(egui::TextEdit::multiline(message).desired_rows(3).desired_width(f32::INFINITY).margin(theme::FIELD_MARGIN));
                    remember_multiline(ui, r.id);
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
                    widgets::checkbox(ui, untracked, "Include untracked files");
                    ui.add_space(8.0);
                    if primary(ui, "Stash", true) {
                        let m = (!message.trim().is_empty()).then(|| message.trim().to_owned());
                        outcome = Outcome::Run(vec![Action::Op(Op::StashSave { message: m, untracked: *untracked })]);
                    }
                }
                Dialog::Merge { target, rebase, mode } => {
                    let current = tab.and_then(|t| t.snap.as_ref()).map(|s| s.head.label()).unwrap_or_default();
                    title(ui, icon::GIT_MERGE, if *rebase { "Rebase" } else { "Merge" });
                    // The branch name keeps the two choices from reading as "current" in the abstract.
                    let shown: String = if current.chars().count() > 20 { current.chars().take(19).chain(['…']).collect() } else { current.clone() };
                    let (merge_label, rebase_label) = if shown.is_empty() { ("Merge into current".to_owned(), "Rebase current onto".to_owned()) } else { (format!("Merge into {shown}"), format!("Rebase {shown} onto")) };
                    widgets::segmented(ui, rebase, &[(false, merge_label.as_str()), (true, rebase_label.as_str())]);
                    ui.add_space(8.0);
                    egui::ComboBox::from_id_salt("merge-target").icon(crate::ui::widgets::combo_caret).width(ui.available_width()).selected_text(if target.is_empty() { "Choose a branch…".to_owned() } else { target.clone() }).show_ui(ui, |ui| {
                        // Listed only while the menu is open, not on every frame of the dialog.
                        if let Some(s) = tab.and_then(|t| t.snap.as_ref()) {
                            for b in s.branches.iter().filter(|b| !b.is_head).map(|b| b.name.clone()).chain(s.remote_branches.iter().map(|b| b.full_name())) {
                                ui.selectable_value(target, b.clone(), b);
                            }
                        }
                    });
                    ui.add_space(8.0);
                    if !*rebase {
                        ui.label(RichText::new("How").size(theme::META_SIZE).color(p.text_dim));
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
                        ui.label(RichText::new(text).size(theme::META_SIZE).color(p.text_faint));
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
                        ui.label(RichText::new("This repository is local only. Create an empty repository on GitHub or GitLab, enter it here, and Canopy adds it as a remote and pushes the current branch.").size(theme::META_SIZE).color(p.text_dim));
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
                    ui.label(RichText::new(format!("{}  {}  →", icon::GIT_BRANCH, request.source)).size(theme::META_SIZE).color(p.text_dim));
                    ui.add_space(4.0);
                    egui::ComboBox::from_id_salt("request-target").icon(crate::ui::widgets::combo_caret).width(ui.available_width()).selected_text(format!("{}  {}", icon::GIT_BRANCH, request.target)).show_ui(ui, |ui| {
                        for b in branches.iter().filter(|b| **b != request.source) {
                            ui.selectable_value(&mut request.target, b.clone(), b);
                        }
                    });
                    ui.add_space(10.0);
                    field(ui, "Title", &mut request.title, "What this change does", true);
                    ui.label(RichText::new("Description").size(theme::META_SIZE).color(p.text_dim));
                    let r = ui.add(egui::TextEdit::multiline(&mut request.body).desired_rows(6).desired_width(f32::INFINITY).hint_text("Why, how it was tested, anything reviewers should know (Markdown)").margin(theme::FIELD_MARGIN));
                    remember_multiline(ui, r.id);
                    ui.add_space(6.0);
                    widgets::checkbox(ui, &mut request.draft, "Create as draft");
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
                        ui.label(RichText::new(glyph).size(theme::ICON_TITLE).color(if *danger { p.danger } else { p.accent }));
                        ui.label(RichText::new(t.as_str()).font(FontId::new(theme::TITLE_SIZE, theme::semibold())));
                    });
                    ui.add_space(10.0);
                    ui.label(RichText::new(body.as_str()).color(p.text_dim));
                    ui.add_space(16.0);
                    match confirm_buttons(ui, confirm, *danger) {
                        Some(true) => outcome = Outcome::Run(vec![Action::Op(op.clone())]),
                        Some(false) => outcome = Outcome::Close,
                        None => {}
                    }
                }
                Dialog::ConfirmClearIdentity { repo, scope } => {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(icon::WARNING).size(theme::ICON_TITLE).color(p.danger));
                        ui.label(RichText::new("Clear Git identity?").font(FontId::new(theme::TITLE_SIZE, theme::semibold())));
                    });
                    ui.add_space(10.0);
                    let target = match scope {
                        canopy_core::identity::Scope::Local => "this repository's configuration",
                        canopy_core::identity::Scope::Global => "your global Git configuration, which every repository without its own identity uses",
                    };
                    ui.label(RichText::new(format!("This removes user.name and user.email from {target}. Commits already made keep their authors. If the same value is also set in another file, that one stays in use.")).color(p.text_dim));
                    if let Some(values) = identity_in_scope(tab, *scope) {
                        ui.add_space(theme::SM);
                        egui::Frame::new().fill(p.panel).stroke(egui::Stroke::new(1.0, p.border)).corner_radius(egui::CornerRadius::same(theme::RADIUS)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(RichText::new("Removing").size(theme::META_SIZE).color(p.text_dim));
                            ui.label(RichText::new(values).font(FontId::monospace(theme::MONO_SIZE)).color(p.text));
                        });
                    }
                    ui.add_space(16.0);
                    match confirm_buttons(ui, "Clear identity", true) {
                        Some(true) => outcome = Outcome::Run(vec![Action::ClearIdentity { repo: repo.clone(), scope: *scope }]),
                        Some(false) => outcome = Outcome::Close,
                        None => {}
                    }
                }
                Dialog::TrustFolder { repo, remote } => {
                    title(ui, icon::SHIELD_CHECK, "Trust this folder?");
                    let name = |path: &std::path::Path| path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
                    ui.label(RichText::new(format!("{} is a partial clone: some of its files were left on the server. To show them, Canopy has to download them from {remote} with git, and git then follows this repository's own configuration, which can run programs and contact servers.", name(repo))).color(p.text_dim));
                    ui.add_space(theme::SM);
                    ui.label(RichText::new("Trust a folder only if everything in it was made by you or by people you trust. You can remove it again in Settings.").color(p.text_dim));
                    if let Some(parent) = crate::trust::parent_offer(repo) {
                        ui.add_space(theme::MD);
                        ui.separator();
                        ui.add_space(theme::XS);
                        ui.label(RichText::new(format!("Trusting {} covers every repository inside it, including ones you clone later. Only do that for a folder that is entirely yours.", name(&parent))).size(theme::META_SIZE).color(p.text_faint));
                        let broader = egui::Button::new(RichText::new(format!("Trust everything in {}", name(&parent))).size(theme::META_SIZE).color(p.warning)).frame(false);
                        if ui.add(broader).on_hover_text(parent.display().to_string()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            outcome = Outcome::Run(vec![Action::TrustFolder(parent)]);
                        }
                    }
                    ui.add_space(theme::MD);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::accent_button(ui, &format!("Trust {}", name(repo)), egui::vec2(130.0, 34.0)).clicked() {
                                outcome = Outcome::Run(vec![Action::TrustFolder(repo.clone())]);
                            }
                            if ui.add_sized([110.0, 34.0], egui::Button::new("Not now")).clicked() {
                                outcome = Outcome::Close;
                            }
                        });
                    });
                }
                Dialog::ConfirmHost { title: t, body, confirm, danger, job } => {
                    let glyph = if *danger { icon::WARNING } else { icon::GIT_MERGE };
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(glyph).size(theme::ICON_TITLE).color(if *danger { p.danger } else { p.accent }));
                        ui.label(RichText::new(t.as_str()).font(FontId::new(theme::TITLE_SIZE, theme::semibold())));
                    });
                    ui.add_space(10.0);
                    ui.label(RichText::new(body.as_str()).color(p.text_dim));
                    ui.add_space(16.0);
                    match confirm_buttons(ui, confirm, *danger) {
                        Some(true) => outcome = Outcome::Run(vec![Action::Host(job.clone())]),
                        Some(false) => outcome = Outcome::Close,
                        None => {}
                    }
                }
                Dialog::Credential { prompt, answer, reveal } => {
                    outcome = credential(ui, prompt, answer, reveal, enter);
                }
                Dialog::Update(a) => {
                    outcome = update::update(ui, a, updater);
                }
                Dialog::WhatsNew { version, notes } => {
                    title(ui, icon::SPARKLE, &format!("What's new in Canopy {version}"));
                    egui::Frame::new().fill(p.panel).stroke(egui::Stroke::new(1.0, p.border)).corner_radius(egui::CornerRadius::same(theme::RADIUS)).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                        ui.set_max_width(ui.available_width());
                        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| crate::ui::notes::markdown(ui, notes));
                    });
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::primary_button(ui, "Continue", true, 120.0).clicked() || enter {
                                outcome = Outcome::Close;
                            }
                            if ui.add_sized([150.0, 34.0], egui::Button::new(format!("{}  All releases", icon::ARROW_SQUARE_OUT))).clicked() {
                                outcome = Outcome::Run(vec![Action::OpenUrl(crate::updater::RELEASES_PAGE.to_owned())]);
                            }
                        });
                    });
                }
                Dialog::Crash(report) => {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(icon::FIRST_AID_KIT).size(theme::ICON_TITLE).color(p.warning));
                        ui.label(RichText::new("Canopy closed unexpectedly").font(FontId::new(theme::TITLE_SIZE, theme::semibold())));
                    });
                    ui.add_space(10.0);
                    ui.label(RichText::new("Sorry about that. A crash report was saved on this computer; nothing was sent anywhere. Sharing it with the developers helps fix the problem.").color(p.text_dim));
                    if !report.message.is_empty() {
                        ui.add_space(8.0);
                        egui::Frame::new().fill(p.panel).stroke(egui::Stroke::new(1.0, p.border)).corner_radius(egui::CornerRadius::same(theme::RADIUS)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                            ui.set_max_width(ui.available_width());
                            ui.label(RichText::new(&report.message).font(FontId::monospace(theme::MONO_SIZE)).color(p.text));
                        });
                    }
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::primary_button(ui, "Open report", true, 130.0).clicked() {
                                outcome = Outcome::Run(vec![Action::OpenInEditor(report.path.clone())]);
                            }
                            if ui.add_sized([90.0, 34.0], egui::Button::new("Dismiss")).clicked() {
                                outcome = Outcome::Close;
                            }
                            if let Some(dir) = report.path.parent()
                                && ui.add_sized([120.0, 34.0], egui::Button::new(format!("{}  Show folder", icon::FOLDER))).clicked()
                            {
                                outcome = Outcome::Run(vec![Action::OpenPath(dir.to_path_buf())]);
                            }
                        });
                    });
                }
                Dialog::Consent { step, updated, reading } => {
                    let read_buttons = |ui: &mut Ui, reading: &mut Option<Document>| {
                        ui.horizontal(|ui| {
                            for doc in [Document::Terms, Document::Privacy] {
                                let open = *reading == Some(doc);
                                let label = format!("{}  {}", if open { icon::CARET_DOWN } else { icon::CARET_RIGHT }, doc.title());
                                if ui.selectable_label(open, label).clicked() {
                                    *reading = if open { None } else { Some(doc) };
                                }
                            }
                        });
                        if let Some(doc) = *reading {
                            ui.add_space(6.0);
                            egui::Frame::new().fill(p.panel).stroke(egui::Stroke::new(1.0, p.border)).corner_radius(egui::CornerRadius::same(theme::RADIUS)).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                                ui.set_max_width(ui.available_width());
                                egui::ScrollArea::vertical().id_salt(("legal", doc.title())).max_height(260.0).show(ui, |ui| crate::ui::notes::markdown(ui, doc.body()));
                            });
                        }
                    };
                    match step {
                        Pending::Terms => {
                            ui.horizontal(|ui| {
                                art::show(ui, art::logo(), egui::vec2(28.0, 28.0));
                                ui.label(RichText::new(if *updated { "We've updated our terms" } else { "Welcome to Canopy" }).font(FontId::new(theme::HEADLINE_SIZE, theme::semibold())));
                            });
                            ui.add_space(10.0);
                            ui.label(RichText::new(if *updated {
                                "Please read and accept the new Terms of Service and Privacy Policy to keep using Canopy."
                            } else {
                                "Before you start, please read and accept the Terms of Service and the Privacy Policy."
                            }).color(p.text_dim));
                            ui.add_space(6.0);
                            for (glyph, text) in [
                                (icon::LOCK_SIMPLE, "Your repositories and tokens are never sent to NecturaLabs. Requests to GitHub and GitLab go straight to them."),
                                (icon::ARROW_CIRCLE_UP, "At most once a day, Canopy checks GitHub for a new version. You can turn this off."),
                                (icon::FIRST_AID_KIT, "Crash reports are sent only if you agree. You choose next, and can change it any time."),
                            ] {
                                bullet(ui, glyph, p.accent, text);
                            }
                            ui.add_space(8.0);
                            read_buttons(ui, reading);

                        }
                        Pending::CrashReports => {
                            title(ui, icon::FIRST_AID_KIT, "Send crash reports?");
                            ui.label(RichText::new("If Canopy crashes, it can send a report to NecturaLabs so we can fix the problem. It also sends a short record of each session: when it started, how long it ran and how it ended.").color(p.text_dim));
                            ui.add_space(6.0);
                            for (glyph, text, color) in [
                                (icon::CHECK, "A report holds the error message, a stack trace, your Canopy version and your operating system.", p.success),
                                (icon::SHIELD_CHECK, "Canopy does not add your name, tokens, logs or repository files. An error message can name a file or branch.", p.accent),
                                (icon::GLOBE_HEMISPHERE_WEST, "Reports are stored in the EU (Germany) by Sentry and deleted after 90 days.", p.accent2),
                            ] {
                                bullet(ui, glyph, color, text);
                            }
                            ui.add_space(6.0);
                            ui.label(RichText::new("Canopy works the same either way. You can change this at any time in Settings → Privacy.").size(theme::META_SIZE).color(p.text_faint));
                            ui.add_space(8.0);
                            let mut privacy_only = reading.filter(|d| *d == Document::Privacy);
                            ui.horizontal(|ui| {
                                let open = privacy_only.is_some();
                                if ui.selectable_label(open, format!("{}  Privacy Policy", if open { icon::CARET_DOWN } else { icon::CARET_RIGHT })).clicked() {
                                    privacy_only = if open { None } else { Some(Document::Privacy) };
                                }
                            });
                            *reading = privacy_only;
                            if reading.is_some() {
                                ui.add_space(6.0);
                                egui::Frame::new().fill(p.panel).stroke(egui::Stroke::new(1.0, p.border)).corner_radius(egui::CornerRadius::same(theme::RADIUS)).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                                    ui.set_max_width(ui.available_width());
                                    egui::ScrollArea::vertical().id_salt("legal-privacy").max_height(240.0).show(ui, |ui| crate::ui::notes::markdown(ui, Document::Privacy.body()));
                                });
                            }

                        }
                    }
                }
                Dialog::Legal(doc) => {
                    let glyph = match doc {
                        Document::Privacy => icon::SHIELD_CHECK,
                        Document::Terms => icon::SCROLL,
                    };
                    title(ui, glyph, doc.title());
                    egui::Frame::new().fill(p.panel).stroke(egui::Stroke::new(1.0, p.border)).corner_radius(egui::CornerRadius::same(theme::RADIUS)).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                        ui.set_max_width(ui.available_width());
                        egui::ScrollArea::vertical().max_height((ctx.content_rect().height() - 180.0).clamp(0.0, 420.0)).show(ui, |ui| crate::ui::notes::markdown(ui, doc.body()));
                    });

                }
                Dialog::About => {
                    ui.vertical_centered(|ui| {
                        art::show(ui, art::logo(), egui::vec2(64.0, 64.0));
                        ui.label(RichText::new("Canopy").font(FontId::new(theme::DISPLAY_SIZE, theme::semibold())));
                        ui.label(RichText::new(crate::build_info::title_label()).color(p.text_dim));
                        ui.label(RichText::new(git_version.strip_prefix("git version ").map_or_else(|| git_version.to_owned(), |v| format!("Git {v}"))).size(theme::META_SIZE).color(p.text_faint));
                        ui.add_space(8.0);
                        ui.label(RichText::new("A fast, beautiful, Linux-first Git client.").color(p.text_dim));
                        ui.label(RichText::new("© 2026 NecturaLabs.").size(theme::META_SIZE).color(p.text_faint));
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
                                outcome = Outcome::Run(vec![Action::OpenUrl(crate::updater::RELEASES_PAGE.to_owned())]);
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.add_space((ui.available_width() - 300.0).max(0.0) / 2.0);
                            if ui.button(format!("{}  Privacy Policy", icon::SHIELD_CHECK)).clicked() {
                                outcome = Outcome::Run(vec![Action::OpenDialog(Dialog::Legal(Document::Privacy))]);
                            }
                            if ui.button(format!("{}  Terms of Service", icon::SCROLL)).clicked() {
                                outcome = Outcome::Run(vec![Action::OpenDialog(Dialog::Legal(Document::Terms))]);
                            }
                            if ui.button("Close").clicked() {
                                outcome = Outcome::Close;
                            }
                        });
                    });
                }
            }
            ui.add_space(theme::SM);
            });
            if document_actions && !pinned_actions {
                outcome = common::document_actions(ui, dialog);
            }
        });
    if modal.should_close()
        && matches!(outcome, Outcome::Keep)
        && !matches!(dialog, Dialog::Consent { .. } | Dialog::Credential { .. })
    {
        outcome = Outcome::Close;
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::{Dialog, DialogStack, Oid, Start, valid_ref_name};
    use crate::askpass::Prompt;
    use crate::ui::legal::{Document, Pending};
    use crossbeam_channel::Receiver;

    fn consent() -> Dialog {
        Dialog::Consent {
            step: Pending::Terms,
            updated: false,
            reading: None,
        }
    }

    fn credential(text: &str) -> (Dialog, Receiver<Option<String>>) {
        let (prompt, answers) = Prompt::with_reply(text);
        (Dialog::credential(prompt), answers)
    }

    /// A short name for the dialog on screen.
    fn shown(stack: &DialogStack) -> String {
        match stack.current() {
            None => "none".into(),
            Some(Dialog::About) => "about".into(),
            Some(Dialog::Consent { .. }) => "consent".into(),
            Some(Dialog::Legal(_)) => "legal".into(),
            Some(Dialog::Credential { prompt, .. }) => prompt.text.clone(),
            Some(Dialog::Stash { message, .. }) => message.clone(),
            Some(other) => format!("{other:?}"),
        }
    }

    fn stash(name: &str) -> Dialog {
        Dialog::Stash {
            message: name.into(),
            untracked: false,
        }
    }

    #[test]
    fn ordinary_dialogs_stack_newest_first_and_none_is_lost() {
        let mut s = DialogStack::default();
        assert_eq!(shown(&s), "none");
        s.open(stash("a"));
        s.open(stash("b"));
        assert_eq!(shown(&s), "b");
        s.close();
        assert_eq!(shown(&s), "a");
        s.dismiss();
        assert_eq!(shown(&s), "none");
        s.close();
        s.dismiss();
        assert_eq!(shown(&s), "none");
    }

    #[test]
    fn consent_cannot_be_replaced_or_dismissed() {
        let mut s = DialogStack::default();
        s.open(stash("notice"));
        s.open(consent());
        s.open(Dialog::About);
        assert_eq!(shown(&s), "consent");
        s.dismiss();
        assert_eq!(shown(&s), "consent");
        s.close_where(|d| matches!(d, Dialog::Consent { .. }));
        assert_eq!(shown(&s), "about");
        s.close();
        assert_eq!(shown(&s), "notice");
    }

    #[test]
    fn legal_documents_are_not_replaced() {
        let mut s = DialogStack::default();
        s.open(Dialog::Legal(Document::Privacy));
        s.open(Dialog::About);
        assert_eq!(shown(&s), "legal");
        s.close();
        assert_eq!(shown(&s), "about");
    }

    #[test]
    fn credentials_go_in_front_one_at_a_time_in_arrival_order() {
        let mut s = DialogStack::default();
        s.open(consent());
        let (first, first_answers) = credential("first");
        let (second, _second_answers) = credential("second");
        s.open(first);
        assert_eq!(shown(&s), "first", "a prompt goes in front of the consent");
        s.open(second);
        assert_eq!(shown(&s), "first", "a second prompt waits behind the first");
        s.open(Dialog::About);
        s.open(consent());
        assert_eq!(shown(&s), "first", "nothing hides a prompt");
        s.close();
        assert_eq!(first_answers.try_recv().ok(), Some(None), "closing cancels");
        assert_eq!(shown(&s), "second");
        s.close();
        assert_eq!(shown(&s), "consent");
    }

    #[test]
    fn a_closed_prompt_is_always_answered() {
        let mut s = DialogStack::default();
        // Answered in the dialog, then closed: the answer stands.
        let (d, answers) = credential("Password: ");
        if let Dialog::Credential { prompt, .. } = &d {
            prompt.answer(Some("hunter2".into()));
        }
        s.open(d);
        s.close();
        assert_eq!(answers.try_recv().ok(), Some(Some("hunter2".into())));
        // Escape on a host key says no; on a password it cancels.
        let (host, host_answers) = credential("Continue connecting (yes/no/[fingerprint])? ");
        s.open(host);
        s.dismiss();
        assert_eq!(host_answers.try_recv().ok(), Some(Some("no".into())));
        let (pw, pw_answers) = credential("Password: ");
        s.open(pw);
        s.dismiss();
        assert_eq!(pw_answers.try_recv().ok(), Some(None));
        // Closed from behind another dialog.
        let (queued, queued_answers) = credential("Username: ");
        let (front, _front_answers) = credential("Password: ");
        s.open(front);
        s.open(queued);
        s.close_where(
            |d| matches!(d, Dialog::Credential { prompt, .. } if prompt.text == "Username: "),
        );
        assert_eq!(queued_answers.try_recv().ok(), Some(None));
        assert_eq!(shown(&s), "Password: ");
    }

    #[test]
    fn withdrawn_prompts_leave_the_stack() {
        let mut s = DialogStack::default();
        s.open(stash("under"));
        let (d, _answers) = credential("Password: ");
        let prompt = match &d {
            Dialog::Credential { prompt, .. } => prompt.clone(),
            _ => unreachable!(),
        };
        s.open(d);
        s.drop_withdrawn();
        assert_eq!(shown(&s), "Password: ");
        prompt.withdraw();
        s.drop_withdrawn();
        assert_eq!(shown(&s), "under");
    }

    #[test]
    fn branch_starts_are_typed_and_shortened_safely() -> Result<(), git2::Error> {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        let oid = Oid::from_str(sha)?;
        assert_eq!(Start::Commit(oid).label(String::new), "commit 0123456");
        assert_eq!(Start::Commit(oid).rev().as_deref(), Some(sha));
        // Names, however long or short and whatever their characters, are shown whole.
        for name in [
            "main",
            "v1",
            "feature/ünïcödé-forty-bytes-long-name",
            "release/2026-10-01-hotfix-for-the-thing",
            "0123456789abcdef",
        ] {
            let start = Start::Branch(name.to_owned());
            assert_eq!(start.label(String::new), name);
            assert_eq!(start.rev().as_deref(), Some(name));
        }
        assert_eq!(Start::Head.label(|| "main".into()), "main");
        assert_eq!(Start::Head.rev(), None);
        assert!(matches!(
            Dialog::new_branch_at(Start::Head),
            Dialog::NewBranch {
                start: Start::Head,
                ..
            }
        ));
        assert!(matches!(
            Dialog::new_branch_at(Start::Commit(oid)),
            Dialog::NewBranch { start: Start::Commit(o), .. } if o == oid
        ));
        Ok(())
    }

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
    #[test]
    fn clone_submission_keeps_its_form_until_the_worker_answers() {
        use super::{Key, Outcome, Settings, show};
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        let mut dialog = Dialog::clone_default(&Settings::default());
        if let Dialog::Clone {
            builder, parent, ..
        } = &mut dialog
        {
            builder.project = "example/repository".into();
            *parent = "existing-parent".into();
        }
        let mut picker = crate::ui::folder_picker::FolderPicker::default();
        let mut kept = false;
        for pressed in [false, true] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: if pressed {
                        vec![egui::Event::Key {
                            key: Key::Enter,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: Default::default(),
                        }]
                    } else {
                        Vec::new()
                    },
                    ..Default::default()
                },
                |_| {
                    let result = show(
                        &ctx,
                        &mut dialog,
                        None,
                        &mut picker,
                        false,
                        &crate::updater::Updater::default(),
                        "git version 2.55.0",
                    );
                    if pressed {
                        kept = matches!(result, Outcome::RunKeep(_));
                    }
                },
            );
            output.textures_delta.clear();
        }
        assert!(
            kept,
            "clone input must remain available after a destination refusal"
        );
    }
    #[test]
    fn clone_completion_preserves_failed_input_and_only_closes_its_pending_form() {
        let mut stack = DialogStack::default();
        let mut pending = Dialog::clone_default(&crate::settings::Settings::default());
        if let Dialog::Clone {
            builder,
            parent,
            pending,
        } = &mut pending
        {
            builder.project = "example/repository".into();
            *parent = "chosen-parent".into();
            *pending = true;
        }
        stack.open(pending);
        stack.open(Dialog::Legal(Document::Privacy));
        stack.finish_clone(false);
        stack.close();
        assert!(
            matches!(stack.current(), Some(Dialog::Clone { builder, parent, pending: false }) if builder.project == "example/repository" && parent == "chosen-parent")
        );
        if let Some(Dialog::Clone { pending, .. }) = stack.current_mut() {
            *pending = true;
        }
        stack.open(Dialog::clone_default(&crate::settings::Settings::default()));
        stack.finish_clone(true);
        stack.close();
        assert!(
            stack.current().is_none(),
            "only the completed pending form closes"
        );
    }
    #[test]
    fn update_progress_and_completion_dialogs_fit_their_content() {
        use super::show;
        use crate::selfupdate::{Installed, Phase};
        use crate::updater::{Available, InstallState, Updater};

        let available = Available {
            build: 1001,
            manifest: canopy_hosting::updates::UpdateManifest {
                version: "1001".into(),
                notes: String::new(),
                pub_date: None,
                release_page: None,
                platforms: Default::default(),
            },
        };
        for state in [
            InstallState::Running {
                build: 1001,
                phase: Phase::Installing,
            },
            InstallState::Done {
                build: 1001,
                installed: Installed {
                    exe: Some("canopy".into()),
                    notes: Vec::new(),
                },
            },
        ] {
            let ctx = egui::Context::default();
            crate::ui::fonts::install(&ctx);
            let mut updater = Updater::default();
            updater.install = state;
            let mut dialog = Dialog::Update(Box::new(available.clone()));
            let mut picker = crate::ui::folder_picker::FolderPicker::default();
            for _ in 0..5 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1440.0, 900.0),
                        )),
                        ..Default::default()
                    },
                    |_| {
                        show(
                            &ctx,
                            &mut dialog,
                            None,
                            &mut picker,
                            false,
                            &updater,
                            "git version 2.55.0",
                        );
                    },
                );
                output.textures_delta.clear();
            }
            let bounds = egui::containers::AreaState::load(&ctx, egui::Id::new("dialog"))
                .expect("update dialog area")
                .rect();
            assert!(
                bounds.height() < 300.0,
                "update actions must stay beside their content, not consume the viewport: {bounds:?}"
            );
        }
    }

    #[test]
    fn clone_form_title_and_actions_are_reachable_at_minimum_window_scale() {
        use super::{Settings, show};
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        crate::ui::theme::apply(&ctx, crate::ui::theme::ThemeId::Midnight, true);
        let mut dialog = Dialog::clone_default(&Settings::default());
        if let Dialog::Clone { builder, .. } = &mut dialog {
            builder.project = "example/repository".into();
        }
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(562.5, 350.0));
        let mut picker = crate::ui::folder_picker::FolderPicker::default();
        for scrolling in [false, true] {
            let mut visible = false;
            for frame in 0..15 {
                let mut events = vec![egui::Event::PointerMoved(viewport.center())];
                if scrolling && frame == 0 {
                    events.push(egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        phase: egui::TouchPhase::Move,
                        delta: egui::vec2(0.0, -1000.0),
                        modifiers: Default::default(),
                    });
                }
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(viewport),
                        events,
                        ..Default::default()
                    },
                    |_| {
                        show(
                            &ctx,
                            &mut dialog,
                            None,
                            &mut picker,
                            false,
                            &crate::updater::Updater::default(),
                            "git version 2.55.0",
                        );
                    },
                );
                output.textures_delta.clear();
                let label = if scrolling {
                    "Clone"
                } else {
                    "Clone repository"
                };
                visible = output.shapes.iter().any(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape
                        && text.galley.text() == label
                    {
                        let bounds = text.visual_bounding_rect();
                        return viewport.contains_rect(bounds)
                            && shape.clip_rect.contains_rect(bounds);
                    }
                    false
                });
            }
            let bounds = egui::containers::AreaState::load(&ctx, egui::Id::new("dialog"))
                .expect("clone dialog area")
                .rect();
            assert!(
                viewport.contains_rect(bounds),
                "dialog frame must fit the viewport: {bounds:?}"
            );
            assert!(
                visible,
                "Clone title and scrolled action must fit the minimum scaled window (scrolling={scrolling})"
            );
        }
    }
    #[test]
    fn consent_choices_are_visible_without_scrolling_at_maximum_zoom() {
        for step in [Pending::Terms, Pending::CrashReports] {
            let ctx = egui::Context::default();
            crate::ui::fonts::install(&ctx);
            crate::ui::theme::apply(&ctx, crate::ui::theme::ThemeId::Daylight, true);
            let mut dialog = Dialog::Consent {
                step,
                updated: false,
                reading: Some(Document::Privacy),
            };
            let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(562.5, 350.0));
            let mut picker = crate::ui::folder_picker::FolderPicker::default();
            let mut visible = 0;
            for _ in 0..4 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(viewport),
                        ..Default::default()
                    },
                    |_| {
                        super::show(
                            &ctx,
                            &mut dialog,
                            None,
                            &mut picker,
                            false,
                            &crate::updater::Updater::default(),
                            "git version 2.55.0",
                        );
                    },
                );
                output.textures_delta.clear();
                let choices = match step {
                    Pending::Terms => ["I agree", "Quit"],
                    Pending::CrashReports => ["Send crash reports", "Don't send"],
                };
                visible = output
                    .shapes
                    .iter()
                    .filter(|shape| {
                        if let egui::Shape::Text(text) = &shape.shape
                            && choices.contains(&text.galley.text())
                        {
                            let rect = text.visual_bounding_rect();
                            viewport.contains_rect(rect) && shape.clip_rect.contains_rect(rect)
                        } else {
                            false
                        }
                    })
                    .count();
            }
            assert_eq!(
                visible, 2,
                "both consent choices must stay visible while reading"
            );
        }
    }
}
