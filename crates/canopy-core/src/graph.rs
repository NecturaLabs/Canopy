//! Commit graph layout.
//!
//! Commits arrive newest first in topological order. Each commit is placed in a *lane* (a column)
//! and every row records the line segments to draw in its upper half (from the row above into
//! the node, or straight through) and its lower half (from the node out to its parents' lanes, or
//! straight through). A renderer only needs the row it is drawing, which keeps virtualised
//! scrolling cheap.
//!
//! Rules, in the spirit of Fork and GitKraken:
//! - a commit takes the lane that was waiting for it (the leftmost one if several were);
//! - its first parent continues in the same lane and colour, so a branch reads as one straight line;
//! - each further (merge) parent joins a lane already waiting for it, or opens a new lane with a
//!   new colour;
//! - lanes that were waiting for the same commit converge into it on its row;
//! - freed lanes are reused from the left, so the graph stays narrow.

use git2::Oid;

/// Vertical anchor of a segment end within a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnd {
    /// Top edge of the row.
    Top,
    /// Vertical centre, where the commit node sits.
    Mid,
    /// Bottom edge of the row.
    Bottom,
}

/// One line to draw inside a row, from `(from_lane, from)` to `(to_lane, to)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    /// Lane of the start point.
    pub from_lane: u16,
    /// Vertical anchor of the start point.
    pub from: LineEnd,
    /// Lane of the end point.
    pub to_lane: u16,
    /// Vertical anchor of the end point.
    pub to: LineEnd,
    /// Colour index; renderers take it modulo their palette size.
    pub color: u16,
}

/// Layout of one commit row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GraphRow {
    /// Lane of the commit node.
    pub lane: u16,
    /// Colour index of the commit node.
    pub color: u16,
    /// Lines to draw in this row.
    pub segments: Vec<Segment>,
    /// Number of lanes this row spans (for sizing the graph column).
    pub width: u16,
    /// The commit has more than one parent.
    pub is_merge: bool,
}

#[derive(Debug, Clone, Copy)]
struct Lane {
    oid: Oid,
    color: u16,
}

fn lane_u16(i: usize) -> u16 {
    u16::try_from(i).unwrap_or(u16::MAX)
}

/// Computes one [`GraphRow`] per commit. `commits` yields `(id, parents)` newest first in
/// topological order (every child before its parents).
pub fn layout<'a, I>(commits: I) -> Vec<GraphRow>
where
    I: IntoIterator<Item = (Oid, &'a [Oid])>,
{
    let mut lanes: Vec<Option<Lane>> = Vec::new();
    let mut next_color: u16 = 0;
    let mut rows = Vec::new();

    for (oid, parents) in commits {
        let before = lanes.clone();

        let waiting = lanes.iter().position(|l| l.is_some_and(|l| l.oid == oid));
        let node_lane = match waiting {
            Some(i) => i,
            None => {
                if let Some(i) = lanes.iter().position(Option::is_none) {
                    i
                } else {
                    lanes.push(None);
                    lanes.len() - 1
                }
            }
        };
        let node_color = if let Some(l) = waiting.and_then(|i| lanes[i]) {
            l.color
        } else {
            let c = next_color;
            next_color = next_color.wrapping_add(1);
            c
        };

        let mut segments = Vec::with_capacity(before.len() + parents.len() + 1);
        for (j, lane) in before.iter().enumerate() {
            if let Some(l) = lane {
                let to_lane = if l.oid == oid { node_lane } else { j };
                segments.push(Segment {
                    from_lane: lane_u16(j),
                    from: LineEnd::Top,
                    to_lane: lane_u16(to_lane),
                    to: LineEnd::Mid,
                    color: l.color,
                });
            }
        }

        for lane in &mut lanes {
            if lane.is_some_and(|l| l.oid == oid) {
                *lane = None;
            }
        }

        let mut parent_lanes = Vec::with_capacity(parents.len());
        if let Some(&first) = parents.first() {
            lanes[node_lane] = Some(Lane {
                oid: first,
                color: node_color,
            });
            parent_lanes.push(node_lane);
        }
        for &parent in parents.iter().skip(1) {
            if let Some(k) = lanes
                .iter()
                .position(|l| l.is_some_and(|l| l.oid == parent))
            {
                parent_lanes.push(k);
                continue;
            }
            let k = if let Some(k) = lanes.iter().position(Option::is_none) {
                k
            } else {
                lanes.push(None);
                lanes.len() - 1
            };
            lanes[k] = Some(Lane {
                oid: parent,
                color: next_color,
            });
            next_color = next_color.wrapping_add(1);
            parent_lanes.push(k);
        }

        for (j, lane) in lanes.iter().enumerate() {
            if let Some(l) = lane {
                let from_lane = if parent_lanes.contains(&j) {
                    node_lane
                } else {
                    j
                };
                segments.push(Segment {
                    from_lane: lane_u16(from_lane),
                    from: LineEnd::Mid,
                    to_lane: lane_u16(j),
                    to: LineEnd::Bottom,
                    color: l.color,
                });
            }
        }

        while lanes.last().is_some_and(Option::is_none) {
            lanes.pop();
        }

        let width = before.len().max(lanes.len()).max(node_lane + 1);
        rows.push(GraphRow {
            lane: lane_u16(node_lane),
            color: node_color,
            segments,
            width: lane_u16(width),
            is_merge: parents.len() > 1,
        });
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oid(n: u8) -> Oid {
        let mut bytes = [0u8; 20];
        bytes[19] = n;
        Oid::from_bytes(&bytes).expect("20 bytes")
    }

    fn run(spec: &[(u8, &[u8])]) -> Vec<GraphRow> {
        let owned: Vec<(Oid, Vec<Oid>)> = spec
            .iter()
            .map(|(c, ps)| (oid(*c), ps.iter().map(|p| oid(*p)).collect()))
            .collect();
        layout(owned.iter().map(|(c, ps)| (*c, ps.as_slice())))
    }

    #[test]
    fn linear_history_stays_in_lane_zero() {
        let rows = run(&[(3, &[2]), (2, &[1]), (1, &[])]);
        assert!(rows.iter().all(|r| r.lane == 0 && r.width == 1));
        assert!(rows.iter().all(|r| r.color == rows[0].color));
        // The root has an incoming line but nothing below it.
        assert!(rows[2].segments.iter().all(|s| s.to != LineEnd::Bottom));
        // The tip has nothing above it.
        assert!(rows[0].segments.iter().all(|s| s.from != LineEnd::Top));
    }

    #[test]
    fn merge_opens_a_lane_and_fork_point_closes_it() {
        // 4 = merge(2, 3); 3 -> 1; 2 -> 1; 1 root.
        let rows = run(&[(4, &[2, 3]), (3, &[1]), (2, &[1]), (1, &[])]);
        assert!(rows[0].is_merge);
        assert_eq!(rows[0].lane, 0);
        assert_eq!(rows[1].lane, 1, "second parent goes to a new lane");
        assert_ne!(rows[1].color, rows[0].color, "new lane gets a new colour");
        assert_eq!(rows[2].lane, 0, "first parent stays in the merge's lane");
        assert_eq!(rows[3].lane, 0);
        // On the fork point both lanes converge into the node.
        let converging = rows[3]
            .segments
            .iter()
            .filter(|s| s.from == LineEnd::Top && s.to_lane == 0)
            .count();
        assert_eq!(converging, 2);
    }

    #[test]
    fn unrelated_branch_tip_reuses_free_lane() {
        // Two tips sharing a parent: 3 -> 1, 2 -> 1.
        let rows = run(&[(3, &[1]), (2, &[1]), (1, &[])]);
        assert_eq!(rows[0].lane, 0);
        assert_eq!(rows[1].lane, 1);
        assert_eq!(rows[2].lane, 0);
        assert_eq!(rows[2].width, 2);
        assert_eq!(rows.iter().map(|r| r.width).max(), Some(2));
    }

    #[test]
    fn merge_parent_joins_existing_lane() {
        // 5 -> 4; 4 = merge(3, 2); 6 -> 2 (another branch already waiting for 2).
        let rows = run(&[
            (6, &[2]),
            (5, &[4]),
            (4, &[3, 2]),
            (3, &[1]),
            (2, &[1]),
            (1, &[]),
        ]);
        let merge = &rows[2];
        assert!(merge.is_merge);
        // No third lane was opened for parent 2: it joins lane 0 where 6's line waits.
        assert!(rows.iter().all(|r| r.width <= 2));
        assert!(merge.segments.iter().any(|s| s.from == LineEnd::Mid
            && s.from_lane == merge.lane
            && s.to_lane != merge.lane));
    }

    #[test]
    fn freed_lanes_are_trimmed() {
        let rows = run(&[(4, &[2, 3]), (3, &[1]), (2, &[1]), (1, &[])]);
        assert_eq!(
            rows[3]
                .segments
                .iter()
                .filter(|s| s.to == LineEnd::Bottom)
                .count(),
            0
        );
    }
}
