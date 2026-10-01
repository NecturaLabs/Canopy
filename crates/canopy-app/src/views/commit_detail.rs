//! The details pane under the graph: commit metadata, changed files and the selected file's diff.

use std::ops::Range;

use egui::{Align2, CornerRadius, FontId, RichText, Sense, Stroke, Ui, vec2};
use egui_phosphor::regular as icon;

use canopy_core::{DiffSource, FileEntry, short_id};

use crate::app::Action;
use crate::tab::{DetailsError, FileSelection, RepoTab};
use crate::ui::theme::{self, Palette, palette};
use crate::ui::{art, widgets};
use crate::views::diff;

/// Draws the details pane.
pub fn show(ui: &mut Ui, tab: &mut RepoTab, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let Some(d) = tab.details.clone() else {
        // A failed load stays on screen with its reason, so selecting the same commit again is
        // not the only way out.
        if let Some(error) = tab
            .details_error
            .clone()
            .filter(|e| tab.selected_commit == Some(e.oid))
        {
            failed(ui, tab, &error);
            return;
        }
        ui.centered_and_justified(|ui| {
            if tab.selected_commit.is_some() {
                ui.spinner();
            } else {
                ui.label(RichText::new("Select a commit to see its changes").color(p.text_faint));
            }
        });
        return;
    };
    egui::Panel::left("detail-meta")
        .resizable(true)
        .default_size(380.0)
        .min_size(260.0)
        .max_size(640.0)
        .frame(
            egui::Frame::new()
                .fill(p.panel)
                .inner_margin(egui::Margin::same(14)),
        )
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let mut lines = d.message.trim().splitn(2, '\n');
                    let summary = lines.next().unwrap_or("").trim();
                    let body = lines.next().unwrap_or("").trim();
                    ui.label(
                        RichText::new(summary)
                            .font(FontId::new(15.5, theme::semibold()))
                            .color(p.text),
                    );
                    if !body.is_empty() {
                        ui.add_space(4.0);
                        ui.label(RichText::new(body).color(p.text_dim).size(12.5));
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        widgets::avatar(ui, 32.0, &d.author.name, &d.author.email);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            ui.label(
                                RichText::new(&d.author.name)
                                    .font(FontId::new(13.0, theme::medium()))
                                    .color(p.text),
                            );
                            ui.label(
                                RichText::new(&d.author.email)
                                    .size(11.5)
                                    .color(p.text_faint),
                            );
                        });
                    });
                    ui.add_space(6.0);
                    meta_row(
                        ui,
                        icon::CALENDAR_BLANK,
                        &widgets::full_date(d.author.time, d.author.offset_minutes),
                    );
                    if d.committer.email != d.author.email || d.committer.name != d.author.name {
                        meta_row(
                            ui,
                            icon::USER_SWITCH,
                            &format!("Committed by {}", d.committer.name),
                        );
                    }
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(icon::HASH).color(p.text_faint));
                        ui.label(
                            RichText::new(d.oid.to_string())
                                .monospace()
                                .size(11.5)
                                .color(p.accent2),
                        );
                        if widgets::icon_button(ui, icon::COPY, "Copy SHA").clicked() {
                            actions.push(Action::Copy(d.oid.to_string()));
                        }
                    });
                    if !d.parents.is_empty() {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(icon::GIT_COMMIT).color(p.text_faint));
                            ui.label(
                                RichText::new(if d.parents.len() > 1 {
                                    "Parents"
                                } else {
                                    "Parent"
                                })
                                .size(12.0)
                                .color(p.text_faint),
                            );
                            for parent in &d.parents {
                                if ui
                                    .link(RichText::new(short_id(*parent)).monospace().size(11.5))
                                    .clicked()
                                {
                                    actions.push(Action::Reveal(*parent));
                                }
                            }
                        });
                    }
                    ui.add_space(12.0);
                    let (adds, dels) = d
                        .files
                        .iter()
                        .fold((0, 0), |(a, r), f| (a + f.additions, r + f.deletions));
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "{} file{} changed",
                                d.files.len(),
                                if d.files.len() == 1 { "" } else { "s" }
                            ))
                            .font(FontId::new(12.5, theme::semibold()))
                            .color(p.text_dim),
                        );
                        if d.stats_known {
                            ui.label(
                                RichText::new(format!("+{adds}"))
                                    .color(p.success)
                                    .size(12.0),
                            );
                            ui.label(RichText::new(format!("−{dels}")).color(p.danger).size(12.0));
                        } else {
                            // A partial clone that has not downloaded the file contents.
                            ui.label(
                                RichText::new("line counts not downloaded")
                                    .color(p.text_faint)
                                    .size(11.5),
                            )
                            .on_hover_text(
                                "This is a partial clone: the line counts and renamed files \
                                 need file contents that were not downloaded. Open a file \
                                 to download it.",
                            );
                        }
                    });
                    ui.add_space(4.0);
                    virtual_rows(ui, d.files.len(), FILE_ROW_H, |ui, i| {
                        let f = &d.files[i];
                        let selected = tab.file.as_ref().is_some_and(|s| s.entry.path == f.path);
                        let id = ui.id().with(("commit-file", &f.path));
                        if file_row(ui, f, selected, id).clicked() {
                            tab.select_file(FileSelection {
                                source: DiffSource::Commit(d.oid),
                                entry: f.clone(),
                                conflicted: false,
                            });
                        }
                    });
                });
        });
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(p.bg))
        .show(ui, |ui| diff::show(ui, tab, actions));
}

/// The details could not be loaded: git's reason and, when downloading can fix it (a partial
/// clone that has not fetched the commit), a Retry button.
fn failed(ui: &mut Ui, tab: &mut RepoTab, error: &DetailsError) {
    let p = palette(ui.ctx());
    // Git's message can be long and the pane short: scroll rather than clip the button.
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.set_max_width(ui.available_width() - 32.0);
                art::show(ui, art::empty_error(), vec2(120.0, 90.0));
                ui.label(
                    RichText::new(format!("Could not load commit {}", short_id(error.oid)))
                        .font(FontId::new(14.0, theme::semibold())),
                );
                ui.label(RichText::new(&error.message).size(12.0).color(p.text_dim));
                if error.can_download {
                    ui.add_space(8.0);
                    if ui
                        .button(format!("{}  Retry", icon::ARROWS_CLOCKWISE))
                        .clicked()
                    {
                        tab.retry_details();
                    }
                }
                ui.add_space(12.0);
            });
        });
}

fn meta_row(ui: &mut Ui, glyph: &str, text: &str) {
    let p = palette(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(RichText::new(glyph).color(p.text_faint));
        ui.label(RichText::new(text).size(12.0).color(p.text_dim));
    });
}

/// Height of [`file_row`].
pub const FILE_ROW_H: f32 = 28.0;

/// The rows of a fixed-height list (`row_h` apart, no spacing) whose top is at `top`, that
/// intersect `min_y..max_y`.
pub fn visible_rows(n: usize, row_h: f32, top: f32, min_y: f32, max_y: f32) -> Range<usize> {
    if n == 0 || row_h <= 0.0 || max_y <= min_y {
        return 0..0;
    }
    let first = ((min_y - top) / row_h).floor().max(0.0);
    let end = ((max_y - top) / row_h).ceil().max(0.0);
    // Float to usize saturates; clamp to the list.
    let first = (first as usize).min(n);
    first..(end as usize).clamp(first, n)
}

/// Draws a list of `n` rows of height `row_h` at the cursor, calling `row` only for the rows in
/// view and reserving the space of the others, so a list of thousands costs only its visible
/// rows. Rows must allocate exactly `row_h` and give themselves ids that do not depend on their
/// position (they come and go while scrolling).
pub fn virtual_rows(ui: &mut Ui, n: usize, row_h: f32, mut row: impl FnMut(&mut Ui, usize)) {
    let spacing = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = 0.0;
    let clip = ui.clip_rect();
    let range = visible_rows(n, row_h, ui.cursor().min.y, clip.min.y, clip.max.y);
    let width = ui.available_width();
    if range.start > 0 {
        ui.allocate_space(vec2(width, range.start as f32 * row_h));
    }
    for i in range.clone() {
        row(ui, i);
    }
    if range.end < n {
        ui.allocate_space(vec2(width, (n - range.end) as f32 * row_h));
    }
    ui.spacing_mut().item_spacing.y = spacing;
}

/// A changed-file row with its badge, name, folder and line counts. `id` identifies the row
/// across frames (for hover and its context menu).
pub fn file_row(ui: &mut Ui, f: &FileEntry, selected: bool, id: egui::Id) -> egui::Response {
    let p = palette(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), FILE_ROW_H), Sense::hover());
    let resp = ui.interact(rect, id, Sense::click());
    let hover = ui
        .ctx()
        .animate_bool_with_time(resp.id, resp.hovered(), 0.1);
    if selected {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(theme::RADIUS_SM),
            Palette::tint(p.accent, if p.dark { 50 } else { 32 }),
        );
    } else if hover > 0.0 {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(theme::RADIUS_SM),
            Palette::tint(p.text, (hover * 12.0) as u8),
        );
    }
    let color = widgets::change_style(p, f.kind);
    let badge =
        egui::Rect::from_center_size(rect.left_center() + vec2(15.0, 0.0), vec2(18.0, 18.0));
    ui.painter()
        .rect_filled(badge, CornerRadius::same(5), Palette::tint(color, 40));
    ui.painter().text(
        badge.center(),
        Align2::CENTER_CENTER,
        f.kind.letter(),
        FontId::new(11.0, theme::semibold()),
        color,
    );
    let (name, dir) = widgets::split_path(&f.path);
    let mut right = rect.max.x - 8.0;
    if f.additions + f.deletions > 0 {
        let dg = ui.painter().text(
            egui::pos2(right, rect.center().y),
            Align2::RIGHT_CENTER,
            format!("−{}", f.deletions),
            FontId::proportional(11.5),
            p.danger,
        );
        let ag = ui.painter().text(
            egui::pos2(dg.min.x - 6.0, rect.center().y),
            Align2::RIGHT_CENTER,
            format!("+{}", f.additions),
            FontId::proportional(11.5),
            p.success,
        );
        right = ag.min.x - 8.0;
    }
    let clip = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + 30.0, rect.min.y),
        egui::pos2(right, rect.max.y),
    );
    let painter = ui.painter().with_clip_rect(clip);
    let ng = painter.layout_no_wrap(name.to_owned(), FontId::new(13.0, theme::medium()), p.text);
    let nx = clip.min.x;
    painter.galley(
        egui::pos2(nx, rect.center().y - ng.size().y / 2.0),
        ng.clone(),
        p.text,
    );
    if !dir.is_empty() {
        painter.text(
            egui::pos2(nx + ng.size().x + 8.0, rect.center().y),
            Align2::LEFT_CENTER,
            dir,
            FontId::proportional(11.5),
            p.text_faint,
        );
    }
    let tip = match &f.old_path {
        Some(old) => format!("{} → {}", old, f.path),
        None => f.path.clone(),
    };
    let _ = Stroke::NONE;
    resp.on_hover_text(tip)
}

#[cfg(test)]
mod tests {
    use super::visible_rows;

    #[test]
    fn visible_rows_cover_the_viewport_only() {
        // Ten rows of 10 starting at y 100.
        assert_eq!(
            visible_rows(10, 10.0, 100.0, 0.0, 50.0),
            0..0,
            "above the list"
        );
        assert_eq!(visible_rows(10, 10.0, 100.0, 0.0, 115.0), 0..2);
        assert_eq!(visible_rows(10, 10.0, 100.0, 125.0, 145.0), 2..5);
        assert_eq!(visible_rows(10, 10.0, 100.0, 190.0, 400.0), 9..10);
        assert_eq!(
            visible_rows(10, 10.0, 100.0, 300.0, 400.0),
            10..10,
            "below the list"
        );
        assert_eq!(visible_rows(0, 10.0, 0.0, 0.0, 100.0), 0..0);
        // A huge list costs only the rows in view.
        let r = visible_rows(1_000_000, 26.0, 0.0, 5_000_000.0, 5_000_900.0);
        assert!(r.len() <= 36, "{r:?}");
    }
}
