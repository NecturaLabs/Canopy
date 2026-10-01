//! Modal dialogs. Each returns the actions to run when the user confirms.

use egui::{FontId, Id, Key, RichText, Ui};
use egui_phosphor::regular as icon;

use canopy_core::forge::{self, ForgeKind};
use canopy_core::{Oid, short_id};
use canopy_hosting::NewRequest;

use crate::app::Action;
use crate::askpass::{Prompt, PromptKind};
use crate::hosting::HostJob;
use crate::settings::{CloneProtocol, Settings};
use crate::tab::RepoTab;
use crate::ui::legal::{Document, Pending};
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

/// Where a new branch starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Start {
    /// The current `HEAD`.
    Head,
    /// A commit, e.g. from the history's context menu.
    Commit(Oid),
    /// A branch (or another revision) by name.
    Branch(String),
}

impl Start {
    /// The revision to hand to git; `None` for `HEAD`.
    pub fn rev(&self) -> Option<String> {
        match self {
            Start::Head => None,
            Start::Commit(oid) => Some(oid.to_string()),
            Start::Branch(name) => Some(name.clone()),
        }
    }

    /// How the dialog names it: `commit 1a2b3c4`, the branch name, or `head()` for `HEAD`.
    fn label(&self, head: impl FnOnce() -> String) -> String {
        match self {
            Start::Head => head(),
            Start::Commit(oid) => format!("commit {}", short_id(*oid)),
            Start::Branch(name) => name.clone(),
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
}

impl Dialog {
    /// How strongly the dialog holds the screen. A dialog never hides one that ranks higher: it
    /// waits behind it instead.
    ///
    /// - 2: git's and ssh's credential questions. Git is blocked until they are answered, so
    ///   they go in front of everything, one at a time in the order they arrived.
    /// - 1: the start-up consent, which must be answered, and the legal documents.
    /// - 0: everything else; the newest shows first.
    fn rank(&self) -> u8 {
        match self {
            Dialog::Credential { .. } => 2,
            Dialog::Consent { .. } | Dialog::Legal(_) => 1,
            _ => 0,
        }
    }
}

/// The dialog on screen and those waiting behind it. Opening never throws a dialog away, and a
/// credential prompt that leaves the stack, however it leaves, always gets an answer.
#[derive(Debug, Default)]
pub struct DialogStack {
    /// Bottom first; the last one is on screen.
    dialogs: Vec<Dialog>,
}

impl DialogStack {
    /// Shows `dialog`, or queues it behind the dialogs that outrank it (see [`Dialog::rank`]).
    pub fn open(&mut self, dialog: Dialog) {
        let rank = dialog.rank();
        // Credential prompts queue in arrival order behind the one on screen; other dialogs go
        // on top of their equals.
        let waits_behind = |d: &Dialog| d.rank() > rank || (rank == 2 && d.rank() == 2);
        let mut at = self.dialogs.len();
        while at > 0 && waits_behind(&self.dialogs[at - 1]) {
            at -= 1;
        }
        self.dialogs.insert(at, dialog);
    }

    /// The dialog on screen.
    pub fn current(&self) -> Option<&Dialog> {
        self.dialogs.last()
    }

    pub fn current_mut(&mut self) -> Option<&mut Dialog> {
        self.dialogs.last_mut()
    }

    /// Whether any open or waiting dialog matches.
    pub fn any(&self, f: impl Fn(&Dialog) -> bool) -> bool {
        self.dialogs.iter().any(f)
    }

    /// Closes the dialog on screen; the next one shows. A credential prompt not answered yet is
    /// cancelled.
    pub fn close(&mut self) {
        if let Some(d) = self.dialogs.pop() {
            cancel(&d);
        }
    }

    /// Escape: closes the dialog on screen, except the consent, which must be answered. A
    /// credential prompt is declined (`no` to an unknown host key).
    pub fn dismiss(&mut self) {
        match self.dialogs.last() {
            Some(Dialog::Consent { .. }) | None => {}
            Some(Dialog::Credential { prompt, .. }) => {
                prompt.decline();
                self.close();
            }
            Some(_) => self.close(),
        }
    }

    /// Closes every dialog that matches, wherever it is (credential prompts are cancelled).
    pub fn close_where(&mut self, f: impl Fn(&Dialog) -> bool) {
        self.dialogs.retain(|d| {
            let close = f(d);
            if close {
                cancel(d);
            }
            !close
        });
    }

    /// Closes the credential prompts git or ssh stopped waiting for.
    pub fn drop_withdrawn(&mut self) {
        self.close_where(
            |d| matches!(d, Dialog::Credential { prompt, .. } if !prompt.is_waiting()),
        );
    }
}

/// Cancels `dialog`'s credential prompt if it has one; a prompt already answered keeps its
/// answer.
fn cancel(dialog: &Dialog) {
    if let Dialog::Credential { prompt, .. } = dialog {
        prompt.answer(None);
    }
}

fn home() -> String {
    crate::settings::home_dir().display().to_string()
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

/// Cancel and confirm buttons with keyboard hints. The safe choice has the initial focus: Cancel
/// when the action destroys something, otherwise the action, so Enter does what is least
/// surprising. `Some(true)` confirms, `Some(false)` cancels.
fn confirm_buttons(ui: &mut Ui, confirm: &str, danger: bool) -> Option<bool> {
    let p = palette(ui.ctx());
    let mut answer = None;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(if danger {
                "Enter cancels · Esc closes"
            } else {
                "Enter confirms · Esc cancels"
            })
            .size(12.0)
            .color(p.text_faint),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let size = egui::vec2(120.0, 34.0);
            let yes = if danger {
                widgets::danger_button(ui, confirm, size)
            } else {
                widgets::accent_button(ui, confirm, size)
            };
            let no = ui.add_sized([90.0, 34.0], egui::Button::new("Cancel"));
            if ui.memory(|m| m.focused().is_none()) {
                if danger {
                    no.request_focus()
                } else {
                    yes.request_focus()
                }
            }
            if yes.clicked() {
                answer = Some(true);
            }
            if no.clicked() {
                answer = Some(false);
            }
        });
    });
    answer
}

/// The credential prompt: the question as git or ssh asked it, and the answer.
fn credential(
    ui: &mut Ui,
    prompt: &Prompt,
    answer: &mut String,
    reveal: &mut bool,
    enter: bool,
) -> Outcome {
    let p = palette(ui.ctx());
    let lower = prompt.text.to_lowercase();
    let (glyph, color, heading) = match prompt.kind {
        PromptKind::Secret if lower.contains("passphrase") => {
            (icon::KEY, p.accent, "SSH key passphrase")
        }
        PromptKind::Secret => (icon::KEY, p.accent, "Password required"),
        PromptKind::Text => (icon::USER, p.accent, "Sign in"),
        PromptKind::YesNo => (icon::SHIELD_WARNING, p.warning, "Trust this host?"),
        PromptKind::Approve | PromptKind::Notice => (icon::SHIELD_CHECK, p.accent, "Confirm"),
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new(glyph).size(20.0).color(color));
        ui.label(RichText::new(heading).font(FontId::new(17.0, theme::semibold())));
    });
    ui.add_space(10.0);
    let asker = if lower.contains("passphrase for key")
        || lower.contains("authenticity of host")
        || lower.contains("host key")
    {
        "SSH asks:"
    } else {
        "Git asks:"
    };
    ui.label(RichText::new(asker).size(12.0).color(p.text_dim));
    ui.add_space(4.0);
    egui::Frame::new()
        .fill(p.panel)
        .stroke(egui::Stroke::new(1.0, p.border))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS))
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical()
                .max_height(180.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(&prompt.text)
                                .font(FontId::monospace(12.0))
                                .color(p.text),
                        )
                        .wrap(),
                    );
                });
        });
    ui.add_space(12.0);
    let escape = ui.input(|i| i.key_pressed(Key::Escape));
    let mut outcome = Outcome::Keep;
    let reply = |value: Option<String>| {
        prompt.answer(value);
        Outcome::Close
    };
    match prompt.kind {
        PromptKind::Secret | PromptKind::Text => {
            let secret = prompt.kind == PromptKind::Secret;
            ui.horizontal(|ui| {
                let width = ui.available_width() - if secret { 40.0 } else { 0.0 };
                let r = ui.add(
                    egui::TextEdit::singleline(answer)
                        .password(secret && !*reveal)
                        .desired_width(width)
                        .margin(egui::Margin::symmetric(8, 6)),
                );
                if ui.memory(|m| m.focused().is_none()) {
                    r.request_focus();
                }
                if secret {
                    let (eye, tip) = if *reveal {
                        (icon::EYE_SLASH, "Hide")
                    } else {
                        (icon::EYE, "Show")
                    };
                    if ui
                        .add_sized([32.0, 30.0], egui::Button::new(eye))
                        .on_hover_text(tip)
                        .clicked()
                    {
                        *reveal = !*reveal;
                    }
                }
            });
            ui.add_space(4.0);
            ui.label(RichText::new("Canopy hands this to git and does not keep it. Your git credential helper may save it.").size(12.0).color(p.text_faint));
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let ok = !answer.is_empty() || secret;
                    if widgets::primary_button(ui, "Continue", ok, 130.0).clicked() || (enter && ok)
                    {
                        outcome = reply(Some(std::mem::take(answer)));
                    }
                    if ui
                        .add_sized([90.0, 34.0], egui::Button::new("Cancel"))
                        .clicked()
                        || escape
                    {
                        outcome = reply(None);
                    }
                });
            });
        }
        PromptKind::YesNo | PromptKind::Approve | PromptKind::Notice => {
            if prompt.kind == PromptKind::YesNo {
                ui.label(RichText::new("Only trust the host if its fingerprint matches the one its owner publishes (GitHub and GitLab list theirs in their documentation).").size(12.5).color(p.text_dim));
                ui.add_space(14.0);
            }
            let (yes, no, yes_answer) = if prompt.kind == PromptKind::YesNo {
                ("Trust host", "Don't trust", "yes")
            } else {
                ("Allow", "Deny", "")
            };
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::accent_button(ui, yes, egui::vec2(130.0, 34.0)).clicked() {
                        outcome = reply(Some(yes_answer.to_owned()));
                    }
                    if ui.add_sized([110.0, 34.0], egui::Button::new(no)).clicked() || escape {
                        prompt.decline();
                        outcome = Outcome::Close;
                    }
                });
            });
        }
    }
    outcome
}

/// An icon and a line of text that wraps within the dialog, aligned under itself.
fn bullet(ui: &mut Ui, glyph: &str, color: egui::Color32, text: &str) {
    let p = palette(ui.ctx());
    ui.horizontal_top(|ui| {
        ui.label(RichText::new(glyph).color(color));
        ui.add(egui::Label::new(RichText::new(text).size(12.5).color(p.text)).wrap());
    });
    ui.add_space(2.0);
}

fn multiline_key() -> Id {
    Id::new("dialog-multiline")
}

/// Records the dialog's multi-line field so Enter there inserts a newline instead of submitting.
fn remember_multiline(ui: &Ui, id: Id) {
    ui.ctx().data_mut(|d| d.insert_temp(multiline_key(), id));
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
        .backdrop_color(p.backdrop())
        .show(ctx, |ui| {
            ui.set_width(460.0);
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
                    let dest = crate::settings::expand_home(parent).join(&name);
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
                        outcome = Outcome::Run(vec![Action::Init(crate::settings::expand_home(path))]);
                    }
                }
                Dialog::NewBranch { name, start, checkout } => {
                    title(ui, icon::GIT_BRANCH, "New branch");
                    let r = field(ui, "Name", name, "feature/something", true);
                    let _ = r;
                    let from = start.label(|| tab.and_then(|t| t.snap.as_ref()).map(|s| s.head.label()).unwrap_or_else(|| "HEAD".into()));
                    ui.label(RichText::new(format!("{}  Starts at {from}", icon::GIT_COMMIT)).size(12.0).color(p.text_faint));
                    ui.add_space(6.0);
                    ui.checkbox(checkout, "Check out after creating");
                    ui.add_space(8.0);
                    let ok = valid_ref_name(name.trim());
                    if !name.is_empty() && !ok {
                        ui.label(RichText::new("Not a valid branch name").size(12.0).color(p.danger));
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
                    ui.label(RichText::new("Message (makes an annotated tag)").size(12.0).color(p.text_dim));
                    let r = ui.add(egui::TextEdit::multiline(message).desired_rows(3).desired_width(f32::INFINITY).margin(egui::Margin::symmetric(8, 6)));
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
                    let r = ui.add(egui::TextEdit::multiline(&mut request.body).desired_rows(6).desired_width(f32::INFINITY).hint_text("Why, how it was tested, anything reviewers should know (Markdown)").margin(egui::Margin::symmetric(8, 6)));
                    remember_multiline(ui, r.id);
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
                    match confirm_buttons(ui, confirm, *danger) {
                        Some(true) => outcome = Outcome::Run(vec![Action::Op(op.clone())]),
                        Some(false) => outcome = Outcome::Close,
                        None => {}
                    }
                }
                Dialog::ConfirmHost { title: t, body, confirm, danger, job } => {
                    let glyph = if *danger { icon::WARNING } else { icon::GIT_MERGE };
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(glyph).size(20.0).color(if *danger { p.danger } else { p.accent }));
                        ui.label(RichText::new(t.as_str()).font(FontId::new(17.0, theme::semibold())));
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
                                outcome = Outcome::Run(vec![Action::OpenUrl(a.download())]);
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
                                outcome = Outcome::Run(vec![Action::OpenUrl(crate::updater::RELEASES_PAGE.to_owned())]);
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
                                ui.set_width(ui.available_width());
                                egui::ScrollArea::vertical().id_salt(("legal", doc.title())).max_height(260.0).show(ui, |ui| crate::ui::notes::markdown(ui, doc.body()));
                            });
                        }
                    };
                    let equal = |ui: &mut Ui, text: &str| ui.add_sized([170.0, 34.0], egui::Button::new(RichText::new(text).font(FontId::new(13.5, theme::medium()))));
                    match step {
                        Pending::Terms => {
                            ui.horizontal(|ui| {
                                art::show(ui, art::logo(), egui::vec2(28.0, 28.0));
                                ui.label(RichText::new(if *updated { "We've updated our terms" } else { "Welcome to Canopy" }).font(FontId::new(18.0, theme::semibold())));
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
                            ui.add_space(12.0);
                            ui.horizontal(|ui| {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if widgets::primary_button(ui, "I agree", true, 130.0).clicked() {
                                        outcome = Outcome::Run(vec![Action::AcceptTerms]);
                                    }
                                    if ui.add_sized([90.0, 34.0], egui::Button::new("Quit")).clicked() {
                                        outcome = Outcome::Run(vec![Action::Quit]);
                                    }
                                });
                            });
                        }
                        Pending::CrashReports => {
                            title(ui, icon::FIRST_AID_KIT, "Send crash reports?");
                            ui.label(RichText::new("If Canopy crashes, it can send a report to NecturaLabs so we can fix the problem. It also sends a short record of each session: when it started, how long it ran and how it ended.").color(p.text_dim));
                            ui.add_space(6.0);
                            for (glyph, text, color) in [
                                (icon::CHECK, "A report holds the error message, a stack trace, your Canopy version and your operating system.", p.success),
                                (icon::X, "Canopy does not add your name, tokens, logs or repository files. An error message can name a file or branch.", p.danger),
                                (icon::GLOBE_HEMISPHERE_WEST, "Reports are stored in the EU (Germany) by Sentry and deleted after 90 days.", p.accent2),
                            ] {
                                bullet(ui, glyph, color, text);
                            }
                            ui.add_space(6.0);
                            ui.label(RichText::new("Canopy works the same either way. You can change this at any time in Settings → Privacy.").size(12.0).color(p.text_faint));
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
                                    ui.set_width(ui.available_width());
                                    egui::ScrollArea::vertical().id_salt("legal-privacy").max_height(240.0).show(ui, |ui| crate::ui::notes::markdown(ui, Document::Privacy.body()));
                                });
                            }
                            ui.add_space(12.0);
                            // Both answers equally prominent: consent must be a free choice.
                            ui.horizontal(|ui| {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if equal(ui, "Send crash reports").clicked() {
                                        outcome = Outcome::Run(vec![Action::SetCrashReports(true)]);
                                    }
                                    if equal(ui, "Don't send").clicked() {
                                        outcome = Outcome::Run(vec![Action::SetCrashReports(false)]);
                                    }
                                });
                            });
                        }
                    }
                }
                Dialog::Legal(doc) => {
                    let glyph = match doc {
                        Document::Privacy => icon::SHIELD_CHECK,
                        Document::Terms => icon::SCROLL,
                        Document::Notices => icon::BOOK_OPEN,
                    };
                    if *doc == Document::Notices {
                        // Licence texts are laid out for 80 columns.
                        ui.set_width(660.0);
                    }
                    title(ui, glyph, doc.title());
                    egui::Frame::new().fill(p.panel).stroke(egui::Stroke::new(1.0, p.border)).corner_radius(egui::CornerRadius::same(theme::RADIUS)).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        if *doc == Document::Notices {
                            // Thousands of lines: lay out only the visible ones.
                            let font = FontId::monospace(11.5);
                            let row = ui.fonts_mut(|f| f.row_height(&font));
                            static LINES: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
                            let lines = LINES.get_or_init(|| crate::ui::legal::NOTICES.lines().collect());
                            egui::ScrollArea::both().max_height(420.0).auto_shrink([false, true]).show_rows(ui, row, lines.len(), |ui, range| {
                                for line in &lines[range] {
                                    ui.add(egui::Label::new(RichText::new(*line).font(font.clone()).color(p.text_dim)).extend());
                                }
                            });
                        } else {
                            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| crate::ui::notes::markdown(ui, doc.body()));
                        }
                    });
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::primary_button(ui, "Close", true, 110.0).clicked() || enter {
                                outcome = Outcome::Close;
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
                        if ui.link(RichText::new("Open-source licences").size(12.0)).clicked() {
                            outcome = Outcome::Run(vec![Action::OpenDialog(Dialog::Legal(Document::Notices))]);
                        }
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
}
