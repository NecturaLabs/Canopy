//! The details pane under the graph: commit metadata, changed files and the selected file's diff.

use std::ops::Range;

use egui::{Align2, CornerRadius, FontId, RichText, Sense, Ui, vec2};
use egui_phosphor::regular as icon;

use canopy_core::{CommitDetails, DiffSource, FileEntry, short_id};

use crate::action::Action;
use crate::tab::{DetailsError, FileSelection, RepoTab};
use crate::ui::theme::{self, Palette, palette};
use crate::ui::{art, widgets};
use crate::views::diff;

/// Draws the details pane.
pub fn show(
    ui: &mut Ui,
    tab: &mut RepoTab,
    settings: &crate::settings::Settings,
    actions: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    header(ui, tab);
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
            } else if tab.snap.as_ref().is_some_and(|s| !s.commits.is_empty()) {
                ui.label(RichText::new("Select a commit to see its changes").color(p.text_faint));
            }
        });
        return;
    };
    let pane_max = (ui.available_width() * 0.4).max(160.0);
    egui::Panel::left("detail-meta")
        .resizable(true)
        .default_size(pane_max.min(320.0))
        .min_size(pane_max.min(220.0))
        .max_size(pane_max.min(640.0))
        .frame(
            egui::Frame::new()
                .fill(p.panel)
                .inner_margin(egui::Margin::same(theme::SM as i8)),
        )
        .show(ui, |ui| {
            ui.spacing_mut().scroll = theme::solid_scroll();
            ui.spacing_mut().item_spacing.y = theme::XS;
            author(ui, &d, tab);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(short_id(d.oid))
                        .monospace()
                        .size(theme::SMALL_SIZE)
                        .color(p.accent2),
                )
                .on_hover_text(d.oid.to_string());
                if widgets::icon_button(ui, icon::COPY, "Copy SHA").clicked() {
                    actions.push(Action::Copy(d.oid.to_string()));
                }
                ui.label(
                    RichText::new(widgets::absolute_date_format(
                        d.author.time,
                        settings.iso_dates,
                    ))
                    .size(theme::SMALL_SIZE)
                    .color(p.text_faint),
                )
                .on_hover_text(widgets::full_date(d.author.time, d.author.offset_minutes));
            });
            let glyph = if tab.history_layout.information_open {
                icon::CARET_DOWN
            } else {
                icon::CARET_RIGHT
            };
            if ui
                .add(egui::Button::new(format!("{glyph}  Commit information")).frame(false))
                .clicked()
            {
                tab.history_layout.information_open = !tab.history_layout.information_open;
            }
            if tab.history_layout.information_open {
                let files_min = FILE_ROW_H * d.files.len().min(2) as f32
                    + ui.spacing().interact_size.y * 2.0
                    + theme::XS * 3.0;
                egui::ScrollArea::vertical()
                    .id_salt("commit-metadata")
                    .max_height((ui.available_height() - files_min).max(0.0))
                    .min_scrolled_height(0.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        message(ui, &d);
                        ui.add_space(theme::XS);
                        ui.add(
                            egui::Label::new(
                                RichText::new(&d.author.email)
                                    .size(theme::SMALL_SIZE)
                                    .color(p.text_dim),
                            )
                            .wrap(),
                        );
                        metadata(ui, &d, actions);
                    });
            }
            ui.add_space(theme::XS);
            file_stats(ui, &d);
            ui.add_space(theme::XS);
            egui::ScrollArea::vertical()
                .id_salt("commit-file-list")
                .min_scrolled_height(0.0)
                .auto_shrink([false, false])
                .show(ui, |ui| files(ui, &d, tab));
        });
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(p.bg))
        .show(ui, |ui| diff::show(ui, tab, actions));
}

/// Pane navigation belongs to the commit, including when its files have not loaded.
fn header(ui: &mut Ui, tab: &mut RepoTab) {
    if tab.history_layout.compact {
        return;
    }
    let p = palette(ui.ctx());
    egui::Panel::top("commit-pane-header")
        .frame(
            egui::Frame::new()
                .fill(p.panel)
                .inner_margin(egui::Margin::symmetric(theme::MD as i8, theme::XS as i8)),
        )
        .show(ui, |ui| {
            let label = if tab.history_layout.expanded {
                format!("{}  Back to history", icon::ARROWS_IN_SIMPLE)
            } else {
                format!("{}  Expand", icon::ARROWS_OUT_SIMPLE)
            };
            let button_width = ui
                .painter()
                .layout_no_wrap(
                    label.clone(),
                    FontId::new(theme::BODY_SIZE, theme::medium()),
                    p.text,
                )
                .size()
                .x
                + ui.spacing().button_padding.x * 2.0;
            let (row, _) = ui.allocate_exact_size(
                vec2(ui.available_width(), ui.spacing().interact_size.y),
                Sense::hover(),
            );
            let button_rect =
                egui::Rect::from_min_max(egui::pos2(row.max.x - button_width, row.min.y), row.max);
            let summary_rect = egui::Rect::from_min_max(
                row.min,
                egui::pos2(button_rect.min.x - theme::SM, row.max.y),
            );
            let mut summary_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(summary_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            summary_ui.set_clip_rect(summary_rect.intersect(ui.clip_rect()));
            let summary = tab
                .details
                .as_ref()
                .map(|d| d.message.lines().next().unwrap_or("").trim())
                .unwrap_or("Commit details");
            summary_ui
                .add(
                    egui::Label::new(
                        RichText::new(summary)
                            .font(FontId::new(theme::BODY_SIZE, theme::semibold()))
                            .color(p.text),
                    )
                    .truncate(),
                )
                .on_hover_text(summary);
            if ui
                .add_enabled_ui(tab.selected_commit.is_some(), |ui| {
                    ui.put(button_rect, egui::Button::new(label))
                })
                .inner
                .clicked()
            {
                tab.history_layout.expanded = !tab.history_layout.expanded;
                tab.history_layout.compact_details = true;
            }
        });
}

fn message(ui: &mut Ui, d: &CommitDetails) {
    let p = palette(ui.ctx());
    let mut lines = d.message.trim().splitn(2, '\n');
    let summary = lines.next().unwrap_or("").trim();
    let body = lines.next().unwrap_or("").trim();
    ui.add(
        egui::Label::new(
            RichText::new(summary)
                .font(FontId::new(theme::TITLE_SIZE, theme::semibold()))
                .color(p.text),
        )
        .wrap(),
    );
    if !body.is_empty() {
        ui.add_space(theme::XS);
        ui.add(
            egui::Label::new(RichText::new(body).color(p.text_dim).size(theme::META_SIZE)).wrap(),
        );
    }
}

fn author(ui: &mut Ui, d: &CommitDetails, tab: &mut RepoTab) {
    let p = palette(ui.ctx());
    ui.horizontal(|ui| {
        tab.request_avatar(d.oid, &d.author.email);
        if let Some(texture) = tab.avatars.get(&d.author.email) {
            ui.add(
                egui::Image::new(texture)
                    .fit_to_exact_size(vec2(theme::XL, theme::XL))
                    .corner_radius(egui::CornerRadius::same((theme::XL / 2.0) as u8)),
            );
        } else {
            widgets::avatar(ui, theme::XL, &d.author.name, &d.author.email);
        }
        ui.add(
            egui::Label::new(
                RichText::new(&d.author.name)
                    .font(FontId::new(theme::BODY_SIZE, theme::medium()))
                    .color(p.text),
            )
            .truncate(),
        )
        .on_hover_text(format!("{} <{}>", d.author.name, d.author.email));
    });
}

fn metadata(ui: &mut Ui, d: &CommitDetails, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
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
    if !d.parents.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(icon::GIT_COMMIT).color(p.text_faint));
            ui.label(
                RichText::new(if d.parents.len() > 1 {
                    "Parents"
                } else {
                    "Parent"
                })
                .size(theme::META_SIZE)
                .color(p.text_faint),
            );
            for parent in &d.parents {
                if ui
                    .link(
                        RichText::new(short_id(*parent))
                            .monospace()
                            .size(theme::SMALL_SIZE),
                    )
                    .clicked()
                {
                    actions.push(Action::Reveal(*parent));
                }
            }
        });
    }
}

fn file_stats(ui: &mut Ui, d: &CommitDetails) {
    let p = palette(ui.ctx());
    let (adds, dels) = d
        .files
        .iter()
        .fold((0, 0), |(a, r), f| (a + f.additions, r + f.deletions));
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(format!(
                "{} file{} changed",
                d.files.len(),
                if d.files.len() == 1 { "" } else { "s" }
            ))
            .font(FontId::new(theme::META_SIZE, theme::semibold()))
            .color(p.text_dim),
        );
        if d.stats_known {
            ui.label(
                RichText::new(format!("+{adds}"))
                    .color(p.success)
                    .font(FontId::monospace(theme::MONO_SMALL_SIZE)),
            );
            ui.label(
                RichText::new(format!("−{dels}"))
                    .color(p.danger)
                    .font(FontId::monospace(theme::MONO_SMALL_SIZE)),
            );
        } else {
            // A partial clone that has not downloaded the file contents.
            ui.label(
                RichText::new("line counts not downloaded")
                    .color(p.text_faint)
                    .size(theme::SMALL_SIZE),
            )
            .on_hover_text(
                "This is a partial clone: the line counts and renamed files \
                 need file contents that were not downloaded. Open a file \
                 to download it.",
            );
        }
    });
}

fn files(ui: &mut Ui, d: &CommitDetails, tab: &mut RepoTab) {
    virtual_rows(ui, d.files.len(), FILE_ROW_H, |ui, i| {
        let f = &d.files[i];
        let selected = tab.file.as_ref().is_some_and(|s| s.entry.path == f.path);
        let id = ui.id().with(("commit-file", &f.path, &f.old_path, f.kind));
        if file_row(ui, f, selected, id).clicked() {
            tab.select_file(FileSelection {
                source: DiffSource::Commit(d.oid),
                entry: f.clone(),
                conflicted: false,
            });
        }
    });
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
                        .font(FontId::new(theme::SUBTITLE_SIZE, theme::semibold())),
                );
                ui.label(
                    RichText::new(&error.message)
                        .size(theme::META_SIZE)
                        .color(p.text_dim),
                );
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
        ui.add(
            egui::Label::new(RichText::new(text).size(theme::META_SIZE).color(p.text_dim)).wrap(),
        );
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
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &f.path)
    });
    let hover = ui
        .ctx()
        .animate_bool_with_time(resp.id, resp.hovered(), 0.1);
    if selected {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(theme::RADIUS_SM), p.selected());
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
        FontId::new(theme::SMALL_SIZE, theme::semibold()),
        color,
    );
    let (name, dir) = widgets::split_path(&f.path);
    let mut right = rect.max.x - 8.0;
    if f.additions + f.deletions > 0 {
        let dg = ui.painter().text(
            egui::pos2(right, rect.center().y),
            Align2::RIGHT_CENTER,
            format!("−{}", f.deletions),
            FontId::monospace(theme::MONO_SMALL_SIZE),
            p.danger,
        );
        let ag = ui.painter().text(
            egui::pos2(dg.min.x - 6.0, rect.center().y),
            Align2::RIGHT_CENTER,
            format!("+{}", f.additions),
            FontId::monospace(theme::MONO_SMALL_SIZE),
            p.success,
        );
        right = ag.min.x - 8.0;
    }
    let clip = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + 30.0, rect.min.y),
        egui::pos2(right, rect.max.y),
    );
    let painter = ui.painter().with_clip_rect(clip);
    let ng = painter.layout_no_wrap(
        name.to_owned(),
        FontId::new(theme::BODY_SIZE, theme::medium()),
        p.text,
    );
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
            FontId::proportional(theme::SMALL_SIZE),
            p.text_faint,
        );
    }
    let tip = match &f.old_path {
        Some(old) => format!("{} → {}", old, f.path),
        None => f.path.clone(),
    };
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
