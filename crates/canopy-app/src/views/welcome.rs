//! The start screen: brand, quick actions and recent repositories.

use egui::{Align2, CornerRadius, FontId, RichText, Sense, Stroke, Ui, vec2};
use egui_phosphor::regular as icon;

use crate::app::Action;
use crate::keymap::{Command, hint};
use crate::settings::Settings;
use crate::ui::theme::{self, Palette, palette};
use crate::ui::{art, widgets};
use crate::views::dialogs::Dialog;

/// Draws the welcome screen.
pub fn show(ui: &mut Ui, settings: &Settings, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let rect = ui.max_rect();
    // Ambient glow.
    let still = settings.reduce_motion;
    let t = if still {
        0.0
    } else {
        ui.input(|i| i.time) as f32
    };
    let drift = vec2((t * 0.25).sin() * 30.0, (t * 0.2).cos() * 20.0);
    widgets::glow(
        ui.painter(),
        rect.center_top() + vec2(-260.0, 220.0) + drift,
        520.0,
        Palette::tint(p.accent, if p.dark { 46 } else { 30 }),
    );
    widgets::glow(
        ui.painter(),
        rect.center_top() + vec2(300.0, 360.0) - drift,
        480.0,
        Palette::tint(p.accent2, if p.dark { 30 } else { 22 }),
    );
    if !still {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space((rect.height() * 0.04).max(16.0));
                let art_w = (rect.width() * 0.5).clamp(320.0, 520.0);
                art::show(ui, art::welcome(), vec2(art_w, art_w * 9.0 / 16.0));
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Canopy")
                        .font(FontId::new(38.0, theme::semibold()))
                        .color(p.text),
                );
                ui.label(
                    RichText::new("A fast, beautiful, Linux-first Git client")
                        .size(15.0)
                        .color(p.text_dim),
                );
                ui.add_space(30.0);

                let card_w = 190.0;
                let total = card_w * 3.0 + 24.0;
                ui.allocate_ui_with_layout(
                    vec2(total, 110.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        if card(
                            ui,
                            icon::FOLDER_OPEN,
                            "Open",
                            "A repository on disk",
                            &hint(Command::Open),
                            card_w,
                        ) {
                            actions.push(Action::OpenDialog(Dialog::open()));
                        }
                        if card(
                            ui,
                            icon::CLOUD_ARROW_DOWN,
                            "Clone",
                            "GitHub, GitLab or any URL",
                            &hint(Command::Clone),
                            card_w,
                        ) {
                            actions.push(Action::OpenDialog(Dialog::clone_default(settings)));
                        }
                        if card(
                            ui,
                            icon::PLANT,
                            "Init",
                            "Start a new repository",
                            "",
                            card_w,
                        ) {
                            actions.push(Action::OpenDialog(Dialog::Init {
                                path: String::new(),
                            }));
                        }
                    },
                );
                ui.add_space(24.0);

                if !settings.seen_intro {
                    intro(ui, total, actions);
                    ui.add_space(20.0);
                }

                if !settings.recent.is_empty() {
                    ui.allocate_ui_with_layout(
                        vec2(total, 20.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| widgets::section_label(ui, "Recent repositories"),
                    );
                    ui.add_space(6.0);
                    for path in &settings.recent {
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let (r, resp) = ui.allocate_exact_size(vec2(total, 46.0), Sense::click());
                        let hover = ui
                            .ctx()
                            .animate_bool_with_time(resp.id, resp.hovered(), 0.12);
                        ui.painter().rect(
                            r,
                            CornerRadius::same(theme::RADIUS),
                            Palette::tint(p.elevated, 200 + (hover * 55.0) as u8),
                            Stroke::new(1.0, p.border.lerp_to_gamma(p.accent, hover * 0.7)),
                            egui::StrokeKind::Inside,
                        );
                        ui.painter().text(
                            r.left_center() + vec2(22.0, 0.0),
                            Align2::CENTER_CENTER,
                            icon::GIT_BRANCH,
                            FontId::proportional(17.0),
                            p.accent.lerp_to_gamma(p.accent2, hover),
                        );
                        ui.painter().text(
                            r.left_center() + vec2(42.0, -8.0),
                            Align2::LEFT_CENTER,
                            name,
                            FontId::new(13.5, theme::semibold()),
                            p.text,
                        );
                        ui.painter().text(
                            r.left_center() + vec2(42.0, 9.0),
                            Align2::LEFT_CENTER,
                            path.display().to_string(),
                            FontId::proportional(11.5),
                            p.text_faint,
                        );
                        let x = egui::Rect::from_center_size(
                            r.right_center() - vec2(20.0, 0.0),
                            vec2(22.0, 22.0),
                        );
                        let xr = ui.interact(x, resp.id.with("forget"), Sense::click());
                        if resp.hovered() || xr.hovered() {
                            ui.painter().text(
                                x.center(),
                                Align2::CENTER_CENTER,
                                icon::X,
                                FontId::proportional(12.0),
                                if xr.hovered() { p.danger } else { p.text_faint },
                            );
                        }
                        if xr.on_hover_text("Remove from list").clicked() {
                            actions.push(Action::ForgetRecent(path.clone()));
                        } else if resp.clicked() {
                            actions.push(Action::OpenRepo(path.clone()));
                        }
                        ui.add_space(6.0);
                    }
                }
                ui.add_space(24.0);
                ui.label(
                    RichText::new(format!(
                        "{} for the command palette   ·   ? for keyboard shortcuts",
                        hint(Command::Palette)
                    ))
                    .size(12.0)
                    .color(p.text_faint),
                );
            });
        });
}

/// First-run tips: how Canopy is meant to be driven.
fn intro(ui: &mut Ui, width: f32, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    ui.allocate_ui_with_layout(
        vec2(width, 10.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            egui::Frame::new()
                .fill(Palette::tint(p.accent, if p.dark { 26 } else { 18 }))
                .stroke(Stroke::new(1.0, Palette::tint(p.accent, 90)))
                .corner_radius(CornerRadius::same(theme::RADIUS_LG))
                .inner_margin(egui::Margin::same(14))
                .show(ui, |ui| {
                    ui.set_width(width - 28.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(icon::LIGHTBULB).size(18.0).color(p.accent));
                        ui.label(
                            RichText::new("Canopy is keyboard-first")
                                .font(FontId::new(14.0, theme::semibold())),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Got it").clicked() {
                                actions.push(Action::DismissIntro);
                            }
                        });
                    });
                    ui.add_space(4.0);
                    for line in [
                        format!("{}  opens the command palette: every action, branch and repository.", hint(Command::Palette)),
                        "?  shows every shortcut. 1 / 2 / 3 switch views, j / k move, f / p / P fetch, pull, push.".to_owned(),
                        "Settings (Ctrl+,) has themes, Git defaults and your GitHub / GitLab tokens for pull requests.".to_owned(),
                    ] {
                        ui.label(RichText::new(line).size(12.5).color(p.text_dim));
                    }
                });
        },
    );
}

fn card(ui: &mut Ui, glyph: &str, title: &str, sub: &str, keys: &str, w: f32) -> bool {
    let p = palette(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(w, 110.0), Sense::click());
    let hover = ui
        .ctx()
        .animate_bool_with_time(resp.id, resp.hovered(), 0.15);
    let lift = hover * 3.0;
    let r = r.translate(vec2(0.0, -lift));
    if hover > 0.0 {
        ui.painter().rect_filled(
            r.translate(vec2(0.0, 6.0)).shrink(4.0),
            CornerRadius::same(theme::RADIUS_LG),
            Palette::tint(p.accent, (hover * 40.0) as u8),
        );
    }
    ui.painter().rect(
        r,
        CornerRadius::same(theme::RADIUS_LG),
        p.elevated,
        Stroke::new(1.0, p.border.lerp_to_gamma(p.accent, hover)),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        r.left_top() + vec2(18.0, 28.0),
        Align2::LEFT_CENTER,
        glyph,
        FontId::proportional(24.0),
        p.accent.lerp_to_gamma(p.accent2, hover),
    );
    ui.painter().text(
        r.left_top() + vec2(18.0, 62.0),
        Align2::LEFT_CENTER,
        title,
        FontId::new(15.0, theme::semibold()),
        p.text,
    );
    ui.painter().text(
        r.left_top() + vec2(18.0, 84.0),
        Align2::LEFT_CENTER,
        sub,
        FontId::proportional(12.0),
        p.text_dim,
    );
    let name = title.to_owned();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
    if !keys.is_empty() {
        ui.painter().text(
            r.right_top() + vec2(-14.0, 28.0),
            Align2::RIGHT_CENTER,
            keys,
            FontId::monospace(10.5),
            p.text_faint,
        );
    }
    resp.clicked()
}
