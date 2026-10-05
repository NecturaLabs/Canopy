//! Painting primitives the kit's components share: layered shadows, gradient fills clipped to a
//! rounded rect, the inner top highlight and bottom line, the field recess, focus rings, 1 pt
//! seams snapped to the pixel grid, Deeper's floating panes with their focus edge and top deck, and
//! scroll fades. Colours come in from the caller, which reads them from the theme.

use egui::epaint::tessellator::path::rounded_rectangle;
use egui::epaint::{CornerRadiusF32, PathShape, PathStroke, RectShape};
use egui::{
    Color32, Context, CornerRadius, Id, Painter, Pos2, Rangef, Rect, Shape, Stroke, StrokeKind,
    TextureHandle, TextureOptions, pos2, vec2,
};

use crate::ui::theme::{self, Elevation, Fill, Material, Palette, Style, Surfaces};

/// Width of a seam, edge or ring (1 pt, as concept 1's 1 px borders).
pub const SEAM: f32 = 1.0;
/// Outside focus ring: gap between the control and the ring, and the ring's width.
pub const FOCUS_OFFSET: f32 = 2.0;
pub const FOCUS_WIDTH: f32 = 2.0;
/// Inset focus ring of rows, tabs, segments and menu items.
pub const INSET_FOCUS_WIDTH: f32 = 1.5;

/// The layers of `material` under a surface at `rect`, largest first. Paint before the surface.
/// A negative spread shrinks the rect (egui's `Shadow::spread` is unsigned); a blur wider than the
/// rect's shorter side is cut to it, as egui's tessellator would.
pub fn shadow(painter: &Painter, rect: Rect, radius: CornerRadius, p: &Palette, m: &Material) {
    painter.extend(shadow_shapes(rect, radius, p, m));
}

/// The shapes [`shadow`] paints, for surfaces painted into a reserved slot once their size is
/// known (floating layers).
pub fn shadow_shapes(rect: Rect, radius: CornerRadius, p: &Palette, m: &Material) -> Vec<Shape> {
    let mut shapes = Vec::with_capacity(m.layers.len());
    for l in m.layers {
        let r = rect
            .translate(vec2(0.0, f32::from(l.dy)))
            .expand(f32::from(l.spread));
        if r.width() <= 0.0 || r.height() <= 0.0 {
            continue;
        }
        let blur = f32::from(l.blur).min((r.size().min_elem() - 1.0).max(0.0));
        let spread = f32::from(l.spread);
        let corner = if spread >= 0.0 {
            radius + CornerRadius::same(spread as u8)
        } else {
            shrink_radius(radius, -spread)
        };
        let color = Palette::tint(p.shadow_color(l.tint), l.alpha);
        shapes.push(
            RectShape::filled(r, corner, color)
                .with_blur_width(blur)
                .into(),
        );
    }
    shapes
}

fn shrink_radius(r: CornerRadius, by: f32) -> CornerRadius {
    let by = by.round().clamp(0.0, 255.0) as u8;
    CornerRadius {
        nw: r.nw.saturating_sub(by),
        ne: r.ne.saturating_sub(by),
        sw: r.sw.saturating_sub(by),
        se: r.se.saturating_sub(by),
    }
}

/// A 1 x 32 ramp from `top` to `bottom`, uploaded once per pair of colours and reused.
fn ramp(ctx: &Context, top: Color32, bottom: Color32) -> TextureHandle {
    let id = Id::new(("canopy-kit-ramp", top, bottom));
    if let Some(t) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return t;
    }
    const N: usize = 32;
    let pixels = (0..N)
        .map(|i| top.lerp_to_gamma(bottom, i as f32 / (N - 1) as f32))
        .collect();
    let image = egui::ColorImage::new([1, N], pixels);
    let handle = ctx.load_texture("canopy-kit-ramp", image, TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, handle.clone()));
    handle
}

/// The shape of a vertical fill: one flat rect, or a ramp-textured rect (Deeper's gradients) with
/// true rounded corners and the tessellator's anti-aliasing.
pub fn fill_shape(ctx: &Context, rect: Rect, radius: CornerRadius, fill: Fill) -> Shape {
    if fill.is_flat() {
        return RectShape::filled(rect, radius, fill.top).into();
    }
    let tex = ramp(ctx, fill.top, fill.bottom);
    RectShape::filled(rect, radius, Color32::WHITE)
        .with_texture(tex.id(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)))
        .into()
}

/// Paints `fill` in `rect`.
pub fn fill(painter: &Painter, rect: Rect, radius: CornerRadius, fill: Fill) {
    painter.add(fill_shape(painter.ctx(), rect, radius, fill));
}

/// A 1 px light line just inside the top edge, inset by the corner radius (white at `alpha`).
pub fn top_highlight(painter: &Painter, rect: Rect, radius: CornerRadius, alpha: u8) {
    if alpha == 0 {
        return;
    }
    let inset = f32::from(radius.nw.max(radius.ne)).max(1.0);
    let ppp = painter.pixels_per_point();
    let w = 1.0 / ppp;
    let y = rect.top() + w * 0.5;
    painter.hline(
        Rangef::new(rect.left() + inset, rect.right() - inset),
        y,
        Stroke::new(w, Color32::from_white_alpha(alpha)),
    );
}

/// A 1 px dark line just inside the bottom edge (light Deeper controls).
pub fn bottom_line(painter: &Painter, rect: Rect, radius: CornerRadius, color: Color32, alpha: u8) {
    if alpha == 0 {
        return;
    }
    let inset = f32::from(radius.sw.max(radius.se)).max(1.0);
    let w = 1.0 / painter.pixels_per_point();
    painter.hline(
        Rangef::new(rect.left() + inset, rect.right() - inset),
        rect.bottom() - w * 0.5,
        Stroke::new(w, Palette::tint(color, alpha)),
    );
}

/// An inner shadow along the top edge (Deeper's field recess): three 1 pt rows fading down, clipped
/// to the rounded rect.
pub fn recess(painter: &Painter, rect: Rect, radius: CornerRadius, color: Color32, alpha: u8) {
    if alpha == 0 {
        return;
    }
    let mut pts = Vec::new();
    rounded_rectangle(&mut pts, rect.shrink(0.5), CornerRadiusF32::from(radius));
    let top = rect.top();
    let stroke = PathStroke::new_uv(1.5, move |_bbox: Rect, p: Pos2| {
        let k = (1.0 - (p.y - top) / 4.0).clamp(0.0, 1.0);
        Palette::tint(color, (f32::from(alpha) * k) as u8)
    })
    .inside();
    painter.add(Shape::Path(PathShape {
        points: pts,
        closed: true,
        fill: Color32::TRANSPARENT,
        stroke,
    }));
}

/// Everything a surface of `level` paints in `style`: its shadow layers, the fill, the top
/// highlight, the bottom line, the recess and a 1 pt ring inside its edge (`ring`, if any).
#[expect(clippy::too_many_arguments, reason = "one call paints a whole surface")]
pub fn surface(
    painter: &Painter,
    rect: Rect,
    radius: CornerRadius,
    p: &Palette,
    style: Style,
    level: Elevation,
    body: Fill,
    ring: Option<Color32>,
) {
    let m = p.material(style, level);
    shadow(painter, rect, radius, p, &m);
    fill(painter, rect, radius, body);
    top_highlight(painter, rect, radius, m.highlight);
    bottom_line(painter, rect, radius, p.shadow, m.bottom_line);
    recess(painter, rect, radius, p.shadow, m.recess);
    if let Some(c) = ring {
        rim(painter, rect, radius, c);
    }
}

/// A 1 pt ring inside `rect`'s edge.
pub fn rim(painter: &Painter, rect: Rect, radius: CornerRadius, color: Color32) {
    painter.rect_stroke(rect, radius, Stroke::new(SEAM, color), StrokeKind::Inside);
}

/// The outside focus ring: `FOCUS_WIDTH` in `color`, `FOCUS_OFFSET` outside the control with the
/// gap filled by `moat` (the surface behind). The painter must not clip outside `rect`.
pub fn focus_ring(
    painter: &Painter,
    rect: Rect,
    radius: CornerRadius,
    color: Color32,
    moat: Color32,
) {
    let grow = |by: f32| radius + CornerRadius::same(by.round() as u8);
    if moat.a() > 0 {
        painter.rect_stroke(
            rect.expand(FOCUS_OFFSET * 0.5),
            grow(FOCUS_OFFSET * 0.5),
            Stroke::new(FOCUS_OFFSET, moat),
            StrokeKind::Middle,
        );
    }
    painter.rect_stroke(
        rect.expand(FOCUS_OFFSET + FOCUS_WIDTH * 0.5),
        grow(FOCUS_OFFSET),
        Stroke::new(FOCUS_WIDTH, color),
        StrokeKind::Middle,
    );
}

/// The inset focus ring of rows, tabs, segments and menu items.
pub fn inset_ring(painter: &Painter, rect: Rect, radius: CornerRadius, color: Color32) {
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(INSET_FOCUS_WIDTH, color),
        StrokeKind::Inside,
    );
}

/// A horizontal 1 pt seam at `y`, snapped to the pixel grid.
pub fn seam_h(painter: &Painter, x: Rangef, y: f32, color: Color32) {
    let ppp = painter.pixels_per_point();
    let stroke = Stroke::new(SEAM, color);
    let mut y = y;
    stroke.round_center_to_pixel(ppp, &mut y);
    painter.hline(x, y, stroke);
}

/// A vertical 1 pt seam at `x`, snapped to the pixel grid.
pub fn seam_v(painter: &Painter, x: f32, y: Rangef, color: Color32) {
    let ppp = painter.pixels_per_point();
    let stroke = Stroke::new(SEAM, color);
    let mut x = x;
    stroke.round_center_to_pixel(ppp, &mut x);
    painter.vline(x, y, stroke);
}

/// Deeper's focused-pane edge: a 1 pt ring inside the pane, `focus` at `theme::FOCUS_EDGE`'s alphas
/// from the top down, scaled by `t` (its fade-in).
pub fn focus_edge(painter: &Painter, rect: Rect, radius: CornerRadius, focus: Color32, t: f32) {
    if t <= 0.0 {
        return;
    }
    let mut pts = Vec::new();
    rounded_rectangle(&mut pts, rect.shrink(0.5), CornerRadiusF32::from(radius));
    let (top, h) = (rect.top(), rect.height().max(1.0));
    let stroke = PathStroke::new_uv(SEAM, move |_bbox: Rect, p: Pos2| {
        let y = ((p.y - top) / h).clamp(0.0, 1.0);
        Palette::tint(focus, (focus_edge_alpha(y) * t) as u8)
    })
    .inside();
    painter.add(Shape::Path(PathShape {
        points: pts,
        closed: true,
        fill: Color32::TRANSPARENT,
        stroke,
    }));
}

/// The focused-pane edge's alpha at `y` (0 at the top of the pane, 1 at the bottom).
pub fn focus_edge_alpha(y: f32) -> f32 {
    let stops = theme::FOCUS_EDGE;
    for w in stops.windows(2) {
        let ((y0, a0), (y1, a1)) = (w[0], w[1]);
        if y <= y1 {
            let k = ((y - y0) / (y1 - y0)).clamp(0.0, 1.0);
            return f32::from(a0) + (f32::from(a1) - f32::from(a0)) * k;
        }
    }
    f32::from(stops[stops.len() - 1].1)
}

/// A content pane in the active style. Standard: a flush `fill` with nothing else. Deeper: a
/// floating card (`Elevation::Pane`, `pane_edge` ring, `pane_radius`) and, while it holds focus
/// (`focus` 0 to 1), the focused-pane edge. `rect` is the card itself (the caller leaves the
/// gutters). Returns the radius it painted with, for content clipping.
pub fn pane(
    painter: &Painter,
    rect: Rect,
    p: &Palette,
    fill_color: Color32,
    focus: f32,
) -> CornerRadius {
    let style = theme::style();
    let s = p.surfaces(style);
    let m = theme::metrics();
    let radius = CornerRadius::same(m.pane_radius);
    if style == Style::Standard {
        painter.rect_filled(rect, CornerRadius::ZERO, fill_color);
        return CornerRadius::ZERO;
    }
    surface(
        painter,
        rect,
        radius,
        p,
        style,
        Elevation::Pane,
        Fill::flat(fill_color),
        s.pane_edge,
    );
    focus_edge(painter, rect, radius, p.focus, focus);
    radius
}

/// The top deck (the repository tab strip and the action bar as one surface). Standard: the flat
/// chrome with a seam at the bottom. Deeper: a lifted gradient with its own highlight and a
/// shadow onto the ground, edged by `deck_edge`.
pub fn deck(painter: &Painter, rect: Rect, p: &Palette, s: &Surfaces) {
    let style = theme::style();
    if style == Style::Standard {
        painter.rect_filled(rect, CornerRadius::ZERO, s.deck.top);
        seam_h(
            painter,
            rect.x_range(),
            rect.bottom() - SEAM * 0.5,
            s.deck_edge,
        );
        return;
    }
    let m = p.material(style, Elevation::Deck);
    shadow(painter, rect, CornerRadius::ZERO, p, &m);
    fill(painter, rect, CornerRadius::ZERO, s.deck);
    let w = 1.0 / painter.pixels_per_point();
    if m.highlight > 0 {
        painter.hline(
            rect.x_range(),
            rect.top() + w * 0.5,
            Stroke::new(w, Color32::from_white_alpha(m.highlight)),
        );
    }
    seam_h(
        painter,
        rect.x_range(),
        rect.bottom() - SEAM * 0.5,
        s.deck_edge,
    );
}

/// Fades the top and bottom edges of a vertical scroll area into `surface` where more content lies
/// beyond them (12 pt, 85%), so a row cut by the viewport reads as "scroll for more".
pub fn scroll_fade<R>(
    ui: &egui::Ui,
    output: &egui::scroll_area::ScrollAreaOutput<R>,
    surface: Color32,
) {
    let rect = output.inner_rect;
    let hidden_above = output.state.offset.y;
    let hidden_below = output.content_size.y - rect.height() - output.state.offset.y;
    let painter = ui.painter_at(rect);
    let band = theme::MD.min(rect.height() / 4.0);
    let surface = surface.gamma_multiply(0.85);
    let clear = Color32::TRANSPARENT;
    let fade = |top: f32, from: Color32, to: Color32| {
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(pos2(rect.left(), top), from);
        mesh.colored_vertex(pos2(rect.right(), top), from);
        mesh.colored_vertex(pos2(rect.right(), top + band), to);
        mesh.colored_vertex(pos2(rect.left(), top + band), to);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(mesh);
    };
    if hidden_above > 1.0 {
        fade(rect.top(), surface, clear);
    }
    if hidden_below > 1.0 {
        fade(rect.bottom() - band, clear, surface);
    }
}

/// A spinner: a 270 degree arc turning once a second around `center`, `color` at full strength
/// over a faint full circle. Repaints itself while painted; still (one fixed arc) with reduced
/// motion.
pub fn spinner(painter: &Painter, center: Pos2, radius: f32, width: f32, color: Color32) {
    let ctx = painter.ctx();
    let still = crate::ui::motion::reduced();
    let turn = if still {
        0.0
    } else {
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
        (ctx.input(|i| i.time) % 1.0) as f32 * std::f32::consts::TAU
    };
    painter.circle_stroke(
        center,
        radius,
        Stroke::new(width, color.gamma_multiply(0.25)),
    );
    let points: Vec<Pos2> = (0..=24)
        .map(|i| {
            let a = turn + i as f32 / 24.0 * std::f32::consts::TAU * 0.75;
            center + vec2(a.cos(), a.sin()) * radius
        })
        .collect();
    painter.add(Shape::line(points, Stroke::new(width, color)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_edge_fades_from_the_top() {
        assert_eq!(focus_edge_alpha(0.0), 191.0);
        assert!((focus_edge_alpha(0.38) - 46.0).abs() < 0.01);
        assert_eq!(focus_edge_alpha(0.7), 0.0);
        assert_eq!(focus_edge_alpha(1.0), 0.0);
        assert!(focus_edge_alpha(0.2) < 191.0 && focus_edge_alpha(0.2) > 46.0);
    }

    #[test]
    fn negative_spread_shrinks_the_shadow() {
        let ctx = Context::default();
        let p = theme::ThemeId::Midnight.palette();
        let rect = Rect::from_min_size(pos2(10.0, 10.0), vec2(100.0, 40.0));
        let mut out = ctx.run_ui(Default::default(), |ui| {
            let m = p.material(Style::Standard, Elevation::Floating);
            shadow(ui.painter(), rect, CornerRadius::same(12), p, &m);
        });
        out.textures_delta.clear();
        let shapes: Vec<Rect> = out
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                Shape::Rect(r) => Some(r.rect),
                _ => None,
            })
            .collect();
        // (10, 28, -8): moved down 10 and 8 smaller on every side.
        assert!(
            shapes.contains(&Rect::from_min_max(pos2(18.0, 28.0), pos2(102.0, 52.0))),
            "{shapes:?}"
        );
    }
}
