//! The measurement instrument: [`state_hash`].
//!
//! Same binary + same inputs ⇒ same hash sequence, tick for tick. That claim
//! is what replays, golden files, balance sweeps and the "did my change alter
//! the game" bisect all rest on, and this module is where it becomes
//! checkable. The scope the claim holds under — replay over the exact build,
//! never cross-platform lockstep — is ADR-0009, not this file; this file is
//! the mechanism the ADR governs.
//!
//! ## Hash exclusions: there are none
//!
//! That is a deliberate list, not an omission. Tick and queue bytes hash
//! (proven by test, not by assertion). Adding an exclusion later requires a
//! test proving the excluded field cannot affect behaviour —
//! excluded-but-influential state is how golden tests go blind. Category (a)
//! presentation-only caches pass on test + review; category (b) anything
//! non-deterministic admitted to `World`, or any change to this rule, is an
//! ADR-level change (all per ADR-0009).

use crate::world::World;

/// BLAKE3 over the world's canonical JSON bytes: `[u8; 32]`.
///
/// Canonical today means exactly what the T007 serde impls produce — struct
/// fields in declaration order, stores and timelines as pair-lists in
/// iteration/canonical order, `BTreeMap`/`BTreeSet` internals in key order.
/// No second canonicalizer: if JSON bytes ever stop being canonical, that
/// task proves it with a failing test first.
///
/// Non-finite floats are rejected explicitly before serialization:
/// `serde_json` serializes NaN/infinity as `null` rather than failing, so
/// relying on `.expect()` would silently collide distinct corrupted states.
/// `Transform` fields are public `f32`, so safe code can construct one — a
/// non-finite value in state means corruption or bridge abuse, and panicking
/// is the honest response. Do not add a float-scrubbing pass; fix the
/// producer.
pub fn state_hash(world: &World) -> [u8; 32] {
    for (_, transform) in world.transforms().iter() {
        for value in transform.position.iter().chain(transform.velocity.iter()) {
            assert!(
                value.is_finite(),
                "world state holds only finite floats; see doc comment"
            );
        }
    }
    let bytes =
        serde_json::to_vec(world).expect("world serialization must not fail after finite check");
    blake3::hash(&bytes).into()
}
