//! The commit graph: a virtualised table with coloured lanes, ref pills and the details pane.

use egui::{
    Align2, Color32, CornerRadius, FontId, Pos2, Rect, RichText, Sense, Stroke, Ui, pos2, vec2,
};
use egui_phosphor::regular as icon;

use canopy_core::ops::ResetMode;
use canopy_core::{GraphRow, LineEnd, Oid, short_id};

use canopy_core::forge::Forge;

use crate::action::Action;
use crate::settings::Settings;
use crate::tab::RepoTab;
use crate::ui::theme::{self, Palette, palette};
use crate::ui::widgets::forge_icon;
use crate::ui::{art, widgets};
use crate::views::{commit_detail, dialogs::Dialog};
use crate::worker::Op;

const ROW_H: f32 = 30.0;
const LANE_W: f32 = 16.0;
const MAX_LANES: u16 = 18;

/// Draws the history view.
pub fn show(ui: &mut Ui, tab: &mut RepoTab, settings: &Settings, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    if tab.snap.is_none() {
        loading(ui, tab);
        return;
    }
    let h = ui.available_height();
    let compact_table_min = ROW_H * 2.0 + theme::XL + theme::MD;
    let has_state_banner = tab
        .snap
        .as_ref()
        .is_some_and(|snap| snap.state != canopy_core::RepoState::Clean);
    let table_min = if tab.history_filter_open || has_state_banner {
        ROW_H * 2.0 + theme::XL * 3.0 + theme::LG
    } else {
        compact_table_min
    };
    let compact = h < 300.0
        + if has_state_banner {
            table_min
        } else {
            compact_table_min
        };
    tab.history_layout.compact = compact;
    let mut layout = tab.history_layout;
    let selected = tab.selected_commit.is_some();
    if compact {
        egui::Panel::top("history-detail-controls")
            .exact_size(theme::XL + theme::SM)
            .frame(
                egui::Frame::new()
                    .fill(p.panel)
                    .inner_margin(egui::Margin::symmetric(theme::MD as i8, theme::XS as i8)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(!layout.compact_details || !selected, "History")
                        .clicked()
                    {
                        layout.compact_details = false;
                        layout.expanded = false;
                        tab.history_layout.expanded = false;
                    }
                    ui.add_enabled_ui(selected, |ui| {
                        if ui
                            .selectable_label(layout.compact_details && selected, "Commit details")
                            .clicked()
                        {
                            layout.compact_details = true;
                        }
                    });
                });
            });
    }
    if compact || layout.expanded {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.bg))
            .show(ui, |ui| {
                if (layout.expanded || layout.compact_details) && selected {
                    commit_detail::show(ui, tab, actions);
                } else if table(ui, tab, settings, actions) {
                    layout.compact_details = true;
                }
            });
    } else {
        let detail_max = (h - table_min).min(h * 0.70);
        egui::Panel::bottom("details")
            .resizable(true)
            .default_size((h * 0.40).clamp(300.0, 320.0).min(detail_max))
            .min_size(260.0_f32.min(detail_max))
            .max_size(detail_max)
            .frame(
                egui::Frame::new()
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0, p.border)),
            )
            .show(ui, |ui| commit_detail::show(ui, tab, actions));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.bg))
            .show(ui, |ui| table(ui, tab, settings, actions));
    }
    tab.history_layout.compact_details = layout.compact_details || tab.history_layout.expanded;
}

fn loading(ui: &mut Ui, tab: &RepoTab) {
    let p = palette(ui.ctx());
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.35);
        if let Some(e) = &tab.load_error {
            art::show(ui, art::empty_error(), vec2(160.0, 120.0));
            ui.label(RichText::new("Could not read this repository").heading());
            ui.label(RichText::new(e).color(p.text_dim));
        } else {
            ui.spinner();
            ui.label(RichText::new("Reading repository…").color(p.text_dim));
        }
    });
}

fn table(ui: &mut Ui, tab: &mut RepoTab, settings: &Settings, actions: &mut Vec<Action>) -> bool {
    let p = palette(ui.ctx());
    let state = tab
        .snap
        .as_ref()
        .and_then(|snap| (snap.state != canopy_core::RepoState::Clean).then_some(snap.state));
    if tab.history_filter_open || state.is_some() {
        let mut close_search = false;
        egui::Frame::new()
            .inner_margin(egui::Margin {
                left: 14,
                right: 14,
                top: 10,
                bottom: 8,
            })
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if tab.history_filter_open {
                        let id = egui::Id::new("history-filter");
                        egui::Frame::new()
                            .fill(p.panel)
                            .stroke(Stroke::new(1.0, p.border))
                            .corner_radius(CornerRadius::same(theme::RADIUS))
                            .inner_margin(egui::Margin::symmetric(10, 4))
                            .show(ui, |ui| {
                                ui.label(RichText::new(icon::FUNNEL_SIMPLE).color(p.text_faint));
                                let edit = ui.add(
                                    egui::TextEdit::singleline(&mut tab.filter)
                                        .id(id)
                                        .hint_text("Filter by message, author or SHA")
                                        .frame(egui::Frame::NONE)
                                        .desired_width(
                                            (ui.available_width() - 48.0).clamp(120.0, 340.0),
                                        ),
                                );
                                if tab.focus_history_filter {
                                    edit.request_focus();
                                    tab.focus_history_filter = false;
                                }
                            });
                        close_search = widgets::icon_button(ui, icon::X, "Close search").clicked();
                    }
                    if let Some(state) = state {
                        state_banner(ui, state, actions);
                    }
                });
            });
        if close_search {
            tab.close_history_filter(ui.ctx());
        }
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
    let lanes = tab.snap_index.graph_lanes.clamp(1, MAX_LANES);
    let graph_w = if show_graph {
        (f32::from(lanes) * LANE_W + 16.0).min(ui.available_width() * 0.24)
    } else {
        0.0
    };

    if snap.commits.is_empty() {
        ui.vertical_centered(|ui| {
            ui.add_space(60.0);
            art::show(ui, art::empty_history(), vec2(160.0, 120.0));
            ui.add_space(8.0);
            ui.label(RichText::new("No commits yet").heading());
            ui.label(
                RichText::new("Stage some files in Changes and make the first commit.")
                    .color(p.text_dim),
            );
        });
        return false;
    }

    // Column layout.
    let full = ui.available_rect_before_wrap();
    let show_author = full.width() >= 560.0;
    let show_date = full.width() >= 760.0;
    let show_sha = full.width() >= 460.0;
    let sha_w = if show_sha { 74.0 } else { 0.0 };
    let date_w = if show_date { 112.0 } else { 0.0 };
    let author_w = if show_author {
        (full.width() * 0.17).clamp(110.0, 200.0)
    } else {
        0.0
    };
    let avatars = settings.show_avatars;
    let name_x = if avatars { 32.0 } else { 6.0 };
    let desc_x = full.min.x + graph_w + 10.0;
    let author_x = full.max.x - sha_w - date_w - author_w;
    let date_x = full.max.x - sha_w - date_w;
    let sha_x = full.max.x - sha_w;

    // Header.
    let (hdr, _) = ui.allocate_exact_size(vec2(full.width(), 22.0), Sense::hover());
    let hf = FontId::new(10.5, theme::semibold());
    for (x, t) in [
        (full.min.x + 12.0, if show_graph { "GRAPH" } else { "" }),
        (desc_x, "DESCRIPTION"),
        (author_x + 6.0, if show_author { "AUTHOR" } else { "" }),
        (date_x, if show_date { "DATE" } else { "" }),
        (sha_x, if show_sha { "COMMIT" } else { "" }),
    ] {
        ui.painter().text(
            pos2(x, hdr.center().y),
            Align2::LEFT_CENTER,
            t,
            hf.clone(),
            p.text_faint,
        );
    }
    ui.painter()
        .hline(full.x_range(), hdr.max.y, Stroke::new(1.0, p.border));

    // Keyboard navigation (keymap: j/k, g/G) moves the selection and asks to reveal it.
    // Only searched for when asked, not every frame.
    let scroll_target = if tab.scroll_to_selected || tab.reveal_selected {
        tab.selected_commit.and_then(|s| {
            (0..row_count).find(|&ri| {
                commit_at(ri)
                    .and_then(|ci| snap.commits.get(ci))
                    .is_some_and(|c| c.oid == s)
            })
        })
    } else {
        None
    };

    let now = chrono::Utc::now().timestamp();
    let mut clicked: Option<Oid> = None;
    let mut area = egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .id_salt("history-rows");
    if let Some(target) = scroll_target {
        let view_h = ui.available_height();
        let offset = (target as f32 * ROW_H - view_h / 2.0 + ROW_H).max(0.0);
        let current =
            egui::scroll_area::State::load(ui.ctx(), ui.make_persistent_id("history-rows"))
                .map(|s| s.offset.y);
        let visible = current.is_some_and(|c| {
            let y = target as f32 * ROW_H;
            y >= c && y + ROW_H <= c + view_h - 10.0
        });
        if !visible || tab.scroll_to_selected {
            area = area.vertical_scroll_offset(offset);
        }
    }
    tab.scroll_to_selected = false;
    tab.reveal_selected = false;

    let total_rows = row_count + usize::from(snap.truncated && filtered.is_none());
    area.show_rows(ui, ROW_H, total_rows, |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for ri in range {
            let Some(ci) = commit_at(ri).filter(|_| ri < row_count) else {
                let (rect, _) = ui.allocate_exact_size(vec2(full.width(), ROW_H), Sense::hover());
                let b = ui.put(
                    Rect::from_center_size(rect.center(), vec2(220.0, 24.0)),
                    egui::Button::new(
                        RichText::new(format!("{}  Load more commits", icon::ARROW_DOWN))
                            .color(p.accent),
                    ),
                );
                if b.clicked() {
                    actions.push(Action::LoadMore);
                }
                continue;
            };
            let Some(c) = snap.commits.get(ci) else {
                ui.allocate_space(vec2(full.width(), ROW_H));
                continue;
            };
            let (rect, resp) = ui.allocate_exact_size(vec2(full.width(), ROW_H), Sense::click());
            let selected = tab.selected_commit == Some(c.oid);
            let hover = ui
                .ctx()
                .animate_bool_with_time(resp.id, resp.hovered(), 0.1);
            let painter = ui.painter();
            if selected {
                painter.rect_filled(
                    rect,
                    CornerRadius::ZERO,
                    Palette::tint(p.accent, if p.dark { 46 } else { 30 }),
                );
                painter.rect_filled(
                    Rect::from_min_size(rect.min, vec2(3.0, ROW_H)),
                    CornerRadius::ZERO,
                    p.accent,
                );
            } else if hover > 0.0 {
                painter.rect_filled(
                    rect,
                    CornerRadius::ZERO,
                    Palette::tint(p.text, (hover * if p.dark { 10.0 } else { 14.0 }) as u8),
                );
            }

            let g = snap.graph.get(ci);
            let is_head = snap.head.oid == Some(c.oid);
            if show_graph && let Some(g) = g {
                paint_graph_row(
                    ui,
                    Rect::from_min_size(rect.min + vec2(10.0, 0.0), vec2(graph_w - 10.0, ROW_H)),
                    g,
                    is_head,
                    p,
                );
            }
            let lane_color = g.map(|g| p.lane(g.color)).unwrap_or(p.accent);

            // Description: ref pills then summary.
            let cy = rect.center().y;
            let desc_clip =
                Rect::from_min_max(pos2(desc_x, rect.min.y), pos2(author_x - 12.0, rect.max.y))
                    .intersect(ui.clip_rect());
            let mut x = desc_x;
            if let Some(labels) = snap.refs.get(&c.oid) {
                let clip_ui = ui.new_child(egui::UiBuilder::new().max_rect(desc_clip));
                let mut clip_ui = clip_ui;
                clip_ui.set_clip_rect(desc_clip);
                for l in labels {
                    if x > desc_clip.max.x - 40.0 {
                        break;
                    }
                    x += widgets::paint_ref_pill(&clip_ui, x, cy, l, lane_color) + 5.0;
                }
                x += 3.0;
            }
            let text_color = if selected || is_head {
                p.text
            } else {
                p.text.gamma_multiply(0.92)
            };
            let font = if is_head {
                FontId::new(13.0, theme::medium())
            } else {
                FontId::proportional(13.0)
            };
            let galley = ui
                .painter()
                .layout_no_wrap(c.summary.clone(), font, text_color);
            ui.painter().with_clip_rect(desc_clip).galley(
                pos2(x, cy - galley.size().y / 2.0),
                galley,
                text_color,
            );

            if show_author {
                // Author.
                if avatars {
                    widgets::paint_avatar(
                        ui,
                        pos2(author_x + 16.0, cy),
                        10.0,
                        &c.author.name,
                        &c.author.email,
                    );
                }
                let ag = ui.painter().layout_no_wrap(
                    c.author.name.clone(),
                    FontId::proportional(12.5),
                    p.text_dim,
                );
                ui.painter()
                    .with_clip_rect(
                        Rect::from_min_max(
                            pos2(author_x + name_x - 2.0, rect.min.y),
                            pos2(date_x - 8.0, rect.max.y),
                        )
                        .intersect(ui.clip_rect()),
                    )
                    .galley(
                        pos2(author_x + name_x, cy - ag.size().y / 2.0),
                        ag,
                        p.text_dim,
                    );
            }
            if show_date {
                ui.painter().text(
                    pos2(date_x, cy),
                    Align2::LEFT_CENTER,
                    if settings.relative_dates {
                        widgets::relative_time(c.author.time, now)
                    } else {
                        widgets::absolute_date(c.author.time)
                    },
                    FontId::proportional(12.5),
                    p.text_faint,
                );
            }
            if show_sha {
                ui.painter().text(
                    pos2(sha_x, cy),
                    Align2::LEFT_CENTER,
                    short_id(c.oid),
                    FontId::monospace(12.0),
                    if selected { p.accent2 } else { p.text_faint },
                );
            }

            let resp = resp.on_hover_text_at_pointer(format!(
                "{}\n{} <{}>\n{}",
                c.summary,
                c.author.name,
                c.author.email,
                widgets::full_date(c.author.time, c.author.offset_minutes)
            ));
            let info = format!(
                "Commit {}, {}, by {}, {}",
                short_id(c.oid),
                c.summary,
                c.author.name,
                widgets::relative_time(c.author.time, now)
            );
            resp.widget_info(|| {
                egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &info)
            });
            if resp.clicked() {
                clicked = Some(c.oid);
            }
            let oid = c.oid;
            let summary = c.summary.clone();
            resp.context_menu(|ui| commit_menu(ui, oid, &summary, forge.as_ref(), actions));
        }
    });

    if let Some(oid) = clicked {
        tab.select_commit(oid);
        return true;
    }
    false
}

fn state_banner(ui: &mut Ui, state: canopy_core::RepoState, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    if state.command().is_some() {
        if ui
            .button(RichText::new(format!("{}  Abort", icon::X_CIRCLE)).color(p.danger))
            .clicked()
        {
            actions.push(Action::Confirm { title: format!("Abort {}", state.label().to_lowercase()), body: "Return the repository to where it was before the operation started? Resolved conflicts are discarded.".into(), confirm: "Abort".into(), danger: true, op: Op::ContinueOrAbort { state, abort: true } });
        }
        if ui
            .button(RichText::new(format!("{}  Continue", icon::PLAY)).color(p.success))
            .clicked()
        {
            actions.push(Action::Op(Op::ContinueOrAbort {
                state,
                abort: false,
            }));
        }
    }
    egui::Frame::new()
        .fill(Palette::tint(p.warning, 40))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(egui::Margin::symmetric(10, 3))
        .show(ui, |ui| {
            ui.label(
                RichText::new(format!("{}  {} in progress", icon::WARNING, state.label()))
                    .color(p.warning)
                    .size(12.5),
            );
        });
}

/// The context menu of a commit row.
pub fn commit_menu(
    ui: &mut Ui,
    oid: Oid,
    summary: &str,
    forge: Option<&Forge>,
    actions: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    ui.set_min_width(240.0);
    if let Some(f) = forge {
        if ui
            .button(format!(
                "{}  Open commit on {}",
                forge_icon(f.kind),
                f.kind.name()
            ))
            .clicked()
        {
            actions.push(Action::OpenUrl(f.commit_url(&oid.to_string())));
        }
        ui.separator();
    }
    if ui
        .button(format!("{}  Check out (detached HEAD)", icon::CHECK))
        .clicked()
    {
        actions.push(Action::Op(Op::CheckoutDetached(oid)));
    }
    if ui
        .button(format!("{}  New branch here…", icon::GIT_BRANCH))
        .clicked()
    {
        actions.push(Action::OpenDialog(Dialog::new_branch_at(
            crate::views::dialogs::Start::Commit(oid),
        )));
    }
    if ui.button(format!("{}  New tag here…", icon::TAG)).clicked() {
        actions.push(Action::OpenDialog(Dialog::NewTag {
            name: String::new(),
            target: oid,
            message: String::new(),
        }));
    }
    ui.separator();
    if ui
        .button(format!("{}  Cherry-pick", icon::CHERRIES))
        .clicked()
    {
        actions.push(Action::Op(Op::CherryPick(oid)));
    }
    if ui
        .button(format!("{}  Revert", icon::ARROW_COUNTER_CLOCKWISE))
        .clicked()
    {
        actions.push(Action::Op(Op::Revert(oid)));
    }
    ui.menu_button(format!("{}  Reset current branch here", icon::CLOCK_COUNTER_CLOCKWISE), |ui| {
        if ui.button("Soft — keep index and working tree").clicked() {
            actions.push(Action::Op(Op::Reset { oid, mode: ResetMode::Soft }));
        }
        if ui.button("Mixed — keep working tree").clicked() {
            actions.push(Action::Op(Op::Reset { oid, mode: ResetMode::Mixed }));
        }
        if ui.button(RichText::new("Hard — discard all changes…").color(p.danger)).clicked() {
            actions.push(Action::Confirm { title: "Hard reset".into(), body: format!("Move the current branch to {} and throw away every uncommitted change? This cannot be undone.", short_id(oid)), confirm: "Reset".into(), danger: true, op: Op::Reset { oid, mode: ResetMode::Hard } });
        }
    });
    ui.separator();
    if ui.button(format!("{}  Copy SHA", icon::COPY)).clicked() {
        actions.push(Action::Copy(oid.to_string()));
    }
    if ui
        .button(format!("{}  Copy message", icon::TEXT_ALIGN_LEFT))
        .clicked()
    {
        actions.push(Action::Copy(summary.to_owned()));
    }
}

fn lane_x(rect: Rect, lane: u16) -> f32 {
    rect.min.x + LANE_W / 2.0 + f32::from(lane) * LANE_W
}

fn anchor(rect: Rect, lane: u16, end: LineEnd) -> Pos2 {
    let y = match end {
        LineEnd::Top => rect.min.y,
        LineEnd::Mid => rect.center().y,
        LineEnd::Bottom => rect.max.y,
    };
    pos2(lane_x(rect, lane), y)
}

/// Paints one row of the graph: edges as straight lines or smooth S-curves, then the node.
fn paint_graph_row(ui: &Ui, rect: Rect, g: &GraphRow, is_head: bool, p: &Palette) {
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let w = 2.0;
    for s in &g.segments {
        if s.from_lane >= MAX_LANES && s.to_lane >= MAX_LANES {
            continue;
        }
        let color = p.lane(s.color);
        let a = anchor(rect, s.from_lane, s.from);
        let b = anchor(rect, s.to_lane, s.to);
        if s.from_lane == s.to_lane {
            painter.line_segment([a, b], Stroke::new(w, color));
        } else {
            let my = (a.y + b.y) / 2.0;
            let shape = egui::epaint::CubicBezierShape::from_points_stroke(
                [a, pos2(a.x, my), pos2(b.x, my), b],
                false,
                Color32::TRANSPARENT,
                egui::epaint::PathStroke::new(w, color),
            );
            painter.add(shape);
        }
    }
    if g.lane >= MAX_LANES {
        return;
    }
    let c = anchor(rect, g.lane, LineEnd::Mid);
    let color = p.lane(g.color);
    if is_head {
        for (r, a) in [(12.0, 18u8), (9.0, 40), (7.5, 70)] {
            painter.circle_filled(c, r, Palette::tint(color, a));
        }
    }
    let r = if is_head { 5.5 } else { 4.5 };
    painter.circle_filled(c, r + 2.0, p.bg);
    if g.is_merge {
        painter.circle_filled(c, r, p.bg);
        painter.circle_stroke(c, r - 0.5, Stroke::new(2.0, color));
    } else {
        painter.circle_filled(c, r, color);
        if is_head {
            painter.circle_filled(c, 2.0, p.bg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resizing_preserves_visible_commit_rows_above_saved_details() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, true);
        let dir = tempfile::tempdir().expect("temporary tab");
        let mut tab = RepoTab::new(dir.path().into(), &ctx, 100, Default::default());
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
        theme::apply(&ctx, theme::ThemeId::Midnight, true);
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
                                show(ui, &mut tab, &Settings::default(), &mut Vec::new())
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
        let mut tab = RepoTab::new(dir.path().into(), ctx, 100, Default::default());
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
            new_mode: Some(0o100644),
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
    fn growing_the_window_restores_readable_author_files_and_diff() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, true);
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
        theme::apply(&ctx, theme::ThemeId::Midnight, true);
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
        theme::apply(&ctx, theme::ThemeId::Midnight, true);
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
            theme::apply(&ctx, theme::ThemeId::Midnight, true);
            let (mut tab, _dir) = populated_tab(&ctx);
            tab.history_layout.expanded = true;
            visible_details(&ctx, &mut tab, size);
            tab.filter_history(&ctx);
            visible_details(&ctx, &mut tab, size);
            assert_eq!(
                ctx.memory(|memory| memory.focused()),
                Some(egui::Id::new("history-filter")),
                "the filter command must restore and focus the list at {size:?}"
            );
        }
    }

    #[test]
    fn history_search_stays_hidden_until_invoked_and_has_no_commit_count() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, true);
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
        theme::apply(&ctx, theme::ThemeId::Midnight, true);
        let (mut tab, _dir) = populated_tab(&ctx);
        tab.filter_history(&ctx);
        let open = visible_details(&ctx, &mut tab, vec2(1440.0, 900.0));
        assert!(
            open.iter().any(|text| text.contains("Filter by message")),
            "the shortcut must reveal the search field: {open:?}"
        );
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(egui::Id::new("history-filter"))
        );
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
            ctx.memory(|memory| memory.focused()),
            Some(egui::Id::new("history-filter")),
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
            theme::apply(&ctx, theme::ThemeId::Midnight, true);
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
            assert_eq!(
                ctx.memory(|memory| memory.focused()),
                Some(egui::Id::new("history-filter"))
            );
        }
    }

    #[test]
    fn opening_search_keeps_the_split_details_at_minimum_window_height() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, true);
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
    fn a_short_commit_diff_does_not_leave_a_large_empty_bottom() {
        for size in [vec2(1440.0, 900.0), vec2(2560.0, 1440.0)] {
            let ctx = egui::Context::default();
            crate::ui::fonts::install(&ctx);
            theme::apply(&ctx, theme::ThemeId::Midnight, true);
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
    fn detail_expander_shares_the_selected_filename_row() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, true);
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
        assert!(
            (expand.center().y - filename.center().y).abs() <= theme::XS,
            "expand {expand:?} belongs on the filename row {filename:?}"
        );
        assert!(expand.center().x > filename.center().x);
        assert!(
            expand.max.x >= size.x - theme::XL * 2.0,
            "expand {expand:?} must sit at the right edge of the diff pane"
        );
    }

    #[test]
    fn long_diff_paths_stop_before_the_expander() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, true);
        let (mut tab, _dir) = populated_tab(&ctx);
        let file = &mut tab.file.as_mut().expect("selected file").entry;
        file.path =
            "very-long-folder-name/another-very-long-folder-name/deeply-nested/file.rs".into();
        file.old_path = Some("old-folder-with-a-long-name/another/deeply-nested/file.rs".into());
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
        let path = output
            .shapes
            .iter()
            .find(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().contains("very-long-folder-name"))
            })
            .expect("rendered directory path");
        assert!(
            path.clip_rect.max.x <= expand.min.x - theme::XS,
            "the long path may paint over Expand: path clip {:?}, Expand {expand:?}",
            path.clip_rect
        );
    }
}
