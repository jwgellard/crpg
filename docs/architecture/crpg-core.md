# crpg-core — architecture

The primitives every other crate may depend on, and nothing else.

**State:** the core primitives are complete (T006a-T006e): entity identity, the
crate error type, fixed-point maths, the named-stream RNG, simulation time
counters, authored-object ULIDs, and runtime string interning.

Decisions: [ADR-0006](../adr/0006-crpg-core-primitives.md) and
[ADR-0007](../adr/0007-reserved-arena-generation.md).
Working contract: [`crates/crpg-core/AGENTS.md`](../../crates/crpg-core/AGENTS.md).

---

## Position

`crpg-core` is the bottom of the dependency graph. It depends on no workspace
crate — `tools/lint/deps.py` enforces that with an empty allowed-edge set, the
only crate in the table that has one — and its only external dependencies are
`indexmap`, `serde` and `thiserror`, plus `proptest` and `serde_json` as
dev-only.

Everything above it inherits its semantics, which is why the semantics were
settled in an ADR before any of it was written. Four consumers in particular:

- **`crpg-sim` (T007)** — `World`'s entity arena *is*
  `GenerationalArena<EntityMeta>`, not a second implementation. It inherits the
  property tests and the serde round trip.
- **`crpg-sim` (T008)** — `state_hash` is defined over canonical serialization,
  so anything here that serializes in a load-order-dependent way would make the
  project's central measurement instrument meaningless.
- **`crpg-rules` (T014)** — every number a rule reads is an integer or
  `Fx16_16`. The rounding rule is decided here, once.
- **Saves and, later, the wire** — every type here crosses an untrusted-input
  boundary eventually.

The consequence worth stating plainly: **changes here are not refactors.** A
change to arena iteration order, fixed-point rounding, or RNG stream derivation
is a silent behaviour change in every existing save file and golden hash. That
is what "changing this requires an ADR" in the crate's `AGENTS.md` means.

## Determinism, structurally

The crate has no clock, no threads, no I/O, and no randomness that is not
seeded and explicit. That is not a coding style; it is the reason the crate can
sit under an authoritative server and a replay harness at the same time.

Two mechanisms hold it:

- **`tools/lint/determinism.py`** covers `crpg-core` alongside `crpg-rules` and
  `crpg-sim` — no `HashMap`/`HashSet`, no wall clock, no threads, no external
  RNG, and no floats. It scans doctests as well as `tests/` and `#[cfg(test)]`
  modules, because a doctest is compiled and run.
- **Ordered collections only.** `BTreeSet` for the arena's free list,
  `BTreeMap` for the RNG's streams. Both are canonical by construction, so two
  logically identical values serialize to identical bytes regardless of the
  order the program touched them in. `IndexMap` preserves *insertion* order,
  which is history rather than content, and history is what makes two runs
  diverge.

## Modules

```
src/
  lib.rs      pub mod + pub use + the crate doc comment, nothing else
  error.rs    CoreError, Result<T>
  entity.rs   EntityId, GenerationalArena<T>          (T006a)
  event.rs    EventEnvelope, EventQueue               (T007, ADR-0008)
  fixed.rs    Fx16_16                                (T006b)
  intern.rs   Interner, Interners, StatId, TagId     (T006e)
  rng.rs      DeterministicRng, Pcg32                (T006c)
  time.rs     Tick, RoundCount                       (T006d)
  ulid.rs     Ulid                                   (T006d)
```

`lib.rs` stays a declaration and crate-root export file on purpose.

`Ulid` is a separate module from `Tick` and `RoundCount` deliberately: they are
all "time" colloquially, but `Tick` and `RoundCount` are simulation counters
while a `Ulid` is an identifier that happens to embed a timestamp, and its
timestamp and randomness are both supplied by the caller rather than read from
a clock.

### `error.rs`

One enum for the whole crate, `#[non_exhaustive]` so later modules can add
variants without a breaking change. It is deliberately small: absence is
reported with `Option`, not an error, because "this id is dead" is an ordinary
outcome. `CorruptArena` and `InvalidEntityId` guard entity deserialization;
`InvalidFixedPoint` guards exact decimal parsing.

### `event.rs`

Generic `(tick, seq)` ordering, stable drain and pair-plus-counter serde —
mechanism only, per ADR-0008. The payload bound is over fields, not the enum
(ADR-0011): every field core-closed, the vocabulary enum owned above, so
`World.events: EventQueue<SimEvent>` is the sanctioned instance. Loading
rejects duplicate sequence numbers and a `next_seq` that does not exceed the
queued max (saturation excepted), so a later push cannot reuse a number.

### `entity.rs`

`EntityId` is `{ index: u32, generation: u32 }`, private fields, minted only by
an arena. `GenerationalArena<T>` is the allocator that issues and validates
them. They live together because id-reuse safety is a property of the
*allocator*, not of the id struct — separating them leaves nothing to test
(ADR-0006 Decision 1).

The arena's five invariants are on the type's rustdoc and restated in
`AGENTS.md`; the two with consequences beyond the module:

- **Ascending-index iteration** is contract, not implementation. Every
  consumer's iteration order is part of the simulation's determinism.
- **Lowest-index reuse**, with the free list in a `BTreeSet`, means allocation
  depends on the *set* of free slots and never on the history that produced it.
  Two arenas holding equal entries allocate identically. This is also why the
  free list is serialized: a loaded arena must issue the ids the saved one
  would have.

**Generation exhaustion.** `u32::MAX` is a reserved tombstone that is never
issued; a slot whose generation would reach it is retired permanently rather
than wrapped. Reserving the value rather than issuing it is what lets
"generation == `u32::MAX`" mean *retired* in the live arena and in the
deserialization guard alike. An earlier version retired on overflow instead, so
a slot at `u32::MAX - 1` could be bumped to the tombstone *and* returned to the
free list — a live arena that serialized to a save its own loader rejected as
corrupt. Reachable only after 2³² removals of one slot, and fixed anyway,
because an invariant that holds "except at one unreachable value" is not an
invariant.

**Deserialization is an untrusted-input boundary.** Both types load through a
`TryFrom` over a private representation struct with `deny_unknown_fields`,
rather than a derived impl: an arena whose slots and free list disagree is
refused, and an `EntityId` at generation 0 or `u32::MAX` is refused. That is
well-formedness only. It does **not** establish that whoever sent an id is
entitled to name that entity — a well-formed id still addresses whatever now
occupies its slot, and authority belongs to the layer that knows who the sender
is (`crpg-net`, T018 onward).

### `fixed.rs`

`Fx16_16` is a `pub` struct over a private raw `i32` newtype with 16
fractional bits. It has no dependency on entity storage. Consumers use it
wherever rules need fractions without introducing floating-point arithmetic
(ADR-0006 Decision 2).

The module contains arithmetic and exact decimal conversion. Wide intermediates
feed either a range check (`checked_*`) or a clamp (operators and
`saturating_*`). Division shares an explicit floor adjustment for either
divisor sign. The named `ceil` and `round` methods offer deliberate alternatives
to flooring; out-of-range rounded integers still saturate.

Serde exposes only the raw integer for snapshots and hashing. `Display` and
`FromStr` are separate exact-decimal paths; the parser cancels decimal factors
before binary scaling to avoid overflowing its intermediate. A human-friendly
serde adapter still belongs to `crpg-data`, not core. Arithmetic and conversion
properties live in `tests/fixed.rs` and need no floating-point oracle.

### `rng.rs`

`DeterministicRng` owns the master seed and every lazily-created `Pcg32` stream.
The stream map is a `BTreeMap`, so its serialized order depends on names rather
than first-use history. Serialization includes each stream's complete 16-byte
state and resumes at the identical next draw.

Deserialization treats snapshots as untrusted input. A standalone `Pcg32`
must have the odd increment required by PCG, and each stream owned by a
`DeterministicRng` must have the increment derived from that object's seed and
the stream name. Stream state itself may be any `u64`: the LCG transition is a
permutation, so every state is reachable.

Stream derivation is length-domain-separated: the name length and each byte are
mixed with the master seed through SplitMix64, then distinct fixed domains
produce the PCG state and odd stream increment. This mapping and PCG32-XSH-RR's
output transform are replay contracts pinned by `tests/rng.rs`; range reduction
uses rejection sampling. Callers select streams by stable subsystem names so
adding draws in one system cannot shift another system's sequence.

### `time.rs`

`Tick(u64)` and `RoundCount(u32)` make the simulation's two time units distinct
and make seconds unrepresentable in core. Their private scalar representations
serialize transparently as integers. Each offers construction, inspection and
explicit saturating or checked advancement; no arithmetic operator obscures
the unit being added. Tick rate remains server configuration and round duration
remains ruleset data (spec §2.5).

### `ulid.rs`

`Ulid(u128)` identifies authored campaign objects (spec §4.3). Its high 48 bits
hold a caller-supplied millisecond timestamp and its low 80 bits caller-supplied
randomness; construction masks over-wide fields and never consults a clock or
entropy source. Display and serde use canonical uppercase Crockford base32 so
integer order, displayed lexical order and authored JSON order agree. Parsing
also accepts lowercase and Crockford's `I`/`L`/`O` aliases while reporting
length, character and 128-bit overflow failures separately.

### `intern.rs`

`Interner` uses an insertion-ordered `IndexSet<String>`, whose position is the
dense `u32` handle and whose serde form is the ordered sequence of strings.
Repeated interning returns the existing position without changing the table;
resolution, iteration and equality use the same first-intern order. Loading
rejects duplicate strings rather than collapsing them and silently renumbering
later handles.

`Interners` owns independent stat and tag tables and wraps their indices in
private-field `StatId` and `TagId` newtypes. Those handle types are deliberately
not serializable or displayable: they are meaningful only with the table that
issued them. Persistence resolves and stores the string, as fixed by
[ADR-0006 Decision 4](../adr/0006-crpg-core-primitives.md#decision-4--interned-ids-are-runtime-only-handles-the-persisted-form-is-always-the-string).

## Open

- Nothing blocking. T006e completed the planned core primitives.
- `blake3` landed as a runtime dependency for `state_hash` in T008a
  (workspace dependency, license gate via the Apache-2.0 alternative).

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark + event-queue hardening · Documented the event substrate module and its load-time sequence guards; corrected the stale blake3-authorisation note.
- 2026-09-06 (UTC) · opencode/muse-spark + ADR-0011 · Recorded the fields-not-enum payload clarification and affirmed the sanctioned sim instance.
