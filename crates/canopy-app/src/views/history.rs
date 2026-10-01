//! The commit graph: a virtualised table with coloured lanes, ref pills and the details pane.

use egui::{
    Align2, Color32, CornerRadius, FontId, Key, Pos2, Rect, RichText, Sense, Stroke, Ui, pos2, vec2,
};
use egui_phosphor::regular as icon;

use canopy_core::ops::ResetMode;
use canopy_core::{GraphRow, LineEnd, Oid, short_id};

use crate::app::Action;
use crate::tab::RepoTab;
use crate::ui::theme::{self, Palette, palette};
use crate::ui::widgets;
use crate::views::{commit_detail, dialogs::Dialog};
use crate::worker::Op;

const ROW_H: f32 = 30.0;
const LANE_W: f32 = 16.0;
const MAX_LANES: u16 = 18;

/// Draws the history view.
pub fn show(ui: &mut Ui, tab: &mut RepoTab, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    if tab.snap.is_none() {
        loading(ui, tab);
        return;
    }
    let h = ui.available_height();
    egui::Panel::bottom("details")
        .resizable(true)
        .default_size((h * 0.46).max(220.0))
        .min_size(160.0)
        .max_size(h - 140.0)
        .frame(
            egui::Frame::new()
                .fill(p.panel)
                .stroke(Stroke::new(1.0, p.border)),
        )
        .show(ui, |ui| commit_detail::show(ui, tab, actions));
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(p.bg))
        .show(ui, |ui| table(ui, tab, actions));
}

fn loading(ui: &mut Ui, tab: &RepoTab) {
    let p = palette(ui.ctx());
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.35);
        if let Some(e) = &tab.load_error {
            ui.label(
                RichText::new(icon::WARNING_CIRCLE)
                    .size(36.0)
                    .color(p.danger),
            );
            ui.label(RichText::new("Could not read this repository").heading());
            ui.label(RichText::new(e).color(p.text_dim));
        } else {
            ui.spinner();
            ui.label(RichText::new("Reading repository…").color(p.text_dim));
        }
    });
}

fn table(ui: &mut Ui, tab: &mut RepoTab, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    // Filter bar.
    egui::Frame::new()
        .inner_margin(egui::Margin {
            left: 14,
            right: 14,
            top: 10,
            bottom: 8,
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
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
                                .hint_text("Filter by message, author or SHA   (Ctrl+F)")
                                .frame(egui::Frame::NONE)
                                .desired_width(340.0),
                        );
                        if !tab.filter.is_empty()
                            && widgets::icon_button(ui, icon::X, "Clear filter").clicked()
                        {
                            tab.filter.clear();
                            edit.request_focus();
                        }
                    });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(s) = &tab.snap {
                        let n = s.commits.len();
                        let more = if s.truncated { "+" } else { "" };
                        ui.label(
                            RichText::new(format!("{n}{more} commits"))
                                .color(p.text_faint)
                                .size(12.0),
                        );
                        if let Some(st) =
                            (s.state != canopy_core::RepoState::Clean).then_some(s.state)
                        {
                            state_banner(ui, st, actions);
                        }
                    }
                });
            });
        });

    let filtered: Option<Vec<usize>> = tab.filtered_rows().map(<[usize]>::to_vec);
    let Some(snap) = tab.snap.as_ref() else {
        return;
    };
    let rows: Vec<usize> = filtered
        .clone()
        .unwrap_or_else(|| (0..snap.commits.len()).collect());
    let show_graph = filtered.is_none();
    let lanes = snap
        .graph
        .iter()
        .map(|g| g.width)
        .max()
        .unwrap_or(1)
        .clamp(1, MAX_LANES);
    let graph_w = if show_graph {
        f32::from(lanes) * LANE_W + 16.0
    } else {
        0.0
    };

    if snap.commits.is_empty() {
        ui.vertical_centered(|ui| {
            ui.add_space(80.0);
            ui.label(RichText::new(icon::PLANT).size(40.0).color(p.accent));
            ui.label(RichText::new("No commits yet").heading());
            ui.label(
                RichText::new("Stage some files in Changes and make the first commit.")
                    .color(p.text_dim),
            );
        });
        return;
    }

    // Column layout.
    let full = ui.available_rect_before_wrap();
    let sha_w = 74.0;
    let date_w = 112.0;
    let author_w = (full.width() * 0.17).clamp(110.0, 200.0);
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
        (author_x + 6.0, "AUTHOR"),
        (date_x, "DATE"),
        (sha_x, "COMMIT"),
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

    // Keyboard navigation.
    let selected_pos = tab
        .selected_commit
        .and_then(|s| rows.iter().position(|&i| snap.commits[i].oid == s));
    let mut nav: Option<usize> = None;
    if !ui.ctx().egui_wants_keyboard_input() {
        let (down, up) = ui.input(|i| {
            (
                i.key_pressed(Key::ArrowDown) || i.key_pressed(Key::J),
                i.key_pressed(Key::ArrowUp) || i.key_pressed(Key::K),
            )
        });
        if down {
            nav = Some(selected_pos.map_or(0, |s| (s + 1).min(rows.len().saturating_sub(1))));
        } else if up {
            nav = Some(selected_pos.map_or(0, |s| s.saturating_sub(1)));
        }
    }
    let scroll_target = if tab.scroll_to_selected {
        selected_pos
    } else {
        nav
    };

    let now = chrono::Utc::now().timestamp();
    let mut clicked: Option<Oid> = nav.map(|n| snap.commits[rows[n]].oid);
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

    let total_rows = rows.len() + usize::from(snap.truncated && filtered.is_none());
    area.show_rows(ui, ROW_H, total_rows, |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for ri in range {
            if ri >= rows.len() {
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
            }
            let ci = rows[ri];
            let c = &snap.commits[ci];
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
                Rect::from_min_max(pos2(desc_x, rect.min.y), pos2(author_x - 12.0, rect.max.y));
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

            // Author.
            widgets::paint_avatar(
                ui,
                pos2(author_x + 16.0, cy),
                10.0,
                &c.author.name,
                &c.author.email,
            );
            let ag = ui.painter().layout_no_wrap(
                c.author.name.clone(),
                FontId::proportional(12.5),
                p.text_dim,
            );
            ui.painter()
                .with_clip_rect(Rect::from_min_max(
                    pos2(author_x + 30.0, rect.min.y),
                    pos2(date_x - 8.0, rect.max.y),
                ))
                .galley(
                    pos2(author_x + 32.0, cy - ag.size().y / 2.0),
                    ag,
                    p.text_dim,
                );
            ui.painter().text(
                pos2(date_x, cy),
                Align2::LEFT_CENTER,
                widgets::relative_time(c.author.time, now),
                FontId::proportional(12.5),
                p.text_faint,
            );
            ui.painter().text(
                pos2(sha_x, cy),
                Align2::LEFT_CENTER,
                short_id(c.oid),
                FontId::monospace(12.0),
                if selected { p.accent2 } else { p.text_faint },
            );

            let resp = resp.on_hover_text_at_pointer(format!(
                "{}\n{} <{}>\n{}",
                c.summary,
                c.author.name,
                c.author.email,
                widgets::full_date(c.author.time, c.author.offset_minutes)
            ));
            if resp.clicked() {
                clicked = Some(c.oid);
            }
            let oid = c.oid;
            let summary = c.summary.clone();
            resp.context_menu(|ui| commit_menu(ui, oid, &summary, actions));
        }
    });

    if let Some(oid) = clicked {
        tab.select_commit(oid);
    }
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
pub fn commit_menu(ui: &mut Ui, oid: Oid, summary: &str, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    ui.set_min_width(240.0);
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
        actions.push(Action::OpenDialog(Dialog::new_branch(Some(
            oid.to_string(),
        ))));
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
