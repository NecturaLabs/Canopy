//! The reading dialog for the Terms of Service and the Privacy Policy: a tall card at the
//! reading measure with both documents as tabs, a scrolling body that shows how far the reader
//! has come, and a footer that either closes it or carries the consent's own answers.

use egui::{Align, Layout, Rangef, Rect, Sense, UiBuilder, Vec2, pos2, vec2};

use super::*;
use crate::ui::kit::IconButton;
use crate::ui::kit::choice::UnderlineTabs;
use crate::ui::kit::overlay::{self, Layer};
use crate::ui::legal::{self, Link};
use crate::ui::motion;

/// The glyph disc before the tabs (as the kit dialog's).
const DISC: f32 = 30.0;
/// The scrollbar's lane beside the text column.
const BAR_LANE: f32 = 12.0;
/// Thickness of the reading-progress line on the header's seam.
const PROGRESS: f32 = 2.0;
/// A scroll far past either end of any document (Home and End).
const TO_THE_END: f32 = 1.0e6;

/// The footer of a reading dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Footer<'a> {
    /// A Close button; Esc and the scrim close it too.
    Close,
    /// The consent's two answers (`first`, rightmost, then `second`) beside a Back button. Equal
    /// answers (`neutral`) are both Secondary; otherwise `first` is Primary. Esc goes back;
    /// the scrim does nothing.
    Consent {
        first: &'a str,
        second: &'a str,
        neutral: bool,
    },
}

/// What the reader's controls answered this frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Answer {
    Open,
    /// Close, Esc or the scrim (`Footer::Close`).
    Close,
    /// Back to the consent summary (`Footer::Consent`: the Back button or Esc).
    Back,
    /// The consent's first answer.
    First,
    /// The consent's second answer.
    Second,
    /// A link in the document.
    Link(Link),
}

/// Shows the reading dialog for `doc` (its tabs switch it) and returns what was answered.
pub(super) fn show(ctx: &egui::Context, doc: &mut Document, footer: Footer<'_>) -> Answer {
    let p = palette(ctx);
    let m = theme::metrics();
    let id = egui::Id::new("legal-reader");
    let consent = matches!(footer, Footer::Consent { .. });
    let (_, fresh) = overlay::presence(ctx, id);
    if fresh {
        let trigger = ctx.memory(egui::Memory::focused);
        ctx.data_mut(|d| d.insert_temp(id.with("trigger"), trigger));
    }
    let entrance = overlay::Entrance::of(ctx, id, motion::SLOW, motion::SLOW, false, None);
    // As tall as fits between the tab strip and the status bar, centred on the window as egui
    // centres a modal; as wide as the reading measure and its margins.
    let screen = ctx.content_rect();
    let room = overlay::modal_room(ctx);
    let max_h = 2.0 * (screen.center().y - room.top()).min(room.bottom() - screen.center().y);
    let pad = m.dialog_pad;
    let gutter = theme::XL;
    let size = vec2(
        (legal::measure(ctx) + 2.0 * gutter + BAR_LANE).min(room.width()),
        max_h.min(theme::SETTINGS_MAX_HEIGHT),
    )
    .max(vec2(DISC * 8.0, DISC * 6.0));
    let mut answer = Answer::Open;
    let modal = egui::Modal::new(id)
        .frame(egui::Frame::NONE)
        .backdrop_color(p.scrim.gamma_multiply(entrance.fade))
        .show(ctx, |ui| {
            let layer = ui.layer_id();
            let start = overlay::next_shape(ui.ctx(), layer);
            ui.multiply_opacity(entrance.fade);
            let (card, _) = ui.allocate_exact_size(size, Sense::hover());
            ui.painter()
                .extend(overlay::surface_shapes(ui.painter(), card, Layer::Dialog));
            ctx.accesskit_node_builder(ui.id(), |n| {
                n.set_role(egui::accesskit::Role::Dialog);
                n.set_label(doc.title());
            });
            let s = theme::surfaces(ui.ctx());

            // Header: the document's glyph, the two documents as tabs, Close.
            let header = Rect::from_min_size(
                card.min,
                vec2(card.width(), pad + DISC.max(m.control) + theme::SM),
            );
            let mut head = ui.new_child(
                UiBuilder::new()
                    .id_salt("legal-reader-header")
                    .max_rect(header.shrink2(vec2(pad, 0.0)).with_min_y(card.top() + pad))
                    .layout(Layout::left_to_right(Align::Center)),
            );
            head.spacing_mut().item_spacing.x = theme::MD;
            let (disc, _) = head.allocate_exact_size(Vec2::splat(DISC), Sense::hover());
            head.painter()
                .circle_filled(disc.center(), DISC / 2.0, p.accent_tint());
            head.painter().text(
                disc.center(),
                egui::Align2::CENTER_CENTER,
                doc.glyph(),
                FontId::proportional(m.icon_md),
                p.accent,
            );
            let options = [
                (Document::Terms, Document::Terms.title()),
                (Document::Privacy, Document::Privacy.title()),
            ];
            UnderlineTabs::new(doc, &options)
                .label("Legal documents")
                .id_salt("legal-reader-tabs")
                .show(&mut head);
            if !consent {
                head.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if IconButton::new(icon::X, "Close")
                        .size(Size::Sm)
                        .id_salt("legal-reader-close")
                        .show(ui)
                        .clicked()
                    {
                        answer = Answer::Close;
                    }
                });
            }

            // Footer: Back or a hint at the left, the answers at the right.
            let footer_rect = Rect::from_min_max(
                pos2(card.left(), card.bottom() - pad - m.control - theme::MD),
                card.max,
            );
            let row = Rect::from_min_size(
                pos2(card.left() + pad, footer_rect.top() + theme::MD),
                vec2(card.width() - 2.0 * pad, m.control),
            );
            let mut foot = ui.new_child(
                UiBuilder::new()
                    .id_salt("legal-reader-footer")
                    .max_rect(row)
                    .layout(Layout::left_to_right(Align::Center)),
            );
            foot.spacing_mut().item_spacing.x = theme::SM;
            match footer {
                Footer::Close => {
                    foot.label(
                        RichText::new("Esc closes")
                            .size(m.meta_size)
                            .color(p.text_faint),
                    );
                    foot.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if Button::new("Close")
                            .primary()
                            .size(Size::Md)
                            .id_salt("legal-reader-done")
                            .show(ui)
                            .clicked()
                        {
                            answer = Answer::Close;
                        }
                    });
                }
                Footer::Consent {
                    first,
                    second,
                    neutral,
                } => {
                    if Button::new("Back")
                        .ghost()
                        .size(Size::Md)
                        .icon(icon::ARROW_LEFT)
                        .id_salt("legal-reader-back")
                        .show(&mut foot)
                        .clicked()
                    {
                        answer = Answer::Back;
                    }
                    // Made left to right (the second answer first) so Tab meets them in reading
                    // order; placed at the right by their width last frame, as the kit footer.
                    let width_key = id.with("answers-width");
                    let last: f32 = ctx.data(|d| d.get_temp(width_key)).unwrap_or(0.0);
                    let slot = Rect::from_min_max(
                        pos2((row.right() - last).max(foot.cursor().left()), row.top()),
                        row.max,
                    );
                    let answers = foot.scope_builder(
                        UiBuilder::new()
                            .max_rect(slot)
                            .layout(Layout::left_to_right(Align::Center)),
                        |ui| {
                            ui.spacing_mut().item_spacing.x = theme::SM;
                            if Button::new(second)
                                .secondary()
                                .size(Size::Md)
                                .id_salt("legal-reader-second")
                                .show(ui)
                                .clicked()
                            {
                                answer = Answer::Second;
                            }
                            let b = Button::new(first)
                                .size(Size::Md)
                                .id_salt("legal-reader-first");
                            let b = if neutral { b.secondary() } else { b.primary() };
                            if b.show(ui).clicked() {
                                answer = Answer::First;
                            }
                        },
                    );
                    let width = answers.response.rect.width();
                    if (width - last).abs() > 0.5 {
                        ctx.data_mut(|d| d.insert_temp(width_key, width));
                        ctx.request_repaint();
                    }
                }
            }

            // Body: the document in a centred column at the measure, scrolling between the
            // header's seam and the footer's.
            let body = Rect::from_min_max(
                pos2(card.left(), header.bottom()),
                pos2(card.right(), footer_rect.top()),
            );
            paint::seam_h(ui.painter(), card.x_range(), body.top(), s.border);
            paint::seam_h(ui.painter(), card.x_range(), body.bottom(), s.border);
            let mut inner = ui.new_child(
                UiBuilder::new()
                    .id_salt("legal-reader-body")
                    .max_rect(body.with_min_y(body.top() + paint::SEAM))
                    .layout(Layout::top_down(Align::Min)),
            );
            inner.set_clip_rect(body.intersect(ui.clip_rect()));
            let view = body.height() - paint::SEAM;
            let ground = overlay::ground(ctx, Layer::Dialog);
            let out = overlay::scroll_body(
                &mut inner,
                ("legal-reader-scroll", *doc),
                view,
                ground,
                |ui| {
                    scroll_keys(ui, view);
                    let column = legal::measure(ui.ctx())
                        .min(ui.available_width() - 2.0 * gutter)
                        .max(DISC * 4.0);
                    let left = ((ui.available_width() - column) / 2.0).max(0.0);
                    ui.add_space(theme::XL);
                    let link = ui
                        .horizontal_top(|ui| {
                            ui.add_space(left);
                            ui.vertical(|ui| {
                                ui.set_width(column);
                                legal::show(ui, *doc)
                            })
                            .inner
                        })
                        .inner;
                    ui.add_space(theme::XXL);
                    link
                },
            );
            if let Some(link) = out.inner {
                answer = Answer::Link(link);
            }
            // How far through the document: a line along the header's seam.
            let scrollable = out.content_size.y - out.inner_rect.height();
            if scrollable > 0.5 {
                let t = (out.state.offset.y / scrollable).clamp(0.0, 1.0);
                let line = Rangef::new(card.left(), card.left() + card.width() * t);
                ui.painter().line_segment(
                    [
                        pos2(line.min, body.top() + PROGRESS / 2.0),
                        pos2(line.max, body.top() + PROGRESS / 2.0),
                    ],
                    egui::Stroke::new(PROGRESS, p.accent),
                );
            }
            // The ring goes over the chrome painted inside the card.
            paint::rim(
                ui.painter(),
                card,
                Layer::Dialog.radius(),
                p.floating_ring(s),
            );
            let center = card.center();
            overlay::transform_since(
                ui.ctx(),
                layer,
                start,
                entrance.transform(center, overlay::DIALOG_SCALE, Vec2::ZERO),
            );
            let end = overlay::next_shape(ui.ctx(), layer);
            overlay::record_exit(
                ui.ctx(),
                id,
                layer,
                0..start.0,
                start.0..end.0,
                overlay::ExitMotion {
                    seconds: motion::EXIT_DIALOG,
                    scale: overlay::DIALOG_EXIT_SCALE,
                    origin: center,
                    offset: Vec2::ZERO,
                    instant_on_input: false,
                },
            );
        });
    if answer == Answer::Open {
        let escape = ctx.input(|i| i.key_pressed(Key::Escape));
        if consent {
            if escape {
                answer = Answer::Back;
            }
        } else if modal.should_close() {
            answer = Answer::Close;
        }
    }
    if matches!(
        answer,
        Answer::Close | Answer::Back | Answer::First | Answer::Second
    ) {
        let trigger: Option<egui::Id> = ctx
            .data(|d| d.get_temp::<Option<egui::Id>>(id.with("trigger")))
            .flatten();
        ctx.data_mut(|d| d.remove::<Option<egui::Id>>(id.with("trigger")));
        if let Some(t) = trigger {
            ctx.memory_mut(|m| m.request_focus(t));
        }
    }
    answer
}

/// Scrolls the document from the keyboard: Page Up/Down and Home/End always, Space and the
/// arrows while no control has focus (focused, they belong to it).
fn scroll_keys(ui: &Ui, view: f32) {
    let page = (view - 3.0 * theme::XL).max(view * 0.5);
    let step = theme::XL * 2.0;
    let free = ui.memory(|m| m.focused().is_none());
    let delta = ui.input(|i| {
        let pressed = |k: Key| i.key_pressed(k) && i.modifiers.is_none();
        if pressed(Key::PageDown) || (free && pressed(Key::Space)) {
            -page
        } else if pressed(Key::PageUp) || (free && i.key_pressed(Key::Space) && i.modifiers.shift) {
            page
        } else if pressed(Key::End) {
            -TO_THE_END
        } else if pressed(Key::Home) {
            TO_THE_END
        } else if free && pressed(Key::ArrowDown) {
            -step
        } else if free && pressed(Key::ArrowUp) {
            step
        } else {
            0.0
        }
    });
    if delta != 0.0 {
        let seconds = motion::time(motion::BASE);
        let animation = if seconds > 0.0 {
            egui::style::ScrollAnimation::duration(seconds)
        } else {
            egui::style::ScrollAnimation::none()
        };
        ui.scroll_with_delta_animation(vec2(0.0, delta), animation);
    }
}
