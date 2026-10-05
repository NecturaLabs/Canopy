//! The start screen: brand, quick actions and recent repositories.

use egui::{Color32, FontId, Id, Pos2, RichText, Ui, vec2};
use egui_phosphor::regular as icon;

use crate::action::Action;
use crate::keymap::{Command, hint};
use crate::settings::Settings;
use crate::ui::kit::chip::KeyChord;
use crate::ui::kit::row::{ListRow, SectionHeader};
use crate::ui::kit::surface::{Callout, Card};
use crate::ui::kit::{Button, IconButton, Size, Tone, anim};
use crate::ui::theme::{self, Palette, palette};
use crate::ui::{art, motion};
use crate::views::dialogs::Dialog;

/// Widest the action cards and the recent list grow.
const COLUMN_WIDTH: f32 = 594.0;
/// Height of an action card.
const CARD_HEIGHT: f32 = 122.0;

/// Draws the welcome screen.
pub fn show(ui: &mut Ui, settings: &Settings, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let rect = ui.max_rect();
    ambient(ui, rect);

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
                ui.add_space(theme::XS);
                ui.label(
                    RichText::new("Canopy")
                        .font(FontId::new(m.hero_size, theme::semibold()))
                        .color(p.text),
                );
                ui.label(
                    RichText::new("A fast, beautiful, Linux-first Git client")
                        .size(m.title_size)
                        .color(p.text_dim),
                );
                ui.add_space(theme::LG);

                let total = (ui.available_width() - theme::XL * 2.0).min(COLUMN_WIDTH);
                let card_w = (total - theme::MD * 2.0) / 3.0;
                ui.allocate_ui_with_layout(
                    vec2(total, CARD_HEIGHT),
                    egui::Layout::left_to_right(egui::Align::Min),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = theme::MD;
                        if card(
                            ui,
                            icon::FOLDER_OPEN,
                            "Open",
                            "Choose a Git folder on this computer",
                            Command::Open,
                            card_w,
                        )
                        .clicked()
                        {
                            actions.push(Action::OpenDialog(Dialog::open()));
                        }
                        if card(
                            ui,
                            icon::CLOUD_ARROW_DOWN,
                            "Clone",
                            "Copy a repository from a remote URL",
                            Command::Clone,
                            card_w,
                        )
                        .clicked()
                        {
                            actions.push(Action::OpenDialog(Dialog::clone_default(settings)));
                        }
                        if card(
                            ui,
                            icon::PLANT,
                            "Init",
                            "Create a Git repository in a folder",
                            Command::Init,
                            card_w,
                        )
                        .clicked()
                        {
                            actions.push(Action::OpenDialog(Dialog::Init {
                                path: String::new(),
                            }));
                        }
                    },
                );
                ui.add_space(theme::XL);

                if !settings.seen_intro {
                    intro(ui, total, actions);
                    ui.add_space(theme::LG);
                }

                if !settings.recent.is_empty() {
                    ui.allocate_ui_with_layout(
                        vec2(total, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            SectionHeader::new("Recent repositories")
                                .first(true)
                                .show(ui);
                            for path in &settings.recent {
                                recent(ui, path, actions);
                            }
                        },
                    );
                }
                ui.add_space(theme::XL);
                ui.label(
                    RichText::new(format!(
                        "{} for the command palette   ·   ? for keyboard shortcuts",
                        hint(Command::Palette)
                    ))
                    .size(m.meta_size)
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

/// The two soft glows behind the welcome screen. They fade and settle in once each time the
/// screen appears, then stay still (none of it with reduced motion), so the screen does not
/// repaint while it only sits there.
fn ambient(ui: &Ui, rect: egui::Rect) {
    let p = palette(ui.ctx());
    let ctx = ui.ctx();
    let id = Id::new("welcome-ambient");
    let frame = ctx.cumulative_frame_nr();
    let last: Option<u64> = ctx.data(|d| d.get_temp(id.with("frame")));
    ctx.data_mut(|d| d.insert_temp(id.with("frame"), frame));
    // Away for a frame or more: the screen has just appeared again.
    let fresh = last.is_none_or(|l| l + 1 < frame);
    let t = if motion::reduced() {
        1.0
    } else if fresh {
        anim::ease_bool(ctx, id, false, 0.0, 0.0)
    } else {
        anim::ease_bool(ctx, id, true, motion::SLOW, 0.0)
    };
    if t <= 0.0 {
        return;
    }
    // Each glow drifts in from a little way off its resting place.
    let settle = vec2(30.0, 20.0) * (1.0 - t);
    let accent = p.accent_tint();
    let accent2 = Palette::tint(p.accent2, accent.a());
    glow(
        ui.painter(),
        rect.center_top() + vec2(-260.0, 220.0) - settle,
        520.0,
        accent.gamma_multiply(t),
    );
    glow(
        ui.painter(),
        rect.center_top() + vec2(300.0, 360.0) + settle,
        480.0,
        accent2.gamma_multiply(t),
    );
}

/// A soft radial glow centred on `center`: `color` at the centre, transparent at `radius`.
fn glow(painter: &egui::Painter, center: Pos2, radius: f32, color: Color32) {
    const SEGMENTS: u32 = 64;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(center, color);
    for i in 0..=SEGMENTS {
        let a = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        mesh.colored_vertex(
            center + vec2(a.cos(), a.sin()) * radius,
            Color32::TRANSPARENT,
        );
    }
    for i in 1..=SEGMENTS {
        mesh.add_triangle(0, i, i + 1);
    }
    painter.add(mesh);
}

/// A recent repository: opens on click, with a button to forget it.
fn recent(ui: &mut Ui, path: &std::path::Path, actions: &mut Vec<Action>) {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let shown = path.display().to_string();
    let mut forget = false;
    let row = ListRow::new(("recent", path))
        .icon(icon::GIT_BRANCH)
        .label(name.clone())
        .detail(shown.clone())
        .tooltip(shown)
        .actions(|ui| {
            forget = IconButton::new(icon::X, format!("Forget {name}"))
                .size(Size::Sm)
                .id_salt(("forget", path))
                .show(ui)
                .clicked();
        })
        .show(ui);
    if forget {
        actions.push(Action::ForgetRecent(path.to_path_buf()));
    } else if row.clicked() {
        actions.push(Action::OpenRepo(path.to_path_buf()));
    }
}

/// First-run tips: how Canopy is meant to be driven.
fn intro(ui: &mut Ui, width: f32, actions: &mut Vec<Action>) {
    let text = [
        format!(
            "{}  opens the command palette: every action, branch and repository.",
            hint(Command::Palette)
        ),
        "?  shows every shortcut. 1 / 2 / 3 switch views, j / k move, f / p / P fetch, pull, push."
            .to_owned(),
        format!(
            "Settings ({}) has themes, Git defaults and your GitHub / GitLab tokens for pull \
             requests.",
            hint(Command::Settings)
        ),
    ]
    .join("\n");
    ui.allocate_ui_with_layout(
        vec2(width, 0.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_width(width);
            let callout = Callout::new(Tone::Accent, text)
                .title("Canopy is keyboard-first")
                .icon(icon::LIGHTBULB)
                .show(ui);
            // The dismiss button sits at the right of the title line.
            let size = Size::Sm.height();
            let pad = theme::SM;
            let at = egui::Rect::from_min_size(
                egui::pos2(callout.rect.right() - pad, callout.rect.top() + pad) - vec2(0.0, 0.0),
                vec2(0.0, size),
            );
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(egui::Rect::from_min_max(
                        egui::pos2(callout.rect.left(), at.top()),
                        egui::pos2(at.right(), at.bottom()),
                    ))
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
            );
            if Button::new("Got it")
                .ghost()
                .size(Size::Sm)
                .id_salt("intro-got-it")
                .show(&mut child)
                .clicked()
            {
                actions.push(Action::DismissIntro);
            }
        },
    );
}

/// A quick action: icon, title, one line of help and the command's keys. Returns its response.
fn card(
    ui: &mut Ui,
    glyph: &str,
    title: &str,
    sub: &str,
    command: Command,
    w: f32,
) -> egui::Response {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    Card::new()
        .interactive(true)
        .width(w)
        .label(title)
        .id_salt(("welcome-card", title))
        .show(ui, |ui| {
            ui.set_min_height(CARD_HEIGHT - 2.0 * m.card_pad);
            ui.spacing_mut().item_spacing.y = theme::XS;
            ui.horizontal(|ui| {
                ui.label(RichText::new(glyph).size(m.icon_xl).color(p.accent));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    KeyChord::command(command).show(ui);
                });
            });
            ui.add_space(theme::SM);
            ui.label(
                RichText::new(title)
                    .font(FontId::new(m.subtitle_size, theme::semibold()))
                    .color(p.text),
            );
            let mut job = egui::text::LayoutJob::simple(
                sub.to_owned(),
                FontId::proportional(m.small_size),
                p.text_dim,
                ui.available_width(),
            );
            job.wrap.max_rows = 2;
            ui.label(job);
        })
        .response
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
        let mut card_rect = egui::Rect::NOTHING;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            card_rect = card(
                ui,
                icon::CLOUD_ARROW_DOWN,
                "Clone",
                "GitHub, GitLab or any URL",
                Command::Clone,
                160.0,
            )
            .rect;
        });
        output.textures_delta.clear();
        assert!(card_rect.width() <= 160.5, "card grew to {card_rect:?}");
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

    #[test]
    fn the_ambient_glow_stops_repainting_once_it_has_settled() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(
            &ctx,
            theme::ThemeId::Midnight,
            &crate::ui::theme::Appearance::default(),
        );
        let mut input = egui::RawInput::default();
        let mut repaint = std::time::Duration::ZERO;
        for frame in 0..40 {
            input.time = Some(f64::from(frame) * 0.05);
            let mut output = ctx.run_ui(input.clone(), |ui| {
                ambient(ui, ui.max_rect());
            });
            output.textures_delta.clear();
            repaint = output
                .viewport_output
                .values()
                .map(|v| v.repaint_delay)
                .min()
                .unwrap_or(std::time::Duration::MAX);
        }
        assert!(
            repaint > std::time::Duration::from_secs(1),
            "still repainting every {repaint:?}"
        );
    }
}
