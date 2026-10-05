//! Menus and context menus (design spec 4.12): [`context`] (secondary click), [`menu`] (a button
//! that opens a menu), [`MenuItem`] with its icon column, label, shortcut, check or submenu caret,
//! danger items, [`Menu::separator`], [`Menu::heading`] and [`Menu::submenu`], and
//! [`stock_style`], which makes any stock egui widget inside a menu match the kit.
//!
//! ```ignore
//! kit::menu::context(&response, |m| {
//!     if m.item(icon::GIT_BRANCH, "New branch here…").shortcut(Command::NewBranch).show(m.ui).clicked() { .. }
//!     m.separator();
//!     m.submenu(icon::ARROW_COUNTER_CLOCKWISE, "Reset current branch here", |m| { .. });
//!     m.item(icon::TRASH, "Delete branch…").danger().show(m.ui);
//! });
//! ```
//!
//! Pointer-opened menus fade in and grow from the anchor side; keyboard-opened ones appear at once
//! with the first item focused. Highlight follows the last input: the hovered item, or the focused
//! one after a key press. Disabled items are hidden in context menus and shown at the disabled
//! opacity, with their reason as a tooltip, in menus opened from a button.

use std::cell::Cell;
use std::sync::Arc;

use egui::containers::menu::{MenuState, SubMenu};
use egui::{
    Align, Align2, Color32, CornerRadius, FontFamily, FontId, Frame, Id, Key, Layout, Margin,
    Popup, Rangef, Rect, Response, Sense, Shadow, Shape, Stroke, Ui, Vec2, Widget, WidgetInfo,
    WidgetType, pos2, vec2,
};
use egui_phosphor::regular as icon;

use super::overlay::{self, Entrance, ExitMotion, Layer, TooltipExt};
use super::{ForceState, focus_visible, paint};
use crate::keymap::{self, Command};
use crate::ui::motion;
use crate::ui::theme;

/// Narrowest and widest menu surface.
const MIN_WIDTH: f32 = 200.0;
const MAX_WIDTH: f32 = 360.0;
/// Item insets, the icon column and the gap after it.
const ITEM_PAD: f32 = 8.0;
const ICON_COLUMN: f32 = 15.0;
const ICON_GAP: f32 = 9.0;
/// Least room between the label and the shortcut, caret or check.
const TRAILING_GAP: f32 = 24.0;
/// Heading row height.
const HEADING_HEIGHT: f32 = 24.0;
/// Room above and below a separator.
const SEPARATOR_SPACE: f32 = 4.0;
/// Letter spacing of headings, in em.
const HEADING_TRACKING: f32 = 0.07;

/// Where a menu shows: decides whether disabled items hide and how the menu is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Opened by a secondary click: disabled items are hidden.
    Context,
    /// Opened from a button (menu-bar style): disabled items show, dimmed, with their reason.
    Button,
    /// Painted in place (the gallery): no popup, no submenus.
    Inline,
}

/// The radius of a highlighted item: four less than the surface's own (7 Comfortable, 3 Dense).
pub fn item_radius() -> CornerRadius {
    let m = theme::metrics();
    CornerRadius::same(m.radius_lg.saturating_sub(5).max(3))
}

/// The plain text of `command`'s keys for a menu's shortcut column: the single key that works
/// outside text fields when there is one, else the chord.
pub fn shortcut_text(command: Command) -> String {
    let hint = keymap::hint(command);
    hint.rsplit(" · ").next().unwrap_or_default().to_owned()
}

/// The style menus give stock egui widgets inside them, so a leftover `ui.button` reads as a menu
/// item: no frame at rest, the strong hover ground, the item radius and height.
pub fn stock_style(style: &mut egui::Style) {
    let m = theme::metrics();
    let p = theme::active().palette();
    let r = item_radius();
    style.spacing.button_padding = vec2(
        ITEM_PAD,
        ((m.menu_item_height - m.body_size) / 2.0).max(0.0),
    );
    style.spacing.interact_size.y = m.menu_item_height;
    style.spacing.item_spacing.y = 0.0;
    let w = &mut style.visuals.widgets;
    w.inactive.weak_bg_fill = Color32::TRANSPARENT;
    w.inactive.bg_stroke = Stroke::NONE;
    for state in [&mut w.hovered, &mut w.active, &mut w.open] {
        state.bg_stroke = Stroke::NONE;
        state.corner_radius = r;
        state.expansion = 0.0;
    }
    w.inactive.corner_radius = r;
    w.hovered.weak_bg_fill = p.hover_strong();
    w.open.weak_bg_fill = p.hover_strong();
    w.active.weak_bg_fill = p.pressed();
}

/// A menu being filled: add items with [`Menu::item`] and show them into [`Menu::ui`].
pub struct Menu<'a> {
    pub ui: &'a mut Ui,
    id: Id,
    kind: Kind,
    /// Whether the icon column is reserved (some item had an icon last frame).
    icons: bool,
    saw_icon: Cell<bool>,
    /// Focus the first item (a keyboard-opened menu's first frame).
    focus_first: Cell<bool>,
}

impl Menu<'_> {
    /// An item with `glyph` (a Phosphor icon, or "" for none) and `label`, verb first, "…" when it
    /// asks for more.
    pub fn item(&self, glyph: &str, label: impl Into<String>) -> MenuItem {
        if !glyph.is_empty() {
            self.saw_icon.set(true);
        }
        let mut item = MenuItem::new(glyph, label);
        item.reserve_icon = self.icons;
        item.hide_disabled = self.kind == Kind::Context;
        item.autofocus = self.focus_first.replace(false);
        item
    }

    /// A 1 pt seam across the whole surface, 4 pt above and below.
    pub fn separator(&mut self) {
        separator(self.ui);
    }

    /// An uppercase group label.
    pub fn heading(&mut self, text: &str) {
        heading(self.ui, text);
    }

    /// An item that opens `add` as a submenu: after the pointer rests on it for `SUBMENU_DELAY`,
    /// or on Right or Enter; Left or Esc closes it. The item stays highlighted while it is open.
    /// Returns what `add` returned, while open.
    pub fn submenu<R>(
        &mut self,
        glyph: &str,
        label: &str,
        add: impl FnOnce(&mut Menu<'_>) -> R,
    ) -> Option<R> {
        let item_id = self.ui.id().with(("kit-submenu", label));
        let sub_id = SubMenu::id_from_widget_id(item_id);
        if self.kind == Kind::Inline {
            let item = self
                .item(glyph, label)
                .submenu_caret()
                .id_salt(("kit-submenu", label));
            item.show(self.ui);
            return None;
        }
        let open = MenuState::from_ui(self.ui, |s, _| s.open_item == Some(sub_id));
        let item = self
            .item(glyph, label)
            .submenu_caret()
            .open(open)
            .id_salt(("kit-submenu", label));
        let response = item.show(self.ui);
        let ctx = self.ui.ctx().clone();
        // Keyboard: Right opens it with its first item focused.
        let keyboard_open = response.has_focus()
            && ctx.input(|i| i.key_pressed(Key::ArrowRight) || i.key_pressed(Key::Enter));
        if keyboard_open && !open {
            MenuState::from_ui(self.ui, |s, _| s.open_item = Some(sub_id));
            ctx.data_mut(|d| d.insert_temp(sub_id.with("kit-focus-first"), true));
        }
        // The pointer must rest on the item before the submenu opens.
        let hover_key = item_id.with("kit-hover-since");
        let now = ctx.input(|i| i.time);
        let rested = if response.hovered() {
            let since = ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(hover_key, || now));
            let delay = f64::from(motion::SUBMENU_DELAY);
            if now - since < delay {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                    delay - (now - since),
                ));
                false
            } else {
                true
            }
        } else {
            ctx.data_mut(|d| d.remove::<f64>(hover_key));
            false
        };
        let mut trigger = response.clone();
        if !(open || rested || keyboard_open || response.clicked()) {
            // Not yet: egui's submenu opens on hover, so it must not see this one hovered.
            trigger.rect = Rect::NOTHING;
            trigger.interact_rect = Rect::NOTHING;
        }
        let m = theme::metrics();
        let parent_kind = self.kind;
        let shown = self.ui.scope(|ui| {
            // egui frames submenus itself; make that frame invisible and paint the kit surface.
            let st = ui.style_mut();
            st.visuals.window_fill = Color32::TRANSPARENT;
            st.visuals.window_stroke = Stroke::NONE;
            st.visuals.popup_shadow = Shadow::NONE;
            st.spacing.menu_margin = Margin::same(m.menu_pad as i8);
            SubMenu::new().show(ui, &trigger, |ui| {
                let layer = ui.layer_id();
                let slot = ui.painter().add(Shape::Noop);
                let focus_first = ctx
                    .data_mut(|d| d.remove_temp::<bool>(sub_id.with("kit-focus-first")))
                    .unwrap_or(false);
                let r = body(ui, sub_id, parent_kind, focus_first, add);
                let rect = ui.min_rect().expand(m.menu_pad);
                let shapes = overlay::surface_shapes(ui.painter(), rect, Layer::Menu);
                ui.painter().set(slot, Shape::Vec(shapes));
                // Left goes back to the parent item.
                if ctx.input(|i| i.key_pressed(Key::ArrowLeft))
                    && ctx.memory(egui::Memory::focused).is_some_and(|f| {
                        ctx.read_response(f).is_some_and(|fr| fr.layer_id == layer)
                    })
                {
                    ctx.data_mut(|d| d.insert_temp(sub_id.with("kit-back"), true));
                }
                r
            })
        });
        if ctx
            .data_mut(|d| d.remove_temp::<bool>(sub_id.with("kit-back")))
            .unwrap_or(false)
        {
            MenuState::from_ui(self.ui, |s, _| s.open_item = None);
            response.request_focus();
        }
        shown.inner.map(|r| r.inner)
    }
}

/// Lays out a menu's items: stacked with no gap, the icon column reserved when any item had an
/// icon the frame before.
fn body<R>(
    ui: &mut Ui,
    id: Id,
    kind: Kind,
    focus_first: bool,
    add: impl FnOnce(&mut Menu<'_>) -> R,
) -> R {
    let m = theme::metrics();
    let icons_key = id.with("kit-menu-icons");
    let icons = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(icons_key))
        .unwrap_or(true);
    let inner = 2.0 * m.menu_pad;
    if kind != Kind::Inline {
        // Popups size to their items, between the narrowest and widest menu. The cap applies
        // while egui measures them (later frames keep the measured width).
        ui.set_min_width(MIN_WIDTH - inner);
        if ui.is_sizing_pass() {
            ui.set_max_width(MAX_WIDTH - inner);
        }
    }
    let ctx = ui.ctx().clone();
    ui.with_layout(Layout::top_down_justified(Align::Min), |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut menu = Menu {
            ui,
            id,
            kind,
            icons,
            saw_icon: Cell::new(false),
            focus_first: Cell::new(focus_first),
        };
        let r = add(&mut menu);
        let saw = menu.saw_icon.get();
        ctx.data_mut(|d| d.insert_temp(icons_key, saw));
        let _ = menu.id;
        r
    })
    .inner
}

/// Shows a menu in `popup` anchored to `anchor`.
fn show_popup<R>(
    anchor: &Response,
    popup: Popup<'_>,
    kind: Kind,
    keyboard_now: bool,
    add: impl FnOnce(&mut Menu<'_>) -> R,
) -> Option<R> {
    let ctx = anchor.ctx.clone();
    let id = popup.get_id();
    let was_open = Popup::is_id_open(&ctx, id);
    let kbd_key = id.with("kit-keyboard-opened");
    let opened_now = anchor.clicked() || anchor.secondary_clicked();
    if opened_now && !was_open {
        ctx.data_mut(|d| d.insert_temp(kbd_key, keyboard_now));
    }
    let keyboard = ctx.data(|d| d.get_temp::<bool>(kbd_key)).unwrap_or(false);
    let anchor_rect = anchor.rect;
    let m = theme::metrics();
    let shown = popup.frame(Frame::NONE).style(stock_style).show(|ui| {
        let layer = ui.layer_id();
        let (_, fresh) = overlay::presence(ui.ctx(), id);
        let e = Entrance::of(ui.ctx(), id, motion::BASE, motion::SLOW, keyboard, None);
        ui.multiply_opacity(e.fade);
        let start = overlay::next_shape(ui.ctx(), layer);
        let r = overlay::floating(ui, Layer::Menu, Margin::same(m.menu_pad as i8), |ui| {
            body(ui, id, kind, fresh && keyboard, add)
        });
        let rect = r.response.rect;
        let point = match kind {
            Kind::Context => Popup::position_of_id(ui.ctx(), id)
                .map_or(anchor_rect, |p| Rect::from_min_size(p, Vec2::ZERO)),
            _ => anchor_rect,
        };
        let (origin, drop) = overlay::popup_origin(rect, point);
        overlay::transform_since(
            ui.ctx(),
            layer,
            start,
            e.transform(origin, overlay::POPUP_SCALE, drop),
        );
        if !keyboard {
            let end = overlay::next_shape(ui.ctx(), layer);
            overlay::record_exit(
                ui.ctx(),
                id,
                layer,
                0..0,
                start.0..end.0,
                ExitMotion::popup(origin),
            );
        }
        r.inner
    });
    if let Some(s) = &shown
        && anchor.layer_id.order == egui::Order::Foreground
    {
        // Context actions inside a navigation popup stay above it.
        ctx.set_sublayer(anchor.layer_id, s.response.layer_id);
    }
    if was_open && !Popup::is_id_open(&ctx, id) && keyboard {
        anchor.request_focus();
    }
    shown.map(|s| s.inner)
}

/// A context menu on `response`, opened by a secondary click at the pointer. Returns what `add`
/// returned while it is open.
pub fn context<R>(response: &Response, add: impl FnOnce(&mut Menu<'_>) -> R) -> Option<R> {
    let popup = Popup::context_menu(response);
    show_popup(response, popup, Kind::Context, false, add)
}

/// A menu opened by clicking `button` (or Enter/Space on it, which opens it with the first item
/// focused). Returns what `add` returned while it is open.
pub fn menu<R>(button: &Response, add: impl FnOnce(&mut Menu<'_>) -> R) -> Option<R> {
    let keyboard = button.clicked() && !button.clicked_by(egui::PointerButton::Primary);
    let popup = Popup::menu(button);
    show_popup(button, popup, Kind::Button, keyboard, add)
}

/// A menu painted in place, `width` wide (the gallery shows menus without opening them).
pub(crate) fn inline<R>(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    width: f32,
    add: impl FnOnce(&mut Menu<'_>) -> R,
) -> R {
    let m = theme::metrics();
    let id = ui.id().with(("kit-inline-menu", Id::new(id_salt)));
    ui.allocate_ui(vec2(width, 0.0), |ui| {
        overlay::floating(ui, Layer::Menu, Margin::same(m.menu_pad as i8), |ui| {
            ui.set_width(width - 2.0 * m.menu_pad);
            body(ui, id, Kind::Inline, false, add)
        })
        .inner
    })
    .inner
}

/// A separator: 1 pt `border` across the whole surface (into the menu's padding), 4 pt above and
/// below.
pub fn separator(ui: &mut Ui) {
    let m = theme::metrics();
    let s = theme::surfaces(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(0.0, 2.0 * SEPARATOR_SPACE + 1.0), Sense::hover());
    let x = ui.max_rect().x_range();
    paint::seam_h(
        ui.painter(),
        Rangef::new(x.min - m.menu_pad, x.max + m.menu_pad),
        rect.center().y,
        s.border,
    );
}

/// A group heading: `micro_size` semibold uppercase, tracked out, `text_faint`.
pub fn heading(ui: &mut Ui, text: &str) {
    let m = theme::metrics();
    let p = theme::palette(ui.ctx());
    let mut job = egui::text::LayoutJob::default();
    job.append(
        &text.to_uppercase(),
        0.0,
        egui::TextFormat {
            font_id: FontId::new(m.micro_size, theme::semibold()),
            color: p.text_faint,
            extra_letter_spacing: HEADING_TRACKING * m.micro_size,
            ..Default::default()
        },
    );
    let galley = ui.painter().layout_job(job);
    let (rect, _) = ui.allocate_exact_size(
        vec2(galley.size().x + 2.0 * ITEM_PAD, HEADING_HEIGHT),
        Sense::hover(),
    );
    ui.painter().galley(
        pos2(
            rect.left() + ITEM_PAD,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        p.text_faint,
    );
}

// ------------------------------------------------------------------------------------ MenuItem

/// What sits at an item's right end.
#[derive(Debug, Clone, PartialEq)]
enum Trailing {
    None,
    Shortcut(String),
    Caret,
}

/// One menu item. Build it with [`Menu::item`] inside a menu, or [`MenuItem::new`].
#[derive(Debug)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct MenuItem {
    glyph: String,
    label: String,
    description: Option<String>,
    trailing: Trailing,
    checked: Option<bool>,
    danger: bool,
    enabled: bool,
    disabled_reason: Option<String>,
    open: bool,
    reserve_icon: bool,
    hide_disabled: bool,
    autofocus: bool,
    id_salt: Option<Id>,
    force: Option<ForceState>,
}

impl MenuItem {
    pub fn new(glyph: &str, label: impl Into<String>) -> Self {
        Self {
            glyph: glyph.to_owned(),
            label: label.into(),
            description: None,
            trailing: Trailing::None,
            checked: None,
            danger: false,
            enabled: true,
            disabled_reason: None,
            open: false,
            reserve_icon: !glyph.is_empty(),
            hide_disabled: false,
            autofocus: false,
            id_salt: None,
            force: None,
        }
    }
    /// The command's keys at the right end, as plain text.
    pub fn shortcut(mut self, command: Command) -> Self {
        self.trailing = Trailing::Shortcut(shortcut_text(command));
        self
    }
    /// A check mark at the right end when `on` (choices); spoken as a checkable item.
    pub fn checked(mut self, on: bool) -> Self {
        self.checked = Some(on);
        self
    }
    /// Destroys something: label and icon in `danger`, a red highlight. Put it last in its group,
    /// after a separator; it still asks for confirmation.
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    /// Why it is disabled, as its tooltip (button menus; context menus hide disabled items).
    pub fn disabled_reason(mut self, text: impl Into<String>) -> Self {
        self.disabled_reason = Some(text.into());
        self
    }
    /// A second line under a semibold label (reset modes), the item twice as tall less 6.
    pub fn description(mut self, text: impl Into<String>) -> Self {
        self.description = Some(text.into());
        self
    }
    pub fn id_salt(mut self, salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        self.id_salt = Some(Id::new(salt));
        self
    }
    fn submenu_caret(mut self) -> Self {
        self.trailing = Trailing::Caret;
        self
    }
    /// Its submenu is open: stays highlighted.
    fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }
    /// Shows the item with forced states (the gallery and tests only).
    pub(crate) fn show_forced(mut self, ui: &mut Ui, force: ForceState) -> Response {
        self.force = Some(force);
        self.show(ui)
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let m = theme::metrics();
        let enabled = self.enabled && ui.is_enabled();
        if !enabled && self.hide_disabled {
            return ui.allocate_response(Vec2::ZERO, Sense::hover());
        }
        let two_line = self.description.is_some();
        let height = if two_line {
            2.0 * m.menu_item_height - 6.0
        } else {
            m.menu_item_height
        };
        let label_font = if two_line {
            FontId::new(m.body_size, theme::semibold())
        } else {
            FontId::new(m.body_size, FontFamily::Proportional)
        };
        let label =
            ui.painter()
                .layout_no_wrap(self.label.clone(), label_font, Color32::PLACEHOLDER);
        let description = self.description.as_ref().map(|d| {
            ui.painter().layout_no_wrap(
                d.clone(),
                FontId::new(m.meta_size, FontFamily::Proportional),
                Color32::PLACEHOLDER,
            )
        });
        let trailing = match (&self.trailing, self.checked) {
            (Trailing::Caret, _) => Some((
                icon::CARET_RIGHT.to_owned(),
                FontId::proportional(m.icon_xs),
            )),
            (_, Some(true)) => Some((icon::CHECK.to_owned(), FontId::proportional(m.icon_sm))),
            (Trailing::Shortcut(s), _) => {
                Some((s.clone(), FontId::new(m.chord_size, FontFamily::Monospace)))
            }
            _ => None,
        }
        .map(|(t, f)| ui.painter().layout_no_wrap(t, f, Color32::PLACEHOLDER));
        let icon_w = if self.reserve_icon || !self.glyph.is_empty() {
            ICON_COLUMN + ICON_GAP
        } else {
            0.0
        };
        let text_w = label
            .size()
            .x
            .max(description.as_ref().map_or(0.0, |d| d.size().x));
        let trailing_w = trailing.as_ref().map_or(0.0, |t| TRAILING_GAP + t.size().x);
        let width = 2.0 * ITEM_PAD + icon_w + text_w + trailing_w;
        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let response = match self.id_salt {
            Some(salt) => {
                let rect = ui.allocate_space(vec2(width, height)).1;
                ui.interact(rect, ui.id().with(salt), sense)
            }
            None => ui.allocate_response(vec2(width, height), sense),
        };
        let rect = response.rect;
        if self.autofocus && enabled {
            response.request_focus();
        }
        let f = self.force.unwrap_or_default();
        let keyboard = focus_visible(&ctx);
        let other_focused = keyboard
            && ctx
                .memory(egui::Memory::focused)
                .is_some_and(|id| id != response.id);
        let highlight = enabled
            && (f.hover
                || f.focus
                || self.open
                || (response.has_focus() && keyboard)
                || (response.hovered() && !other_focused));
        if ui.is_rect_visible(rect) {
            let mut painter = ui.painter().clone();
            if !enabled && ui.is_enabled() {
                painter.multiply_opacity(theme::DISABLED_ALPHA);
            }
            if highlight {
                let ground = if self.danger {
                    p.danger_highlight()
                } else {
                    p.hover_strong()
                };
                painter.rect_filled(rect, item_radius(), ground);
            }
            let (text_color, icon_color) = if self.danger {
                (p.danger, p.danger)
            } else {
                (p.text, p.text_faint)
            };
            let mut x = rect.left() + ITEM_PAD;
            let cy = rect.center().y;
            if !self.glyph.is_empty() {
                painter.text(
                    pos2(x + ICON_COLUMN / 2.0, cy),
                    Align2::CENTER_CENTER,
                    &self.glyph,
                    FontId::proportional(ICON_COLUMN),
                    icon_color,
                );
            }
            x += icon_w;
            match &description {
                Some(d) => {
                    let gap = 2.0;
                    let total = label.size().y + gap + d.size().y;
                    let top = cy - total / 2.0;
                    painter.galley(pos2(x, top), Arc::clone(&label), text_color);
                    painter.galley(
                        pos2(x, top + label.size().y + gap),
                        Arc::clone(d),
                        p.text_dim,
                    );
                }
                None => {
                    painter.galley(
                        pos2(x, cy - label.size().y / 2.0),
                        Arc::clone(&label),
                        text_color,
                    );
                }
            }
            if let Some(t) = &trailing {
                let color = if self.checked == Some(true) && self.trailing != Trailing::Caret {
                    p.accent
                } else {
                    p.text_faint
                };
                painter.galley(
                    pos2(rect.right() - ITEM_PAD - t.size().x, cy - t.size().y / 2.0),
                    Arc::clone(t),
                    color,
                );
            }
        }
        let name = self.label.clone();
        let checked = self.checked;
        response.widget_info(|| match checked {
            Some(on) => WidgetInfo::selected(WidgetType::Checkbox, enabled, on, &name),
            None => WidgetInfo::labeled(WidgetType::Button, enabled, &name),
        });
        let caret = self.trailing == Trailing::Caret;
        ctx.accesskit_node_builder(response.id, |n| {
            n.set_role(if checked.is_some() {
                egui::accesskit::Role::MenuItemCheckBox
            } else {
                egui::accesskit::Role::MenuItem
            });
            if caret {
                n.set_has_popup(egui::accesskit::HasPopup::Menu);
                if self.open {
                    n.set_expanded(true);
                }
            }
        });
        match (enabled, self.disabled_reason) {
            (false, Some(reason)) => response.kit_tooltip(reason).show(),
            _ => response,
        }
    }
}

impl Widget for MenuItem {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

// ------------------------------------------------------------------------------------- gallery

/// The gallery's Menus section: a context menu as History shows it (icons, shortcuts, a submenu
/// caret, separators, a danger item, one highlighted), the danger item highlighted, a button menu
/// with a heading, checks, a disabled item and two-line reset modes, and an open submenu beside
/// its parent.
pub(crate) fn gallery(ui: &mut Ui) {
    let m = theme::metrics();
    let p = theme::palette(ui.ctx());
    let small = |ui: &mut Ui, text: &str| {
        ui.label(
            egui::RichText::new(text)
                .size(m.small_size)
                .color(p.text_faint),
        );
    };
    let width = 260.0_f32.min(ui.available_width());
    small(
        ui,
        "Context menu, one item highlighted; danger highlighted; keyboard focus",
    );
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = vec2(theme::LG, theme::LG);
        inline(ui, "g-context", width, |m| {
            m.item(icon::SIGN_IN, "Check out").show(m.ui);
            m.item(icon::GIT_BRANCH, "New branch here…")
                .shortcut(Command::NewBranch)
                .show_forced(m.ui, ForceState::HOVER);
            m.item(icon::TAG, "New tag here…")
                .shortcut(Command::Tag)
                .show(m.ui);
            m.separator();
            m.item(icon::GIT_COMMIT, "Cherry-pick").show(m.ui);
            m.item(icon::ARROW_U_UP_LEFT, "Revert commit…").show(m.ui);
            m.submenu(icon::COPY, "Copy", |_| {});
            m.separator();
            m.item(icon::ARROW_COUNTER_CLOCKWISE, "Reset main to here…")
                .danger()
                .show(m.ui);
        });
        ui.vertical(|ui| {
            inline(ui, "g-danger", width, |m| {
                m.item(icon::GIT_BRANCH, "Rename branch…").show(m.ui);
                m.item(icon::ARROW_SQUARE_OUT, "Push branch").show(m.ui);
                m.separator();
                m.item(icon::TRASH, "Delete branch…")
                    .danger()
                    .show_forced(m.ui, ForceState::HOVER);
            });
            inline(ui, "g-focus", width, |m| {
                m.item(icon::FOLDER_OPEN, "Show in file browser").show(m.ui);
                m.item(icon::CLOCK_COUNTER_CLOCKWISE, "Show history")
                    .show_forced(m.ui, ForceState::FOCUS);
                m.item(icon::COPY, "Copy path").show(m.ui);
            });
        });
    });
    ui.add_space(theme::SM);
    small(
        ui,
        "Button menu: heading, checks, disabled with a reason, two-line items; open submenu",
    );
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(theme::LG, theme::LG);
        inline(ui, "g-button", width + 40.0, |m| {
            m.heading("Sort branches");
            m.item("", "By name").checked(true).show(m.ui);
            m.item("", "By last commit").checked(false).show(m.ui);
            m.separator();
            m.heading("Reset current branch here");
            m.item("", "Soft")
                .description("Keep changes staged")
                .show_forced(m.ui, ForceState::HOVER);
            m.item("", "Mixed")
                .description("Keep changes, unstaged")
                .show(m.ui);
            m.item("", "Hard")
                .description("Discard all changes")
                .danger()
                .show(m.ui);
            m.separator();
            m.item(icon::ARROW_UP, "Push")
                .enabled(false)
                .disabled_reason("Nothing to push")
                .show(m.ui);
        });
    });
    ui.add_space(theme::SM);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        inline(ui, "g-parent", width, |m| {
            m.item(icon::COPY, "Copy SHA")
                .shortcut(Command::CopySha)
                .show(m.ui);
            m.item(icon::COPY, "Copy")
                .submenu_caret()
                .open(true)
                .show(m.ui);
            m.item(icon::LINK, "Copy link").show(m.ui);
        });
        ui.vertical(|ui| {
            ui.add_space(m.menu_item_height);
            inline(ui, "g-child", MIN_WIDTH, |m| {
                m.item("", "Short SHA").show(m.ui);
                m.item("", "Full SHA").show_forced(m.ui, ForceState::HOVER);
                m.item("", "Subject").show(m.ui);
            });
        });
    });
    ui.add_space(theme::SM);
    small(ui, "Live: open the menu, or right-click the box");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::LG;
        let button = super::Button::new("Sort")
            .caret()
            .id_salt("g-live-menu")
            .show(ui);
        menu(&button, |m| {
            m.heading("Sort branches");
            m.item("", "By name").checked(true).show(m.ui);
            m.item("", "By last commit").checked(false).show(m.ui);
            m.separator();
            m.submenu(icon::FUNNEL, "Show", |m| {
                m.item("", "Local branches").checked(true).show(m.ui);
                m.item("", "Remote branches").checked(true).show(m.ui);
            });
        });
        let (rect, area) = ui.allocate_exact_size(vec2(180.0, m.control), Sense::click());
        let s = theme::surfaces(ui.ctx());
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(m.radius),
            Stroke::new(1.0, s.border),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            "Right-click here",
            FontId::proportional(m.meta_size),
            p.text_dim,
        );
        context(&area, |m| {
            m.item(icon::COPY, "Copy SHA")
                .shortcut(Command::CopySha)
                .show(m.ui);
            m.separator();
            m.item(icon::TRASH, "Delete branch…").danger().show(m.ui);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context_() -> egui::Context {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        ctx
    }

    fn run(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        mut f: impl FnMut(&mut Ui),
    ) -> egui::FullOutput {
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1024.0, 640.0))),
                events,
                ..Default::default()
            },
            |ui| f(ui),
        );
        out.textures_delta.clear();
        out
    }

    fn click(at: egui::Pos2, button: egui::PointerButton) -> [Vec<egui::Event>; 2] {
        [
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                },
            ],
            vec![egui::Event::PointerButton {
                pos: at,
                button,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            }],
        ]
    }

    #[test]
    fn items_take_the_menu_item_height_and_full_width() {
        let ctx = context_();
        for density in theme::Density::ALL {
            theme::apply(
                &ctx,
                theme::ThemeId::Midnight,
                theme::Appearance {
                    density,
                    ..theme::Appearance::STILL
                },
            );
            let m = theme::metrics();
            let mut rects = Vec::new();
            let _ = run(&ctx, Vec::new(), |ui| {
                rects.clear();
                inline(ui, "t", 260.0, |menu| {
                    rects.push(menu.item(icon::TAG, "New tag…").show(menu.ui).rect);
                    rects.push(menu.item("", "Plain").show(menu.ui).rect);
                    rects.push(
                        menu.item("", "Hard")
                            .description("Discard")
                            .show(menu.ui)
                            .rect,
                    );
                });
            });
            assert_eq!(rects[0].height(), m.menu_item_height, "{density:?}");
            assert_eq!(rects[1].width(), rects[0].width(), "justified");
            assert_eq!(rects[0].width(), 260.0 - 2.0 * m.menu_pad);
            assert_eq!(rects[2].height(), 2.0 * m.menu_item_height - 6.0);
        }
    }

    #[test]
    fn disabled_items_hide_in_context_menus_and_never_click() {
        let ctx = context_();
        let mut rect = Rect::NOTHING;
        let mut hidden = Rect::NOTHING;
        let mut clicked = false;
        let frame =
            |events: Vec<egui::Event>, rect: &mut Rect, hidden: &mut Rect, clicked: &mut bool| {
                let _ = run(&ctx, events, |ui| {
                    let r = MenuItem::new(icon::ARROW_UP, "Push")
                        .enabled(false)
                        .disabled_reason("Nothing to push")
                        .show(ui);
                    *rect = r.rect;
                    *clicked |= r.clicked();
                    let mut item = MenuItem::new(icon::ARROW_UP, "Push").enabled(false);
                    item.hide_disabled = true;
                    *hidden = item.show(ui).rect;
                });
            };
        frame(Vec::new(), &mut rect, &mut hidden, &mut clicked);
        for events in click(rect.center(), egui::PointerButton::Primary) {
            frame(events, &mut rect, &mut hidden, &mut clicked);
        }
        assert!(!clicked, "a disabled item never fires");
        assert_eq!(hidden.size(), Vec2::ZERO, "hidden in context menus");
        assert!(rect.height() > 0.0, "shown in button menus");
    }

    #[test]
    fn enter_on_a_focused_item_activates_it() {
        let ctx = context_();
        let mut clicked = false;
        let mut id = None;
        for frame in 0..3 {
            let events = if frame == 2 {
                vec![egui::Event::Key {
                    key: Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::default(),
                }]
            } else {
                Vec::new()
            };
            let _ = run(&ctx, events, |ui| {
                let r = MenuItem::new(icon::TAG, "New tag…").show(ui);
                if frame == 0 {
                    r.request_focus();
                }
                id = Some(r.id);
                clicked |= r.clicked();
            });
        }
        assert!(clicked);
    }

    #[test]
    fn items_report_menu_roles() {
        use egui::accesskit::Role;

        let ctx = context_();
        ctx.enable_accesskit();
        let out = run(&ctx, Vec::new(), |ui| {
            MenuItem::new(icon::TAG, "New tag…").show(ui);
            MenuItem::new("", "By name").checked(true).show(ui);
            MenuItem::new(icon::COPY, "Copy").submenu_caret().show(ui);
        });
        let nodes = out
            .platform_output
            .accesskit_update
            .expect("accesskit")
            .nodes;
        let role = |label: &str| {
            nodes
                .iter()
                .find(|(_, n)| n.label() == Some(label))
                .map(|(_, n)| n.role())
        };
        assert_eq!(role("New tag…"), Some(Role::MenuItem));
        assert_eq!(role("By name"), Some(Role::MenuItemCheckBox));
        assert!(
            nodes
                .iter()
                .any(|(_, n)| n.label() == Some("Copy") && n.has_popup().is_some())
        );
    }

    #[test]
    fn danger_items_paint_in_danger_and_highlight_red() {
        let ctx = context_();
        let p = theme::palette(&ctx);
        let out = run(&ctx, Vec::new(), |ui| {
            MenuItem::new(icon::TRASH, "Delete branch…")
                .danger()
                .show_forced(ui, ForceState::HOVER);
        });
        let mut fills = Vec::new();
        let mut texts = Vec::new();
        for c in &out.shapes {
            match &c.shape {
                Shape::Rect(r) => fills.push(r.fill),
                Shape::Text(t) => texts.push(t.fallback_color),
                _ => {}
            }
        }
        assert!(fills.contains(&p.danger_highlight()), "{fills:?}");
        assert!(texts.iter().all(|c| *c == p.danger), "{texts:?}");
    }

    #[test]
    fn a_context_menu_opens_on_secondary_click_and_an_item_closes_it() {
        struct Seen {
            target: Rect,
            item: Rect,
            chosen: bool,
            open: bool,
        }
        let ctx = context_();
        let frame = |events: Vec<egui::Event>, seen: &mut Seen| {
            let _ = run(&ctx, events, |ui| {
                let r = ui.allocate_response(vec2(200.0, 40.0), Sense::click());
                seen.target = r.rect;
                seen.open = context(&r, |m| {
                    let i = m.item(icon::TAG, "New tag…").show(m.ui);
                    seen.item = i.rect;
                    seen.chosen |= i.clicked();
                })
                .is_some();
            });
        };
        let mut seen = Seen {
            target: Rect::NOTHING,
            item: Rect::NOTHING,
            chosen: false,
            open: false,
        };
        frame(Vec::new(), &mut seen);
        assert!(!seen.open);
        for events in click(seen.target.center(), egui::PointerButton::Secondary) {
            frame(events, &mut seen);
        }
        frame(Vec::new(), &mut seen);
        frame(Vec::new(), &mut seen);
        assert!(seen.open, "secondary click opens it");
        for events in click(seen.item.center(), egui::PointerButton::Primary) {
            frame(events, &mut seen);
        }
        frame(Vec::new(), &mut seen);
        assert!(seen.chosen, "the item fired");
        assert!(!seen.open, "and the menu closed");
    }

    #[test]
    fn shortcut_text_prefers_the_single_key() {
        assert_eq!(shortcut_text(Command::Fetch), "f");
    }
}
