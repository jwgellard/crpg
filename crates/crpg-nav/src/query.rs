//! Point queries: `locate` and `nearest_on_mesh` (ADR-0036 V4).

use crpg_core::Fx16_16;

use crate::{NavMesh, PolyId};

/// A point's `x` and `z` as raw `i128`.
type Xz = [i128; 2];

fn xz(v: [Fx16_16; 3]) -> Xz {
    [i128::from(v[0].to_raw()), i128::from(v[2].to_raw())]
}

/// `(a - o) x (b - o)` in `x`/`z`; positive when `b` is left of `o -> a`.
fn cross(o: Xz, a: Xz, b: Xz) -> i128 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

/// Squared horizontal distance.
fn d2(a: Xz, b: Xz) -> i128 {
    (a[0] - b[0]) * (a[0] - b[0]) + (a[1] - b[1]) * (a[1] - b[1])
}

/// Whether `q` is inside or on the boundary of the counter-clockwise polygon `v`.
fn contains(v: &[Xz], q: Xz) -> bool {
    (0..v.len()).all(|i| cross(v[i], v[(i + 1) % v.len()], q) >= 0)
}

/// B-Q5: the candidate of edge `a -> b` of polygon `v` for the point `p`.
fn edge_candidate(v: &[Xz], a: Xz, b: Xz, p: Xz) -> Xz {
    let (ex, ez) = (b[0] - a[0], b[1] - a[1]);
    let t = (p[0] - a[0]) * ex + (p[1] - a[1]) * ez;
    let l2 = ex * ex + ez * ez;
    if t <= 0 {
        return a;
    }
    if t >= l2 {
        return b;
    }
    let (px, pz) = (ex * t, ez * t);
    let fx = a[0] + px.div_euclid(l2);
    let cx = fx + i128::from(px.rem_euclid(l2) != 0);
    let fz = a[1] + pz.div_euclid(l2);
    let cz = fz + i128::from(pz.rem_euclid(l2) != 0);
    let mut best: Option<(i128, Xz)> = None;
    for c in [[fx, fz], [fx, cz], [cx, fz], [cx, cz]] {
        if contains(v, c) {
            let d = d2(c, p);
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, c));
            }
        }
    }
    match best {
        Some((_, c)) => c,
        None if d2(a, p) <= d2(b, p) => a,
        None => b,
    }
}

/// A coordinate inside the mesh's bounding box, back to raw. Every candidate
/// is a vertex or a contained lattice point, so the clamp never changes it.
fn raw_i32(v: i128) -> i32 {
    v.clamp(i128::from(i32::MIN), i128::from(i32::MAX)) as i32
}

impl NavMesh {
    /// A polygon's vertices as `x`/`z`; `None` past the end.
    fn polygon_xz(&self, id: PolyId) -> Option<Vec<Xz>> {
        let indices = self.polygon(id)?;
        Some(
            indices
                .iter()
                .map(|&i| xz(self.vertices()[i as usize]))
                .collect(),
        )
    }

    /// The lowest polygon containing `p` in `x`/`z`, boundary included (B-Q2).
    pub fn locate(&self, p: [Fx16_16; 3]) -> Option<PolyId> {
        let q = xz(p);
        (0..self.polygon_count() as u32)
            .map(PolyId)
            .find(|&id| self.polygon_xz(id).is_some_and(|v| contains(&v, q)))
    }

    /// The nearest point on the mesh in `x`/`z`, with the polygon's `y` (B-Q3 to B-Q5).
    ///
    /// On the mesh the answer is `p` at the polygon's height. Off it, the
    /// answer is the nearest edge candidate; the lowest polygon id, then the
    /// lowest edge index, wins a tie, so `locate` of the answer is `Some`.
    pub fn nearest_on_mesh(&self, p: [Fx16_16; 3]) -> [Fx16_16; 3] {
        let q = xz(p);
        if let Some(y) = self.locate(p).and_then(|id| self.polygon_y(id)) {
            return [p[0], y, p[2]];
        }
        let mut best: Option<(i128, Xz, Fx16_16)> = None;
        for id in (0..self.polygon_count() as u32).map(PolyId) {
            let (Some(v), Some(y)) = (self.polygon_xz(id), self.polygon_y(id)) else {
                continue;
            };
            for i in 0..v.len() {
                let c = edge_candidate(&v, v[i], v[(i + 1) % v.len()], q);
                let d = d2(c, q);
                if best.is_none_or(|(bd, _, _)| d < bd) {
                    best = Some((d, c, y));
                }
            }
        }
        match best {
            Some((_, c, y)) => [
                Fx16_16::from_raw(raw_i32(c[0])),
                y,
                Fx16_16::from_raw(raw_i32(c[1])),
            ],
            // A validated mesh has at least one polygon, so this is unreachable.
            None => p,
        }
    }
}
