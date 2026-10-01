//! Modal dialogs. Each returns the actions to run when the user confirms.

use egui::{FontId, Id, Key, RichText, Ui};
use egui_phosphor::regular as icon;

use canopy_core::{Oid, short_id};

use crate::app::Action;
use crate::tab::RepoTab;
use crate::ui::theme::{self, palette};
use crate::ui::widgets;
use crate::worker::Op;

/// Every dialog Canopy shows.
#[derive(Debug, Clone)]
pub enum Dialog {
    Open {
        path: String,
    },
    Clone {
        url: String,
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
    },
    AddRemote {
        name: String,
        url: String,
    },
    Confirm {
        title: String,
        body: String,
        confirm: String,
        danger: bool,
        op: Op,
    },
    Settings,
    About,
}

/// What a dialog wants after this frame.
pub enum Outcome {
    Keep,
    Close,
    Run(Vec<Action>),
}

fn home() -> String {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or_default()
}

impl Dialog {
    /// The open-repository dialog, prefilled with the home directory.
    pub fn open() -> Self {
        Dialog::Open { path: home() }
    }

    /// The clone dialog with `~/src` (or home) as the destination.
    pub fn clone_default() -> Self {
        let h = home();
        let src = std::path::Path::new(&h).join("src");
        let parent = if src.is_dir() {
            src.display().to_string()
        } else {
            h
        };
        Dialog::Clone {
            url: String::new(),
            parent,
        }
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
                Dialog::Clone { url, parent } => {
                    title(ui, icon::CLOUD_ARROW_DOWN, "Clone repository");
                    field(ui, "Repository URL", url, "https://github.com/owner/repo.git or git@host:owner/repo.git", true);
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
                Dialog::Merge { target, rebase } => {
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
                    ui.add_space(6.0);
                    if !target.is_empty() {
                        let text = if *rebase { format!("Replay {current}'s commits on top of {target}.") } else { format!("Merge {target} into {current}.") };
                        ui.label(RichText::new(text).size(12.0).color(p.text_faint));
                    }
                    ui.add_space(8.0);
                    if primary(ui, if *rebase { "Rebase" } else { "Merge" }, !target.is_empty()) {
                        let op = if *rebase { Op::Rebase(target.clone()) } else { Op::Merge(target.clone()) };
                        outcome = Outcome::Run(vec![Action::Op(op)]);
                    }
                }
                Dialog::AddRemote { name, url } => {
                    title(ui, icon::CLOUD, "Add remote");
                    field(ui, "Name", name, "origin", false);
                    field(ui, "URL", url, "git@github.com:owner/repo.git", true);
                    let ok = valid_ref_name(name.trim()) && !url.trim().is_empty();
                    if primary(ui, "Add remote", ok) {
                        outcome = Outcome::Run(vec![Action::Op(Op::AddRemote { name: name.trim().to_owned(), url: url.trim().to_owned() })]);
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
                            let b = egui::Button::new(RichText::new(confirm.as_str()).color(egui::Color32::WHITE).strong()).fill(if *danger { p.danger } else { p.accent }).min_size(egui::vec2(120.0, 34.0));
                            if ui.add(b).clicked() {
                                outcome = Outcome::Run(vec![Action::Op(op.clone())]);
                            }
                            if ui.add_sized([90.0, 34.0], egui::Button::new("Cancel")).clicked() {
                                outcome = Outcome::Close;
                            }
                        });
                    });
                }
                // Rendered by the app, which owns the settings.
                Dialog::Settings => {}
                Dialog::About => {
                    ui.vertical_centered(|ui| {
                        let (r, _) = ui.allocate_exact_size(egui::vec2(64.0, 64.0), egui::Sense::hover());
                        widgets::paint_logo(ui.painter(), r.center(), 56.0, p);
                        ui.label(RichText::new("Canopy").font(FontId::new(24.0, theme::semibold())));
                        ui.label(RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).color(p.text_dim));
                        ui.add_space(8.0);
                        ui.label(RichText::new("A fast, beautiful, Linux-first Git client.").color(p.text_dim));
                        ui.label(RichText::new("© 2026 NecturaLabs. Built with Rust, egui and libgit2.").size(12.0).color(p.text_faint));
                        ui.label(RichText::new("Fonts: Inter and JetBrains Mono (SIL OFL 1.1). Icons: Phosphor (MIT).").size(12.0).color(p.text_faint));
                        ui.add_space(10.0);
                        if ui.button("Close").clicked() {
                            outcome = Outcome::Close;
                        }
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
