//! The tab strip and the action bar.

use egui::{Align, Align2, CornerRadius, FontId, Layout, RichText, Sense, Stroke, Ui, vec2};
use egui_phosphor::regular as icon;

use crate::app::Action;
use crate::tab::{RepoTab, View};
use crate::ui::theme::{self, Palette, palette};
use crate::ui::widgets;
use crate::worker::Op;

/// Repository tabs, browser style.
pub fn tabs(ui: &mut Ui, tabs: &[RepoTab], active: usize, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    ui.horizontal_centered(|ui| {
        ui.add_space(10.0);
        let (logo, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
        widgets::paint_logo(ui.painter(), logo.center(), 22.0, p);
        ui.add_space(4.0);
        ui.label(
            RichText::new("Canopy")
                .font(FontId::new(14.0, theme::semibold()))
                .color(p.text),
        );
        ui.add_space(14.0);
        for (i, tab) in tabs.iter().enumerate() {
            let selected = i == active;
            let text = tab.name.clone();
            let galley =
                ui.painter()
                    .layout_no_wrap(text, FontId::new(13.0, theme::medium()), p.text);
            let w = galley.size().x + 58.0;
            let (rect, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
            let hover = ui
                .ctx()
                .animate_bool_with_time(resp.id, resp.hovered(), 0.12);
            let fill = if selected {
                p.panel
            } else {
                Palette::tint(p.text, (hover * 12.0) as u8)
            };
            ui.painter().rect_filled(
                rect,
                CornerRadius {
                    nw: 8,
                    ne: 8,
                    sw: 0,
                    se: 0,
                },
                fill,
            );
            if selected {
                let bar = egui::Rect::from_min_size(rect.min, vec2(rect.width(), 2.0));
                widgets::gradient_rect(ui.painter(), bar, p.accent, p.accent2, 1);
            }
            let busy = tab.running.is_some();
            let glyph = if busy {
                icon::CIRCLE_NOTCH
            } else {
                icon::FOLDER_SIMPLE
            };
            ui.painter().text(
                rect.left_center() + vec2(14.0, 0.0),
                Align2::CENTER_CENTER,
                glyph,
                FontId::proportional(14.0),
                if selected { p.accent } else { p.text_faint },
            );
            ui.painter().galley(
                rect.left_center() + vec2(26.0, -galley.size().y / 2.0),
                galley,
                if selected { p.text } else { p.text_dim },
            );
            let close = egui::Rect::from_center_size(
                rect.right_center() - vec2(15.0, 0.0),
                vec2(18.0, 18.0),
            );
            let close_resp = ui.interact(close, resp.id.with("close"), Sense::click());
            if selected || resp.hovered() || close_resp.hovered() {
                if close_resp.hovered() {
                    ui.painter()
                        .rect_filled(close, CornerRadius::same(4), p.hover);
                }
                ui.painter().text(
                    close.center(),
                    Align2::CENTER_CENTER,
                    icon::X,
                    FontId::proportional(11.0),
                    p.text_dim,
                );
            }
            if close_resp.clicked() {
                actions.push(Action::CloseTab(i));
            } else if resp.clicked() {
                actions.push(Action::SwitchTab(i));
            } else if resp.middle_clicked() {
                actions.push(Action::CloseTab(i));
            }
            resp.on_hover_text(tab.path.display().to_string());
        }
        if widgets::icon_button(ui, icon::PLUS, "Open repository (Ctrl+O)").clicked() {
            actions.push(Action::OpenDialog(crate::views::dialogs::Dialog::open()));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(8.0);
            let theme_glyph = if p.dark { icon::SUN } else { icon::MOON };
            if widgets::icon_button(ui, theme_glyph, "Toggle light / dark theme").clicked() {
                actions.push(Action::ToggleTheme);
            }
            if widgets::icon_button(ui, icon::GEAR_SIX, "Settings (Ctrl+,)").clicked() {
                actions.push(Action::OpenDialog(crate::views::dialogs::Dialog::Settings));
            }
            // Search pill that opens the command palette.
            let (rect, resp) = ui.allocate_exact_size(vec2(260.0, 26.0), Sense::click());
            let hover = ui
                .ctx()
                .animate_bool_with_time(resp.id, resp.hovered(), 0.12);
            ui.painter().rect(
                rect,
                CornerRadius::same(13),
                p.bg,
                Stroke::new(1.0, p.border.lerp_to_gamma(p.accent, hover * 0.6)),
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                rect.left_center() + vec2(14.0, 0.0),
                Align2::CENTER_CENTER,
                icon::MAGNIFYING_GLASS,
                FontId::proportional(13.0),
                p.text_faint,
            );
            ui.painter().text(
                rect.left_center() + vec2(28.0, 0.0),
                Align2::LEFT_CENTER,
                "Search commands",
                FontId::proportional(12.5),
                p.text_faint,
            );
            ui.painter().text(
                rect.right_center() - vec2(12.0, 0.0),
                Align2::RIGHT_CENTER,
                "Ctrl K",
                FontId::monospace(11.0),
                p.text_faint,
            );
            if resp.clicked() {
                actions.push(Action::Palette);
            }
        });
    });
}

/// The action bar of the active repository.
pub fn actions_bar(ui: &mut Ui, tab: &RepoTab, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let snap = tab.snap.as_ref();
    let has_remote = snap.is_some_and(|s| !s.remotes.is_empty());
    let busy = tab.running.is_some();
    let branch = snap.and_then(|s| s.current_branch());
    ui.horizontal_centered(|ui| {
        ui.add_space(8.0);
        // View switcher (segmented control).
        let changes = snap.map(|s| s.status.len()).unwrap_or(0);
        segmented(ui, tab.view, changes, actions);
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(4.0);
        if widgets::tool_button(
            ui,
            icon::ARROWS_CLOCKWISE,
            "Fetch",
            "Fetch all remotes (Ctrl+Shift+F)",
            has_remote && !busy,
        )
        .clicked()
        {
            actions.push(Action::Op(Op::Fetch));
        }
        let behind = branch.map(|b| b.behind).unwrap_or(0);
        let pull_label = if behind > 0 {
            format!("Pull ↓{behind}")
        } else {
            "Pull".to_owned()
        };
        if widgets::tool_button(
            ui,
            icon::ARROW_FAT_LINE_DOWN,
            &pull_label,
            "Pull the current branch (Ctrl+Shift+L)",
            has_remote && !busy,
        )
        .clicked()
        {
            actions.push(Action::Pull);
        }
        let ahead = branch.map(|b| b.ahead).unwrap_or(0);
        let push_label = if ahead > 0 {
            format!("Push ↑{ahead}")
        } else {
            "Push".to_owned()
        };
        if widgets::tool_button(
            ui,
            icon::ARROW_FAT_LINE_UP,
            &push_label,
            "Push the current branch (Ctrl+Shift+K)",
            has_remote && !busy && branch.is_some(),
        )
        .clicked()
        {
            actions.push(Action::Op(Op::Push { force: false }));
        }
        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);
        if widgets::tool_button(
            ui,
            icon::GIT_BRANCH,
            "Branch",
            "New branch (Ctrl+B)",
            snap.is_some(),
        )
        .clicked()
        {
            actions.push(Action::OpenDialog(
                crate::views::dialogs::Dialog::new_branch(None),
            ));
        }
        if widgets::tool_button(
            ui,
            icon::GIT_MERGE,
            "Merge",
            "Merge a branch into the current one",
            branch.is_some() && !busy,
        )
        .clicked()
        {
            actions.push(Action::OpenDialog(crate::views::dialogs::Dialog::Merge {
                target: String::new(),
                rebase: false,
            }));
        }
        let dirty = snap.is_some_and(|s| !s.status.is_clean());
        if widgets::tool_button(
            ui,
            icon::ARCHIVE,
            "Stash",
            "Stash changes (Ctrl+Shift+S)",
            dirty && !busy,
        )
        .clicked()
        {
            actions.push(Action::OpenDialog(crate::views::dialogs::Dialog::Stash {
                message: String::new(),
                untracked: true,
            }));
        }
        let stashes = snap.map(|s| s.stashes.len()).unwrap_or(0);
        if widgets::tool_button(
            ui,
            icon::TRAY_ARROW_UP,
            "Pop",
            "Pop the latest stash",
            stashes > 0 && !busy,
        )
        .clicked()
        {
            actions.push(Action::Op(Op::StashApply {
                index: 0,
                pop: true,
            }));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(8.0);
            if widgets::tool_button(
                ui,
                icon::TERMINAL_WINDOW,
                "Terminal",
                "Open a terminal here",
                true,
            )
            .clicked()
            {
                actions.push(Action::OpenTerminal(tab.path.clone()));
            }
            if widgets::tool_button(ui, icon::FOLDER_OPEN, "Files", "Show in file manager", true)
                .clicked()
            {
                actions.push(Action::OpenPath(tab.path.clone()));
            }
            let _ = p;
        });
    });
}

fn segmented(ui: &mut Ui, view: View, changes: usize, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let items = [
        (View::History, icon::GRAPH, "History", 0),
        (View::Changes, icon::PENCIL_SIMPLE_LINE, "Changes", changes),
    ];
    let (outer, _) = ui.allocate_exact_size(vec2(236.0, 34.0), Sense::hover());
    ui.painter().rect(
        outer,
        CornerRadius::same(10),
        p.bg,
        Stroke::new(1.0, p.border),
        egui::StrokeKind::Inside,
    );
    let w = (outer.width() - 6.0) / 2.0;
    let t = ui.ctx().animate_value_with_time(
        egui::Id::new("seg-view"),
        if view == View::History { 0.0 } else { 1.0 },
        0.2,
    );
    let thumb = egui::Rect::from_min_size(
        outer.min + vec2(3.0 + t * w, 3.0),
        vec2(w, outer.height() - 6.0),
    );
    ui.painter()
        .rect_filled(thumb, CornerRadius::same(8), p.elevated);
    ui.painter().rect_stroke(
        thumb,
        CornerRadius::same(8),
        Stroke::new(1.0, Palette::tint(p.accent, 120)),
        egui::StrokeKind::Inside,
    );
    for (i, (v, glyph, label, count)) in items.into_iter().enumerate() {
        let r = egui::Rect::from_min_size(
            outer.min + vec2(3.0 + i as f32 * w, 3.0),
            vec2(w, outer.height() - 6.0),
        );
        let resp = ui.interact(r, egui::Id::new(("seg", i)), Sense::click());
        let active = v == view;
        let color = if active {
            p.text
        } else if resp.hovered() {
            p.text_dim.lerp_to_gamma(p.text, 0.5)
        } else {
            p.text_dim
        };
        let text = format!("{glyph}  {label}");
        let galley = ui
            .painter()
            .layout_no_wrap(text, FontId::new(13.0, theme::medium()), color);
        let badge_w = if count > 0 { 26.0 } else { 0.0 };
        let start = r.center().x - (galley.size().x + badge_w) / 2.0;
        ui.painter().galley(
            egui::pos2(start, r.center().y - galley.size().y / 2.0),
            galley.clone(),
            color,
        );
        if count > 0 {
            let br = egui::Rect::from_center_size(
                egui::pos2(start + galley.size().x + 15.0, r.center().y),
                vec2(22.0, 16.0),
            );
            ui.painter()
                .rect_filled(br, CornerRadius::same(8), p.accent);
            ui.painter().text(
                br.center(),
                Align2::CENTER_CENTER,
                if count > 99 {
                    "99+".to_owned()
                } else {
                    count.to_string()
                },
                FontId::new(10.5, theme::semibold()),
                egui::Color32::WHITE,
            );
        }
        if resp.clicked() {
            actions.push(Action::SetView(v));
        }
    }
}
