//! Canopy's illustrations: the logo mark, the welcome canopy and the empty-state spots, drawn with
//! the painter from the active theme's colours, so every theme gets art in its own palette.
//!
//! Each piece is a small scene of strokes, fills and nodes in its own coordinate box. Strokes use
//! a subset of path syntax (absolute `M`, `L`, `H`, `V`, `C` and `Z`). When an illustration
//! appears it grows in once: strokes draw from their start and nodes pop in as their branch
//! reaches them. After that nothing moves, so an idle screen does not repaint; with reduce motion
//! the finished picture shows at once.

use std::sync::OnceLock;

use egui::epaint::{ColorMode, PathShape, PathStroke};
use egui::{Color32, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};

use super::motion;
use super::theme::{Palette, palette};

/// An illustration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Art {
    /// The Canopy mark, 24×24. Never animated: it is chrome.
    Logo,
    /// The welcome screen canopy, 640×360.
    Welcome,
    /// "Working tree clean", 160×120.
    Clean,
    /// "No commits yet", 160×120.
    History,
    /// A repository with no remote, 160×120.
    LocalOnly,
    /// Something went wrong, 160×120.
    Error,
}

/// The Canopy mark.
pub fn logo() -> Art {
    Art::Logo
}

/// The welcome screen illustration.
pub fn welcome() -> Art {
    Art::Welcome
}

/// "Working tree clean".
pub fn empty_clean() -> Art {
    Art::Clean
}

/// "No commits yet".
pub fn empty_history() -> Art {
    Art::History
}

/// A repository with no remote.
pub fn local_only() -> Art {
    Art::LocalOnly
}

/// Something went wrong.
pub fn empty_error() -> Art {
    Art::Error
}

/// A soft radial halo: `color` at the centre fading to nothing at `radius`.
fn radial_halo(painter: &egui::Painter, center: Pos2, radius: f32, color: Color32) {
    const SEGMENTS: u32 = 64;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(center, color);
    for i in 0..=SEGMENTS {
        #[expect(
            clippy::cast_precision_loss,
            reason = "at most 64 segments, exact in f32"
        )]
        let a = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        mesh.colored_vertex(
            center + vec2(a.cos(), a.sin()) * radius,
            Color32::TRANSPARENT,
        );
    }
    for i in 1..=SEGMENTS {
        mesh.add_triangle(0, i, i + 1);
    }
    painter.add(mesh);
}

/// Draws `art` at `size` points (its aspect ratio is kept, centred in `size`).
pub fn show(ui: &mut Ui, art: Art, size: Vec2) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let scene = scene(art);
    let scale = (size.x / scene.size.x).min(size.y / scene.size.y);
    let origin = rect.center() - scene.size * scale / 2.0;
    let progress = progress(ui, art, resp.id);
    let p = palette(ui.ctx());
    let painter = ui.painter_at(rect.expand(4.0));
    let to_screen = |v: Pos2| origin + v.to_vec2() * scale;
    let breathe = breathing(ui, art);
    for item in &scene.items {
        let local = ((progress - item.at) / item.span.max(0.001)).clamp(0.0, 1.0);
        if local <= 0.0 {
            continue;
        }
        match &item.shape {
            Geometry::Halo { center, radius } => {
                let c = to_screen(*center);
                let r = radius * scale * (0.6 + 0.4 * ease_out(local)) * breathe;
                let color = item.ink.resolve(p, c.y, origin.y, scene.size.y * scale);
                radial_halo(
                    &painter,
                    c,
                    r,
                    Palette::tint(
                        color,
                        crate::ui::saturate_u8(item.alpha * f32::from(color.a())),
                    ),
                );
            }
            Geometry::Ellipse { center, radius } => {
                let c = to_screen(*center);
                let fill = item.ink.resolve(p, c.y, origin.y, scene.size.y * scale);
                let points: Vec<Pos2> = (0..48)
                    .map(|i| {
                        let a = crate::ui::int_f32(i) / 48.0 * std::f32::consts::TAU;
                        c + vec2(a.cos() * radius.x, a.sin() * radius.y) * scale * ease_out(local)
                    })
                    .collect();
                painter.add(Shape::convex_polygon(
                    points,
                    fade(fill, item.alpha * local),
                    Stroke::NONE,
                ));
            }
            Geometry::Node { center, radius } => {
                let c = to_screen(*center);
                let color = item.ink.resolve(p, c.y, origin.y, scene.size.y * scale);
                let r = radius * scale * pop(local);
                painter.circle_filled(c, r, fade(color, item.alpha));
            }
            Geometry::Path { subpaths, closed } => {
                let height = scene.size.y * scale;
                let fill_alpha = item.fill * ease_out(local);
                for sub in subpaths {
                    let pts: Vec<Pos2> = sub.points.iter().map(|v| to_screen(*v)).collect();
                    if *closed && fill_alpha > 0.0 {
                        let color = item.ink.resolve(p, pts[0].y, origin.y, height);
                        fill_star(&painter, &pts, fade(color, fill_alpha));
                    }
                    if item.width > 0.0 {
                        let drawn = partial(&pts, &sub.lengths, ease_in_out(local));
                        stroke(
                            &painter,
                            drawn,
                            item.width * scale,
                            item,
                            p,
                            (origin.y, height),
                            *closed && local >= 1.0,
                        );
                    }
                }
            }
        }
    }
    if progress < 1.0 {
        let frame = ui.ctx().cumulative_frame_nr();
        ui.ctx().data_mut(|d| d.insert_temp(growing_id(), frame));
        ui.ctx().request_repaint();
    }
    resp
}

fn growing_id() -> egui::Id {
    egui::Id::new("art-growing")
}

/// Whether an illustration was still growing in the last frame (screenshots wait for it).
pub fn growing(ctx: &egui::Context) -> bool {
    let frame = ctx.cumulative_frame_nr();
    ctx.data(|d| d.get_temp::<u64>(growing_id()))
        .is_some_and(|last| last + 1 >= frame)
}

/// How far the illustration has grown, from 0 to 1: one second after it first appears, restarting
/// whenever it was not drawn in the frame before.
fn progress(ui: &Ui, art: Art, id: egui::Id) -> f32 {
    if art == Art::Logo || motion::reduced() {
        return 1.0;
    }
    growth(ui, art, id)
}

/// The grow-in timing behind `progress`, whatever the motion settings.
fn growth(ui: &Ui, art: Art, id: egui::Id) -> f32 {
    let now = ui.input(|i| i.time);
    let frame = ui.ctx().cumulative_frame_nr();
    let key = id.with(("art-growth", art));
    let start = ui.ctx().data_mut(|d| {
        let (start, last) = d.get_temp::<(f64, u64)>(key).unwrap_or((now, frame));
        let start = if frame > last + 1 { now } else { start };
        d.insert_temp(key, (start, frame));
        start
    });
    let duration = if art == Art::Welcome { 1.6 } else { 1.0 };
    crate::ui::to_f32(((now - start) / duration).clamp(0.0, 1.0))
}

/// The welcome canopy's halos breathe slowly; the screen already repaints for its drifting glow.
fn breathing(ui: &Ui, art: Art) -> f32 {
    if art != Art::Welcome || motion::reduced() {
        return 1.0;
    }
    1.0 + 0.08 * (crate::ui::to_f32(ui.input(|i| i.time)) * 0.9).sin()
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

fn ease_in_out(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

/// A node popping in: slightly past full size, then settling.
fn pop(t: f32) -> f32 {
    let c = 1.70158;
    let u = t - 1.0;
    1.0 + (c + 1.0) * u * u * u + c * u * u
}

fn fade(c: Color32, alpha: f32) -> Color32 {
    c.gamma_multiply(alpha.clamp(0.0, 1.0))
}

/// The first `t` (0 to 1) of a polyline, by length.
fn partial(pts: &[Pos2], lengths: &[f32], t: f32) -> Vec<Pos2> {
    let total = lengths.last().copied().unwrap_or(0.0);
    if t >= 1.0 || total <= 0.0 {
        return pts.to_vec();
    }
    let goal = total * t;
    let mut out = vec![pts[0]];
    for i in 1..pts.len() {
        if lengths[i] <= goal {
            out.push(pts[i]);
            continue;
        }
        let seg = lengths[i] - lengths[i - 1];
        let f = if seg > 0.0 {
            (goal - lengths[i - 1]) / seg
        } else {
            0.0
        };
        out.push(pts[i - 1] + (pts[i] - pts[i - 1]) * f);
        break;
    }
    out
}

/// Strokes a polyline with round caps, in one colour or a vertical gradient, solid or dashed.
fn stroke(
    painter: &egui::Painter,
    pts: Vec<Pos2>,
    width: f32,
    item: &Item,
    p: &Palette,
    (top, height): (f32, f32),
    closed: bool,
) {
    if pts.len() < 2 {
        return;
    }
    let ink = item.ink;
    let alpha = item.alpha;
    let color_at = move |y: f32| fade(ink.resolve(p, y, top, height), alpha);
    if let Some((dash, gap)) = item.dash {
        let lengths = cumulative(&pts);
        let total = lengths.last().copied().unwrap_or(0.0);
        let mut at = 0.0;
        #[expect(
            clippy::while_float,
            reason = "each step moves a positive dash and gap along the line"
        )]
        while at < total {
            let a = point_at(&pts, &lengths, at);
            let b = point_at(&pts, &lengths, (at + dash).min(total));
            painter.line_segment([a, b], Stroke::new(width, color_at(a.y)));
            cap(painter, a, width, color_at(a.y));
            cap(painter, b, width, color_at(b.y));
            at += dash + gap;
        }
        return;
    }
    let (first, last) = (pts[0], pts[pts.len() - 1]);
    let color = match ink {
        Ink::Gradient(..) => {
            let p = *p;
            ColorMode::UV(std::sync::Arc::new(move |_rect: Rect, pos: Pos2| {
                fade(ink.resolve(&p, pos.y, top, height), alpha)
            }))
        }
        _ => ColorMode::Solid(color_at(first.y)),
    };
    let shape = if closed {
        PathShape::closed_line(
            pts,
            PathStroke {
                width,
                color,
                kind: egui::StrokeKind::Middle,
            },
        )
    } else {
        PathShape::line(
            pts,
            PathStroke {
                width,
                color,
                kind: egui::StrokeKind::Middle,
            },
        )
    };
    painter.add(shape);
    if !closed {
        cap(painter, first, width, color_at(first.y));
        cap(painter, last, width, color_at(last.y));
    }
}

fn cap(painter: &egui::Painter, at: Pos2, width: f32, color: Color32) {
    painter.circle_filled(at, width / 2.0, color);
}

/// Fills a closed outline as a fan from its centre: right for the star-shaped outlines used here
/// (every edge visible from the centre), which convex filling would not be.
fn fill_star(painter: &egui::Painter, pts: &[Pos2], color: Color32) {
    if pts.len() < 3 {
        return;
    }
    let centre =
        pts.iter().fold(Vec2::ZERO, |acc, p| acc + p.to_vec2()) / crate::ui::count_f32(pts.len());
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(centre.to_pos2(), color);
    for p in pts {
        mesh.colored_vertex(*p, color);
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a mesh holds far fewer than 2^32 vertices"
    )]
    let n = pts.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    painter.add(mesh);
}

fn cumulative(pts: &[Pos2]) -> Vec<f32> {
    let mut out = Vec::with_capacity(pts.len());
    let mut total = 0.0;
    for (i, p) in pts.iter().enumerate() {
        if i > 0 {
            total += p.distance(pts[i - 1]);
        }
        out.push(total);
    }
    out
}

fn point_at(pts: &[Pos2], lengths: &[f32], at: f32) -> Pos2 {
    for i in 1..pts.len() {
        if lengths[i] >= at {
            let seg = lengths[i] - lengths[i - 1];
            let f = if seg > 0.0 {
                (at - lengths[i - 1]) / seg
            } else {
                0.0
            };
            return pts[i - 1] + (pts[i] - pts[i - 1]) * f;
        }
    }
    pts[pts.len() - 1]
}

/// Which theme colour a part takes.
#[derive(Debug, Clone, Copy)]
enum Ink {
    /// The brand colour (selection, the primary end of the gradient).
    Accent,
    /// Halfway between the two accents.
    Mid,
    /// The second accent.
    Accent2,
    Success,
    Warning,
    Danger,
    /// Neutral structure: pots, domes, soil, broken lines.
    Neutral,
    /// From `Accent` at the bottom of the scene to `Accent2` at the top, between the two heights
    /// given as fractions of the scene (bottom, top).
    Gradient(f32, f32),
}

impl Ink {
    fn resolve(self, p: &Palette, y: f32, top: f32, height: f32) -> Color32 {
        let mid = p.accent.lerp_to_gamma(p.accent2, 0.5);
        match self {
            Self::Accent => p.accent,
            Self::Mid => mid,
            Self::Accent2 => p.accent2,
            Self::Success => p.success,
            Self::Warning => p.warning,
            Self::Danger => p.danger,
            Self::Neutral => p.text_faint,
            Self::Gradient(bottom, upper) => {
                let f = ((y - top) / height.max(1.0)).clamp(0.0, 1.0);
                let t = ((bottom - f) / (bottom - upper).max(0.001)).clamp(0.0, 1.0);
                if t < 0.55 {
                    p.accent.lerp_to_gamma(mid, t / 0.55)
                } else {
                    mid.lerp_to_gamma(p.accent2, (t - 0.55) / 0.45)
                }
            }
        }
    }
}

enum Geometry {
    Path {
        subpaths: Vec<Polyline>,
        closed: bool,
    },
    Node {
        center: Pos2,
        radius: f32,
    },
    Halo {
        center: Pos2,
        radius: f32,
    },
    Ellipse {
        center: Pos2,
        radius: Vec2,
    },
}

struct Polyline {
    points: Vec<Pos2>,
    lengths: Vec<f32>,
}

struct Item {
    shape: Geometry,
    ink: Ink,
    /// Stroke width in scene units; 0 for no stroke.
    width: f32,
    /// Opacity of the stroke or node.
    alpha: f32,
    /// Opacity of the fill of a closed path (0 for none).
    fill: f32,
    dash: Option<(f32, f32)>,
    /// When the part starts growing and for how long, as fractions of the whole animation.
    at: f32,
    span: f32,
}

struct Scene {
    size: Vec2,
    items: Vec<Item>,
}

/// Builds the scenes once.
fn scene(art: Art) -> &'static Scene {
    static SCENES: OnceLock<[Scene; 6]> = OnceLock::new();
    let all = SCENES.get_or_init(|| {
        [
            logo_scene(),
            welcome_scene(),
            clean_scene(),
            history_scene(),
            local_only_scene(),
            error_scene(),
        ]
    });
    match art {
        Art::Logo => &all[0],
        Art::Welcome => &all[1],
        Art::Clean => &all[2],
        Art::History => &all[3],
        Art::LocalOnly => &all[4],
        Art::Error => &all[5],
    }
}

/// A stroked path. `d` is absolute path syntax (`M L H V C Z`).
fn path(d: &str, ink: Ink, width: f32, at: f32, span: f32) -> Item {
    let (subpaths, closed) = parse(d);
    Item {
        shape: Geometry::Path { subpaths, closed },
        ink,
        width,
        alpha: 1.0,
        fill: 0.0,
        dash: None,
        at,
        span,
    }
}

fn node(x: f32, y: f32, r: f32, ink: Ink, at: f32) -> Item {
    Item {
        shape: Geometry::Node {
            center: pos2(x, y),
            radius: r,
        },
        ink,
        width: 0.0,
        alpha: 1.0,
        fill: 0.0,
        dash: None,
        at,
        span: 0.25,
    }
}

fn halo(x: f32, y: f32, r: f32, ink: Ink, alpha: f32, at: f32) -> Item {
    Item {
        shape: Geometry::Halo {
            center: pos2(x, y),
            radius: r,
        },
        ink,
        width: 0.0,
        alpha,
        fill: 0.0,
        dash: None,
        at,
        span: 0.4,
    }
}

impl Item {
    fn alpha(mut self, a: f32) -> Self {
        self.alpha = a;
        self
    }

    fn fill(mut self, a: f32) -> Self {
        self.fill = a;
        self
    }

    fn dashed(mut self, dash: f32, gap: f32) -> Self {
        self.dash = Some((dash, gap));
        self
    }
}

fn logo_scene() -> Scene {
    let violet = Ink::Accent;
    Scene {
        size: vec2(24.0, 24.0),
        items: vec![
            path("M12 20.5V4", violet, 2.0, 0.0, 1.0),
            path("M12 15.5C12 12 5 13 5 7", Ink::Mid, 2.0, 0.0, 1.0),
            path("M12 15.5C12 12 19 13 19 7", Ink::Accent2, 2.0, 0.0, 1.0),
            node(12.0, 4.0, 2.5, violet, 0.0),
            node(5.0, 7.0, 2.5, Ink::Mid, 0.0),
            node(19.0, 7.0, 2.5, Ink::Accent2, 0.0),
            node(12.0, 20.5, 2.5, violet, 0.0),
        ],
    }
}

fn welcome_scene() -> Scene {
    let g = Ink::Gradient(236.0 / 360.0, 38.0 / 360.0);
    let mut items = vec![
        halo(200.0, 78.0, 44.0, Ink::Accent2, 0.16, 0.55),
        halo(440.0, 78.0, 44.0, Ink::Accent2, 0.16, 0.55),
        halo(320.0, 43.0, 40.0, Ink::Accent2, 0.18, 0.5),
        path("M320 233V43", g, 2.75, 0.0, 0.45).alpha(0.82),
        path("M320 192C320 157 200 171 200 130V78", g, 2.75, 0.12, 0.4).alpha(0.82),
        path("M320 192C320 157 440 171 440 130V78", g, 2.75, 0.12, 0.4).alpha(0.82),
        path("M320 147C320 120 270 123 270 94V57", g, 2.75, 0.2, 0.35).alpha(0.82),
        path("M320 147C320 120 370 123 370 94V57", g, 2.75, 0.2, 0.35).alpha(0.82),
        path("M200 130C200 108 105 141 105 106", g, 2.0, 0.35, 0.3).alpha(0.74),
        path("M200 107C200 91 153 105 153 67", g, 2.0, 0.38, 0.3).alpha(0.74),
        path("M440 130C440 108 535 141 535 106", g, 2.0, 0.35, 0.3).alpha(0.74),
        path("M440 107C440 91 487 105 487 67", g, 2.0, 0.38, 0.3).alpha(0.74),
        path("M270 94C270 74 236 80 236 39", g, 2.0, 0.4, 0.3).alpha(0.74),
        path("M370 94C370 74 404 80 404 39", g, 2.0, 0.4, 0.3).alpha(0.74),
        path("M200 130C239 130 270 123 270 94", g, 2.0, 0.42, 0.25).alpha(0.74),
        path("M440 130C401 130 370 123 370 94", g, 2.0, 0.42, 0.25).alpha(0.74),
        path("M105 106C105 91 66 110 66 84", g, 1.75, 0.58, 0.25).alpha(0.62),
        path("M535 106C535 91 574 110 574 84", g, 1.75, 0.58, 0.25).alpha(0.62),
        path("M153 67C153 49 121 61 121 45", g, 1.75, 0.6, 0.25).alpha(0.62),
        path("M487 67C487 49 519 61 519 45", g, 1.75, 0.6, 0.25).alpha(0.62),
    ];
    for (x, y, r, a, at) in [
        (320.0, 43.0, 5.0, 1.0, 0.42),
        (270.0, 57.0, 4.5, 1.0, 0.52),
        (370.0, 57.0, 4.5, 1.0, 0.52),
        (200.0, 78.0, 5.0, 1.0, 0.5),
        (440.0, 78.0, 5.0, 1.0, 0.5),
        (236.0, 39.0, 3.5, 0.8, 0.68),
        (404.0, 39.0, 3.5, 0.8, 0.68),
        (153.0, 67.0, 3.75, 0.8, 0.66),
        (487.0, 67.0, 3.75, 0.8, 0.66),
        (121.0, 45.0, 3.0, 0.65, 0.74),
        (519.0, 45.0, 3.0, 0.65, 0.74),
    ] {
        items.push(node(x, y, r, Ink::Accent2, at).alpha(a));
    }
    for (x, y, r, a, at) in [
        (200.0, 130.0, 4.0, 1.0, 0.36),
        (440.0, 130.0, 4.0, 1.0, 0.36),
        (270.0, 94.0, 3.5, 1.0, 0.42),
        (370.0, 94.0, 3.5, 1.0, 0.42),
        (105.0, 106.0, 3.75, 0.8, 0.62),
        (535.0, 106.0, 3.75, 0.8, 0.62),
        (66.0, 84.0, 3.0, 0.65, 0.72),
        (574.0, 84.0, 3.0, 0.65, 0.72),
    ] {
        items.push(node(x, y, r, Ink::Mid, at).alpha(a));
    }
    for (y, r, at) in [(147.0, 4.5, 0.22), (192.0, 5.0, 0.12), (233.0, 4.0, 0.0)] {
        items.push(node(320.0, y, r, Ink::Accent, at).alpha(0.86));
    }
    Scene {
        size: vec2(640.0, 250.0),
        items,
    }
}

fn clean_scene() -> Scene {
    let pot = Ink::Accent;
    Scene {
        size: vec2(160.0, 120.0),
        items: vec![
            path("M63 80H97L93 100H67Z", pot, 2.5, 0.0, 0.3).fill(0.08),
            path("M60 80H100", pot, 2.5, 0.05, 0.25),
            path("M75 88H85", pot, 2.5, 0.2, 0.2).alpha(0.55),
            path("M80 79V35", Ink::Success, 2.5, 0.2, 0.4),
            path("M80 66C80 53 58 62 58 43", Ink::Success, 2.5, 0.35, 0.35),
            path("M80 60C80 48 101 56 101 38", Ink::Success, 2.5, 0.4, 0.35),
            node(80.0, 35.0, 4.5, pot, 0.6),
            node(58.0, 43.0, 4.5, pot, 0.7),
            node(101.0, 38.0, 4.5, pot, 0.75),
            path("M37 57V65M33 61H41", Ink::Accent2, 2.5, 0.8, 0.15),
            path("M119 61V69M115 65H123", Ink::Accent2, 2.5, 0.85, 0.15),
            path("M113 23V29M110 26H116", Ink::Accent2, 2.5, 0.9, 0.1),
        ],
    }
}

fn history_scene() -> Scene {
    Scene {
        size: vec2(160.0, 120.0),
        items: vec![
            Item {
                shape: Geometry::Ellipse {
                    center: pos2(80.0, 92.0),
                    radius: vec2(35.0, 8.0),
                },
                ink: Ink::Neutral,
                width: 0.0,
                alpha: 0.14,
                fill: 0.0,
                dash: None,
                at: 0.0,
                span: 0.3,
            },
            path("M49 92C58 85 102 85 111 92", Ink::Neutral, 2.5, 0.0, 0.3),
            path(
                "M80 94C72 90 72 82 80 78C88 82 88 90 80 94Z",
                Ink::Accent,
                2.5,
                0.2,
                0.3,
            )
            .fill(0.18),
            path("M80 78C80 70 76 63 80 55", Ink::Accent, 2.5, 0.45, 0.3),
            path(
                "M80 69C67 70 63 62 65 58C73 57 80 62 80 69Z",
                Ink::Accent,
                2.5,
                0.55,
                0.3,
            )
            .fill(0.15),
            node(80.0, 50.0, 5.0, Ink::Accent2, 0.75),
            path("M80 39V23", Ink::Mid, 2.5, 0.85, 0.15)
                .alpha(0.65)
                .dashed(3.0, 6.0),
        ],
    }
}

fn local_only_scene() -> Scene {
    let tree = Ink::Gradient(85.0 / 120.0, 45.0 / 120.0);
    Scene {
        size: vec2(160.0, 120.0),
        items: vec![
            path("M36 95.5C36 93 38 91 40.5 91H119.5C122 91 124 93 124 95.5C124 98 122 100 119.5 100H40.5C38 100 36 98 36 95.5Z", Ink::Neutral, 2.5, 0.0, 0.3)
                .fill(0.1),
            path("M80 85V45", tree, 2.5, 0.15, 0.35),
            path("M80 72C80 60 61 69 61 54", tree, 2.5, 0.3, 0.3),
            path("M80 72C80 60 99 69 99 54", tree, 2.5, 0.3, 0.3),
            node(80.0, 85.0, 3.5, Ink::Accent, 0.15),
            node(80.0, 45.0, 4.5, Ink::Mid, 0.5),
            node(61.0, 54.0, 4.5, Ink::Accent, 0.58),
            node(99.0, 54.0, 4.5, Ink::Accent2, 0.6),
            path("M41 91V59C41 37 58 20 80 20C102 20 119 37 119 59V91", Ink::Neutral, 2.5, 0.55, 0.4)
                .alpha(0.8),
            path("M50 56C51 44 58 34 69 30", Ink::Neutral, 2.5, 0.85, 0.15).alpha(0.6),
        ],
    }
}

fn error_scene() -> Scene {
    Scene {
        size: vec2(160.0, 120.0),
        items: vec![
            path("M49 95V69C49 47 78 69 88 51", Ink::Neutral, 2.5, 0.0, 0.4),
            path("M49 78C49 68 70 76 70 62", Ink::Neutral, 2.5, 0.15, 0.3),
            node(49.0, 95.0, 4.5, Ink::Mid, 0.0),
            node(70.0, 62.0, 4.0, Ink::Accent, 0.4),
            path(
                "M97 34C88 34 82 41 82 50C82 57 87 62 94 64L98 55L92 50L99 43Z",
                Ink::Danger,
                2.5,
                0.4,
                0.3,
            )
            .fill(0.22),
            path(
                "M105 32L107 41L101 48L108 54L104 62C114 61 120 54 120 46C120 39 114 33 105 32Z",
                Ink::Danger,
                2.5,
                0.45,
                0.3,
            )
            .fill(0.22),
            path("M124 27L128 31L122 35Z", Ink::Danger, 1.5, 0.7, 0.15).fill(1.0),
            path("M124 65L129 67L126 72Z", Ink::Danger, 1.5, 0.72, 0.15).fill(1.0),
            path("M106 19V23M117 21L115 24", Ink::Warning, 2.5, 0.8, 0.15),
            path(
                "M100 45L101.5 48L104 49.5L101.5 51L100 54L98.5 51L96 49.5L98.5 48Z",
                Ink::Warning,
                0.0,
                0.75,
                0.2,
            )
            .fill(1.0),
        ],
    }
}

/// Parses absolute `M L H V C Z` path data into flattened subpaths; whether the path is closed.
fn parse(d: &str) -> (Vec<Polyline>, bool) {
    let mut tokens = Vec::new();
    let mut num = String::new();
    let flush = |num: &mut String, tokens: &mut Vec<Token>| {
        if !num.is_empty() {
            tokens.push(Token::Num(num.parse().unwrap_or(0.0)));
            num.clear();
        }
    };
    for ch in d.chars() {
        match ch {
            'M' | 'L' | 'H' | 'V' | 'C' | 'Z' => {
                flush(&mut num, &mut tokens);
                tokens.push(Token::Cmd(ch));
            }
            '-' if !num.is_empty() => {
                flush(&mut num, &mut tokens);
                num.push(ch);
            }
            ' ' | ',' => flush(&mut num, &mut tokens),
            _ => num.push(ch),
        }
    }
    flush(&mut num, &mut tokens);

    let mut subpaths: Vec<Vec<Pos2>> = Vec::new();
    let mut closed = false;
    let mut cur = Pos2::ZERO;
    let mut cmd = 'M';
    let mut i = 0;
    let nums = |i: &mut usize, n: usize| -> Option<Vec<f32>> {
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            match tokens.get(*i) {
                Some(Token::Num(v)) => out.push(*v),
                _ => return None,
            }
            *i += 1;
        }
        Some(out)
    };
    while i < tokens.len() {
        if let Some(Token::Cmd(c)) = tokens.get(i) {
            cmd = *c;
            i += 1;
            if cmd == 'Z' {
                closed = true;
                continue;
            }
        }
        match cmd {
            'M' => {
                let Some(v) = nums(&mut i, 2) else { break };
                cur = pos2(v[0], v[1]);
                subpaths.push(vec![cur]);
                cmd = 'L';
            }
            'L' => {
                let Some(v) = nums(&mut i, 2) else { break };
                cur = pos2(v[0], v[1]);
                push(&mut subpaths, cur);
            }
            'H' => {
                let Some(v) = nums(&mut i, 1) else { break };
                cur = pos2(v[0], cur.y);
                push(&mut subpaths, cur);
            }
            'V' => {
                let Some(v) = nums(&mut i, 1) else { break };
                cur = pos2(cur.x, v[0]);
                push(&mut subpaths, cur);
            }
            'C' => {
                let Some(v) = nums(&mut i, 6) else { break };
                let (a, b, c) = (pos2(v[0], v[1]), pos2(v[2], v[3]), pos2(v[4], v[5]));
                for s in 1..=16 {
                    let t = crate::ui::int_f32(s) / 16.0;
                    let u = 1.0 - t;
                    let pt = cur.to_vec2() * (u * u * u)
                        + a.to_vec2() * (3.0 * u * u * t)
                        + b.to_vec2() * (3.0 * u * t * t)
                        + c.to_vec2() * (t * t * t);
                    push(&mut subpaths, pt.to_pos2());
                }
                cur = c;
            }
            _ => break,
        }
    }
    let polylines = subpaths
        .into_iter()
        .filter(|s| s.len() > 1)
        .map(|points| {
            let lengths = cumulative(&points);
            Polyline { points, lengths }
        })
        .collect();
    (polylines, closed)
}

fn push(subpaths: &mut Vec<Vec<Pos2>>, p: Pos2) {
    match subpaths.last_mut() {
        Some(s) => s.push(p),
        None => subpaths.push(vec![p]),
    }
}

enum Token {
    Num(f32),
    Cmd(char),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_parse_into_the_expected_shapes() {
        let (subs, closed) = parse("M37 57V65M33 61H41");
        assert!(!closed);
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[0].points, vec![pos2(37.0, 57.0), pos2(37.0, 65.0)]);
        assert_eq!(subs[1].points, vec![pos2(33.0, 61.0), pos2(41.0, 61.0)]);
        let (subs, closed) = parse("M63 80H97L93 100H67Z");
        assert!(closed);
        assert_eq!(subs[0].points.len(), 4);
        let (subs, _) = parse("M12 15.5C12 12 5 13 5 7");
        assert_eq!(subs[0].points.len(), 17);
        assert_eq!(subs[0].points.last(), Some(&pos2(5.0, 7.0)));
    }

    #[test]
    fn every_scene_has_drawable_parts_inside_its_box() {
        for art in [
            Art::Logo,
            Art::Welcome,
            Art::Clean,
            Art::History,
            Art::LocalOnly,
            Art::Error,
        ] {
            let s = scene(art);
            assert!(!s.items.is_empty(), "{art:?}");
            let bounds = Rect::from_min_size(Pos2::ZERO, s.size);
            for item in &s.items {
                assert!(
                    (0.0..=1.0).contains(&(item.at + item.span)),
                    "{art:?} timing"
                );
                if let Geometry::Path { subpaths, .. } = &item.shape {
                    assert!(!subpaths.is_empty(), "{art:?} path parsed");
                    for p in subpaths.iter().flat_map(|s| &s.points) {
                        assert!(bounds.contains(*p), "{art:?} point {p:?} outside");
                    }
                }
            }
        }
    }

    /// Painting must not take the context's lock inside another of its closures (that deadlocks),
    /// and must finish growing and report it.
    #[test]
    fn every_illustration_paints_and_finishes_growing() {
        let ctx = egui::Context::default();
        for frame in 0..4 {
            let input = egui::RawInput {
                time: Some(f64::from(frame)),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                for art in [
                    Art::Logo,
                    Art::Welcome,
                    Art::Clean,
                    Art::History,
                    Art::LocalOnly,
                    Art::Error,
                ] {
                    show(ui, art, vec2(160.0, 120.0));
                }
            });
            output.textures_delta.clear();
            assert!(!output.shapes.is_empty(), "the art painted nothing");
        }
        assert!(!growing(&ctx), "still growing after the animation's length");
    }

    #[test]
    fn a_different_scene_in_the_same_place_grows_in_again() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("spot");
        let mut seen = Vec::new();
        for (time, art) in [(0.0, Art::Clean), (2.0, Art::Clean), (2.5, Art::Error)] {
            let input = egui::RawInput {
                time: Some(time),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| seen.push(growth(ui, art, id)));
            output.textures_delta.clear();
        }
        assert_eq!(seen, [0.0, 1.0, 0.0]);
    }

    #[test]
    fn a_partial_stroke_ends_on_the_line_at_the_fraction_of_its_length() {
        let pts = vec![pos2(0.0, 0.0), pos2(10.0, 0.0), pos2(10.0, 10.0)];
        let lengths = cumulative(&pts);
        assert_eq!(
            partial(&pts, &lengths, 0.25),
            vec![pos2(0.0, 0.0), pos2(5.0, 0.0)]
        );
        assert_eq!(partial(&pts, &lengths, 1.0), pts);
    }
}
