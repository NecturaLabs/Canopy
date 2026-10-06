//! [`Select`]: a choice of one value from a list, replacing `egui::ComboBox`. Design spec 4.3.
//!
//! The trigger is drawn as a text field (the same well, edge, hover, focus halo, label, helper and
//! error as [`super::field`]), so a form's selects and fields read as one set, with the value and
//! a caret that turns over while the list is open; the list is a floating menu surface under the
//! trigger (above it when there is no room). Focus stays on the trigger: Up and Down open the list
//! and move the highlight, Home and End jump, Enter or Space choose, Esc closes and keeps focus,
//! and typing jumps to the first option that starts with what was typed (the buffer clears after
//! a second). Opening with the pointer animates in and closing fades out; from the keyboard both
//! are instant.

use egui::accesskit::{Invalid, Role};
use egui::epaint::TextShape;
use egui::{
    Align, Align2, Color32, CornerRadius, Event, FontFamily, FontId, Id, Key, Layout, Modifiers,
    PointerButton, Popup, PopupCloseBehavior, RectAlign, Response, Sense, Ui, UiBuilder, Widget,
    WidgetInfo, WidgetType, pos2, vec2,
};
use egui_phosphor::regular as icon;

use super::button::tooltip_text;
use super::overlay::{self, Entrance, ExitMotion, TooltipExt};
use super::toggle::{component_id, gallery_caption, gallery_row, remembered};
use super::{ForceState, Interact, Size, anim, field, glass, paint};
use crate::ui::motion;
use crate::ui::theme::{self, Elevation, Fill};

/// The caret turns over in 0.25 s.
const CARET_TURN: f32 = 0.25;
/// The list: 6 below the trigger, 2 wider on each side, at most 8.5 rows tall.
const LIST_GAP: f32 = 6.0;
const LIST_OVERHANG: f32 = 2.0;
const LIST_ROWS: f32 = 8.5;
const LIST_MIN_WIDTH: f32 = 200.0;
/// Menu items: inner padding and the gap before the check.
const ITEM_PAD: f32 = theme::SM;
/// Type-ahead: the buffer clears this long after the last key.
const TYPE_AHEAD_RESET: f64 = 1.0;

/// What a select remembers between frames while its list is open.
#[derive(Debug, Clone, Default)]
struct ListState {
    highlight: Option<usize>,
    /// The last highlight came from the keyboard (scroll it into view).
    by_keyboard: bool,
    /// The list was opened from the keyboard: it shows and closes at once.
    opened_by_keyboard: bool,
    typed: String,
    typed_at: f64,
}

/// One value from a list: `Select::new(&mut minutes, &[(0, "Off"), (5, "Every 5 minutes")])`.
#[derive(Debug)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct Select<'a, T: PartialEq + Clone> {
    value: &'a mut T,
    options: &'a [(T, &'a str)],
    label: Option<String>,
    name: Option<String>,
    placeholder: String,
    helper: Option<String>,
    error: Option<String>,
    width: Option<f32>,
    fit: bool,
    size: Size,
    enabled: bool,
    loading: bool,
    tooltip: Option<String>,
    disabled_reason: Option<String>,
    id_salt: Option<Id>,
    id: Option<Id>,
    force: Option<ForceState>,
}

impl<'a, T: PartialEq + Clone> Select<'a, T> {
    pub fn new(value: &'a mut T, options: &'a [(T, &'a str)]) -> Self {
        Self {
            value,
            options,
            label: None,
            name: None,
            placeholder: String::new(),
            helper: None,
            error: None,
            width: None,
            fit: false,
            size: Size::Md,
            enabled: true,
            loading: false,
            tooltip: None,
            disabled_reason: None,
            id_salt: None,
            id: None,
            force: None,
        }
    }
    /// A label above the trigger (also its accessible name).
    pub fn label(mut self, text: impl Into<String>) -> Self {
        self.label = Some(text.into());
        self
    }
    /// The accessible name when no label is shown.
    pub fn name(mut self, text: impl Into<String>) -> Self {
        self.name = Some(text.into());
        self
    }
    /// Shown in `text_faint` when the value is none of the options.
    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = text.into();
        self
    }
    /// A hint below the trigger (hidden while there is an error), as a text field shows one.
    pub fn helper(mut self, text: impl Into<String>) -> Self {
        self.helper = Some(text.into());
        self
    }
    /// An error below the trigger with a danger edge and halo, as a text field shows one.
    pub fn error(mut self, text: Option<&str>) -> Self {
        self.error = text.map(str::to_owned);
        self
    }
    /// Trigger width; the available width by default.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }
    /// The trigger is as wide as its longest option (or placeholder) needs, no wider ([`fit_width`]).
    pub fn fit_to_options(mut self) -> Self {
        self.fit = true;
        self
    }
    pub fn size(mut self, size: Size) -> Self {
        self.size = size;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    /// The options are still loading: a spinner replaces the caret and the list stays shut.
    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }
    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some(text.into());
        self
    }
    pub fn disabled_reason(mut self, text: impl Into<String>) -> Self {
        self.disabled_reason = Some(text.into());
        self
    }
    pub fn id_salt(mut self, salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        self.id_salt = Some(Id::new(salt));
        self
    }
    /// The select's own id, set by the caller (not derived from the parent `Ui`), so code
    /// elsewhere can focus its trigger ([`trigger_id`]) or open its list.
    pub fn id(mut self, id: Id) -> Self {
        self.id = Some(id);
        self
    }
    /// `open` shows the list (statically, at its final position); `hover` with `open` highlights
    /// the first option that is not chosen.
    pub(crate) fn show_forced(mut self, ui: &mut Ui, force: ForceState) -> Response {
        self.force = Some(force);
        self.show(ui)
    }

    /// Shows the trigger (and the list while open). The response is the trigger's and reports
    /// `changed` when a new value was chosen.
    pub fn show(mut self, ui: &mut Ui) -> Response {
        let width = match self.width {
            Some(w) => w,
            None if self.fit => fit_width(
                ui,
                self.options
                    .iter()
                    .map(|(_, l)| *l)
                    .chain([self.placeholder.as_str()]),
            ),
            None => ui.available_width(),
        };
        self.width = Some(width);
        let id = self
            .id
            .unwrap_or_else(|| component_id(ui, "kit-select", self.id_salt));
        // Label, trigger and message stack like a text field's, in a slot of their own size that
        // the parent's layout places (at the right of a right-to-left row, centred in a centred
        // one). The stack's height is last frame's; a change is measured and redrawn at once.
        let height_id = id.with("kit-select-height");
        let height = ui
            .data(|d| d.get_temp::<f32>(height_id))
            .unwrap_or_else(|| self.size.height());
        let shown =
            ui.allocate_ui_with_layout(vec2(width, height), Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                self.show_inner(ui, id)
            });
        let used = shown.response.rect.height();
        if (used - height).abs() > 0.5 {
            ui.data_mut(|d| d.insert_temp(height_id, used));
            ui.ctx().request_discard("select measured");
        }
        shown.inner
    }

    fn show_inner(self, ui: &mut Ui, id: Id) -> Response {
        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let style = theme::style();
        let s = p.surfaces(style);
        let m = theme::metrics();
        let enabled = self.enabled && ui.is_enabled();
        let popup_id = id.with("kit-select-list");
        let state_id = id.with("kit-select-state");
        let width = self.width.unwrap_or_else(|| ui.available_width());
        let chosen = self.options.iter().position(|(v, _)| v == self.value);

        // Labels and text paint at full strength even in a disabled `Ui` (in disabled tokens).
        let ink = super::content_painter(ui);
        if let Some(text) = &self.label {
            let color = if enabled { p.text_dim } else { p.disabled_text };
            let g = ink.layout_no_wrap(
                text.clone(),
                FontId::new(m.meta_size, theme::medium()),
                color,
            );
            let (rect, _) = ui.allocate_exact_size(vec2(width, g.size().y), Sense::hover());
            ink.galley(rect.min, g, color);
            ui.add_space(m.field_label_gap);
        }

        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let size = vec2(width, self.size.height());
        let (rect, mut trigger) = if self.id.is_some() {
            let rect = ui.allocate_space(size).1;
            (rect, ui.interact(rect, trigger_id(id), sense))
        } else {
            ui.allocate_exact_size(size, sense)
        };
        let force = self.force;
        // A forced list shows only while its trigger is on screen (egui would push it into view).
        let forced_open = force.is_some_and(|f| f.open) && ui.is_rect_visible(rect);
        let can_open = enabled && !self.loading;

        // Opening, closing and the keys, before painting so this frame shows the result.
        let mut open = forced_open || Popup::is_id_open(&ctx, popup_id);
        let mut st: ListState = ctx.data(|d| d.get_temp(state_id)).unwrap_or_default();
        let now = ctx.input(|i| i.time);
        let pointer_click = trigger.clicked_by(PointerButton::Primary);
        let key_click = trigger.clicked() && !pointer_click;
        let mut set_open: Option<bool> = None;
        let mut picked: Option<usize> = None;
        if can_open && !forced_open {
            if trigger.has_focus() {
                // Up and Down drive the list, and while it is open Esc closes it instead of
                // taking focus away from the trigger.
                ui.memory_mut(|mem| {
                    mem.set_focus_lock_filter(
                        trigger.id,
                        egui::EventFilter {
                            vertical_arrows: true,
                            escape: open,
                            ..Default::default()
                        },
                    );
                });
            }
            if pointer_click {
                set_open = Some(!open);
                st = ListState {
                    highlight: None,
                    by_keyboard: false,
                    ..ListState::default()
                };
            } else if trigger.has_focus() {
                let (down, up, home, end) = ui.input_mut(|i| {
                    (
                        i.consume_key(Modifiers::NONE, Key::ArrowDown),
                        i.consume_key(Modifiers::NONE, Key::ArrowUp),
                        open && i.consume_key(Modifiers::NONE, Key::Home),
                        open && i.consume_key(Modifiers::NONE, Key::End),
                    )
                });
                let last = self.options.len().saturating_sub(1);
                if !open && (down || up || key_click) {
                    set_open = Some(true);
                    st = ListState {
                        highlight: chosen.or(Some(0)),
                        by_keyboard: true,
                        opened_by_keyboard: true,
                        ..ListState::default()
                    };
                } else if open {
                    let at = st.highlight.or(chosen);
                    if down {
                        st.highlight = Some(at.map_or(0, |i| (i + 1).min(last)));
                    } else if up {
                        st.highlight = Some(at.map_or(last, |i| i.saturating_sub(1)));
                    } else if home {
                        st.highlight = Some(0);
                    } else if end {
                        st.highlight = Some(last);
                    } else if key_click {
                        picked = st.highlight.or(chosen);
                        set_open = Some(false);
                    }
                    if ui.input(|i| i.key_pressed(Key::Escape)) {
                        set_open = Some(false);
                        trigger.request_focus();
                    }
                    let typed: String = ui.input_mut(|i| {
                        let mut out = String::new();
                        i.events.retain(|e| match e {
                            Event::Text(t) => {
                                out.push_str(t);
                                false
                            }
                            _ => true,
                        });
                        out
                    });
                    if !typed.trim().is_empty() {
                        if now - st.typed_at > TYPE_AHEAD_RESET {
                            st.typed.clear();
                        }
                        st.typed.push_str(&typed.to_lowercase());
                        st.typed_at = now;
                        if let Some(i) = self
                            .options
                            .iter()
                            .position(|(_, l)| l.to_lowercase().starts_with(&st.typed))
                        {
                            st.highlight = Some(i);
                        }
                    }
                }
            }
        }
        if let Some(o) = set_open {
            if o {
                Popup::open_id(&ctx, popup_id);
            } else {
                Popup::close_id(&ctx, popup_id);
            }
            open = o;
        }
        if !can_open && !forced_open && open {
            Popup::close_id(&ctx, popup_id);
            open = false;
        }

        // The trigger.
        let ist = Interact::of(&ctx, &trigger, enabled, force.filter(|f| !f.open));
        let open_t = match force {
            Some(f) if f.open => f.progress.unwrap_or(1.0),
            Some(_) => 0.0,
            None => anim::ease(&ctx, id.with("kit-select-caret"), open, CARET_TURN),
        };
        let error = self.error.as_deref().filter(|e| !e.is_empty());
        if ui.is_rect_visible(rect) {
            // The trigger is a field (spec 4.2's well, edge and halo), so a form's selects and
            // text fields read as one set; the caret says it opens a list.
            let painter = if enabled {
                ui.painter().clone()
            } else {
                super::content_painter(ui)
            };
            let radius = CornerRadius::same(m.radius);
            let look = field::look(p, style);
            let ring_t = open_t.max(ist.focus);
            let (fill, edge) = if enabled {
                let fill = look
                    .wash
                    .lerp_to_gamma(look.hover_wash, ist.hover.max(ist.press) * (1.0 - open_t));
                let edge = if error.is_some() {
                    (p.danger_base, p.danger_base)
                } else {
                    let to = |c: Color32| {
                        c.lerp_to_gamma(p.control_border, ist.hover)
                            .lerp_to_gamma(p.focus, ring_t)
                    };
                    (to(look.side), to(look.base))
                };
                (fill, edge)
            } else {
                (s.disabled_fill, (s.disabled_edge, s.disabled_edge))
            };
            painter.rect_filled(rect, radius, fill);
            let material = p.material(style, Elevation::Field);
            if enabled && material.recess > 0 {
                paint::recess(&painter, rect, radius, p.shadow, material.recess);
            }
            let halo = if !enabled {
                None
            } else if error.is_some() {
                Some((p.danger_base, field::ERROR_HALO_ALPHA, 1.0))
            } else {
                (ring_t > 0.0).then_some((p.focus, field::FOCUS_HALO_ALPHA, ring_t))
            };
            field::paint_box(&painter, rect, radius, edge, halo);
            // The caret brightens while the pointer is on the trigger or the list is open.
            let (faint, placeholder, text_color) = if enabled {
                (
                    p.text_faint
                        .lerp_to_gamma(p.text_dim, ist.hover.max(open_t)),
                    p.text_faint,
                    p.text,
                )
            } else {
                (p.disabled_text, p.disabled_text, p.disabled_text)
            };
            let caret_x = rect.right() - theme::SM - m.icon_md / 2.0;
            if self.loading {
                paint::spinner(
                    &painter,
                    pos2(caret_x, rect.center().y),
                    m.icon_md * 0.33,
                    1.6,
                    faint,
                );
            } else {
                let g = painter.layout_no_wrap(
                    icon::CARET_DOWN.to_owned(),
                    FontId::proportional(m.icon_md),
                    faint,
                );
                let pos = pos2(caret_x, rect.center().y) - g.size() / 2.0;
                let angle = std::f32::consts::PI * open_t;
                painter.add(
                    TextShape::new(pos, g, faint)
                        .with_angle_and_anchor(angle, Align2::CENTER_CENTER),
                );
            }
            let (text, color) = match chosen {
                Some(i) => (self.options[i].1.to_owned(), text_color),
                None => (self.placeholder.clone(), placeholder),
            };
            let max = (rect.width() - m.field_pad_x - caret_room(m)).max(0.0);
            let mut job = egui::text::LayoutJob::simple_singleline(
                text,
                FontId::new(m.body_size, FontFamily::Proportional),
                color,
            );
            job.wrap = egui::text::TextWrapping::truncate_at_width(max);
            let g = painter.layout_job(job);
            painter.galley(
                pos2(
                    rect.left() + m.field_pad_x,
                    rect.center().y - g.size().y / 2.0,
                ),
                g,
                color,
            );
        }

        // The list.
        if open {
            if forced_open && force.is_some_and(|f| f.hover) {
                st.highlight = (0..self.options.len()).find(|i| Some(*i) != chosen);
            }
            // A pointer-opened list grows in from the trigger and fades out when it closes (spec
            // 4.12 motion); a keyboard-opened one comes and goes at once, as does every list with
            // reduced motion.
            let keyboard = st.opened_by_keyboard;
            let progress = forced_open.then(|| force.and_then(|f| f.progress).unwrap_or(1.0));
            let anchor = rect.expand2(vec2(LIST_OVERHANG, 0.0));
            let item_h = m.menu_item_height;
            let rows = crate::ui::count_f32(self.options.len());
            let list_w = anchor
                .width()
                .max(LIST_MIN_WIDTH.min(anchor.width() + 120.0));
            let visible = rows.min(LIST_ROWS);
            let list_h = visible * item_h + 2.0 * m.menu_pad;
            let popup = Popup::new(popup_id, ctx.clone(), anchor, ui.layer_id())
                .kind(egui::PopupKind::Popup)
                .align(RectAlign::BOTTOM_START)
                .gap(LIST_GAP)
                .width(list_w)
                .frame(egui::Frame::NONE)
                .close_behavior(PopupCloseBehavior::CloseOnClickOutside);
            // Egui's popup memory keeps it open and closes it on Esc or a click outside.
            let popup = if forced_open {
                popup.open(true)
            } else {
                popup.open_memory(None)
            };
            let options = self.options;
            let shown = popup.show(|ui| {
                let layer = ui.layer_id();
                let e = Entrance::of(
                    ui.ctx(),
                    popup_id,
                    motion::BASE,
                    motion::SLOW,
                    keyboard,
                    progress,
                );
                ui.multiply_opacity(e.fade);
                let start = overlay::next_shape(ui.ctx(), layer);
                let (list, _) = ui.allocate_exact_size(vec2(list_w, list_h), Sense::hover());
                let radius = CornerRadius::same(m.radius_lg);
                let painter = ui.painter();
                let fill = glass::layer_fill(painter, list, radius);
                paint::surface(
                    painter,
                    list,
                    radius,
                    p,
                    style,
                    Elevation::Floating,
                    Fill::flat(fill),
                    Some(p.floating_ring(s)),
                );
                let inner = list.shrink(m.menu_pad);
                let mut child = ui.new_child(
                    UiBuilder::new()
                        .max_rect(inner)
                        .accessibility_parent(popup_id.with("listbox")),
                );
                let mut chose = None;
                let scroll_to = st.highlight.filter(|_| st.by_keyboard);
                let out = egui::ScrollArea::vertical()
                    .id_salt(popup_id.with("scroll"))
                    .max_height(inner.height())
                    .auto_shrink([false, true])
                    .show(&mut child, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for (i, (_, label)) in options.iter().enumerate() {
                            let (row, r) =
                                ui.allocate_exact_size(vec2(inner.width(), item_h), Sense::click());
                            if r.hovered() && ui.input(|i| i.pointer.is_moving()) {
                                st.highlight = Some(i);
                                st.by_keyboard = false;
                            }
                            if scroll_to == Some(i) {
                                ui.scroll_to_rect(row, None);
                            }
                            let on = chosen == Some(i);
                            let radius = CornerRadius::same(m.radius_lg.saturating_sub(5).max(3));
                            if st.highlight == Some(i) {
                                ui.painter().rect_filled(row, radius, p.hover_strong());
                            }
                            let font = if on {
                                FontId::new(m.body_size, theme::medium())
                            } else {
                                FontId::new(m.body_size, FontFamily::Proportional)
                            };
                            ui.painter().text(
                                pos2(row.left() + ITEM_PAD, row.center().y),
                                Align2::LEFT_CENTER,
                                *label,
                                font,
                                p.text,
                            );
                            if on {
                                ui.painter().text(
                                    pos2(row.right() - ITEM_PAD, row.center().y),
                                    Align2::RIGHT_CENTER,
                                    icon::CHECK,
                                    FontId::proportional(m.icon_sm),
                                    p.accent,
                                );
                            }
                            if r.clicked() {
                                chose = Some(i);
                            }
                            r.widget_info(|| {
                                WidgetInfo::selected(WidgetType::SelectableLabel, true, on, *label)
                            });
                            ui.ctx().accesskit_node_builder(r.id, |n| {
                                n.set_role(Role::ListBoxOption);
                                n.clear_toggled();
                                n.set_selected(on);
                            });
                        }
                    });
                paint::scroll_fade(ui, &out, fill);
                let (origin, drop) = overlay::popup_origin(list, anchor);
                overlay::transform_since(
                    ui.ctx(),
                    layer,
                    start,
                    e.transform(origin, overlay::POPUP_SCALE, drop),
                );
                if !keyboard && !forced_open {
                    let end = overlay::next_shape(ui.ctx(), layer);
                    overlay::record_exit(
                        ui.ctx(),
                        popup_id,
                        layer,
                        0..0,
                        start.0..end.0,
                        ExitMotion::popup(origin),
                    );
                }
                chose
            });
            if let Some(inner) = shown {
                if let Some(i) = inner.inner {
                    picked = Some(i);
                    Popup::close_id(&ctx, popup_id);
                }
                ctx.accesskit_node_builder(popup_id.with("listbox"), |n| {
                    n.set_role(Role::ListBox);
                });
            }
        }
        if let Some(i) = picked
            && chosen != Some(i)
        {
            *self.value = self.options[i].0.clone();
            trigger.mark_changed();
        }
        if !forced_open {
            ctx.data_mut(|d| d.insert_temp(state_id, st));
        }

        let value = self
            .options
            .iter()
            .find(|(v, _)| v == self.value)
            .map_or(self.placeholder.clone(), |(_, l)| (*l).to_owned());
        let name = self
            .label
            .clone()
            .or_else(|| self.name.clone())
            .unwrap_or_default();
        let expanded = open;
        trigger.widget_info(|| {
            let mut info = WidgetInfo::labeled(WidgetType::ComboBox, enabled, &name);
            info.current_text_value = Some(value.clone());
            info
        });
        let description = error.map(str::to_owned).or_else(|| self.helper.clone());
        let has_error = error.is_some();
        ctx.accesskit_node_builder(trigger.id, |n| {
            n.set_expanded(expanded);
            n.set_has_popup(egui::accesskit::HasPopup::Listbox);
            if let Some(d) = &description {
                n.set_description(d.clone());
            }
            if has_error {
                n.set_invalid(Invalid::True);
            }
        });

        // Helper or error below, as a text field shows them.
        let message = error
            .map(|e| (e, true))
            .or(self.helper.as_deref().map(|h| (h, false)));
        if let Some((text, is_error)) = message {
            ui.add_space(m.field_label_gap);
            let color = if !enabled {
                p.disabled_text
            } else if is_error {
                p.danger
            } else {
                p.text_dim
            };
            let icon_g = is_error.then(|| {
                ink.layout_no_wrap(
                    icon::WARNING_CIRCLE.to_owned(),
                    FontId::proportional(m.icon_sm),
                    color,
                )
            });
            let indent = icon_g
                .as_ref()
                .map_or(0.0, |g| g.size().x + m.field_label_gap);
            let g = ink.layout(
                text.to_owned(),
                FontId::new(m.meta_size, FontFamily::Proportional),
                color,
                (width - indent).max(1.0),
            );
            let (row, _) = ui.allocate_exact_size(vec2(width, g.size().y), Sense::hover());
            if let Some(ig) = icon_g {
                let first = g.rows.first().map_or(g.size().y, |r| r.rect().height());
                ink.galley(
                    pos2(row.left(), row.top() + (first - ig.size().y) / 2.0),
                    ig,
                    color,
                );
            }
            ink.galley(pos2(row.left() + indent, row.top()), g, color);
        }

        let tip = tooltip_text(
            enabled,
            self.tooltip.as_deref(),
            self.disabled_reason.as_deref(),
            None,
        );
        match tip {
            Some(t) => trigger.kit_tooltip(t).show(),
            None => trigger,
        }
    }
}

impl<T: PartialEq + Clone> Widget for Select<'_, T> {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

/// The trigger of a select whose id was set with `.id(id)`: what `Memory::request_focus` takes.
pub fn trigger_id(id: Id) -> Id {
    id.with("kit-select-trigger")
}

/// What the trigger keeps right of its text: a gap, the caret and the inset after it.
fn caret_room(m: &theme::Metrics) -> f32 {
    theme::SM + m.icon_md + theme::SM
}

/// The trigger width that shows each of `labels` whole: the widest one plus the trigger's padding
/// and caret. For a row that sizes a select before showing it; [`Select::fit_to_options`] does
/// this for the select's own options.
pub fn fit_width<'l>(ui: &Ui, labels: impl IntoIterator<Item = &'l str>) -> f32 {
    let m = theme::metrics();
    let font = FontId::new(m.body_size, FontFamily::Proportional);
    let widest = labels
        .into_iter()
        .map(|l| {
            ui.painter()
                .layout_no_wrap(l.to_owned(), font.clone(), Color32::PLACEHOLDER)
                .size()
                .x
        })
        .fold(0.0, f32::max);
    (m.field_pad_x + widest + caret_room(m)).ceil()
}

// ------------------------------------------------------------------------------------- gallery

const STRATEGIES: [(u8, &str); 3] = [
    (0, "Merge (fast-forward if possible)"),
    (1, "Rebase"),
    (2, "Fast-forward only"),
];

/// The gallery's selects: live with a helper and with an error, one open, then hover, focus,
/// disabled, loading, placeholder and small.
pub(crate) fn gallery(ui: &mut Ui) {
    gallery_caption(
        ui,
        "Live (pointer or keyboard) with a label and a helper; an error; beside a text field",
    );
    // Top-aligned, as a form lines up labelled controls of different heights.
    ui.with_layout(
        Layout::left_to_right(Align::Min).with_main_wrap(true),
        |ui| {
            ui.spacing_mut().item_spacing = vec2(theme::LG, theme::SM);
            remembered(ui, "select-live", 1_u8, |ui, v| {
                Select::new(v, &STRATEGIES)
                    .label("Pull strategy")
                    .helper("Canopy passes it to git on every pull.")
                    .width(280.0)
                    .tooltip("How Pull combines your work with the remote's")
                    .id_salt("g-sel0")
                    .show(ui);
            });
            Select::new(&mut 9_u8, &STRATEGIES)
                .label("Required")
                .placeholder("Choose a strategy…")
                .error(Some("Choose how to combine the branches."))
                .width(240.0)
                .id_salt("g-sel6")
                .show(ui);
            field::TextField::new(&mut "origin".to_owned())
                .label("Text field, for comparison")
                .width(200.0)
                .id_salt("g-sel-field")
                .show(ui);
        },
    );
    ui.add_space(theme::SM);
    gallery_caption(ui, "Open, with the pointer on another option");
    Select::new(&mut 1_u8, &STRATEGIES)
        .width(280.0)
        .id_salt("g-sel7")
        .show_forced(
            ui,
            ForceState {
                open: true,
                hover: true,
                ..ForceState::default()
            },
        );
    // Room for the open list.
    ui.add_space(
        3.0 * theme::metrics().menu_item_height + 2.0 * theme::metrics().menu_pad + theme::MD,
    );
    let w = 160.0;
    gallery_caption(ui, "Hover, focus, disabled");
    gallery_row(ui, |ui| {
        Select::new(&mut 1_u8, &STRATEGIES)
            .width(w)
            .id_salt("g-sel1")
            .show_forced(ui, ForceState::HOVER);
        Select::new(&mut 1_u8, &STRATEGIES)
            .width(w)
            .id_salt("g-sel2")
            .show_forced(ui, ForceState::FOCUS);
        Select::new(&mut 1_u8, &STRATEGIES)
            .width(w)
            .enabled(false)
            .disabled_reason("Set by the repository's configuration")
            .id_salt("g-sel3")
            .show(ui);
    });
    ui.add_space(theme::SM);
    gallery_caption(
        ui,
        "Loading, a placeholder (as wide as its longest option), small",
    );
    gallery_row(ui, |ui| {
        Select::new(&mut 1_u8, &STRATEGIES)
            .width(w)
            .loading(true)
            .id_salt("g-sel4")
            .show(ui);
        Select::new(&mut 9_u8, &STRATEGIES)
            .fit_to_options()
            .placeholder("Choose a strategy…")
            .name("Strategy")
            .id_salt("g-sel5")
            .show(ui);
        Select::new(&mut 1_u8, &STRATEGIES)
            .width(w)
            .size(Size::Sm)
            .id(Id::new("gallery-select-small"))
            .show(ui);
    });
}

#[cfg(test)]
mod tests {
    use super::super::testkit::{self, frame, key, spoken};
    use super::*;

    #[test]
    fn a_select_with_a_caller_set_id_can_be_focused_from_elsewhere() {
        let ctx = testkit::ctx();
        let id = Id::new("pull-strategy");
        let mut v = 0_u8;
        let mut focused = false;
        for i in 0..3 {
            if i == 1 {
                ctx.memory_mut(|m| m.request_focus(trigger_id(id)));
            }
            frame(&ctx, Vec::new(), |ui| {
                ui.scope(|ui| {
                    focused = Select::new(&mut v, &STRATEGIES).id(id).show(ui).has_focus();
                });
            });
        }
        assert!(focused);
    }

    #[test]
    fn a_select_fitted_to_its_options_shows_the_longest_whole_and_no_more() {
        let ctx = testkit::ctx();
        let mut v = 0_u8;
        let mut fit = 0.0;
        let mut trigger = egui::Rect::NOTHING;
        let shapes = frame(&ctx, Vec::new(), |ui| {
            ui.set_width(600.0);
            fit = fit_width(ui, STRATEGIES.iter().map(|(_, l)| *l));
            trigger = Select::new(&mut v, &STRATEGIES)
                .fit_to_options()
                .id_salt("fit")
                .show(ui)
                .rect;
        });
        assert!(
            (trigger.width() - fit).abs() < 0.5,
            "{} vs {fit}",
            trigger.width()
        );
        assert!(fit < 600.0, "no wider than it needs");
        let longest = STRATEGIES[0].1;
        assert!(
            shapes.iter().any(|c| match &c.shape {
                egui::Shape::Text(t) => t.galley.text() == longest,
                _ => false,
            }),
            "the longest option is not cut short"
        );
    }

    fn show(ui: &mut Ui, v: &mut u8) -> Response {
        Select::new(v, &STRATEGIES)
            .name("Pull strategy")
            .id_salt("s")
            .show(ui)
    }

    fn open_id(ctx: &egui::Context) -> bool {
        Popup::is_any_open(ctx)
    }

    #[test]
    fn keyboard_opens_moves_chooses_and_closes() {
        let ctx = testkit::ctx();
        let mut v = 0_u8;
        let mut id = Id::NULL;
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        frame(&ctx, key(Key::ArrowDown), |ui| id = show(ui, &mut v).id);
        assert!(open_id(&ctx), "Down opens the list");
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        frame(&ctx, key(Key::ArrowDown), |ui| id = show(ui, &mut v).id);
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        assert_eq!(v, 0, "moving the highlight does not choose");
        let mut changed = false;
        frame(&ctx, key(Key::Enter), |ui| {
            changed = show(ui, &mut v).changed();
        });
        assert!(changed);
        assert_eq!(v, 1, "Enter chooses the highlighted option");
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        assert!(!open_id(&ctx), "and closes the list");
        assert!(
            ctx.memory(|m| m.has_focus(id)),
            "focus stays on the trigger"
        );
    }

    #[test]
    fn escape_closes_and_keeps_focus_and_typing_jumps() {
        let ctx = testkit::ctx();
        let mut v = 0_u8;
        let mut id = Id::NULL;
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        frame(&ctx, key(Key::Space), |ui| id = show(ui, &mut v).id);
        assert!(open_id(&ctx), "Space opens the list");
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        frame(&ctx, vec![Event::Text("f".into())], |ui| {
            id = show(ui, &mut v).id;
        });
        frame(&ctx, key(Key::Enter), |ui| id = show(ui, &mut v).id);
        assert_eq!(v, 2, "typing f highlighted Fast-forward only");
        frame(&ctx, key(Key::ArrowDown), |ui| id = show(ui, &mut v).id);
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        frame(&ctx, key(Key::Escape), |ui| id = show(ui, &mut v).id);
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        assert!(!open_id(&ctx), "Esc closes");
        assert!(
            ctx.memory(|m| m.has_focus(id)),
            "and keeps focus on the trigger"
        );
        assert_eq!(v, 2);
    }

    #[test]
    fn pointer_chooses_from_the_list() {
        let ctx = testkit::ctx();
        let mut v = 0_u8;
        let mut rect = egui::Rect::NOTHING;
        frame(&ctx, Vec::new(), |ui| rect = show(ui, &mut v).rect);
        testkit::click(&ctx, rect.center(), |ui| {
            show(ui, &mut v);
        });
        assert!(open_id(&ctx));
        // Two frames for the list's sizing pass, then click the third row.
        frame(&ctx, Vec::new(), |ui| {
            show(ui, &mut v);
        });
        frame(&ctx, Vec::new(), |ui| {
            show(ui, &mut v);
        });
        let m = theme::metrics();
        let row = pos2(
            rect.center().x,
            rect.bottom() + LIST_GAP + m.menu_pad + 2.5 * m.menu_item_height,
        );
        testkit::click(&ctx, row, |ui| {
            show(ui, &mut v);
        });
        assert_eq!(v, 2);
        frame(&ctx, Vec::new(), |ui| {
            show(ui, &mut v);
        });
        assert!(!open_id(&ctx));
    }

    /// Whether a frame painted the option `label` (the list, open or leaving).
    fn painted(shapes: &[egui::epaint::ClippedShape], label: &str) -> bool {
        fn walk(shape: &egui::Shape, label: &str) -> bool {
            match shape {
                egui::Shape::Vec(v) => v.iter().any(|s| walk(s, label)),
                egui::Shape::Text(t) => t.galley.text() == label,
                _ => false,
            }
        }
        shapes.iter().any(|c| walk(&c.shape, label))
    }

    #[test]
    fn a_pointer_opened_list_fades_out_and_a_keyboard_one_vanishes() {
        let ctx = testkit::ctx();
        motion::set_reduced(false);
        let last = STRATEGIES[2].1;
        let mut v = 0_u8;
        let mut rect = egui::Rect::NOTHING;
        frame(&ctx, Vec::new(), |ui| rect = show(ui, &mut v).rect);
        testkit::click(&ctx, rect.center(), |ui| {
            show(ui, &mut v);
        });
        for _ in 0..12 {
            frame(&ctx, Vec::new(), |ui| {
                show(ui, &mut v);
            });
        }
        assert!(open_id(&ctx));
        // A click outside closes it; the list keeps painting for a moment as it leaves.
        testkit::click(
            &ctx,
            pos2(rect.right() + 300.0, rect.bottom() + 300.0),
            |ui| {
                show(ui, &mut v);
            },
        );
        let leaving = frame(&ctx, Vec::new(), |ui| {
            show(ui, &mut v);
        });
        assert!(!open_id(&ctx), "the click outside closed the list");
        assert!(painted(&leaving, last), "the closed list fades out");
        let mut gone = false;
        for _ in 0..30 {
            gone = !painted(
                &frame(&ctx, Vec::new(), |ui| {
                    show(ui, &mut v);
                }),
                last,
            );
        }
        assert!(gone, "and is gone once its exit has run");

        // Opened and closed from the keyboard: no exit.
        let mut id = Id::NULL;
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        frame(&ctx, key(Key::ArrowDown), |ui| id = show(ui, &mut v).id);
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        assert!(open_id(&ctx), "Down opened it");
        frame(&ctx, key(Key::Escape), |ui| id = show(ui, &mut v).id);
        let after = frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        motion::set_reduced(true);
        assert!(!open_id(&ctx));
        assert!(
            !painted(&after, last),
            "a keyboard-opened list closes at once"
        );
    }

    #[test]
    fn unsalted_selects_keep_their_own_list() {
        let ctx = testkit::ctx();
        let (mut a, mut b) = (0_u8, 0_u8);
        let show = |ui: &mut Ui, a: &mut u8, b: &mut u8| {
            Select::new(a, &STRATEGIES).name("First").show(ui);
            Select::new(b, &STRATEGIES).name("Second").show(ui).rect
        };
        let mut second = egui::Rect::NOTHING;
        frame(&ctx, Vec::new(), |ui| second = show(ui, &mut a, &mut b));
        testkit::click(&ctx, second.center(), |ui| {
            show(ui, &mut a, &mut b);
        });
        let nodes = spoken(&ctx, |ui| {
            show(ui, &mut a, &mut b);
        });
        let expanded = |name: &str| testkit::find(&nodes, name).and_then(|n| n.expanded);
        assert_eq!(expanded("Second"), Some(true), "the clicked select opens");
        assert_eq!(expanded("First"), Some(false), "its neighbour stays shut");
    }

    #[test]
    fn disabled_and_loading_selects_stay_shut() {
        let ctx = testkit::ctx();
        for (enabled, loading) in [(false, false), (true, true)] {
            let mut v = 0_u8;
            let mut rect = egui::Rect::NOTHING;
            let s = |ui: &mut Ui, v: &mut u8| {
                Select::new(v, &STRATEGIES)
                    .enabled(enabled)
                    .loading(loading)
                    .id_salt(("d", enabled))
                    .show(ui)
            };
            frame(&ctx, Vec::new(), |ui| rect = s(ui, &mut v).rect);
            testkit::click(&ctx, rect.center(), |ui| {
                s(ui, &mut v);
            });
            assert!(!open_id(&ctx), "enabled {enabled}, loading {loading}");
        }
    }

    #[test]
    fn trigger_speaks_as_a_combo_box_with_its_value() {
        let ctx = testkit::ctx();
        let mut v = 1_u8;
        let nodes = spoken(&ctx, |ui| {
            show(ui, &mut v);
        });
        let n = nodes
            .iter()
            .find(|n| n.role == Role::ComboBox)
            .expect("a combo box");
        assert_eq!(n.label.as_deref(), Some("Pull strategy"));
        assert_eq!(n.value.as_deref(), Some("Rebase"));
        assert_eq!(n.expanded, Some(false));
    }

    /// A settings row puts its control at the right in a right-to-left layout: a fixed-width
    /// select sits there, at its width, as it did before it stacked a label and message.
    #[test]
    fn a_select_in_a_right_to_left_row_sits_at_the_right() {
        let ctx = testkit::ctx();
        let mut v = 1_u8;
        let (mut trigger, mut row) = (egui::Rect::NOTHING, egui::Rect::NOTHING);
        frame(&ctx, Vec::new(), |ui| {
            ui.set_width(600.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                row = ui.max_rect();
                trigger = Select::new(&mut v, &STRATEGIES)
                    .width(200.0)
                    .id_salt("rtl")
                    .show(ui)
                    .rect;
            });
        });
        assert!((trigger.width() - 200.0).abs() < 0.5, "{trigger:?}");
        assert!(
            (trigger.right() - row.right()).abs() < 0.5,
            "the trigger ends at {} in a row ending at {}",
            trigger.right(),
            row.right()
        );
    }

    /// A select's helper describes it and its error marks it invalid, as a text field's do.
    #[test]
    fn a_select_speaks_its_helper_and_its_error() {
        let ctx = testkit::ctx();
        ctx.enable_accesskit();
        let combo = |helper: &str, error: Option<&str>| {
            let mut v = 9_u8;
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                Select::new(&mut v, &STRATEGIES)
                    .name("Strategy")
                    .helper(helper)
                    .error(error)
                    .id_salt("speaks")
                    .show(ui);
            });
            out.textures_delta.clear();
            out.platform_output
                .accesskit_update
                .and_then(|u| {
                    u.nodes
                        .into_iter()
                        .map(|(_, n)| n)
                        .find(|n| n.role() == Role::ComboBox)
                })
                .expect("a combo box")
        };
        let helped = combo("Canopy passes it to git.", None);
        assert_eq!(helped.description(), Some("Canopy passes it to git."));
        assert_eq!(helped.invalid(), None);
        let wrong = combo("Canopy passes it to git.", Some("Choose a strategy."));
        assert_eq!(
            wrong.description(),
            Some("Choose a strategy."),
            "the error wins"
        );
        assert_eq!(wrong.invalid(), Some(Invalid::True));
    }

    #[test]
    fn keyboard_lists_open_at_once_even_with_motion_on() {
        let ctx = testkit::ctx();
        // Motion on: a pointer-opened list would fade in; a keyboard-opened one never does.
        motion::set_reduced(false);
        let overlay = theme::surfaces(&ctx).overlay;
        let mut v = 0_u8;
        let mut id = Id::NULL;
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        frame(&ctx, key(Key::ArrowDown), |ui| id = show(ui, &mut v).id);
        let shapes = frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        motion::set_reduced(true);
        let opaque = shapes.iter().any(|c| {
            matches!(&c.shape, egui::Shape::Rect(r) if r.fill == overlay && r.blur_width == 0.0)
        });
        assert!(opaque, "the list is fully shown on its first visible frame");
    }
}
