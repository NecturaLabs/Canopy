//! The start screen: brand, quick actions and recent repositories.

use egui::{Align2, CornerRadius, FontId, RichText, Sense, Stroke, Ui, vec2};
use egui_phosphor::regular as icon;

use crate::action::Action;
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
    let still = crate::ui::motion::reduced();
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
            // Sit the whole stack a little above the middle, from its height last frame; a tall
            // window otherwise leaves everything in its upper half.
            let height_id = egui::Id::new("welcome-content-height");
            let last: f32 = ui.ctx().data(|d| d.get_temp(height_id)).unwrap_or(0.0);
            let top = if rect.height() < 400.0 {
                theme::XS
            } else {
                ((rect.height() - last) * 0.55).max((rect.height() * 0.04).max(theme::LG))
            };
            let start = ui.cursor().top();
            ui.vertical_centered(|ui| {
                ui.add_space(top);
                let art_w = (rect.width() * 0.5)
                    .min(rect.height() * if rect.height() < 400.0 { 0.30 } else { 0.50 })
                    .clamp(120.0, 520.0);
                art::show(ui, art::welcome(), vec2(art_w, art_w * 25.0 / 64.0));
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Canopy")
                        .font(FontId::new(theme::HERO_SIZE, theme::semibold()))
                        .color(p.text),
                );
                ui.label(
                    RichText::new("A fast, beautiful, Linux-first Git client")
                        .size(theme::TITLE_SIZE)
                        .color(p.text_dim),
                );
                ui.add_space(theme::LG);

                let total = (ui.available_width() - theme::XL * 2.0).min(594.0);
                let card_w = (total - theme::MD * 2.0) / 3.0;
                ui.allocate_ui_with_layout(
                    vec2(total, 122.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        if card(
                            ui,
                            icon::FOLDER_OPEN,
                            "Open",
                            "Choose a Git folder on this computer",
                            &hint(Command::Open),
                            card_w,
                        ) {
                            actions.push(Action::OpenDialog(Dialog::open()));
                        }
                        if card(
                            ui,
                            icon::CLOUD_ARROW_DOWN,
                            "Clone",
                            "Copy a repository from a remote URL",
                            &hint(Command::Clone),
                            card_w,
                        ) {
                            actions.push(Action::OpenDialog(Dialog::clone_default(settings)));
                        }
                        if card(
                            ui,
                            icon::PLANT,
                            "Init",
                            "Create a Git repository in a folder",
                            &hint(Command::Init),
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
                        resp.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name)
                        });
                        let forget = format!("Forget {name}");
                        let hover = ui
                            .ctx()
                            .animate_bool_with_time(resp.id, resp.hovered(), 0.12);
                        ui.painter().rect(
                            r,
                            CornerRadius::same(theme::RADIUS),
                            Palette::tint(p.elevated(), 200 + (hover * 55.0) as u8),
                            Stroke::new(1.0, p.border().lerp_to_gamma(p.accent, hover * 0.7)),
                            egui::StrokeKind::Inside,
                        );
                        ui.painter().text(
                            r.left_center() + vec2(22.0, 0.0),
                            Align2::CENTER_CENTER,
                            icon::GIT_BRANCH,
                            FontId::proportional(theme::ICON_MD),
                            p.accent.lerp_to_gamma(p.accent2, hover),
                        );
                        let text_width = (r.width() - theme::XL * 3.0).max(0.0);
                        let painter = ui.painter().with_clip_rect(r.intersect(ui.clip_rect()));
                        for (text, font, color, y) in [
                            (
                                name,
                                FontId::new(theme::BODY_SIZE, theme::semibold()),
                                p.text,
                                -8.0,
                            ),
                            (
                                path.display().to_string(),
                                FontId::proportional(theme::SMALL_SIZE),
                                p.text_faint,
                                9.0,
                            ),
                        ] {
                            let galley =
                                widgets::single_line_galley(ui, text, font, color, text_width);
                            painter.galley(
                                r.left_center() + vec2(42.0, y - galley.size().y / 2.0),
                                galley,
                                color,
                            );
                        }
                        let x = egui::Rect::from_center_size(
                            r.right_center() - vec2(20.0, 0.0),
                            vec2(22.0, 22.0),
                        );
                        let xr = ui.interact(x, resp.id.with("forget"), Sense::click());
                        xr.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &forget)
                        });
                        if resp.hovered() || xr.hovered() {
                            ui.painter().text(
                                x.center(),
                                Align2::CENTER_CENTER,
                                icon::X,
                                FontId::proportional(theme::ICON_XS),
                                if xr.hovered() { p.danger } else { p.text_faint },
                            );
                        }
                        if xr.on_hover_text("Remove from list").clicked() {
                            actions.push(Action::ForgetRecent(path.clone()));
                        } else if resp
                            .clone()
                            .on_hover_text(path.display().to_string())
                            .clicked()
                        {
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
                    .size(theme::META_SIZE)
                    .color(p.text_faint),
                );
            });
            let used = ui.cursor().top() - start - top;
            if (used - last).abs() > 0.5 {
                ui.ctx().data_mut(|d| d.insert_temp(height_id, used));
                ui.ctx().request_repaint();
            }
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
                        ui.label(RichText::new(icon::LIGHTBULB).size(theme::ICON_LG).color(p.accent));
                        ui.label(
                            RichText::new("Canopy is keyboard-first")
                                .font(FontId::new(theme::SUBTITLE_SIZE, theme::semibold())),
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
                        format!("Settings ({}) has themes, Git defaults and your GitHub / GitLab tokens for pull requests.", hint(Command::Settings)),
                    ] {
                        ui.label(RichText::new(line).size(theme::META_SIZE).color(p.text_dim));
                    }
                });
        },
    );
}

fn card(ui: &mut Ui, glyph: &str, title: &str, sub: &str, keys: &str, w: f32) -> bool {
    let p = palette(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(w, 122.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, title));
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
        p.elevated(),
        Stroke::new(1.0, p.border().lerp_to_gamma(p.accent, hover)),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        r.left_top() + vec2(18.0, 28.0),
        Align2::LEFT_CENTER,
        glyph,
        FontId::proportional(theme::ICON_XL),
        p.accent.lerp_to_gamma(p.accent2, hover),
    );
    ui.painter().text(
        r.left_top() + vec2(18.0, 62.0),
        Align2::LEFT_CENTER,
        title,
        FontId::new(theme::TITLE_SIZE, theme::semibold()),
        p.text,
    );
    let mut subtitle_job = egui::text::LayoutJob::simple(
        sub.to_owned(),
        FontId::proportional(theme::SMALL_SIZE),
        p.text_dim,
        (r.width() - theme::LG * 2.0).max(0.0),
    );
    subtitle_job.wrap.max_rows = 2;
    let subtitle = ui.painter().layout_job(subtitle_job);
    ui.painter()
        .galley(r.left_top() + vec2(theme::LG, 82.0), subtitle, p.text_dim);
    let name = title.to_owned();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
    if !keys.is_empty() {
        ui.painter().text(
            r.right_top() + vec2(-14.0, 28.0),
            Align2::RIGHT_CENTER,
            keys,
            FontId::monospace(theme::MONO_SMALL_SIZE),
            p.text_faint,
        );
    }
    resp.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_action_text_fits_a_minimum_zoomed_window() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(
            &ctx,
            theme::ThemeId::Midnight,
            &crate::ui::theme::Appearance::STILL,
        );
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            card(
                ui,
                icon::CLOUD_ARROW_DOWN,
                "Clone",
                "GitHub, GitLab or any URL",
                "Ctrl+Shift+O",
                160.0,
            );
        });
        output.textures_delta.clear();
        let card_rect = output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::Shape::Rect(rect) = &shape.shape {
                    Some(rect.rect)
                } else {
                    None
                }
            })
            .expect("card surface");
        for shape in &output.shapes {
            if let egui::Shape::Text(text) = &shape.shape {
                assert!(
                    card_rect.contains_rect(text.visual_bounding_rect()),
                    "quick action text overflows: {:?}",
                    text.visual_bounding_rect()
                );
            }
        }
    }
}
