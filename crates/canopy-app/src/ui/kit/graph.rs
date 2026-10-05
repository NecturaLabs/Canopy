//! The history graph row painter (design spec 4.23): circle nodes on a ring of the row's own
//! background, hollow merges, the HEAD halo, a dashed WIP node, and edges that are straight
//! within a lane and one vertical-tangent cubic between lanes.
//!
//! `views/history.rs` lays rows out with `canopy_core::graph::layout` and calls [`paint_row`] for
//! each visible row with the row's composite background ([`super::row::ground`]). Nothing here
//! animates: rows snap.

use egui::emath::GuiRounding as _;
use egui::epaint::{CubicBezierShape, PathShape, PathStroke};
use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2};

use canopy_core::{GraphRow, LineEnd};

use crate::ui::theme::{self, Palette};

/// The graph column never grows past this; more lanes than fit share the width.
pub const MAX_WIDTH: f32 = 220.0;

/// Where the lanes of a graph column sit: the first lane's centre from the column's left edge,
/// and the distance between lanes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lanes {
    pub x0: f32,
    pub pitch: f32,
}

impl Lanes {
    /// The lanes for a graph `count` lanes wide in the active metrics (`graph_x0`, `graph_lane`),
    /// squeezed to fit [`MAX_WIDTH`].
    pub fn new(count: u16) -> Self {
        let m = theme::metrics();
        let count = count.max(1);
        let natural = column_width_for(m.graph_x0, m.graph_lane, count);
        let pitch = if natural > MAX_WIDTH && count > 1 {
            (MAX_WIDTH - 2.0 * m.graph_x0) / f32::from(count - 1)
        } else {
            m.graph_lane
        };
        Self {
            x0: m.graph_x0,
            pitch,
        }
    }

    /// The centre x of `lane` in a column whose left edge is `left`.
    pub fn x(self, left: f32, lane: u16) -> f32 {
        left + self.x0 + f32::from(lane) * self.pitch
    }
}

fn column_width_for(x0: f32, lane: f32, count: u16) -> f32 {
    2.0 * x0 + f32::from(count.max(1) - 1) * lane
}

/// The graph column's width for `count` lanes: `history_columns[0]` (76 / 66), growing with the
/// lanes up to [`MAX_WIDTH`].
pub fn column_width(count: u16) -> f32 {
    let m = theme::metrics();
    column_width_for(m.graph_x0, m.graph_lane, count).clamp(m.history_columns[0], MAX_WIDTH)
}

/// How a row's node and edges are marked beyond its layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Marks {
    /// The opaque background of the row (pane, or hover, zebra or selection over it): the node's
    /// ring and a merge's hollow take it, so the ring always matches the row.
    pub ground: Color32,
    /// The checked-out commit: a halo behind the node.
    pub head: bool,
    /// The uncommitted-changes row at the top: a dashed hollow node in `text_faint` with dashed
    /// edges.
    pub wip: bool,
    /// The row under the WIP row: its edge up into the WIP node is dashed too.
    pub below_wip: bool,
}

impl Marks {
    pub fn new(ground: Color32) -> Self {
        Self {
            ground,
            head: false,
            wip: false,
            below_wip: false,
        }
    }
}

fn anchor(rect: Rect, lanes: Lanes, lane: u16, end: LineEnd) -> Pos2 {
    let y = match end {
        LineEnd::Top => rect.top(),
        LineEnd::Mid => rect.center().y,
        LineEnd::Bottom => rect.bottom(),
    };
    pos2(lanes.x(rect.left(), lane), y)
}

/// The shape of one edge from `a` to `b`: a straight line within a lane, else one cubic whose
/// control points sit at the vertical midpoint above `b` and below `a`, so it leaves and enters
/// vertically.
pub fn edge_points(a: Pos2, b: Pos2) -> [Pos2; 4] {
    let my = f32::midpoint(a.y, b.y);
    [a, pos2(a.x, my), pos2(b.x, my), b]
}

/// Points along a short dashed path between `a` and `b` (straight or the edge cubic).
fn dashed(painter: &Painter, a: Pos2, b: Pos2, stroke: Stroke) {
    let pts: Vec<Pos2> = if (a.x - b.x).abs() < 0.5 {
        vec![a, b]
    } else {
        CubicBezierShape::from_points_stroke(
            edge_points(a, b),
            false,
            Color32::TRANSPARENT,
            Stroke::NONE,
        )
        .flatten(Some(0.1))
    };
    painter.extend(Shape::dashed_line(&pts, stroke, 3.0, 3.0));
}

/// The `graph_node_r` circle's centre on `rect`'s row for the node's lane, snapped to device
/// pixels so prerasterised discs stay crisp.
pub fn node_center(painter: &Painter, rect: Rect, lanes: Lanes, lane: u16) -> Pos2 {
    anchor(rect, lanes, lane, LineEnd::Mid).round_to_pixel_center(painter.pixels_per_point())
}

/// Paints one row of the graph: its edges, then its node.
pub fn paint_row(painter: &Painter, rect: Rect, lanes: Lanes, row: &GraphRow, marks: Marks) {
    let p = theme::palette(painter.ctx());
    let m = theme::metrics();
    let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    let width = m.graph_edge;
    let faint = Stroke::new(width, p.text_faint);
    let mut segments: Vec<_> = row.segments.iter().collect();
    segments.sort_by_key(|s| s.from_lane.min(s.to_lane));
    for s in segments {
        let a = anchor(rect, lanes, s.from_lane, s.from);
        let b = anchor(rect, lanes, s.to_lane, s.to);
        let touches_node = |lane: u16, end: LineEnd| lane == row.lane && end == LineEnd::Mid;
        let from_wip = marks.wip && touches_node(s.from_lane, s.from);
        let into_wip = marks.below_wip
            && s.from == LineEnd::Top
            && s.from_lane == row.lane
            && s.to_lane == row.lane;
        if from_wip || into_wip {
            dashed(&painter, a, b, faint);
            continue;
        }
        let color = p.lane(s.color);
        if s.from_lane == s.to_lane {
            painter.line_segment([a, b], Stroke::new(width, color));
        } else {
            painter.add(CubicBezierShape::from_points_stroke(
                edge_points(a, b),
                false,
                Color32::TRANSPARENT,
                PathStroke::new(width, color),
            ));
        }
    }
    paint_node(&painter, rect, lanes, row, marks, p);
}

fn paint_node(
    painter: &Painter,
    rect: Rect,
    lanes: Lanes,
    row: &GraphRow,
    marks: Marks,
    p: &Palette,
) {
    let m = theme::metrics();
    let c = node_center(painter, rect, lanes, row.lane);
    let r = m.graph_node_r;
    let color = p.lane(row.color);
    if marks.head {
        painter.circle_filled(
            c,
            r + m.graph_head_halo,
            Palette::tint(color, m.graph_head_halo_alpha),
        );
    }
    painter.circle_filled(c, r + m.graph_ring, marks.ground);
    if marks.wip {
        let stroke = Stroke::new(m.graph_merge_stroke, p.text_faint);
        // Four dashes, each two thirds of its quarter.
        for q in 0..4 {
            let start = q as f32 * std::f32::consts::FRAC_PI_2 + 0.26;
            let points: Vec<Pos2> = (0..=8)
                .map(|i| {
                    let a = start + i as f32 / 8.0 * std::f32::consts::FRAC_PI_3;
                    c + egui::vec2(a.cos(), a.sin()) * (r - 0.2)
                })
                .collect();
            painter.add(Shape::Path(PathShape::line(points, stroke)));
        }
    } else if row.is_merge {
        painter.circle(
            c,
            r - 0.2,
            marks.ground,
            Stroke::new(m.graph_merge_stroke, color),
        );
    } else {
        painter.circle_filled(c, r, color);
    }
}

// ------------------------------------------------------------------------------------ gallery

/// A synthetic history: lane changes, a merge, an octopus merge, HEAD on a merge and a WIP row.
fn sample() -> Vec<GraphRow> {
    use canopy_core::Oid;
    let oid = |i: u8| Oid::from_bytes(&[i; 20]).unwrap_or(Oid::ZERO_SHA1);
    // (id, parents), newest first. 1 is HEAD (a merge of 2 and 5); 7 is an octopus merge.
    let graph: [(u8, Vec<u8>); 14] = [
        (1, vec![2, 5]),
        (2, vec![3]),
        (3, vec![4]),
        (5, vec![6]),
        (4, vec![7]),
        (6, vec![7]),
        (7, vec![8, 9, 10]),
        (8, vec![11]),
        (9, vec![11]),
        (10, vec![12]),
        (11, vec![12]),
        (12, vec![13]),
        (13, vec![14]),
        (14, vec![]),
    ];
    let ids: Vec<(Oid, Vec<Oid>)> = graph
        .iter()
        .map(|(id, parents)| (oid(*id), parents.iter().map(|p| oid(*p)).collect()))
        .collect();
    let mut rows = vec![GraphRow {
        lane: 0,
        color: 0,
        segments: vec![canopy_core::GraphSegment {
            from_lane: 0,
            from: LineEnd::Mid,
            to_lane: 0,
            to: LineEnd::Bottom,
            color: 0,
        }],
        width: 1,
        is_merge: false,
    }];
    rows.extend(canopy_core::graph::layout(
        ids.iter().map(|(id, parents)| (*id, parents.as_slice())),
    ));
    rows
}

/// The gallery's Graph section: the sample history at Comfortable and Dense metrics side by side,
/// on the pane with every other row selected, hovered or zebra-striped.
pub(crate) fn gallery(ui: &mut egui::Ui) {
    let rows = sample();
    let style = theme::style();
    ui.horizontal_top(|ui| {
        for density in theme::Density::ALL {
            theme::with_look(style, density, || {
                ui.vertical(|ui| {
                    let p = theme::palette(ui.ctx());
                    let m = theme::metrics();
                    let s = p.surfaces(style);
                    ui.label(
                        egui::RichText::new(density.label())
                            .size(m.small_size)
                            .color(p.text_faint),
                    );
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let lanes_n = rows.iter().map(|r| r.width).max().unwrap_or(1);
                    let lanes = Lanes::new(lanes_n);
                    let w = column_width(lanes_n) + 120.0;
                    for (i, row) in rows.iter().enumerate() {
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(w, m.history_row_height),
                            egui::Sense::hover(),
                        );
                        let selected = i == 3;
                        let hover = if i == 5 { 1.0 } else { 0.0 };
                        let zebra = super::row::zebra(i);
                        let ground =
                            super::row::ground(p, s.pane, zebra, selected, hover, false, false);
                        ui.painter().rect_filled(rect, 0.0, ground);
                        let marks = Marks {
                            head: i == 1,
                            wip: i == 0,
                            below_wip: i == 1,
                            ..Marks::new(ground)
                        };
                        paint_row(ui.painter(), rect, lanes, row, marks);
                        let label = match i {
                            0 => "Uncommitted changes".to_owned(),
                            1 => "HEAD: merge".to_owned(),
                            7 => "Octopus merge".to_owned(),
                            _ if row.is_merge => "Merge".to_owned(),
                            _ => format!("Commit {i}"),
                        };
                        ui.painter().text(
                            pos2(rect.left() + column_width(lanes_n) + 4.0, rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            label,
                            egui::FontId::proportional(m.body_size),
                            if i == 0 { p.text_dim } else { p.text },
                        );
                    }
                });
            });
            ui.add_space(theme::XL);
        }
    });
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

    #[test]
    fn lanes_follow_the_density() {
        let _ = ctx();
        let l = Lanes::new(3);
        assert_eq!((l.x0, l.pitch), (18.0, 14.0));
        assert_eq!(l.x(100.0, 2), 100.0 + 18.0 + 28.0);
        assert_eq!(column_width(1), 76.0);
        theme::with_look(Style::Standard, Density::Dense, || {
            let l = Lanes::new(3);
            assert_eq!((l.x0, l.pitch), (15.0, 12.0));
            assert_eq!(column_width(1), 66.0);
        });
    }

    #[test]
    fn wide_graphs_squeeze_into_the_maximum_width() {
        let _ = ctx();
        assert_eq!(column_width(40), MAX_WIDTH);
        let l = Lanes::new(40);
        assert!(l.pitch < 14.0);
        assert!((l.x(0.0, 39) + l.x0 - MAX_WIDTH).abs() < 1e-3);
        assert!(column_width(4) > 76.0 && column_width(4) < MAX_WIDTH);
    }

    #[test]
    fn edges_leave_and_enter_vertically() {
        let [a, c1, c2, b] = edge_points(pos2(10.0, 0.0), pos2(38.0, 28.0));
        assert_eq!((c1.x, c1.y), (10.0, 14.0));
        assert_eq!((c2.x, c2.y), (38.0, 14.0));
        assert_eq!((a, b), (pos2(10.0, 0.0), pos2(38.0, 28.0)));
    }

    /// The circles one row paints: (radius, fill, stroke colour).
    fn circles(row: &GraphRow, marks: Marks) -> Vec<(f32, Color32, Color32)> {
        let ctx = ctx();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let rect = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(76.0, 28.0));
            paint_row(ui.painter(), rect, Lanes::new(1), row, marks);
        });
        out.textures_delta.clear();
        out.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                Shape::Circle(s) => Some((s.radius, s.fill, s.stroke.color)),
                _ => None,
            })
            .collect()
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "a test ground colour, not one of the theme's"
    )]
    fn nodes_are_rings_of_the_row_ground() {
        let p = theme::ThemeId::Midnight.palette();
        let ground = Color32::from_rgb(1, 2, 3);
        let row = GraphRow {
            lane: 0,
            color: 2,
            segments: Vec::new(),
            width: 1,
            is_merge: false,
        };
        let lane = p.lane(2);
        assert_eq!(
            circles(&row, Marks::new(ground)),
            vec![
                (6.2, ground, Color32::TRANSPARENT),
                (4.2, lane, Color32::TRANSPARENT)
            ]
        );
        let merge = GraphRow {
            is_merge: true,
            ..row
        };
        let hollow = circles(&merge, Marks::new(ground));
        assert_eq!(hollow[1].1, ground, "a merge is hollow");
        assert_eq!(hollow[1].2, lane);
        let head = circles(
            &merge,
            Marks {
                head: true,
                ..Marks::new(ground)
            },
        );
        assert_eq!(head.len(), 3, "halo, ring, hollow node");
        assert!((head[0].0 - (4.2 + 5.2)).abs() < 1e-4);
        assert_eq!(head[0].1, Palette::tint(lane, 61));
    }

    #[test]
    fn the_sample_has_merges_lane_changes_and_an_octopus() {
        let rows = sample();
        assert_eq!(rows.len(), 15);
        assert!(rows[1].is_merge, "HEAD is a merge");
        assert!(rows.iter().filter(|r| r.is_merge).count() >= 2);
        assert!(
            rows.iter()
                .flat_map(|r| &r.segments)
                .any(|s| s.from_lane != s.to_lane)
        );
        assert!(
            rows.iter().any(|r| r.width >= 3),
            "the octopus opens three lanes"
        );
    }
}
