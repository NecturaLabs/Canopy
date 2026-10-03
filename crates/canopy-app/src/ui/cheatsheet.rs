//! The keyboard cheatsheet (`?` or Ctrl+/): every binding from [`crate::keymap::BINDINGS`],
//! grouped by section and searchable, in an overlay styled like the command palette.

use egui::{Align2, CornerRadius, FontId, Id, Order, RichText, Sense, Stroke, vec2};
use egui_phosphor::regular as icon;

use super::theme::{self, Palette, palette};
use super::widgets;
use crate::keymap::{self, BINDINGS, Section};

/// Cheatsheet state.
#[derive(Debug, Default)]
pub struct Cheatsheet {
    pub open: bool,
    query: String,
    just_opened: bool,
}

impl Cheatsheet {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.query.clear();
        self.just_opened = self.open;
    }

    /// Draws the overlay when open.
    pub fn show(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        let p = palette(ctx);
        let screen = ctx.content_rect();
        egui::Area::new(Id::new("cheatsheet-scrim"))
            .order(Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let resp = ui.allocate_rect(screen, Sense::click());
                ui.painter()
                    .rect_filled(screen, CornerRadius::ZERO, p.backdrop());
                if resp.clicked() {
                    self.open = false;
                }
            });
        let width = 860.0_f32
            .min(screen.width() - theme::XL * 2.0 - theme::LG * 2.0)
            .max(0.0);
        let height = (screen.height() - theme::XL * 2.0 - theme::LG * 2.0).max(0.0);
        egui::Area::new(Id::new("cheatsheet"))
            .order(Order::Tooltip)
            .pivot(Align2::CENTER_TOP)
            .fixed_pos(egui::pos2(screen.center().x, screen.min.y + theme::XL))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(p.elevated)
                    .stroke(Stroke::new(1.0, Palette::tint(p.accent, 120)))
                    .corner_radius(CornerRadius::same(theme::RADIUS_LG))
                    .shadow(ctx.global_style().visuals.window_shadow)
                    .inner_margin(egui::Margin::same(16))
                    .show(ui, |ui| {
                        ui.set_width(width);
                        let content_top = ui.cursor().min.y;
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(icon::KEYBOARD).size(20.0).color(p.accent));
                            ui.label(
                                RichText::new("Keyboard shortcuts")
                                    .font(FontId::new(theme::TITLE_SIZE, theme::semibold())),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new("Esc to close")
                                            .size(theme::SMALL_SIZE)
                                            .color(p.text_faint),
                                    );
                                    let edit = ui.add(
                                        egui::TextEdit::singleline(&mut self.query)
                                            .hint_text(format!(
                                                "{}  Search shortcuts",
                                                icon::MAGNIFYING_GLASS
                                            ))
                                            .desired_width(240.0)
                                            .margin(theme::FIELD_MARGIN),
                                    );
                                    if self.just_opened {
                                        edit.request_focus();
                                        self.just_opened = false;
                                    }
                                },
                            );
                        });
                        ui.add_space(4.0);
                        ui.add(egui::Label::new(
                            RichText::new(
                                "Single keys work when no text field has focus. Super is left to your window manager.",
                            )
                            .size(theme::META_SIZE)
                            .color(p.text_dim),
                        ).wrap());
                        ui.add_space(10.0);
                        let q = self.query.trim().to_lowercase();
                        let remaining = (height - (ui.cursor().min.y - content_top)).max(0.0);
                        let list = egui::ScrollArea::vertical()
                            .max_height(remaining)
                            .min_scrolled_height(remaining)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                if width < 800.0 {
                                    for section in Section::ALL {
                                        rows(ui, section, &q);
                                    }
                                } else {
                                    ui.columns(2, |cols| {
                                        for (i, section) in Section::ALL.iter().enumerate() {
                                            rows(&mut cols[usize::from(i >= 1)], *section, &q);
                                        }
                                    });
                                }
                            });
                        widgets::scroll_fade(ui, &list, p.elevated);
                    });
            });
    }
}

fn rows(ui: &mut egui::Ui, section: Section, query: &str) {
    let p = palette(ui.ctx());
    let items: Vec<_> = BINDINGS
        .iter()
        .filter(|b| b.section == section && keymap::listed_here(b))
        .map(|b| (b, keymap::all_keys(b.command)))
        .filter(|(b, keys)| {
            query.is_empty()
                || b.description.to_lowercase().contains(query)
                || keys.iter().any(|k| k.to_lowercase().contains(query))
        })
        .collect();
    if items.is_empty() {
        return;
    }
    super::widgets::section_label(ui, section.title());
    ui.add_space(2.0);
    for (b, keys) in items {
        let key_width: f32 = keys
            .iter()
            .map(|key| {
                ui.painter()
                    .layout_no_wrap(
                        key.to_owned(),
                        FontId::monospace(theme::MONO_SMALL_SIZE),
                        p.text_dim,
                    )
                    .size()
                    .x
                    + 18.0
            })
            .sum();
        let description = ui.painter().layout_no_wrap(
            b.description.to_owned(),
            FontId::proportional(theme::BODY_SIZE),
            p.text,
        );
        let stacked = description.size().x + key_width + theme::LG > ui.available_width();
        let (rect, _) = ui.allocate_exact_size(
            vec2(ui.available_width(), if stacked { 46.0 } else { 24.0 }),
            Sense::hover(),
        );
        let description_y = if stacked {
            rect.min.y + 12.0
        } else {
            rect.center().y
        };
        ui.painter()
            .with_clip_rect(rect.intersect(ui.clip_rect()))
            .galley(
                egui::pos2(rect.min.x + 4.0, description_y - description.size().y / 2.0),
                description,
                p.text,
            );
        let key_y = if stacked {
            rect.min.y + 34.0
        } else {
            rect.center().y
        };
        // Key caps, right-aligned.
        let mut x = rect.right() - 4.0;
        for chord in keys.iter().rev() {
            let g = ui.painter().layout_no_wrap(
                chord.to_owned(),
                FontId::monospace(theme::MONO_SMALL_SIZE),
                p.text_dim,
            );
            let r = egui::Rect::from_min_max(
                egui::pos2(x - g.size().x - 12.0, key_y - 10.0),
                egui::pos2(x, key_y + 10.0),
            );
            ui.painter().rect(
                r,
                CornerRadius::same(5),
                p.bg,
                Stroke::new(1.0, p.border),
                egui::StrokeKind::Inside,
            );
            ui.painter()
                .galley(r.center() - g.size() / 2.0, g, p.text_dim);
            x = r.min.x - 6.0;
        }
    }
    ui.add_space(12.0);
}
