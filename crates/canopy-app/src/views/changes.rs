//! The staging area: conflicted, unstaged and staged files, the commit composer and the diff.

use egui::{Align2, CornerRadius, FontId, RichText, Sense, Stroke, Ui, vec2};
use egui_phosphor::regular as icon;

use canopy_core::identity::Identity;
use canopy_core::{DiffSource, FileEntry};

use crate::action::Action;
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
            tab.deselect_file();
        }
    }

    let pane_max = (ui.available_width() * 0.4).max(160.0);
    egui::Panel::left("changes-list")
        .resizable(true)
        .default_size(pane_max.min(400.0))
        .min_size(pane_max.min(220.0))
        .max_size(pane_max.min(700.0))
        .frame(egui::Frame::new().fill(p.panel).stroke(Stroke::new(1.0, p.border)))
        .show(ui, |ui| {
            let composer_height = (ui.available_height() * 0.60).max(60.0);
            // Short windows show a two-row description (`composer_fields`), so the composer
            // needs less room and the file lists keep it.
            let cap = if ui.ctx().content_rect().height() < 600.0 { 240.0 } else { 280.0 };
            egui::Panel::bottom("composer")
                .default_size(composer_height.min(cap))
                .min_size(composer_height.min(cap.min(250.0)))
                .max_size(composer_height.min(cap))
                .frame(egui::Frame::new().fill(p.panel).inner_margin(egui::Margin::same(12)))
                .show(ui, |ui| composer(ui, tab, &snap, actions));
            egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 8))).show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let st = &snap.status;
                    if let Some(u) = &snap.status_unavailable {
                        // A partial clone that lacks the tree of HEAD: the list is empty because
                        // it could not be read, not because nothing changed.
                        ui.vertical_centered(|ui| {
                            ui.add_space(40.0);
                            art::show(ui, art::empty_error(), vec2(160.0, 120.0));
                            ui.add_space(6.0);
                            ui.label(RichText::new("Changes not available").font(FontId::new(theme::TITLE_SIZE, theme::semibold())));
                            ui.label(RichText::new(&u.message).color(p.text_dim).size(theme::META_SIZE));
                            if u.can_download {
                                ui.add_space(8.0);
                                if ui.button(format!("{}  Retry", icon::ARROWS_CLOCKWISE)).clicked() {
                                    actions.push(Action::Refresh);
                                }
                            }
                        });
                        return;
                    }
                    if st.is_clean() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(40.0);
                            art::show(ui, art::empty_clean(), vec2(160.0, 120.0));
                            ui.add_space(6.0);
                            ui.label(RichText::new("Working tree clean").font(FontId::new(theme::TITLE_SIZE, theme::semibold())));
                            ui.label(RichText::new("Nothing to commit. Edit some files and they show up here.").color(p.text_dim).size(theme::META_SIZE));
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
    let compact = ui.available_width() < 280.0;
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        widgets::section_label(ui, title);
        widgets::count_badge(ui, files.len(), color);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match which {
            List::Unstaged if !files.is_empty() => {
                if ui.small_button(if compact { icon::PLUS.to_owned() } else { format!("{}  Stage all", icon::PLUS) }).on_hover_text("Stage all files").clicked() {
                    actions.push(Action::Op(Op::StageAll));
                }
                if ui.small_button(RichText::new(icon::ARROW_COUNTER_CLOCKWISE).color(p.danger)).on_hover_text("Discard all unstaged changes").clicked() {
                    actions.push(Action::Confirm { title: "Discard all changes".into(), body: format!("Throw away every unstaged change in {} files, including untracked files? This cannot be undone.", files.len()), confirm: "Discard all".into(), danger: true, op: Op::Discard(files.to_vec()) });
                }
            }
            List::Staged if !files.is_empty()
                && ui.small_button(if compact { icon::MINUS.to_owned() } else { format!("{}  Unstage all", icon::MINUS) }).on_hover_text("Unstage all files").clicked() => {
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
                .size(theme::META_SIZE)
                .color(p.text_faint),
            );
        });
        return;
    }
    commit_detail::virtual_rows(ui, files.len(), commit_detail::FILE_ROW_H, |ui, i| {
        let f = &files[i];
        let source = if which == List::Staged {
            DiffSource::Staged
        } else {
            DiffSource::Unstaged
        };
        let conflicted = which == List::Conflicted;
        let selected = tab.file.as_ref().is_some_and(|s| {
            s.entry.path == f.path && s.source == source && s.conflicted == conflicted
        });
        let id = ui.id().with(("change", which as u8, &f.path));
        let resp = commit_detail::file_row(ui, f, selected, id);
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
                FontId::proportional(theme::BODY_SIZE),
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
                actions.push(Action::OpenInEditor(tab.path.join(&entry.path)));
            }
            if ui.button(format!("{}  Copy path", icon::COPY)).clicked() {
                actions.push(Action::Copy(entry.path.clone()));
            }
        });
    });
}

fn composer(
    ui: &mut Ui,
    tab: &mut RepoTab,
    snap: &canopy_core::RepoSnapshot,
    actions: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    ui.style_mut().spacing.scroll.floating = false;
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
    let warning = identity_notice(tab.identity.as_ref(), tab.identity_error.is_some());
    let hint = warning.map_or_else(
        || crate::keymap::hint(crate::keymap::Command::Commit).to_owned(),
        |w| format!("{w} Open Settings → Git to correct it. Git's own error will appear if you commit without a usable identity."),
    );
    // The button and the identity warning sit at the bottom; the fields take the rest, so a
    // warning that wraps never pushes the button out of the panel.
    egui::Panel::bottom("commit-footer")
        .frame(egui::Frame::new())
        .show(ui, |ui| {
            if let Some(warning) = warning {
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("{}  {warning}", icon::WARNING))
                                .color(p.warning)
                                .size(theme::SMALL_SIZE),
                        )
                        .wrap(),
                    );
                    if ui.small_button("Edit identity").clicked() {
                        actions.push(Action::IdentitySettings);
                    }
                });
                ui.add_space(theme::XS);
            }
            if widgets::primary_button(ui, &label, can, ui.available_width())
                .on_hover_text(hint)
                .clicked()
                && can
            {
                actions.push(Action::Commit);
            }
        });
    egui::ScrollArea::vertical()
        .id_salt("commit-fields")
        .min_scrolled_height(0.0)
        .auto_shrink([false, false])
        .show(ui, |ui| composer_fields(ui, tab, actions));
}

fn composer_fields(ui: &mut Ui, tab: &mut RepoTab, actions: &mut Vec<Action>) {
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
            ui.label(
                RichText::new(format!("{n}/72"))
                    .size(theme::SMALL_SIZE)
                    .color(color),
            )
            .on_hover_text("Keep the summary under 50 characters; 72 at most.");
        });
    });
    ui.add_space(4.0);
    let summary = ui.add(
        egui::TextEdit::singleline(&mut tab.commit_summary)
            .id(egui::Id::new("commit-summary"))
            .hint_text("Summary (required)")
            .font(FontId::new(theme::LEAD_SIZE, theme::medium()))
            .desired_width(f32::INFINITY)
            .margin(theme::FIELD_MARGIN),
    );
    if tab.focus_summary {
        summary.request_focus();
        tab.focus_summary = false;
    }
    ui.add_space(6.0);
    ui.add(
        egui::TextEdit::multiline(&mut tab.commit_body)
            .hint_text("Description")
            .desired_rows(if ui.ctx().content_rect().height() < 600.0 {
                2
            } else {
                4
            })
            .desired_width(f32::INFINITY)
            .margin(theme::FIELD_MARGIN),
    );
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let was = tab.amend;
        widgets::checkbox(ui, &mut tab.amend, "Amend last commit")
            .on_hover_text("Replace the last commit with the staged changes and this message");
        if tab.amend && !was && tab.commit_summary.is_empty() {
            actions.push(Action::PrefillAmend);
        }
    });
    ui.add_space(8.0);
}

/// The line above the commit button when Git's identity needs attention.
fn identity_notice(identity: Option<&Identity>, read_failed: bool) -> Option<&'static str> {
    identity
        .and_then(Identity::warning)
        .or(read_failed.then_some("Could not read Git identity."))
}

#[cfg(test)]
mod tests {
    use canopy_core::identity::{Field, Person, Source};

    use super::*;

    fn identity(name: &str, email: &str, source: Option<Source>, usable: bool) -> Identity {
        let field = |value: &str| Field {
            value: value.into(),
            source: source.clone(),
        };
        let person = Person {
            name: field(name),
            email: field(email),
            usable,
        };
        Identity {
            author: person.clone(),
            committer: person,
        }
    }

    #[test]
    fn the_commit_panel_warns_only_when_the_identity_needs_attention() {
        let configured = Some(Source::Config {
            scope: "global".into(),
            origin: "file:/home/ada/.gitconfig".into(),
        });
        let good = identity("Ada Lovelace", "ada@example.com", configured.clone(), true);
        assert_eq!(identity_notice(Some(&good), false), None);
        assert_eq!(
            identity_notice(None, false),
            None,
            "still reading: no warning yet"
        );

        let placeholder = identity("t", "t@t", configured.clone(), true);
        assert_eq!(
            identity_notice(Some(&placeholder), false),
            Some("This identity looks like a placeholder.")
        );
        let guessed = identity("ada", "ada@host.example", Some(Source::Guessed), true);
        assert_eq!(
            identity_notice(Some(&guessed), false),
            Some("Git guessed part of this identity from the computer.")
        );
        let unusable = identity("", "", None, false);
        assert_eq!(
            identity_notice(Some(&unusable), false),
            Some("Git cannot use this identity for a commit.")
        );
        assert_eq!(
            identity_notice(None, true),
            Some("Could not read Git identity.")
        );
    }

    #[test]
    fn wrapped_identity_notice_does_not_overlap_commit_fields() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, true);
        let dir = tempfile::tempdir().expect("temporary tab");
        let mut tab = RepoTab::new(dir.path().into(), &ctx, 100, Default::default());
        let unusable = identity("", "", None, false);
        let warning = unusable.warning().expect("identity warning");
        tab.identity = Some(unusable);
        tab.snap = Some(std::sync::Arc::new(canopy_core::RepoSnapshot::default()));
        for _ in 0..3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        vec2(562.5, 350.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::Panel::top("test-actions")
                        .exact_size(84.0)
                        .show(ui, |_| {});
                    egui::Panel::bottom("test-status")
                        .exact_size(26.0)
                        .show(ui, |_| {});
                    show(ui, &mut tab, &mut Vec::new());
                },
            );
            output.textures_delta.clear();
            let warning_rect = output
                .shapes
                .iter()
                .find_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape
                        && text.galley.text().contains(warning)
                    {
                        Some(text.visual_bounding_rect())
                    } else {
                        None
                    }
                })
                .expect("painted identity notice");
            let summary = output
                .shapes
                .iter()
                .find_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape
                        && text.galley.text().contains("Summary (required)")
                    {
                        Some(text.visual_bounding_rect().intersect(shape.clip_rect))
                    } else {
                        None
                    }
                })
                .expect("painted summary hint");
            assert!(
                !summary.is_positive() || !warning_rect.intersects(summary),
                "identity notice {warning_rect:?} overlaps painted summary {summary:?}",
            );
        }
    }

    #[test]
    fn commit_button_is_visible_in_default_desktop_layouts() {
        for size in [vec2(600.0, 375.0), vec2(960.0, 600.0), vec2(1440.0, 900.0)] {
            let ctx = egui::Context::default();
            crate::ui::fonts::install(&ctx);
            theme::apply(&ctx, theme::ThemeId::Midnight, true);
            let dir = tempfile::tempdir().expect("temporary tab");
            let mut tab = RepoTab::new(dir.path().into(), &ctx, 100, Default::default());
            tab.snap = Some(std::sync::Arc::new(canopy_core::RepoSnapshot::default()));
            let mut visible = false;
            for _ in 0..3 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ui| {
                        egui::Panel::top("test-actions")
                            .exact_size(96.0)
                            .show(ui, |_| {});
                        egui::Panel::bottom("test-status")
                            .exact_size(26.0)
                            .show(ui, |_| {});
                        show(ui, &mut tab, &mut Vec::new());
                    },
                );
                output.textures_delta.clear();
                visible = output.shapes.iter().any(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape
                        && text.galley.text() == "Stage files to commit"
                    {
                        return shape.clip_rect.contains_rect(text.visual_bounding_rect());
                    }
                    false
                });
            }
            assert!(visible, "Commit button must be visible at {size:?}");
        }
    }
}
