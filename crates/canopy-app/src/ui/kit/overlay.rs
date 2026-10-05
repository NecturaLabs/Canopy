//! Floating layers: the surface every one of them shares ([`floating`]: soft shadow, the glass
//! hook, the overlay fill, Deeper's top highlight and the 1 pt ring), [`Tooltip`], [`Popover`],
//! [`Dialog`] and its [`scrim`], and the motion they share: entrances that fade and grow from the
//! anchor side, and exits that fade out what the layer showed last. Design spec 4.11, 4.13, 4.14.
//!
//! **Glass.** [`backdrop`] is the one place the backdrop blur plugs in: every floating layer except
//! tooltips asks it for the blurred backdrop between its shadow and its fill, and paints the
//! translucent overlay over it when it gets one. With Translucent surfaces off, or the blur
//! unavailable, it returns `None` and the layer is opaque.
//!
//! **Exits.** A floating layer records what it painted each frame it is open ([`record_exit`]);
//! the frame it is no longer shown, an egui plugin keeps painting that snapshot as it fades (and
//! shrinks or slides) for the exit's duration. Nothing is recorded with reduced motion. A snapshot
//! drops the glass backdrop: the blur callback cannot fade or move with it, so a closing layer
//! fades out as its translucent fill alone.

use std::sync::Arc;

use egui::emath::TSTransform;
use egui::epaint::{ClippedShape, RectShape};
use egui::layers::ShapeIdx;
use egui::{
    Align, Align2, Color32, Context, CornerRadius, FontId, Frame, Id, InnerResponse, Key, LayerId,
    Layout, Margin, Order, Painter, Popup, PopupCloseBehavior, PopupKind, Pos2, Rangef, Rect,
    RectAlign, Response, RichText, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2,
    vec2,
};
use egui_phosphor::regular as icon;

use super::{Button, ForceState, IconButton, Size, paint};
use crate::keymap::{self, Command};
use crate::ui::motion;
use crate::ui::theme::{self, Elevation, Fill, Palette};

/// Room kept between a dialog card and the window edges (both sides together).
const DIALOG_MARGIN: f32 = 28.0;
/// The dialog's glyph disc.
const DISC: f32 = 30.0;
/// Gap between a tooltip or popover and its anchor.
const TOOLTIP_GAP: f32 = 8.0;
const POPOVER_GAP: f32 = 6.0;
/// Widest tooltip.
const TOOLTIP_WIDTH: f32 = 260.0;
/// Pointer-opened popups grow from this scale and drop this far from the anchor edge.
pub(crate) const POPUP_SCALE: f32 = 0.95;
const POPUP_DROP: f32 = 4.0;
/// Dialogs grow from this scale and exit towards the second.
const DIALOG_SCALE: f32 = 0.94;
const DIALOG_EXIT_SCALE: f32 = 0.97;
/// Tooltips grow from this scale and rise this far.
const TOOLTIP_SCALE: f32 = 0.96;
const TOOLTIP_RISE: f32 = 3.0;

/// What a floating surface is: decides its radius, fill and whether it may be glass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Layer {
    Menu,
    Popover,
    Dialog,
    Toast,
    /// Never glass; the `tooltip` fill (dark in light themes) and a smaller radius.
    Tooltip,
}

impl Layer {
    pub fn radius(self) -> CornerRadius {
        let m = theme::metrics();
        match self {
            Self::Tooltip => CornerRadius::same(m.radius_sm.saturating_add(2)),
            _ => CornerRadius::same(m.radius_lg),
        }
    }

    fn may_be_glass(self) -> bool {
        self != Self::Tooltip
    }
}

// --------------------------------------------------------------------------- surface and glass

/// The glass hook: what the backdrop blur paints under a floating layer at `rect`, between the
/// layer's shadow and its fill (design spec 7.4: a `Shape::Callback` into `glass/gl.rs`), or
/// `None` for an opaque layer (Translucent surfaces off, the blur unavailable, or the frame's
/// glass layers used up).
pub(crate) fn backdrop(painter: &Painter, rect: Rect, radius: CornerRadius) -> Option<Shape> {
    super::glass::backdrop_shape(painter, rect, radius)
}

/// Everything a floating surface of `layer` paints at `rect`, bottom first: the `Floating`
/// shadow, the glass backdrop (if any), the fill (translucent over glass), the top highlight
/// (Deeper dark) and the 1 pt ring.
pub(crate) fn surface_shapes(painter: &Painter, rect: Rect, layer: Layer) -> Vec<Shape> {
    let ctx = painter.ctx();
    let p = theme::palette(ctx);
    let style = theme::style();
    let s = p.surfaces(style);
    let radius = layer.radius();
    let material = p.material(style, Elevation::Floating);
    let mut shapes = paint::shadow_shapes(rect, radius, p, &material);
    let fill = if layer == Layer::Tooltip {
        s.tooltip
    } else {
        let glass = layer
            .may_be_glass()
            .then(|| backdrop(painter, rect, radius))
            .flatten();
        match glass {
            Some(b) => {
                shapes.push(b);
                Fill::flat(Palette::tint(s.overlay, p.glass_alpha))
            }
            None => Fill::flat(s.overlay),
        }
    };
    shapes.push(paint::fill_shape(ctx, rect, radius, fill));
    if material.highlight > 0 {
        let inset = f32::from(radius.nw.max(radius.ne)).max(1.0);
        let w = 1.0 / ctx.pixels_per_point();
        shapes.push(Shape::hline(
            Rangef::new(rect.left() + inset, rect.right() - inset),
            rect.top() + w * 0.5,
            Stroke::new(w, Color32::from_white_alpha(material.highlight)),
        ));
    }
    shapes.push(
        RectShape::stroke(
            rect,
            radius,
            Stroke::new(paint::SEAM, p.floating_ring(s)),
            StrokeKind::Inside,
        )
        .into(),
    );
    shapes
}

/// Lays `add` out inside a floating surface of `layer` with `margin` around it, then paints the
/// surface underneath (in a slot reserved before the content, so it sits below it). Works inside
/// a popup's area or inline (the gallery).
pub fn floating<R>(
    ui: &mut Ui,
    layer: Layer,
    margin: Margin,
    add: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let slot = ui.painter().add(Shape::Noop);
    let r = Frame::new().inner_margin(margin).show(ui, add);
    let shapes = surface_shapes(ui.painter(), r.response.rect, layer);
    ui.painter().set(slot, Shape::Vec(shapes));
    InnerResponse::new(r.inner, r.response)
}

/// The scrim behind dialogs, at `alpha` of its strength (its fade).
pub fn scrim(painter: &Painter, rect: Rect, alpha: f32) {
    let p = theme::palette(painter.ctx());
    painter.rect_filled(rect, CornerRadius::ZERO, p.scrim.gamma_multiply(alpha));
}

// --------------------------------------------------------------------------------- motion

/// How long the overlay `id` has been shown in an unbroken run of frames, and whether this is
/// the first frame of the run (it was not shown the frame before).
pub(crate) fn presence(ctx: &Context, id: Id) -> (f32, bool) {
    #[derive(Clone, Copy)]
    struct Run {
        since: f64,
        last: u64,
    }
    let now = ctx.input(|i| i.time);
    let pass = ctx.cumulative_pass_nr();
    let (since, fresh) = ctx.data_mut(|d| {
        let key = id.with("kit-shown-since");
        let fresh = d.get_temp::<Run>(key).is_none_or(|r| r.last + 1 < pass);
        let run = d.get_temp_mut_or_insert_with(key, || Run {
            since: now,
            last: pass,
        });
        if fresh {
            run.since = now;
        }
        run.last = pass;
        (run.since, fresh)
    });
    ((now - since) as f32, fresh)
}

/// How far an entrance has come: `fade` (opacity, 0 to 1) and `grow` (the transform's progress
/// along the spring curve, which overshoots 1 a little).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Entrance {
    pub fade: f32,
    pub grow: f32,
}

impl Entrance {
    pub const DONE: Self = Self {
        fade: 1.0,
        grow: 1.0,
    };

    /// The entrance of `id`, `fade_s` for the opacity (`EASE_OUT`) and `grow_s` for the transform
    /// (`SPRING_CURVE`), counted from its first frame. Done at once with reduced motion or when
    /// `instant` (keyboard-opened menus); `progress` fixes it (the gallery).
    pub fn of(
        ctx: &Context,
        id: Id,
        fade_s: f32,
        grow_s: f32,
        instant: bool,
        progress: Option<f32>,
    ) -> Self {
        if let Some(t) = progress {
            return Self::at(t, t);
        }
        let (elapsed, _) = presence(ctx, id);
        let (fade_s, grow_s) = (motion::time(fade_s), motion::time(grow_s));
        if instant || (fade_s <= 0.0 && grow_s <= 0.0) {
            return Self::DONE;
        }
        let k = |s: f32| {
            if s <= 0.0 {
                1.0
            } else {
                (elapsed / s).clamp(0.0, 1.0)
            }
        };
        let (a, b) = (k(fade_s), k(grow_s));
        if a < 1.0 || b < 1.0 {
            ctx.request_repaint();
        }
        Self::at(a, b)
    }

    /// The entrance at linear times `fade_t` and `grow_t` (0 to 1).
    pub fn at(fade_t: f32, grow_t: f32) -> Self {
        Self {
            fade: motion::EASE_OUT.at(fade_t),
            grow: motion::SPRING_CURVE.at(grow_t),
        }
    }

    /// The transform at this point: scaled from `from` about `origin` and offset by `offset`
    /// (both reached at `grow == 1`).
    pub fn transform(self, origin: Pos2, from: f32, offset: Vec2) -> TSTransform {
        let scale = from + (1.0 - from) * self.grow;
        let o = origin.to_vec2();
        TSTransform::from_translation(offset * (1.0 - self.grow) + o)
            * TSTransform::from_scaling(scale)
            * TSTransform::from_translation(-o)
    }
}

/// Where the next shape on `layer` will go.
pub(crate) fn next_shape(ctx: &Context, layer: LayerId) -> ShapeIdx {
    ctx.graphics(|g| {
        g.get(layer)
            .map_or(ShapeIdx(0), egui::layers::PaintList::next_idx)
    })
}

/// Transforms every shape painted on `layer` since `start`.
pub(crate) fn transform_since(ctx: &Context, layer: LayerId, start: ShapeIdx, tf: TSTransform) {
    if tf == TSTransform::IDENTITY {
        return;
    }
    ctx.graphics_mut(|g| {
        let list = g.entry(layer);
        let end = list.next_idx();
        list.transform_range(start, end, tf);
    });
}

/// How a closed layer leaves: fade over `seconds` while its moving part scales towards `scale`
/// about `origin` and slides by `offset`; its fixed part (a scrim) only fades.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ExitMotion {
    pub seconds: f32,
    pub scale: f32,
    pub origin: Pos2,
    pub offset: Vec2,
    /// Vanish at once if the frame it closed in had a press, scroll or key (tooltips).
    pub instant_on_input: bool,
}

impl ExitMotion {
    /// Menus and popovers: fade with a small shrink towards the anchor side.
    pub fn popup(origin: Pos2) -> Self {
        Self {
            seconds: motion::EXIT_POPUP,
            scale: POPUP_SCALE,
            origin,
            offset: Vec2::ZERO,
            instant_on_input: false,
        }
    }
}

#[derive(Clone)]
struct Snapshot {
    key: Id,
    order: Order,
    fixed: Arc<Vec<ClippedShape>>,
    moving: Arc<Vec<ClippedShape>>,
    motion: ExitMotion,
    pass: u64,
}

#[derive(Clone)]
struct Exit {
    snap: Snapshot,
    started: f64,
}

#[derive(Clone, Default)]
struct Exits {
    open: Vec<Snapshot>,
    leaving: Vec<Exit>,
}

fn exits_id() -> Id {
    Id::new("canopy-kit-exits")
}

/// Paints closing layers at the end of every pass.
struct ExitPlugin;

impl egui::Plugin for ExitPlugin {
    fn debug_name(&self) -> &'static str {
        "canopy-kit-exits"
    }

    fn on_end_pass(&mut self, ui: &mut Ui) {
        paint_exits(ui.ctx());
    }
}

/// Records what `key` painted on `layer` this frame (`fixed` then `moving` index ranges), so it
/// can fade out when it stops being shown. Does nothing with reduced motion.
pub(crate) fn record_exit(
    ctx: &Context,
    key: Id,
    layer: LayerId,
    fixed: std::ops::Range<usize>,
    moving: std::ops::Range<usize>,
    motion: ExitMotion,
) {
    if motion::time(motion.seconds) <= 0.0 {
        return;
    }
    let installed = Id::new("canopy-kit-exit-plugin");
    if !ctx.data(|d| d.get_temp::<bool>(installed).unwrap_or(false)) {
        ctx.add_plugin(ExitPlugin);
        ctx.data_mut(|d| d.insert_temp(installed, true));
    }
    let take = |r: std::ops::Range<usize>| -> Vec<ClippedShape> {
        ctx.graphics(|g| {
            g.get(layer).map_or_else(Vec::new, |l| {
                l.all_entries()
                    .skip(r.start)
                    .take(r.end.saturating_sub(r.start))
                    .map(|c| ClippedShape {
                        clip_rect: c.clip_rect,
                        shape: without_callbacks(c.shape.clone()),
                    })
                    .collect()
            })
        })
    };
    let snap = Snapshot {
        key,
        order: layer.order,
        fixed: Arc::new(take(fixed)),
        moving: Arc::new(take(moving)),
        motion,
        pass: ctx.cumulative_pass_nr(),
    };
    ctx.data_mut(|d| {
        let e = d.get_temp_mut_or_default::<Exits>(exits_id());
        e.open.retain(|s| s.key != key);
        e.leaving.retain(|x| x.snap.key != key);
        e.open.push(snap);
    });
}

/// `shape` with every paint callback (the glass backdrop) taken out: a callback paints at the
/// place and opacity it was made with, so it could neither fade nor move with an exit.
fn without_callbacks(shape: Shape) -> Shape {
    match shape {
        Shape::Callback(_) => Shape::Noop,
        Shape::Vec(shapes) => Shape::Vec(shapes.into_iter().map(without_callbacks).collect()),
        other => other,
    }
}

/// Starts the exits of layers not shown this pass and paints the ones still leaving.
fn paint_exits(ctx: &Context) {
    let pass = ctx.cumulative_pass_nr();
    let now = ctx.input(|i| i.time);
    let interrupted = ctx.input(|i| {
        i.pointer.any_pressed()
            || i.smooth_scroll_delta != Vec2::ZERO
            || i.events
                .iter()
                .any(|e| matches!(e, egui::Event::Key { pressed: true, .. }))
    });
    let leaving = ctx.data_mut(|d| {
        let e = d.get_temp_mut_or_default::<Exits>(exits_id());
        let (shown, closed): (Vec<_>, Vec<_>) = e.open.drain(..).partition(|s| s.pass == pass);
        e.open = shown;
        for snap in closed {
            if !(snap.motion.instant_on_input && interrupted) {
                e.leaving.push(Exit { snap, started: now });
            }
        }
        e.leaving
            .retain(|x| ((now - x.started) as f32) < x.snap.motion.seconds);
        e.leaving.clone()
    });
    for x in &leaving {
        let t = ((now - x.started) as f32 / x.snap.motion.seconds).clamp(0.0, 1.0);
        let k = motion::EASE_OUT.at(t);
        let alpha = 1.0 - k;
        let m = x.snap.motion;
        let scale = 1.0 + (m.scale - 1.0) * k;
        let o = m.origin.to_vec2();
        let tf = TSTransform::from_translation(m.offset * k + o)
            * TSTransform::from_scaling(scale)
            * TSTransform::from_translation(-o);
        let mut painter = ctx.layer_painter(LayerId::new(x.snap.order, x.snap.key.with("exit")));
        painter.set_opacity(alpha);
        for (shapes, tf) in [(&x.snap.fixed, TSTransform::IDENTITY), (&x.snap.moving, tf)] {
            for c in shapes.iter() {
                let mut shape = c.shape.clone();
                shape.transform(tf);
                painter.with_clip_rect(tf.mul_rect(c.clip_rect)).add(shape);
            }
        }
    }
    if !leaving.is_empty() {
        ctx.request_repaint();
    }
}

// --------------------------------------------------------------------------------- tooltip

/// Kit tooltips on any response: `response.kit_tooltip("Fetch all remotes").shortcut(Command::Fetch).show()`.
pub trait TooltipExt {
    fn kit_tooltip(self, text: impl Into<String>) -> Tooltip;
}

impl TooltipExt for Response {
    fn kit_tooltip(self, text: impl Into<String>) -> Tooltip {
        Tooltip {
            response: self,
            text: text.into(),
            detail: None,
            shortcut: None,
        }
    }
}

/// A tooltip (concept `.k-tip`): the `tooltip` surface, 12 pt medium text, an optional dim second
/// line and the shortcut as keycaps. Shown after `TOOLTIP_DELAY` (at once within `TOOLTIP_SKIP` of
/// another), above the control and flipped to stay in the window, for enabled and disabled
/// controls alike (a disabled one says why).
#[derive(Debug)]
#[must_use = "call .show()"]
pub struct Tooltip {
    response: Response,
    text: String,
    detail: Option<String>,
    shortcut: Option<Command>,
}

/// Above first, then below, then the sides.
const TOOLTIP_ALIGNS: [RectAlign; 4] = [
    RectAlign::TOP,
    RectAlign::BOTTOM,
    RectAlign::RIGHT,
    RectAlign::LEFT,
];

impl Tooltip {
    /// A dim second line (what the action affects, or why it is unavailable).
    pub fn detail(mut self, text: impl Into<String>) -> Self {
        self.detail = Some(text.into());
        self
    }
    /// The command's keys as keycaps after the text.
    pub fn shortcut(mut self, command: Command) -> Self {
        self.shortcut = Some(command);
        self
    }

    pub fn show(self) -> Response {
        let Self {
            response,
            text,
            detail,
            shortcut,
        } = self;
        let mut tip = if response.enabled() {
            egui::containers::Tooltip::for_enabled(&response)
        } else {
            egui::containers::Tooltip::for_disabled(&response)
        };
        tip.popup = tip
            .popup
            .frame(Frame::NONE)
            .gap(TOOLTIP_GAP)
            .align(RectAlign::TOP)
            .align_alternatives(&TOOLTIP_ALIGNS)
            .width(TOOLTIP_WIDTH);
        let anchor = response.rect;
        let id = response.id.with("kit-tooltip");
        tip.show(|ui| {
            let layer = ui.layer_id();
            let ctx = ui.ctx().clone();
            let e = Entrance::of(&ctx, id, motion::FAST_OUT, motion::BASE, false, None);
            ui.multiply_opacity(e.fade);
            let start = next_shape(&ctx, layer);
            let r = tooltip_body(ui, &text, detail.as_deref(), shortcut);
            let rect = r.rect;
            let below = rect.center().y > anchor.center().y;
            let origin = if below {
                rect.center_top()
            } else {
                rect.center_bottom()
            };
            let rise = if below { -TOOLTIP_RISE } else { TOOLTIP_RISE };
            transform_since(
                &ctx,
                layer,
                start,
                e.transform(origin, TOOLTIP_SCALE, vec2(0.0, rise)),
            );
            let end = next_shape(&ctx, layer);
            record_exit(
                &ctx,
                id,
                layer,
                0..0,
                start.0..end.0,
                ExitMotion {
                    seconds: motion::TOOLTIP_OUT,
                    scale: 1.0,
                    origin,
                    offset: Vec2::ZERO,
                    instant_on_input: true,
                },
            );
        });
        response
    }
}

/// The keycaps of `command`'s keys: the single key that works outside text fields when there is
/// one, else the chord, split into its keys.
pub(crate) fn chord_keys(command: Command) -> Vec<String> {
    let hint = keymap::hint(command);
    let chord = hint.rsplit(" · ").next().unwrap_or_default();
    split_chord(chord)
}

/// "Ctrl+Shift+F" as ["Ctrl", "Shift", "F"]; a "+" key stays a key ("Ctrl++").
pub(crate) fn split_chord(chord: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut rest = chord;
    while !rest.is_empty() {
        if let Some(i) = rest[1..].find('+').map(|i| i + 1) {
            keys.push(rest[..i].to_owned());
            rest = &rest[i + 1..];
        } else {
            keys.push(rest.to_owned());
            break;
        }
    }
    keys
}

/// A tooltip's content and surface at the cursor of `ui` (inside its popup, or inline).
pub(crate) fn tooltip_body(
    ui: &mut Ui,
    text: &str,
    detail: Option<&str>,
    shortcut: Option<Command>,
) -> Response {
    let m = theme::metrics();
    let keys = shortcut.map(chord_keys).unwrap_or_default();
    let pad = m.tooltip_pad;
    floating(ui, Layer::Tooltip, pad, |ui| {
        let p = theme::palette(ui.ctx());
        ui.set_max_width(TOOLTIP_WIDTH - f32::from(pad.left + pad.right));
        ui.spacing_mut().item_spacing = vec2(theme::SM, 2.0);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(text)
                            .font(FontId::new(m.tooltip_size, theme::medium()))
                            .color(p.on_tooltip),
                    )
                    .wrap(),
                );
                if let Some(d) = detail {
                    ui.add(
                        egui::Label::new(
                            RichText::new(d)
                                .size(m.tooltip_size)
                                .color(p.on_tooltip_dim),
                        )
                        .wrap(),
                    );
                }
            });
            if !keys.is_empty() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    for k in &keys {
                        tooltip_keycap(ui, k);
                    }
                });
            }
        });
    })
    .response
}

/// A keycap inside a tooltip: 18 pt, the theme's keycap in dark themes; on the dark tooltip of a
/// light theme, a dark cap stepped up from the tooltip with no bottom line.
fn tooltip_keycap(ui: &mut Ui, key: &str) {
    let ctx = ui.ctx().clone();
    let p = theme::palette(&ctx);
    let s = theme::surfaces(&ctx);
    let m = theme::metrics();
    let font = FontId::new(m.chord_size, egui::FontFamily::Monospace);
    let (fill, text, ring, bottom) = if p.dark {
        (s.keycap.top, p.text_dim, s.edge, Some(s.keycap_edge))
    } else {
        let base = s.tooltip.top;
        (
            theme::mix(base, p.on_tooltip, 0.09),
            theme::mix(p.on_tooltip_dim, p.on_tooltip, 0.5),
            theme::mix(base, p.on_tooltip, 0.16),
            None,
        )
    };
    let g = ui
        .painter()
        .layout_no_wrap(key.to_owned(), font, Color32::PLACEHOLDER);
    let h = 18.0;
    let w = (g.size().x + 10.0).max(h);
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let radius = CornerRadius::same(m.keycap_radius.min(4));
    let painter = ui.painter();
    painter.rect_filled(rect, radius, fill);
    painter.rect_stroke(rect, radius, Stroke::new(1.0, ring), StrokeKind::Inside);
    if let Some(b) = bottom {
        painter.hline(
            Rangef::new(rect.left() + 2.0, rect.right() - 2.0),
            rect.bottom() - 1.5,
            Stroke::new(1.0, b),
        );
    }
    painter.galley(rect.center() - g.size() / 2.0, g, text);
}

// --------------------------------------------------------------------------------- popover

/// A small panel anchored to a control (diff options, author card, theme quick-pick): the overlay
/// surface with `card_pad` inside, 6 pt from the anchor and at least as wide. Clicking the anchor
/// toggles it; a click outside or Esc closes it and gives focus back to the anchor. One popover is
/// open at a time.
#[must_use = "call .show(ui, ..)"]
pub struct Popover<'a> {
    id: Id,
    anchor: &'a Response,
    width: Option<f32>,
    open: Option<&'a mut bool>,
    force: Option<ForceState>,
}

impl<'a> Popover<'a> {
    pub fn new(id_salt: impl std::hash::Hash + std::fmt::Debug, anchor: &'a Response) -> Self {
        Self {
            id: anchor.id.with(("kit-popover", Id::new(id_salt))),
            anchor,
            width: None,
            open: None,
            force: None,
        }
    }
    /// The content width (default: the content's own, at least the anchor's).
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }
    /// Keeps the open state in `open` instead of toggling on anchor clicks.
    pub fn open(mut self, open: &'a mut bool) -> Self {
        self.open = Some(open);
        self
    }
    /// Shows it with forced states (`OPEN` keeps it open; the gallery and tests only).
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the kit's own tests force it open")
    )]
    pub(crate) fn force(mut self, force: ForceState) -> Self {
        self.force = Some(force);
        self
    }

    pub fn show<R>(self, add: impl FnOnce(&mut Ui) -> R) -> Option<InnerResponse<R>> {
        let Popover {
            id,
            anchor,
            width,
            open,
            force,
        } = self;
        let ctx = anchor.ctx.clone();
        let current = Id::new("canopy-kit-open-popover");
        let was_open = Popup::is_id_open(&ctx, id);
        let mut popup = Popup::from_toggle_button_response(anchor)
            .id(id)
            .kind(PopupKind::Popup)
            .frame(Frame::NONE)
            .gap(POPOVER_GAP)
            .align(RectAlign::BOTTOM_START)
            .close_behavior(PopupCloseBehavior::CloseOnClickOutside);
        if let Some(w) = width {
            popup = popup.width(w);
        }
        let forced = force.is_some_and(|f| f.open);
        if forced {
            popup = popup.open(true);
        } else if let Some(o) = open {
            popup = popup.open_bool(o);
        }
        let keyboard = anchor.clicked() && !anchor.clicked_by(egui::PointerButton::Primary);
        let anchor_rect = anchor.rect;
        let progress = force.and_then(|f| f.progress);
        let shown = popup.show(|ui| {
            let layer = ui.layer_id();
            let e = Entrance::of(ui.ctx(), id, motion::BASE, motion::SLOW, keyboard, progress);
            ui.multiply_opacity(e.fade);
            let start = next_shape(ui.ctx(), layer);
            let pad = theme::metrics().card_pad as i8;
            let r = floating(ui, Layer::Popover, Margin::same(pad), |ui| {
                let inner = anchor_rect.width() - 2.0 * f32::from(pad);
                ui.set_min_width(width.unwrap_or(0.0).max(inner));
                add(ui)
            });
            let rect = r.response.rect;
            let (origin, drop) = popup_origin(rect, anchor_rect);
            transform_since(
                ui.ctx(),
                layer,
                start,
                e.transform(origin, POPUP_SCALE, drop),
            );
            if !forced {
                let end = next_shape(ui.ctx(), layer);
                record_exit(
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
        let now_open = !forced && shown.is_some() && Popup::is_id_open(&ctx, id);
        if now_open && !was_open {
            // One popover at a time: opening this one closes the one open before.
            let before: Option<Id> = ctx.data(|d| d.get_temp(current));
            if let Some(o) = before.filter(|o| *o != id) {
                Popup::close_id(&ctx, o);
            }
            ctx.data_mut(|d| d.insert_temp(current, id));
        } else if was_open && !now_open && !forced {
            anchor.request_focus();
        }
        shown
    }
}

/// The corner of a popup at `rect` nearest its anchor, and the offset it drops in from (towards
/// the anchor).
pub(crate) fn popup_origin(rect: Rect, anchor: Rect) -> (Pos2, Vec2) {
    let above = rect.center().y < anchor.center().y;
    let x = if anchor.center().x <= rect.center().x {
        rect.left()
    } else {
        rect.right()
    };
    if above {
        (pos2(x, rect.bottom()), vec2(0.0, POPUP_DROP))
    } else {
        (pos2(x, rect.top()), vec2(0.0, -POPUP_DROP))
    }
}

// ---------------------------------------------------------------------------------- dialog

/// A dialog's width: confirmations, forms, or wide content (spec 2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DialogWidth {
    #[default]
    Confirm,
    Form,
    #[expect(dead_code, reason = "used by the view migration")]
    Wide,
}

impl DialogWidth {
    pub fn points(self) -> f32 {
        let m = theme::metrics();
        match self {
            Self::Confirm => m.dialog_width,
            Self::Form => m.dialog_width_form,
            Self::Wide => m.dialog_width_wide,
        }
    }
}

/// What the user did with a dialog this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogResult {
    /// Still open.
    Open,
    /// Cancel, Esc, the close button or a click on the scrim.
    Cancel,
    /// The confirming button (or Enter, outside destructive dialogs).
    Confirm,
}

/// Which control takes focus when a dialog opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitialFocus {
    Cancel,
    Confirm,
    /// The body requests focus itself (a form's first field).
    Body,
}

/// The footer's buttons and hint, set inside [`Dialog::footer`].
#[derive(Debug, Default)]
pub struct Footer {
    cancel: Option<String>,
    confirm: Option<String>,
    confirm_icon: Option<String>,
    confirm_enabled: bool,
    confirm_loading: bool,
    confirm_reason: Option<String>,
    hint: Option<String>,
    /// Two equal choices ([`Footer::choice`]): both Secondary, and Enter picks neither.
    neutral: bool,
}

impl Footer {
    /// A Cancel button.
    pub fn cancel(&mut self) -> &mut Self {
        self.cancel = Some("Cancel".to_owned());
        self
    }
    /// A Cancel button with another label ("Close", "Not now").
    pub fn cancel_label(&mut self, label: impl Into<String>) -> &mut Self {
        self.cancel = Some(label.into());
        self
    }
    /// The confirming button, rightmost: primary, or Danger in destructive dialogs. Label it verb
    /// and object ("Delete branch").
    pub fn confirm(&mut self, label: impl Into<String>) -> &mut Self {
        self.confirm = Some(label.into());
        self.confirm_enabled = true;
        self
    }
    /// A leading glyph on the confirming button.
    pub fn confirm_icon(&mut self, glyph: &str) -> &mut Self {
        self.confirm_icon = Some(glyph.to_owned());
        self
    }
    /// Disables the confirming button, saying why in its tooltip.
    pub fn confirm_disabled(&mut self, reason: impl Into<String>) -> &mut Self {
        self.confirm_enabled = false;
        self.confirm_reason = Some(reason.into());
        self
    }
    /// The confirmed operation is running.
    pub fn confirm_loading(&mut self, loading: bool) -> &mut Self {
        self.confirm_loading = loading;
        self
    }
    /// A short left-aligned hint ("Esc cancels").
    pub fn hint(&mut self, text: impl Into<String>) -> &mut Self {
        self.hint = Some(text.into());
        self
    }
    /// Two equal answers to a question with no safe default (consent): both Secondary, the same
    /// size, neither focused unless [`Dialog::initial_focus`] says so, and Enter picks neither.
    /// `first` (rightmost) answers `Confirm`, `second` answers `Cancel`.
    pub fn choice(&mut self, first: impl Into<String>, second: impl Into<String>) -> &mut Self {
        self.confirm = Some(first.into());
        self.confirm_enabled = true;
        self.cancel = Some(second.into());
        self.neutral = true;
        self
    }
}

/// What a dialog shows between its description and its footer.
type DialogBody<'a> = Box<dyn FnOnce(&mut Ui) + 'a>;

/// A modal dialog (concept `.k-dialog`) over the scrim:
/// `Dialog::new("delete-branch").title("Delete branch 'feature/x'?").icon(icon::TRASH).destructive()
/// .description("…").footer(|f| { f.cancel().confirm("Delete branch"); }).show(ctx)`.
///
/// Focus moves into it on open (Cancel in destructive dialogs, else the confirming button, unless
/// the body takes it), stays inside while it is open and returns to the control that had it when
/// the dialog closes through its result. Esc and a click on the scrim cancel; Enter confirms except
/// in destructive dialogs, where only the focused button answers.
#[must_use = "call .show(ctx)"]
pub struct Dialog<'a> {
    id: Id,
    title: String,
    icon: Option<String>,
    destructive: bool,
    description: Option<String>,
    body: Option<DialogBody<'a>>,
    footer: Footer,
    width: DialogWidth,
    closable: bool,
    blocking: bool,
    initial_focus: Option<InitialFocus>,
    force: Option<ForceState>,
}

impl<'a> Dialog<'a> {
    pub fn new(id_salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        Self {
            id: Id::new(("kit-dialog", Id::new(id_salt))),
            title: String::new(),
            icon: None,
            destructive: false,
            description: None,
            body: None,
            footer: Footer::default(),
            width: DialogWidth::Confirm,
            closable: false,
            blocking: false,
            initial_focus: None,
            force: None,
        }
    }
    /// The question or task, as a title (destructive: the question, "Delete branch 'x'?").
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }
    /// A glyph in a 30 pt disc before the title.
    pub fn icon(mut self, glyph: &str) -> Self {
        self.icon = Some(glyph.to_owned());
        self
    }
    /// Destroys something: danger disc and button, focus starts on Cancel, Enter does not confirm.
    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }
    /// What happens and whether it can be undone (13 pt, dim).
    pub fn description(mut self, text: impl Into<String>) -> Self {
        self.description = Some(text.into());
        self
    }
    /// Anything else between the description and the footer; scrolls when the card is taller
    /// than the window.
    pub fn body(mut self, add: impl FnOnce(&mut Ui) + 'a) -> Self {
        self.body = Some(Box::new(add));
        self
    }
    pub fn footer(mut self, f: impl FnOnce(&mut Footer)) -> Self {
        f(&mut self.footer);
        self
    }
    pub fn width(mut self, width: DialogWidth) -> Self {
        self.width = width;
        self
    }
    /// A close button top-right (non-blocking dialogs only).
    pub fn closable(mut self) -> Self {
        self.closable = true;
        self
    }
    pub fn initial_focus(mut self, focus: InitialFocus) -> Self {
        self.initial_focus = Some(focus);
        self
    }
    /// Must be answered with its buttons: a click on the scrim (or Esc reaching the modal) does
    /// not cancel it (the start-up consent, git's credential prompts).
    pub fn blocking(mut self) -> Self {
        self.blocking = true;
        self
    }
    /// Shows the card where the cursor is, without the modal layer, scrim or focus handling, with
    /// forced states (the gallery only).
    pub(crate) fn show_inline(mut self, ui: &mut Ui, force: ForceState) -> DialogResult {
        self.force = Some(force);
        let max_h = ui.available_height().max(120.0);
        let (result, _) = self.card(ui, max_h, false);
        result
    }

    /// Shows the dialog for this frame. Call it every frame while the dialog is open; stop once
    /// it returns `Cancel` or `Confirm`.
    pub fn show(self, ctx: &Context) -> DialogResult {
        let id = self.id;
        let blocking = self.blocking;
        let screen = ctx.content_rect();
        let (_, fresh) = presence(ctx, id);
        if fresh {
            let trigger = ctx.memory(egui::Memory::focused);
            ctx.data_mut(|d| d.insert_temp(id.with("kit-trigger"), trigger));
        }
        let e = Entrance::of(ctx, id, motion::SLOW, motion::SLOW, false, None);
        let fade = e.fade;
        let max_h = (screen.height() - DIALOG_MARGIN).max(120.0);
        let modal = egui::Modal::new(id.with("modal"))
            .backdrop_color(theme::palette(ctx).scrim.gamma_multiply(fade))
            .frame(Frame::NONE)
            .show(ctx, |ui| {
                let layer = ui.layer_id();
                // The scrim egui painted is the fixed part of the exit; the card moves.
                let start = next_shape(ui.ctx(), layer);
                ui.multiply_opacity(fade);
                let (result, rect) = self.card(ui, max_h, fresh);
                let center = rect.center();
                transform_since(
                    ui.ctx(),
                    layer,
                    start,
                    e.transform(center, DIALOG_SCALE, Vec2::ZERO),
                );
                let end = next_shape(ui.ctx(), layer);
                record_exit(
                    ui.ctx(),
                    id,
                    layer,
                    0..start.0,
                    start.0..end.0,
                    ExitMotion {
                        seconds: motion::EXIT_DIALOG,
                        scale: DIALOG_EXIT_SCALE,
                        origin: center,
                        offset: Vec2::ZERO,
                        instant_on_input: false,
                    },
                );
                result
            });
        let mut result = modal.inner;
        if result == DialogResult::Open && !blocking && modal.should_close() {
            result = DialogResult::Cancel;
        }
        if result != DialogResult::Open {
            let trigger: Option<Id> = ctx
                .data(|d| d.get_temp::<Option<Id>>(id.with("kit-trigger")))
                .flatten();
            ctx.data_mut(|d| d.remove::<Option<Id>>(id.with("kit-trigger")));
            if let Some(t) = trigger {
                ctx.memory_mut(|m| m.request_focus(t));
            }
        }
        result
    }

    /// The card at the cursor, at most `max_h` tall. Returns the result and the card's rect.
    fn card(self, ui: &mut Ui, max_h: f32, fresh: bool) -> (DialogResult, Rect) {
        let Dialog {
            id,
            title,
            icon: glyph,
            destructive,
            description,
            body,
            footer,
            width,
            closable,
            blocking: _,
            initial_focus,
            force,
        } = self;
        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let s = theme::surfaces(&ctx);
        let m = theme::metrics();
        let screen = ctx.content_rect();
        let w = width
            .points()
            .min(screen.width() - DIALOG_MARGIN)
            .min(ui.available_width().max(DISC * 4.0));
        let pad = m.dialog_pad;
        let mut result = DialogResult::Open;
        let enter = ctx.input(|i| i.key_pressed(Key::Enter));
        let r = floating(ui, Layer::Dialog, Margin::same(pad as i8), |ui| {
            // The card's own node, so its controls are announced inside the dialog.
            ctx.accesskit_node_builder(ui.id(), |n| {
                n.set_role(if destructive {
                    egui::accesskit::Role::AlertDialog
                } else {
                    egui::accesskit::Role::Dialog
                });
                n.set_label(title.as_str());
            });
            ui.set_width(w - 2.0 * pad);
            // Stacked whatever the caller's layout is.
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing = vec2(theme::SM, m.dialog_gap);
                let top = ui.cursor().top();
                // Title row.
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    if let Some(g) = &glyph {
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(DISC), Sense::hover());
                        let (tint, color) = if destructive {
                            (p.danger_tint(), p.danger)
                        } else {
                            (p.accent_tint(), p.accent)
                        };
                        ui.painter().circle_filled(rect.center(), DISC / 2.0, tint);
                        ui.painter().text(
                            rect.center(),
                            Align2::CENTER_CENTER,
                            g,
                            FontId::proportional(m.icon_md),
                            color,
                        );
                    }
                    // The close button sits at the right end, past the title.
                    let close_w = if closable {
                        m.control_sm + 2.0 + ui.spacing().item_spacing.x
                    } else {
                        0.0
                    };
                    let title_w = (ui.available_width() - close_w).max(0.0);
                    ui.allocate_ui_with_layout(
                        vec2(title_w, DISC),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.set_min_width(title_w);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&title)
                                        .font(FontId::new(m.title_size, theme::semibold()))
                                        .color(p.text),
                                )
                                .wrap(),
                            );
                        },
                    );
                    if closable
                        && IconButton::new(icon::X, "Close")
                            .size(Size::Sm)
                            .id_salt(id.with("close"))
                            .show(ui)
                            .clicked()
                    {
                        result = DialogResult::Cancel;
                    }
                });
                // Description and body, scrolling inside the card when it would be too tall.
                let used = ui.cursor().top() - top;
                let footer_h = m.control + theme::XS;
                let room = (max_h - 2.0 * pad - used - footer_h - m.dialog_gap).max(48.0);
                let has_body = description.is_some() || body.is_some();
                let mut scrolled = false;
                if has_body {
                    let out = egui::ScrollArea::vertical()
                        .id_salt(id.with("body"))
                        .max_height(room)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = m.dialog_gap;
                            if let Some(d) = &description {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(d)
                                            .size(m.body_size)
                                            .line_height(Some((m.body_size * 1.5).round()))
                                            .color(p.text_dim),
                                    )
                                    .wrap(),
                                );
                            }
                            if let Some(b) = body {
                                b(ui);
                            }
                        });
                    scrolled = out.content_size.y > out.inner_rect.height() + 0.5;
                }
                // Footer: 4 pt more above it, a seam only when the body scrolls.
                ui.add_space(theme::XS);
                let (row, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), m.control), Sense::hover());
                if scrolled {
                    paint::seam_h(
                        ui.painter(),
                        Rangef::new(row.left() - pad, row.right() + pad),
                        row.top() - theme::XS - m.dialog_gap / 2.0,
                        s.border,
                    );
                }
                if let Some(h) = &footer.hint {
                    ui.painter().text(
                        row.left_center(),
                        Align2::LEFT_CENTER,
                        h,
                        FontId::proportional(m.meta_size),
                        p.text_faint,
                    );
                }
                let mut cancel_focus = false;
                ui.scope_builder(
                    UiBuilder::new()
                        .max_rect(row)
                        .layout(Layout::right_to_left(Align::Center)),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = theme::SM;
                        let mut confirm_id = None;
                        if let Some(label) = &footer.confirm {
                            let mut b = Button::new(label.as_str())
                                .size(Size::Md)
                                .enabled(footer.confirm_enabled)
                                .loading(footer.confirm_loading)
                                .id_salt(id.with("confirm"));
                            b = if footer.neutral {
                                b.secondary()
                            } else if destructive {
                                b.danger()
                            } else {
                                b.primary()
                            };
                            if let Some(g) = &footer.confirm_icon {
                                b = b.icon(g);
                            }
                            if let Some(r) = &footer.confirm_reason {
                                b = b.disabled_reason(r.as_str());
                            }
                            let forced = force.filter(|f| f.focus && !destructive);
                            let r = match forced {
                                Some(f) => b.show_forced(ui, f),
                                None => b.show(ui),
                            };
                            if r.clicked() {
                                result = DialogResult::Confirm;
                            }
                            confirm_id = Some(r.id);
                        }
                        if let Some(label) = &footer.cancel {
                            let b = Button::new(label.as_str())
                                .secondary()
                                .size(Size::Md)
                                .id_salt(id.with("cancel"));
                            let forced = force.filter(|f| f.focus && destructive);
                            let r = match forced {
                                Some(f) => b.show_forced(ui, f),
                                None => b.show(ui),
                            };
                            if r.clicked() {
                                result = DialogResult::Cancel;
                            }
                            cancel_focus = r.has_focus();
                            if fresh {
                                let want = initial_focus.unwrap_or(if footer.neutral {
                                    InitialFocus::Body
                                } else if destructive {
                                    InitialFocus::Cancel
                                } else {
                                    InitialFocus::Confirm
                                });
                                match (want, confirm_id) {
                                    (InitialFocus::Cancel, _) | (InitialFocus::Confirm, None) => {
                                        r.request_focus();
                                    }
                                    (InitialFocus::Confirm, Some(c)) => {
                                        ctx.memory_mut(|m| m.request_focus(c));
                                    }
                                    (InitialFocus::Body, _) => {}
                                }
                            }
                        } else if fresh
                            && let Some(c) = confirm_id
                            && initial_focus != Some(InitialFocus::Body)
                        {
                            ctx.memory_mut(|m| m.request_focus(c));
                        }
                    },
                );
                if result == DialogResult::Open
                    && force.is_none()
                    && !destructive
                    && !footer.neutral
                    && enter
                    && footer.confirm.is_some()
                    && footer.confirm_enabled
                    && !footer.confirm_loading
                    && !cancel_focus
                    && !ctx.egui_wants_keyboard_input()
                {
                    result = DialogResult::Confirm;
                }
            });
        });
        (result, r.response.rect)
    }
}

// --------------------------------------------------------------------------------- gallery

/// The gallery's Overlays section, drawn inline and forced open: tooltips with keycaps, a
/// popover, both dialogs on a local scrim, their entrances part-way, and the toast cards.
pub(crate) fn gallery(ui: &mut Ui) {
    let m = theme::metrics();
    let p = theme::palette(ui.ctx());
    let small = |ui: &mut Ui, text: &str| {
        ui.label(RichText::new(text).size(m.small_size).color(p.text_faint));
    };
    small(ui, "Tooltips: plain, with a shortcut, with a second line");
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(theme::MD, theme::SM);
        tooltip_body(ui, "Settings", None, None);
        tooltip_body(ui, "Fetch all remotes", None, Some(Command::Fetch));
        tooltip_body(
            ui,
            "Push main to origin",
            Some("2 commits ahead"),
            Some(Command::Commit),
        );
    });
    small(ui, "Tooltip entrance at 0.3, 0.6 and 1");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::MD;
        for t in [0.3, 0.6, 1.0] {
            entering(ui, t, TOOLTIP_SCALE, vec2(0.0, TOOLTIP_RISE), |ui| {
                tooltip_body(ui, "Fetch all remotes", None, Some(Command::Fetch));
            });
        }
    });
    ui.add_space(theme::SM);
    small(ui, "Popover");
    let pad = m.card_pad as i8;
    floating(ui, Layer::Popover, Margin::same(pad), |ui| {
        ui.set_width(240.0);
        ui.label(
            RichText::new("Diff options")
                .font(FontId::new(m.body_size, theme::semibold()))
                .color(p.text),
        );
        ui.label(
            RichText::new("Context lines, whitespace and word diff apply to every file.")
                .size(m.meta_size)
                .color(p.text_dim),
        );
        ui.horizontal(|ui| {
            Button::new("Reset")
                .ghost()
                .size(Size::Sm)
                .id_salt("g-pop-reset")
                .show(ui);
            Button::new("Apply")
                .primary()
                .size(Size::Sm)
                .id_salt("g-pop-apply")
                .show(ui);
        });
    });
    ui.add_space(theme::SM);
    small(
        ui,
        "Dialogs on a scrim: destructive (Cancel focused) and normal",
    );
    for (i, destructive) in [true, false].into_iter().enumerate() {
        let width = ui.available_width();
        let height = if destructive { 250.0 } else { 220.0 };
        let (area, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        // A busy backdrop so the scrim reads.
        let painter = ui.painter_at(area);
        for row in 0..((height / m.history_row_height) as usize) {
            let y = area.top() + row as f32 * m.history_row_height + m.history_row_height / 2.0;
            painter.text(
                pos2(area.left() + theme::MD, y),
                Align2::LEFT_CENTER,
                [
                    "docs: add API reference",
                    "feat(api): add ETag headers",
                    "fix(ui): improve contrast",
                ][row % 3],
                FontId::proportional(m.body_size),
                p.text,
            );
        }
        scrim(&painter, area, 1.0);
        let inner = area.shrink(theme::LG);
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(inner)
                .layout(Layout::top_down(Align::Center)),
            |ui| {
                let d = if destructive {
                    Dialog::new(("g-dialog", i))
                        .title("Delete branch feature/dark-mode?")
                        .icon(icon::WARNING)
                        .destructive()
                        .description(
                            "feature/dark-mode has 1 commit that is on no other branch. You can \
                             restore it from the reflog for 90 days.",
                        )
                        .footer(|f| {
                            f.cancel()
                                .confirm("Delete branch")
                                .confirm_icon(icon::TRASH)
                                .hint("Esc cancels");
                        })
                } else {
                    Dialog::new(("g-dialog", i))
                        .title("Rename branch")
                        .icon(icon::PENCIL_SIMPLE)
                        .closable()
                        .description("Renames feature/auth locally. Its upstream keeps its name.")
                        .footer(|f| {
                            f.cancel().confirm("Rename");
                        })
                };
                d.show_inline(ui, ForceState::FOCUS);
            },
        );
        // Below the whole scrim, however tall the card came out.
        ui.advance_cursor_after_rect(area);
    }
    small(ui, "Live: hover the icon, open the popover, open a dialog");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::LG;
        IconButton::new(icon::ARROWS_CLOCKWISE, "Fetch")
            .id_salt("g-live-tip")
            .show(ui)
            .kit_tooltip("Fetch all remotes")
            .detail("origin and upstream")
            .shortcut(Command::Fetch)
            .show();
        let anchor = Button::new("Diff options")
            .caret()
            .id_salt("g-live-popover")
            .show(ui);
        Popover::new("g-live-popover", &anchor)
            .width(240.0)
            .show(|ui| {
                ui.label(
                    RichText::new("Context lines, whitespace and word diff apply to every file.")
                        .size(m.meta_size)
                        .color(p.text_dim),
                );
            });
        let open_key = ui.id().with("g-live-dialog-open");
        if Button::new("Open dialog")
            .id_salt("g-live-dialog")
            .show(ui)
            .clicked()
        {
            ui.ctx().data_mut(|d| d.insert_temp(open_key, true));
        }
        if ui
            .ctx()
            .data(|d| d.get_temp::<bool>(open_key).unwrap_or(false))
        {
            let result = Dialog::new(open_key)
                .title("Create tag")
                .icon(icon::TAG)
                .width(DialogWidth::Form)
                .description("Tags 38b14e7 on main.")
                .body(|ui| {
                    ui.label(
                        RichText::new("v1.4.0")
                            .font(FontId::monospace(m.mono_size))
                            .color(p.text),
                    );
                })
                .initial_focus(InitialFocus::Confirm)
                .footer(|f| {
                    f.cancel_label("Not now")
                        .confirm("Create tag")
                        .hint("Esc cancels");
                })
                .show(ui.ctx());
            if result != DialogResult::Open {
                ui.ctx().data_mut(|d| d.remove::<bool>(open_key));
            }
        }
    });
    ui.add_space(theme::SM);
    small(ui, "Dialog entrance at 0.3, 0.6 and 1");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::MD;
        for (i, t) in [0.3, 0.6, 1.0].into_iter().enumerate() {
            entering(ui, t, DIALOG_SCALE, Vec2::ZERO, |ui| {
                ui.set_max_width(180.0);
                Dialog::new(("g-dialog-motion", i))
                    .title("Discard changes?")
                    .destructive()
                    .footer(|f| {
                        f.confirm("Discard");
                    })
                    .show_inline(ui, ForceState::default());
            });
        }
    });
    ui.add_space(theme::SM);
    super::toast::gallery(ui);
}

/// Paints `add` as an entrance at linear time `t` would show it (the gallery's motion rows).
fn entering(ui: &mut Ui, t: f32, from: f32, offset: Vec2, add: impl FnOnce(&mut Ui)) {
    let e = Entrance::at(t, t);
    let layer = ui.layer_id();
    let start = next_shape(ui.ctx(), layer);
    let r = ui.scope(|ui| {
        ui.multiply_opacity(e.fade);
        add(ui);
    });
    let origin = r.response.rect.center();
    transform_since(ui.ctx(), layer, start, e.transform(origin, from, offset));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> Context {
        let ctx = Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, &theme::Appearance::STILL);
        ctx
    }

    fn run(
        ctx: &Context,
        events: Vec<egui::Event>,
        mut f: impl FnMut(&mut Ui),
    ) -> egui::FullOutput {
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1024.0, 640.0))),
                events,
                ..Default::default()
            },
            |ui| f(ui),
        );
        out.textures_delta.clear();
        out
    }

    fn key(k: Key) -> Vec<egui::Event> {
        vec![egui::Event::Key {
            key: k,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        }]
    }

    #[test]
    fn chords_split_into_keys() {
        assert_eq!(split_chord("Ctrl+Shift+F"), ["Ctrl", "Shift", "F"]);
        assert_eq!(split_chord("f"), ["f"]);
        assert_eq!(split_chord("Ctrl++"), ["Ctrl", "+"]);
        assert_eq!(split_chord("+"), ["+"]);
        assert_eq!(chord_keys(Command::Fetch), ["f"]);
    }

    #[test]
    fn floating_surfaces_have_shadow_fill_and_ring() {
        let ctx = context();
        let out = run(&ctx, Vec::new(), |ui| {
            floating(ui, Layer::Menu, Margin::same(4), |ui| {
                ui.label("Item");
            });
        });
        let p = theme::palette(&ctx);
        let s = theme::surfaces(&ctx);
        let mut flat = Vec::new();
        for c in &out.shapes {
            if let Shape::Vec(v) = &c.shape {
                flat.extend(v.iter().cloned());
            }
        }
        let rects: Vec<&RectShape> = flat
            .iter()
            .filter_map(|s| match s {
                Shape::Rect(r) => Some(r),
                _ => None,
            })
            .collect();
        assert!(rects.iter().any(|r| r.blur_width > 0.0), "a soft shadow");
        assert!(
            rects.iter().any(|r| r.fill == s.overlay),
            "the opaque overlay"
        );
        assert!(
            rects
                .iter()
                .any(|r| r.stroke.color == p.floating_ring(s) && r.stroke.width == 1.0),
            "a 1 pt ring"
        );
        assert!(
            rects.iter().all(
                |r| r.corner_radius == CornerRadius::same(theme::metrics().radius_lg)
                    || r.blur_width > 0.0
            ),
            "radius_lg"
        );
    }

    #[test]
    fn exit_snapshots_drop_the_glass_backdrop() {
        let callback = || {
            Shape::Callback(egui::PaintCallback {
                rect: Rect::from_min_size(Pos2::ZERO, Vec2::splat(10.0)),
                callback: Arc::new(()),
            })
        };
        let fill = Shape::rect_filled(
            Rect::from_min_size(Pos2::ZERO, Vec2::splat(10.0)),
            0.0,
            Color32::RED,
        );
        let out = without_callbacks(Shape::Vec(vec![
            callback(),
            fill.clone(),
            Shape::Vec(vec![callback()]),
        ]));
        let Shape::Vec(out) = out else {
            panic!("a list stays a list");
        };
        assert!(matches!(out[0], Shape::Noop));
        assert!(matches!(out[1], Shape::Rect(_)));
        assert!(matches!(&out[2], Shape::Vec(v) if matches!(v[0], Shape::Noop)));
        assert!(matches!(without_callbacks(callback()), Shape::Noop));
        assert!(matches!(without_callbacks(fill), Shape::Rect(_)));
    }

    #[test]
    fn the_glass_hook_is_opaque_without_the_blur() {
        // No OpenGL context in tests, so the blur is unavailable whatever the setting says.
        let ctx = context();
        let _ = run(&ctx, Vec::new(), |ui| {
            let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(50.0));
            assert!(backdrop(ui.painter(), rect, CornerRadius::ZERO).is_none());
        });
    }

    #[test]
    fn entrances_are_instant_with_reduced_motion() {
        let ctx = context();
        let _ = run(&ctx, Vec::new(), |ui| {
            let e = Entrance::of(
                ui.ctx(),
                Id::new("x"),
                motion::BASE,
                motion::SLOW,
                false,
                None,
            );
            assert_eq!(e, Entrance::DONE);
            assert_eq!(
                e.transform(pos2(10.0, 10.0), 0.9, vec2(0.0, 4.0)),
                TSTransform::IDENTITY
            );
        });
        // Nothing is recorded for an exit either.
        let _ = run(&ctx, Vec::new(), |ui| {
            let ctx = ui.ctx();
            record_exit(
                ctx,
                Id::new("x"),
                LayerId::background(),
                0..0,
                0..0,
                ExitMotion::popup(Pos2::ZERO),
            );
            let exits = ctx
                .data(|d| d.get_temp::<Exits>(exits_id()))
                .unwrap_or_default();
            assert!(exits.open.is_empty());
        });
    }

    #[test]
    fn entrances_start_small_and_land_on_identity() {
        let a = Entrance::at(0.0, 0.0);
        assert_eq!(a.fade, 0.0);
        let tf = a.transform(Pos2::ZERO, POPUP_SCALE, vec2(0.0, -POPUP_DROP));
        assert!((tf.scaling - POPUP_SCALE).abs() < 1e-6);
        assert_eq!(tf.translation, vec2(0.0, -POPUP_DROP));
        let done = Entrance::at(1.0, 1.0);
        assert_eq!(
            done.transform(pos2(5.0, 5.0), 0.5, vec2(3.0, 3.0)),
            TSTransform::IDENTITY
        );
    }

    #[test]
    fn closed_layers_fade_out_from_their_last_frame() {
        let ctx = context();
        motion::set_reduced(false);
        let key = Id::new("menu");
        let mut layer = LayerId::background();
        for frame in 0..3 {
            let open = frame < 2;
            let out = run(&ctx, Vec::new(), |root| {
                let ctx = root.ctx().clone();
                if open {
                    egui::Area::new(Id::new("area")).show(&ctx, |ui| {
                        let ctx = ui.ctx().clone();
                        let ctx = &ctx;
                        layer = ui.layer_id();
                        let start = next_shape(ctx, layer);
                        let rect = Rect::from_min_size(pos2(10.0, 10.0), vec2(100.0, 60.0));
                        ui.painter()
                            .extend(surface_shapes(ui.painter(), rect, Layer::Menu));
                        let end = next_shape(ctx, layer);
                        record_exit(
                            ctx,
                            key,
                            layer,
                            0..0,
                            start.0..end.0,
                            ExitMotion::popup(pos2(10.0, 10.0)),
                        );
                    });
                }
            });
            if frame == 2 {
                // Nothing shows the menu any more, yet its surface is still painted, fading.
                assert!(
                    out.shapes
                        .iter()
                        .any(|c| matches!(&c.shape, Shape::Rect(r) if r.blur_width > 0.0)),
                    "the closed menu is still painted while it fades"
                );
            }
        }
        motion::set_reduced(true);
    }

    #[test]
    fn destructive_dialog_focuses_cancel_and_ignores_enter() {
        let ctx = context();
        let show = |ctx: &Context| {
            Dialog::new("delete")
                .title("Delete branch 'x'?")
                .destructive()
                .footer(|f| {
                    f.cancel().confirm("Delete branch");
                })
                .show(ctx)
        };
        let mut result = DialogResult::Open;
        for _ in 0..3 {
            let _ = run(&ctx, Vec::new(), |ui| result = show(ui.ctx()));
        }
        assert_eq!(result, DialogResult::Open);
        let focused = ctx.memory(egui::Memory::focused);
        let _ = run(&ctx, key(Key::Enter), |ui| result = show(ui.ctx()));
        // Enter clicked the focused Cancel button, never Delete.
        assert_ne!(result, DialogResult::Confirm, "focus {focused:?}");
        assert_eq!(result, DialogResult::Cancel);
    }

    #[test]
    fn enter_confirms_and_esc_cancels_a_normal_dialog() {
        for (k, expect) in [
            (Key::Escape, DialogResult::Cancel),
            (Key::Enter, DialogResult::Confirm),
        ] {
            let ctx = context();
            let show = |ctx: &Context| {
                Dialog::new("rename")
                    .title("Rename branch")
                    .footer(|f| {
                        f.cancel().confirm("Rename");
                    })
                    .show(ctx)
            };
            let mut result = DialogResult::Open;
            for _ in 0..3 {
                let _ = run(&ctx, Vec::new(), |ui| result = show(ui.ctx()));
            }
            assert_eq!(result, DialogResult::Open);
            let _ = run(&ctx, key(k), |ui| result = show(ui.ctx()));
            assert_eq!(result, expect, "{k:?}");
        }
    }

    #[test]
    fn a_neutral_choice_ignores_enter_and_a_blocking_dialog_ignores_esc() {
        let ctx = context();
        let show = |ctx: &Context| {
            Dialog::new("consent")
                .title("Send crash reports?")
                .blocking()
                .footer(|f| {
                    f.choice("Send crash reports", "Don't send");
                })
                .show(ctx)
        };
        let mut result = DialogResult::Open;
        for _ in 0..3 {
            let _ = run(&ctx, Vec::new(), |ui| result = show(ui.ctx()));
        }
        assert_eq!(
            ctx.memory(egui::Memory::focused),
            None,
            "neither choice is focused"
        );
        for k in [Key::Enter, Key::Escape] {
            let _ = run(&ctx, key(k), |ui| result = show(ui.ctx()));
            assert_eq!(result, DialogResult::Open, "{k:?}");
        }
        // Both choices are drawn alike: two Secondary buttons, no primary fill.
        let out = run(&ctx, Vec::new(), |ui| result = show(ui.ctx()));
        let p = theme::palette(&ctx);
        let primary = p.surfaces(theme::style()).primary.ends()[0];
        let fills_primary = out.shapes.iter().any(|c| {
            fn has(shape: &Shape, c: Color32) -> bool {
                match shape {
                    Shape::Vec(v) => v.iter().any(|s| has(s, c)),
                    Shape::Rect(r) => r.fill == c,
                    _ => false,
                }
            }
            has(&c.shape, primary)
        });
        assert!(!fills_primary, "no choice is drawn as the primary button");
    }

    #[test]
    fn a_disabled_confirm_ignores_enter() {
        let ctx = context();
        let show = |ctx: &Context| {
            Dialog::new("form")
                .title("Create tag")
                .initial_focus(InitialFocus::Body)
                .footer(|f| {
                    f.cancel()
                        .confirm("Create tag")
                        .confirm_disabled("Name the tag first");
                })
                .show(ctx)
        };
        let mut result = DialogResult::Open;
        for _ in 0..3 {
            let _ = run(&ctx, Vec::new(), |ui| result = show(ui.ctx()));
        }
        let _ = run(&ctx, key(Key::Enter), |ui| result = show(ui.ctx()));
        assert_eq!(result, DialogResult::Open);
    }

    #[test]
    fn dialogs_report_their_role_and_title() {
        let ctx = context();
        ctx.enable_accesskit();
        let mut out = None;
        for _ in 0..2 {
            out = Some(run(&ctx, Vec::new(), |ui| {
                let _ = Dialog::new("delete")
                    .title("Delete branch 'x'?")
                    .destructive()
                    .footer(|f| {
                        f.cancel().confirm("Delete branch");
                    })
                    .show(ui.ctx());
            }));
        }
        let nodes = out
            .and_then(|o| o.platform_output.accesskit_update)
            .expect("accesskit")
            .nodes;
        assert!(
            nodes
                .iter()
                .any(|(_, n)| n.role() == egui::accesskit::Role::AlertDialog
                    && n.label() == Some("Delete branch 'x'?")),
            "the dialog node"
        );
        assert!(
            nodes
                .iter()
                .any(|(_, n)| n.label() == Some("Delete branch"))
        );
    }

    #[test]
    fn popovers_open_from_their_anchor_and_close_on_escape() {
        let ctx = context();
        let mut shown = false;
        let mut anchor_rect = Rect::NOTHING;
        let frame = |events: Vec<egui::Event>, shown: &mut bool, anchor_rect: &mut Rect| {
            let _ = run(&ctx, events, |ui| {
                let r = Button::new("Options").id_salt("anchor").show(ui);
                *anchor_rect = r.rect;
                *shown = Popover::new("opts", &r)
                    .show(|ui| {
                        ui.label("Inside");
                    })
                    .is_some();
            });
        };
        frame(Vec::new(), &mut shown, &mut anchor_rect);
        assert!(!shown);
        let c = anchor_rect.center();
        frame(
            vec![
                egui::Event::PointerMoved(c),
                egui::Event::PointerButton {
                    pos: c,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                },
            ],
            &mut shown,
            &mut anchor_rect,
        );
        frame(
            vec![egui::Event::PointerButton {
                pos: c,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            }],
            &mut shown,
            &mut anchor_rect,
        );
        frame(Vec::new(), &mut shown, &mut anchor_rect);
        assert!(shown, "a click on the anchor opens it");
        frame(key(Key::Escape), &mut shown, &mut anchor_rect);
        frame(Vec::new(), &mut shown, &mut anchor_rect);
        assert!(!shown, "Esc closes it");
    }

    #[test]
    fn forced_popovers_stay_open() {
        let ctx = context();
        let mut shown = false;
        for _ in 0..2 {
            let _ = run(&ctx, Vec::new(), |ui| {
                let r = Button::new("Options").show(ui);
                shown = Popover::new("opts", &r)
                    .force(ForceState::OPEN)
                    .show(|ui| {
                        ui.label("Inside");
                    })
                    .is_some();
            });
        }
        assert!(shown);
    }
}
