//! Paths: A* over polygons, then an integer funnel (ADR-0036 V4, V5).

use std::collections::BTreeSet;
use std::fmt;

use crpg_core::Fx16_16;

use crate::{seg_len, NavMesh, PolyId, MAX_PATH_LENGTH_RAW, MAX_PATH_POINTS};

/// A position as `[x, y, z]`.
type P3 = [Fx16_16; 3];

/// A walkable path: at least two points and no zero-length segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavPath {
    points: Vec<P3>,
    length: u64,
}

impl NavPath {
    /// The points, `from` first.
    pub fn points(&self) -> &[[Fx16_16; 3]] {
        &self.points
    }

    /// Consumes the path into its points (the host copies them into `MoveRoute`).
    pub fn into_points(self) -> Vec<[Fx16_16; 3]> {
        self.points
    }

    /// Sum of U4 segment lengths.
    pub fn length(&self) -> u64 {
        self.length
    }
}

/// Why no path was returned; checked in declaration order (B-P1, B-P8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavError {
    /// The start is not on the mesh.
    StartOffMesh,
    /// The goal is not on the mesh.
    GoalOffMesh,
    /// Start and goal have the same `x` and `z`.
    AlreadyThere,
    /// The goal is in a part of the mesh the start cannot reach.
    Unreachable,
    /// The path needs more than `MAX_PATH_POINTS` points.
    PathTooComplex {
        /// How many points the path needs.
        points: usize,
    },
    /// The path is longer than `MAX_PATH_LENGTH_RAW`.
    TooLong {
        /// The path's raw U4 length.
        length: u64,
    },
}

impl fmt::Display for NavError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NavError::StartOffMesh => write!(f, "path start is off the navmesh"),
            NavError::GoalOffMesh => write!(f, "path goal is off the navmesh"),
            NavError::AlreadyThere => write!(f, "path start and goal are the same point"),
            NavError::Unreachable => write!(f, "path goal cannot be reached from the start"),
            NavError::PathTooComplex { points } => {
                write!(f, "path needs {points} points, more than {MAX_PATH_POINTS}")
            }
            NavError::TooLong { length } => {
                write!(f, "path length {length} exceeds {MAX_PATH_LENGTH_RAW}")
            }
        }
    }
}

impl std::error::Error for NavError {}

/// A portal between two corridor polygons; `left` is the end of the shared
/// edge and `right` its start, as the polygon being left holds it.
#[derive(Clone, Copy)]
struct Portal {
    left: P3,
    right: P3,
}

/// `(a - o) x (b - o)` in `x`/`z`; positive when `b` is left of `o -> a`.
fn cross(o: P3, a: P3, b: P3) -> i128 {
    let c = |v: P3, i: usize| i128::from(v[i].to_raw());
    (c(a, 0) - c(o, 0)) * (c(b, 2) - c(o, 2)) - (c(a, 2) - c(o, 2)) * (c(b, 0) - c(o, 0))
}

fn same(a: P3, b: P3) -> bool {
    a[0] == b[0] && a[2] == b[2]
}

impl NavMesh {
    /// The floor centroid of a polygon's vertices (sums in `i64`).
    fn centroid(&self, id: PolyId) -> P3 {
        let idx = self.polygon(id).unwrap_or(&[]);
        let n = idx.len().max(1) as i64;
        let (mut sx, mut sz) = (0i64, 0i64);
        for &i in idx {
            let v = self.vertices()[i as usize];
            sx += i64::from(v[0].to_raw());
            sz += i64::from(v[2].to_raw());
        }
        let raw = |s: i64| Fx16_16::from_raw(s.div_euclid(n) as i32);
        [raw(sx), Fx16_16::from_raw(0), raw(sz)]
    }

    /// B-P3: the polygon corridor from `s` to `g`, or `None` when unreachable.
    fn corridor(&self, s: PolyId, g: PolyId, from: P3, goal: P3) -> Option<Vec<PolyId>> {
        let count = self.polygon_count();
        let pt = |id: PolyId| {
            if id == s {
                from
            } else if id == g {
                goal
            } else {
                self.centroid(id)
            }
        };
        let h = |id: PolyId| seg_len(pt(id), goal);
        let mut cost: Vec<Option<u64>> = vec![None; count];
        let mut parent: Vec<Option<PolyId>> = vec![None; count];
        let mut closed = vec![false; count];
        let mut open: BTreeSet<(u64, u64, u32)> = BTreeSet::new();
        cost[s.0 as usize] = Some(0);
        open.insert((h(s), h(s), s.0));
        while let Some((_, _, raw_id)) = open.pop_first() {
            let cur = PolyId(raw_id);
            closed[cur.0 as usize] = true;
            if cur == g {
                let mut path = vec![g];
                let mut at = g;
                while let Some(prev) = parent[at.0 as usize] {
                    path.push(prev);
                    at = prev;
                }
                path.reverse();
                return Some(path);
            }
            let base = cost[cur.0 as usize].unwrap_or(0);
            for q in self
                .neighbours(cur)
                .unwrap_or(&[])
                .iter()
                .flatten()
                .copied()
            {
                let qi = q.0 as usize;
                if closed[qi] {
                    continue;
                }
                let c = base + seg_len(pt(cur), pt(q));
                let old = cost[qi];
                if old.is_none_or(|o| c < o) {
                    if let Some(o) = old {
                        open.remove(&(o + h(q), h(q), q.0));
                    }
                    cost[qi] = Some(c);
                    parent[qi] = Some(cur);
                    open.insert((c + h(q), h(q), q.0));
                }
            }
        }
        None
    }

    /// B-P4: the portals of a corridor, `from` and `goal` as degenerate ends.
    fn portals(&self, corridor: &[PolyId], from: P3, goal: P3) -> Vec<Portal> {
        let mut portals = vec![Portal {
            left: from,
            right: from,
        }];
        for pair in corridor.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let edge = self
                .neighbours(a)
                .and_then(|n| n.iter().position(|&x| x == Some(b)));
            let poly = self.polygon(a).unwrap_or(&[]);
            if let (Some(e), false) = (edge, poly.is_empty()) {
                let vertex = |k: usize| self.vertices()[poly[k % poly.len()] as usize];
                portals.push(Portal {
                    left: vertex(e + 1),
                    right: vertex(e),
                });
            }
        }
        portals.push(Portal {
            left: goal,
            right: goal,
        });
        portals
    }

    /// A* over polygons then an integer funnel (B-P1 to B-P8).
    ///
    /// The path starts at `from` exactly and ends at `to` in `x`/`z`, with the
    /// goal polygon's height; every segment lies inside the mesh. Shortest
    /// paths are not promised.
    pub fn find_path(&self, from: [Fx16_16; 3], to: [Fx16_16; 3]) -> Result<NavPath, NavError> {
        let s = self.locate(from).ok_or(NavError::StartOffMesh)?;
        let g = self.locate(to).ok_or(NavError::GoalOffMesh)?;
        if same(from, to) {
            return Err(NavError::AlreadyThere);
        }
        let goal = [to[0], self.polygon_y(g).unwrap_or(to[1]), to[2]];
        let corridor = if s == g {
            vec![s]
        } else {
            self.corridor(s, g, from, goal)
                .ok_or(NavError::Unreachable)?
        };
        let raw = funnel(&self.portals(&corridor, from, goal));
        let mut points = vec![raw[0]];
        for &q in &raw[1..] {
            if points.last().is_some_and(|&last| !same(last, q)) {
                points.push(q);
            }
        }
        if points.len() > MAX_PATH_POINTS {
            return Err(NavError::PathTooComplex {
                points: points.len(),
            });
        }
        let length: u64 = points.windows(2).map(|w| seg_len(w[0], w[1])).sum();
        if length > MAX_PATH_LENGTH_RAW {
            return Err(NavError::TooLong { length });
        }
        Ok(NavPath { points, length })
    }
}

/// B-P5: the simple stupid funnel with exact `i128` cross products.
fn funnel(portals: &[Portal]) -> Vec<P3> {
    let first = portals[0];
    let (mut apex, mut left, mut right) = (first.left, first.left, first.right);
    let (mut left_i, mut right_i) = (0usize, 0usize);
    let mut raw = vec![apex];
    let mut i = 1;
    while i < portals.len() {
        let Portal { left: l, right: r } = portals[i];
        if cross(apex, right, r) >= 0 {
            if same(apex, right) || cross(apex, left, r) < 0 {
                right = r;
                right_i = i;
            } else {
                raw.push(left);
                apex = left;
                (left, right, right_i) = (apex, apex, left_i);
                i = left_i + 1;
                continue;
            }
        }
        if cross(apex, left, l) <= 0 {
            if same(apex, left) || cross(apex, right, l) > 0 {
                left = l;
                left_i = i;
            } else {
                raw.push(right);
                apex = right;
                (left, right, left_i) = (apex, apex, right_i);
                i = right_i + 1;
                continue;
            }
        }
        i += 1;
    }
    raw.push(portals[portals.len() - 1].left);
    raw
}
