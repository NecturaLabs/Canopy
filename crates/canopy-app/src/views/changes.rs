//! The staging area: conflicted, unstaged and staged files, the commit composer and the diff.

use egui::{Align2, CornerRadius, FontId, RichText, Sense, Stroke, Ui, vec2};
use egui_phosphor::regular as icon;

use canopy_core::{DiffSource, FileEntry};

use crate::app::Action;
use crate::tab::{FileSelection, RepoTab};
use crate::ui::theme::{self, Palette, palette};
use crate::ui::{art, widgets};
use crate::views::{commit_detail, diff, resolve};
use crate::worker::Op;

#[derive(Clone, Copy, PartialEq, Eq)]
enum List {
    Conflicted,
    Unstaged,
    Staged,
}

/// Draws the changes view.
pub fn show(ui: &mut Ui, tab: &mut RepoTab, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let Some(snap) = tab.snap.clone() else {
        ui.centered_and_justified(|ui| ui.spinner());
        return;
    };
    // Default selection: the first changed file.
    if tab
        .file
        .as_ref()
        .is_none_or(|f| matches!(f.source, DiffSource::Commit(_)))
    {
        let st = &snap.status;
        let first = st
            .conflicted
            .first()
            .map(|e| (DiffSource::Unstaged, e, true))
            .or_else(|| {
                st.unstaged
                    .first()
                    .map(|e| (DiffSource::Unstaged, e, false))
            })
            .or_else(|| st.staged.first().map(|e| (DiffSource::Staged, e, false)));
        if let Some((source, entry, conflicted)) = first {
            tab.select_file(FileSelection {
                source,
                entry: entry.clone(),
                conflicted,
            });
        } else if tab.file.is_some() {
            tab.file = None;
            tab.diff = None;
        }
    }

    egui::Panel::left("changes-list")
        .resizable(true)
        .default_size(400.0)
        .min_size(300.0)
        .max_size(700.0)
        .frame(egui::Frame::new().fill(p.panel).stroke(Stroke::new(1.0, p.border)))
        .show(ui, |ui| {
            egui::Panel::bottom("composer")
                .frame(egui::Frame::new().fill(p.panel).inner_margin(egui::Margin::same(12)))
                .show(ui, |ui| composer(ui, tab, &snap, actions));
            egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 8))).show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let st = &snap.status;
                    if st.is_clean() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(40.0);
                            art::show(ui, art::empty_clean(), vec2(160.0, 120.0));
                            ui.add_space(6.0);
                            ui.label(RichText::new("Working tree clean").font(FontId::new(15.0, theme::semibold())));
                            ui.label(RichText::new("Nothing to commit. Edit some files and they show up here.").color(p.text_dim).size(12.5));
                        });
                        return;
                    }
                    if !st.conflicted.is_empty() {
                        list(ui, tab, List::Conflicted, &st.conflicted, actions);
                        ui.add_space(10.0);
                    }
                    list(ui, tab, List::Unstaged, &st.unstaged, actions);
                    ui.add_space(10.0);
                    list(ui, tab, List::Staged, &st.staged, actions);
                });
            });
        });
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(p.bg))
        .show(ui, |ui| {
            if tab.resolver.is_some() && tab.file.as_ref().is_some_and(|f| f.conflicted) {
                resolve::show(ui, tab, actions);
            } else {
                diff::show(ui, tab, actions);
            }
        });
}

fn list(
    ui: &mut Ui,
    tab: &mut RepoTab,
    which: List,
    files: &[FileEntry],
    actions: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    let (title, color) = match which {
        List::Conflicted => ("Conflicts", p.danger),
        List::Unstaged => ("Unstaged", p.warning),
        List::Staged => ("Staged", p.success),
    };
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        widgets::section_label(ui, title);
        widgets::count_badge(ui, files.len(), color);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match which {
            List::Unstaged if !files.is_empty() => {
                if ui.small_button(format!("{}  Stage all", icon::PLUS)).clicked() {
                    actions.push(Action::Op(Op::StageAll));
                }
                if ui.small_button(RichText::new(icon::ARROW_COUNTER_CLOCKWISE).color(p.danger)).on_hover_text("Discard all unstaged changes").clicked() {
                    actions.push(Action::Confirm { title: "Discard all changes".into(), body: format!("Throw away every unstaged change in {} files, including untracked files? This cannot be undone.", files.len()), confirm: "Discard all".into(), danger: true, op: Op::Discard(files.to_vec()) });
                }
            }
            List::Staged if !files.is_empty()
                && ui.small_button(format!("{}  Unstage all", icon::MINUS)).clicked() => {
                    actions.push(Action::Op(Op::UnstageAll));
                }
            _ => {}
        });
    });
    ui.add_space(4.0);
    if files.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                RichText::new(if which == List::Staged {
                    "Nothing staged yet"
                } else {
                    "No unstaged changes"
                })
                .size(12.0)
                .color(p.text_faint),
            );
        });
        return;
    }
    for f in files {
        let source = if which == List::Staged {
            DiffSource::Staged
        } else {
            DiffSource::Unstaged
        };
        let conflicted = which == List::Conflicted;
        let selected = tab.file.as_ref().is_some_and(|s| {
            s.entry.path == f.path && s.source == source && s.conflicted == conflicted
        });
        let resp = commit_detail::file_row(ui, f, selected);
        // Hover action button on the right.
        let btn = egui::Rect::from_center_size(
            resp.rect.right_center() - vec2(18.0, 0.0),
            vec2(24.0, 22.0),
        );
        let (glyph, tip) = if which == List::Staged {
            (icon::MINUS, "Unstage")
        } else if conflicted {
            (icon::CHECK, "Mark resolved")
        } else {
            (icon::PLUS, "Stage")
        };
        let bresp = ui.interact(btn, resp.id.with("act"), Sense::click());
        if resp.hovered() || bresp.hovered() || selected {
            ui.painter().rect(
                btn,
                CornerRadius::same(theme::RADIUS_SM),
                if bresp.hovered() {
                    Palette::tint(p.accent, 90)
                } else {
                    p.elevated
                },
                Stroke::new(1.0, p.border),
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                btn.center(),
                Align2::CENTER_CENTER,
                glyph,
                FontId::proportional(13.0),
                p.text,
            );
        }
        let toggle = || {
            if which == List::Staged {
                Op::Unstage(vec![f.path.clone()])
            } else {
                Op::Stage(vec![f.path.clone()])
            }
        };
        if bresp.on_hover_text(tip).clicked() || resp.double_clicked() {
            actions.push(Action::Op(toggle()));
        } else if resp.clicked() {
            tab.select_file(FileSelection {
                source,
                entry: f.clone(),
                conflicted,
            });
        }
        let entry = f.clone();
        resp.context_menu(|ui| {
            ui.set_min_width(200.0);
            if ui.button(format!("{}  {}", glyph, tip)).clicked() {
                actions.push(Action::Op(toggle()));
            }
            if which == List::Unstaged
                && ui
                    .button(
                        RichText::new(format!(
                            "{}  Discard changes…",
                            icon::ARROW_COUNTER_CLOCKWISE
                        ))
                        .color(p.danger),
                    )
                    .clicked()
            {
                actions.push(Action::Confirm {
                    title: "Discard changes".into(),
                    body: format!(
                        "Throw away the unstaged changes to {}? This cannot be undone.",
                        entry.path
                    ),
                    confirm: "Discard".into(),
                    danger: true,
                    op: Op::Discard(vec![entry.clone()]),
                });
            }
            ui.separator();
            if ui
                .button(format!("{}  Open file", icon::ARROW_SQUARE_OUT))
                .clicked()
            {
                actions.push(Action::OpenExternal(
                    tab.path.join(&entry.path).display().to_string(),
                ));
            }
            if ui.button(format!("{}  Copy path", icon::COPY)).clicked() {
                actions.push(Action::Copy(entry.path.clone()));
            }
        });
    }
}

fn composer(
    ui: &mut Ui,
    tab: &mut RepoTab,
    snap: &canopy_core::RepoSnapshot,
    actions: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    ui.painter().hline(
        ui.max_rect().x_range(),
        ui.max_rect().min.y - 12.0,
        Stroke::new(1.0, p.border),
    );
    ui.horizontal(|ui| {
        widgets::section_label(ui, "Commit");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let n = tab.commit_summary.chars().count();
            let color = if n > 72 {
                p.danger
            } else if n > 50 {
                p.warning
            } else {
                p.text_faint
            };
            ui.label(RichText::new(format!("{n}/72")).size(11.5).color(color))
                .on_hover_text("Keep the summary under 50 characters; 72 at most.");
        });
    });
    ui.add_space(4.0);
    let summary = ui.add(
        egui::TextEdit::singleline(&mut tab.commit_summary)
            .id(egui::Id::new("commit-summary"))
            .hint_text("Summary (required)")
            .font(FontId::new(13.5, theme::medium()))
            .desired_width(f32::INFINITY)
            .margin(egui::Margin::symmetric(8, 6)),
    );
    if tab.focus_summary {
        summary.request_focus();
        tab.focus_summary = false;
    }
    ui.add_space(6.0);
    ui.add(
        egui::TextEdit::multiline(&mut tab.commit_body)
            .hint_text("Description")
            .desired_rows(4)
            .desired_width(f32::INFINITY)
            .margin(egui::Margin::symmetric(8, 6)),
    );
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let was = tab.amend;
        ui.checkbox(&mut tab.amend, "Amend last commit")
            .on_hover_text("Replace the last commit with the staged changes and this message");
        if tab.amend && !was && tab.commit_summary.is_empty() {
            actions.push(Action::PrefillAmend);
        }
    });
    ui.add_space(8.0);
    let staged = snap.status.staged.len();
    let can = (staged > 0 || tab.amend)
        && !tab.commit_summary.trim().is_empty()
        && tab.running.is_none()
        && snap.status.conflicted.is_empty();
    let branch = snap.head.label();
    let label = if tab.amend {
        format!("Amend {branch}")
    } else if staged > 0 {
        format!(
            "Commit {staged} file{} to {branch}",
            if staged == 1 { "" } else { "s" }
        )
    } else {
        "Stage files to commit".to_owned()
    };
    if widgets::primary_button(ui, &label, can, ui.available_width())
        .on_hover_text(crate::keymap::hint(crate::keymap::Command::Commit))
        .clicked()
        && can
    {
        actions.push(Action::Commit);
    }
}
