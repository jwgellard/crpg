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
- `interest` (T027b): `MAX_PAGE_OPS`, `EntityKey` (`area` then full
  `EntityId`), `InterestSet`, `FieldMask` (`presence`/`health`, default
  denies), `EntityFact`, `TurnState`, `ViewerContext`, `ViewCursor`,
  `StateProjection` (`ops`, `enters`, `cursor`), `SnapshotProjection`,
  `InterestError` (`WrongEpoch`/`TooManyFacts`/`DuplicateFact`/
  `IdExhausted`), and `InterestView` (`new`, `epoch`, `cursor`, `len`,
  `is_empty`, `net_id`, `entity`, `project_delta`, `project_snapshot`).
- `replica` (T046): `MAX_REPLICA_ENTITIES` (`2 * MAX_VISIBLE_ENTITIES`),
  `ReplicaEntity` (`net_id`, `health`), closed `ReplicaError`
  (`WrongLane`/`TooManyOps`/`EventSeq`/`UnknownEntity`/`DuplicateEntity`/
  `ReusedEntityId`/`TooManyEntities`/`WrongEpoch`/`SnapshotOrder`/
  `SnapshotCursor`), and `ReplicaWorld` (`new`, `epoch`, `entities`,
  `entity`, `len`, `is_empty`, `turn`, `next_event_seq`, `server_tick`,
  `apply_delta`, `apply_snapshot`). Not `Serialize`, no restore path.

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

## Resume and reconnect (T025a)

`protocol_resume`, `codec_resume`, and (T025a2) `resume` carry the separate
versioned resume subprotocol (first byte `0x91`, lane 3) beside frozen v1/v2
and the `0x81` transfer wire. The channel or transport chooses the codec; no
resume decoder falls back to another. `protocol_resume` holds the constants,
`ConnectionFence`, the opaque `ResumeProof` (16..=128 bytes, redacted `Debug`,
`ct_eq`, no `PartialEq`/`Eq`/`Ord`/`Hash`/`Display`), the claim/progress
shapes and the messages. `codec_resume` holds the four functions
(`encode_request`/`decode_request`, `encode_down`/`decode_down`) with the
S§7 precedence: whole-frame cap → version `0x91` → lane 3 → tag → fields left
to right → trailing-byte reject. `resume` holds `SessionFence`,
`ResumeDecision`, `check_result` and the additive `rebind_receiver`.

The fence window (S§15): state is `epoch`, current fence `c`, confirmed fence
`k`, and `max_fence`, with `1 ≤ k ≤ c ≤ max_fence`.

| Call | Result | Change |
|---|---|---|
| `check_frame(tag)` with `tag == c` | `Ok` | `k = c` |
| `check_frame(tag)` with `tag < c` | `Err(Fenced)` | none |
| `check_frame(tag)` with `tag > c` | `Err(UnknownFence)` | none |
| `check_request`: `epoch` wrong / `fence > c` / `fence < k` / `c == max_fence` | `WrongEpoch` / `UnknownFence` / `StaleFence` / `FenceExhausted` | none |
| `check_request` otherwise | `Ok(c + 1)` | none |
| `resolve` on a fence error | the same error | none |
| `resolve` on `Reject(reason)` | `Ok(Rejected)` | none |
| `resolve` on `Accept` with invalid progress | `Err(InvalidProgress)` | none |
| `resolve` on `Accept` | `Ok(Accepted { granted: c })` | `c = c + 1` |
| `check_result(epoch, presented, result)` | `WrongEpoch` / `StaleFence` (granted ≤ presented) / `Ok` | — |

Grants strictly increase and are never re-issued, so at most one connection
of a session passes `check_frame` at any time. A lost result is recoverable
through the `[k, c]` window; nothing lowers `c` or `k`.

Receiver rebind (`rebind_receiver(id, now_ms)`, S§16), in order: run the
pool-wide expiry sweep (clamping `now_ms`); unknown id → `UnknownReceiver`;
discard and retire the in-flight transfer; drop every held delta (freeing its
accounting); set `AwaitSnapshot` and `resync = None`; keep `epoch`, `wire`,
`published`, `next_event_seq`, `retired_through` and queued updates. A second
rebind is an idempotent no-op apart from the mode; late chunks of the
discarded transfer are `StaleTransfer`.

Carriage over QUIC (S§20) lives in T024q: channel `0x91` under `/2`, an
opt-in resume-first connection on `bind_resumable`, the result in place of
the welcome, one total order per direction after `Accepted`, and no new
`ServerEvent`/`SendError`/`ServerError` variant.

Host requirements (H1–H8, S§21; the ADR-0028 pattern, see
[ADR-0032](../adr/0032-resume-subprotocol-and-connection-fencing.md)): one
`SessionFence` per session, never re-created; tag connections with their
fence and `check_frame` every inbound frame; build the `ResumeDecision` only
after authenticating the proof; answer any net refusal with `Rejected
{ Denied }`; on `Accepted`, fence the old connection, send the result, then
stage the resume snapshot before later deltas; never reset the epoch, ledger,
seqs, ordinals, transfer ids or budgets; issue a `Ticket` and never persist
proofs; a `Busy`/`Denied` never moves the fence.

Mechanism here, authenticated policy in `crpg-server`.

- 2026-10-30 (UTC) · opencode-go/deepseek-v4-pro + T025a1 · Added Resume and reconnect section.

- 2026-11-01 (UTC) · opencode-go/deepseek-v4.1-flash + T025a2 · Recorded the as-built mechanisms behind the same section: `resume`'s fence window and explicit decision, `rebind_receiver`'s keep-the-floors steps, and cases 11–21 as the mechanism evidence (mechanism here, authenticated policy in `crpg-server`).

## Interest projection (T027b)

`interest` is the per-viewer state filter: one `InterestView` per session
epoch, one viewer in exactly one area. The host supplies every value in a
`ViewerContext` (epoch, authoritative tick, viewer area, per-row `FieldMask`
grants and health values, despawn keys, the turn fact and its grant, and the
two one-page resend flags). Net compares supplied areas only: a row is visible
when `key.area == interest.area && fields.presence`; a foreign row is
suppressed, never an error. No `World`, RNG, event, ownership, LOS or legality
input is read, and no wire form is added — output is existing v1 `DeltaOp`
values (wrapped in `Legacy` for v2 sessions) or one T024 `SnapshotBody`.

Identity is `(area, full EntityId)`: two areas may mint equal `EntityId`s, so
keying on the id alone would alias a viewer's old `NetId` onto a different
entity after an area change.

**Op ownership (D5).** Each op has exactly one producer:

| Op | Owner |
|---|---|
| `EntityEnter`, `Health`, `Turn`, `EntityLeave`, `Despawned` | `InterestView` |
| `Spawned`, `Died`, `ActionResolved`, `TurnStarted`, `EncounterEnded` | `project_events` |
| `Receipt` | host |

`Spawned`/`Died` keep their pinned journal position in history. `Despawned`
belongs to the view so a removal is exactly one op (leave or notice, never
both); a removed key listed in `context.despawned` emits `Despawned`, every
other removed key emits `EntityLeave`.

**Page layout (item 5).** `project_delta` returns, in order: one
`EntityEnter` per entering key (`StateProjection::enters` is that count), then
`Health` per visible key, then at most one `Turn`, then one removal op per
removed key. Each section is ascending `NetId`; the whole page is at most
`MAX_PAGE_OPS = 3 * MAX_VISIBLE_ENTITIES + 1`. History slots between
`ops[..enters]` and `ops[enters..]`. The view never returns `Spawned`, `Died`
or `Receipt`.

**Retirement (items 4, 9).** Entering keys take `next_id` upward in ascending
`EntityKey` order, once each; ids run `1..=u64::MAX - 1`. A removed key's
mapping is gone when the call returns: `net_id` and `entity` report `None`, a
re-entering key gets a new id, and no id is ever reissued in an epoch. Nothing
is stored to represent a grave, so there is no table to bound: retired means
below `next_id` and not live.

**Snapshot, cutover and cursor (items 12–14).** `project_snapshot` is the only
replacement primitive: live visible keys keep their ids, removed keys retire
with no op, and the body carries one entity per visible key plus the rendered
turn (`active` inside the body, or `None` when the actor is hidden). It records
exactly what the body says, so the next delta emits only differences. Every
successful snapshot and every non-empty delta page advances
`ViewCursor { tick, revision }` by one saturating revision; an empty page
leaves the cursor unchanged. `ViewCursor` is view state, never wire, and never
`event_seq`.

**Errors (item 1).** Checked before any mutation, in order: `WrongEpoch`,
`TooManyFacts` (`facts` or `despawned` over `MAX_VISIBLE_ENTITIES`),
`DuplicateFact` (any two rows with one key, visible or not), `IdExhausted`
(fewer ids than entering keys). `Err` changes nothing; `IdExhausted` needs a
new epoch and a fresh view.

Mechanism here, authenticated grants and host composition in `crpg-server`
(T027c).

- 2026-11-02 (UTC) · opencode-go/deepseek-v4.1-flash + T027b · Added the `interest` public-surface entry and its section: D5 owner table, item 5 page layout, derived retirement, snapshot/cutover and cursor rules, and the error order.

## Client replica (T046)

`replica` is the engine-neutral client copy of one filtered v2 session. It
consumes whole decoded values only: `apply_delta(&protocol_v2::DeltaFrame)` and
`apply_snapshot(&reassembly::Publication)`. Storage is one `Vec<ReplicaEntity>`
strictly ascending by `NetId`, one optional `TurnFact`, and three `u64`s
(`next_event_seq` from 1, `server_tick` from 0, `high_water` from 0). `None`
health is unknown, never zero; `dead` lives only in `HealthFact`. There is no
events, receipts, history or retired-id table, and no `Serialize`, `Default`
or restore constructor.

**Sequence law.** A frame is accepted only when `event_seq` equals
`next_event_seq`; on `Ok`, `next_event_seq = event_seq + 1` and
`server_tick = frame.server_tick`, including empty and notice-only frames. Gaps,
duplicates, reorders, rewinds, `0`, a restart at 1 after progress, and values
above `SEQ_LAST` are `EventSeq`; a backwards tick is not an error. There is no
in-band reset: a host narrowing reset arrives as `RebindRequired` on a fenced
connection, and the caller builds a new replica for the new session. After a
publication at `SEQ_LAST`, `next_event_seq` is `u64::MAX` and nothing more is
accepted.

**Delta precedence.** Lane (`WrongLane`) → op count over `MAX_DELTA_OPS`
(`TooManyOps`) → sequence (`EventSeq`) → ops in frame order, each checked
against the working state left by the ops before it. A failure anywhere leaves
the replica equal to its pre-call clone (validate and apply on a working copy,
then commit).

| Op | Checks | Effect |
|---|---|---|
| `EntityEnter { e }` | live → `DuplicateEntity`; `e ≤ high_water` → `ReusedEntityId`; `len == MAX_REPLICA_ENTITIES` → `TooManyEntities` | push `{ e, health: None }`; `high_water = e` |
| `EntityLeave { e }` | not live → `UnknownEntity` | remove the row and its health |
| `Despawned { e }` | not live → `UnknownEntity` | exactly as `EntityLeave` |
| `Spawned { e }`, `Died { e }` | not live → `UnknownEntity` | none |
| `Health { e, .. }` | not live → `UnknownEntity` | replace all three health fields |
| `Turn { active, round }` | `Some(a)` not live → `UnknownEntity` | replace the whole turn fact |
| `Receipt { epoch, .. }` | `epoch != replica epoch` → `WrongEpoch` | none |
| `ActionResolved { actor, target, .. }` | actor not live, then target → `UnknownEntity` | none |
| `TurnStarted { actor, .. }` | actor not live → `UnknownEntity` | none |
| `EncounterEnded { .. }` | — | none |

**High-water rule.** `high_water` is the largest `NetId` ever stored. A delta
enter at or below it, and a snapshot row at or below it that is not live just
before the snapshot, is `ReusedEntityId`. Delta enters are legal only in
rising order, so an enter is a push. This enforces N3 without a grave table;
it depends on T027b's rising, once-only id minting (K1).

**Snapshot precedence and replacement.** Cursor below the last applied
`event_seq` or above `SEQ_LAST` → `SnapshotCursor`; a body over
`MAX_VISIBLE_ENTITIES` → `TooManyEntities`; a row not strictly above the
previous row → `SnapshotOrder`; an id at or below `high_water` that is not
live before the call → `ReusedEntityId`; a turn `active` outside the body →
`UnknownEntity`. On `Ok` the rows become exactly the body's rows with health
verbatim, the turn becomes exactly the body's turn, `next_event_seq` is
`cursor + 1`, `server_tick` is `base_tick`, and `high_water` keeps the larger
of itself and the body's last id (an empty body leaves it unchanged). A
cursor equal to or ahead of the last applied sequence is legal.

**Live bound (D8).** Live rows may reach `MAX_REPLICA_ENTITIES =
2 * MAX_VISIBLE_ENTITIES` between frames or within one, because a T027b page
puts all enters before all removals and may split at any op boundary. This is
a client bound, not wire.

**Errors.** Every `ReplicaError` variant is fatal (ADR-0033 code 53) with a
small payload for `last_error`, and exact `Display` text
(`<Variant> at replica: ...`). The replica stays usable after `Err`; fencing
is the bridge's policy. The conformance `Replica` stays an independent test
oracle and is not replaced by this type.

- 2026-11-02 (UTC) · opencode-go/deepseek-v4.1-flash + T046 · Added the `replica` public-surface entry and section: the apply-only surface, op table, sequence law and no-reset rule, high-water reuse check, snapshot precedence, and the 2,048-row mid-page bound.

## Protocol v3 (T140, ADR-0029 N1–N4)

`protocol_v3` / `codec_v3` add wire version 3 beside frozen v1, v2, `0x81`
and `0x91`. A wire version is a schema version: byte 3 is a new module pair,
not a variant of v2. T140a fixes the version, the intents, native tags
0..=10, the rejection codes, and the scalar ops 14..=20; T140b–e add the
fact ops (tags 11..=13), the filters, and the `0x82` snapshot.

**Version gate.** `encode_*` writes byte 3 first. `decode_*` checks the
frame cap (4,096 intent, 65,536 delta bytes), then the version: empty input
is `Malformed`, any first byte other than 3 (1, 2, `0x81`, `0x91`) is
`UnsupportedVersion`. The v1 and v2 codecs refuse byte 3 the same way. There
is no negotiation, autodetection or downgrade; the host picks a version per
session.

**Intents.** The v1 header (`epoch`, `lane` 0, `seq`, `observed_tick`,
nonzero `actor`) then a body tag: 0 `DeclareAction` and 1 `EndTurn` keep
their v1 payloads; 2 `Stop`, 3 `Pause`, 4 `Unpause` carry nothing; 5
`RequestMode` carries one mode byte (0 `TurnBased`, 1 `RealTime`). For
`Pause`, `Unpause` and `RequestMode` the `actor` names the requesting peer's
bound character, and the host (T141) decides what that authorizes.

**Native tags 0..=10.** Re-declared in `protocol_v3::DeltaOp`, not wrapped in
v2's `Legacy`: a receipt's code must reach the new codes, and v1's
`RejectionCode` is frozen at 1..=20. Payload bytes are identical to v2, so
v3 bytes differ from v2 only in byte 0 (the test `a4` proves it for every
v1 code), and `From` conversions from the v1 and v2 ops let existing
projections feed v3 pages.

**Codes 21..=30.** `ModeUnsupported`, `WrongMode`, `StatEvaluation`,
`EncounterTerminal`, `RulesetMismatch`, `PlacementInUse`, `NotCreature`,
`AlreadyParticipant`, `NoEntrants`, `NoPauseRight`. `RejectionCode::from_u8`
is `Some` for exactly 1..=30. A paused host still runs commands, so there is
no `Paused` code.

**Scalar ops.** 14 `RoundStarted`, 15 `ModeChanged`, 16 `OrderEnded`, 17
`Joined`, 18 `Left` (notices that mint or retire no id), 19 `Clock`, 20
`HostFacts`. Round and the active combatant stay in the v1 `Turn` op.
Tags 11..=13 are the T140b fact ops (below).

**`Clock` is the only pause carrier.** `Clock { paused }` is
`Option<Option<PauseReason>>`: `None` is running (`00`), `Some(None)` is
paused with the reason withheld (`01 00`), `Some(Some(r))` is paused for `r`
(`01 01 <tag> [trigger]`; the trigger byte follows only the `Auto` tag).
It carries no tick: the server tick is the header of every delta frame, and
a heartbeat is an ordinary frame holding a `Clock` that consumes an
`event_seq`. `POLICY_CLOCK_HEARTBEAT_TICKS = 100` is policy, never wire: half
the 200-tick observed-tick window, so one lost heartbeat still leaves a
fresh tick.

**`HostFacts`.** The viewer's two pause rights (never withheld), the
auto-pause toggles (`AutoPauseToggles`, five bits, `0x1F` mask) and the turn
timer as remaining milliseconds. A withheld toggle set or timer is `None`
(a `00` flag, no further bytes); a disclosed timer is
`Option<Option<u32>>`, where `Some(None)` is "disclosed, no timer running"
and never shares bytes with withheld.

**Errors.** No new variant: `CodecError` covers every refusal. Unknown op,
body, status, code, mode, reason and trigger bytes are `UnknownMessage`;
ids of 0, flag bytes other than 0/1, toggle bits over `0x1F`, truncation and
trailing bytes are `Malformed`; counts and strings over their bounds are
`LimitExceeded`. Encode checks lane, count, then each op in order, then the
frame cap, and never returns partial bytes. `tests/v3_wire.rs` carries the
literal byte oracles (A1–A11).

### T140b: fact ops, bounds and `frame_spans`

Three ops carry the viewer's complete image of one subject, so a `None` is
"not disclosed", never zero, and a disclosed nullable value is a nested
`Option`. An op whose fields are all withheld tells the client to forget its
subject. Every flag is a raw `bool`; a byte other than 0/1 is `Malformed`.

| Tag | Op | Fields after the tag |
|---|---|---|
| 11 | `Encounter(EncounterFacts)` | `mode_disclosed` [`mode`]; `pending_disclosed` [`pending_some` [`pending`]]; `clock_disclosed` [`clock_some` [`round_ticks` u32, `elapsed` u64]] |
| 12 | `Timeline { entries }` | `disclosed` [`count` varint, then per entry `key` i32 zigzag, `entity` u64] |
| 13 | `EntityFacts { entity, facts }` | `entity` u64, then the fact set: pools, effects, order, stats, tags, each a `disclosed` flag then its body |

**Limits, and Reading 8.** At most 16 pools, 16 effects, 32 stats, 8 tags,
1,024 timeline entries; stat names and tags are 1..=32 bytes; `round_ticks`
is nonzero and `elapsed` is at most `i32::MAX`. A count over its cap is
`LimitExceeded`, read before any element (a ten-byte hostile count
included). Shape faults are `Malformed`: `current > max`, a repeated pool or
stat name, effects not strictly ascending by id, tags not strictly ascending
by bytes, an empty name, a descending timeline key, a repeated timeline
entity, target 0. A stat `kind` byte over 1 is `UnknownMessage`. The public
`check_pools`, `check_effects`, `check_stats`, `check_tags`, `check_timeline`
and `ClockFact::check` are the encode-side checks and return exactly the
error `encode_delta` returns. The wire never truncates or refuses an
over-limit subject: the host (T140c) renders an over-limit family as
withheld, which fails closed and keeps the session going.

**Timeline tie rule.** Entries are in the authority's `(key, EntityId)`
order, but the client sees `NetId`s, whose order can differ inside a tie. The
codec therefore checks only non-decreasing keys and distinct entities, and
never `NetId` order. Hidden entities have no entry.

**`frame_spans`.** `codec_v3::frame_spans(ops)` is the one frame splitter. It
runs the per-op encode checks first (the first failure is returned, no
spans), then closes a frame exactly when the next op would make it hold more
than 256 ops or more than `MAX_FRAME_OP_BYTES_V3` = 65,512 bytes of encoded
ops (tags included). 65,512 is the 65,536-byte frame cap less the 24-byte
largest header (version 1, lane 1, two 10-byte varints, a 2-byte count), so a
lane-0 frame holding one span encodes for every tick and `event_seq`. No valid
op is larger than 15,364 bytes (`Timeline` of 1,024; an `EntityFacts` is at
most 2,755), so the split never fails on size. A page of v3 ops is not a
frame: counting ops alone (as the v1/v2 host does) would build frames that
`encode_delta` refuses with `FrameTooLarge`.

**`projection_v3`.** `project_events` is `projection_v2::project_events` for
v3: more than 256 candidates is `TooManyEvents`; candidates are visited in
input order and each fully disclosed one yields one op; nothing is sorted or
deduplicated. `V2(c)` yields exactly the v2 ops for `c`, converted with
`From`, and its `StringLimit`/`InvalidOutcome` fail the whole call.
`RoundStarted` needs `round`, `ModeChanged` needs `mode` and `round`,
`OrderEnded` needs `actor`, `Joined`/`Left` need `entity` and
`disclose == true`; a candidate missing any of these is omitted without
looking at its other fields. `tests/v3_facts_wire.rs` carries the literal
oracles (B1-B9). `codec_v3` also exposes crate-private `extend_*`/`take_*`
for the encounter block, the timeline block and the fact set, which T140d
reuses for the `0x82` body.

- 2026-11-10 (UTC) · claude-code/claude-sonnet-5-5 + T140b · Added the fact-op table, limits and Reading 8, the timeline tie rule, `frame_spans` with its 65,512-byte rule, and `projection_v3`; the T140a text that called tags 11..=13 unknown is superseded here.

- 2026-11-10 (UTC) · claude-code/claude-sonnet-5-5 + T140a · Added the Protocol v3 section for the version gate, intents, native tags 0..=10, codes 21..=30, the scalar ops and `Clock` as the only pause carrier; fact ops, filters and `0x82` follow in T140b–e.

## Agent log

- 2026-09-29 (UTC) · opencode/gpt-6-astra + T021 review fixes · Documented checked string-length overflow and transport-backed cache/retry conformance. These close malformed-length acceptance and replace codec-only redelivery evidence with actual dropped/duplicate delivery checks.

- 2026-09-28 (UTC) · opencode/muse-spark + T018a crate opening · Opened the doc with the lane-0 v1 protocol/codec/local-Transport shape, the frozen-wire and no-gameplay-execution boundaries, and the T018b/c plus evolution-queue plan, so later tasks extend rather than restart it.
- 2026-09-28 (UTC) · opencode/muse-spark + T018b simulated transport · Extended the scope and module flow with the injected-time fabric (rate → size → queue staging, seeded bounded faults, driver-owned seq/cache state) and the bytes-only fabric versus admission-drivers split, leaving T018c conformance and the evolution queue planned.
- 2026-09-28 (UTC) · opencode/muse-spark + T018c conformance · Extended the scope with the sim-backed driver, replica oracle, and four suites (rejection-without-mutation, filtered convergence with negative controls, 5,000-tick exercise, receipt liveness), closing the T018 proof with the evolution queue still planned.
- 2026-09-28 (UTC) · opencode/muse-spark + T018 review fixes · Recorded enforced failure-response budgets with wire drops, egress staging under v1_egress depths, reachable SeqExhausted precedence, and the bound driver peer table.
- 2026-09-29 (UTC) · opencode/muse-spark + T021 v2 event wire · Recorded the as-built explicit v2 modules (tagged history events beside frozen v1, bounded-before-allocation codec, host-fed ordered projection) and the six-case acceptance suite, so T022 consumes version-selected codecs plus ordered conformance evidence.
- 2026-10-04 (UTC) · claude-code + T023 · Appended a dated Scope pointer to `crpg-net-quic` for real QUIC, so this crate's no-I/O boundary names where the transport went; doc-only, no source or manifest change.

- 2026-10-25 (UTC) · opencode-go/beam-free + A0-8 · Absorbed the moved Public surface, Invariants, and Allowed dependencies sections from AGENTS.md; no content lost.
- 2026-11-10 (UTC) · claude-code/claude-sonnet-5-5 + T140a · Added the "Protocol v3 (T140, ADR-0029 N1–N4)" section for the new `protocol_v3`/`codec_v3` pair; no earlier section changed.
- 2026-11-10 (UTC) · claude-code/claude-sonnet-5-5 + T140b · Extended the Protocol v3 section with the fact ops, their limits, the timeline tie rule, `frame_spans` and `projection_v3`; no earlier section changed.
