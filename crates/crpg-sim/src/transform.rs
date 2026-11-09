//! Spatial state: [`Transform`].
//!
//! Positions and velocities live in `f32`, outside any rules path (spec §2.4,
//! E006-A). Since T036 (ADR-0036 U2, M3) a `Transform` is the `f32` *mirror*
//! of a mover's authoritative `Fx16_16` position, written by movement code
//! only and read by no rule; nothing converts an `f32` back. Anything a rule reads — damage, modifiers, DCs, durations — uses
//! integers or `Fx16_16` and lives in `crpg-rules`, never here. In particular,
//! no constructor, helper or test in this module may smuggle in an `f64`:
//! the determinism lint bans it crate-wide as `no-f64`.
//!
//! Rotation and facing stay deferred (ADR-0036 D46f): movement is positional
//! and the mirror carries none.

use serde::{Deserialize, Serialize};

/// Where an entity is and where it is going, in world units.
///
/// For a creature with a [`Mover`](crate::Mover) this is the movers' mirror:
/// `position` is `to_f32` of the authoritative position and `velocity` the
/// last tick's displacement, both rewritten by movement code only.
///
/// Both fields are plain arrays, not a vector type: there is no math library
/// in the dependency set and a three-float array serializes canonically with
/// no help.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    /// World-space position.
    pub position: [f32; 3],
    /// World-space velocity, same units per tick.
    pub velocity: [f32; 3],
}
