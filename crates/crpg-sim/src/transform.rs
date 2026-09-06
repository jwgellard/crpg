//! Spatial state: [`Transform`].
//!
//! Positions and velocities live in `f32`, outside any rules path (spec §2.4,
//! E006-A). Anything a rule reads — damage, modifiers, DCs, durations — uses
//! integers or `Fx16_16` and lives in `crpg-rules`, never here. In particular,
//! no constructor, helper or test in this module may smuggle in an `f64`:
//! the determinism lint bans it crate-wide as `no-f64`.
//!
//! Rotation is deliberately absent. It arrives with the movement task, which
//! is the first consumer that can say what representation it needs.

use serde::{Deserialize, Serialize};

/// Where an entity is and where it is going, in world units.
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
