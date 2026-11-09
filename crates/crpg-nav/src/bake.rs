//! The box bake: a rectangle minus rectangular holes, as a full grid of cells.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crpg_core::Fx16_16;

use crate::{NavMesh, NavMeshError, COORD_LIMIT_RAW};

/// Most holes `bake_box` accepts.
pub const MAX_BAKE_HOLES: usize = 32;

/// An axis-aligned rectangle in `x`/`z`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NavRect {
    /// Smallest `x`.
    pub min_x: Fx16_16,
    /// Smallest `z`.
    pub min_z: Fx16_16,
    /// Largest `x`.
    pub max_x: Fx16_16,
    /// Largest `z`.
    pub max_z: Fx16_16,
}

impl NavRect {
    /// The four values in field order `min_x, min_z, max_x, max_z`, raw.
    fn raw(&self) -> [i32; 4] {
        [self.min_x, self.min_z, self.max_x, self.max_z].map(Fx16_16::to_raw)
    }
}

/// Why a bake failed; checked in declaration order (B-B1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BakeError {
    /// More than `MAX_BAKE_HOLES` holes.
    TooManyHoles {
        /// The number of holes given.
        count: usize,
    },
    /// A height or rectangle value exceeds `COORD_LIMIT_RAW` in magnitude.
    OutOfBounds,
    /// The outer rectangle has no area.
    EmptyOuter,
    /// A hole has no area.
    EmptyHole {
        /// Index of the first such hole.
        hole: usize,
    },
    /// A hole is not within the outer rectangle.
    HoleOutside {
        /// Index of the first such hole.
        hole: usize,
    },
    /// The holes cover the whole outer rectangle.
    NoWalkableArea,
    /// The baked mesh failed validation (cannot happen for inputs passing B-B1).
    Mesh(NavMeshError),
}

impl fmt::Display for BakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyHoles { count } => write!(f, "bake has {count} holes, more than 32"),
            Self::OutOfBounds => f.write_str("bake coordinate is outside the coordinate limit"),
            Self::EmptyOuter => f.write_str("bake outer rectangle is empty"),
            Self::EmptyHole { hole } => write!(f, "bake hole {hole} is empty"),
            Self::HoleOutside { hole } => {
                write!(f, "bake hole {hole} is not inside the outer rectangle")
            }
            Self::NoWalkableArea => f.write_str("bake holes cover the whole outer rectangle"),
            Self::Mesh(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for BakeError {}

/// The sorted distinct values of `edges`.
fn sorted_distinct(edges: impl Iterator<Item = i32>) -> Vec<i32> {
    edges.collect::<BTreeSet<_>>().into_iter().collect()
}

/// Bakes `outer` minus `holes` at height `y` (B-B1 to B-B3).
///
/// The sorted distinct `x` and `z` edges of the outer rectangle and every
/// hole cut the box into cells; each cell no hole covers is one unmerged
/// 4-vertex rectangle, so adjacency is exact and no T-junction arises.
pub fn bake_box(y: Fx16_16, outer: NavRect, holes: &[NavRect]) -> Result<NavMesh, BakeError> {
    if holes.len() > MAX_BAKE_HOLES {
        return Err(BakeError::TooManyHoles { count: holes.len() });
    }
    let limit = COORD_LIMIT_RAW.unsigned_abs();
    let in_bounds = |r: &NavRect| r.raw().iter().all(|c| c.unsigned_abs() <= limit);
    if y.to_raw().unsigned_abs() > limit || !in_bounds(&outer) || !holes.iter().all(in_bounds) {
        return Err(BakeError::OutOfBounds);
    }
    let o = outer.raw();
    if o[0] >= o[2] || o[1] >= o[3] {
        return Err(BakeError::EmptyOuter);
    }
    let hs: Vec<[i32; 4]> = holes.iter().map(NavRect::raw).collect();
    for (hole, h) in hs.iter().enumerate() {
        if h[0] >= h[2] || h[1] >= h[3] {
            return Err(BakeError::EmptyHole { hole });
        }
        if h[0] < o[0] || h[2] > o[2] || h[1] < o[1] || h[3] > o[3] {
            return Err(BakeError::HoleOutside { hole });
        }
    }
    let xs = sorted_distinct(
        [o[0], o[2]]
            .into_iter()
            .chain(hs.iter().flat_map(|h| [h[0], h[2]])),
    );
    let zs = sorted_distinct(
        [o[1], o[3]]
            .into_iter()
            .chain(hs.iter().flat_map(|h| [h[1], h[3]])),
    );

    let mut index: BTreeMap<(i32, i32), u32> = BTreeMap::new();
    let mut vertices: Vec<[Fx16_16; 3]> = Vec::new();
    let mut polygons: Vec<Vec<u32>> = Vec::new();
    for i in 0..xs.len() - 1 {
        for j in 0..zs.len() - 1 {
            let covered = hs
                .iter()
                .any(|h| h[0] <= xs[i] && xs[i + 1] <= h[2] && h[1] <= zs[j] && zs[j + 1] <= h[3]);
            if covered {
                continue;
            }
            let corners = [
                (xs[i], zs[j]),
                (xs[i + 1], zs[j]),
                (xs[i + 1], zs[j + 1]),
                (xs[i], zs[j + 1]),
            ];
            let poly = corners
                .iter()
                .map(|&corner| {
                    *index.entry(corner).or_insert_with(|| {
                        let (x, z) = corner;
                        vertices.push([x, y.to_raw(), z].map(Fx16_16::from_raw));
                        (vertices.len() - 1) as u32
                    })
                })
                .collect();
            polygons.push(poly);
        }
    }
    if polygons.is_empty() {
        return Err(BakeError::NoWalkableArea);
    }
    NavMesh::new(vertices, polygons).map_err(BakeError::Mesh)
}
