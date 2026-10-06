//! The keyboard cheatsheet (`?` or Ctrl+/): every binding from [`crate::keymap::BINDINGS`],
//! grouped by section and searchable, in an overlay styled like the command palette.

use egui::{Align2, FontId, Id, LayerId, Order, RichText, Sense, vec2};
use egui_phosphor::regular as icon;

use super::kit::a11y::{self, FocusReturn};
use super::kit::chip::Keycap;
use super::kit::overlay::{self, Layer};
use super::kit::row::SectionHeader;
use super::kit::{IconButton, Size};
use super::palette::KEY_GAP;
use super::theme::{self, palette};
use crate::keymap::{self, BINDINGS, Section};

/// Cheatsheet state.
#[derive(Debug, Default)]
pub struct Cheatsheet {
    pub open: bool,
    query: String,
    just_opened: bool,
    /// The control focus goes back to when the cheatsheet closes.
    back: FocusReturn,
}

/// The cheatsheet's own layers: the scrim (which traps focus above it) and the card.
fn scrim_layer() -> LayerId {
    LayerId::new(Order::Foreground, Id::new("cheatsheet-scrim"))
}
fn card_layer() -> LayerId {
    LayerId::new(Order::Tooltip, Id::new("cheatsheet"))
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
            self.back.restore(ctx, card_layer());
            return;
        }
        // Tab stays in the cheatsheet, and nothing behind it takes a key or a click.
        a11y::trap_focus(ctx, scrim_layer());
        let p = palette(ctx);
        let m = theme::metrics();
        let screen = ctx.content_rect();
        egui::Area::new(scrim_layer().id)
            .order(Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                // A click target only: no Tab stop, no unnamed node.
                let resp = ui.allocate_rect(screen, Sense::CLICK);
                overlay::scrim(ui.painter(), screen, 1.0);
                if resp.clicked() {
                    self.open = false;
                }
            });
        let width = 860.0_f32
            .min(screen.width() - theme::XL * 2.0 - theme::LG * 2.0)
            .max(0.0);
        let height = (screen.height() - theme::XL * 2.0 - theme::LG * 2.0).max(0.0);
        egui::Area::new(card_layer().id)
            .order(Order::Tooltip)
            .pivot(Align2::CENTER_TOP)
            .fixed_pos(egui::pos2(screen.center().x, screen.min.y + theme::XL))
            .show(ctx, |ui| {
                overlay::floating(ui, Layer::Popover, egui::Margin::ZERO, |ui| {
                    a11y::dialog(ui, "Keyboard shortcuts", false);
                    ui.set_width(width);
                    let content_top = ui.cursor().min.y;
                    let pad = m.card_pad;
                    egui::Frame::NONE
                        .inner_margin(egui::Margin {
                            left: crate::ui::points_i8(pad),
                            right: crate::ui::points_i8(pad),
                            top: crate::ui::points_i8(pad),
                            bottom: crate::ui::points_i8(theme::XS),
                        })
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(icon::KEYBOARD)
                                        .size(m.icon_lg)
                                        .color(p.accent),
                                );
                                ui.label(
                                    RichText::new("Keyboard shortcuts")
                                        .font(theme::heading(m.title_size))
                                        .color(p.text),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if IconButton::new(icon::X, "Close shortcuts (Esc)")
                                            .size(Size::Sm)
                                            .show(ui)
                                            .clicked()
                                        {
                                            self.open = false;
                                        }
                                    },
                                );
                            });
                            ui.add(
                                egui::Label::new(
                                    RichText::new(
                                        "Single keys work when no text field has focus. Capital letters mean Shift plus that key. Super is left to your window manager.",
                                    )
                                    .size(m.meta_size)
                                    .color(p.text_dim),
                                )
                                .wrap(),
                            );
                        });
                    let edit = super::palette::surface_search(
                        ui,
                        &mut self.query,
                        "Search shortcuts…",
                        "Search shortcuts",
                    );
                    if self.just_opened {
                        self.back.remember(ctx);
                        edit.request_focus();
                        self.just_opened = false;
                    }
                    let q = self.query.trim().to_lowercase();
                    let remaining = (height - (ui.cursor().min.y - content_top) - pad).max(0.0);
                    let ground = overlay::ground(ctx, Layer::Popover);
                    egui::Frame::NONE
                        .inner_margin(egui::Margin::symmetric(crate::ui::points_i8(pad), crate::ui::points_i8(theme::SM)))
                        .show(ui, |ui| {
                            overlay::scroll_body(ui, "cheatsheet-list", remaining, ground, |ui| {
                                    if width < 800.0 {
                                        for section in Section::ALL {
                                            rows(ui, section, &q);
                                        }
                                    } else {
                                        ui.columns(2, |cols| {
                                            let heights: Vec<f32> = Section::ALL
                                                .iter()
                                                .map(|section| {
                                                    section_height(&cols[0], *section, &q)
                                                })
                                                .collect();
                                            let total: f32 = heights.iter().sum();
                                            let split = (1..Section::ALL.len())
                                                .min_by(|a, b| {
                                                    let imbalance = |n: usize| {
                                                        (2.0 * heights[..n].iter().sum::<f32>()
                                                            - total)
                                                            .abs()
                                                    };
                                                    imbalance(*a).total_cmp(&imbalance(*b))
                                                })
                                                .unwrap_or(1);
                                            for (i, section) in Section::ALL.iter().enumerate() {
                                                rows(
                                                    &mut cols[usize::from(i >= split)],
                                                    *section,
                                                    &q,
                                                );
                                            }
                                        });
                                    }
                            });
                        });
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

/// A row's height: one line, or two when the keys do not fit beside the description.
fn row_height(stacked: bool) -> f32 {
    let m = theme::metrics();
    if stacked {
        m.keycap_height + m.row_height
    } else {
        m.row_height
    }
}

fn section_height(ui: &egui::Ui, section: Section, query: &str) -> f32 {
    let m = theme::metrics();
    let items = matching(section, query);
    if items.is_empty() {
        return 0.0;
    }
    let mut height = m.section_header_height + theme::MD;
    for (binding, keys) in items {
        let description = ui.painter().layout_no_wrap(
            binding.description.to_owned(),
            FontId::proportional(m.body_size),
            palette(ui.ctx()).text,
        );
        let stacked =
            description.size().x + keycaps_width(ui, &keys) + theme::LG > ui.available_width();
        height += row_height(stacked) + ui.spacing().item_spacing.y;
    }
    height
}

/// One section of bindings: its header, then a row per binding with the description left and
/// the keys as keycaps right (below it when the row is too narrow). Shared with Settings.
pub(crate) fn rows(ui: &mut egui::Ui, section: Section, query: &str) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let items = matching(section, query);
    if items.is_empty() {
        return;
    }
    let _ = SectionHeader::new(section.title()).first(true).show(ui);
    for (b, keys) in items {
        let key_width = keycaps_width(ui, &keys);
        let description = ui.painter().layout_no_wrap(
            b.description.to_owned(),
            FontId::proportional(m.body_size),
            p.text,
        );
        let stacked = description.size().x + key_width + theme::LG > ui.available_width();
        let (rect, _) = ui.allocate_exact_size(
            vec2(ui.available_width(), row_height(stacked)),
            Sense::hover(),
        );
        let description_y = if stacked {
            rect.min.y + m.row_height / 2.0
        } else {
            rect.center().y
        };
        ui.painter()
            .with_clip_rect(rect.intersect(ui.clip_rect()))
            .galley(
                egui::pos2(
                    rect.min.x + theme::XS,
                    description_y - description.size().y / 2.0,
                ),
                description,
                p.text,
            );
        let key_y = if stacked {
            rect.max.y - m.keycap_height / 2.0
        } else {
            rect.center().y
        };
        // Key caps, right-aligned.
        let mut x = rect.right() - theme::XS;
        for chord in keys.iter().rev() {
            let cap = Keycap::new(chord.clone());
            let size = cap.measure(ui);
            x -= size.x;
            cap.paint(
                ui,
                egui::Rect::from_min_size(egui::pos2(x, key_y - size.y / 2.0), size),
            );
            x -= KEY_GAP;
        }
    }
    ui.add_space(theme::MD);
}

/// The width of a row of key caps, with the gaps between them.
fn keycaps_width(ui: &egui::Ui, chords: &[String]) -> f32 {
    chords
        .iter()
        .map(|c| Keycap::new(c.clone()).measure(ui).x + KEY_GAP)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alternative_shortcuts_have_separate_keycaps() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let chords = keymap::all_keys(keymap::Command::Palette);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    vec2(960.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| {
                rows(ui, Section::Anywhere, "command palette");
            },
        );
        output.textures_delta.clear();
        let mut caps: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape
                    && chords.iter().any(|chord| chord == text.galley.text())
                {
                    Some(text.visual_bounding_rect())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(caps.len(), chords.len());
        caps.sort_by(|a, b| a.min.x.total_cmp(&b.min.x));
        assert!(
            caps.windows(2).all(|pair| pair[0].max.x < pair[1].min.x),
            "alternative shortcut labels overlap: {caps:?}"
        );
    }

    /// A11Y-7, A11Y-9, A11Y-21: the cheatsheet is a modal dialog named "Keyboard shortcuts" with a
    /// named search that has focus; Tab stays inside it, and focus goes back to what had it when
    /// it closes.
    #[test]
    fn the_cheatsheet_is_a_modal_dialog_that_gives_focus_back() {
        use egui::accesskit::Role;
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        ctx.enable_accesskit();
        let mut sheet = Cheatsheet::default();
        let tab = || {
            vec![egui::Event::Key {
                key: egui::Key::Tab,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }]
        };
        let frame = |sheet: &mut Cheatsheet, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    crate::ui::kit::Button::new("Behind").show(ui);
                    sheet.show(ui.ctx());
                },
            );
            out.textures_delta.clear();
            out.platform_output.accesskit_update.expect("accesskit")
        };
        frame(&mut sheet, Vec::new());
        frame(&mut sheet, tab());
        let behind = ctx
            .memory(egui::Memory::focused)
            .expect("the button behind has focus");
        sheet.toggle();
        frame(&mut sheet, Vec::new());
        let update = frame(&mut sheet, Vec::new());
        let (_, dialog) = update
            .nodes
            .iter()
            .find(|(_, n)| n.role() == Role::Dialog)
            .expect("a dialog");
        assert_eq!(dialog.label(), Some("Keyboard shortcuts"));
        assert!(dialog.is_modal(), "modal");
        let (search, _) = update
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Search shortcuts"))
            .expect("a named search");
        assert_eq!(update.focus, *search, "the search has focus");
        for _ in 0..3 {
            frame(&mut sheet, tab());
            frame(&mut sheet, Vec::new());
            let focused = ctx
                .memory(egui::Memory::focused)
                .and_then(|f| ctx.read_response(f));
            assert!(
                focused.is_some_and(|r| r.layer_id == card_layer()),
                "Tab stays in the cheatsheet"
            );
        }
        sheet.toggle();
        frame(&mut sheet, Vec::new());
        assert_eq!(
            ctx.memory(egui::Memory::focused),
            Some(behind),
            "focus is back on what had it"
        );
    }
}
