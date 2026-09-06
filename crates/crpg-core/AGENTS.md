# crpg-core — agent contract

Scope note: this file describes the finished core-primitives crate after
T006a-T006e. Later work consumes these types; it does not move higher-level
rules or simulation state into this crate.

## Purpose

Design doc: [`docs/architecture/crpg-core.md`](../../docs/architecture/crpg-core.md)
— what the crate is and how its pieces fit. This file is the working contract:
what you may do and what will break. Decisions live in ADR-0006. Keep the three
linked rather than copied.

The primitives every other crate is allowed to depend on, and nothing else.
`crpg-core` is the bottom of the dependency graph (`core <- data <- rules <-
sim <- ...`) and depends on no workspace crate. Everything here is
deterministic by construction.

That is entity identity — `EntityId` and the `GenerationalArena<T>` that issues
it — plus `Fx16_16` fixed-point arithmetic, `DeterministicRng` with named PCG32
streams, unit-safe simulation counters, authored-object ULIDs, runtime string
interning, and the crate-wide error type.

## Public API  (changing this requires an ADR)

`CoreError`, `Result<T>`, `EntityId`, `GenerationalArena<T>`, `Fx16_16`,
`DeterministicRng`, `Pcg32`, `Tick`, `RoundCount`, `Ulid`, `Interner`,
`Interners`, `StatId`, `TagId`, `EventEnvelope`, `EventQueue`.

`CoreError`: `CorruptArena` and `InvalidEntityId` guard deserialization;
`InvalidFixedPoint` rejects malformed, inexact or out-of-range decimals. The
three ULID variants distinguish wrong length, invalid characters and overflow.

- `EntityId::index() -> u32`, `EntityId::generation() -> u32`. Fields are
  private; only an arena mints an id.
- `GenerationalArena<T>`: `new`, `with_capacity`, `insert`, `remove`, `get`,
  `get_mut`, `contains`, `len`, `is_empty`, `clear`, `iter`, `iter_mut`,
  `ids`, `Default`, `Serialize`/`Deserialize`.

Semantics come from **ADR-0006 Decision 1**, with its exhaustion boundary
superseded by **ADR-0007**. Changing any of them is an ADR, not a refactor:
`crpg-sim`'s `World` (T007), `state_hash` (T008) and every saved campaign
inherit them.

### Fixed-point contract (T006b, ADR-0006 Decision 2)

`fixed::Fx16_16` is also re-exported at the crate root. Its rustdoc lists the
constants, conversions, rounding/sign queries, checked/saturating arithmetic,
operators, assignment operators, `Sum`, `Display`/`FromStr`, and serde API.

1. Arithmetic saturates, never wraps or panics. Zero division returns `MAX`,
   `MIN`, or `ZERO` by numerator sign; checked division returns `None`.
2. Lossy arithmetic floors. `ceil` and `round` are explicit alternatives;
   `round` uses halves away from zero. All three saturate at range edges.
3. Display is shortest exact decimal; parsing rejects precision loss. Serde
   stays the raw `i32`, not the display string.

Keep intermediate arithmetic in `i64`. Do not replace floor division with
`div_euclid`: `EPSILON / from_int(-3)` must be `-EPSILON`, not zero. `Sum`
saturates at each step in iterator order, so regrouping it changes results.
`tests/fixed.rs` pins these behaviours with integer-only property tests.

### RNG contract (T006c, ADR-0006 Decision 3)

`rng::DeterministicRng` and `rng::Pcg32` are re-exported at the crate root.
`DeterministicRng` is the simulation's only randomness source. Callers use
named streams; they do not construct or retain free-standing streams.

1. PCG32-XSH-RR and stream derivation are replay and save-format contracts.
   Changing either changes every later draw and requires an ADR.
2. A stream derives only from the master seed and its name. Creation order and
   draws from other streams cannot affect its sequence.
3. The owning `BTreeMap` is serialized so save/load resumes every touched
   stream, in canonical name order. Do not replace it with insertion ordering.
4. `gen_range_u32` uses rejection sampling; `gen_range_i32` handles the full
   inclusive `i32` domain. The latter returns `lo` without drawing for `lo > hi`.
5. Deserialization rejects an even PCG increment and any owned stream whose
   increment does not match its master seed and name. Loaded state must obey the
   same stream-selection invariant as newly constructed state.

`tests/rng.rs` pins the first 16 outputs for the recorded seed and stream name.
Never re-bless that golden vector casually: first establish why changing every
random decision and replay in the project is intended.

### Time and ULID contract (T006d)

`time::{Tick, RoundCount}` and `ulid::Ulid` are re-exported at the crate root.
`Tick` and `RoundCount` expose only explicit saturating or checked arithmetic;
they deliberately implement no arithmetic operators. `Ulid` stores the
timestamp in the high 48 bits and caller-supplied randomness in the low 80.

ULID display is the canonical 26-character uppercase Crockford base32 form.
Parsing is case-insensitive, accepts `I`/`L` as `1` and `O` as `0`, and rejects
wrong lengths, alphabet violations and values wider than 128 bits distinctly.
Serde uses that string form, not the underlying `u128`.

### Interner contract (T006e, ADR-0006 Decision 4)

`intern::{Interner, Interners, StatId, TagId}` are re-exported at the crate
root. `Interner` assigns dense `u32` handles in first-intern order and
serializes as that ordered string list. `Interners` owns separate stat and tag
tables so crossing namespaces is a type error.

Order is part of `Interner` equality because it determines every handle.
Deserialization rejects duplicate strings rather than collapsing the list and
renumbering later handles.

The handle newtypes expose only `index`; their fields are private and they have
no raw constructors. They are runtime-only and deliberately have no
`Serialize`, `Deserialize`, or `Display` implementation. Persistence resolves
the handle through the issuing interner and stores the string.

### Event substrate contract (T007, ADR-0008)

`event::{EventEnvelope, EventQueue}` are re-exported at the crate root. The
queue is mechanism only: `push` stamps `(tick, seq)`, `drain` yields ascending
`(tick, seq)` stably and clears, serde covers the envelopes plus the sequence
counter with load-time guards (unique `seq`, `next_seq` exceeding the queued
max, saturation at `u64::MAX` excepted). No game variants, no dispatch, no
handlers — those live in `crpg-sim` (`SimEvent`), `crpg-data` (IR types) and
`crpg-rules` (hooks).

Payloads are core-closed types (`EntityId`, `Tick`, integers / `Fx16_16`,
`Ulid`, `String`) by convention and review, not by a compiler bound — a bound
would take a trait, and ADR-0008 forbids new traits. `tests/event.rs` pins
ordering and the round trip with integer payloads.

## Invariants

The arena's five invariants are on the type's rustdoc and are contract, not
implementation detail. Restated here because they are the things a change is
most likely to break silently:

1. **Ascending index order.** `iter`, `iter_mut` and `ids` always yield in
   ascending slot-index order. Consumers rely on this for determinism.
2. **Lowest-index reuse.** `insert` takes the lowest free index, not the most
   recently freed. Reuse depends on the *set* of free slots, never on the
   history that produced it — which is why the free list is a `BTreeSet` and
   not a stack, and why two arenas holding equal entries allocate identically.
3. **Dead ids stay dead.** `remove` bumps the slot generation before the slot
   returns to the free list. A removed `EntityId` never resolves again.
4. **Generations never wrap, and `u32::MAX` is never issued.** It is a
   reserved tombstone. A slot whose generation would *reach* it is retired
   permanently: vacant, not on the free list, generation pinned at `u32::MAX`.
   Every issued generation is in `1..=u32::MAX - 1`, so "generation ==
   `u32::MAX`" means retired with no qualifications — in the live arena and in
   the deserialization guard alike.
5. **`len` counts live entries**, not slots.

Crate-wide:

- Generations start at **1**, so an all-zero `EntityId` is never valid. There
  is no `EntityId::NONE`; absence is `Option<EntityId>`.
- `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]`. Every public item has a
  doc comment (spec §15.6).
- No `HashMap`/`HashSet` — anywhere, including tests. No floats, no clock, no
  threads, no I/O, no randomness that is not seeded and explicit.
- Deserialization is an untrusted-input boundary, at both levels. An arena
  whose slots and free list disagree is rejected with `CoreError::CorruptArena`,
  not loaded — including a *retired* slot found on the free list, and an
  *occupied* slot sitting at the tombstone generation. An `EntityId` whose
  generation is 0 or `u32::MAX` is rejected with `CoreError::InvalidEntityId`,
  so the "generations start at 1, `u32::MAX` is never issued" invariant holds
  for ids that arrived from outside as well as for ids an arena minted. Neither
  check makes a deserialized id *authoritative*: a well-formed id still
  addresses whatever now occupies its slot, and deciding whether a peer may name
  it belongs to the layer that knows who the peer is.
- The float and hash-map bans are lint-enforced here, not just documented:
  `tools/lint/determinism.py` covers `crpg-core` alongside `crpg-rules`.

## Allowed dependencies

`indexmap` (serde), `serde` (derive), `thiserror`. Dev-only: `proptest`,
`serde_json` — both authorised by ADR-0006 and **neither may become a normal
dependency**. No workspace crate, ever. Anything else needs approval.

## Definition of done for any change

```
cargo fmt --all
cargo clippy -p crpg-core --all-targets -- -D warnings
cargo test -p crpg-core
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
```

Any `proptest-regressions/` file a failure produces is **committed**, not
ignored: it is the shrunk counterexample, and losing it loses the regression.

## Known traps

- **`StatId`/`TagId` must never gain `Serialize`, `Deserialize` or `Display`.**
  Persist the string, resolved through the `Interner`. See ADR-0006 Decision 4.
- **`len` is not serialized.** It is recomputed by `TryFrom<ArenaRepr<T>>` on
  deserialization so it cannot disagree with the slots. If you add a field to
  the arena, decide deliberately whether it belongs in the serialized form, and
  extend the `TryFrom` guard if it can be inconsistent.
- **The free list *is* serialized, on purpose.** It is what makes a loaded
  arena allocate the ids the saved one would have. The property test asserts on
  the *next allocation* and not only on equality, so that a change to how the
  free list is represented has to preserve the behaviour, not just the shape —
  keep it that way.
- **Slot state is three-valued**, and the third is easy to miss: vacant-and-free
  (on the free list) versus vacant-and-retired (not on it, generation
  `u32::MAX`). Code that treats "vacant" as "reusable" reintroduces the wrapping
  bug invariant 4 exists to prevent. Vacant-retired-*and*-free is not a state,
  it is corruption (`defect::RETIRED_BUT_FREE`), and so is occupied-at-the-
  tombstone (`defect::OCCUPIED_AT_RETIRED`). **All four arms of that match need
  a generation check**, not just the vacant two.
- **The exhaustion boundary is `u32::MAX - 1`, not `u32::MAX`,** and this is
  where the runtime and the guard previously disagreed. `remove` used to bump a
  slot at `u32::MAX - 1` to the tombstone *and* return it to the free list,
  which produced a live arena that serialized to JSON its own `TryFrom` rejected
  as corrupt, and which would then issue an id at `u32::MAX`. Retiring on
  *reaching* the tombstone is what makes the two agree. `remove` and `clear`
  share one `retire_or_free` helper so they cannot drift apart again, and
  `no_removal_produces_an_arena_that_fails_to_load` is the regression guard:
  every arena a removal can produce must be one the loader accepts. If you touch
  the generation arithmetic, that test and
  `the_last_issuable_generation_is_issued_and_then_retires_the_slot` are the two
  that will notice.
- **Generation retirement is untestable from `tests/`.** Reaching the tombstone
  honestly needs four billion insert/remove pairs, so it is a unit test in
  `src/entity.rs` using a `#[cfg(test)]` `force_generation` helper that reaches
  private state. Do not "simplify" it into an integration test. The helper mints
  only generations an arena could have issued — it rejects `u32::MAX` — so a
  test cannot accidentally assert on an arena that cannot exist. That is exactly
  how the old test managed to cover retirement while missing the boundary: it
  forced the slot to `u32::MAX` and removed, which tests a state no arena
  reaches. Corrupt shapes are built from raw JSON instead, which is how they
  arrive in reality.
- **`EntityId::index()` is not identity.** It is reissued after a removal.
  Anything keying off the index alone is a latent aliasing bug; key off the
  whole id.
- **`clear` is not `*self = new()`.** It keeps the slots and bumps generations,
  which is what keeps ids issued before the clear dead. Replacing the arena
  wholesale would resurrect them.
- **`CoreError` is `#[non_exhaustive]` and deliberately small.** Add a variant
  only for something that genuinely fails. Absence is reported with `Option`
  (`get`, `remove`), because "this id is dead" is an ordinary outcome, not an
  error.
- **`Tick` has no seconds conversion, and adding one is a spec §2.5
  violation.** Tick rate is server configuration and round length is ruleset
  data. Do not add a tick-rate constant, `Duration`, or milliseconds/seconds
  conversion to `Tick` or `RoundCount`.
- **`Ulid` generation does not live here.** Core has no clock or entropy source.
  Callers author ids by supplying both fields to `Ulid::from_parts`; do not add
  `new`, `now`, `SystemTime`, or an RNG-backed constructor.
- **The event queue takes core-closed payloads only.** `EventQueue<P>` is
  generic, so nothing stops you writing `EventQueue<CombatEvent>` — except
  review, this entry, and ADR-0008. A non-core payload compiles and is still a
  layering violation: it smuggles game vocabulary into the primitives crate
  and subjects game churn to the strictest stability contract in the
  workspace. Concrete event types go in `crpg-sim`, `crpg-data` or
  `crpg-rules`, never here.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark + T007 · Added the event-substrate contract, payload trap and Public API names for ADR-0008's scoped core exception.
- 2026-09-06 (UTC) · opencode/muse-spark + event-queue hardening · Validated EventQueue deserialization (unique seq, next_seq exceeding the queued max) and recorded it in the substrate contract.
