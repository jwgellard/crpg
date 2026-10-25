# crpg-net architecture

## Scope

`crpg-net` owns the authoritative-transport proof for headless combat. As of
T018a that means the versioned lane-0 combat wire: the [`protocol`] module
(explicit version + lane, per-lane `seq` identity, closed combat and
filtered-replica vocabularies, versioned hard maxima, frozen receipt codes,
documented host-policy defaults), the bounded `postcard` [`codec`] (canonical
encode, staged decode with exact error mapping, no `World`/RNG/event access),
and the local [`transport::Transport`] byte-pipe trait (E003-B: local to net,
no contracts trait). Verification status belongs to the task records
([T018a](../../tasks/T018a.md), then T018b/T018c).

What exists today is the T018a foundation, the T018b simulated transport,
and the T018c conformance gate: the test-only `NetDriver` double (supplied
visibility/ownership tables, epoch plus per-lane seq/cache plus freshness
admission, immediate `perform_action` legality mapped to `IllegalAction`
receipts, filtered delta pumps with redeliverable per-client frames, and an
independent permitted-view projection), the `Replica` oracle (buffered,
deduped, per-fact comparison), and the four suites — malicious-client
rejection without mutation, filtered convergence with negative controls,
the 5,000-tick loss/jitter exercise, and receipt liveness. The driver
sequences public sim calls only, observes a detached queue clone, and never
drains the authoritative queue. What is planned is the E017 Appendix B §B7
evolution queue (richer event history behind a version bump, lane-1
movement, QUIC, reconnect, prediction, host slice). Real network I/O, host
auth/perception, and movement wire are named follow-ons, not here.

*2026-10-04 (UTC), T023:* real QUIC lives in `crpg-net-quic`
([ADR-0024](../adr/0024-separate-crpg-net-quic-crate.md),
[crpg-net-quic.md](crpg-net-quic.md)); never add I/O, threads, clocks or
QUIC crates here.

## Public surface

Modules `protocol`, `codec`, `transport`, re-exported at the root only as
modules (no glob re-export). T018a lists every required type, field,
variant, function signature, and limit constant; changing the interface
requires an ADR.

- `protocol`: `PROTOCOL_VERSION`, `LANE_COMBAT`, `SEQ_FIRST`/`SEQ_LAST`, the
  nine `MAX_*` wire hard maxima, the `POLICY_*` operational defaults
  (host-supplied, tightening-only, never wire), the twelve frozen
  `*_TAG_*` discriminant bytes, `SessionEpoch`, `NetId` (`new` returns
  `None` on 0; `get` returns the raw id), `IntentFrame`/`IntentBody`
  (`DeclareAction { ability: Ulid, target: NetId }`, `EndTurn`), `DeltaFrame`/
  `DeltaOp` (8 ops: `EntityEnter`, `EntityLeave`, `Spawned`, `Despawned`,
  `Died`, `Health`, `Turn`, `Receipt`), `ReceiptStatus`
  (`Applied`/`Rejected`), `RejectionCode` (`#[repr(u8)]`, frozen `1..=20`,
  with `as_u8`/`from_u8`).
- `codec`: `CodecError` (`FrameTooLarge`, `UnsupportedVersion`,
  `WrongDirection`, `UnknownMessage`, `Malformed`, `LimitExceeded`) with
  first-failure-wins precedence, and `encode_intent`/`decode_intent`/
  `encode_delta`/`decode_delta`. No `World`/RNG/event access.
- `transport`: local `Transport` trait only (`Error`, `send`, `recv`).
  E003-B: no contracts trait introduced or implemented here.
- `sim` (T018b): `SimClock` (`now`, saturating `advance`; the only time
  source), `QueueCaps`/`RateCaps` (exact fields per T018b; `v1()` from T018a
  policy plus `v1_egress()` downstream depths), integer `TokenBucket` (`new`,
  `consume`, `available`), `PeerId` (`get`; monotonic, never reused),
  driver-owned `PeerState` (`new` binds epoch with full failure budgets,
  `is_exhausted` reports lane exhaustion, `try_consume_failure` spends one
  failure response against both budgets), `FaultSchedule`
  (`loss_every`/`dup_every`/`reorder_depth`/`seed`, plus `clean()`),
  `InMemoryTransport` (`new`, `clock_mut`, `add_peer`, `remove_peer`,
  `send_to`, `recv_from`, `tick`). T018a shapes unchanged.
- `protocol_v2` (T021): `PROTOCOL_VERSION = 2`, `DELTA_TAG_ACTION_RESOLVED`
  (8) / `DELTA_TAG_TURN_STARTED` (9) / `DELTA_TAG_ENCOUNTER_ENDED` (10),
  re-exported v1 `IntentFrame`/`IntentBody`/`NetId`/`SessionEpoch`/
  `ReceiptStatus`/`RejectionCode` (not v1's version const), `DeltaFrame`
  (`lane`, `server_tick`, `event_seq`, `ops`), `DeltaOp` (`Legacy` plus the
  three event variants with actor/target/ability/outcome/damage,
  actor/round, encounter/round fields). Derives `Debug, Clone, PartialEq,
  Eq`; no serde. `Legacy` adds no wire tag.
- `codec_v2` (T021): `encode_intent`/`decode_intent`/
  `encode_delta`/`decode_delta`; `CodecError` re-exported from `codec`
  (same six variants). v2 intent is v1 with first byte 2; v2 delta header
  is version/lane/`server_tick`/`event_seq`/op-count; tags 0..7 identical
  to v1; new payloads in exact spec order. Delta `event_seq` accepts full
  `u64`; intent/receipt `seq` keeps `SEQ_FIRST..=SEQ_LAST`.
- `projection_v2` (T021): `EventCandidate` (all-`Option` per-field
  candidates with disclose flags), `ProjectionError`
  (`TooManyEvents`/`InvalidOutcome`/`StringLimit`, `Error`+`Display` as
  `<VariantName> at projection/events`), `project_events` (at most 256
  inputs, ordered subsequence, no partial output on error).
- `protocol_transfer` (T024): transfer subprotocol constants, `TransferId`,
  `BeginTransfer`, `TransferChunk`, `CancelReason`, `ResyncReason`,
  `TransferDown`/`TransferUp`, `SnapshotBody` and related fact types.
- `codec_transfer` (T024): `encode_down`/`decode_down`/`encode_up`/
  `decode_up`/`encode_body`/`decode_body`/`plan_transfer`.
- `reassembly` (T024): `TransferAssembly`, `TransferLimits`,
  `ReassemblyPool` and cutover gate types.

## Module flow

- `protocol` holds the frozen v1 contract: `PROTOCOL_VERSION`/`LANE_COMBAT`/
  `SEQ_FIRST`/`SEQ_LAST`, the nine wire hard maxima, the `POLICY_*`
  operational defaults (host-supplied, tightening-only, never wire), the
  twelve frozen discriminant tags, `SessionEpoch`/`NetId`, `IntentFrame`/
  `IntentBody` (2 variants), `DeltaFrame`/`DeltaOp` (8 ops), and
  `ReceiptStatus`/`RejectionCode` with frozen `1..=20` values.
- `codec` holds the canonical `postcard` encode (`encode_intent`,
  `encode_delta`) and the staged decode (`decode_intent`, `decode_delta`)
  with the `CodecError` precedence: length cap → version → lane/direction →
  discriminant → lengths/collection bounds → complete decode → trailing-byte
  check → semantic value checks. Tags are matched as explicit `u8` bytes, not
  derived enum discriminants, so unknown values map exactly to
  `UnknownMessage` instead of a generic serde failure.
- `transport` holds the local `Transport` trait only: `send` queues one
  framed datagram, `recv` takes the next or returns `None`. Bounded
  in-memory semantics are T018b's implementation, not this trait.
- `sim` holds the T018b delivery fabric: `SimClock` (the only time source),
  `QueueCaps`/`RateCaps` (`v1()` from T018a policy, plus `v1_egress()` depths
  for the downstream direction), integer-milli-token `TokenBucket`s, `PeerId`
  (monotonic, never reused) with the bounded peer table, driver-owned
  `PeerState` (epoch, lane, `next_new_seq`, 256-entry canonical-bytes cache,
  failure-response budgets spent via `try_consume_failure` with
  `is_exhausted` lane-exhaustion reporting), `FaultSchedule` (periodic
  loss/dup counters plus seeded block-rotation reorder bounded by
  `reorder_depth`), and `InMemoryTransport` (`send_to` stages under
  rate → size → queue checks; `tick` pumps in ascending peer order;
  `recv_from` releases accounting). Staged datagrams count against the caps
  until received or the peer is removed, so slow peers end in `QueueFull`,
  never silent loss. Egress enforces at the same staging point with the
  `v1_egress()` depths; failure budgets enforce on the driver wire path with
  drops that retain outcomes for retry.

A frame on the wire is a version byte followed by `postcard`-encoded
primitives (single-byte `u8`/`bool`, varint integers, length-prefixed
strings, raw fixed arrays) in the frozen field order documented in
[`codec`]. v1 freezes tags, field order, and numeric codes; anything additive
needs a new protocol version plus new fixtures, never ignore-and-continue.

## Invariants

- v1 freezes tags, field order, and numeric codes. Additive variants, field
  additions, or meaning changes require a new protocol version plus new
  fixtures. v1 never ignores-and-continues on unknown input.
- Decode order is contract: length cap → version → lane/direction →
  discriminant → lengths/collection bounds → complete decode →
  trailing-byte check → semantic value checks (`NetId` nonzero, `seq` in
  `SEQ_FIRST..=SEQ_LAST`, ULID text parses). First failure wins.
- Bounds fire on encode as well as decode (`WrongDirection` lane,
  `Malformed` `seq`, `LimitExceeded` op count; post-check encoded length
  against the frame cap).
- `NetId` zero is unrepresentable upstream (`new` returns `None`) and
  `Malformed` downstream. `seq` 0 and `u64::MAX` are `Malformed` on both
  sides. Receipt `lane`/`seq` are checked like intent `lane`/`seq`.
- The op vector grows by push, never `with_capacity` from the untrusted
  count; string bodies are bounded by the already-checked frame cap plus the
  explicit `MAX_WIRE_STRING_BYTES` gate before ULID parsing.
- Policy constants are documentation for T018b/c drivers, not wire and not a
  host API. Rate time is host monotonic/injected, never `World` tick.
- The fabric never decodes: `send_to` checks rate → size → queue in that
  order (unknown peer is `Unauthenticated`; every attempt deducts both
  budgets; size gate is the `MAX_DELTA_FRAME_BYTES` fabric MTU; caps fail
  closed to `QueueFull`). Staged datagrams count until received or the peer
  is removed; `recv_from` on unknown/empty is `None`.
- `tick` pumps in ascending `PeerId` order with per-peer loss/dup counters
  from 1 (`None`/`0` disables) and block-rotation reorder inside
  `reorder_depth + 1` blocks from the reseeded `"net-fault"` stream; batches
  never cross a tick, duplicates carry no extra accounting.
- `PeerState` is driver-owned: epoch close on `SeqConflict`, `SeqExhausted`
  once `next_new_seq` passes `SEQ_LAST` (cached retries still return retained
  outcomes; anything else non-cached reports exhaustion without consuming
  `seq`), no advance on `SeqGap`, `StaleSeq` past the 256-entry/2-MiB cache,
  200-tick freshness with saturating edges, failure budgets spent per
  rejected response (fresh and cached alike) via `try_consume_failure` on an
  injected fail clock with drops that retain the outcome for retry. The
  fabric ignores epochs past the `add_peer` signature (binding lives in
  drivers); T018b's double uses canned legality, T018c's uses real sim calls.
- Egress enforces at fabric staging with the downstream depths: drivers stage
  host-to-peer bytes through the same rate → size → queue checks using
  `v1_egress()` caps (128 frames / 2 MiB per peer, 16 MiB host), so slow
  peers end in `QueueFull`, never silent loss. `bind_peer` enforces the
  8-peer proof population with `ServerBusy` (rotation of a bound peer always
  succeeds); the fabric enforces the same bound on its side.
- Snapshot reassembly is provisional in v1 (no transfer wire): the enforced
  contribution is the injected-time deadline model plus the 1-in-flight /
  32-chunk / 5-second bounds, proved by a test-local tracker.
- T018c doubles are test-only (`tests/support/`, never shipped) with
  dev-only `crpg-data`/`crpg-sim`/`serde_json` edges for authored fixtures
  and public sim calls. Admission order is decode → epoch → seq/cache →
  exhaustion → freshness → mapping/visibility/ownership → immediate
  `perform_action` legality; early failures (including `SeqExhausted`)
  consume no seq, terminal outcomes consume exactly one and cache canonical
  bytes. `perform_action` precedence stays authoritative and maps to
  `IllegalAction` with the typed error retained. The wire path (`admit`)
  spends failure budgets on rejections and drops bytes (empty `Vec`) when
  empty while the status path still reports the retained outcome.
- Delivery discloses or omits per op: hidden spawns consume no ids and no
  sequences, revoked ids retire to graves and are never reused, death
  retains the terminal entity (never implies despawn), despawn reads as
  `Despawned` only when disclosable. Redelivery re-encodes byte-identically
  from the sent log. The oracle compares per-fact with events as multisets;
  negative controls must fail loudly. Every rejection needs a positive
  control and a byte-identical complete-state assertion.
- v2 is explicit and additive: v1 bytes/fixtures/precedence untouched (the
  v2 codec duplicates the staged skeleton; no shared version-parameterized
  path). v1 refuses byte 2, v2 refuses byte 1 (`UnsupportedVersion`); a new
  tag in a valid v1 envelope is `UnknownMessage` — separate tests, no
  autodetect/negotiation/downgrade. `Legacy` flattens to tags 0..7 with
  unchanged payloads. Outcome is T020's five symbolic forms only
  (`custom:<0..255>`, no leading zeros); no roll/DC/margin/grants on wire.
- v2 strings are bounded before allocation: the length varint is checked
  against `MAX_WIRE_STRING_BYTES` before body availability, UTF-8, or any
  copy (a hostile length with an absent body is `LimitExceeded`, never
  truncation). Outcome-length errors are `LimitExceeded`; bad UTF-8,
  truncation, short bodies, and ULID/outcome semantics are `Malformed`.
  Reject overflowing tenth-byte length payloads before shifting; exercise
  all v2 string paths with overflowing, truncated, and valid oversized
  prefixes, including otherwise-valid bodies that would expose wrapping.
  The op vector grows by push, never `with_capacity` from the untrusted
  count; encode checks lane/count/op-semantics then frame length with no
  partial bytes.
- Projection consumes host-authorized candidates only: any missing required
  field/grant omits the whole event without inspecting hidden payload
  further (a hidden bad outcome is suppression, not an error). Output is an
  ordered subsequence (no sort/dedup); same-tick order is journal order.
  The codec checks shape, not entitlement; per-client frame `event_seq`
  starts at 1 after suppression/chunking and never reuses history seq; a
  suppressed-only page emits no frame. State ops stay host-assembled, so
  event suppression never suppresses Health/Turn updates.
- v2 retry conformance must enter the same fixture admission path on retry,
  recover a genuinely dropped simulated-transport frame from cached bytes,
  and consume duplicate delivery once in sequence order. Check the complete
  authority hash, full journal, execution count, and independent event oracle;
  cloning bytes and calling the codec alone does not establish recovery.
- Transfer decode precedence (T024 S§6): whole-frame cap first (`decode_down`
  over 65,536 / `decode_up` over 4,096 → `FrameTooLarge`), then first byte
  (not `0x81` → `UnsupportedVersion`; empty → `Malformed`), then lane (not
  2 → `WrongDirection`), then tag (the other direction's tag →
  `WrongDirection`; any other unknown → `UnknownMessage`), then fields left to
  right (value above its cap → `LimitExceeded`; zero or inconsistent →
  `Malformed`; unassigned `reason` byte → `UnknownMessage`), then trailing
  bytes → `Malformed`. Segment length is checked against 4,096 before body
  availability.
- Cutover barrier (T024 S§10.7): a delta is releasable when the mode is
  `Live` and either no transfer is in flight or its `event_seq ≤` the
  in-flight cursor; the FIFO queue is the only way updates leave the pool, so
  deltas at or below the cursor queued before publication pop before the
  snapshot, deltas above the cursor are never queued before it, and a later
  delta at or below a published cursor is `Superseded`. No delta is applied
  twice, to the wrong base, or skipped, and a gap is never released past.
- `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]`. No `HashMap`/`HashSet`
  (use `BTreeMap`/`IndexMap`); no `f32`/`f64` in policy paths (no spatial
  wire in v1 lane 0); no `SystemTime`/threads/I-O/unseeded RNG
  (`DeterministicRng` only if needed; the codec path needs none).

## Conformance (T018c)

The suites live in `tests/conformance.rs` with the driver and fixtures in
`tests/support/` (test-only, never shipped). Admission runs
decode → epoch → seq/cache → exhaustion → freshness →
mapping/visibility/ownership → immediate `perform_action` legality: early
failures (including `SeqExhausted`, which outranks gap/stale so clients
rotate epochs instead of retrying) consume no seq, terminal outcomes consume
exactly one and cache canonical bytes. Rejected wire replies spend the
peer's failure-response budgets and drop (empty bytes, retained outcome)
when empty; peer binding enforces the 8-peer proof population. Delivery pumps
disclose-or-omit per op (hidden spawns consume no ids and no sequences;
revoked ids retire to graves and are never reused; death retains the
terminal entity; despawn reads as `Despawned` only when disclosable);
redelivery re-encodes from the sent log byte-identically. The oracle
compares the reconstructed replica per-fact against the independently
projected view (events as multisets with separately gapless numberings),
and the negative controls prove it fails loudly. Every rejection carries a
positive control and a byte-identical complete-state assertion. A passing
double is not host integration passing: E012/E018/E022 still own the real
host.

## v2 event wire (T021)

`protocol_v2` adds the explicit history-projection vocabulary beside frozen
v1: `PROTOCOL_VERSION = 2`, tags 8/9/10
(`ActionResolved`/`TurnStarted`/`EncounterEnded`), and `DeltaOp::Legacy` — a
Rust-only wrapper that contributes no extra tag, so old-vocabulary v2 bytes
differ from v1 only in the version byte. v1 exports, bytes, and fixtures
are unchanged. `codec_v2` duplicates the staged skeleton with version byte
2 (byte 1 is `UnsupportedVersion`, never reinterpreted) and reads string
lengths as bounded integers checked against `MAX_WIRE_STRING_BYTES` before
any body inspection or copy — never v1's owned-string-before-check shape.
The length reader rejects overflow in the tenth varint byte before shifting;
unrepresentable lengths are `Malformed`, while representable lengths above
the string cap remain `LimitExceeded` even without a body.
`projection_v2` consumes host-supplied per-field `EventCandidate`s (each
`Some` is an independently authorized disclosure for this
viewer/generation) and appends exactly one op per fully disclosed event in
input order, omitting anything partially hidden without inspecting it
further; the host assigns gapless per-client frame `event_seq` after
suppression/chunking and assembles permitted state ops itself. The six-case
`tests/events_v2.rs` suite proves explicit version selection (both
directions, plus separate unknown-tag refusal), literal new-tag byte
oracles, bounded-before-allocation limits, field-by-field suppression with
two-peer views, ordered projection of real T020 `HistoryWorld` journals,
and byte-identical redelivery from a driver cache with exactly one sim
mutation. The retry fixture sends initial history and action results through
the simulated fabric, drops the action result, then retries the same command
through cache-first admission. Duplicate deliveries are consumed once by a
sequence-keyed receiver and checked against a hand-authored ordered oracle;
the full authority hash and journal remain unchanged after retry. No new
dependency; the sim edge stays dev-only for tests.

## Authorities and consumers

Netids are client-scoped replica ids: zero is never valid, ids are never
reused in one epoch, and drivers keep the whole `EntityId` including its
generation internally, never serialize raw arena ids, and drop revoked or
hidden ids from the actionable mapping before new requests execute.
Gameplay legality stays authoritative in sim (`perform_action` precedence);
net never manufactures `legal_actions`, never drains the authoritative queue,
never serializes the evolving sim enum, and never touches `World`, RNG, or
events from the codec path. Rejected inputs leave complete authoritative
state byte-identical; that proof belongs to the T018c drivers, which stay
net-local and never import testkit at runtime.

Decisions: [ADR-0004](../adr/0004-quic-movement-spike.md) (spike context;
per-intent `seq` retained, highest-seq-wins movement explicitly not lifted to
lane 0), [ADR-0011](../adr/0011-event-payload-fields.md) (core-closed
payload-field discipline the wire projections respect), [E017 Appendix
B](../../tasks/E017-t018-interface-debt.md) (durable wire versus provisional
scope, per-lane seq, two-tier limits, filtered projections, evolution
queue). Working contract:
[`crates/crpg-net/AGENTS.md`](../../crates/crpg-net/AGENTS.md).

[`protocol`]: ../../crates/crpg-net/src/protocol.rs
[`codec`]: ../../crates/crpg-net/src/codec.rs
[`sim`]: ../../crates/crpg-net/src/sim.rs
[`transport::Transport`]: ../../crates/crpg-net/src/transport.rs
[`protocol_v2`]: ../../crates/crpg-net/src/protocol_v2.rs
[`codec_v2`]: ../../crates/crpg-net/src/codec_v2.rs
[`projection_v2`]: ../../crates/crpg-net/src/projection_v2.rs

## Allowed dependencies

`crpg-core` (path: `Ulid` vocabulary), `serde` (workspace: codec trait
bounds; explicitly named usable by T018a), `postcard = "1"` (crates.io:
bounded wire codec; approval, license, and ban/source justification recorded
in [T018a](../../tasks/T018a.md); if `cargo deny` objects, stop — do not
widen `deny.toml`). T021 adds no dependency: production net consumes
host-supplied candidates and the v2 codec reuses the same two crates.
Dev-only for the T018c/T021 suites: `crpg-data`, `crpg-sim`
(path: authored fixtures and public sim calls, anticipated by T018a) and
`serde_json` (workspace: fixture authoring only; zero new lock packages).
Anything else needs approval.

## Snapshot transfer (T024)

`protocol_transfer`, `codec_transfer`, and `reassembly` carry the separate
versioned snapshot-transfer subprotocol (first byte `0x81`, lane 2). The byte
layer `TransferAssembly` proves the full 1 MiB / 32-chunk envelope; the
`ReassemblyPool` adds deadline, budgets, body validation, atomic publication
and the delivery-cursor cutover gate. Host requirements are in T024 S§11.

- 2026-10-25 (UTC) · opencode-go/beam-free + T024 · Added Snapshot transfer section.

## Agent log

- 2026-09-29 (UTC) · opencode/gpt-6-astra + T021 review fixes · Documented checked string-length overflow and transport-backed cache/retry conformance. These close malformed-length acceptance and replace codec-only redelivery evidence with actual dropped/duplicate delivery checks.

- 2026-09-28 (UTC) · opencode/muse-spark + T018a crate opening · Opened the doc with the lane-0 v1 protocol/codec/local-Transport shape, the frozen-wire and no-gameplay-execution boundaries, and the T018b/c plus evolution-queue plan, so later tasks extend rather than restart it.
- 2026-09-28 (UTC) · opencode/muse-spark + T018b simulated transport · Extended the scope and module flow with the injected-time fabric (rate → size → queue staging, seeded bounded faults, driver-owned seq/cache state) and the bytes-only fabric versus admission-drivers split, leaving T018c conformance and the evolution queue planned.
- 2026-09-28 (UTC) · opencode/muse-spark + T018c conformance · Extended the scope with the sim-backed driver, replica oracle, and four suites (rejection-without-mutation, filtered convergence with negative controls, 5,000-tick exercise, receipt liveness), closing the T018 proof with the evolution queue still planned.
- 2026-09-28 (UTC) · opencode/muse-spark + T018 review fixes · Recorded enforced failure-response budgets with wire drops, egress staging under v1_egress depths, reachable SeqExhausted precedence, and the bound driver peer table.
- 2026-09-29 (UTC) · opencode/muse-spark + T021 v2 event wire · Recorded the as-built explicit v2 modules (tagged history events beside frozen v1, bounded-before-allocation codec, host-fed ordered projection) and the six-case acceptance suite, so T022 consumes version-selected codecs plus ordered conformance evidence.
- 2026-10-04 (UTC) · claude-code + T023 · Appended a dated Scope pointer to `crpg-net-quic` for real QUIC, so this crate's no-I/O boundary names where the transport went; doc-only, no source or manifest change.

- 2026-10-25 (UTC) · opencode-go/beam-free + A0-8 · Absorbed the moved Public surface, Invariants, and Allowed dependencies sections from AGENTS.md; no content lost.
