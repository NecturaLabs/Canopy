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
                            ui.label(RichText::new(icon::KEYBOARD).size(theme::ICON_TITLE).color(p.accent));
                            ui.label(
                                RichText::new("Keyboard shortcuts")
                                    .font(FontId::new(theme::TITLE_SIZE, theme::semibold())),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if widgets::icon_button(ui, icon::X, "Close shortcuts (Esc)")
                                        .clicked()
                                    {
                                        self.open = false;
                                    }
                                    ui.label(
                                        RichText::new("Esc to close")
                                            .size(theme::SMALL_SIZE)
                                            .color(p.text_faint),
                                    );
                                    let edit = ui.add(
                                        egui::TextEdit::singleline(&mut self.query)
                                            .hint_text(format!(
                                                "{}  Search shortcuts…",
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
                                "Single keys work when no text field has focus. Capital letters mean Shift plus that key. Super is left to your window manager.",
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
                                        let heights: Vec<f32> = Section::ALL.iter().map(|section| {
                                            section_height(&cols[0], *section, &q)
                                        }).collect();
                                        let total: f32 = heights.iter().sum();
                                        let split = (1..Section::ALL.len()).min_by(|a, b| {
                                            let imbalance = |n: usize| (2.0 * heights[..n].iter().sum::<f32>() - total).abs();
                                            imbalance(*a).total_cmp(&imbalance(*b))
                                        }).unwrap_or(1);
                                        for (i, section) in Section::ALL.iter().enumerate() {
                                            rows(&mut cols[usize::from(i >= split)], *section, &q);
                                        }
                                    });
                                }
                            });
                        widgets::scroll_fade(ui, &list, p.elevated);
                    });
            });
    }
}

fn matching(section: Section, query: &str) -> Vec<(&'static keymap::Binding, Vec<String>)> {
    BINDINGS
        .iter()
        .filter(|b| b.section == section && keymap::listed_here(b))
        .map(|b| (b, keymap::all_keys(b.command)))
        .filter(|(b, keys)| {
            query.is_empty()
                || b.description.to_lowercase().contains(query)
                || keys.iter().any(|k| k.to_lowercase().contains(query))
        })
        .collect()
}

fn section_height(ui: &egui::Ui, section: Section, query: &str) -> f32 {
    let items = matching(section, query);
    if items.is_empty() {
        return 0.0;
    }
    let mut height = theme::SMALL_SIZE + theme::LG;
    for (binding, keys) in items {
        let description = ui.painter().layout_no_wrap(
            binding.description.to_owned(),
            FontId::proportional(theme::BODY_SIZE),
            palette(ui.ctx()).text,
        );
        let stacked =
            description.size().x + keycaps_width(ui, &keys) + theme::LG > ui.available_width();
        height += if stacked { 46.0 } else { 24.0 } + ui.spacing().item_spacing.y;
    }
    height
}

pub(crate) fn rows(ui: &mut egui::Ui, section: Section, query: &str) {
    let p = palette(ui.ctx());
    let items = matching(section, query);
    if items.is_empty() {
        return;
    }
    super::widgets::section_label(ui, section.title());
    ui.add_space(2.0);
    for (b, keys) in items {
        let key_width = keycaps_width(ui, &keys);
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
            x = paint_keycap(ui, x - keycap_width(ui, chord), key_y, chord) - KEYCAP_GAP;
        }
    }
    ui.add_space(12.0);
}

/// Space between two key caps.
pub(crate) const KEYCAP_GAP: f32 = 6.0;

/// The width of one key cap.
pub(crate) fn keycap_width(ui: &egui::Ui, chord: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(
            chord.to_owned(),
            FontId::monospace(theme::MONO_SMALL_SIZE),
            palette(ui.ctx()).text_dim,
        )
        .size()
        .x
        + 12.0
}

/// The width of a row of key caps, with the gaps between them.
pub(crate) fn keycaps_width(ui: &egui::Ui, chords: &[String]) -> f32 {
    chords
        .iter()
        .map(|c| keycap_width(ui, c) + KEYCAP_GAP)
        .sum()
}

/// Paints one key cap with its left edge at `left`, centred on `center_y`, and returns its right
/// edge.
pub(crate) fn paint_keycap(ui: &egui::Ui, left: f32, center_y: f32, chord: &str) -> f32 {
    let p = palette(ui.ctx());
    let g = ui.painter().layout_no_wrap(
        chord.to_owned(),
        FontId::monospace(theme::MONO_SMALL_SIZE),
        p.text_dim,
    );
    let r = egui::Rect::from_min_max(
        egui::pos2(left, center_y - 10.0),
        egui::pos2(left + g.size().x + 12.0, center_y + 10.0),
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
    r.max.x
}
