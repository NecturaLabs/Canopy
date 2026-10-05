//! [`Select`]: a choice of one value from a list, replacing `egui::ComboBox`. Design spec 4.3.
//!
//! The trigger is a Secondary-styled control showing the value and a caret that turns over while
//! the list is open; the list is a floating menu surface under the trigger (above it when there is
//! no room). Focus stays on the trigger: Up and Down open the list and move the highlight, Home
//! and End jump, Enter or Space choose, Esc closes and keeps focus, and typing jumps to the first
//! option that starts with what was typed (the buffer clears after a second). Opening with the
//! pointer animates in; opening from the keyboard is instant.

use egui::accesskit::Role;
use egui::emath::TSTransform;
use egui::epaint::TextShape;
use egui::{
    Align2, Color32, CornerRadius, Event, FontFamily, FontId, Id, Key, Modifiers, PointerButton,
    Popup, PopupCloseBehavior, RectAlign, Response, Sense, Ui, UiBuilder, Widget, WidgetInfo,
    WidgetType, pos2, vec2,
};
use egui_phosphor::regular as icon;

use super::button::{disabled_painter, tooltip_text, with_tooltip};
use super::toggle::{component_id, gallery_caption, gallery_row, remembered};
use super::{ForceState, Interact, Size, anim, glass, paint};
use crate::ui::motion;
use crate::ui::theme::{self, Elevation, Fill};

/// Trigger padding: 10 left of the value, 8 right of the caret, 8 between (concept `.k-trigger`).
const PAD_LEFT: f32 = 10.0;
const PAD_RIGHT: f32 = 8.0;
const GAP: f32 = 8.0;
/// The caret turns over in 0.25 s.
const CARET_TURN: f32 = 0.25;
/// The list: 6 below the trigger, 2 wider on each side, at most 8.5 rows tall.
const LIST_GAP: f32 = 6.0;
const LIST_OVERHANG: f32 = 2.0;
const LIST_ROWS: f32 = 8.5;
const LIST_MIN_WIDTH: f32 = 200.0;
/// Menu items: inner padding and the gap before the check.
const ITEM_PAD: f32 = 8.0;
/// Pointer-opened lists: fade in over 0.16 s while scaling from 0.95 and dropping 4 pt on
/// `SPRING_CURVE` over `SLOW`.
const FADE_IN: f32 = 0.16;
const SCALE_FROM: f32 = 0.95;
const DROP: f32 = 4.0;
/// Type-ahead: the buffer clears this long after the last key.
const TYPE_AHEAD_RESET: f64 = 1.0;

/// What a select remembers between frames while its list is open.
#[derive(Debug, Clone, Default)]
struct ListState {
    highlight: Option<usize>,
    by_keyboard: bool,
    opened_at: f64,
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
    width: Option<f32>,
    size: Size,
    enabled: bool,
    loading: bool,
    tooltip: Option<String>,
    disabled_reason: Option<String>,
    id_salt: Option<Id>,
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
            width: None,
            size: Size::Md,
            enabled: true,
            loading: false,
            tooltip: None,
            disabled_reason: None,
            id_salt: None,
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
    /// Trigger width; the available width by default.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
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
    /// `open` shows the list (statically, at its final position); `hover` with `open` highlights
    /// the first option that is not chosen.
    pub(crate) fn show_forced(mut self, ui: &mut Ui, force: ForceState) -> Response {
        self.force = Some(force);
        self.show(ui)
    }

    /// Shows the trigger (and the list while open). The response is the trigger's and reports
    /// `changed` when a new value was chosen.
    pub fn show(self, ui: &mut Ui) -> Response {
        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let style = theme::style();
        let s = p.surfaces(style);
        let m = theme::metrics();
        let enabled = self.enabled && ui.is_enabled();
        let id = component_id(ui, "kit-select", self.id_salt);
        let popup_id = id.with("kit-select-list");
        let state_id = id.with("kit-select-state");
        let width = self.width.unwrap_or_else(|| ui.available_width());
        let chosen = self.options.iter().position(|(v, _)| v == self.value);

        if let Some(text) = &self.label {
            let g = ui.painter().layout_no_wrap(
                text.clone(),
                FontId::new(12.0, theme::medium()),
                p.text_dim,
            );
            let (rect, _) = ui.allocate_exact_size(vec2(width, g.size().y), Sense::hover());
            disabled_painter(ui, enabled, theme::DISABLED_ALPHA).galley(rect.min, g, p.text_dim);
            ui.add_space(m.field_label_gap);
        }

        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, mut trigger) = ui.allocate_exact_size(vec2(width, self.size.height()), sense);
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
                    opened_at: now,
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
                        opened_at: now,
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
        if ui.is_rect_visible(rect) {
            let painter = disabled_painter(ui, enabled, theme::DISABLED_ALPHA);
            let radius = CornerRadius::same(m.radius);
            let body = blend(
                blend(s.raised, s.raised_hover, ist.hover),
                s.raised_pressed,
                ist.press,
            );
            let ring = s.raised_edge.lerp_to_gamma(p.focus, open_t);
            paint::surface(
                &painter,
                rect,
                radius,
                p,
                style,
                Elevation::Control,
                body,
                Some(ring),
            );
            if ist.focus > 0.0 {
                paint::focus_ring(
                    &painter,
                    rect,
                    radius,
                    p.focus.gamma_multiply(ist.focus),
                    Color32::TRANSPARENT,
                );
            }
            let caret_x = rect.right() - PAD_RIGHT - m.icon_md / 2.0;
            if self.loading {
                paint::spinner(
                    &painter,
                    pos2(caret_x, rect.center().y),
                    m.icon_md * 0.33,
                    1.6,
                    p.text_faint,
                );
            } else {
                let g = painter.layout_no_wrap(
                    icon::CARET_DOWN.to_owned(),
                    FontId::proportional(m.icon_md),
                    p.text_faint,
                );
                let pos = pos2(caret_x, rect.center().y) - g.size() / 2.0;
                let angle = std::f32::consts::PI * open_t;
                painter.add(
                    TextShape::new(pos, g, p.text_faint)
                        .with_angle_and_anchor(angle, Align2::CENTER_CENTER),
                );
            }
            let (text, color) = match chosen {
                Some(i) => (self.options[i].1.to_owned(), p.text),
                None => (self.placeholder.clone(), p.text_faint),
            };
            let max = (caret_x - m.icon_md / 2.0 - GAP - rect.left() - PAD_LEFT).max(0.0);
            let mut job = egui::text::LayoutJob::simple_singleline(
                text,
                FontId::new(m.body_size, FontFamily::Proportional),
                color,
            );
            job.wrap = egui::text::TextWrapping::truncate_at_width(max);
            let g = painter.layout_job(job);
            painter.galley(
                pos2(rect.left() + PAD_LEFT, rect.center().y - g.size().y / 2.0),
                g,
                color,
            );
        }

        // The list.
        if open {
            let reveal = if forced_open || st.by_keyboard || motion::reduced() {
                1.0
            } else {
                (crate::ui::to_f32(now - st.opened_at) / motion::SLOW).clamp(0.0, 1.0)
            };
            let fade = if forced_open || st.by_keyboard || motion::reduced() {
                1.0
            } else {
                (crate::ui::to_f32(now - st.opened_at) / FADE_IN).clamp(0.0, 1.0)
            };
            if reveal < 1.0 || fade < 1.0 {
                ctx.request_repaint();
            }
            if forced_open && force.is_some_and(|f| f.hover) {
                st.highlight = (0..self.options.len()).find(|i| Some(*i) != chosen);
            }
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
                ui.multiply_opacity(fade);
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
                chose
            });
            if let Some(inner) = shown {
                let layer = inner.response.layer_id;
                let k = motion::SPRING_CURVE.at(reveal);
                let scale = SCALE_FROM + (1.0 - SCALE_FROM) * k;
                let origin = inner.response.rect.center_top();
                let tf = TSTransform::from_translation(vec2(0.0, -DROP * (1.0 - k)))
                    * TSTransform::from_translation(origin.to_vec2())
                    * TSTransform::from_scaling(scale)
                    * TSTransform::from_translation(-origin.to_vec2());
                ctx.set_transform_layer(layer, tf);
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
        ctx.accesskit_node_builder(trigger.id, |n| {
            n.set_expanded(expanded);
            n.set_has_popup(egui::accesskit::HasPopup::Listbox);
        });
        let tip = tooltip_text(
            enabled,
            self.tooltip.as_deref(),
            self.disabled_reason.as_deref(),
            None,
        );
        with_tooltip(trigger, tip)
    }
}

impl<T: PartialEq + Clone> Widget for Select<'_, T> {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

/// `a` moved towards `b` by `t`, both ends.
fn blend(a: Fill, b: Fill, t: f32) -> Fill {
    if t <= 0.0 {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    Fill {
        top: a.top.lerp_to_gamma(b.top, t),
        bottom: a.bottom.lerp_to_gamma(b.bottom, t),
    }
}

// ------------------------------------------------------------------------------------- gallery

const STRATEGIES: [(u8, &str); 3] = [
    (0, "Merge (fast-forward if possible)"),
    (1, "Rebase"),
    (2, "Fast-forward only"),
];

/// The gallery's selects: live, hover, focus, disabled, loading, placeholder, and one open.
pub(crate) fn gallery(ui: &mut Ui) {
    gallery_caption(ui, "Live (pointer or keyboard), with a label");
    remembered(ui, "select-live", 1_u8, |ui, v| {
        Select::new(v, &STRATEGIES)
            .label("Pull strategy")
            .width(280.0)
            .tooltip("How Pull combines your work with the remote's")
            .id_salt("g-sel0")
            .show(ui);
    });
    ui.add_space(theme::SM);
    gallery_caption(ui, "Hover, focus, disabled, loading, placeholder, small");
    gallery_row(ui, |ui| {
        let w = 160.0;
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
        Select::new(&mut 1_u8, &STRATEGIES)
            .width(w)
            .loading(true)
            .id_salt("g-sel4")
            .show(ui);
        Select::new(&mut 9_u8, &STRATEGIES)
            .width(w)
            .placeholder("Choose a strategy…")
            .name("Strategy")
            .id_salt("g-sel5")
            .show(ui);
        Select::new(&mut 1_u8, &STRATEGIES)
            .width(w)
            .size(Size::Sm)
            .id_salt("g-sel6")
            .show(ui);
    });
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
    ui.add_space(3.0 * theme::metrics().menu_item_height + 2.0 * theme::metrics().menu_pad + 12.0);
}

#[cfg(test)]
mod tests {
    use super::super::testkit::{self, frame, key, spoken};
    use super::*;

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
