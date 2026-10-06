//! The commit graph: a virtualised table with coloured lanes, ref pills and the details pane.

use egui::{Align, FontId, Layout, Rect, RichText, Ui, UiBuilder, pos2, vec2};
use egui_phosphor::regular as icon;

use canopy_core::ops::ResetMode;
use canopy_core::{Oid, short_id};

use canopy_core::forge::Forge;

use crate::action::Action;
use crate::keymap::Command;
use crate::settings::Settings;
use crate::tab::RepoTab;
use crate::ui::kit::avatar::{self, Person};
use crate::ui::kit::chip::RefPill;
use crate::ui::kit::choice::Segmented;
use crate::ui::kit::field::SearchField;
use crate::ui::kit::graph::{self, Lanes, Marks};
use crate::ui::kit::menu::{self, Menu};
use crate::ui::kit::overlay::TooltipExt;
use crate::ui::kit::progress::Spinner;
use crate::ui::kit::row::{ListRow, RowKind, caps_galley};
use crate::ui::kit::surface::{EmptyState, HeaderKind, PaneHeader};
use crate::ui::kit::{Button, IconButton, paint};
use crate::ui::theme::{self, palette};
use crate::ui::widgets::forge_icon;
use crate::ui::{art, widgets};
use crate::views::{commit_detail, dialogs::Dialog, diff, state_banner};
use crate::worker::Op;

/// Lanes past this many are clipped off the graph column.
const MAX_LANES: u16 = 18;
/// Left inset of a table cell's content (concept 1's `.h-row > span`).
const CELL_PAD: f32 = 10.0;
/// Left inset of the graph column's header label.
const GRAPH_LABEL_X: f32 = 12.0;
/// Gap between ref pills, and between the last pill and the summary.
const PILL_GAP: f32 = 6.0;
/// Gap between an author's avatar and name.
const AVATAR_GAP: f32 = 8.0;
/// The id `RepoTab::filter_history` focuses; the filter field takes focus from it.
const FILTER_ID: &str = "history-filter";

/// Which half of History a compact window shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompactPane {
    History,
    Details,
}

/// Draws the history view.
pub fn show(ui: &mut Ui, tab: &mut RepoTab, settings: &Settings, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let s = p.s();
    let row_h = RowKind::History.height();
    if tab.snap.is_none() {
        loading(ui, tab);
        return;
    }
    state_banner::show(ui, tab, actions);
    if tab
        .snap
        .as_ref()
        .is_some_and(|snap| snap.commits.is_empty())
    {
        tab.history_layout.compact_details = false;
        tab.history_layout.expanded = false;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(s.pane))
            .show(ui, |ui| table(ui, tab, settings, actions));
        return;
    }
    let h = ui.available_height();
    let compact_table_min = row_h * 6.0 + theme::XL + theme::MD;
    let table_preferred = if tab.history_filter_open {
        row_h * 6.0 + theme::XL * 3.0 + theme::LG
    } else {
        compact_table_min
    };
    let table_required = table_preferred - row_h * 4.0;
    let compact = h < 260.0 + table_required;
    let table_min = table_preferred.min((h - 260.0).max(table_required));
    tab.history_layout.compact = compact;
    let mut layout = tab.history_layout;
    let selected = tab.selected_commit.is_some();
    if compact {
        egui::Panel::top("history-detail-controls")
            .exact_size(theme::XL + theme::SM)
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let strip = ui.max_rect();
                commit_detail::paint_header_strip(ui, strip);
                let mut row = ui.new_child(
                    UiBuilder::new()
                        .max_rect(strip.shrink2(vec2(theme::MD, 0.0)))
                        .layout(Layout::left_to_right(Align::Center)),
                );
                let before = if layout.compact_details && selected {
                    CompactPane::Details
                } else {
                    CompactPane::History
                };
                let mut pane = before;
                Segmented::new(
                    &mut pane,
                    &[
                        (CompactPane::History, "History"),
                        (CompactPane::Details, "Commit details"),
                    ],
                )
                .label("History pane")
                .id_salt("history-compact-pane")
                .enabled(selected)
                .disabled_reason("Select a commit to see its details")
                .show(&mut row);
                match (before, pane) {
                    (CompactPane::Details, CompactPane::History) => {
                        layout.compact_details = false;
                        layout.expanded = false;
                        tab.history_layout.expanded = false;
                    }
                    (CompactPane::History, CompactPane::Details) => layout.compact_details = true,
                    _ => {}
                }
                if layout.compact_details && selected {
                    let summary = tab.details.as_ref().map_or("Commit details", |d| {
                        d.message.lines().next().unwrap_or("").trim()
                    });
                    row.add_space(theme::SM);
                    row.add(egui::Label::new(RichText::new(summary).color(p.text_dim)).truncate())
                        .kit_tooltip(summary)
                        .show();
                }
            });
    }
    if compact || layout.expanded {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(s.pane))
            .show(ui, |ui| {
                if (layout.expanded || layout.compact_details) && selected {
                    commit_detail::show(ui, tab, settings, actions);
                } else if table(ui, tab, settings, actions) {
                    layout.compact_details = true;
                }
            });
    } else {
        let detail_max = (h - table_min).min(h * 0.70);
        let detail_min = 260.0_f32.min(detail_max);
        let panel_id = egui::Id::new("details");
        let automatic_id = panel_id.with("automatic-height");
        let previous_default = ui.ctx().data_mut(|d| d.get_persisted::<f32>(automatic_id));
        let fallback = (h * 0.40).max(260.0).min(detail_max);
        // Fitted to the open diff. Without one (the selected commit's details or diff still on
        // their way, a commit with no files, an error) the pane keeps the height it has: a click
        // must not show the pane at another height for the frames before the diff arrives.
        let default_height = match &tab.diff {
            Some(d) => diff::inspector_height(d, fallback)
                .max(260.0)
                .min(detail_max),
            None => {
                previous_default.map_or(fallback, |previous| previous.clamp(detail_min, detail_max))
            }
        };
        let state = egui::containers::panel::PanelState::load(ui.ctx(), panel_id);
        let automatic = state.is_none()
            || state
                .zip(previous_default)
                .is_some_and(|(state, previous)| (state.size().y - previous).abs() <= 1.0);
        // Kept with the panel's own state, so the pane still fits its diffs after a restart.
        if automatic {
            ui.ctx().data_mut(|data| {
                data.insert_persisted(automatic_id, default_height);
                if let Some(mut state) = state {
                    state.outer_rect.min.y = state.outer_rect.max.y - default_height;
                    data.insert_persisted(panel_id, state);
                }
            });
        } else {
            ui.ctx().data_mut(|data| data.remove::<f32>(automatic_id));
        }
        egui::Panel::bottom("details")
            .resizable(true)
            .show_separator_line(false)
            .default_size(default_height)
            .min_size(detail_min)
            .max_size(detail_max)
            .frame(egui::Frame::new().fill(s.pane))
            .show(ui, |ui| {
                let top = ui.max_rect();
                commit_detail::show(ui, tab, settings, actions);
                // The seam between the table and the details (concept 1's `.h-det` top border).
                paint::seam_h(
                    ui.painter(),
                    top.x_range(),
                    top.top() + paint::SEAM * 0.5,
                    s.border,
                );
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(s.pane))
            .show(ui, |ui| table(ui, tab, settings, actions));
    }
    tab.history_layout.compact_details = layout.compact_details || tab.history_layout.expanded;
}

fn loading(ui: &mut Ui, tab: &RepoTab) {
    let p = palette(ui.ctx());
    if let Some(e) = &tab.load_error {
        EmptyState::new("Could not read this repository")
            .art(art::empty_error())
            .icon(icon::WARNING_OCTAGON)
            .body(e.clone())
            .show(ui);
        return;
    }
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.35);
        Spinner::new().label("the repository").show(ui);
        ui.label(RichText::new("Reading repository…").color(p.text_dim));
    });
}

/// The search field over the table, and its close button.
fn filter_bar(ui: &mut Ui, tab: &mut RepoTab) {
    let mut close_search = false;
    egui::Frame::new()
        .inner_margin(egui::Margin {
            left: 14,
            right: 14,
            top: 10,
            bottom: 8,
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                // The filter command focuses `FILTER_ID`; the field takes that focus over.
                let wants_focus = tab.focus_history_filter
                    || ui.memory(|m| m.has_focus(egui::Id::new(FILTER_ID)));
                let field = SearchField::new(&mut tab.filter)
                    .placeholder("Filter by message, author or SHA…")
                    .name("Filter commits")
                    .shortcut(Command::Filter)
                    .width((ui.available_width() - 48.0).clamp(120.0, 340.0))
                    .id_salt(FILTER_ID)
                    .show(ui);
                if wants_focus {
                    field.request_focus();
                    tab.focus_history_filter = false;
                }
                ui.data_mut(|d| d.insert_temp(filter_field_key(), field.id));
                close_search = IconButton::new(icon::X, "Close search")
                    .id_salt("history-filter-close")
                    .show(ui)
                    .clicked();
            });
        });
    if close_search {
        tab.close_history_filter(ui.ctx());
    }
}

/// Where the filter field's own id is kept, for code that asks whether it has focus.
fn filter_field_key() -> egui::Id {
    egui::Id::new(FILTER_ID).with("field")
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

fn table(ui: &mut Ui, tab: &mut RepoTab, settings: &Settings, actions: &mut Vec<Action>) -> bool {
    let p = palette(ui.ctx());
    let s = p.s();
    let m = theme::metrics();
    let row_h = RowKind::History.height();
    if tab.history_filter_open {
        filter_bar(ui, tab);
    }

    // Refresh the filter cache, then borrow it (and everything else) without copying: a frame
    // must not do work proportional to the number of commits.
    tab.filtered_rows();
    let forge = tab.forge().map(|(_, f)| f.clone());
    let Some(snap) = tab.snap.clone() else {
        return false;
    };
    let filtered = tab.filtered.as_deref();
    let show_graph = filtered.is_none();
    let row_count = filtered.map_or(snap.commits.len(), <[usize]>::len);
    // Display row -> commit index.
    let commit_at = |ri: usize| filtered.map_or(Some(ri), |f| f.get(ri).copied());

    if snap.commits.is_empty() {
        EmptyState::new("No commits yet")
            .art(art::empty_history())
            .icon(icon::GIT_COMMIT)
            .body("Stage some files in Changes and make the first commit.")
            .show(ui);
        return false;
    }

    let now = chrono::Utc::now().timestamp();
    let row_spacing = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = 0.0;
    let outer = ui.available_rect_before_wrap();
    let mut full = outer;
    full.max.x -= theme::LG;
    let header = PaneHeader::new(HeaderKind::Table).top_of_pane(!tab.history_filter_open);
    let view_h = (full.height() - header.height()).max(row_h);
    let scroll_id = ui.make_persistent_id(egui::IdSalt::new("history-rows"));
    let mut offset =
        egui::scroll_area::State::load(ui.ctx(), scroll_id).map_or(0.0, |s| s.offset.y);
    if tab.scroll_to_selected || tab.reveal_selected {
        let target = tab.selected_commit.and_then(|selected| {
            (0..row_count).find(|&ri| {
                commit_at(ri)
                    .and_then(|ci| snap.commits.get(ci))
                    .is_some_and(|c| c.oid == selected)
            })
        });
        if let Some(target) = target {
            let y = crate::ui::count_f32(target) * row_h;
            if tab.scroll_to_selected || y < offset || y + row_h > offset + view_h {
                offset = (y - view_h / 2.0 + row_h).max(0.0);
            }
        }
    }
    let total_rows = row_count + usize::from(snap.truncated && filtered.is_none());
    offset = offset.min((crate::ui::count_f32(total_rows) * row_h - view_h).max(0.0));
    tab.scroll_to_selected = false;
    tab.reveal_selected = false;
    let first = crate::ui::saturate_usize((offset / row_h).floor());
    let last = crate::ui::saturate_usize(((offset + view_h) / row_h).ceil());
    let date_text = |seconds| {
        if settings.relative_dates {
            widgets::relative_time_format(seconds, now, settings.iso_dates)
        } else {
            widgets::absolute_date_format(seconds, settings.iso_dates)
        }
    };
    let mut lanes = 1;
    for ri in first..last.min(row_count) {
        if let Some(graph) = commit_at(ri).and_then(|ci| snap.graph.get(ci)) {
            lanes = lanes.max(graph.width.clamp(1, MAX_LANES));
        }
    }
    let [_, author_col, date_col, sha_col] = m.history_columns;
    let graph_w = if show_graph {
        graph::column_width(lanes)
    } else {
        0.0
    };
    let geometry = Lanes::new(lanes);
    let show_author = full.width() >= 560.0;
    let show_date = full.width() >= 760.0;
    let show_sha = full.width() >= 460.0;
    let author_w = if show_author { author_col } else { 0.0 };
    let date_w = if show_date { date_col } else { 0.0 };
    let sha_w = if show_sha { sha_col } else { 0.0 };
    let desc_w = (full.width() - graph_w - author_w - date_w - sha_w).max(0.0);
    let desc_x = full.min.x + graph_w;
    let author_x = desc_x + desc_w;
    let date_x = author_x + author_w;
    let sha_x = date_x + date_w;
    let avatars = settings.show_avatars;

    // Column header: the table pane's own strip, distinct from the neutral pane edges in Deeper.
    let (hdr, _) =
        ui.allocate_exact_size(vec2(outer.width(), header.height()), egui::Sense::hover());
    header.paint(ui.painter(), hdr);
    for (x, t, shown) in [
        (full.min.x + GRAPH_LABEL_X, "Graph", show_graph),
        (desc_x + CELL_PAD, "Description", true),
        (author_x + CELL_PAD, "Author", show_author),
        (date_x + CELL_PAD, "Date", show_date),
        (sha_x + CELL_PAD, "Commit", show_sha),
    ] {
        if shown {
            let g = caps_galley(ui, t, p.text_faint);
            ui.painter()
                .galley(pos2(x, hdr.center().y - g.size().y / 2.0), g, p.text_faint);
        }
    }

    let mut clicked: Option<Oid> = None;
    let area = egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .id_salt("history-rows")
        .vertical_scroll_offset(offset);
    let mut wanted_pictures = Vec::new();
    let list = area.show_rows(ui, row_h, total_rows, |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for ri in range {
            let Some(ci) = commit_at(ri).filter(|_| ri < row_count) else {
                let (rect, _) =
                    ui.allocate_exact_size(vec2(full.width(), row_h), egui::Sense::hover());
                let mut slot = ui.new_child(
                    UiBuilder::new()
                        .id_salt("history-load-more")
                        .max_rect(rect)
                        .layout(
                            Layout::left_to_right(Align::Center).with_main_align(Align::Center),
                        ),
                );
                if Button::new("Load more commits")
                    .secondary()
                    .icon(icon::ARROW_DOWN)
                    .id_salt("history-load-more")
                    .show(&mut slot)
                    .clicked()
                {
                    actions.push(Action::LoadMore);
                }
                continue;
            };
            let Some(c) = snap.commits.get(ci) else {
                ui.allocate_space(vec2(full.width(), row_h));
                continue;
            };
            let selected = tab.selected_commit == Some(c.oid);
            let g = snap.graph.get(ci);
            let is_head = snap.head.oid == Some(c.oid);
            let lane = g.map_or(0, |g| g.color);
            let picture = (avatars && show_author).then(|| {
                // Copied only for an author whose lookup is due, not for every row every frame.
                if tab.avatars.wants_now(&c.author.email) {
                    wanted_pictures.push((c.oid, c.author.email.clone()));
                }
                tab.avatars
                    .get(&c.author.email)
                    .map(egui::TextureHandle::id)
            });

            // Ref pills, then the summary after them.
            let desc_right = author_x - CELL_PAD;
            let mut pills = Vec::new();
            let mut x = desc_x + CELL_PAD;
            for l in snap.refs.get(&c.oid).into_iter().flatten() {
                if x > desc_right - 40.0 {
                    break;
                }
                let pill = RefPill::from_label(l, lane);
                let left = x;
                x += pill.width(ui) + PILL_GAP;
                pills.push((pill, left));
            }
            let summary_x = x;
            let date = date_text(c.author.time);
            let response = ListRow::new(("history-row", c.oid))
                .kind(RowKind::History)
                .index(ri)
                .selected(selected)
                .show_with(ui, |rc, painter| {
                    let cy = rc.rect.center().y;
                    if show_graph && let Some(g) = g {
                        let column =
                            Rect::from_min_size(rc.rect.min, vec2(graph_w, rc.rect.height()));
                        let mut marks = Marks::new(rc.ground);
                        marks.head = is_head;
                        graph::paint_row(painter, column, geometry, g, marks);
                    }
                    let font = if selected || is_head {
                        FontId::new(m.body_size, theme::medium())
                    } else {
                        FontId::proportional(m.body_size)
                    };
                    paint_line(
                        painter, summary_x, desc_right, cy, &c.summary, font, rc.text,
                    );
                    if show_author {
                        let mut name_x = author_x + CELL_PAD;
                        if let Some(picture) = picture {
                            let d = m.avatar_row;
                            let person =
                                Person::new(&c.author.name, &c.author.email).picture(picture);
                            avatar::paint(
                                painter,
                                pos2(name_x + d / 2.0, cy),
                                d,
                                &person,
                                None,
                                Some(egui::Id::new(("history-avatar", &c.author.email))),
                            );
                            name_x += d + AVATAR_GAP;
                        }
                        paint_line(
                            painter,
                            name_x,
                            date_x - CELL_PAD,
                            cy,
                            &c.author.name,
                            FontId::proportional(m.body_size),
                            rc.dim,
                        );
                    }
                    if show_date {
                        paint_line(
                            painter,
                            date_x + CELL_PAD,
                            sha_x - CELL_PAD,
                            cy,
                            &date,
                            FontId::proportional(m.body_size),
                            rc.dim,
                        );
                    }
                    if show_sha {
                        paint_line(
                            painter,
                            sha_x + CELL_PAD,
                            full.max.x,
                            cy,
                            &short_id(c.oid),
                            FontId::monospace(m.sha_size),
                            rc.accent,
                        );
                    }
                });
            if !pills.is_empty() {
                let clip = Rect::from_min_max(
                    pos2(desc_x, response.rect.top()),
                    pos2(desc_right, response.rect.bottom()),
                )
                .intersect(ui.clip_rect());
                let painter = ui.painter().with_clip_rect(clip);
                for (pill, left) in &pills {
                    pill.paint(ui, &painter, *left, response.rect.center().y);
                }
            }
            // The row paints its own columns; screen readers get the commit in one sentence.
            let info = format!(
                "Commit {}, {}, by {}, {}",
                short_id(c.oid),
                c.summary,
                c.author.name,
                widgets::relative_time_format(c.author.time, now, settings.iso_dates)
            );
            ui.ctx()
                .accesskit_node_builder(response.id, |n| n.set_label(info));
            let response = response.on_hover_text_at_pointer(format!(
                "{}\n{} <{}>\n{}",
                c.summary,
                c.author.name,
                c.author.email,
                widgets::full_date(c.author.time, c.author.offset_minutes)
            ));
            if response.clicked() {
                clicked = Some(c.oid);
            }
            let oid = c.oid;
            let summary = c.summary.clone();
            menu::context(&response, |m| {
                commit_menu(m, oid, &summary, forge.as_ref(), actions);
            });
        }
    });
    ui.spacing_mut().item_spacing.y = row_spacing;

    paint::scroll_fade(ui, &list, s.pane);

    for (oid, email) in wanted_pictures {
        tab.request_avatar(oid, &email);
    }
    if let Some(oid) = clicked {
        tab.select_commit(oid);
        return true;
    }
    false
}

/// The context menu of a commit row.
pub fn commit_menu(
    m: &mut Menu<'_>,
    oid: Oid,
    summary: &str,
    forge: Option<&Forge>,
    actions: &mut Vec<Action>,
) {
    if let Some(f) = forge {
        if m.item(
            forge_icon(f.kind),
            format!("Open commit on {}", f.kind.name()),
        )
        .shortcut(Command::OpenOnForge)
        .show(m.ui)
        .clicked()
        {
            actions.push(Action::OpenUrl(f.commit_url(&oid.to_string())));
        }
        m.separator();
    }
    if m.item(icon::CHECK, "Check out (detached HEAD)")
        .show(m.ui)
        .clicked()
    {
        actions.push(Action::Op(Op::CheckoutDetached(oid)));
    }
    if m.item(icon::GIT_BRANCH, "New branch here…")
        .shortcut(Command::NewBranch)
        .show(m.ui)
        .clicked()
    {
        actions.push(Action::OpenDialog(Dialog::new_branch_at(
            crate::views::dialogs::Start::Commit(oid),
        )));
    }
    if m.item(icon::TAG, "New tag here…")
        .shortcut(Command::Tag)
        .show(m.ui)
        .clicked()
    {
        actions.push(Action::OpenDialog(Dialog::NewTag {
            name: String::new(),
            target: oid,
            message: String::new(),
        }));
    }
    m.separator();
    if m.item(icon::CHERRIES, "Cherry-pick").show(m.ui).clicked() {
        actions.push(Action::Op(Op::CherryPick(oid)));
    }
    if m.item(icon::ARROW_COUNTER_CLOCKWISE, "Revert")
        .show(m.ui)
        .clicked()
    {
        actions.push(Action::Op(Op::Revert(oid)));
    }
    m.submenu(
        icon::CLOCK_COUNTER_CLOCKWISE,
        "Reset current branch here",
        |m| {
            if m.item("", "Soft")
                .description("Keep index and working tree")
                .show(m.ui)
                .clicked()
            {
                actions.push(Action::Op(Op::Reset {
                    oid,
                    mode: ResetMode::Soft,
                }));
            }
            if m.item("", "Mixed")
                .description("Keep working tree")
                .show(m.ui)
                .clicked()
            {
                actions.push(Action::Op(Op::Reset {
                    oid,
                    mode: ResetMode::Mixed,
                }));
            }
            m.separator();
            if m.item("", "Hard reset…")
                .description("Discard all uncommitted changes")
                .danger()
                .show(m.ui)
                .clicked()
            {
                actions.push(Action::Confirm {
                    title: "Hard reset".into(),
                    body: format!(
                        "Move the current branch to {} and throw away every uncommitted \
                         change? This cannot be undone.",
                        short_id(oid)
                    ),
                    confirm: "Reset".into(),
                    danger: true,
                    op: Op::Reset {
                        oid,
                        mode: ResetMode::Hard,
                    },
                });
            }
        },
    );
    m.separator();
    if m.item(icon::COPY, "Copy SHA")
        .shortcut(Command::CopySha)
        .show(m.ui)
        .clicked()
    {
        actions.push(Action::Copy(oid.to_string()));
    }
    if m.item(icon::TEXT_ALIGN_LEFT, "Copy message")
        .show(m.ui)
        .clicked()
    {
        actions.push(Action::Copy(summary.to_owned()));
    }
}

#[cfg(test)]
mod tests {
    use egui::Pos2;

    use super::*;

    /// The id of the history filter's text field, once it has been shown.
    fn filter_field(ctx: &egui::Context) -> Option<egui::Id> {
        let id = ctx.data(|d| d.get_temp::<egui::Id>(filter_field_key()));
        assert!(id.is_some(), "the filter field has been shown");
        id
    }

    #[test]
    fn resizing_preserves_visible_commit_rows_above_saved_details() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let dir = tempfile::tempdir().expect("temporary tab");
        let mut tab = RepoTab::new(
            dir.path().into(),
            &ctx,
            100,
            crate::settings::DiffPrefs::default(),
        );
        tab.snap = Some(std::sync::Arc::new(canopy_core::RepoSnapshot {
            commits: (0..10)
                .map(|i| canopy_core::CommitSummary {
                    oid: Oid::from_bytes(&[i; 20]).expect("fixture oid"),
                    summary: format!("Commit {i}"),
                    author: canopy_core::Signature {
                        name: "Author".into(),
                        email: "author@example.com".into(),
                        time: 0,
                        offset_minutes: 0,
                    },
                    parents: Vec::new(),
                })
                .collect(),
            ..Default::default()
        }));
        for size in [vec2(1440.0, 900.0), vec2(600.0, 375.0)] {
            let mut visible = 0;
            for _ in 0..3 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ui| {
                        egui::Panel::top("test-actions")
                            .exact_size(96.0)
                            .show(ui, |_| {});
                        egui::Panel::bottom("test-status")
                            .exact_size(26.0)
                            .show(ui, |_| {});
                        show(ui, &mut tab, &Settings::default(), &mut Vec::new());
                    },
                );
                output.textures_delta.clear();
                visible = output
                    .shapes
                    .iter()
                    .filter(|shape| {
                        if let egui::Shape::Text(text) = &shape.shape
                            && ["Commit 0", "Commit 1"].contains(&text.galley.text())
                        {
                            return shape.clip_rect.contains_rect(text.visual_bounding_rect());
                        }
                        false
                    })
                    .count();
            }
            assert_eq!(
                visible, 2,
                "the saved detail pane must leave two complete commit rows at {size:?}"
            );
        }
    }
    #[test]
    fn populated_details_remain_visible_in_a_minimum_zoomed_window() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let (mut tab, _dir) = populated_tab(&ctx);
        for size in [vec2(562.5, 350.0), vec2(1440.0, 900.0), vec2(600.0, 375.0)] {
            let mut visible = Vec::new();
            for _ in 0..4 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ui| {
                        egui::Panel::top("test-actions")
                            .exact_size(96.0)
                            .show(ui, |_| {});
                        egui::Panel::bottom("test-status")
                            .exact_size(26.0)
                            .show(ui, |_| {});
                        egui::CentralPanel::default()
                            .frame(egui::Frame::new())
                            .show(ui, |ui| {
                                show(ui, &mut tab, &Settings::default(), &mut Vec::new());
                            });
                    },
                );
                output.textures_delta.clear();
                visible = output
                    .shapes
                    .iter()
                    .filter_map(|shape| {
                        if let egui::Shape::Text(text) = &shape.shape
                            && shape.clip_rect.contains_rect(text.visual_bounding_rect())
                        {
                            return Some(text.galley.text().to_owned());
                        }
                        None
                    })
                    .collect();
            }
            let labels = if size.y < 562.0 {
                &["Selected commit", "file.rs", "History", "Commit details"][..]
            } else {
                &["Selected commit", "file.rs", "Commit 0", "Commit 1"][..]
            };
            for label in labels {
                assert!(
                    visible.iter().any(|text| text == *label),
                    "{label} must remain visible at {size:?}; painted {visible:?}"
                );
            }
        }
    }
    fn populated_tab(ctx: &egui::Context) -> (RepoTab, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temporary tab");
        let mut tab = RepoTab::new(
            dir.path().into(),
            ctx,
            100,
            crate::settings::DiffPrefs::default(),
        );
        let author = canopy_core::Signature {
            name: "Author".into(),
            email: "author@example.com".into(),
            time: 0,
            offset_minutes: 0,
        };
        let oid = Oid::from_bytes(&[1; 20]).expect("fixture oid");
        tab.snap = Some(std::sync::Arc::new(canopy_core::RepoSnapshot {
            commits: (0..10)
                .map(|i| canopy_core::CommitSummary {
                    oid,
                    summary: format!("Commit {i}"),
                    author: author.clone(),
                    parents: Vec::new(),
                })
                .collect(),
            ..Default::default()
        }));
        let file = canopy_core::FileEntry {
            path: "file.rs".into(),
            old_path: None,
            kind: canopy_core::ChangeKind::Added,
            additions: 16,
            deletions: 0,
            lfs: false,
            old_mode: None,
            new_mode: Some(0o100_644),
        };
        tab.selected_commit = Some(oid);
        tab.details = Some(std::sync::Arc::new(canopy_core::CommitDetails {
            oid,
            message: "Selected commit".into(),
            author: author.clone(),
            committer: author,
            parents: Vec::new(),
            files: vec![file.clone()],
            stats_known: true,
        }));
        tab.file = Some(crate::tab::FileSelection {
            source: canopy_core::DiffSource::Commit(oid),
            entry: file,
            conflicted: false,
        });
        tab.diff = Some(std::sync::Arc::new(canopy_core::FileDiff {
            path: "file.rs".into(),
            hunks: vec![canopy_core::Hunk {
                header: "@@ -0,0 +1,16 @@".into(),
                old_start: 0,
                old_lines: 0,
                new_start: 1,
                new_lines: 16,
                lines: (0..16)
                    .map(|i| canopy_core::DiffLine {
                        kind: canopy_core::LineKind::Added,
                        old_no: None,
                        new_no: Some(i + 1),
                        text: format!("contents {i}"),
                    })
                    .collect(),
            }],
            ..Default::default()
        }));
        (tab, dir)
    }

    fn visible_details(ctx: &egui::Context, tab: &mut RepoTab, size: egui::Vec2) -> Vec<String> {
        let mut visible = Vec::new();
        for _ in 0..4 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ui| {
                    egui::Panel::top("test-actions")
                        .exact_size(96.0)
                        .show(ui, |_| {});
                    egui::Panel::bottom("test-status")
                        .exact_size(26.0)
                        .show(ui, |_| {});
                    egui::CentralPanel::default()
                        .frame(egui::Frame::new())
                        .show(ui, |ui| {
                            show(ui, tab, &Settings::default(), &mut Vec::new());
                        });
                },
            );
            output.textures_delta.clear();
            visible = output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape
                        && shape.clip_rect.contains_rect(text.visual_bounding_rect())
                    {
                        return Some(text.galley.text().to_owned());
                    }
                    None
                })
                .collect();
        }
        visible
    }

    #[test]
    fn truncated_history_can_scroll_to_the_load_more_control() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let (mut tab, _dir) = populated_tab(&ctx);
        std::sync::Arc::make_mut(tab.snap.as_mut().expect("snapshot")).truncated = true;
        let mut output = egui::FullOutput::default();
        for _ in 0..4 {
            output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(960.0, 200.0))),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let id = ui.make_persistent_id(egui::IdSalt::new("history-rows"));
                        let mut scroll =
                            egui::scroll_area::State::load(&ctx, id).unwrap_or_default();
                        scroll.offset.y = f32::MAX;
                        scroll.store(&ctx, id);
                        table(ui, &mut tab, &Settings::default(), &mut Vec::new());
                    });
                },
            );
            output.textures_delta.clear();
        }
        let control = output.shapes.iter().find(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().contains("Load more commits"))
        }).expect("load-more control at the end of truncated history");
        let egui::Shape::Text(text) = &control.shape else {
            panic!("text control");
        };
        assert!(
            control.clip_rect.contains_rect(text.visual_bounding_rect()),
            "the load-more control must be completely visible: {:?}, clip {:?}",
            text.visual_bounding_rect(),
            control.clip_rect
        );
    }

    #[test]
    fn growing_the_window_restores_readable_author_files_and_diff() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let (mut tab, _dir) = populated_tab(&ctx);
        visible_details(&ctx, &mut tab, vec2(562.5, 350.0));
        let visible = visible_details(&ctx, &mut tab, vec2(1440.0, 900.0));
        assert!(
            visible.iter().any(|text| text == "Author"),
            "author must be readable: {visible:?}"
        );
        assert!(
            visible.iter().any(|text| text == "1 file changed"),
            "changed files must be readable: {visible:?}"
        );
        assert!(
            visible
                .iter()
                .filter(|text| text.starts_with("contents "))
                .count()
                >= 5,
            "a useful amount of the diff must be readable: {visible:?}"
        );
    }

    #[test]
    fn compact_details_show_the_author_and_a_useful_diff() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let (mut tab, _dir) = populated_tab(&ctx);
        let visible = visible_details(&ctx, &mut tab, vec2(562.5, 350.0));
        assert!(
            visible.iter().any(|text| text == "Author"),
            "author must be readable: {visible:?}"
        );
        assert!(
            visible
                .iter()
                .filter(|text| text.starts_with("contents "))
                .count()
                >= 5,
            "a useful amount of the diff must be readable: {visible:?}"
        );
    }
    #[test]
    fn short_details_keep_unselected_changed_files_visible() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let (mut tab, _dir) = populated_tab(&ctx);
        let mut details = (**tab.details.as_ref().expect("fixture details")).clone();
        let mut other = details.files[0].clone();
        other.path = "other.rs".into();
        details.files.push(other);
        tab.details = Some(std::sync::Arc::new(details));
        for size in [vec2(562.5, 350.0), vec2(960.0, 600.0)] {
            let visible = visible_details(&ctx, &mut tab, size);
            assert!(
                visible.iter().any(|text| text == "other.rs"),
                "the unselected changed-file row must be fully visible at {size:?}: {visible:?}"
            );
        }
    }

    #[test]
    fn filtering_returns_from_full_details_and_focuses_the_filter() {
        for size in [vec2(562.5, 350.0), vec2(1440.0, 900.0)] {
            let ctx = egui::Context::default();
            crate::ui::fonts::install(&ctx);
            theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
            let (mut tab, _dir) = populated_tab(&ctx);
            tab.history_layout.expanded = true;
            visible_details(&ctx, &mut tab, size);
            tab.filter_history(&ctx);
            visible_details(&ctx, &mut tab, size);
            assert_eq!(
                ctx.memory(egui::Memory::focused),
                filter_field(&ctx),
                "the filter command must restore and focus the list at {size:?}"
            );
        }
    }

    #[test]
    fn history_search_stays_hidden_until_invoked_and_has_no_commit_count() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let (mut tab, _dir) = populated_tab(&ctx);
        tab.history_layout.show_list();
        let visible = visible_details(&ctx, &mut tab, vec2(1440.0, 900.0));
        assert!(
            visible
                .iter()
                .all(|text| !text.contains("Filter by message")),
            "the search box must be hidden initially: {visible:?}"
        );
        assert!(
            visible.iter().all(|text| text != "10 commits"),
            "the total commit count must not take header space: {visible:?}"
        );
    }

    #[test]
    fn filter_shortcut_toggles_search_and_clears_its_query_on_close() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let (mut tab, _dir) = populated_tab(&ctx);
        tab.filter_history(&ctx);
        let open = visible_details(&ctx, &mut tab, vec2(1440.0, 900.0));
        assert!(
            open.iter().any(|text| text.contains("Filter by message")),
            "the shortcut must reveal the search field: {open:?}"
        );
        assert_eq!(ctx.memory(egui::Memory::focused), filter_field(&ctx));
        tab.filter = "Commit 3".into();
        tab.filter_history(&ctx);
        let closed = visible_details(&ctx, &mut tab, vec2(1440.0, 900.0));
        assert!(
            closed
                .iter()
                .all(|text| !text.contains("Filter by message")),
            "the second shortcut must hide the search field: {closed:?}"
        );
        assert!(
            tab.filter.is_empty(),
            "closing search must restore all rows"
        );
        assert_ne!(
            ctx.memory(egui::Memory::focused),
            filter_field(&ctx),
            "closing search must return keyboard focus to History"
        );
    }

    #[test]
    fn search_returns_from_hidden_details_without_losing_the_query() {
        for (size, expanded, compact) in [
            (vec2(1440.0, 900.0), true, false),
            (vec2(562.5, 350.0), false, true),
        ] {
            let ctx = egui::Context::default();
            crate::ui::fonts::install(&ctx);
            theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
            let (mut tab, _dir) = populated_tab(&ctx);
            tab.filter_history(&ctx);
            tab.filter = "fix".into();
            tab.history_layout.expanded = expanded;
            tab.history_layout.compact = compact;
            tab.history_layout.compact_details = compact;
            tab.filter_history(&ctx);
            assert!(tab.history_filter_open, "search must reopen at {size:?}");
            assert_eq!(tab.filter, "fix", "the query must survive at {size:?}");
            let visible = visible_details(&ctx, &mut tab, size);
            assert!(visible.iter().any(|text| text == "fix"), "{visible:?}");
            assert_eq!(ctx.memory(egui::Memory::focused), filter_field(&ctx));
        }
    }

    #[test]
    fn opening_search_keeps_the_split_details_at_minimum_window_height() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let (mut tab, _dir) = populated_tab(&ctx);
        tab.history_layout.show_list();
        tab.filter_history(&ctx);
        let visible = visible_details(&ctx, &mut tab, vec2(900.0, 560.0));
        assert!(
            !tab.history_layout.compact,
            "opening search must not replace the split with a mode switch: {visible:?}"
        );
        assert!(visible.iter().any(|text| text == "file.rs"), "{visible:?}");
        assert!(visible.iter().any(|text| text == "Commit 0"), "{visible:?}");
    }

    #[test]
    fn a_short_diff_loaded_later_updates_the_automatic_inspector_height() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let (mut tab, _dir) = populated_tab(&ctx);
        let mut diff = (**tab.diff.as_ref().expect("fixture diff")).clone();
        diff.hunks[0].lines.truncate(7);
        tab.diff = None;
        let size = vec2(2560.0, 1440.0);
        visible_details(&ctx, &mut tab, size);
        tab.diff = Some(std::sync::Arc::new(diff));
        visible_details(&ctx, &mut tab, size);
        let panel = egui::containers::panel::PanelState::load(&ctx, egui::Id::new("details"))
            .expect("rendered inspector");
        assert!(
            panel.size().y <= 320.0,
            "a loaded short diff must reclaim the automatic empty pane space: {:?}",
            panel.size()
        );
    }

    /// The top of the details pane after one frame of History at `size`.
    fn details_top(ctx: &egui::Context, tab: &mut RepoTab, size: egui::Vec2) -> f32 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                ..Default::default()
            },
            |ui| {
                egui::Panel::top("test-actions")
                    .exact_size(96.0)
                    .show(ui, |_| {});
                egui::Panel::bottom("test-status")
                    .exact_size(26.0)
                    .show(ui, |_| {});
                egui::CentralPanel::default()
                    .frame(egui::Frame::new())
                    .show(ui, |ui| {
                        show(ui, tab, &Settings::default(), &mut Vec::new());
                    });
            },
        );
        output.textures_delta.clear();
        egui::containers::panel::PanelState::load(ctx, egui::Id::new("details"))
            .map_or(f32::NAN, |state| state.outer_rect.top())
    }

    /// O2: clicking another commit must not show the details pane at another height for the
    /// frames before the commit's details and diff arrive. Frame by frame, from the click until
    /// the new (equally short) diff is on screen, the pane's top stays where it was.
    #[test]
    fn selecting_a_commit_keeps_the_details_pane_in_place_while_it_loads() {
        for style in [theme::Style::Standard, theme::Style::Deeper] {
            for density in [theme::Density::Comfortable, theme::Density::Dense] {
                let ctx = egui::Context::default();
                crate::ui::fonts::install(&ctx);
                theme::apply(
                    &ctx,
                    theme::ThemeId::Midnight,
                    theme::Appearance {
                        style,
                        density,
                        ..theme::Appearance::STILL
                    },
                );
                let (mut tab, _dir) = populated_tab(&ctx);
                let mut diff = (**tab.diff.as_ref().expect("fixture diff")).clone();
                diff.hunks[0].lines.truncate(7);
                let diff = std::sync::Arc::new(diff);
                tab.diff = Some(std::sync::Arc::clone(&diff));
                let size = vec2(1440.0, 900.0);
                let mut settled = f32::NAN;
                for _ in 0..4 {
                    settled = details_top(&ctx, &mut tab, size);
                }
                let next = Oid::from_bytes(&[2; 20]).expect("fixture oid");
                let mut details = (**tab.details.as_ref().expect("fixture details")).clone();
                details.oid = next;
                let entry = details.files[0].clone();
                tab.select_commit(next);
                let mut tops = Vec::new();
                // The click: nothing of the new commit has arrived.
                for _ in 0..3 {
                    tops.push(details_top(&ctx, &mut tab, size));
                }
                // Its details arrive; the diff is still on its way.
                tab.details = Some(std::sync::Arc::new(details));
                tops.push(details_top(&ctx, &mut tab, size));
                // The diff arrives.
                tab.file = Some(crate::tab::FileSelection {
                    source: canopy_core::DiffSource::Commit(next),
                    entry,
                    conflicted: false,
                });
                tab.diff = Some(std::sync::Arc::clone(&diff));
                for _ in 0..3 {
                    tops.push(details_top(&ctx, &mut tab, size));
                }
                assert!(
                    tops.iter().all(|top| (top - settled).abs() < 0.5),
                    "{style:?} {density:?}: the details pane settled at {settled} and then \
                     moved, frame by frame: {tops:?}"
                );
            }
        }
    }

    /// eframe's storage in memory: what it writes to disk on exit.
    #[derive(Default)]
    struct Saved(std::collections::HashMap<String, String>);

    impl eframe::Storage for Saved {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }
        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }
        fn flush(&mut self) {}
    }

    /// A pane the user never resized still fits its diffs after a restart (egui keeps the panel's
    /// state across runs; the height it was fitted to must come with it, or the restored pane
    /// reads as resized by hand and stops following the diff).
    #[test]
    fn the_automatic_inspector_height_survives_a_restart() {
        let size = vec2(1440.0, 900.0);
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let (mut tab, _dir) = populated_tab(&ctx);
        let long = std::sync::Arc::clone(tab.diff.as_ref().expect("fixture diff"));
        let mut short = (*long).clone();
        short.hunks[0].lines.truncate(7);
        tab.diff = Some(std::sync::Arc::new(short));
        let mut short_top = f32::NAN;
        for _ in 0..4 {
            short_top = details_top(&ctx, &mut tab, size);
        }
        // What eframe saves on exit and restores at the next start, through its own storage.
        let mut saved = Saved::default();
        eframe::set_value(&mut saved, eframe::APP_KEY, &ctx.memory(Clone::clone));
        let restarted = egui::Context::default();
        let memory: egui::Memory =
            eframe::get_value(&saved, eframe::APP_KEY).expect("restored memory");
        restarted.memory_mut(|m| *m = memory);
        crate::ui::fonts::install(&restarted);
        theme::apply(
            &restarted,
            theme::ThemeId::Midnight,
            theme::Appearance::STILL,
        );
        let (mut tab, _dir) = populated_tab(&restarted);
        tab.diff = Some(long);
        let mut long_top = f32::NAN;
        for _ in 0..4 {
            long_top = details_top(&restarted, &mut tab, size);
        }
        assert!(
            long_top < short_top - 20.0,
            "after a restart the pane must still grow to a longer diff: {short_top} -> {long_top}"
        );
    }

    #[test]
    fn manual_inspector_height_survives_a_short_diff() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let (mut tab, _dir) = populated_tab(&ctx);
        let mut diff = (**tab.diff.as_ref().expect("fixture diff")).clone();
        diff.hunks[0].lines.truncate(7);
        tab.diff = None;
        let size = vec2(2560.0, 1440.0);
        visible_details(&ctx, &mut tab, size);
        let panel_id = egui::Id::new("details");
        let mut panel =
            egui::containers::panel::PanelState::load(&ctx, panel_id).expect("rendered inspector");
        panel.outer_rect.min.y = panel.outer_rect.max.y - 480.0;
        ctx.data_mut(|data| data.insert_persisted(panel_id, panel));
        tab.diff = Some(std::sync::Arc::new(diff));
        visible_details(&ctx, &mut tab, size);
        let panel =
            egui::containers::panel::PanelState::load(&ctx, panel_id).expect("manual inspector");
        assert!((panel.size().y - 480.0).abs() <= 1.0, "{:?}", panel.size());
    }

    #[test]
    fn a_short_commit_diff_does_not_leave_a_large_empty_bottom() {
        for size in [vec2(1440.0, 900.0), vec2(2560.0, 1440.0)] {
            let ctx = egui::Context::default();
            crate::ui::fonts::install(&ctx);
            theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
            let (mut tab, _dir) = populated_tab(&ctx);
            let mut diff = (**tab.diff.as_ref().expect("fixture diff")).clone();
            diff.hunks[0].lines.truncate(7);
            tab.diff = Some(std::sync::Arc::new(diff));
            let mut last_line_bottom = 0.0;
            for _ in 0..4 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ui| {
                        egui::Panel::top("test-actions")
                            .exact_size(96.0)
                            .show(ui, |_| {});
                        egui::Panel::bottom("test-status")
                            .exact_size(26.0)
                            .show(ui, |_| {});
                        egui::CentralPanel::default()
                            .frame(egui::Frame::new())
                            .show(ui, |ui| {
                                show(ui, &mut tab, &Settings::default(), &mut Vec::new());
                            });
                    },
                );
                output.textures_delta.clear();
                last_line_bottom = output
                    .shapes
                    .iter()
                    .find_map(|shape| {
                        if let egui::Shape::Text(text) = &shape.shape
                            && text.galley.text() == "contents 6"
                        {
                            Some(text.visual_bounding_rect().max.y)
                        } else {
                            None
                        }
                    })
                    .expect("last line of short diff");
            }
            let empty = size.y - 26.0 - last_line_bottom;
            assert!(
                empty <= 75.0,
                "short diff at {size:?} leaves {empty:.1} points unused"
            );
        }
    }
    #[test]
    fn expanded_details_can_return_without_a_selected_file() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let (mut tab, _dir) = populated_tab(&ctx);
        tab.history_layout.expanded = true;
        tab.file = None;
        tab.diff = None;
        for loading in [false, true] {
            if loading {
                tab.details = None;
            }
            let visible = visible_details(&ctx, &mut tab, vec2(1440.0, 900.0));
            assert!(
                visible.iter().any(|text| text.ends_with("Back to history")),
                "the pane needs a return control without a file, loading={loading}: {visible:?}"
            );
        }
    }

    #[test]
    fn detail_expander_shares_the_commit_summary_row() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let (mut tab, _dir) = populated_tab(&ctx);
        let size = vec2(1440.0, 900.0);
        visible_details(&ctx, &mut tab, size);
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                ..Default::default()
            },
            |ui| {
                egui::Panel::top("test-actions")
                    .exact_size(96.0)
                    .show(ui, |_| {});
                egui::Panel::bottom("test-status")
                    .exact_size(26.0)
                    .show(ui, |_| {});
                egui::CentralPanel::default()
                    .frame(egui::Frame::new())
                    .show(ui, |ui| {
                        show(ui, &mut tab, &Settings::default(), &mut Vec::new());
                    });
            },
        );
        let bounds = |needle: &str| {
            output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape
                        && text.galley.text().ends_with(needle)
                    {
                        Some(text.visual_bounding_rect())
                    } else {
                        None
                    }
                })
                .min_by(|a, b| a.min.y.total_cmp(&b.min.y))
                .expect("rendered control/fixture filename")
        };
        let filename = bounds("file.rs");
        let expand = bounds("Expand");
        let summary = bounds("Selected commit");
        assert!(
            (expand.center().y - summary.center().y).abs() <= theme::XS,
            "expand {expand:?} belongs on the commit summary row {summary:?}"
        );
        assert!(expand.center().x > summary.center().x);
        assert!(filename.min.y > expand.max.y);
        assert!(
            expand.max.x >= size.x - theme::XL * 2.0,
            "expand {expand:?} must sit at the right edge of the diff pane"
        );
    }

    #[test]
    fn long_commit_messages_stop_before_the_expander() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let (mut tab, _dir) = populated_tab(&ctx);
        let mut details = (**tab.details.as_ref().expect("fixture details")).clone();
        details.message = "very-long-commit-summary ".repeat(20);
        tab.details = Some(std::sync::Arc::new(details));
        let size = vec2(900.0, 560.0);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                ..Default::default()
            },
            |ui| {
                egui::Panel::top("test-actions")
                    .exact_size(96.0)
                    .show(ui, |_| {});
                egui::Panel::bottom("test-status")
                    .exact_size(26.0)
                    .show(ui, |_| {});
                egui::CentralPanel::default()
                    .frame(egui::Frame::new())
                    .show(ui, |ui| {
                        show(ui, &mut tab, &Settings::default(), &mut Vec::new());
                    });
            },
        );
        output.textures_delta.clear();
        let expand = output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape
                    && text.galley.text().ends_with("Expand")
                {
                    Some(text.visual_bounding_rect())
                } else {
                    None
                }
            })
            .expect("rendered Expand control");
        let summary = output
            .shapes
            .iter()
            .find(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().contains("very-long-commit-summary"))
            })
            .expect("rendered commit summary");
        assert!(
            summary.clip_rect.max.x <= expand.min.x - theme::XS,
            "the long summary may paint over Expand: summary clip {:?}, Expand {expand:?}",
            summary.clip_rect
        );
    }

    /// The history's ref pills keep their labels readable on every surface a row can have, in
    /// every theme and lane colour: local branches and tags in the neutral text colour, remote
    /// branches and detached HEAD in the dim one.
    #[test]
    fn rendered_ref_pills_use_readable_neutral_text_in_every_theme() {
        use canopy_core::{RefKind, RefLabel};
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        for id in theme::ThemeId::ALL {
            theme::apply(&ctx, id, theme::Appearance::STILL);
            let p = palette(&ctx);
            for kind in [
                RefKind::CurrentBranch,
                RefKind::LocalBranch,
                RefKind::RemoteBranch,
                RefKind::Tag,
                RefKind::Head,
            ] {
                for lane in (0u16..).take(p.lanes.len()) {
                    let label = RefLabel {
                        name: "feature/readability".into(),
                        kind,
                    };
                    let pill = RefPill::from_label(&label, lane);
                    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                        pill.paint(ui, ui.painter(), 20.0, 30.0);
                    });
                    output.textures_delta.clear();
                    let text = output
                        .shapes
                        .iter()
                        .filter_map(|shape| match &shape.shape {
                            egui::Shape::Text(text) => Some(text),
                            _ => None,
                        })
                        .find(|text| ["feature/readability", "HEAD"].contains(&text.galley.text()))
                        .expect("painted ref text");
                    let actual = text
                        .override_text_color
                        .unwrap_or(text.galley.job.sections[0].format.color);
                    let (fill, _, _) = pill.colors(p);
                    for surface in p.component_surfaces(p.s()) {
                        let ratio = theme::contrast(actual, theme::over(fill, surface));
                        assert!(
                            ratio >= 4.5,
                            "{} {kind:?} lane {lane}: painted ink {actual:?} contrast {ratio:.2}:1",
                            p.name
                        );
                    }
                    let tone = match kind {
                        RefKind::RemoteBranch | RefKind::Head => p.text_dim,
                        _ => p.text,
                    };
                    assert_eq!(
                        actual, tone,
                        "{} {kind:?} ref text keeps a consistent neutral tone",
                        p.name
                    );
                }
            }
        }
    }
}
