//! The details pane under the graph: commit metadata, changed files and the selected file's diff.

use std::ops::Range;

use egui::{
    Align, Align2, CornerRadius, FontId, Layout, Rect, RichText, Ui, UiBuilder, pos2, vec2,
};
use egui_phosphor::regular as icon;

use canopy_core::{CommitDetails, DiffSource, FileEntry, short_id};

use crate::action::Action;
use crate::keymap::Command;
use crate::tab::{DetailsError, FileSelection, RepoTab};
use crate::ui::kit::avatar::{Avatar, AvatarSize, Person};
use crate::ui::kit::button::LinkButton;
use crate::ui::kit::chip::{ChangeBadge, DiffStat};
use crate::ui::kit::progress::Spinner;
use crate::ui::kit::row::{ListRow, RowKind, TreeRow, tree_children};
use crate::ui::kit::{Button, IconButton, Size, paint};
use crate::ui::theme::{self, Style, palette};
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
    let s = p.s();
    let m = theme::metrics();
    let headed = header(ui, tab);
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
                Spinner::new().label("commit details").active(true).show(ui);
            } else if tab.snap.as_ref().is_some_and(|s| !s.commits.is_empty()) {
                ui.label(RichText::new("Select a commit to see its changes").color(p.text_faint));
            }
        });
        return;
    };
    let pane_max = (ui.available_width() * 0.4).max(160.0);
    // A compact window keeps the changed files in view: the tighter inset of its controls.
    let pad = if tab.history_layout.compact {
        egui::Margin::same(crate::ui::points_i8(theme::SM))
    } else {
        egui::Margin::symmetric(
            crate::ui::points_i8(m.detail_pad.x),
            crate::ui::points_i8(m.detail_pad.y),
        )
    };
    egui::Panel::left("detail-meta")
        .resizable(true)
        .default_size(pane_max.min(m.detail_info_width))
        .min_size(pane_max.min(220.0))
        .max_size(pane_max.min(640.0))
        .frame(egui::Frame::new().fill(s.pane).inner_margin(pad))
        .show(ui, |ui| {
            ui.spacing_mut().scroll = theme::solid_scroll();
            ui.spacing_mut().item_spacing.y = if tab.history_layout.compact {
                theme::CONTROL_INSET
            } else {
                theme::XS
            };
            author(ui, &d, tab);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(short_id(d.oid))
                        .font(FontId::new(m.sha_size, egui::FontFamily::Monospace))
                        .color(p.accent),
                )
                .on_hover_text(d.oid.to_string());
                if IconButton::new(icon::COPY, "Copy SHA")
                    .size(Size::Xs)
                    .shortcut(Command::CopySha)
                    .show(ui)
                    .clicked()
                {
                    actions.push(Action::Copy(d.oid.to_string()));
                }
                ui.label(
                    RichText::new(widgets::absolute_date_format(
                        d.author.time,
                        settings.iso_dates,
                    ))
                    .size(m.meta_size)
                    .color(p.text_dim),
                )
                .on_hover_text(widgets::full_date(d.author.time, d.author.offset_minutes));
            });
            let files_min = file_row_height() * crate::ui::count_f32(d.files.len().min(2))
                + ui.spacing().interact_size.y * 2.0
                + theme::XS * 3.0;
            // Measured here: the body has no height limit of its own.
            let info_room =
                (ui.available_height() - files_min - RowKind::List.height() - theme::XS * 2.0)
                    .max(0.0);
            let mut open = tab.history_layout.information_open;
            // The pane header already shows a summary that fits it; the block then starts with
            // the rest of the message.
            let summary = !headed;
            let first_line = if summary {
                FontId::new(m.headline_size, theme::semibold())
            } else {
                FontId::proportional(m.meta_size)
            };
            let first_line = ui.fonts_mut(|f| f.row_height(&first_line));
            information(ui, &mut open, info_room, first_line, |ui| {
                if message(ui, &d, summary) {
                    ui.add_space(theme::XS);
                }
                ui.add(
                    egui::Label::new(
                        RichText::new(&d.author.email)
                            .size(m.small_size)
                            .color(p.text_dim),
                    )
                    .wrap(),
                );
                metadata(ui, &d, actions);
            });
            tab.history_layout.information_open = open;
            ui.add_space(theme::XS);
            file_stats(ui, &d);
            ui.add_space(theme::XS);
            let list = egui::ScrollArea::vertical()
                .id_salt("commit-file-list")
                .min_scrolled_height(0.0)
                .auto_shrink([false, false])
                .show(ui, |ui| files(ui, &d, tab));
            paint::scroll_fade(ui, &list, s.pane);
        });
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(s.pane))
        .show(ui, |ui| diff::show(ui, tab, actions));
}

/// Pane navigation belongs to the commit, including when its files have not loaded: the commit's
/// summary and the Expand / Back to history button on one header row. Returns whether it showed
/// the whole summary (false in a compact window, which has no header, or when it was cut).
fn header(ui: &mut Ui, tab: &mut RepoTab) -> bool {
    if tab.history_layout.compact {
        return false;
    }
    let p = palette(ui.ctx());
    let m = theme::metrics();
    egui::Panel::top("commit-pane-header")
        .exact_size(m.pane_header_height)
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            let font = FontId::new(m.body_size, theme::semibold());
            let row = ui.max_rect();
            paint_header_strip(ui, row);
            let inner = Rect::from_min_max(
                pos2(row.left() + PANE_HEADER_PAD.0, row.top()),
                pos2(row.right() - PANE_HEADER_PAD.1, row.bottom()),
            );
            let mut buttons = ui.new_child(
                UiBuilder::new()
                    .max_rect(inner)
                    .layout(Layout::right_to_left(Align::Center)),
            );
            let (glyph, label) = if tab.history_layout.expanded {
                (icon::ARROWS_IN_SIMPLE, "Back to history")
            } else {
                (icon::ARROWS_OUT_SIMPLE, "Expand")
            };
            let toggle = Button::new(label)
                .secondary()
                .size(Size::Sm)
                .icon(glyph)
                .id_salt("commit-pane-expand")
                .enabled(tab.selected_commit.is_some())
                .show(&mut buttons);
            let summary_rect = Rect::from_min_max(
                inner.min,
                pos2(toggle.rect.left() - theme::SM, inner.bottom()),
            );
            let mut summary_ui = ui.new_child(
                UiBuilder::new()
                    .max_rect(summary_rect)
                    .layout(Layout::left_to_right(Align::Center)),
            );
            summary_ui.set_clip_rect(summary_rect.intersect(ui.clip_rect()));
            let summary = tab.details.as_ref().map_or("Commit details", |d| {
                d.message.lines().next().unwrap_or("").trim()
            });
            let whole = ui
                .fonts_mut(|f| f.layout_no_wrap(summary.to_owned(), font.clone(), p.text))
                .size()
                .x
                <= summary_rect.width();
            summary_ui
                .add(egui::Label::new(RichText::new(summary).font(font).color(p.text)).truncate())
                .on_hover_text(summary);
            if toggle.clicked() {
                tab.history_layout.expanded = !tab.history_layout.expanded;
                tab.history_layout.compact_details = true;
            }
            whole && tab.details.is_some()
        })
        .inner
}

/// Left and right insets of the detail header's content (concept 1's `.dh`).
const PANE_HEADER_PAD: (f32, f32) = (14.0, 10.0);

/// The detail header's ground: the chrome step over a seam in Standard; in Deeper the card's own
/// ground (the header sits inside the floating pane) over the same seam.
pub(super) fn paint_header_strip(ui: &Ui, rect: Rect) {
    let p = palette(ui.ctx());
    let s = p.s();
    let fill = if theme::style() == Style::Deeper {
        s.pane
    } else {
        s.panel
    };
    ui.painter().rect_filled(rect, CornerRadius::ZERO, fill);
    paint::seam_h(
        ui.painter(),
        rect.x_range(),
        rect.bottom() - paint::SEAM * 0.5,
        s.border,
    );
}

/// The "Commit information" disclosure: open by default in every density (it keeps the user's
/// choice), its body flush with the author row. The body takes its natural height up to `room`
/// (what the changed files leave), never less than one headline line, and scrolls beyond that
/// with a faded edge instead of cutting a line through its glyphs. It is never shorter than the
/// body's `first_line`, so opening it always shows the start of the message.
fn information(
    ui: &mut Ui,
    open: &mut bool,
    room: f32,
    first_line: f32,
    body: impl FnOnce(&mut Ui),
) {
    const LABEL: &str = "Commit information";
    let s = palette(ui.ctx()).s();
    let id = ui.id().with("commit-information");
    let row = TreeRow::new("commit-information", 0)
        .kind(RowKind::List)
        .label(LABEL)
        .toggle_on_click(true)
        .open(open)
        .show(ui)
        .response;
    let shown = *open;
    // A disclosure is a collapsing header, not an item of a tree.
    row.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::CollapsingHeader, true, LABEL));
    ui.ctx().accesskit_node_builder(row.id, |n| {
        n.clear_selected();
        n.set_expanded(shown);
    });
    tree_children(ui, id, shown, 0, |ui| {
        ui.add_space(theme::XS);
        let out = egui::ScrollArea::vertical()
            .id_salt("commit-metadata")
            .max_height(room.max(first_line))
            .min_scrolled_height(0.0)
            .auto_shrink([false, true])
            .show(ui, body);
        paint::scroll_fade(ui, &out, s.pane);
        ui.add_space(theme::XS);
    });
}

/// The commit message: its summary as a headline when `summary` (the pane header does not show
/// it whole), then the rest of the message. Returns whether it drew anything.
fn message(ui: &mut Ui, d: &CommitDetails, summary: bool) -> bool {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let mut lines = d.message.trim().splitn(2, '\n');
    let head = lines.next().unwrap_or("").trim();
    let body = lines.next().unwrap_or("").trim();
    if summary {
        ui.add(
            egui::Label::new(
                RichText::new(head)
                    .font(FontId::new(m.headline_size, theme::semibold()))
                    .color(p.text),
            )
            .wrap(),
        );
    }
    if !body.is_empty() {
        if summary {
            ui.add_space(theme::XS);
        }
        ui.add(egui::Label::new(RichText::new(body).color(p.text_dim).size(m.meta_size)).wrap());
    }
    summary || !body.is_empty()
}

fn author(ui: &mut Ui, d: &CommitDetails, tab: &mut RepoTab) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    ui.horizontal(|ui| {
        tab.request_avatar(d.oid, &d.author.email);
        let picture = tab
            .avatars
            .get(&d.author.email)
            .map(egui::TextureHandle::id);
        Avatar::new(Person::new(&d.author.name, &d.author.email).picture(picture))
            .size(AvatarSize::Detail)
            .id_salt(("commit-author", &d.author.email))
            .show(ui);
        ui.add(
            egui::Label::new(
                RichText::new(&d.author.name)
                    .font(FontId::new(m.body_size, theme::semibold()))
                    .color(p.text),
            )
            .truncate(),
        )
        .on_hover_text(format!("{} <{}>", d.author.name, d.author.email));
    });
}

fn metadata(ui: &mut Ui, d: &CommitDetails, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
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
            ui.label(
                RichText::new(icon::GIT_COMMIT)
                    .size(m.icon_sm)
                    .color(p.text_faint),
            );
            ui.label(
                RichText::new(if d.parents.len() > 1 {
                    "Parents"
                } else {
                    "Parent"
                })
                .size(m.meta_size)
                .color(p.text_faint),
            );
            for parent in &d.parents {
                if LinkButton::new(short_id(*parent))
                    .text_size(m.meta_size)
                    .show(ui)
                    .on_hover_text(format!("Show commit {parent}"))
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
    let m = theme::metrics();
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
            .size(m.meta_size)
            .color(p.text_dim),
        );
        if d.stats_known {
            DiffStat::new(adds, dels).show(ui);
        } else {
            // A partial clone that has not downloaded the file contents.
            ui.label(
                RichText::new("line counts not downloaded")
                    .color(p.text_faint)
                    .size(m.small_size),
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
    virtual_rows(ui, d.files.len(), file_row_height(), |ui, i| {
        let f = &d.files[i];
        let selected = tab.file.as_ref().is_some_and(|s| s.entry.path == f.path);
        let id = ui.id().with(("commit-file", &f.path, &f.old_path, f.kind));
        if file_list_row(ui, f, selected, id).clicked() {
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
    let m = theme::metrics();
    // Git's message can be long and the pane short: scroll rather than clip the button.
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(theme::XL);
            ui.vertical_centered(|ui| {
                ui.set_max_width(ui.available_width() - theme::XXL);
                art::show(ui, art::empty_error(), vec2(120.0, 90.0));
                ui.label(
                    RichText::new(format!("Could not load commit {}", short_id(error.oid)))
                        .font(FontId::new(m.subtitle_size, theme::semibold())),
                );
                ui.label(
                    RichText::new(&error.message)
                        .size(m.meta_size)
                        .color(p.text_dim),
                );
                if error.can_download {
                    ui.add_space(theme::SM);
                    if Button::new("Retry")
                        .secondary()
                        .icon(icon::ARROWS_CLOCKWISE)
                        .show(ui)
                        .clicked()
                    {
                        tab.retry_details();
                    }
                }
                ui.add_space(theme::MD);
            });
        });
}

fn meta_row(ui: &mut Ui, glyph: &str, text: &str) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    ui.horizontal(|ui| {
        ui.label(RichText::new(glyph).size(m.icon_sm).color(p.text_faint));
        ui.add(egui::Label::new(RichText::new(text).size(m.meta_size).color(p.text_dim)).wrap());
    });
}

/// The rows of a fixed-height list (`row_h` apart, no spacing) whose top is at `top`, that
/// intersect `min_y..max_y`.
pub fn visible_rows(n: usize, row_h: f32, top: f32, min_y: f32, max_y: f32) -> Range<usize> {
    if n == 0 || row_h <= 0.0 || max_y <= min_y {
        return 0..0;
    }
    let first = ((min_y - top) / row_h).floor().max(0.0);
    let end = ((max_y - top) / row_h).ceil().max(0.0);
    // Float to usize saturates; clamp to the list.
    let first = crate::ui::saturate_usize(first).min(n);
    first..crate::ui::saturate_usize(end).clamp(first, n)
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
        ui.allocate_space(vec2(width, crate::ui::count_f32(range.start) * row_h));
    }
    for i in range.clone() {
        row(ui, i);
    }
    if range.end < n {
        ui.allocate_space(vec2(width, crate::ui::count_f32(n - range.end) * row_h));
    }
    ui.spacing_mut().item_spacing.y = spacing;
}

/// Height of [`file_list_row`] in the active density (`row_height`).
pub fn file_row_height() -> f32 {
    RowKind::File.height()
}

/// A changed-file row: the status letter, the name, its folder and the line counts, as a kit
/// [`ListRow`] (selection is the wash, with no side bar). `id` identifies the row across frames
/// (for hover and its context menu). Lists of these are [`file_row_height`] apart.
pub fn file_list_row(ui: &mut Ui, f: &FileEntry, selected: bool, id: egui::Id) -> egui::Response {
    file_list_row_inner(ui, f, selected, id, None)
}

/// [`file_list_row`] with actions revealed at its right end while it is hovered, selected or
/// focused: `actions` adds `IconButton::size(Size::Sm)`s right to left.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the changes list calls it once its migration lands"
    )
)]
pub fn file_list_row_with<'a>(
    ui: &mut Ui,
    f: &FileEntry,
    selected: bool,
    id: egui::Id,
    actions: impl FnOnce(&mut Ui) + 'a,
) -> egui::Response {
    file_list_row_inner(ui, f, selected, id, Some(Box::new(actions)))
}

/// Room the row keeps at its right end for revealed actions: two small icon buttons, as
/// `ListRow::actions` reserves.
fn row_actions_width() -> f32 {
    (theme::metrics().control_sm + 2.0) * 2.0
}

/// A file row's hover-revealed actions.
type RowActions<'a> = Box<dyn FnOnce(&mut Ui) + 'a>;

fn file_list_row_inner(
    ui: &mut Ui,
    f: &FileEntry,
    selected: bool,
    id: egui::Id,
    actions: Option<RowActions<'_>>,
) -> egui::Response {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let (name, dir) = widgets::split_path(&f.path);
    let (badge_fill, badge_ink) = ChangeBadge::new(f.kind).colors(p);
    let stats = (f.additions + f.deletions > 0).then(|| diffstat_galley(ui, f));
    let name_font = FontId::new(m.body_size, theme::medium());
    let dir_font = FontId::proportional(m.meta_size);
    let reserve = if actions.is_some() {
        row_actions_width()
    } else {
        0.0
    };
    let tip = match &f.old_path {
        Some(old) => format!("{} → {}", old, f.path),
        None => f.path.clone(),
    };
    let mut row = ListRow::new(id)
        .kind(RowKind::File)
        .selected(selected)
        .tooltip(tip);
    if let Some(add) = actions {
        row = row.actions(add);
    }
    let response = row.show_with(ui, |rc, painter| {
        let cy = rc.rect.center().y;
        let badge = Rect::from_min_size(
            pos2(rc.content.left(), cy - CHANGE_BADGE / 2.0),
            vec2(CHANGE_BADGE, CHANGE_BADGE),
        );
        painter.rect_filled(badge, CornerRadius::same(4), badge_fill);
        painter.text(
            badge.center(),
            Align2::CENTER_CENTER,
            f.kind.letter(),
            FontId::new(CHANGE_LETTER, egui::FontFamily::Monospace),
            badge_ink,
        );
        let mut right = rc.content.right() - reserve;
        if let Some(g) = stats {
            right -= g.size().x;
            painter.galley(pos2(right, cy - g.size().y / 2.0), g, p.text);
            right -= ROW_GAP;
        }
        let x = badge.right() + ROW_GAP;
        let end = paint_line(painter, x, right, cy, name, name_font, rc.text);
        if !dir.is_empty() {
            paint_line(
                painter,
                end + ROW_GAP,
                right,
                cy,
                dir,
                dir_font,
                p.text_faint,
            );
        }
    });
    // The row paints its own columns; screen readers get the file's path.
    ui.ctx()
        .accesskit_node_builder(response.id, |n| n.set_label(f.path.clone()));
    response
}

/// The status letter's square (concept 1's `.fs`, as `kit::chip::ChangeBadge`) and its letter.
const CHANGE_BADGE: f32 = 17.0;
const CHANGE_LETTER: f32 = 10.0;
/// Gap between a file row's badge, name, folder and counts (concept 1's `.frow`).
const ROW_GAP: f32 = 8.0;

/// `+n −n` as `kit::chip::DiffStat` paints it: mono in `success` and `danger` at 85%.
fn diffstat_galley(ui: &Ui, f: &FileEntry) -> std::sync::Arc<egui::Galley> {
    let p = palette(ui.ctx());
    let font = FontId::new(theme::metrics().sha_size, egui::FontFamily::Monospace);
    let mut job = egui::text::LayoutJob::default();
    let fmt = |c| egui::TextFormat::simple(font.clone(), c);
    job.append(&format!("+{}", f.additions), 0.0, fmt(p.success));
    job.append(
        &format!("−{}", f.deletions),
        6.0,
        fmt(p.danger.gamma_multiply(0.85)),
    );
    ui.painter().layout_job(job)
}

/// Paints `text` on one line from `x`, ellipsized before `right`; returns where it ends.
fn paint_line(
    painter: &egui::Painter,
    x: f32,
    right: f32,
    cy: f32,
    text: &str,
    font: FontId,
    color: egui::Color32,
) -> f32 {
    if right <= x {
        return x;
    }
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), font, color, right - x);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let g = painter.layout_job(job);
    let end = x + g.size().x;
    painter.galley(pos2(x, cy - g.size().y / 2.0), g, color);
    end
}

#[cfg(test)]
mod tests {
    use canopy_core::{ChangeKind, FileEntry};
    use egui_phosphor::regular as icon;

    use super::{file_list_row, file_list_row_with, file_row_height, visible_rows};
    use crate::ui::kit::{IconButton, Size};
    use crate::ui::theme::{self, Appearance, Density, ThemeId};

    fn entry() -> FileEntry {
        FileEntry {
            path: "src/views/history.rs".into(),
            old_path: None,
            kind: ChangeKind::Modified,
            additions: 12,
            deletions: 3,
            lfs: false,
            old_mode: Some(0o100_644),
            new_mode: Some(0o100_644),
        }
    }

    fn ctx(density: Density) -> egui::Context {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(
            &ctx,
            ThemeId::Midnight,
            Appearance {
                density,
                ..Appearance::STILL
            },
        );
        ctx
    }

    /// Labels and selection of the AccessKit nodes one frame of `f` produced.
    fn spoken(ctx: &egui::Context, mut f: impl FnMut(&mut egui::Ui)) -> Vec<(String, bool)> {
        ctx.enable_accesskit();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| f(ui));
        out.textures_delta.clear();
        out.platform_output
            .accesskit_update
            .map(|u| {
                u.nodes
                    .iter()
                    .filter_map(|(_, n)| {
                        n.label()
                            .map(|l| (l.to_owned(), n.is_selected().unwrap_or(false)))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn file_rows_follow_the_density() {
        for (density, height) in [(Density::Comfortable, 30.0), (Density::Dense, 24.0)] {
            let ctx = ctx(density);
            let mut drawn = 0.0;
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                assert_eq!(file_row_height(), height, "{density:?}");
                drawn = file_list_row(ui, &entry(), false, egui::Id::new("row"))
                    .rect
                    .height();
            });
            out.textures_delta.clear();
            assert_eq!(drawn, height, "{density:?}");
        }
    }

    #[test]
    fn a_file_row_speaks_its_path_and_selection() {
        let ctx = ctx(Density::Comfortable);
        let f = entry();
        let nodes = spoken(&ctx, |ui| {
            file_list_row(ui, &f, true, egui::Id::new("row"));
        });
        assert!(
            nodes.contains(&("src/views/history.rs".to_owned(), true)),
            "{nodes:?}"
        );
    }

    #[test]
    fn a_selected_file_row_reveals_its_actions() {
        let ctx = ctx(Density::Comfortable);
        let f = entry();
        let mut nodes = Vec::new();
        for _ in 0..2 {
            nodes = spoken(&ctx, |ui| {
                file_list_row_with(ui, &f, true, egui::Id::new("row"), |ui| {
                    IconButton::new(icon::PLUS, "Stage file")
                        .size(Size::Sm)
                        .show(ui);
                });
            });
        }
        assert!(
            nodes.iter().any(|(l, _)| l == "Stage file"),
            "the action is shown on a selected row: {nodes:?}"
        );
        assert!(nodes.iter().any(|(l, _)| l == "src/views/history.rs"));
    }

    /// qa-a F1/F15, qa-b F2: one rule in every density. The block opens even when the pane has
    /// no room to spare (it showed open in Dense and shut in Comfortable), shows at least a whole
    /// headline line, and its body starts at the column's edge, not under the disclosure label.
    #[test]
    fn commit_information_opens_in_every_density_flush_with_the_column() {
        for density in [Density::Comfortable, Density::Dense] {
            let ctx = ctx(density);
            let mut body = None;
            let mut column = 0.0;
            let mut shown = 0.0;
            for _ in 0..3 {
                let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                    column = ui.max_rect().left();
                    let mut open = true;
                    let line = ui.text_style_height(&egui::TextStyle::Body);
                    super::information(ui, &mut open, 0.0, line, |ui| {
                        body = Some(ui.label("fix(retry): tune defaults").rect);
                    });
                    shown = ui.min_rect().height();
                });
                out.textures_delta.clear();
            }
            let body = body.unwrap_or_else(|| panic!("{density:?}: open with no room to spare"));
            assert!(
                (body.left() - column).abs() < 0.5,
                "{density:?}: body at {} against the column at {column}",
                body.left()
            );
            assert!(
                shown >= super::RowKind::List.height() + body.height(),
                "{density:?}: the row and a whole first line show, got {shown}"
            );
        }
    }

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
