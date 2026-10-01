//! The keyboard cheatsheet (`?` or Ctrl+/): every binding from [`crate::keymap::BINDINGS`],
//! grouped by section and searchable, in an overlay styled like the command palette.

use egui::{Align2, CornerRadius, FontId, Id, Order, RichText, Sense, Stroke, vec2};
use egui_phosphor::regular as icon;

use super::theme::{self, Palette, palette};
use crate::keymap::{BINDINGS, Section};

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
        let width = 860.0_f32.min(screen.width() - 40.0);
        let height = (screen.height() - 120.0).max(240.0);
        egui::Area::new(Id::new("cheatsheet"))
            .order(Order::Tooltip)
            .pivot(Align2::CENTER_TOP)
            .fixed_pos(egui::pos2(screen.center().x, screen.min.y + 60.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(p.elevated)
                    .stroke(Stroke::new(1.0, Palette::tint(p.accent, 120)))
                    .corner_radius(CornerRadius::same(theme::RADIUS_LG))
                    .shadow(ctx.global_style().visuals.window_shadow)
                    .inner_margin(egui::Margin::same(16))
                    .show(ui, |ui| {
                        ui.set_width(width);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(icon::KEYBOARD).size(20.0).color(p.accent));
                            ui.label(
                                RichText::new("Keyboard shortcuts")
                                    .font(FontId::new(17.0, theme::semibold())),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new("Esc to close")
                                            .size(11.5)
                                            .color(p.text_faint),
                                    );
                                    let edit = ui.add(
                                        egui::TextEdit::singleline(&mut self.query)
                                            .hint_text(format!(
                                                "{}  Search shortcuts",
                                                icon::MAGNIFYING_GLASS
                                            ))
                                            .desired_width(240.0),
                                    );
                                    if self.just_opened {
                                        edit.request_focus();
                                        self.just_opened = false;
                                    }
                                },
                            );
                        });
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(
                                "Single keys work when no text field has focus. Super is left to your window manager.",
                            )
                            .size(12.0)
                            .color(p.text_dim),
                        );
                        ui.add_space(10.0);
                        let q = self.query.trim().to_lowercase();
                        egui::ScrollArea::vertical()
                            .max_height(height - 90.0)
                            .min_scrolled_height(height - 90.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.columns(2, |cols| {
                                    for (i, section) in Section::ALL.iter().enumerate() {
                                        let col = &mut cols[usize::from(i >= 1)];
                                        rows(col, *section, &q);
                                    }
                                });
                            });
                    });
            });
    }
}

fn rows(ui: &mut egui::Ui, section: Section, query: &str) {
    let p = palette(ui.ctx());
    let items: Vec<_> = BINDINGS
        .iter()
        .filter(|b| b.section == section)
        .filter(|b| {
            query.is_empty()
                || b.description.to_lowercase().contains(query)
                || b.keys().to_lowercase().contains(query)
        })
        .collect();
    if items.is_empty() {
        return;
    }
    super::widgets::section_label(ui, section.title());
    ui.add_space(2.0);
    for b in items {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
        ui.painter().text(
            rect.left_center() + vec2(4.0, 0.0),
            Align2::LEFT_CENTER,
            b.description,
            FontId::proportional(13.0),
            p.text,
        );
        // Key caps, right-aligned.
        let mut x = rect.right() - 4.0;
        for chord in b
            .keys()
            .split("  ·  ")
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            let g =
                ui.painter()
                    .layout_no_wrap(chord.to_owned(), FontId::monospace(11.5), p.text_dim);
            let r = egui::Rect::from_min_max(
                egui::pos2(x - g.size().x - 12.0, rect.center().y - 10.0),
                egui::pos2(x, rect.center().y + 10.0),
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
