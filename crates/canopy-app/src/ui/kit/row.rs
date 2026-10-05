//! Rows: [`ListRow`], [`TreeRow`] with its animated disclosure, [`Disclosure`],
//! [`SectionHeader`], and [`Selection`] for Shift and Ctrl clicks (design spec 4.16).
//!
//! Rows snap: selection changes at once and the hover ground follows the pointer within `FAST`.
//! Selection is a translucent wash with no side bar; the keyboard cursor on an unselected row is
//! an inset 1.5 pt focus ring. Every row is focusable and activates with Enter or Space.

use std::collections::BTreeSet;
use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat};
use egui::{
    Align, Align2, Color32, CornerRadius, FontFamily, FontId, Id, Key, Layout, Modifiers, Painter,
    Rect, Response, Sense, Shape, Ui, UiBuilder, Widget, WidgetInfo, WidgetType, pos2, vec2,
};
use egui_phosphor::regular as icon;

use super::button::IconButton;
use super::{ForceState, KitResponse, Size, anim, focus_visible, paint};
use crate::ui::motion;
use crate::ui::theme::{self, Palette};

/// Which list a row belongs to: its height, ground and label treatment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RowKind {
    /// Lists in panes (`row_height`), rounded.
    #[default]
    List,
    /// File rows in the changes and commit lists (`row_height`), rounded.
    File,
    /// History table rows (`history_row_height`): full-span, zebra in Dense.
    History,
    /// Sidebar items (`sidebar_row_height`) on the chrome: dim labels, the strong hover,
    /// semibold when selected.
    Sidebar,
    /// Pull and merge request rows (`request_row_height`).
    Request,
    /// Command palette and cheatsheet rows (`palette_row_height`).
    Palette,
}

impl RowKind {
    /// The row height in the active metrics.
    pub fn height(self) -> f32 {
        let m = theme::metrics();
        match self {
            Self::List | Self::File => m.row_height,
            Self::History => m.history_row_height,
            Self::Sidebar => m.sidebar_row_height,
            Self::Request => m.request_row_height,
            Self::Palette => m.palette_row_height,
        }
    }

    /// Tables span the full width with square ends; lists are rounded.
    fn rounded(self) -> bool {
        self != Self::History
    }

    fn chrome(self) -> bool {
        self == Self::Sidebar
    }

    /// The surface the row sits on by default.
    fn ground(self, p: &Palette) -> Color32 {
        let s = p.s();
        if self.chrome() { s.panel } else { s.pane }
    }
}

/// The opaque colour of a row's background: its ground with the zebra, selection, hover and
/// press overlays on top. The graph paints node rings and avatar rings in it.
#[expect(
    clippy::fn_params_excessive_bools,
    reason = "zebra, selection, hover and press are independent overlays"
)]
pub fn ground(
    p: &Palette,
    base: Color32,
    zebra: bool,
    selected: bool,
    hover: f32,
    pressed: bool,
    strong_hover: bool,
) -> Color32 {
    let mut c = base;
    if zebra {
        c = theme::over(p.zebra(), c);
    }
    if selected {
        c = theme::over(p.selected(), c);
    }
    if hover > 0.0 {
        let h = if strong_hover {
            p.hover_strong()
        } else {
            p.hover()
        };
        c = theme::over(h.gamma_multiply(hover), c);
    }
    if pressed {
        c = theme::over(p.pressed(), c);
    }
    c
}

/// Whether row `index` (0-based) of a history table gets the zebra stripe: even rows counting
/// from one, in Dense only.
pub fn zebra(index: usize) -> bool {
    theme::metrics().zebra && index % 2 == 1
}

/// What a row's content paints with: its rect, the content rect inside the padding, and the
/// colours of its state.
#[derive(Debug, Clone, Copy)]
pub struct RowCtx {
    pub rect: Rect,
    /// `rect` less the horizontal padding.
    pub content: Rect,
    /// Hover, 0 to 1.
    pub hover: f32,
    pub selected: bool,
    /// The opaque background colour (for rings drawn on the row).
    pub ground: Color32,
    /// Label colour.
    pub text: Color32,
    /// Secondary text colour.
    pub dim: Color32,
    /// Icon colour: `text_faint` at rest, `text` on hover, `accent` when selected.
    pub icon: Color32,
    /// SHA and other accentuated trailing text: `text_faint`, `accent` when selected.
    pub accent: Color32,
}

/// Horizontal padding inside a row.
const PAD: f32 = 10.0;
/// Gap between a row's icon, label, detail and trailing text.
const GAP: f32 = 8.0;

/// Everything rows share up to their content: allocation, interaction, the ground layers.
struct Frame {
    rect: Rect,
    response: Response,
    painter: Painter,
    ctx: RowCtx,
    radius: CornerRadius,
    cursor: bool,
    enabled: bool,
}

struct FrameSpec {
    id_salt: Id,
    kind: RowKind,
    height: f32,
    selected: bool,
    cursor: bool,
    zebra: bool,
    enabled: bool,
    joined: (bool, bool),
    ground: Option<Color32>,
    force: Option<ForceState>,
}

fn frame(ui: &mut Ui, spec: &FrameSpec) -> Frame {
    let c = ui.ctx().clone();
    let p = theme::palette(&c);
    let m = theme::metrics();
    let enabled = spec.enabled && ui.is_enabled();
    let mut width = ui.available_width();
    if !width.is_finite() {
        width = theme::metrics().sidebar_width;
    }
    let rect = ui.allocate_space(vec2(width, spec.height)).1;
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, ui.id().with(spec.id_salt), sense);
    let f = spec.force.unwrap_or_default();
    // The pointer anywhere over the row (its trailing actions included) hovers it.
    let hovered = enabled && (f.hover || ui.rect_contains_pointer(rect));
    let pressed = enabled && (f.pressed || response.is_pointer_button_down_on());
    let hover = match spec.force {
        Some(f) => f32::from(u8::from(hovered)) * f.progress.unwrap_or(1.0),
        None => anim::ease_bool(
            &c,
            response.id.with("kit-row-hover"),
            hovered,
            motion::FAST,
            motion::FAST,
        ),
    };
    let cursor = spec.cursor || f.focus || (response.has_focus() && focus_visible(&c));
    let base = spec.ground.unwrap_or_else(|| spec.kind.ground(p));
    let strong = spec.kind.chrome();
    let ground = ground(p, base, spec.zebra, spec.selected, hover, pressed, strong);
    let r = if spec.kind.rounded() { m.radius_sm } else { 0 };
    let radius = CornerRadius {
        nw: if spec.joined.0 { 0 } else { r },
        ne: if spec.joined.0 { 0 } else { r },
        sw: if spec.joined.1 { 0 } else { r },
        se: if spec.joined.1 { 0 } else { r },
    };
    let mut painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    if !enabled && ui.is_enabled() {
        painter.multiply_opacity(theme::DISABLED_ALPHA);
    }
    if ui.is_rect_visible(rect) {
        if spec.zebra {
            painter.rect_filled(rect, radius, p.zebra());
        }
        if spec.selected {
            painter.rect_filled(rect, radius, p.selected());
        }
        if hover > 0.0 {
            let h = if strong { p.hover_strong() } else { p.hover() };
            painter.rect_filled(rect, radius, h.gamma_multiply(hover));
        }
        if pressed {
            painter.rect_filled(rect, radius, p.pressed());
        }
    }
    let lit = hover.max(f32::from(u8::from(spec.selected)));
    let rest_text = if strong { p.text_dim } else { p.text };
    let ctx = RowCtx {
        rect,
        content: rect.shrink2(vec2(PAD, 0.0)),
        hover,
        selected: spec.selected,
        ground,
        text: rest_text.lerp_to_gamma(p.text, lit),
        dim: p.text_dim,
        icon: if spec.selected {
            p.accent
        } else {
            p.text_faint.lerp_to_gamma(p.text, hover)
        },
        accent: if spec.selected {
            p.accent
        } else {
            p.text_faint
        },
    };
    Frame {
        rect,
        response,
        painter,
        ctx,
        radius,
        cursor,
        enabled,
    }
}

impl Frame {
    /// The keyboard cursor ring on an unselected row, drawn over the content.
    fn finish(&self) {
        if self.cursor && !self.ctx.selected {
            let p = theme::palette(self.painter.ctx());
            paint::inset_ring(&self.painter, self.rect, self.radius, p.focus);
        }
    }
}

/// Paints `text` on one line from `x`, ellipsized at `right`; returns where it ends.
fn paint_text(
    painter: &Painter,
    x: f32,
    right: f32,
    cy: f32,
    text: &str,
    font: FontId,
    color: Color32,
) -> f32 {
    if right <= x {
        return x;
    }
    let mut job = LayoutJob::simple(text.to_owned(), font, color, right - x);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let g = painter.layout_job(job);
    let w = g.size().x;
    painter.galley(pos2(x, cy - g.size().y / 2.0), g, color);
    x + w
}

fn row_widget_info(
    ui: &Ui,
    response: &Response,
    enabled: bool,
    selected: bool,
    label: &str,
    role: egui::accesskit::Role,
) {
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::SelectableLabel, enabled, selected, label)
    });
    // A list or tree item is selected, not toggled.
    ui.ctx().accesskit_node_builder(response.id, |n| {
        n.set_role(role);
        n.clear_toggled();
        n.set_selected(selected);
    });
}

fn with_tip(
    response: Response,
    enabled: bool,
    tooltip: Option<&str>,
    reason: Option<&str>,
) -> Response {
    let tip = if enabled { tooltip } else { reason.or(tooltip) };
    match tip {
        Some(t) => response.on_hover_text(t).on_disabled_hover_text(t),
        None => response,
    }
}

// ------------------------------------------------------------------------------------ ListRow

/// A row's hover-revealed actions.
type Actions<'a> = Box<dyn FnOnce(&mut Ui) + 'a>;

/// A row of a list or table. Content comes from slots (`icon`, `label`, `detail`, `trailing`)
/// or a painter closure ([`ListRow::show_with`]); hover-revealed actions from
/// [`ListRow::actions`].
#[must_use = "call .show(ui), .show_with(ui, ..) or ui.add(..)"]
pub struct ListRow<'a> {
    id_salt: Id,
    kind: RowKind,
    height: Option<f32>,
    selected: bool,
    cursor: bool,
    zebra: bool,
    enabled: bool,
    joined: (bool, bool),
    ground: Option<Color32>,
    icon: Option<String>,
    label: String,
    detail: Option<String>,
    trailing: Option<String>,
    indent: f32,
    tooltip: Option<String>,
    disabled_reason: Option<String>,
    actions: Option<Actions<'a>>,
    force: Option<ForceState>,
}

impl std::fmt::Debug for ListRow<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListRow")
            .field("id_salt", &self.id_salt)
            .field("kind", &self.kind)
            .field("label", &self.label)
            .field("selected", &self.selected)
            .finish_non_exhaustive()
    }
}

impl<'a> ListRow<'a> {
    pub fn new(id_salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        Self {
            id_salt: Id::new(id_salt),
            kind: RowKind::List,
            height: None,
            selected: false,
            cursor: false,
            zebra: false,
            enabled: true,
            joined: (false, false),
            ground: None,
            icon: None,
            label: String::new(),
            detail: None,
            trailing: None,
            indent: 0.0,
            tooltip: None,
            disabled_reason: None,
            actions: None,
            force: None,
        }
    }

    /// The list the row belongs to: height, ground and label treatment.
    pub fn kind(mut self, kind: RowKind) -> Self {
        self.kind = kind;
        self
    }
    /// The spec's name for [`ListRow::kind`].
    pub fn height_from(self, kind: RowKind) -> Self {
        self.kind(kind)
    }
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
    /// The keyboard cursor is on this row (views with their own cursor model).
    pub fn cursor(mut self, cursor: bool) -> Self {
        self.cursor = cursor;
        self
    }
    /// The row's position in its table, for Dense's zebra stripes.
    pub fn index(mut self, index: usize) -> Self {
        self.zebra = self.kind == RowKind::History && zebra(index);
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    /// The rows above and below are selected too: square the shared corners so the selection
    /// reads as one block.
    pub fn joined(mut self, above: bool, below: bool) -> Self {
        self.joined = (above, below);
        self
    }
    /// The surface under the row when it is not the kind's usual one.
    pub fn ground(mut self, ground: Color32) -> Self {
        self.ground = Some(ground);
        self
    }
    pub fn icon(mut self, glyph: &str) -> Self {
        self.icon = Some(glyph.to_owned());
        self
    }
    /// The label, also the row's AccessKit name.
    pub fn label(mut self, text: impl Into<String>) -> Self {
        self.label = text.into();
        self
    }
    pub fn detail(mut self, text: impl Into<String>) -> Self {
        self.detail = Some(text.into());
        self
    }
    /// Right-aligned mono text (a count, a SHA): `text_faint`, `accent` when selected.
    pub fn trailing(mut self, text: impl Into<String>) -> Self {
        self.trailing = Some(text.into());
        self
    }
    /// Extra left indent for the content.
    pub fn indent(mut self, indent: f32) -> Self {
        self.indent = indent;
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
    /// Actions shown at the right edge while the row is hovered, selected or focused (they fade
    /// in over `FAST`). Add `IconButton::size(Size::Sm)`s right to left.
    pub fn actions(mut self, add: impl FnOnce(&mut Ui) + 'a) -> Self {
        self.actions = Some(Box::new(add));
        self
    }
    pub(crate) fn show_forced(mut self, ui: &mut Ui, force: ForceState) -> Response {
        self.force = Some(force);
        self.show(ui)
    }

    fn spec(&self) -> FrameSpec {
        FrameSpec {
            id_salt: self.id_salt,
            kind: self.kind,
            height: self.height.unwrap_or_else(|| self.kind.height()),
            selected: self.selected,
            cursor: self.cursor,
            zebra: self.zebra,
            enabled: self.enabled,
            joined: self.joined,
            ground: self.ground,
            force: self.force,
        }
    }

    /// Shows the row with its slots.
    pub fn show(self, ui: &mut Ui) -> Response {
        self.show_with(ui, |_, _| {})
    }

    /// Shows the row, then `paint` draws custom content (after the slots, under the cursor ring).
    pub fn show_with(mut self, ui: &mut Ui, paint: impl FnOnce(&RowCtx, &Painter)) -> Response {
        let m = theme::metrics();
        let f = frame(ui, &self.spec());
        let rc = f.ctx;
        let cy = rc.rect.center().y;
        let actions = self.actions.take();
        let actions_w = if actions.is_some() {
            (m.control_sm + 2.0) * 2.0
        } else {
            0.0
        };
        if ui.is_rect_visible(f.rect) {
            let mut x = rc.content.left() + self.indent;
            let mut right = rc.content.right() - actions_w;
            if let Some(g) = &self.icon {
                let glyph =
                    f.painter
                        .layout_no_wrap(g.clone(), FontId::proportional(m.icon_md), rc.icon);
                f.painter.galley(
                    pos2(x, cy - glyph.size().y / 2.0),
                    Arc::clone(&glyph),
                    rc.icon,
                );
                x += glyph.size().x + GAP;
            }
            if let Some(t) = &self.trailing {
                let g = f.painter.layout_no_wrap(
                    t.clone(),
                    FontId::new(m.sha_size, FontFamily::Monospace),
                    rc.accent,
                );
                right -= g.size().x;
                f.painter
                    .galley(pos2(right, cy - g.size().y / 2.0), g, rc.accent);
                right -= GAP;
            }
            let family = match (self.selected, self.kind.chrome()) {
                (true, true) => theme::semibold(),
                (true, false) => theme::medium(),
                _ => FontFamily::Proportional,
            };
            let end = paint_text(
                &f.painter,
                x,
                right,
                cy,
                &self.label,
                FontId::new(m.body_size, family),
                rc.text,
            );
            if let Some(d) = &self.detail {
                paint_text(
                    &f.painter,
                    end + GAP,
                    right,
                    cy,
                    d,
                    FontId::proportional(m.meta_size),
                    rc.dim,
                );
            }
            paint(&rc, &f.painter);
        }
        f.finish();
        if let Some(add) = actions {
            let shown = f.enabled
                && (rc.hover > 0.0 || self.selected || f.response.has_focus() || f.cursor);
            let reveal = anim::ease_bool(
                ui.ctx(),
                f.response.id.with("kit-row-actions"),
                shown,
                motion::FAST,
                motion::FAST,
            );
            if reveal > 0.0 {
                let area = Rect::from_min_max(
                    pos2(rc.content.right() - actions_w, f.rect.top()),
                    pos2(rc.content.right(), f.rect.bottom()),
                );
                let mut child = ui.new_child(
                    UiBuilder::new()
                        .max_rect(area)
                        .layout(Layout::right_to_left(Align::Center)),
                );
                child.multiply_opacity(reveal);
                child.spacing_mut().item_spacing.x = 2.0;
                add(&mut child);
            }
        }
        row_widget_info(
            ui,
            &f.response,
            f.enabled,
            self.selected,
            &self.label,
            egui::accesskit::Role::ListItem,
        );
        with_tip(
            f.response,
            f.enabled,
            self.tooltip.as_deref(),
            self.disabled_reason.as_deref(),
        )
    }
}

impl Widget for ListRow<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

// ------------------------------------------------------------------------------------ TreeRow

/// Where the keyboard asked to move from a tree row: Left on a closed row goes to its parent,
/// Right on an open one into its first child. The view moves its cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeNav {
    Parent,
    FirstChild,
}

/// A row in a tree: a [`ListRow`] with an indent per level and a disclosure chevron that turns
/// over `BASE` (space is kept for it on leaves). Left collapses or asks for the parent; Right
/// expands or asks for the first child.
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct TreeRow<'a> {
    id_salt: Id,
    depth: usize,
    open: Option<&'a mut bool>,
    kind: RowKind,
    icon: Option<String>,
    label: String,
    trailing: Option<String>,
    selected: bool,
    cursor: bool,
    enabled: bool,
    toggle_on_click: bool,
    force: Option<ForceState>,
}

impl std::fmt::Debug for TreeRow<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeRow")
            .field("id_salt", &self.id_salt)
            .field("depth", &self.depth)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl<'a> TreeRow<'a> {
    pub fn new(id_salt: impl std::hash::Hash + std::fmt::Debug, depth: usize) -> Self {
        Self {
            id_salt: Id::new(id_salt),
            depth,
            open: None,
            kind: RowKind::Sidebar,
            icon: None,
            label: String::new(),
            trailing: None,
            selected: false,
            cursor: false,
            enabled: true,
            toggle_on_click: false,
            force: None,
        }
    }
    /// The row has children, shown while `open`.
    pub fn open(mut self, open: &'a mut bool) -> Self {
        self.open = Some(open);
        self
    }
    pub fn kind(mut self, kind: RowKind) -> Self {
        self.kind = kind;
        self
    }
    pub fn icon(mut self, glyph: &str) -> Self {
        self.icon = Some(glyph.to_owned());
        self
    }
    pub fn label(mut self, text: impl Into<String>) -> Self {
        self.label = text.into();
        self
    }
    pub fn trailing(mut self, text: impl Into<String>) -> Self {
        self.trailing = Some(text.into());
        self
    }
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
    pub fn cursor(mut self, cursor: bool) -> Self {
        self.cursor = cursor;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    /// A click anywhere on the row opens or closes it (disclosures, group rows).
    pub fn toggle_on_click(mut self, on: bool) -> Self {
        self.toggle_on_click = on;
        self
    }
    pub(crate) fn show_forced(
        mut self,
        ui: &mut Ui,
        force: ForceState,
    ) -> KitResponse<Option<TreeNav>> {
        self.force = Some(force);
        self.show(ui)
    }

    pub fn show(self, ui: &mut Ui) -> KitResponse<Option<TreeNav>> {
        let m = theme::metrics();
        let p = theme::palette(ui.ctx());
        let spec = FrameSpec {
            id_salt: self.id_salt,
            kind: self.kind,
            height: self.kind.height(),
            selected: self.selected,
            cursor: self.cursor,
            zebra: false,
            enabled: self.enabled,
            joined: (false, false),
            ground: None,
            force: self.force,
        };
        let f = frame(ui, &spec);
        let rc = f.ctx;
        let id = f.response.id;
        let has_children = self.open.is_some();
        let mut open_now = self.open.as_deref().copied().unwrap_or(false);
        let chevron_x = f.rect.left() + 6.0 + crate::ui::count_f32(self.depth) * m.tree_indent;
        let chevron = Rect::from_center_size(
            pos2(chevron_x + 6.0, f.rect.center().y),
            vec2(16.0, f.rect.height()),
        );
        let mut toggled = false;
        if has_children && f.enabled {
            let hit = ui.interact(chevron, id.with("kit-tree-chevron"), Sense::click());
            if hit.clicked() {
                toggled = true;
            }
        }
        if has_children
            && f.enabled
            && ((self.toggle_on_click && f.response.clicked()) || f.response.double_clicked())
        {
            toggled = true;
        }
        let mut nav = None;
        if f.response.has_focus() && f.enabled {
            ui.memory_mut(|mem| {
                mem.set_focus_lock_filter(
                    id,
                    egui::EventFilter {
                        horizontal_arrows: true,
                        ..Default::default()
                    },
                );
            });
            let (right, left) = ui.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::ArrowRight),
                    i.consume_key(Modifiers::NONE, Key::ArrowLeft),
                )
            });
            if right {
                if has_children && !open_now {
                    toggled = true;
                } else if has_children {
                    nav = Some(TreeNav::FirstChild);
                }
            }
            if left {
                if has_children && open_now {
                    toggled = true;
                } else {
                    nav = Some(TreeNav::Parent);
                }
            }
        }
        if toggled {
            open_now = !open_now;
        }
        if let Some(open) = self.open {
            *open = open_now;
        }
        let turn = match self.force.and_then(|f| f.progress) {
            Some(t) if has_children => {
                if open_now {
                    t
                } else {
                    1.0 - t
                }
            }
            _ => anim::ease_bool(
                ui.ctx(),
                id.with("kit-tree-turn"),
                open_now,
                motion::BASE,
                motion::BASE,
            ),
        };
        if ui.is_rect_visible(f.rect) {
            let cy = f.rect.center().y;
            if has_children {
                let color = p.text_faint.lerp_to_gamma(p.text_dim, rc.hover);
                paint_chevron(&f.painter, chevron.center(), turn, color);
            }
            let mut x = chevron_x + 12.0 + 4.0;
            if let Some(g) = &self.icon {
                let glyph =
                    f.painter
                        .layout_no_wrap(g.clone(), FontId::proportional(m.icon_md), rc.icon);
                f.painter.galley(
                    pos2(x, cy - glyph.size().y / 2.0),
                    Arc::clone(&glyph),
                    rc.icon,
                );
                x += glyph.size().x + GAP;
            }
            let mut right = rc.content.right();
            if let Some(t) = &self.trailing {
                let g = f.painter.layout_no_wrap(
                    t.clone(),
                    FontId::new(m.micro_size, FontFamily::Monospace),
                    p.text_faint,
                );
                right -= g.size().x;
                f.painter
                    .galley(pos2(right, cy - g.size().y / 2.0), g, p.text_faint);
                right -= GAP;
            }
            let family = match (self.selected, self.kind.chrome()) {
                (true, true) => theme::semibold(),
                (true, false) => theme::medium(),
                _ => FontFamily::Proportional,
            };
            paint_text(
                &f.painter,
                x,
                right,
                cy,
                &self.label,
                FontId::new(m.body_size, family),
                rc.text,
            );
        }
        f.finish();
        let mut response = f.response;
        if toggled {
            response.mark_changed();
        }
        row_widget_info(
            ui,
            &response,
            f.enabled,
            self.selected,
            &self.label,
            egui::accesskit::Role::TreeItem,
        );
        if has_children {
            ui.ctx()
                .accesskit_node_builder(response.id, |n| n.set_expanded(open_now));
        }
        KitResponse {
            response,
            value: nav,
        }
    }
}

/// The disclosure chevron (`CARET_RIGHT`, 12 pt) at `center`, turned by `turn` quarter turns.
fn paint_chevron(painter: &Painter, center: egui::Pos2, turn: f32, color: Color32) {
    let g = painter.layout_no_wrap(
        icon::CARET_RIGHT.to_owned(),
        FontId::proportional(12.0),
        color,
    );
    let pos = center - g.size() / 2.0;
    let shape = egui::epaint::TextShape::new(pos, g, color)
        .with_angle_and_anchor(turn * std::f32::consts::FRAC_PI_2, Align2::CENTER_CENTER);
    painter.add(Shape::Text(shape));
}

/// The children of an open tree row (or disclosure): they grow open and shut over `BASE`,
/// except lists of more than 40 and with reduced motion, which snap. Returns what `add`
/// returned while any of them shows.
pub fn tree_children<R>(
    ui: &mut Ui,
    id: Id,
    open: bool,
    count: usize,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let openness = if count > 40 {
        f32::from(u8::from(open))
    } else {
        anim::ease_bool(
            ui.ctx(),
            id.with("kit-tree-body"),
            open,
            motion::BASE,
            motion::BASE,
        )
    };
    if openness <= 0.0 {
        return None;
    }
    let key = id.with("kit-tree-body-height");
    let full: Option<f32> = ui.ctx().data(|d| d.get_temp(key));
    let top = ui.cursor().top();
    let width = ui.available_width();
    let shown_h = match full {
        Some(h) if openness < 1.0 => h * openness,
        _ => f32::INFINITY,
    };
    let max_rect = Rect::from_min_size(pos2(ui.cursor().left(), top), vec2(width, f32::INFINITY));
    let mut child = ui.new_child(
        UiBuilder::new()
            .id_salt(id.with("kit-tree-body-ui"))
            .max_rect(max_rect)
            .layout(Layout::top_down(Align::Min)),
    );
    if shown_h.is_finite() {
        let clip = Rect::from_min_size(
            pos2(ui.max_rect().left(), top),
            vec2(ui.max_rect().width(), shown_h),
        );
        child.set_clip_rect(clip.intersect(ui.clip_rect()));
    }
    let r = add(&mut child);
    let measured = child.min_rect().height();
    ui.ctx().data_mut(|d| d.insert_temp(key, measured));
    let height = if openness < 1.0 {
        measured * openness
    } else {
        measured
    };
    ui.allocate_rect(
        Rect::from_min_size(
            pos2(max_rect.left(), top),
            vec2(child.min_rect().width(), height),
        ),
        Sense::hover(),
    );
    Some(r)
}

// --------------------------------------------------------------------------------- Disclosure

/// A titled section that opens and closes (the commit information block, toast and SSH details,
/// CI checks): a [`TreeRow`] header that toggles on click, then its body.
#[must_use = "call .show(ui, ..)"]
pub struct Disclosure<'a> {
    id_salt: Id,
    label: String,
    open: &'a mut bool,
    icon: Option<String>,
}

impl std::fmt::Debug for Disclosure<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Disclosure")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl<'a> Disclosure<'a> {
    pub fn new(
        id_salt: impl std::hash::Hash + std::fmt::Debug,
        label: impl Into<String>,
        open: &'a mut bool,
    ) -> Self {
        Self {
            id_salt: Id::new(id_salt),
            label: label.into(),
            open,
            icon: None,
        }
    }
    pub fn icon(mut self, glyph: &str) -> Self {
        self.icon = Some(glyph.to_owned());
        self
    }
    pub fn show<R>(self, ui: &mut Ui, body: impl FnOnce(&mut Ui) -> R) -> KitResponse<Option<R>> {
        let id = ui.id().with(self.id_salt);
        let mut open = *self.open;
        let mut row = TreeRow::new(self.id_salt, 0)
            .kind(RowKind::List)
            .label(self.label.clone())
            .toggle_on_click(true)
            .open(&mut open);
        if let Some(g) = &self.icon {
            row = row.icon(g);
        }
        let r = row.show(ui);
        *self.open = open;
        // A disclosure is a collapsing header, not an item of a tree.
        r.response
            .widget_info(|| WidgetInfo::labeled(WidgetType::CollapsingHeader, true, &self.label));
        ui.ctx().accesskit_node_builder(r.response.id, |n| {
            n.clear_selected();
            n.set_expanded(open);
        });
        let value = tree_children(ui, id, open, 0, |ui| {
            ui.add_space(theme::XS);
            let r = ui
                .horizontal(|ui| {
                    ui.add_space(6.0 + 12.0 + 4.0);
                    ui.vertical(body).inner
                })
                .inner;
            ui.add_space(theme::XS);
            r
        });
        KitResponse {
            response: r.response,
            value,
        }
    }
}

// ------------------------------------------------------------------------------ SectionHeader

/// A sidebar section's header: an uppercase label with an optional glyph, chevron, count and an
/// action that shows on hover. Click (or Enter) opens and closes a collapsible one.
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct SectionHeader<'a> {
    title: String,
    icon: Option<String>,
    count: Option<usize>,
    open: Option<&'a mut bool>,
    action: Option<(String, String)>,
    first: bool,
    force: Option<ForceState>,
}

impl std::fmt::Debug for SectionHeader<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SectionHeader")
            .field("title", &self.title)
            .finish_non_exhaustive()
    }
}

impl<'a> SectionHeader<'a> {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            icon: None,
            count: None,
            open: None,
            action: None,
            first: false,
            force: None,
        }
    }
    pub fn icon(mut self, glyph: &str) -> Self {
        self.icon = Some(glyph.to_owned());
        self
    }
    pub fn count(mut self, n: usize) -> Self {
        self.count = Some(n);
        self
    }
    /// Makes the section collapsible.
    pub fn open(mut self, open: &'a mut bool) -> Self {
        self.open = Some(open);
        self
    }
    /// An icon button at the right that shows on hover ("+" Add remote…); `label` is its tooltip.
    pub fn action(mut self, glyph: &str, label: impl Into<String>) -> Self {
        self.action = Some((glyph.to_owned(), label.into()));
        self
    }
    /// The first section: no gap above it.
    pub fn first(mut self, first: bool) -> Self {
        self.first = first;
        self
    }
    pub(crate) fn show_forced(mut self, ui: &mut Ui, force: ForceState) -> KitResponse<bool> {
        self.force = Some(force);
        self.show(ui)
    }

    /// Shows the header; the value is whether its action was clicked.
    pub fn show(self, ui: &mut Ui) -> KitResponse<bool> {
        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let m = theme::metrics();
        if !self.first {
            ui.add_space(m.section_gap);
        }
        let width = ui.available_width();
        let rect = ui.allocate_space(vec2(width, m.section_header_height)).1;
        let collapsible = self.open.is_some();
        let sense = if collapsible {
            Sense::click()
        } else {
            Sense::hover()
        };
        let mut response = ui.interact(rect, ui.id().with(("kit-section", &self.title)), sense);
        let f = self.force.unwrap_or_default();
        let hovered = f.hover || ui.rect_contains_pointer(rect);
        let hover = match self.force {
            Some(f) => f32::from(u8::from(hovered)) * f.progress.unwrap_or(1.0),
            None => anim::ease_bool(
                &ctx,
                response.id.with("kit-section-hover"),
                hovered,
                motion::FAST,
                motion::FAST_OUT,
            ),
        };
        let mut open_now = self.open.as_deref().copied().unwrap_or(true);
        if collapsible && response.clicked() {
            open_now = !open_now;
            response.mark_changed();
        }
        if let Some(open) = self.open {
            *open = open_now;
        }
        let focused = f.focus || (response.has_focus() && focus_visible(&ctx));
        let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        let cy = rect.center().y;
        let label_color = p.text_faint.lerp_to_gamma(p.text_dim, hover);
        let mut x = rect.left() + 6.0;
        if collapsible {
            let turn = anim::ease_bool(
                &ctx,
                response.id.with("kit-section-turn"),
                open_now,
                motion::BASE,
                motion::BASE,
            );
            paint_chevron(&painter, pos2(x + 6.0, cy), turn, label_color);
            x += 12.0 + 6.0;
        }
        if let Some(g) = &self.icon {
            let glyph = painter.layout_no_wrap(g.clone(), FontId::proportional(13.0), p.text_faint);
            painter.galley(
                pos2(x, cy - glyph.size().y / 2.0),
                Arc::clone(&glyph),
                p.text_faint,
            );
            x += glyph.size().x + 6.0;
        }
        let title = caps_galley(ui, &self.title, label_color);
        painter.galley(pos2(x, cy - title.size().y / 2.0), title, label_color);
        let action_w = if self.action.is_some() { 22.0 } else { 0.0 };
        if let Some(n) = self.count {
            let g = painter.layout_no_wrap(
                n.to_string(),
                FontId::new(m.micro_size, FontFamily::Monospace),
                p.text_faint,
            );
            let right = rect.right() - 6.0 - action_w;
            painter.galley(
                pos2(right - g.size().x, cy - g.size().y / 2.0),
                g,
                p.text_faint,
            );
        }
        if focused {
            paint::inset_ring(&painter, rect, CornerRadius::same(m.radius_sm), p.focus);
        }
        let mut acted = false;
        if let Some((glyph, label)) = &self.action {
            let reveal = match self.force {
                Some(_) => f32::from(u8::from(hovered || focused)),
                None => anim::ease_bool(
                    &ctx,
                    response.id.with("kit-section-action"),
                    hovered || focused,
                    motion::FAST,
                    motion::FAST,
                ),
            };
            if reveal > 0.0 {
                let area = Rect::from_min_max(
                    pos2(rect.right() - action_w - 2.0, rect.top()),
                    pos2(rect.right() - 2.0, rect.bottom()),
                );
                let mut child = ui.new_child(
                    UiBuilder::new()
                        .max_rect(area)
                        .layout(Layout::right_to_left(Align::Center)),
                );
                child.multiply_opacity(reveal);
                acted = IconButton::new(glyph, label.clone())
                    .size(Size::Xs)
                    .id_salt(("kit-section-action", &self.title))
                    .show(&mut child)
                    .clicked();
            }
        }
        if collapsible {
            response.widget_info(|| {
                WidgetInfo::labeled(WidgetType::CollapsingHeader, true, &self.title)
            });
            ctx.accesskit_node_builder(response.id, |n| n.set_expanded(open_now));
        } else {
            response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &self.title));
        }
        KitResponse {
            response,
            value: acted,
        }
    }
}

/// An uppercase label at `micro_size` semibold with 0.07 em tracking (section headers, table
/// column headers).
pub fn caps_galley(ui: &Ui, text: &str, color: Color32) -> Arc<egui::Galley> {
    let m = theme::metrics();
    let mut job = LayoutJob::default();
    job.append(
        &text.to_uppercase(),
        0.0,
        TextFormat {
            font_id: FontId::new(m.micro_size, theme::semibold()),
            color,
            extra_letter_spacing: m.micro_size * 0.07,
            ..Default::default()
        },
    );
    ui.painter().layout_job(job)
}

// ---------------------------------------------------------------------------------- Selection

/// A multi-selection of rows by index with its anchor: a click selects one row, Ctrl toggles
/// one, Shift selects the range from the anchor (Ctrl+Shift adds it).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    pub rows: BTreeSet<usize>,
    pub anchor: Option<usize>,
}

impl Selection {
    /// Applies a click on row `index` with `modifiers`.
    pub fn click(&mut self, index: usize, modifiers: Modifiers) {
        let ctrl = modifiers.command || modifiers.ctrl;
        match (ctrl, modifiers.shift, self.anchor) {
            (_, true, Some(anchor)) => {
                let range = anchor.min(index)..=anchor.max(index);
                if !ctrl {
                    self.rows.clear();
                }
                self.rows.extend(range);
            }
            (true, _, _) => {
                if !self.rows.remove(&index) {
                    self.rows.insert(index);
                }
                self.anchor = Some(index);
            }
            _ => {
                self.rows.clear();
                self.rows.insert(index);
                self.anchor = Some(index);
            }
        }
    }

    pub fn contains(&self, index: usize) -> bool {
        self.rows.contains(&index)
    }

    /// Whether the rows above and below `index` are selected too (for [`ListRow::joined`]).
    pub fn neighbours(&self, index: usize) -> (bool, bool) {
        (
            index > 0 && self.rows.contains(&(index - 1)),
            self.rows.contains(&(index + 1)),
        )
    }
}

// ------------------------------------------------------------------------------------ gallery

/// The gallery's Lists section: every `ListRow` state, tree rows open and closed at three depths, a
/// disclosure and section headers.
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
    ui.spacing_mut().item_spacing.y = 2.0;
    small(
        ui,
        "List rows: rest, hover, selected, selected + hover, cursor, pressed, disabled",
    );
    let states: [(&str, Option<ForceState>, bool, bool, bool); 7] = [
        ("Rest", None, false, false, true),
        ("Hover", Some(ForceState::HOVER), false, false, true),
        ("Selected", None, true, false, true),
        (
            "Selected and hovered",
            Some(ForceState::HOVER),
            true,
            false,
            true,
        ),
        ("Keyboard cursor", None, false, true, true),
        ("Pressed", Some(ForceState::PRESSED), false, false, true),
        ("Disabled", None, false, false, false),
    ];
    for (i, (name, force, selected, cursor, enabled)) in states.into_iter().enumerate() {
        let row = ListRow::new(("gallery-row", i))
            .kind(RowKind::File)
            .icon(icon::FILE_TEXT)
            .label(name)
            .detail("src/ui")
            .trailing("+4 −1")
            .selected(selected)
            .cursor(cursor)
            .enabled(enabled)
            .disabled_reason("Not available");
        match force {
            Some(f) => row.show_forced(ui, f),
            None => row.show(ui),
        };
    }
    small(
        ui,
        "Palette row (indented, tooltip), request row on the chrome ground",
    );
    ListRow::new("gallery-row-palette")
        .height_from(RowKind::Palette)
        .icon(icon::ARROWS_CLOCKWISE)
        .label("Fetch all remotes")
        .indent(theme::XS)
        .tooltip("Runs git fetch --all")
        .show(ui);
    ListRow::new("gallery-row-request")
        .kind(RowKind::Request)
        .ground(theme::surfaces(ui.ctx()).panel)
        .label("Add dark mode")
        .detail("#42 · feature/dark-mode")
        .show_with(ui, |rc, painter| {
            // Custom content gets the row's opaque ground for rings drawn on it.
            let c = pos2(rc.content.right() - 12.0, rc.rect.center().y);
            painter.circle_filled(c, 7.0, rc.ground);
            painter.circle_filled(c, 5.0, rc.accent);
        });
    small(ui, "With hover actions (revealed on the selected row)");
    ListRow::new("gallery-row-actions")
        .kind(RowKind::File)
        .icon(icon::FILE_CODE)
        .label("main.rs")
        .detail("src")
        .selected(true)
        .actions(|ui| {
            IconButton::new(icon::MINUS, "Unstage")
                .size(Size::Sm)
                .id_salt("gallery-row-unstage")
                .show(ui);
            IconButton::new(icon::ARROW_COUNTER_CLOCKWISE, "Discard")
                .size(Size::Sm)
                .id_salt("gallery-row-discard")
                .show(ui);
        })
        .show(ui);
    small(
        ui,
        "History rows (zebra on even rows in Dense), joined selection",
    );
    let mut sel = Selection::default();
    sel.click(1, Modifiers::NONE);
    sel.click(2, Modifiers::SHIFT);
    ui.spacing_mut().item_spacing.y = 0.0;
    for i in 0..4 {
        let (above, below) = sel.neighbours(i);
        ListRow::new(("gallery-history", i))
            .kind(RowKind::History)
            .index(i)
            .label(format!("feat(ui): commit number {i}"))
            .trailing(format!(
                "{:07x}",
                0x00ab_cdef_u32.wrapping_mul(u32::try_from(i).map_or(0, |i| i + 3))
            ))
            .selected(sel.contains(i))
            .joined(above, below)
            .show(ui);
    }
    ui.spacing_mut().item_spacing.y = 2.0;
    small(
        ui,
        "Tree rows: open and closed at depth 0 to 2, mid-turn chevron",
    );
    let id = ui.id().with("gallery-tree");
    let mut state: [bool; 2] = ui.ctx().data(|d| d.get_temp(id)).unwrap_or([true, false]);
    let [mut origin, mut upstream] = state;
    TreeRow::new("gallery-tree-origin", 0)
        .icon(icon::CLOUD)
        .label("origin")
        .trailing("3")
        .open(&mut origin)
        .show(ui);
    tree_children(ui, id.with("origin"), origin, 3, |ui| {
        TreeRow::new("gallery-tree-main", 1)
            .icon(icon::GIT_BRANCH)
            .label("main")
            .selected(true)
            .show(ui);
        TreeRow::new("gallery-tree-feature", 1)
            .icon(icon::FOLDER_SIMPLE)
            .label("feature")
            .open(&mut upstream)
            .show(ui);
        TreeRow::new("gallery-tree-leaf", 2)
            .icon(icon::GIT_BRANCH)
            .label("feature/dark-mode")
            .show_forced(ui, ForceState::HOVER);
    });
    TreeRow::new("gallery-tree-cursor", 1)
        .icon(icon::GIT_BRANCH)
        .label("keyboard cursor")
        .cursor(true)
        .show(ui);
    TreeRow::new("gallery-tree-disabled", 1)
        .icon(icon::GIT_BRANCH)
        .label("disabled")
        .enabled(false)
        .show(ui);
    TreeRow::new("gallery-tree-mid", 0)
        .icon(icon::CLOUD)
        .label("upstream (chevron at 0.5)")
        .open(&mut true)
        .show_forced(
            ui,
            ForceState {
                progress: Some(0.5),
                ..ForceState::default()
            },
        );
    state = [origin, upstream];
    ui.ctx().data_mut(|d| d.insert_temp(id, state));
    small(ui, "Disclosure");
    let did = ui.id().with("gallery-disclosure");
    let mut open: bool = ui.ctx().data(|d| d.get_temp(did)).unwrap_or(true);
    Disclosure::new("gallery-disclosure", "Commit information", &mut open)
        .icon(icon::INFO)
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new("Author, committer and parents.")
                    .size(m.meta_size)
                    .color(p.text_dim),
            );
        });
    ui.ctx().data_mut(|d| d.insert_temp(did, open));
    small(
        ui,
        "Section headers: rest, hover with action, focus, static",
    );
    let hid = ui.id().with("gallery-sections");
    let mut opens: [bool; 3] = ui.ctx().data(|d| d.get_temp(hid)).unwrap_or([true; 3]);
    SectionHeader::new("Branches")
        .icon(icon::GIT_BRANCH)
        .count(6)
        .open(&mut opens[0])
        .first(true)
        .show(ui);
    SectionHeader::new("Remotes")
        .icon(icon::CLOUD)
        .count(1)
        .open(&mut opens[1])
        .action(icon::PLUS, "Add remote…")
        .show_forced(ui, ForceState::HOVER);
    SectionHeader::new("Tags")
        .icon(icon::TAG)
        .count(3)
        .open(&mut opens[2])
        .show_forced(ui, ForceState::FOCUS);
    SectionHeader::new("Stashes")
        .icon(icon::ARCHIVE)
        .count(1)
        .show(ui);
    ui.ctx().data_mut(|d| d.insert_temp(hid, opens));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::{Density, Style};

    fn ctx() -> egui::Context {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        ctx
    }

    fn key(key: Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    fn run(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        mut f: impl FnMut(&mut Ui),
    ) -> egui::FullOutput {
        let mut out = ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| f(ui),
        );
        out.textures_delta.clear();
        out
    }

    #[test]
    fn rows_take_their_kind_height_in_every_density_and_style() {
        let ctx = ctx();
        for density in Density::ALL {
            for style in Style::ALL {
                theme::apply(
                    &ctx,
                    theme::ThemeId::Midnight,
                    theme::Appearance {
                        style,
                        density,
                        ..theme::Appearance::STILL
                    },
                );
                let m = theme::metrics();
                let mut h = Vec::new();
                run(&ctx, Vec::new(), |ui| {
                    h = vec![
                        ListRow::new(1)
                            .kind(RowKind::History)
                            .show(ui)
                            .rect
                            .height(),
                        ListRow::new(2).kind(RowKind::File).show(ui).rect.height(),
                        TreeRow::new(3, 0).show(ui).response.rect.height(),
                        SectionHeader::new("x")
                            .first(true)
                            .show(ui)
                            .response
                            .rect
                            .height(),
                    ];
                });
                assert_eq!(
                    h,
                    vec![
                        m.history_row_height,
                        m.row_height,
                        m.sidebar_row_height,
                        m.section_header_height
                    ],
                    "{density:?} {style:?}"
                );
            }
        }
    }

    #[test]
    fn zebra_stripes_even_rows_in_dense_only() {
        let _ = ctx();
        assert!(!zebra(1));
        theme::with_look(Style::Standard, Density::Dense, || {
            assert!(!zebra(0));
            assert!(zebra(1));
            assert!(!zebra(2));
        });
    }

    #[test]
    fn the_ground_composites_the_row_state() {
        let p = theme::ThemeId::Midnight.palette();
        let pane = p.standard.pane;
        assert_eq!(ground(p, pane, false, false, 0.0, false, false), pane);
        let sel = ground(p, pane, false, true, 0.0, false, false);
        assert_eq!(sel, theme::over(p.selected(), pane));
        let both = ground(p, pane, false, true, 1.0, false, false);
        assert_eq!(both, theme::over(p.hover(), sel));
    }

    #[test]
    fn rows_activate_from_the_keyboard_unless_disabled() {
        let ctx = ctx();
        for enabled in [true, false] {
            let mut clicked = false;
            for i in 0..3 {
                let events = if i == 1 {
                    vec![key(Key::Enter)]
                } else {
                    Vec::new()
                };
                run(&ctx, events, |ui| {
                    let r = ListRow::new("r").label("Row").enabled(enabled).show(ui);
                    if i == 0 {
                        ui.memory_mut(|m| m.request_focus(r.id));
                    }
                    clicked |= r.clicked();
                });
            }
            assert_eq!(clicked, enabled);
        }
    }

    #[test]
    fn tree_rows_open_with_right_and_ask_for_the_parent_with_left() {
        let ctx = ctx();
        let mut open = false;
        let mut navs = Vec::new();
        let presses = [
            None,
            Some(Key::ArrowRight),
            Some(Key::ArrowRight),
            Some(Key::ArrowLeft),
            Some(Key::ArrowLeft),
        ];
        let mut opens = Vec::new();
        for (i, k) in presses.into_iter().enumerate() {
            run(&ctx, k.map(key).into_iter().collect(), |ui| {
                let r = TreeRow::new("t", 0)
                    .label("origin")
                    .open(&mut open)
                    .show(ui);
                if i == 0 {
                    ui.memory_mut(|m| m.request_focus(r.response.id));
                }
                navs.push(r.value);
            });
            opens.push(open);
        }
        assert_eq!(opens, [false, true, true, false, false]);
        assert_eq!(
            navs,
            [
                None,
                None,
                Some(TreeNav::FirstChild),
                None,
                Some(TreeNav::Parent)
            ]
        );
    }

    #[test]
    fn a_leaf_asks_for_its_parent_and_never_opens() {
        let ctx = ctx();
        let mut nav = None;
        for i in 0..3 {
            let events = match i {
                1 => vec![key(Key::ArrowRight)],
                2 => vec![key(Key::ArrowLeft)],
                _ => Vec::new(),
            };
            run(&ctx, events, |ui| {
                let r = TreeRow::new("leaf", 1).label("main").show(ui);
                if i == 0 {
                    ui.memory_mut(|m| m.request_focus(r.response.id));
                }
                if i == 1 {
                    assert_eq!(r.value, None);
                }
                nav = r.value;
            });
        }
        assert_eq!(nav, Some(TreeNav::Parent));
    }

    #[test]
    fn a_section_header_toggles_on_enter_and_reports_its_action() {
        let ctx = ctx();
        let mut open = true;
        for i in 0..3 {
            let events = if i == 1 {
                vec![key(Key::Enter)]
            } else {
                Vec::new()
            };
            run(&ctx, events, |ui| {
                let r = SectionHeader::new("Branches").open(&mut open).show(ui);
                if i == 0 {
                    ui.memory_mut(|m| m.request_focus(r.response.id));
                }
                assert!(!r.value);
            });
        }
        assert!(!open);
    }

    #[test]
    fn tree_children_snap_with_reduced_motion() {
        let ctx = ctx();
        let id = Id::new("children");
        let mut shown = None;
        run(&ctx, Vec::new(), |ui| {
            shown = tree_children(ui, id, true, 3, |ui| ui.label("child").rect.height());
        });
        assert!(shown.is_some());
        run(&ctx, Vec::new(), |ui| {
            shown = tree_children(ui, id, false, 3, |ui| ui.label("child").rect.height());
        });
        assert!(shown.is_none(), "closed children disappear at once");
    }

    #[test]
    fn hover_lands_at_once_with_reduced_motion() {
        let ctx = ctx();
        let mut rect = Rect::NOTHING;
        let mut hovers = Vec::new();
        for i in 0..3 {
            let events = if i == 1 {
                vec![egui::Event::PointerMoved(rect.center())]
            } else {
                Vec::new()
            };
            run(&ctx, events, |ui| {
                let r = ListRow::new("h")
                    .label("Row")
                    .show_with(ui, |rc, _| hovers.push(rc.hover));
                rect = r.rect;
            });
        }
        // The frame the pointer arrives in already paints the full hover ground.
        assert_eq!(hovers, vec![0.0, 1.0, 1.0]);
    }

    #[test]
    fn selection_follows_click_ctrl_and_shift() {
        let mut s = Selection::default();
        s.click(3, Modifiers::NONE);
        assert_eq!(s.rows.iter().copied().collect::<Vec<_>>(), [3]);
        s.click(6, Modifiers::SHIFT);
        assert_eq!(s.rows.iter().copied().collect::<Vec<_>>(), [3, 4, 5, 6]);
        assert_eq!(s.anchor, Some(3));
        s.click(5, Modifiers::COMMAND);
        assert_eq!(s.rows.iter().copied().collect::<Vec<_>>(), [3, 4, 6]);
        s.click(1, Modifiers::SHIFT | Modifiers::COMMAND);
        assert_eq!(
            s.rows.iter().copied().collect::<Vec<_>>(),
            [1, 2, 3, 4, 5, 6]
        );
        s.click(9, Modifiers::NONE);
        assert_eq!(s.rows.iter().copied().collect::<Vec<_>>(), [9]);
        assert_eq!(s.neighbours(9), (false, false));
        s.click(10, Modifiers::SHIFT);
        assert_eq!(s.neighbours(9), (false, true));
        assert_eq!(s.neighbours(10), (true, false));
    }

    #[test]
    fn rows_announce_their_role_selection_and_expansion() {
        let ctx = ctx();
        ctx.enable_accesskit();
        let out = run(&ctx, Vec::new(), |ui| {
            ListRow::new("a").label("main.rs").selected(true).show(ui);
            TreeRow::new("b", 0)
                .label("origin")
                .open(&mut true)
                .show(ui);
            SectionHeader::new("Branches").open(&mut false).show(ui);
        });
        let update = out.platform_output.accesskit_update.expect("accesskit");
        let node = |label: &str| {
            update
                .nodes
                .iter()
                .find(|(_, n)| n.label() == Some(label))
                .map_or_else(|| panic!("no node {label}"), |(_, n)| n.clone())
        };
        let row = node("main.rs");
        assert_eq!(row.role(), egui::accesskit::Role::ListItem);
        assert_eq!(row.is_selected(), Some(true));
        let tree = node("origin");
        assert_eq!(tree.role(), egui::accesskit::Role::TreeItem);
        assert_eq!(tree.is_expanded(), Some(true));
        let header = node("Branches");
        assert_eq!(header.is_expanded(), Some(false));
    }

    #[test]
    fn hover_reveals_nothing_while_disabled() {
        let ctx = ctx();
        let mut grounds = Vec::new();
        run(&ctx, Vec::new(), |ui| {
            ListRow::new("d")
                .enabled(false)
                .show_with(ui, |rc, _| grounds.push(rc.ground));
            ListRow::new("e")
                .enabled(false)
                .show_forced(ui, ForceState::HOVER);
        });
        let p = theme::palette(&ctx);
        assert_eq!(grounds, vec![p.standard.pane]);
    }
}
