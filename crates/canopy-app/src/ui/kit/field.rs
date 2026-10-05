//! Text inputs: [`TextField`], [`SearchField`], [`TextArea`] and [`CodeArea`]. Design spec 4.2.
//!
//! Each wraps an `egui::TextEdit` with no frame of its own: the kit reserves the field's rect,
//! paints the `field` fill (and Deeper's recess) under the text, then the border, the focus or
//! error halo, the leading icon and the trailing slot (clear button, keycap, counter) once the
//! edit's hover and focus are known. A label above, and a helper or error message below, are
//! optional. Text fields always show focus, whatever the input modality.

use std::ops::RangeInclusive;
use std::sync::Arc;

use egui::accesskit::Invalid;
use egui::{
    Align, Color32, CornerRadius, FontFamily, FontId, Galley, Id, Key, Layout, Margin, Modifiers,
    Rect, Response, Sense, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, Vec2, Widget, WidgetInfo,
    WidgetType, pos2, vec2,
};
use egui_phosphor::regular as icon;

use super::button::{spinner_due, tooltip_text, with_tooltip};
use super::toggle::{component_id, gallery_caption, remembered};
use super::{ForceState, IconButton, KitResponse, Size, anim, paint};
use crate::keymap::{self, Command};
use crate::ui::motion;
use crate::ui::theme::{self, Elevation, Palette};

/// The label above a field and the message below it (concept `.k-lab`, `.k-errmsg`).
const LABEL_SIZE: f32 = 12.0;
const MESSAGE_ICON: f32 = 14.0;
const MESSAGE_GAP: f32 = 6.0;
/// The focus and error halo, drawn outside the border (3 pt; focus at 26%, error at 18%).
const HALO: f32 = 3.0;
const FOCUS_HALO_ALPHA: u8 = 66;
const ERROR_HALO_ALPHA: u8 = 46;
/// The hover border: `control_border` moved 40% towards `text_dim`.
const HOVER_MIX: f32 = 0.4;
/// Gap between the leading icon and the text, and the search field's right padding (concept
/// `.k-search`: gap 8, padding 0 5 0 10).
const ICON_GAP: f32 = 8.0;
const SEARCH_PAD_RIGHT: f32 = 5.0;
/// Text areas: vertical padding inside the field.
const AREA_PAD_Y: f32 = 8.0;
/// The summary counter turns `warning` past 50 characters and `danger` past 72.
const COUNTER_WARN: usize = 51;
const COUNTER_DANGER: usize = 73;
/// Deeper's search recess is a little lighter than the field's (spec 2.3: 64 vs 71 dark, 15 vs 18
/// light).
const SEARCH_RECESS: (u8, u8) = (64, 15);
/// The keycap hint in an empty search field (concept `.k-key`).
const KEYCAP_PAD: f32 = 5.0;
/// Line-number gutter of a code area: padding either side of the numbers.
const GUTTER_PAD: f32 = 8.0;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    Line,
    Search,
    Area {
        rows: RangeInclusive<usize>,
    },
    Code {
        rows: RangeInclusive<usize>,
        line_numbers: bool,
    },
}

impl Kind {
    fn multiline(&self) -> bool {
        matches!(self, Self::Area { .. } | Self::Code { .. })
    }
}

/// Everything the four fields share.
#[derive(Debug)]
struct Core<'t> {
    text: &'t mut String,
    kind: Kind,
    label: Option<String>,
    name: Option<String>,
    placeholder: String,
    helper: Option<String>,
    error: Option<String>,
    icon: Option<String>,
    password: bool,
    loading: bool,
    read_only: bool,
    enabled: bool,
    tooltip: Option<String>,
    disabled_reason: Option<String>,
    width: Option<f32>,
    size: Size,
    counter: bool,
    guide: Option<usize>,
    shortcut: Option<Command>,
    id_salt: Option<Id>,
    force: Option<ForceState>,
}

impl<'t> Core<'t> {
    fn new(text: &'t mut String, kind: Kind) -> Self {
        Self {
            text,
            kind,
            label: None,
            name: None,
            placeholder: String::new(),
            helper: None,
            error: None,
            icon: None,
            password: false,
            loading: false,
            read_only: false,
            enabled: true,
            tooltip: None,
            disabled_reason: None,
            width: None,
            size: Size::Md,
            counter: false,
            guide: None,
            shortcut: None,
            id_salt: None,
            force: None,
        }
    }

    /// Shows the field; returns the edit's response and whether Ctrl+Enter submitted it.
    fn show(self, ui: &mut Ui) -> (Response, bool) {
        let enabled = self.enabled && ui.is_enabled();
        let id = component_id(ui, "kit-field", self.id_salt);
        ui.scope(|ui| {
            if !enabled {
                ui.disable();
            }
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                self.show_inner(ui, id, enabled)
            })
            .inner
        })
        .inner
    }

    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "values drawn on screen, where f32 rounding is invisible; egui lays out in f32 points, u8 channels and i8 margins; the values are bounded by the layout and float casts saturate; a negative value saturates to 0, the clamp the layout wants"
    )]
    fn show_inner(self, ui: &mut Ui, id: Id, enabled: bool) -> (Response, bool) {
        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let style = theme::style();
        let s = p.surfaces(style);
        let m = theme::metrics();
        let width = self
            .width
            .unwrap_or_else(|| ui.available_width())
            .max(m.control * 2.0);

        let label_id = self.label.as_ref().map(|text| {
            let g = ui.painter().layout_no_wrap(
                text.clone(),
                FontId::new(LABEL_SIZE, theme::medium()),
                p.text_dim,
            );
            let (rect, _) = ui.allocate_exact_size(vec2(width, g.size().y), Sense::hover());
            ui.painter().galley(rect.min, g, p.text_dim);
            let r = ui.interact(rect, id.with("label"), Sense::hover());
            r.widget_info(|| WidgetInfo::labeled(WidgetType::Label, enabled, text));
            ui.add_space(m.field_label_gap);
            r.id
        });

        let font = match self.kind {
            Kind::Code { .. } => FontId::new(m.mono_size, FontFamily::Monospace),
            _ => FontId::new(m.body_size, FontFamily::Proportional),
        };
        let line_h = ui.fonts_mut(|f| f.row_height(&font)) + ui.spacing().extra_text_line_spacing;
        let pad_y = if self.kind.multiline() {
            AREA_PAD_Y
        } else {
            0.0
        };
        let height = match &self.kind {
            Kind::Area { rows } | Kind::Code { rows, .. } => {
                let min = *rows.start() as f32 * line_h + 2.0 * pad_y;
                let max = (*rows.end()).max(*rows.start()) as f32 * line_h + 2.0 * pad_y;
                let content: f32 = ctx
                    .data(|d| d.get_temp(id.with("kit-area-content")))
                    .unwrap_or(min);
                let target = content.clamp(min, max);
                anim::ease_value(&ctx, id.with("kit-area-height"), target, motion::BASE)
            }
            _ => self.size.height(),
        };
        let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        let radius = CornerRadius::same(m.radius);
        let painter = ui.painter().clone();
        painter.rect_filled(rect, radius, s.field);
        let material = p.material(style, Elevation::Field);
        if material.recess > 0 {
            let recess = if self.kind == Kind::Search {
                if p.dark {
                    SEARCH_RECESS.0
                } else {
                    SEARCH_RECESS.1
                }
            } else {
                material.recess
            };
            paint::recess(&painter, rect, radius, p.shadow, recess);
        }

        // The slots either side of the text.
        let icon_glyph = match self.kind {
            Kind::Search => Some(icon::MAGNIFYING_GLASS.to_owned()),
            _ => self.icon.clone(),
        };
        let mut left = m.field_pad_x;
        if icon_glyph.is_some() {
            left += m.icon_md + ICON_GAP;
        }
        let gutter = match &self.kind {
            Kind::Code {
                line_numbers: true, ..
            } => {
                let lines =
                    self.text.lines().count().max(1) + usize::from(self.text.ends_with('\n'));
                let digits = lines.to_string().len().max(2);
                let w = ui.fonts_mut(|f| {
                    f.layout_no_wrap("0".repeat(digits), font.clone(), Color32::PLACEHOLDER)
                        .size()
                        .x
                });
                Some(w + 2.0 * GUTTER_PAD)
            }
            _ => None,
        };
        if let Some(g) = gutter {
            left = g + GUTTER_PAD;
        }
        let filled = !self.text.is_empty();
        let counter = self.counter.then(|| {
            let n = self.text.chars().count();
            let color = if n >= COUNTER_DANGER {
                p.danger
            } else if n >= COUNTER_WARN {
                p.warning
            } else {
                p.text_faint
            };
            let g = ui.painter().layout_no_wrap(
                n.to_string(),
                FontId::new(m.small_size, FontFamily::Monospace),
                color,
            );
            (g, color)
        });
        let keycap = (self.kind == Kind::Search && !filled)
            .then_some(self.shortcut)
            .flatten()
            .map(|c| {
                // The bare key when there is one (concept 1 shows `/`), else the chord.
                let hint = keymap::hint(c);
                let key = hint.rsplit(" · ").next().unwrap_or_default().to_owned();
                ui.painter().layout_no_wrap(
                    key,
                    FontId::new(m.chord_size, FontFamily::Monospace),
                    p.text_dim,
                )
            });
        let clear_side = Size::Xs.height();
        let mut right = if self.kind == Kind::Search {
            SEARCH_PAD_RIGHT
        } else {
            m.field_pad_x
        };
        if self.kind == Kind::Search {
            let cap = keycap.as_ref().map_or(0.0, |g| {
                (g.size().x + 2.0 * KEYCAP_PAD).max(m.keycap_height)
            });
            right += cap.max(clear_side) + ICON_GAP;
        }
        if let Some((g, _)) = &counter {
            right += g.size().x + ICON_GAP;
        }
        if self.password {
            right += clear_side + ICON_GAP;
        }
        if self.loading {
            right += m.icon_md + ICON_GAP;
        }
        let reveal_id = id.with("kit-field-reveal");
        let revealed =
            self.password && ctx.data(|d| d.get_temp::<bool>(reveal_id)).unwrap_or(false);

        // The edit itself, in a frameless child over the reserved rect.
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Center)),
        );
        {
            let v = child.visuals_mut();
            v.weak_text_color = Some(p.text_faint);
            v.selection.bg_fill = p.selected();
        }
        let interactive = enabled && !self.read_only;
        let text_color = if self.read_only { p.text_dim } else { p.text };
        let margin = Margin {
            left: left.round() as i8,
            right: right.round() as i8,
            top: pad_y as i8,
            bottom: pad_y as i8,
        };
        let multiline = self.kind.multiline();
        let edit_id = id.with("kit-edit");
        let mut edit = if multiline {
            TextEdit::multiline(self.text)
        } else {
            TextEdit::singleline(self.text)
        }
        .id(edit_id)
        .frame(egui::Frame::NONE.inner_margin(margin))
        .font(font.clone())
        .text_color(text_color)
        .hint_text(self.placeholder.clone())
        .interactive(interactive)
        .password(self.password && !revealed)
        .desired_width(width);
        if let Kind::Code { .. } = self.kind {
            edit = edit.code_editor();
        }
        let (mut response, content_h, galley, galley_pos) = if multiline {
            let rows = match &self.kind {
                Kind::Area { rows } | Kind::Code { rows, .. } => *rows.start(),
                _ => 1,
            };
            let out = egui::ScrollArea::vertical()
                .id_salt(id.with("kit-area-scroll"))
                .max_height(height)
                .auto_shrink(false)
                .show(&mut child, |ui| {
                    edit.desired_rows(rows)
                        .min_size(vec2(width, height))
                        .show(ui)
                });
            let o = out.inner;
            let content = o.galley.size().y + 2.0 * pad_y;
            // The edit is as tall as its text; the field is what shows of it.
            let mut response = o.response.response;
            response.rect = rect;
            (response, content, o.galley, o.galley_pos)
        } else {
            let o = edit
                .min_size(vec2(width, height))
                .vertical_align(Align::Center)
                .show(&mut child);
            let h = o.galley.size().y;
            (o.response.response, h, o.galley, o.galley_pos)
        };
        if multiline {
            ctx.data_mut(|d| d.insert_temp(id.with("kit-area-content"), content_h));
        }

        // Esc in a search field clears it first, then lets go of focus.
        let escaped = ui.input(|i| i.key_pressed(Key::Escape));
        if self.kind == Kind::Search && escaped && response.lost_focus() && filled {
            self.text.clear();
            response.mark_changed();
            response.request_focus();
        }
        let submitted = multiline
            && response.has_focus()
            && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter));

        let force = self.force.unwrap_or_default();
        let hovered = interactive && (force.hover || response.hovered());
        let focused = force.focus || response.has_focus();
        let focus_t = match self.force {
            Some(f) => f32::from(u8::from(focused)) * f.progress.unwrap_or(1.0),
            None => anim::ease_bool(&ctx, id.with("kit-field-focus"), focused, motion::BASE, 0.0),
        };
        let hover_t = match self.force {
            Some(f) => f32::from(u8::from(hovered)) * f.progress.unwrap_or(1.0),
            None => anim::ease_bool(
                &ctx,
                id.with("kit-field-hover"),
                hovered,
                motion::FAST,
                motion::FAST_OUT,
            ),
        };
        let error = self.error.as_deref().filter(|e| !e.is_empty());
        let border = if !interactive {
            s.border
        } else if error.is_some() {
            p.danger_base
        } else {
            let rest = p
                .control_border
                .lerp_to_gamma(theme::mix(p.control_border, p.text_dim, HOVER_MIX), hover_t);
            rest.lerp_to_gamma(p.focus, focus_t)
        };
        let halo = if error.is_some() && interactive {
            Some((p.danger_base, ERROR_HALO_ALPHA, 1.0_f32.max(focus_t)))
        } else if interactive && focus_t > 0.0 {
            Some((p.focus, FOCUS_HALO_ALPHA, focus_t))
        } else {
            None
        };
        if let Some((color, alpha, t)) = halo {
            let w = HALO * t;
            painter.rect_stroke(
                rect.expand(w / 2.0),
                radius + CornerRadius::same((w / 2.0).round() as u8),
                Stroke::new(w, Palette::tint(color, alpha)),
                StrokeKind::Middle,
            );
        }
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(paint::SEAM, border),
            StrokeKind::Inside,
        );

        if let Some(glyph) = icon_glyph {
            let g = painter.layout_no_wrap(glyph, FontId::proportional(m.icon_md), p.text_faint);
            let y = if multiline {
                rect.top() + pad_y + (line_h - g.size().y) / 2.0
            } else {
                rect.center().y - g.size().y / 2.0
            };
            painter.galley(pos2(rect.left() + m.field_pad_x, y), g, p.text_faint);
        }
        if let Some(gutter) = gutter {
            paint_line_numbers(&painter, rect, gutter, &galley, galley_pos, &font, p);
        }
        if let Some(col) = self.guide {
            let w = ui.fonts_mut(|f| {
                f.layout_no_wrap("0".repeat(col), font.clone(), Color32::PLACEHOLDER)
                    .size()
                    .x
            });
            let x = rect.left() + left + w;
            if x < rect.right() - right {
                let dashes = egui::Shape::dashed_line(
                    &[pos2(x, rect.top() + 2.0), pos2(x, rect.bottom() - 2.0)],
                    Stroke::new(paint::SEAM, s.border),
                    1.0,
                    2.0,
                );
                painter.extend(dashes);
            }
        }

        // Trailing slot, right to left: the search keycap or clear button, the show-password
        // toggle, the spinner, then the counter.
        let mut x = rect.right()
            - if self.kind == Kind::Search {
                SEARCH_PAD_RIGHT
            } else {
                m.field_pad_x
            };
        let cy = if multiline {
            rect.top() + pad_y + line_h / 2.0
        } else {
            rect.center().y
        };
        if self.kind == Kind::Search {
            let reveal = match self.force {
                Some(_) => f32::from(u8::from(filled)),
                None => anim::ease(&ctx, id.with("kit-search-clear"), filled, motion::FAST),
            };
            if let Some(g) = &keycap {
                let w = (g.size().x + 2.0 * KEYCAP_PAD).max(m.keycap_height);
                let cap = Rect::from_min_size(
                    pos2(x - w, cy - m.keycap_height / 2.0),
                    vec2(w, m.keycap_height),
                );
                paint_keycap(&painter, cap, g, p);
                x = cap.left() - ICON_GAP;
            } else if reveal > 0.0 && interactive {
                let at = Rect::from_min_size(
                    pos2(x - clear_side, cy - clear_side / 2.0),
                    Vec2::splat(clear_side),
                );
                let mut slot = ui.new_child(UiBuilder::new().max_rect(at));
                slot.multiply_opacity(reveal);
                let clear = slot.put(
                    at,
                    IconButton::new(icon::X, "Clear search")
                        .size(Size::Xs)
                        .id_salt(id.with("kit-search-clear-button")),
                );
                if clear.clicked() {
                    self.text.clear();
                    response.mark_changed();
                    response.request_focus();
                }
                x = at.left() - ICON_GAP;
            }
        }
        if self.password {
            let at = Rect::from_min_size(
                pos2(x - clear_side, cy - clear_side / 2.0),
                Vec2::splat(clear_side),
            );
            let (glyph, name) = if revealed {
                (icon::EYE_SLASH, "Hide secret")
            } else {
                (icon::EYE, "Show secret")
            };
            let toggle = ui.put(
                at,
                IconButton::new(glyph, name)
                    .size(Size::Xs)
                    .enabled(enabled)
                    .id_salt(id.with("kit-field-reveal-button")),
            );
            if toggle.clicked() {
                ctx.data_mut(|d| d.insert_temp(reveal_id, !revealed));
            }
            x = at.left() - ICON_GAP;
        }
        if spinner_due(&ctx, id, self.loading) {
            let r = m.icon_md / 2.0;
            paint::spinner(&painter, pos2(x - r, cy), r * 0.66, 1.6, p.text_faint);
            x -= m.icon_md + ICON_GAP;
        }
        if let Some((g, color)) = counter {
            painter.galley(pos2(x - g.size().x, cy - g.size().y / 2.0), g, color);
        }

        // Helper or error below.
        let message = error
            .map(|e| (e, true))
            .or(self.helper.as_deref().map(|h| (h, false)));
        if let Some((text, is_error)) = message {
            ui.add_space(MESSAGE_GAP);
            let color = if is_error { p.danger } else { p.text_dim };
            let mut job_x = 0.0;
            let icon_g = is_error.then(|| {
                painter.layout_no_wrap(
                    icon::WARNING_CIRCLE.to_owned(),
                    FontId::proportional(MESSAGE_ICON),
                    color,
                )
            });
            if let Some(g) = &icon_g {
                job_x = g.size().x + MESSAGE_GAP;
            }
            let g = painter.layout(
                text.to_owned(),
                FontId::new(LABEL_SIZE, FontFamily::Proportional),
                color,
                (width - job_x).max(1.0),
            );
            let (row, _) = ui.allocate_exact_size(vec2(width, g.size().y), Sense::hover());
            if let Some(ig) = icon_g {
                let first_row = g.rows.first().map_or(g.size().y, |r| r.rect().height());
                painter.galley(
                    pos2(row.left(), row.top() + (first_row - ig.size().y) / 2.0),
                    ig,
                    color,
                );
            }
            painter.galley(pos2(row.left() + job_x, row.top()), g, color);
        }

        if let Some(label_id) = label_id {
            response = response.labelled_by(label_id);
        }
        let name = self.name.clone().or_else(|| self.label.clone());
        let read_only = self.read_only;
        let description = error.map(str::to_owned).or(self.helper.clone());
        let has_error = error.is_some();
        ctx.accesskit_node_builder(response.id, |n| {
            if let Some(name) = &name {
                n.set_label(name.clone());
            }
            if read_only {
                n.set_read_only();
            }
            if multiline {
                n.set_role(egui::accesskit::Role::MultilineTextInput);
            }
            if let Some(d) = &description {
                n.set_description(d.clone());
            }
            if has_error {
                n.set_invalid(Invalid::True);
            }
        });
        let tip = tooltip_text(
            enabled,
            self.tooltip.as_deref(),
            self.disabled_reason.as_deref(),
            None,
        );
        (with_tooltip(response, tip), submitted)
    }
}

/// A keycap: `keycap` fill, `edge` ring and a 1 px bottom line, the chord in `text_dim` mono.
fn paint_keycap(painter: &egui::Painter, rect: Rect, chord: &Arc<Galley>, p: &Palette) {
    let s = theme::surfaces(painter.ctx());
    let m = theme::metrics();
    let radius = CornerRadius::same(m.keycap_radius);
    paint::fill(painter, rect, radius, s.keycap);
    paint::rim(painter, rect, radius, s.edge);
    painter.hline(
        egui::Rangef::new(rect.left() + 2.0, rect.right() - 2.0),
        rect.bottom() - 1.0,
        Stroke::new(1.0 / painter.pixels_per_point(), s.keycap_edge),
    );
    painter.galley(
        rect.center() - chord.size() / 2.0,
        Arc::clone(chord),
        p.text_dim,
    );
}

/// Line numbers in the code area's gutter (`gutter` wide), one per logical line, right-aligned
/// and level with the text rows, with a seam between the gutter and the code.
fn paint_line_numbers(
    painter: &egui::Painter,
    rect: Rect,
    gutter: f32,
    galley: &Galley,
    galley_pos: egui::Pos2,
    font: &FontId,
    p: &Palette,
) {
    let s = theme::surfaces(painter.ctx());
    let painter = painter.with_clip_rect(painter.clip_rect().intersect(rect.shrink(1.0)));
    let right = rect.left() + gutter - GUTTER_PAD;
    let mut line = 1;
    let mut starts_line = true;
    for row in &galley.rows {
        if starts_line {
            let g = painter.layout_no_wrap(line.to_string(), font.clone(), p.text_faint);
            let y = galley_pos.y + row.pos.y + (row.rect().height() - g.size().y) / 2.0;
            painter.galley(pos2(right - g.size().x, y), g, p.text_faint);
            line += 1;
        }
        starts_line = row.ends_with_newline;
    }
    paint::seam_v(
        &painter,
        rect.left() + gutter,
        rect.y_range().shrink(1.0),
        s.border,
    );
}

// ----------------------------------------------------------------------------------- builders

/// The setters every field shares, as an `impl` block for `$field`.
macro_rules! common_setters {
    ($field:ident) => {
        #[expect(
            clippy::allow_attributes,
            reason = "a setter is dead for some fields and used by others, which `expect` cannot say"
        )]
        impl $field<'_> {
            /// A label above the field (also its accessible name).
            #[allow(
                dead_code,
                reason = "shared by the four fields; not every field uses each yet"
            )]
            pub fn label(mut self, text: impl Into<String>) -> Self {
                self.core.label = Some(text.into());
                self
            }
            /// The accessible name when no label is shown.
            #[allow(
                dead_code,
                reason = "shared by the four fields; not every field uses each yet"
            )]
            pub fn name(mut self, text: impl Into<String>) -> Self {
                self.core.name = Some(text.into());
                self
            }
            #[allow(
                dead_code,
                reason = "shared by the four fields; not every field uses each yet"
            )]
            pub fn placeholder(mut self, text: impl Into<String>) -> Self {
                self.core.placeholder = text.into();
                self
            }
            /// A hint below the field (hidden while there is an error).
            #[allow(
                dead_code,
                reason = "shared by the four fields; not every field uses each yet"
            )]
            pub fn helper(mut self, text: impl Into<String>) -> Self {
                self.core.helper = Some(text.into());
                self
            }
            /// An error below the field with a danger border and halo. Set it on blur or submit,
            /// never on each keystroke.
            #[allow(
                dead_code,
                reason = "shared by the four fields; not every field uses each yet"
            )]
            pub fn error(mut self, text: Option<&str>) -> Self {
                self.core.error = text.map(str::to_owned);
                self
            }
            /// Text that can be selected and copied but not changed (`text_dim`, no caret).
            #[allow(
                dead_code,
                reason = "shared by the four fields; not every field uses each yet"
            )]
            pub fn read_only(mut self, read_only: bool) -> Self {
                self.core.read_only = read_only;
                self
            }
            #[allow(
                dead_code,
                reason = "shared by the four fields; not every field uses each yet"
            )]
            pub fn enabled(mut self, enabled: bool) -> Self {
                self.core.enabled = enabled;
                self
            }
            #[allow(
                dead_code,
                reason = "shared by the four fields; not every field uses each yet"
            )]
            pub fn tooltip(mut self, text: impl Into<String>) -> Self {
                self.core.tooltip = Some(text.into());
                self
            }
            #[allow(
                dead_code,
                reason = "shared by the four fields; not every field uses each yet"
            )]
            pub fn disabled_reason(mut self, text: impl Into<String>) -> Self {
                self.core.disabled_reason = Some(text.into());
                self
            }
            /// Field width; the available width by default.
            #[allow(
                dead_code,
                reason = "shared by the four fields; not every field uses each yet"
            )]
            pub fn width(mut self, width: f32) -> Self {
                self.core.width = Some(width);
                self
            }
            #[allow(
                dead_code,
                reason = "shared by the four fields; not every field uses each yet"
            )]
            pub fn id_salt(mut self, salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
                self.core.id_salt = Some(Id::new(salt));
                self
            }
        }
    };
}

/// A one-line text input: `TextField::new(&mut name).label("Name").show(ui)`.
#[derive(Debug)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct TextField<'t> {
    core: Core<'t>,
}

impl<'t> TextField<'t> {
    pub fn new(text: &'t mut String) -> Self {
        Self {
            core: Core::new(text, Kind::Line),
        }
    }
    /// A leading Phosphor glyph in `text_faint`.
    pub fn icon(mut self, glyph: &str) -> Self {
        self.core.icon = Some(glyph.to_owned());
        self
    }
    /// Hides the text (secrets), with a button that shows it.
    pub fn password(mut self, password: bool) -> Self {
        self.core.password = password;
        self
    }
    /// Busy (checking the value): a spinner in the trailing slot after `LOADING_DELAY`.
    pub fn loading(mut self, loading: bool) -> Self {
        self.core.loading = loading;
        self
    }
    /// `Sm` for compact fields; `Md` by default.
    pub fn size(mut self, size: Size) -> Self {
        self.core.size = size;
        self
    }
    /// A right-aligned character count: `warning` past 50, `danger` past 72 (commit summary).
    pub fn counter(mut self, counter: bool) -> Self {
        self.core.counter = counter;
        self
    }
    pub(crate) fn show_forced(mut self, ui: &mut Ui, force: ForceState) -> Response {
        self.core.force = Some(force);
        self.show(ui)
    }
    pub fn show(self, ui: &mut Ui) -> Response {
        self.core.show(ui).0
    }
}

common_setters!(TextField);

impl Widget for TextField<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

/// A filter or search input: magnifier, the shortcut's keycap while empty, a clear button once
/// filled; Esc clears, then lets go of focus.
#[derive(Debug)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct SearchField<'t> {
    core: Core<'t>,
}

impl<'t> SearchField<'t> {
    pub fn new(text: &'t mut String) -> Self {
        Self {
            core: Core::new(text, Kind::Search),
        }
    }
    /// Busy (results are loading): a spinner in the trailing slot after `LOADING_DELAY`.
    pub fn loading(mut self, loading: bool) -> Self {
        self.core.loading = loading;
        self
    }
    /// The command that focuses this field; its chord shows as a keycap while the field is empty.
    pub fn shortcut(mut self, command: Command) -> Self {
        self.core.shortcut = Some(command);
        self
    }
    pub fn size(mut self, size: Size) -> Self {
        self.core.size = size;
        self
    }
    pub(crate) fn show_forced(mut self, ui: &mut Ui, force: ForceState) -> Response {
        self.core.force = Some(force);
        self.show(ui)
    }
    pub fn show(self, ui: &mut Ui) -> Response {
        self.core.show(ui).0
    }
}

common_setters!(SearchField);

impl Widget for SearchField<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

/// Multi-line text (commit description, comments) that grows with its content between a minimum
/// and maximum number of rows, then scrolls. Ctrl+Enter submits.
#[derive(Debug)]
#[must_use = "call .show(ui)"]
pub struct TextArea<'t> {
    core: Core<'t>,
}

impl<'t> TextArea<'t> {
    pub fn new(text: &'t mut String) -> Self {
        Self {
            core: Core::new(text, Kind::Area { rows: 3..=12 }),
        }
    }
    /// The rows it shows when empty, and the most it grows to (3 to 12 by default).
    pub fn rows(mut self, rows: RangeInclusive<usize>) -> Self {
        self.core.kind = Kind::Area { rows };
        self
    }
    /// A dotted guide at this column (72 for commit descriptions).
    pub fn guide(mut self, column: usize) -> Self {
        self.core.guide = Some(column);
        self
    }
    pub(crate) fn show_forced(mut self, ui: &mut Ui, force: ForceState) -> KitResponse<bool> {
        self.core.force = Some(force);
        self.show(ui)
    }
    /// The value is whether Ctrl+Enter submitted the text this frame.
    pub fn show(self, ui: &mut Ui) -> KitResponse<bool> {
        let (response, value) = self.core.show(ui);
        KitResponse { response, value }
    }
}

common_setters!(TextArea);

/// A [`TextArea`] for code: monospace, Tab inserts a tab, optional line numbers (the conflict
/// hand-edit box).
#[derive(Debug)]
#[must_use = "call .show(ui)"]
pub struct CodeArea<'t> {
    core: Core<'t>,
}

impl<'t> CodeArea<'t> {
    pub fn new(text: &'t mut String) -> Self {
        Self {
            core: Core::new(
                text,
                Kind::Code {
                    rows: 3..=16,
                    line_numbers: false,
                },
            ),
        }
    }
    pub fn rows(mut self, rows: RangeInclusive<usize>) -> Self {
        if let Kind::Code { line_numbers, .. } = self.core.kind {
            self.core.kind = Kind::Code { rows, line_numbers };
        }
        self
    }
    pub fn line_numbers(mut self, on: bool) -> Self {
        if let Kind::Code { rows, .. } = &self.core.kind {
            self.core.kind = Kind::Code {
                rows: rows.clone(),
                line_numbers: on,
            };
        }
        self
    }
    pub(crate) fn show_forced(mut self, ui: &mut Ui, force: ForceState) -> KitResponse<bool> {
        self.core.force = Some(force);
        self.show(ui)
    }
    /// The value is whether Ctrl+Enter submitted the text this frame.
    pub fn show(self, ui: &mut Ui) -> KitResponse<bool> {
        let (response, value) = self.core.show(ui);
        KitResponse { response, value }
    }
}

common_setters!(CodeArea);

// ------------------------------------------------------------------------------------- gallery

/// The gallery's fields: every text field state, the search field empty and filled, the text
/// area with its counter and guide, and the code area.
pub(crate) fn gallery(ui: &mut Ui) {
    ui.spacing_mut().item_spacing.y = theme::MD;
    let cols = |ui: &mut Ui, f: &mut dyn FnMut(&mut Ui, usize)| {
        ui.columns(2, |cols| {
            for (i, col) in cols.iter_mut().enumerate() {
                col.spacing_mut().item_spacing.y = theme::MD;
                f(col, i);
            }
        });
    };
    gallery_caption(ui, "Text field states");
    cols(ui, &mut |ui, i| {
        if i == 0 {
            TextField::new(&mut String::new())
                .label("Branch name")
                .placeholder("feature/…")
                .id_salt("g-f0")
                .show(ui);
            TextField::new(&mut "feature/search".to_owned())
                .label("Hover")
                .id_salt("g-f1")
                .show_forced(ui, ForceState::HOVER);
            TextField::new(&mut "feature/search".to_owned())
                .label("Focused")
                .id_salt("g-f2")
                .show_forced(ui, ForceState::FOCUS);
            TextField::new(&mut "feature/dark mode".to_owned())
                .label("Error")
                .error(Some("Branch names cannot contain spaces."))
                .id_salt("g-f3")
                .show(ui);
        } else {
            remembered(ui, "field-live", "Ada Lovelace".to_owned(), |ui, v| {
                TextField::new(v)
                    .label("Live, with an icon and a helper")
                    .icon(icon::USER)
                    .helper("Used for new commits in this repository.")
                    .id_salt("g-f4")
                    .show(ui);
            });
            TextField::new(&mut "ada@example.com".to_owned())
                .label("Disabled")
                .enabled(false)
                .disabled_reason("Set by your global Git configuration")
                .id_salt("g-f5")
                .show(ui);
            TextField::new(&mut "https://github.com/necturalabs/canopy.git".to_owned())
                .label("Read-only")
                .read_only(true)
                .tooltip("Copy it with Ctrl+C")
                .id_salt("g-f6")
                .show(ui);
            TextField::new(&mut "hunter2".to_owned())
                .label("Secret, small, checking")
                .loading(true)
                .password(true)
                .size(Size::Sm)
                .id_salt("g-f7")
                .show(ui);
        }
    });
    gallery_caption(
        ui,
        "Search: empty with keycap, filled with clear (live), focused",
    );
    cols(ui, &mut |ui, i| {
        if i == 0 {
            SearchField::new(&mut String::new())
                .placeholder("Filter commits, authors, SHAs…")
                .shortcut(Command::Filter)
                .name("Filter commits")
                .id_salt("g-s0")
                .show(ui);
            SearchField::new(&mut String::new())
                .placeholder("Search…")
                .shortcut(Command::Filter)
                .id_salt("g-s1")
                .show_forced(ui, ForceState::FOCUS);
        } else {
            remembered(ui, "search-live", "dark mode".to_owned(), |ui, v| {
                SearchField::new(v)
                    .placeholder("Filter branches…")
                    .shortcut(Command::Filter)
                    .id_salt("g-s2")
                    .show(ui);
            });
            SearchField::new(&mut "auth".to_owned())
                .placeholder("Filter branches…")
                .size(Size::Sm)
                .loading(true)
                .id_salt("g-s3")
                .show(ui);
        }
    });
    gallery_caption(
        ui,
        "Summary counter at 50 and 73 characters; description with the guide",
    );
    cols(ui, &mut |ui, i| {
        let text = if i == 0 { 50 } else { 73 };
        TextField::new(&mut "x".repeat(text))
            .counter(true)
            .name("Summary")
            .id_salt(("g-sum", i))
            .show(ui);
    });
    remembered(
        ui,
        "area-live",
        "Explain why, not what: the diff already says what.".to_owned(),
        |ui, v| {
            TextArea::new(v)
                .rows(3..=12)
                .guide(72)
                .placeholder("Description")
                .name("Description")
                .id_salt("g-area")
                .show(ui);
        },
    );
    gallery_caption(ui, "Code area with line numbers (live), focused");
    cols(ui, &mut |ui, i| {
        if i == 0 {
            remembered(
                ui,
                "code-live",
                "Self {\n    attempts: 5,\n    backoff_ms: 500,\n}".to_owned(),
                |ui, v| {
                    CodeArea::new(v)
                        .rows(3..=8)
                        .line_numbers(true)
                        .name("Resolved text")
                        .id_salt("g-code")
                        .show(ui);
                },
            );
        } else {
            CodeArea::new(&mut "fn main() {}".to_owned())
                .rows(3..=3)
                .name("Focused code")
                .id_salt("g-code-f")
                .show_forced(ui, ForceState::FOCUS);
            TextArea::new(&mut "Looks good to me.".to_owned())
                .rows(2..=2)
                .label("Comment, hover")
                .id_salt("g-area-h")
                .show_forced(ui, ForceState::HOVER);
            TextArea::new(&mut String::new())
                .rows(2..=2)
                .placeholder("Leave a comment")
                .enabled(false)
                .disabled_reason("Comments need a hosting account")
                .id_salt("g-area-d")
                .show(ui);
        }
    });
}

#[cfg(test)]
mod tests {
    use egui::accesskit::Role;

    use super::super::testkit::{self, find, frame, key, key_with, spoken};
    use super::*;

    fn type_text(text: &str) -> Vec<egui::Event> {
        vec![egui::Event::Text(text.to_owned())]
    }

    #[test]
    fn fields_follow_the_metrics_in_every_density_and_style() {
        for density in theme::Density::ALL {
            for style in theme::Style::ALL {
                let ctx = testkit::ctx_with(theme::ThemeId::Daylight, style, density);
                let m = theme::metrics();
                let mut heights = Vec::new();
                frame(&ctx, Vec::new(), |ui| {
                    heights.push(TextField::new(&mut String::new()).show(ui).rect.height());
                    heights.push(SearchField::new(&mut String::new()).show(ui).rect.height());
                });
                assert_eq!(heights, vec![m.control, m.control], "{density:?} {style:?}");
            }
        }
    }

    #[test]
    fn typing_edits_and_disabled_or_read_only_does_not() {
        let ctx = testkit::ctx();
        for (enabled, read_only, expect) in
            [(true, false, "ab"), (false, false, "a"), (true, true, "a")]
        {
            let mut text = "a".to_owned();
            let show = |ui: &mut Ui, text: &mut String| {
                TextField::new(text)
                    .enabled(enabled)
                    .read_only(read_only)
                    .id_salt("t")
                    .show(ui)
                    .id
            };
            let mut id = Id::NULL;
            frame(&ctx, Vec::new(), |ui| id = show(ui, &mut text));
            ctx.memory_mut(|m| m.request_focus(id));
            frame(&ctx, Vec::new(), |ui| id = show(ui, &mut text));
            frame(&ctx, type_text("b"), |ui| id = show(ui, &mut text));
            assert_eq!(text, expect, "enabled {enabled}, read-only {read_only}");
        }
    }

    #[test]
    fn escape_clears_a_search_then_lets_go() {
        let ctx = testkit::ctx();
        let mut text = "auth".to_owned();
        let show = |ui: &mut Ui, text: &mut String| SearchField::new(text).id_salt("s").show(ui).id;
        let mut id = Id::NULL;
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut text));
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut text));
        frame(&ctx, key(Key::Escape), |ui| id = show(ui, &mut text));
        assert_eq!(text, "");
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut text));
        assert!(ctx.memory(|m| m.has_focus(id)), "the first Esc keeps focus");
        frame(&ctx, key(Key::Escape), |ui| id = show(ui, &mut text));
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut text));
        assert!(!ctx.memory(|m| m.has_focus(id)), "the second Esc lets go");
    }

    #[test]
    fn ctrl_enter_submits_a_text_area() {
        let ctx = testkit::ctx();
        let mut text = "body".to_owned();
        let show = |ui: &mut Ui, text: &mut String| {
            let r = TextArea::new(text).id_salt("a").show(ui);
            (r.response.id, r.value)
        };
        let (mut id, mut submitted) = (Id::NULL, false);
        frame(&ctx, Vec::new(), |ui| (id, _) = show(ui, &mut text));
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, Vec::new(), |ui| (id, _) = show(ui, &mut text));
        frame(&ctx, key_with(Key::Enter, Modifiers::COMMAND), |ui| {
            (id, submitted) = show(ui, &mut text);
        });
        assert!(submitted);
        assert_eq!(text, "body", "Ctrl+Enter adds no newline");
    }

    #[test]
    fn a_text_area_grows_with_its_content_within_its_rows() {
        let ctx = testkit::ctx();
        let height = |text: &str| {
            let mut text = text.to_owned();
            let mut h = 0.0;
            for _ in 0..3 {
                frame(&ctx, Vec::new(), |ui| {
                    h = TextArea::new(&mut text)
                        .rows(2..=4)
                        .id_salt("grow")
                        .show(ui)
                        .response
                        .rect
                        .height();
                });
            }
            h
        };
        let two = height("");
        let four = height("1\n2\n3\n4");
        let many = height("1\n2\n3\n4\n5\n6\n7\n8");
        assert!(four > two, "{four} > {two}");
        assert!(
            (many - four).abs() < 1.0,
            "stops growing at the last row: {many} vs {four}"
        );
    }

    #[test]
    fn fields_are_named_and_errors_are_announced() {
        let ctx = testkit::ctx();
        let mut text = "feature/dark mode".to_owned();
        let nodes = spoken(&ctx, |ui| {
            TextField::new(&mut text)
                .label("Branch name")
                .error(Some("Branch names cannot contain spaces."))
                .show(ui);
        });
        let field = nodes
            .iter()
            .find(|n| n.role == Role::TextInput)
            .expect("a text input");
        assert_eq!(field.label.as_deref(), Some("Branch name"));
        let mut code = String::new();
        let nodes = spoken(&ctx, |ui| {
            CodeArea::new(&mut code).name("Resolved text").show(ui);
        });
        assert_eq!(
            find(&nodes, "Resolved text").map(|n| n.role),
            Some(Role::MultilineTextInput)
        );
    }

    #[test]
    fn unsalted_fields_keep_their_own_state() {
        let ctx = testkit::ctx();
        let (mut a, mut b) = (String::new(), String::new());
        let show = |ui: &mut Ui, a: &mut String, b: &mut String| {
            let first = TextField::new(a).show(ui).id;
            let second = TextField::new(b).show(ui).id;
            (first, second)
        };
        let mut ids = (Id::NULL, Id::NULL);
        frame(&ctx, Vec::new(), |ui| ids = show(ui, &mut a, &mut b));
        assert_ne!(ids.0, ids.1);
        ctx.memory_mut(|m| m.request_focus(ids.1));
        frame(&ctx, Vec::new(), |ui| ids = show(ui, &mut a, &mut b));
        frame(&ctx, type_text("x"), |ui| ids = show(ui, &mut a, &mut b));
        assert_eq!((a.as_str(), b.as_str()), ("", "x"));
    }

    #[test]
    fn a_secret_can_be_shown_and_hidden_again() {
        let ctx = testkit::ctx();
        let mut text = "hunter2".to_owned();
        let show = |ui: &mut Ui, text: &mut String| {
            TextField::new(text)
                .password(true)
                .width(240.0)
                .id_salt("p")
                .show(ui)
                .rect
        };
        let mut rect = Rect::NOTHING;
        frame(&ctx, Vec::new(), |ui| rect = show(ui, &mut text));
        let nodes = spoken(&ctx, |ui| {
            show(ui, &mut text);
        });
        assert!(
            find(&nodes, "Show secret").is_some(),
            "the toggle offers to show it"
        );
        let m = theme::metrics();
        let at = pos2(
            rect.right() - m.field_pad_x - Size::Xs.height() / 2.0,
            rect.center().y,
        );
        testkit::click(&ctx, at, |ui| {
            show(ui, &mut text);
        });
        let nodes = spoken(&ctx, |ui| {
            show(ui, &mut text);
        });
        assert!(
            find(&nodes, "Hide secret").is_some(),
            "a click shows the secret"
        );
        assert_eq!(text, "hunter2", "the toggle never edits the text");
    }

    #[test]
    fn focus_halo_is_instant_with_reduced_motion() {
        let ctx = testkit::ctx();
        let p = theme::palette(&ctx);
        let halo = Palette::tint(p.focus, FOCUS_HALO_ALPHA);
        let mut text = String::new();
        let strokes = |ctx: &egui::Context, text: &mut String| {
            let mut id = Id::NULL;
            let shapes = frame(ctx, Vec::new(), |ui| {
                id = TextField::new(text).id_salt("h").show(ui).id;
            });
            let painted = shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    egui::Shape::Rect(r) => Some((r.stroke.color, r.stroke.width)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            (id, painted)
        };
        let (id, _) = strokes(&ctx, &mut text);
        ctx.memory_mut(|m| m.request_focus(id));
        let (_, painted) = strokes(&ctx, &mut text);
        assert!(
            painted.contains(&(halo, HALO)),
            "the focused field shows its whole halo on the first frame: {painted:?}"
        );
    }
}
