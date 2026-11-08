//! The validated navigation mesh, its adjacency and its canonical bytes.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crpg_core::Fx16_16;

use crate::{
    COORD_LIMIT_RAW, MAX_NAV_POLYGONS, MAX_NAV_VERTICES, MAX_POLYGON_VERTICES, NAV_FORMAT_VERSION,
    NAV_MAGIC,
};

/// A point's `x` and `z` as raw `i128`.
type Xz = [i128; 2];

fn xz(v: [Fx16_16; 3]) -> Xz {
    [i128::from(v[0].to_raw()), i128::from(v[2].to_raw())]
}

/// `(a - o) x (b - o)` in `x`/`z`; positive when `b` is left of `o -> a`.
fn cross(o: Xz, a: Xz, b: Xz) -> i128 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

/// `(a - o) . (c - b)` in `x`/`z`.
fn dot(o: Xz, a: Xz, b: Xz, c: Xz) -> i128 {
    (a[0] - o[0]) * (c[0] - b[0]) + (a[1] - o[1]) * (c[1] - b[1])
}

/// Index of a polygon in its mesh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PolyId(pub u32);

/// A validated navigation mesh (fields private).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavMesh {
    vertices: Vec<[Fx16_16; 3]>,
    polygons: Vec<Vec<u32>>,
    neighbours: Vec<Vec<Option<PolyId>>>,
}

/// Why a mesh is invalid; checked in declaration order (B-M1 to B-M9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavMeshError {
    /// The vertex count is outside `3..=MAX_NAV_VERTICES`.
    VertexCount {
        /// The offending count.
        count: usize,
    },
    /// The polygon count is outside `1..=MAX_NAV_POLYGONS`.
    PolygonCount {
        /// The offending count.
        count: usize,
    },
    /// A vertex component exceeds `COORD_LIMIT_RAW` in magnitude.
    VertexOutOfBounds {
        /// Index of the first such vertex.
        vertex: u32,
    },
    /// A vertex equals an earlier one in all three components.
    DuplicateVertex {
        /// Index of the later vertex.
        vertex: u32,
    },
    /// A polygon has fewer than 3 or more than `MAX_POLYGON_VERTICES` vertices.
    PolygonSize {
        /// The polygon id.
        poly: u32,
        /// Its vertex count.
        count: usize,
    },
    /// A polygon names a vertex index past the vertex list.
    VertexIndex {
        /// The polygon id.
        poly: u32,
        /// The first missing index, in list order.
        index: u32,
    },
    /// A polygon repeats a vertex index or a vertex position.
    RepeatedVertex {
        /// The polygon id.
        poly: u32,
    },
    /// A polygon's vertices do not share one `y`.
    NotHorizontal {
        /// The polygon id.
        poly: u32,
    },
    /// A polygon is not convex and counter-clockwise.
    NotConvex {
        /// The polygon id.
        poly: u32,
    },
    /// A polygon repeats a directed edge already used by an earlier polygon.
    DuplicateEdge {
        /// The later polygon id.
        poly: u32,
    },
}

/// Why canonical bytes do not decode (B-C2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavDecodeError {
    /// The bytes end before the form is complete.
    Truncated,
    /// The first four bytes are not `NAV_MAGIC`.
    BadMagic,
    /// The version byte is not `NAV_FORMAT_VERSION`.
    UnsupportedVersion {
        /// The version byte found.
        version: u8,
    },
    /// Bytes remain after the last polygon.
    TrailingBytes,
    /// The bytes decode but the mesh is invalid.
    Invalid(NavMeshError),
}

impl fmt::Display for NavMeshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VertexCount { count } => {
                write!(f, "navmesh vertex count {count} is outside 3..=65536")
            }
            Self::PolygonCount { count } => {
                write!(f, "navmesh polygon count {count} is outside 1..=16384")
            }
            Self::VertexOutOfBounds { vertex } => {
                write!(f, "navmesh vertex {vertex} is outside the coordinate limit")
            }
            Self::DuplicateVertex { vertex } => {
                write!(f, "navmesh vertex {vertex} repeats an earlier vertex")
            }
            Self::PolygonSize { poly, count } => {
                write!(
                    f,
                    "navmesh polygon {poly} has {count} vertices, outside 3..=8"
                )
            }
            Self::VertexIndex { poly, index } => {
                write!(f, "navmesh polygon {poly} names missing vertex {index}")
            }
            Self::RepeatedVertex { poly } => {
                write!(f, "navmesh polygon {poly} repeats a vertex position")
            }
            Self::NotHorizontal { poly } => write!(f, "navmesh polygon {poly} is not horizontal"),
            Self::NotConvex { poly } => {
                write!(
                    f,
                    "navmesh polygon {poly} is not convex and counter-clockwise"
                )
            }
            Self::DuplicateEdge { poly } => {
                write!(f, "navmesh polygon {poly} repeats a directed edge")
            }
        }
    }
}

impl std::error::Error for NavMeshError {}

impl fmt::Display for NavDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => f.write_str("navmesh bytes end early"),
            Self::BadMagic => f.write_str("navmesh bytes do not start with CNAV"),
            Self::UnsupportedVersion { version } => {
                write!(f, "navmesh format version {version} is not supported")
            }
            Self::TrailingBytes => f.write_str("navmesh bytes continue after the last polygon"),
            Self::Invalid(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for NavDecodeError {}

/// Checks one polygon (B-M5 to B-M7). `poly` is its id.
fn check_polygon(
    vertices: &[[Fx16_16; 3]],
    poly: u32,
    indices: &[u32],
) -> Result<(), NavMeshError> {
    let count = indices.len();
    if !(3..=MAX_POLYGON_VERTICES).contains(&count) {
        return Err(NavMeshError::PolygonSize { poly, count });
    }
    if let Some(&index) = indices.iter().find(|&&i| i as usize >= vertices.len()) {
        return Err(NavMeshError::VertexIndex { poly, index });
    }
    let v: Vec<[Fx16_16; 3]> = indices.iter().map(|&i| vertices[i as usize]).collect();
    for i in 0..count {
        for j in (i + 1)..count {
            if indices[i] == indices[j] || xz(v[i]) == xz(v[j]) {
                return Err(NavMeshError::RepeatedVertex { poly });
            }
        }
    }
    if v.iter().any(|p| p[1] != v[0][1]) {
        return Err(NavMeshError::NotHorizontal { poly });
    }
    let p: Vec<Xz> = v.iter().map(|&q| xz(q)).collect();
    let mut strict_left = false;
    for i in 0..count {
        let (prev, next) = ((i + count - 1) % count, (i + 1) % count);
        if (0..count).any(|k| cross(p[i], p[next], p[k]) < 0) {
            return Err(NavMeshError::NotConvex { poly });
        }
        let turn = cross(p[prev], p[i], p[next]);
        if turn == 0 && dot(p[prev], p[i], p[i], p[next]) <= 0 {
            return Err(NavMeshError::NotConvex { poly });
        }
        strict_left |= turn > 0;
    }
    if strict_left {
        Ok(())
    } else {
        Err(NavMeshError::NotConvex { poly })
    }
}

impl NavMesh {
    /// Validates and builds a mesh (B-M1 to B-M9).
    pub fn new(
        vertices: Vec<[Fx16_16; 3]>,
        polygons: Vec<Vec<u32>>,
    ) -> Result<NavMesh, NavMeshError> {
        if !(3..=MAX_NAV_VERTICES).contains(&vertices.len()) {
            return Err(NavMeshError::VertexCount {
                count: vertices.len(),
            });
        }
        if !(1..=MAX_NAV_POLYGONS).contains(&polygons.len()) {
            return Err(NavMeshError::PolygonCount {
                count: polygons.len(),
            });
        }
        let limit = COORD_LIMIT_RAW.unsigned_abs();
        for (i, v) in vertices.iter().enumerate() {
            if v.iter().any(|c| c.to_raw().unsigned_abs() > limit) {
                return Err(NavMeshError::VertexOutOfBounds { vertex: i as u32 });
            }
        }
        let mut seen = BTreeSet::new();
        for (i, v) in vertices.iter().enumerate() {
            if !seen.insert(v.map(Fx16_16::to_raw)) {
                return Err(NavMeshError::DuplicateVertex { vertex: i as u32 });
            }
        }
        for (id, indices) in polygons.iter().enumerate() {
            check_polygon(&vertices, id as u32, indices)?;
        }
        // Directed edge (a, b) -> (polygon, edge index).
        let mut edges: BTreeMap<(u32, u32), (usize, usize)> = BTreeMap::new();
        for (id, indices) in polygons.iter().enumerate() {
            for (e, &a) in indices.iter().enumerate() {
                let b = indices[(e + 1) % indices.len()];
                if edges.insert((a, b), (id, e)).is_some() {
                    return Err(NavMeshError::DuplicateEdge { poly: id as u32 });
                }
            }
        }
        let neighbours = polygons
            .iter()
            .map(|indices| {
                indices
                    .iter()
                    .enumerate()
                    .map(|(e, &a)| {
                        let b = indices[(e + 1) % indices.len()];
                        edges.get(&(b, a)).map(|&(q, _)| PolyId(q as u32))
                    })
                    .collect()
            })
            .collect();
        Ok(NavMesh {
            vertices,
            polygons,
            neighbours,
        })
    }

    /// Every vertex, in index order.
    pub fn vertices(&self) -> &[[Fx16_16; 3]] {
        &self.vertices
    }

    /// Number of polygons.
    pub fn polygon_count(&self) -> usize {
        self.polygons.len()
    }

    /// A polygon's vertex indices, counter-clockwise; `None` past the end.
    pub fn polygon(&self, id: PolyId) -> Option<&[u32]> {
        self.polygons.get(id.0 as usize).map(Vec::as_slice)
    }

    /// A polygon's height; `None` past the end.
    pub fn polygon_y(&self, id: PolyId) -> Option<Fx16_16> {
        let first = *self.polygon(id)?.first()?;
        Some(self.vertices[first as usize][1])
    }

    /// Per edge `i` (vertex `i` to `i + 1`), the polygon across it; `None` past the end.
    pub fn neighbours(&self, id: PolyId) -> Option<&[Option<PolyId>]> {
        self.neighbours.get(id.0 as usize).map(Vec::as_slice)
    }

    /// The canonical byte form (B-C1).
    pub fn to_canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&NAV_MAGIC);
        out.push(NAV_FORMAT_VERSION);
        out.extend_from_slice(&(self.vertices.len() as u32).to_le_bytes());
        out.extend_from_slice(&(self.polygons.len() as u32).to_le_bytes());
        for v in &self.vertices {
            for c in v {
                out.extend_from_slice(&c.to_raw().to_le_bytes());
            }
        }
        for indices in &self.polygons {
            out.push(indices.len() as u8);
            for i in indices {
                out.extend_from_slice(&i.to_le_bytes());
            }
        }
        out
    }

    /// Reads the canonical byte form and validates it (B-C2).
    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<NavMesh, NavDecodeError> {
        let magic = bytes.get(..4).ok_or(NavDecodeError::Truncated)?;
        if magic != NAV_MAGIC {
            return Err(NavDecodeError::BadMagic);
        }
        let version = *bytes.get(4).ok_or(NavDecodeError::Truncated)?;
        if version != NAV_FORMAT_VERSION {
            return Err(NavDecodeError::UnsupportedVersion { version });
        }
        let mut r = Reader { bytes, at: 5 };
        let vertex_count = r.u32()? as usize;
        let polygon_count = r.u32()? as usize;
        if vertex_count > MAX_NAV_VERTICES {
            return Err(NavDecodeError::Invalid(NavMeshError::VertexCount {
                count: vertex_count,
            }));
        }
        if polygon_count > MAX_NAV_POLYGONS {
            return Err(NavDecodeError::Invalid(NavMeshError::PolygonCount {
                count: polygon_count,
            }));
        }
        let mut vertices = Vec::new();
        for _ in 0..vertex_count {
            let x = r.i32()?;
            let y = r.i32()?;
            let z = r.i32()?;
            vertices.push([x, y, z].map(Fx16_16::from_raw));
        }
        let mut polygons = Vec::new();
        for _ in 0..polygon_count {
            let n = r.u8()?;
            let mut indices = Vec::new();
            for _ in 0..n {
                indices.push(r.u32()?);
            }
            polygons.push(indices);
        }
        if r.at != bytes.len() {
            return Err(NavDecodeError::TrailingBytes);
        }
        NavMesh::new(vertices, polygons).map_err(NavDecodeError::Invalid)
    }
}

/// A forward-only byte cursor; every read past the end is `Truncated`.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], NavDecodeError> {
        let end = self.at + N;
        let slice = self
            .bytes
            .get(self.at..end)
            .ok_or(NavDecodeError::Truncated)?;
        self.at = end;
        let mut out = [0u8; N];
        out.copy_from_slice(slice);
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, NavDecodeError> {
        Ok(self.take::<1>()?[0])
    }

    fn u32(&mut self) -> Result<u32, NavDecodeError> {
        Ok(u32::from_le_bytes(self.take()?))
    }

    fn i32(&mut self) -> Result<i32, NavDecodeError> {
        Ok(i32::from_le_bytes(self.take()?))
    }
}
