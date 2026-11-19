#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! The engine's first AI: a pure, read-only decision for one creature in an
//! encounter (T056a).
//!
//! [`decide`] picks the next step for one actor — attack the nearest hostile
//! it may legally attack, walk toward the nearest hostile, or end the turn /
//! hold — using only the legality the players use: [`crpg_sim::legal_actions`]
//! and its one validation path [`crpg_sim::validate_action`]. It never mutates
//! the world and never calls a host operation; the host driver applies each
//! [`Decision`] through its own trusted operations and revalidates.
//!
//! Hostility is caller-supplied ([`Sides`]): sim, rules and core know nothing
//! of sides, so the driver hands the AI the table it built from authored data.
//!
//! A move is a destination the host will accept, checked with the host's own
//! route ([`host_route`]) and the same movement validation the host runs.
//! [`point_along`] is the integer walk along a route that picks the standoff
//! point.
//!
//! The crate follows the simulation crates' determinism rules by contract: no
//! `f32`/`f64`, no `HashMap`/`HashSet`, no clock, thread or randomness, no
//! interior mutability, and every distance is sim's integer U4 measure. See
//! `docs/architecture/crpg-ai.md` and `crates/crpg-ai/AGENTS.md`.

mod mvp;
mod route;
mod sides;

pub use mvp::{decide, Decision};
pub use route::{host_route, point_along, RouteError, AI_ROUTE_TAG, STANDOFF_MARGIN_RAW};
pub use sides::{SideId, Sides, SidesError};
