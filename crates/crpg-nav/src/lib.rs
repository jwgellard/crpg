#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! Pure, integer navigation: validated convex horizontal polygon meshes, a
//! canonical byte form, and (in later parts) a box bake, queries and paths.
//!
//! This part (T035a) holds the integer geometry helpers, [`NavMesh`] with its
//! validation order and adjacency, and the canonical bytes. Positions are
//! `[Fx16_16; 3]` as `[x, y, z]`, `y` up, metres; only `x` and `z` matter to
//! geometry. There is no float anywhere (ADR-0036 V7), and the only
//! dependency is `crpg-core`.

mod bake;
mod geom;
mod mesh;
mod query;

pub use bake::{bake_box, BakeError, NavRect, MAX_BAKE_HOLES};
pub use crpg_core::Fx16_16;
pub use geom::{ceil_sqrt, seg_len};
pub use mesh::{NavDecodeError, NavMesh, NavMeshError, PolyId};

/// Largest absolute raw coordinate of any point (2^29, 8,192 m; ADR-0036 U3).
pub const COORD_LIMIT_RAW: i32 = 1 << 29;
/// Most points in a path, start included (ADR-0036 U7).
pub const MAX_PATH_POINTS: usize = 64;
/// Longest path, raw U4 length (2^28, 4,096 m; ADR-0036 U7).
pub const MAX_PATH_LENGTH_RAW: u64 = 1 << 28;
/// Most polygons in a mesh.
pub const MAX_NAV_POLYGONS: usize = 16_384;
/// Most vertices in a mesh.
pub const MAX_NAV_VERTICES: usize = 65_536;
/// Most vertices in one polygon.
pub const MAX_POLYGON_VERTICES: usize = 8;
/// First four bytes of the canonical form.
pub const NAV_MAGIC: [u8; 4] = *b"CNAV";
/// Version byte of the canonical form.
pub const NAV_FORMAT_VERSION: u8 = 1;
