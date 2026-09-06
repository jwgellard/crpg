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
/// The `.expect()` on serialization is correct, not lazy: `serde_json`
/// refuses only non-finite floats, and no sim API can produce one — no
/// division, no parsing, no FFI input — so a NaN in world state means memory
/// corruption or bridge abuse, and panicking is the honest response. Do not
/// add a float-scrubbing pass.
pub fn state_hash(world: &World) -> [u8; 32] {
    let bytes =
        serde_json::to_vec(world).expect("world state holds only finite floats; see doc comment");
    blake3::hash(&bytes).into()
}
