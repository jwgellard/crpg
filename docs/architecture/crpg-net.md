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
- Decode order is contract, and it is wire order: length cap → version →
  every later field in its frozen order, each checked as it is read (a lane
  byte → `WrongDirection`; `seq` outside `SEQ_FIRST..=SEQ_LAST` or a zero
  `NetId` → `Malformed`; a discriminant tag or code → `UnknownMessage`; an
  op count over its bound → `LimitExceeded` before any op is read; a v1
  ability string read whole within the frame cap, then bounded, then parsed;
  a v2+ string length bounded before its body) → trailing-byte check. First
  failure wins and no check waits for a later field: an intent with `seq` 0
  and an unknown body tag is `Malformed`, not `UnknownMessage` (T042 X9).
  *2026-11-17 (UTC), T042: corrected; the earlier text put the `NetId`, `seq`
  and ULID checks after the trailing-byte check, which no codec does.*
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

## Malicious-client suite (T042)

T042 adds `tests/malicious_client.rs`, one table-driven suite over the whole
client-to-server byte boundary: the eight server-facing decoders (lane-0
v1–v4, the `0x81`/`0x82` transfer upstream, the `0x91` resume request and the
`0xA1` movement intent), each with its own encoder-built pinned fixture, its
own cap, version, lane and tag checks, every truncation and single-byte
corruption, and seeded noise. It is the wire-level answer to spec §16.2's
malicious-client mandate; host-level classes (ownership, timing, interest,
replay, rate) stay T041's.

A value a decoder accepts must re-encode canonically and the re-encoding must
decode back to itself (ULID case aliases re-encode canonically); the host's
replay cache compares those canonical bytes (`crpg-server` `Host::canonical`),
so a decoder that accepts what its encoder refuses, or re-encodes to bytes
that decode differently, is a defect. A new server-facing decoder is a new
`Wire` row in the same task.

| Spec class (§16.2) | Wire level, `crpg-net` | Host level |
|---|---|---|
| Oversized packets | `malicious_client.rs` X3 | T041; `crates/crpg-server/tests/host_quic.rs` `quic_malicious_frames_rejected_without_mutation` |
| Malformed frames | `malicious_client.rs` X2, X4–X8, X10–X13; the per-codec suites | T041; the same `host_quic.rs` test |
| Intents for entities not owned | X9 (zero ids only); `conformance.rs` `forged_and_cross_client_ids_rejected` (T018c double) | T041 (`NotAuthorized`) |
| Intents at illegal times | none: `observed_tick` is carried, not judged; `conformance.rs` `stale_and_future_ticks_rejected_terminally`, `unknown_ability_out_of_turn_and_self_target_rejected` (double) | T041 |
| Flooding | `transport.rs` `token_bucket_replenishes_exactly`, `rate_buckets_gate_and_replenish_exactly`; `token_bucket_regression.rs`; `conformance.rs` `over_rate_bursts_rejected_before_admit`, `slow_peer_terminates_with_queue_full_never_silent_loss` | T041 |
| Replay of old packets | none in a decoder (stateless); `reconnect_wire.rs` `fence_window_retries_lost_grants_without_reissuing`, `fenced_connection_frames_never_reach_admission`; `conformance.rs` `replayed_seq_same_bytes_cached_conflict_closes`, `seq_gap_does_not_advance_stale_never_reapplies` | T041 |
| Requesting entities outside the interest set | `interest.rs` `i08_retired_ids_do_not_resolve`; `conformance.rs` `revoked_visibility_ids_rejected` | T041 |
| Clean rejection and a log line | every refusal is a `CodecError` with a fixed text (X14) | Question 1's owner |

## Hostile-server suite (T042b)

T042b adds `tests/support/hostile.rs` and four table-driven suites over the
client side of the same byte boundary: `tests/hostile_delta.rs` (T042b1, the
lane-0 v1–v4 delta decoders on `EntityEnter`, `Health` and `Receipt`),
`tests/hostile_delta_ops.rs` (T042b2, the v2 event op, the v3 fact ops and the
v4 movement ops through the same four decoders), `tests/hostile_transfer.rs`
(T042b3, the `0x81`/`0x82` downstream frames and both snapshot bodies) and
`tests/hostile_session.rs` (T042b4, the `0x91` resume downstream and the
`0xA1` movement downstream). The surface is the ten decoders a server's bytes
reach: `codec`/`codec_v2`/`codec_v3`/`codec_v4::decode_delta`,
`codec_transfer::decode_down` and `decode_body`, `transfer_v3::decode_down`
and `decode_body`, `codec_resume::decode_down` and
`movement_v4::decode_down`. The QUIC `Welcome` decode, the client's close and
the Godot bridge's fault mapping are other crates and stay untested here. It
is the client-side mirror of T042's server-side suite; the argument and the
behavioural rows are [T042b](../../tasks/T042b.md).

The fixtures are encoder-built and pinned by literal, and each suite checks
decode precedence in wire order (cap, version, lane, tag, then fields, then
trailing bytes); every short input, truncation, foreign first byte, foreign
lane and upstream frame sent downstream; every tag or kind byte; and every
single-byte corruption. A corrupted byte must be a refusal other than
`FrameTooLarge`, the exact same bytes, a ULID case alias, or a *shorter*
string that is itself stable; the shorter branch is the one allowance for
`postcard`'s non-minimal varints. Seeded noise from one fixed seed adds a pass
that is not a row table. A decoder that accepts what its encoder refuses, or
whose re-encoding decodes differently, is a defect;
`single_byte_corruption_is_refused_or_exact` and `seeded_noise_is_stable` are
the tests that say so in each file.

| Spec class (§16.2) | Client-side decoders, `crpg-net` | Receivers and client programs |
|---|---|---|
| Oversized packets | `hostile_delta.rs`, `hostile_transfer.rs` and `hostile_session.rs` `oversize_is_refused_first` (delta frames, the `0x81`/`0x82` frames and both bodies, the `0x91`/`0xA1` frames); T042b1 covers the caps of the `hostile_delta_ops.rs` decoders | out of scope: the pools' size gates (T042b Reading 7) |
| Malformed frames | `hostile_delta.rs` `short_inputs_are_refused_exactly`, `every_truncation_is_malformed`, `trailing_bytes_are_malformed`, `foreign_first_bytes_are_unsupported`, `foreign_lanes_are_wrong_direction`, `op_tag_matrix`, `header_scalars_and_op_count`, `health_and_receipt_fields`; `hostile_delta_ops.rs` `every_truncation_is_malformed`, `trailing_bytes_are_malformed`, `foreign_first_bytes_are_unsupported`, `foreign_lanes_are_wrong_direction`, `action_resolved_fields`, `entity_facts_fields`, `encounter_timeline_clock_host_fields`, `moved_and_position_fields`; `hostile_transfer.rs` `short_inputs_are_refused_exactly`, `every_truncation_is_malformed`, `trailing_bytes_are_malformed`, `foreign_first_bytes_are_unsupported`, `foreign_lanes_are_wrong_direction`, `tag_matrix`, `frame_fields_sweeps_and_built_chunks`, `body_fields`; `hostile_session.rs` `short_inputs_are_refused_exactly`, `every_truncation_is_malformed`, `trailing_bytes_are_malformed`, `foreign_first_bytes_are_unsupported`, `foreign_lanes_are_wrong_direction`, `tag_and_kind_matrix`, `resume_fields`, `movement_fields` | the bridge's fault 52 (`crates/crpg-godot`); out of scope here |
| Intents for entities not owned | none: a decoder checks shape, not ownership (`hostile_delta.rs` `health_and_receipt_fields` pins that zero ids are refused) | the replica's refusals (T046, T140r2, T026p5 suites) |
| Intents at illegal times | none: `server_tick` and `event_seq` are carried, not judged (`hostile_delta.rs` `header_scalars_and_op_count`) | the receivers |
| Flooding | none: a decoder is a function of one byte slice and holds no budget | the client pools and the transfer receiver |
| Replay of old packets | none in a decoder (stateless); `hostile_delta.rs` `header_scalars_and_op_count` pins that `event_seq` 0 is accepted, which is wire shape, not replay | the replica and pool gates |
| Requesting entities outside the interest set | none: projection and interest are not decoders | `interest.rs`, `projection_v3`, `projection_v4` |
| Clean rejection and a log line | every refusal is an existing `CodecError` whose fixed `Display` text `malicious_client.rs` `refusal_texts_are_fixed` already pins (T042, the same type); upstream frames sent downstream are refused by `hostile_delta.rs` `upstream_intents_sent_down_are_refused` and by `hostile_transfer.rs` and `hostile_session.rs` `upstream_frames_sent_down_are_refused` | writing the line and the close (fault 52, `ProtocolViolation`) belong to the client programs (D67a), `crpg-net-quic`/`crpg-godot` |

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

### T140c: `facts_v3`, the per-viewer fact filter

`FactView` is the companion of the one `InterestView` of an epoch (it is not a
field of it, so T027b's structs and tests stay as they are). It mints nothing:
every `NetId` comes from `view.net_id`. It is driven only through
`project_page(view, facts, state, context)`, which validates before either view
mutates, so a call is all-or-nothing. `FactView` keeps the last encounter image,
the last timeline image and, per visible key, the `(NetId, FactSet)` it told the
viewer. It is not `Serialize` and has no restore constructor.

**Op ownership (extends the D5 table above).**

| Op | Owner |
|---|---|
| `EntityEnter`, `Health`, `Turn`, `EntityLeave`, `Despawned` | `InterestView` |
| `Encounter`, `Timeline`, `EntityFacts` | `FactView` (`project_page`) |
| `Spawned`, `Died`, `ActionResolved`, `TurnStarted`, `EncounterEnded`, `RoundStarted`, `ModeChanged`, `OrderEnded`, `Joined`, `Left` | `projection_v3::project_events` |
| `Clock`, `HostFacts` | `clock_op` / `host_facts_op` with the host's per-viewer `HostGrants` |
| `Receipt` | host |

**Checks (C-1), before anything changes, first failure wins.** The state's or
the interest view's epoch differs from the fact view's (`WrongEpoch`); more
than 1,024 rows or 1,024 timeline entries (`TooManyFacts`, counted before
duplicates); two rows, or two timeline entries, with one `EntityKey`
(`DuplicateFact`, visible or not); then `project_delta`'s own error as
`Interest(e)`. The timeline checks read the input whenever an encounter is
supplied, whatever its area or timeline grant. After the interest projection
nothing can fail.

**Images.** The encounter image is `EncounterFacts::default()` unless the
encounter is in the viewer's area; then each field is present only under its
grant, and a clock that fails `ClockFact::check` is withheld, never refused. The
timeline image is `Some` only for an in-area encounter with the timeline grant:
the input sorted ascending by `(key, EntityKey)`, hidden entities dropped, so
ties keep the authority's order, not the `NetId` order. A visible key renders
a `FactSet`: a family needs its `FactMask` flag and its `check_*` (Reading 8:
over-limit is withheld, never truncated or refused); the order is `None`
without `fields.order`, `Some(None)` with no order, and withheld when its target
has no `NetId` for this viewer. A visible key with no row renders
`FactSet::default()`. Rows for keys that are not visible are ignored.

**Memory and emission (C-5, C-6).** Before rendering, every remembered key whose
current `net_id` is not the stored one is dropped (it left, was retired, or
re-entered with a new id, including a retirement made by a snapshot applied
straight to the view). Fact ops follow the view's ops: `Encounter`, then
`Timeline`, then one `EntityFacts` per visible key in ascending `NetId`. Each is
sent when its image differs from the remembered one (no remembered row is the
default image), or under `resend` when the image is not withheld; then every
image is remembered. A withheld image that changes is therefore still sent
once, as the forget op (an all-`None` op), and never repeated. `PageV3.ops` is
the view's ops converted with `From`, then the fact ops; `enters` and `cursor`
are the view's, so fact ops alone do not advance the cursor. A page holds at
most `MAX_PAGE_OPS_V3` = 4,099 ops and is not a frame: split it only with
`codec_v3::frame_spans`.

**Grants.** `FactMask`, `EncounterGrants` and `HostGrants` all default to deny.
No input names an owner or controller: the host sets `fields.order` (and the
`OrderEnded` candidate) only for the controlling peer.

**Host facts (C-9, Reading 13).** `clock_op` and `host_facts_op` are stateless
and pure. Two host facts are deliberately not granted: whether the session is
paused (every client observes a pause) and the viewer's own pause rights (they
describe only that viewer). The pause reason, the auto-pause toggles and the
turn timer each have a `HostGrants` flag; withheld is `None`, never "no timer
running" (`Some(None)`). `tests/v3_fact_view.rs` carries the oracles (C1-C15
and the grant, order, memory and bound cases).

- 2026-11-10 (UTC) · claude-code/claude-sonnet-5-5 + T140c · Added the `facts_v3` section: the extended op-owner table, the C-1 check order, the images, memory and emission rules, and Reading 13's two ungranted host facts.

- 2026-11-10 (UTC) · claude-code/claude-sonnet-5-5 + T140b · Added the fact-op table, limits and Reading 8, the timeline tie rule, `frame_spans` with its 65,512-byte rule, and `projection_v3`; the T140a text that called tags 11..=13 unknown is superseded here.

- 2026-11-10 (UTC) · claude-code/claude-sonnet-5-5 + T140a · Added the Protocol v3 section for the version gate, intents, native tags 0..=10, codes 21..=30, the scalar ops and `Clock` as the only pause carrier; fact ops, filters and `0x82` follow in T140b–e.

### T140d: `transfer_v3`, the `0x82` snapshot

`0x82` is the `0x81` transfer subprotocol (T024) with a new first byte, a wider
body and a larger envelope. Every frame layout is `0x81`'s and every check is
T024's, in T024's order; exactly three bounds differ. The Begin `total_bytes`
cap is `MAX_SNAPSHOT_BYTES_V3` = 4,194,304 (4 MiB), and the Begin `chunk_count`
cap and the Chunk `index` bound (encode and decode) are
`MAX_SNAPSHOT_CHUNKS_V3` = 128 (4 MiB / 32,768). Segments (4,096 bytes, at most
8 per chunk), chunks (32,768 bytes), the tags, the 65,536/4,096-byte frame caps
and the canonical cut are unchanged. Because `BeginTransfer::validate`, the
`0x81` codec, `TransferAssembly` and `TransferLimits` hard-code the v1 caps
(1 MiB, 32 chunks), `transfer_v3` encodes and decodes Begin and Chunk itself
through `check_begin` and `chunk_len` (`validate` and `chunk_len` with the v3
caps, in their order), and copies the `Cancel`, `Published` and `Resync`
codecs. The v1/v2/`0x81` constants in `protocol` stay 1 MiB and 32 chunks;
`BeginTransfer`'s fields already hold the wider values (`total_bytes: u32`,
`chunk_count: u8`, 128 fits one raw byte), so no layout changes. The `0x81`
decoders and `codec_v3` refuse `0x82` with `UnsupportedVersion`, and the v3
transfer decoders refuse every other first byte, `0x81` included.

**Body (D-2).** `entity_count` varint, then per entity `entity u64`,
`health_present` and its three fields, and the fact set (the T140b layout:
pools, effects, order, stats, tags); then `turn_present` (`active`, `round`),
the encounter block (the tag 11 payload) and the timeline block (the tag 12
payload). `SnapshotBodyV3 { entities, turn, encounter, timeline }` reuses the
T140b `take_*`/`extend_*` helpers. Host facts are not in the body: they are per
viewer and not state at the cursor, so the host sends them as ops after
publication.

**Decode order (D-3).** Body over 4,194,304 bytes (`LimitExceeded`); entity
count over 1,024 (`LimitExceeded`, before growth); per entity the id (0 is
`Malformed`), strict ascent (`Malformed`), health, then the fact set; then each
order target must be an entity of the body; then the turn (`active` must be an
entity); the encounter; the timeline (every entry's entity must be an entity);
trailing bytes. `encode_body` checks in the same order and returns the same
error (D-4), so it refuses exactly what `decode_body` refuses.

**Size (Readings 14, 15).** The largest valid body is 2,847,788 bytes: 2
(count) + 1,024 x 2,766 (entity 10, health 12, pools 2 + 16 x 37, effects
2 + 16 x 37, order 40, stats 2 + 32 x 39, tags 2 + 8 x 33) + 22 (turn) + 17
(encounter) + 15,363 (timeline). The smallest power-of-two-MiB cap that holds
it is 4 MiB, which leaves 1,346,516 bytes of headroom; it needs 87 chunks at 8
segments per chunk (the last holds 29,740 bytes), and its plan is 88 frames and
2,851,124 bytes. That is over the 2 MiB per-peer egress bytes, so the host
paces `0x82` frames (H-7); the egress caps do not grow.

**`plan_transfer` (D-5).** Checks in order: `cursor` above `SEQ_LAST`
(`Malformed`); `segments_per_chunk` 0 (`Malformed`) or above 8
(`LimitExceeded`); the body (`encode_body`'s error); then a chunk count above
128, computed in `usize` before any narrowing to `u8` (one segment gives 696).
It builds the Begin, cuts the chunks canonically and returns `encode_down` of
the Begin then of each Chunk. The cap binds only below 8 segments per chunk.

**`project_snapshot_v3` (D-6).** It runs `project_page`'s checks (epochs,
bounds, duplicate keys) and then `InterestView::project_snapshot`, so a refusal
leaves both views unchanged. The body is the view body's entities in order,
each with its health and its row image (`FactSet::default()` without a row),
the view's turn, and the encounter and timeline images. The `FactView` then
replaces its memory with exactly these images, so the next `project_page` emits
only differences; a key the snapshot retired forgets its image. The body always
passes `encode_body`. `tests/v3_snapshot.rs` carries the oracles (D1-D10).

### T140e: `reassembly_v3`, v3 receiver sizing

`reassembly_v3` re-sizes T024's receiver for the 4 MiB `0x82` envelope. It
fixes no byte: every number is policy. It reuses `AssemblyError`,
`ChunkOutcome` and `LimitsError` from `reassembly` by path and edits no
existing module. `TransferAssembly` and `TransferLimits` keep the v1 caps (1
MiB, 32 chunks, a `u32` bitmap) and stay byte- and test-identical.

**`TransferAssemblyV3`.** `TransferAssembly`'s behaviour with a `u128` chunk
bitmap and the v3 Begin rules: `new` is `transfer_v3::check_begin` and then a
zeroed `total_bytes` buffer; `accept` checks the transfer (`WrongTransfer`),
the index against `chunk_count` (`OutOfRange`), the length against
`transfer_v3::chunk_len` (`WrongLength`), then a set bit (`Duplicate` for equal
bytes, `Conflict` otherwise); an `Err` or `Duplicate` changes nothing. Inside
the v1 envelope it equals `TransferAssembly` call by call (E3).

**`TransferLimitsV3` (Reading 18).** Same seven fields in the same order as
`TransferLimits`, with `v3()` at the upper bound of every range:
`{ 8, 20_000, 4_194_304, 33_554_432, 128, 2_097_152, 16_777_216 }`. Staging
per receiver is the whole envelope (4,194,304), floored at the largest valid
body (2,847,788, as v1's floor is its largest body); pool staging is 8 x 4 MiB,
so eight maximal transfers still fit at once. The deadline is 20,000 ms: T024's
5 s per 1 MiB at four times the bytes. The held-delta budgets stay 128 frames,
2 MiB per receiver and 16 MiB per pool, because T024 tied them to the egress
caps (`POLICY_EGRESS_*`), which do not change; a receiver whose held deltas
overflow during a slow transfer resyncs by T024's existing overflow rule.
Worst-case pool memory goes from 24 MiB to 48 MiB (32 MiB staging + 16 MiB held
deltas), plus decoded bodies. `validate` checks the fields in declaration order,
first failure wins; per field, zero or under the floor is `InvalidLimits`
before an over-bound `LoosensPolicy`. The bounds are read by path from
`protocol` and `transfer_v3` rather than repeated.

**Transport (Reading 19).** The `crpg-net-quic` windows are flow control, not
size caps, so a larger transfer needs no change there. What bounds a `0x82`
plan is the egress queue: the largest plan is 88 frames and 2,851,124 bytes,
above the 2 MiB per-peer egress bytes, so the host paces the frames (H-7, T141)
and the egress caps do not grow.

The client receiver (`ReassemblyPool` gating of v3 deltas, publication of
`0x82` bodies, a v3 `ReplicaWorld`) is a later `crpg-net` task built on these
two types. `tests/v3_reassembly.rs` carries the oracles (E1-E6).

### T140r1: `pool_v3`, the v3 receiver gate

`pool_v3` is T024's reassembly and cutover gate for one v3 session. It
reuses T024's vocabulary by path and adds a v3 type only where the v1 type
cannot serve: `TransferOutcome`, `DeltaOutcome`, `TransferError`,
`FailReason`, `GateMode`, `InFlightStatus`, `PublishedBase`, `Rebound` and
`ResyncReason` are re-exported unchanged, and `TransferLimitsV3` /
`LimitsError` come from T140e. New are `PublicationV3` (the `0x82`
transfer, its base tick and cursor, and the decoded `SnapshotBodyV3`),
`ReplicaUpdateV3` (`Snapshot(PublicationV3)` or `Delta(DeltaFrame)`, in FIFO
order), `ReceiverIdV3` (a separate id type, so a v1 receiver cannot be
passed to a v3 pool) and `ReceiverStatusV3` (T024's status without the
`wire` field). `ReceiverIdV3`'s field is private, so only the pool mints one.

**Transliteration rule (P-1).** `ReassemblyPoolV3`'s private types and every
method body are `reassembly.rs:438–1105` with exactly these substitutions,
and nothing else:

| In `reassembly.rs` | In `pool_v3.rs` |
|---|---|
| `TransferLimits` | `TransferLimitsV3` |
| `TransferAssembly` (type, `::new`, `accept`, `into_bytes`) | `TransferAssemblyV3` |
| `codec_transfer::decode_down` | `transfer_v3::decode_down` |
| `codec_transfer::decode_body` | `transfer_v3::decode_body` |
| the `match wire { V1 => …, V2 => … }` decode | `codec_v3::decode_delta(bytes)` |
| `GatedDelta` / `gated.event_seq()` | `DeltaFrame` / `frame.event_seq` |
| `Publication`, `ReplicaUpdate`, `ReceiverId`, `ReceiverStatus` | `PublicationV3`, `ReplicaUpdateV3`, `ReceiverIdV3`, `ReceiverStatusV3` |
| `Receiver.wire`, `add_receiver`'s `wire` parameter, `ReceiverStatus.wire` | removed |

Check order, saturating arithmetic, the clamp-then-sweep prologue, the
`BTreeMap` iteration order and FIFO queue handling stay T024's. The
substitution is proved two ways: the P8 mirror runs the same script on a
`ReassemblyPool` (V2 receivers) and on `ReassemblyPoolV3` and compares every
result, status, total and popped update after each step, and a reviewer diffs
the two ranges hunk by hunk. A fix to one pool is therefore a fix to both,
in one task.

**Sizing is T140e's.** The pool takes `TransferLimitsV3` and calls
`validate()` unchanged; it re-sizes nothing. The client receiver that
T140e's section calls "a later task" is this module for the gate half: `0x82`
bodies are decoded and published, v3 deltas are gated by `event_seq`, and
the queued `ReplicaUpdateV3`s feed T140r2's `ReplicaWorldV3`.

**Version isolation.** One pool holds one wire. `ReassemblyPoolV3` accepts
only `0x82` transfer frames and v3 delta frames; v1, v2 and `0x81` bytes are
`Codec(UnsupportedVersion)`. `ReassemblyPool` is unchanged and still refuses
both v3 forms the same way. The host (T141) picks the pool per session; there
is no fallback or negotiation.

### T140r2: `replica_v3`, the v3 client replica

`replica_v3` is T046's client replica for one filtered v3 session. It reuses
`ReplicaError` and `MAX_REPLICA_ENTITIES` from `replica` by path — no new
error type and no new variant, so the bridge's fatal-53 mapping and T046's
`Display` strings are unchanged — and holds `ReplicaEntityV3` (the row's
`net_id`, its optional `HealthFact` and its complete `FactSet`). New types are
`PauseState` (`Running`, or `Paused { reason }` with a withheld reason `None`)
and `ReplicaWorldV3`.

**State.** The v2 replica's rows, turn, `next_event_seq`, `server_tick` and
`high_water`, plus the v3 additions: per entity the `FactSet`, the encounter
image (`EncounterFacts::default()` is every family withheld), the timeline
image (`Option<Vec<TimelineEntry>>`, `None` is withheld/forget), the pause
state (`None` before the first `Clock` op) and the viewer's host facts (`None`
before the first `HostFacts` op). The epoch is fixed at `new`. The only
`&mut self` methods are `apply_delta` and `apply_snapshot`; every query reads
committed state.

**The op table (R-5..R-8).** Tags 0..=10 behave exactly as T046 item 6 on the
v3 variants of the same name: `EntityEnter` refuses a live id
(`DuplicateEntity`), an id at or below `high_water` (`ReusedEntityId`) then a
full row set (`TooManyEntities`); `EntityLeave`/`Despawned` require a live id
and remove the row with its health and facts; `Spawned`/`Died` require a live
id and change nothing; `Health` replaces the health whole; `Turn` checks a
`Some` active and replaces the turn whole; `Receipt` checks the epoch;
`ActionResolved` checks actor then target; `TurnStarted` checks the actor;
`EncounterEnded` clears nothing. `Encounter(e)` stores `e` whole. `Timeline {
entries }` checks every entry's `entity` is live, then stores the image in
input order (`None` included). `EntityFacts { entity, facts }` checks the row
is live, then, when `facts.order == Some(Some(o))`, that `o.target` is live,
then stores the fact set whole. `RoundStarted`, `ModeChanged`, `OrderEnded`,
`Joined` and `Left` (tags 14..=18) are accepted and dropped: their ids are
host-supplied mappings and no recorded rule places them, so checking them
would make an unrecorded host choice fatal; a presenter pairs a notice's id
with `entity()` and treats a miss as "not visible". `Clock { paused }` sets
`Running` for `None` and `Paused { reason }` for `Some(reason)`;
`HostFacts(h)` stores `h`.

**The liveness rule and why it holds (Reading 7).** Every id a v3 op names
must be live when that op applies, checked against the working state the ops
before it left. Legal pages satisfy this because T140c emits the fact ops
after the whole interest page (enters and removals included) and resolves
every `NetId` against the ids live after that page; the host splits pages with
`frame_spans` at one `event_seq` per span. So the checked ids are T046's for
tags 0..=10 plus `EntityFacts.entity`, its order's `target`, and every
`Timeline` entry's `entity`. T046's checks on `ActionResolved`/`TurnStarted`
keep resting on the recorded history slot, after the enters and before the
removals (T027b D6); `T140 H-4` fixes only journal order, so no new T141 rule
is assumed.

**Stale references (Reading 8).** References are checked once, when the op
applies; stored ones may go stale. A later removal edits no stored order
target, timeline entry or `turn.active`. A new op naming such an id is refused
by the same liveness rule. **Images (Reading 9).** `Encounter`, `Timeline` and
`EntityFacts` each replace their subject's image whole; an all-withheld image
is stored as such (forget). The replica re-runs no codec value check (counts,
`current ≤ max`, ordering, duplicates): those belong to the decoder, exactly
as T046 stores health verbatim.

**Delta precedence and atomicity.** `apply_delta` checks lane
(`WrongLane`), op count (`TooManyOps`), then sequence (`event_seq` not the next
expected one, or above `SEQ_LAST`, is `EventSeq`), then applies the ops to a
working clone and commits only on `Ok` (T046 D9), setting
`next_event_seq = event_seq + 1` and `server_tick`. **Snapshot precedence.**
`apply_snapshot` checks in `transfer_v3`'s body decode order: cursor
(`SnapshotCursor` with `min = next_event_seq − 1` saturating), the visible
bound (`TooManyEntities`), row order (`SnapshotOrder`) and id reuse
(`ReusedEntityId`), then order targets, then the turn's active, then the
timeline entities (`UnknownEntity`). On success it replaces the rows, turn,
encounter and timeline whole, sets `next_event_seq = cursor + 1` (saturating)
and `server_tick = base_tick`, and raises `high_water` to the last row's id.

**Host state across a snapshot (Reading 11).** `Clock` and `HostFacts` are not
in the `0x82` body; the host re-sends both after every publication (T140
H-2). `apply_snapshot` therefore leaves `pause` and `host_facts` as they are,
and both are `None` until their first op.

## Protocol v4 (T026p, ADR-0036 L1–L6)

`protocol_v4` / `codec_v4` add lane-0 wire version 4 beside frozen v1, v2,
v3, `0x81`, `0x82` and `0x91`. T026p1 fixes lane 0: every v3 message by its
v3 bytes, the turn-based `DeclareMove` intent, and the `Moved` and `Position`
delta ops over the bounded `Point` type. T026p2 (below) fixes lane 1 (`0xA1`).
The position filter, the v4 pool, the replica and the outbox are T026p3–p5 and
extend this section as they land.

**Lanes.** A v4 session is lane-0 version 4 plus the movement subprotocol
version `0xA1` (T026p2). Each lane keeps its own first byte, so one can change
without a lane-0 version and the reverse:

| Lane | Carries | First byte | Module |
|---|---|---|---|
| 0 | combat and session (v1–v4) | 1, 2, 3, 4 | `protocol` … `protocol_v4` |
| 1 | movement intents, receipts, position samples (T026p2) | `0xA1` | `movement_v4` |
| 2 | snapshot transfer | `0x81`, `0x82` | `protocol_transfer`, `transfer_v3` |
| 3 | session resume | `0x91` | `protocol_resume` |

Every codec refuses another version's first byte with `UnsupportedVersion`
and a foreign lane with `WrongDirection`; the v1, v2 and v3 codecs refuse
byte 4 the same way. There is no negotiation: the host picks the versions per
session.

**v3 ops are wrapped, not re-declared.** `DeltaOp::V3(op)` carries every v3
op (tags 0..=20) with exactly its v3 bytes, so byte identity holds by
construction instead of by a second 21-variant copy to keep in step. The
v4 decoder reads a tag of 0..=20 only through `codec_v3::take_op`, and the
encoder writes it only through `codec_v3::extend_op`; those two functions
became `pub(crate)` for this (the only edit to `codec_v3`). Intents have no
per-body function to call, so v4 re-declares the six small v3 bodies (tags
0..=5, `From<protocol_v3::IntentBody>` converts) and adds tag 6.
`RejectionCode`, `ReceiptStatus` and `EncounterMode` are v3's types
re-exported; **v4 adds no rejection code** (`DeclareMove` refusals reuse
`NotAuthorized`, `WrongMode`, `IllegalAction` and the admission codes).

**Intent.** `04`, `epoch`, `lane` 0, `seq`, `observed_tick`, `actor`, body
tag, body. Tag 6 `DeclareMove` is one point, and the header's actor moves
(ADR-0036 L6). Decode order is v3's with tag 6 added: cap, version, `epoch`,
`lane`, `seq`, `observed_tick`, `actor`, tag (7..=255 `UnknownMessage`), body,
trailing bytes.

**Delta.** `04`, `lane` 0, `server_tick`, `event_seq`, op count (at most
256, checked before any op is read), then ops. Tag 21 `Moved` is `entity`, a
count varint (2..=64, both bounds checked before any point is read) and the
points; a point equal to the previous one in `x` and `z` is `Malformed` (a
zero horizontal segment, exactly sim's `ZeroSegment`; `y` may differ). Route
length is not checked on the wire: it needs a square root, and sim validated
it. Tag 22 `Position` is `entity`, a disclosed flag and, only when set, a
point and a `moving` flag: `None` means "withheld from now on while the entity
stays visible" (the forget image), and has no bytes after its `00` flag. Tags
23..=255 are `UnknownMessage`. `check_points` is the encode check for
`Moved`, in the decoder's order, and returns exactly the error `encode_delta`
does.

**`Point` and U3.** A position is raw `Fx16_16` metres `[x, y, z]`; `Point`'s
field is private and `Point::new` refuses any component whose `unsigned_abs()`
exceeds `COORD_LIMIT_RAW` (2^29, ADR-0036 U3), so a `Point` cannot be out of
bounds and an encoded one is at most 15 bytes (a coordinate is a zigzag `i32`,
at most 5 bytes). The decoder checks each coordinate when it is read: `x`
before `y` before `z`. `COORD_LIMIT_RAW` and `MAX_MOVED_POINTS` (64) are
declared here because net cannot use sim outside tests; test W11 asserts they
equal `crpg_sim::COORD_LIMIT_RAW` and `MAX_ROUTE_POINTS`.

**`frame_spans`.** T140 B-6's greedy split over v4 ops: every op's encode check
runs first (first failure, no spans), then a frame closes when one more op
would pass 256 ops or `MAX_FRAME_OP_BYTES_V3` (65,512) op bytes. The v4 header
is at most 24 bytes, so one span always encodes within 65,536 bytes. For all
`V3` ops it equals `codec_v3::frame_spans`. The largest `Moved` is 972 bytes
and the largest `Position` 28.

**Errors.** No new variant: `CodecError` covers every refusal (see T140a's
list; the new ones are intent tag 7..=255 and op tag 23..=255 as
`UnknownMessage`; a `Moved` count over 64 as `LimitExceeded`; a count under 2,
a zero horizontal segment, a coordinate beyond U3 and a flag byte other than
0/1 as `Malformed`). `tests/v4_wire.rs` carries the literal byte oracles
(W1–W11) and a differential against the v3 codec over every v3 op and a
mutation corpus.

### T026p2: `movement_v4`, lane 1

Lane 1 is its own separately versioned subprotocol (D13): every message starts
with `MOVEMENT_VERSION` (`0xA1`) and carries `lane` 1 as a second guard.
Lane-0 v4 starts with byte 4, so `codec_v4::decode_intent` and `decode_delta`
refuse lane-1 bytes, and `movement_v4` refuses a lane-0 v4 intent, with
`UnsupportedVersion`; the v1, v2, v3, `0x81`/`0x82` and `0x91` codecs refuse
`0xA1` the same way. A "v4 session" is lane-0 version 4 with movement version
`0xA1`, picked by the host; there is no negotiation. The module reads no
`World`, checks shape and not entitlement, and reuses v3's `RejectionCode`
(gameplay refusals are receipts, never codec errors). It calls
`codec_v4::{extend_point, take_point}` and duplicates only the five postcard
helpers it uses.

**Four messages.**

| Message | Direction | Layout after `0xA1` |
|---|---|---|
| `MoveIntent` | up | `epoch[16]`, `lane` 1, `seq`, `actor`, tag: 0 `MoveTo` + one point, 1 `Halt` |
| `MoveReceipt` | down, kind 0 | `lane` 1, `kind` 0, `epoch[16]`, `seq`, `actor`, `processed_tick`, outcome: 0 `Accepted` + `start_tick`, 1 `Superseded`, 2 `Rejected` + v3 code 1..=30 |
| `MoveSample` | down, kind 1 | `lane` 1, `kind` 1, `server_tick`, `processed_seq` (0..=`SEQ_LAST`), count varint (at most 38), per entry `entity`, point, `moving` |
| `MoveDown` | down | the sum of the receipt and the sample, which `decode_down` returns |

A lane-1 intent has **no `observed_tick`** (ADR-0036 L2: "There are no other
fields"). The largest intent is 54 bytes and the largest receipt 60. Intents
are per input and reliable, each earning exactly one receipt in `seq` order
(L2, L3); the host keeps the latest target per actor per tick and answers the
earlier ones `Superseded` (H2). A receipt carries `epoch` and `processed_tick`
beyond ADR's `{ seq, actor, outcome }` so it names the processed `seq` and a
server tick without using the tick as identity.

**Sample bounds (Reading 6).** A sample is one self-contained datagram of at
most `MAX_MOVE_DOWN_BYTES` (1,024) bytes and `MAX_SAMPLE_ENTRIES` (38) entries.
QUIC guarantees 1,200-byte UDP payloads and a 1-RTT packet spends at most about
44 bytes on its header, tag and DATAGRAM framing, so 1,024 fits inside that.
38 is the largest `n` with 24 + 26 `n` <= 1,024 (`MAX_SAMPLE_HEADER_BYTES`
24, `MAX_SAMPLE_ENTRY_BYTES` 26), so a sample within the count cap always fits
(1,012 bytes at most) and splitting is fixed chunking; two `const` assertions
pin both sides of that, so 38 cannot move alone. Entries are in strictly
ascending `NetId`, checked on decode and by `check_sample`, which `encode_down`
returns the error of (order: `processed_seq` above `SEQ_LAST`, more than 38
entries, then entity order). A sample has no epoch, because a datagram belongs
to one connection; `processed_seq` is the highest lane-1 `seq` receipted to
the receiving peer (0 before any). `sample_datagrams(tick, processed_seq,
entries)` cuts one tick's entries into chunks of 38 and encodes each; empty
input gives no datagrams, and it checks `processed_seq` and strict ascent over
the whole input before any output, so a fault in a late chunk yields nothing.
1,024 visible entities make 27 datagrams (26 of 38, one of 36).

**Decode order** is contract in both directions: whole-message cap (4,096 for an
intent, 1,024 downstream, `FrameTooLarge`), empty (`Malformed`), version,
`lane`, then fields in wire order, each checked when read, then trailing
bytes. A varint count that overflows `u64` is `Malformed`; a count over 38 is
`LimitExceeded` before any entry is read.

**Policies (Reading 14, U7).** `POLICY_MOVE_INTENTS_PER_SEC` 20,
`POLICY_MOVE_INTENT_BURST` 40 and `POLICY_MOVE_IN_FLIGHT` 8 are host-enforced,
tightening-only and never on the wire; a `const` assertion keeps the in-flight
limit within the retry cache. Lane-1 traffic also counts toward every existing
aggregate (`POLICY_INTENT_*`, the ingress and egress caps, the failure
budgets). The interpolation delay is client policy. `tests/v4_movement.rs`
carries the literal byte oracles (M1–M8).

### T026p3: `positions_v4` and `projection_v4`, the position filter

`positions_v4` decides who is told where an entity is; the host supplies every
position and grant (`PositionRow { key, granted, point, moving }`), and the
module reads no `World`, RNG or clock. **Disclosure is a reliable anchor, then
unreliable samples (Reading 7).** The lane-0 op `Position { Some }` starts
disclosure of one entity's position (and repeats it on a resend page);
`Position { None }` stops it while the entity stays visible, the "forget"
image. Values between those two travel only in lane-1 samples, so a walking
creature costs no lane-0 traffic. An entity that leaves view needs no
`Position` op: `EntityLeave` removes it.

**`PositionView`** is the per-viewer companion of one `InterestView` and one
`FactView`: per visible key it remembers the `PositionFact` the viewer was last
told and the `NetId` it was told under. It mints no id (every `NetId` is read
from the `InterestView`), is not `Serialize` and has no restore constructor.

**Op ownership (extends the table above).**

| Op | Owner |
|---|---|
| `Position` | `PositionView` (`positions_v4::project_page_v4`) |
| `Moved` | `projection_v4::project_events` |
| every v3 op, wrapped in `V3` | its v3 owner, unchanged |
| position samples (lane 1, not an op) | `positions_v4::project_samples` |

**`project_page_v4`** checks before anything changes, first failure wins: the
`PositionView`'s epoch against the `FactView`'s (`WrongEpoch`); more than 1,024
rows (`TooManyFacts`); two rows with one key (`DuplicateFact`); then
`facts_v3::project_page` with its error unchanged. The position checks come
before every fact check, and every `Err` leaves all three views equal to their
clones. Then it forgets, silently, each remembered key whose `view.net_id` is
no longer the id it was told under (left, retired, or re-entered under a new
id), and emits at most one `Position` per visible key in ascending `NetId`,
after `project_page`'s ops wrapped in `V3`: a granted row not yet told is
anchored; a told key whose row is not granted (or is gone) is withdrawn with
`None`; on a resend page every granted position is re-anchored; otherwise
nothing, and the memory keeps the old value (a changed point is the samples'
job). A page is at most `MAX_PAGE_OPS_V4` (5,123) ops and is not a frame.

**`project_samples`** reads the current grant on every call. For each
remembered key in ascending `NetId` whose id is still current and whose row is
granted now, it emits a `SampleEntry` when the value differs from the told one
(or on `refresh`) and remembers it; any other remembered key gives nothing and
keeps its memory. It adds no key, so a position is sampled only after its
anchor, and **revocation is immediate**: a row that stops being granted leaves
the samples in that call, ahead of the page whose `None` tells the client to
forget. Its checks are the page's position checks (epoch against the interest
view's, then rows); `positions` is untouched on `Err`. Cut the result into
datagrams with `movement_v4::sample_datagrams`.

**Reconnect (Reading 8).** `project_snapshot_v4` checks the epoch, runs
`facts_v3::project_snapshot_v3` (its error unchanged, nothing changed on `Err`)
and on `Ok` returns that result untouched and empties the `PositionView`. The
`0x82` body carries no positions; the next `project_page_v4` re-anchors every
granted visible position in the first delta above the snapshot's cursor, and
`project_samples` returns nothing until then. This meets ADR-0036 L5 ("resync
snapshots carry only granted positions") without a `0x83` body.

**`Moved` disclosure (Reading 9).** `projection_v4::EventCandidate` is `V3(c)`
(exactly `projection_v3::project_events(&[c])`, wrapped; its errors fail the
call) or `Moved { entity, points, disclose }`, which yields a `Moved` op
exactly when `entity` is `Some`, `disclose` is true and `check_points` is
`Ok`; otherwise it is withheld. More than 256 candidates is `TooManyEvents`.
Order is the input's, with no sort and no deduplication. `disclose` is the
host's position grant for the viewer, as `Joined`/`Left`'s is for notices.
`tests/v4_positions.rs` (Q1..Q12 plus ordering and bound cases) is the suite.

### T026p4: `pool_v4`, the v4 receiver gate

`pool_v4` is `pool_v3` for one v4 session: `ReassemblyPoolV4` is
`ReassemblyPoolV3` with the v4 delta decoder and nothing else changed. It
re-exports, by path, everything the v3 pool already shares (`PublicationV3`,
`ReceiverStatusV3`, `TransferLimitsV3`, `LimitsError`, `ResyncReason` and
T024's outcome, error, status and mode types), so a v4 receiver's status and
publications have exactly the v3 types. New are `ReceiverIdV4` (a separate
id type with a private field, so a v3 receiver cannot be passed to a v4
pool) and `ReplicaUpdateV4` (`Snapshot(PublicationV3)` or
`Delta(protocol_v4::DeltaFrame)`, in FIFO order). The pool reads no op: a
decoded v4 delta, `Moved` and `Position` included, is gated by `event_seq`
alone and released as decoded.

**Transliteration rule (V-1).** `pool_v4.rs` is `pool_v3.rs` with exactly
these substitutions and nothing else:

| In `pool_v3.rs` | In `pool_v4.rs` |
|---|---|
| `use crate::codec_v3::decode_delta;` | `use crate::codec_v4::decode_delta;` |
| `use crate::protocol_v3::DeltaFrame;` | `use crate::protocol_v4::DeltaFrame;` |
| the definitions of `PublicationV3` and `ReceiverStatusV3` | `pub use crate::pool_v3::{PublicationV3, ReceiverStatusV3};` |
| `ReceiverIdV3`, `ReplicaUpdateV3`, `ReassemblyPoolV3` | `ReceiverIdV4`, `ReplicaUpdateV4`, `ReassemblyPoolV4` |

`TransferAssemblyV3`, `TransferLimitsV3` and every `transfer_v3` call stay:
the `0x82` snapshot is the v3 snapshot, so a v4 session transfers the v3
body. Check order, saturating arithmetic, the clamp-then-sweep prologue,
`BTreeMap` order and FIFO handling are unchanged. The check is runnable
(contract T026p V-1, bash, repository root; it must print nothing, and the
reviewer runs it): `diff -B` of `pool_v3.rs` after `sed` deletes lines 39-50
and 72-95 and applies the five name substitutions, against `pool_v4.rs` less
the one `pub use` line, both with whole-line `//` comments removed. Lines
39-50 and 72-95 are the two re-exported definitions with their docs and
derives, so a change to `pool_v3.rs` that moves them breaks the check on
purpose.

**Proved two ways.** `tests/v4_pool.rs` drives one script on a `ReassemblyPoolV3`
and a `ReassemblyPoolV4` of the same limits (the same transfer bytes, each
delta's v3 bytes with byte 0 changed from 3 to 4) and compares every result,
every receiver's whole status, both pool totals and every popped update (a v4
`Delta` against the v3 frame with its ops wrapped in `V3`), after every step.
V1 is T140r's P8 script; V5 is one test each for its P2, P3, P4, P5, P9,
P10 and P11. The v3 pool is the oracle, so a fix to one pool that is not made
to the other fails the mirror.

**Version isolation.** One pool holds one wire. `ReassemblyPoolV4` accepts
`0x82` transfer frames and v4 delta frames only: v1, v2 and v3 delta bytes,
the lane-1 `0xA1` sample bytes and `0x81` frames are
`Codec(UnsupportedVersion)`; `ReassemblyPoolV3` refuses v4 delta bytes the
same way. The host picks the pool per session; there is no fallback or
negotiation.

### T026p5: `replica_v4` and `outbox_v4`, the v4 client replica and sender

`replica_v4::ReplicaWorldV4` is the v3 client replica plus positions and
samples. It holds a `ReplicaWorldV3`, the disclosed positions
(`BTreeMap<NetId, PositionState { tick, point, moving }>`) and the newest
sample's clock (`SampleClock { server_tick, processed_seq }`). Every v3 query
is `v3()`. It reads no `World`, clock or I/O and invents no position: a
position comes only from a `Position` anchor or a sample.

- **`apply_delta`** checks, in v3's order, lane, op count, then `event_seq`
  (`ReplicaError`, reused unchanged), then applies the ops in frame order on a
  working copy of the whole replica and commits on success, so an `Err` leaves
  the replica equal to its pre-call clone. A `V3` op goes through
  `ReplicaWorldV3::apply_op` (the one `pub(crate)` token in `replica_v3.rs`);
  a v3 `EntityLeave` or `Despawned` also removes that entity's position. The
  sequence and tick advance by applying an empty v3 frame to the inner
  replica. `Position { Some }` needs a live row (`UnknownEntity` otherwise) and
  stores `{ frame.server_tick, point, moving }` unless the stored position is
  newer than the frame; `Position { None }` needs a live row and forgets the
  position. **`Moved` is an unchecked notice**, like `Joined`/`Left`
  (Reading 9): no liveness check, nothing stored; a presenter pairs its id with
  `v3().entity()` and treats a miss as "not visible".
- **`apply_snapshot`** is the inner `apply_snapshot`; on `Ok` the positions
  are emptied (the next page re-anchors them, Reading 8) and the sample clock
  is kept.
- **`apply_sample`** never fails: a datagram that is old, repeated or about an
  entity the replica does not know is ordinary traffic. The clock moves only
  to a strictly newer `server_tick`; an entry is stored only for an entity
  that already has a position and only when the sample is strictly newer than
  that position (L4, newest wins). It returns how many entries it stored and
  never touches `v3()`. The invariant is that every key of `positions()` is a
  live row of `v3()`.

`outbox_v4::MoveOutbox` is the client's lane-1 sender state for one epoch:
`next_seq` from 1, at most `POLICY_MOVE_IN_FLIGHT` (8) unreceipted intents,
`pending()` in ascending `seq` for a resend after a resume, and
`processed_seq`. `send` mints the intent (`Exhausted`, `Full`); `receive`
completes the oldest pending intent with its receipt, whatever the outcome,
and checks epoch, then emptiness, then `seq`, then actor (`WrongEpoch`,
`Unexpected { expected, got }`, `WrongActor { seq }`). `OutboxError` has no
payload in its `Display` and no `source()`. A refusal changes nothing.
`tests/v4_replica.rs` (R1..R9, O1..O4) is the suite; R2 runs the T140r R2
script on a `ReplicaWorldV3` and, wrapped in `V3`, on a `ReplicaWorldV4`, and
R9 drives a `ReassemblyPoolV4` into the replica.

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
- 2026-11-10 (UTC) · claude-code/claude-sonnet-5-5 + T140c · Extended the Protocol v3 section with `facts_v3` (`FactView`, `project_page`, the per-row, per-encounter and host-fact grants, the op-owner table and emission rules); no earlier section changed.

- 2026-11-10 (UTC) · claude-code/claude-sonnet-5-5 + T140d · Extended the Protocol v3 section with `transfer_v3` (the `0x82` frames, the v3 caps and `check_begin`, the body and its 2,847,788-byte bound, `plan_transfer`) and `project_snapshot_v3`; no earlier section changed.

- 2026-11-10 (UTC) · claude-code/claude-sonnet-5-5 + T140e · Extended the Protocol v3 section with `reassembly_v3` (the 128-chunk assembly, the re-sized limits, the worst-case memory and the transport note); no earlier section changed.

- 2026-11-13 (UTC) · opencode/deepseek-v4.1-flash-free + T140r1 · Added the `pool_v3` section (the reused and new types, the P-1 substitution table, the P8 mirror, version isolation and T140e's sizing), superseding T140e's "later task" sentence without editing it; no earlier section changed.
- 2026-11-13 (UTC) · opencode/deepseek-v4.1-flash-free + T140r2 · Added the `replica_v3` section (state, the R-5..R-8 op table, the liveness rule and its T140c emission-order/T027b history-slot justification, stale references, images, delta/snapshot precedence and host state kept across a snapshot); no earlier section changed.
- 2026-11-14 (UTC) · claude-code/claude-sonnet-5-5 + T026p1 · Added the "Protocol v4 (T026p, ADR-0036 L1–L6)" section for the new `protocol_v4`/`codec_v4` pair (lane table, `V3` wrapping and why, `Moved`, `Position`, `Point` and U3, `frame_spans`); no earlier section changed.
- 2026-11-14 (UTC) · claude-code/claude-sonnet-5-5 + T026p2 · Added the lane-1 `movement_v4` subsection (the `0xA1` subprotocol, the four messages and their layouts, the 1,024-byte and 38-entry sample bounds and splitter, the decode order and the U7 policies) and marked lane 1 built in the lane table.
- 2026-11-14 (UTC) · claude-code/claude-sonnet-5-5 + T026p3 · Added the `positions_v4`/`projection_v4` subsection (the anchor-then-samples disclosure, `PositionView`, the page, sample and snapshot rules, reconnect and `Moved` disclosure) and extended the op-owner table; no earlier section changed.
- 2026-11-14 (UTC) · claude-code/claude-sonnet-5-5 + T026p4 · Added the `pool_v4` subsection (what is re-exported from `pool_v3`, the V-1 substitution table and its runnable `diff -B` check, the two-pool mirror proof and version isolation); no earlier section changed.
- 2026-11-15 (UTC) · claude-code/claude-sonnet-5-5 + T026p5 · Added the `replica_v4`/`outbox_v4` subsection with the replica's apply rules, the sample and anchor ordering rule and the outbox window and receipt order.
- 2026-11-17 (UTC) · opencode-go/deepseek-v4.1-flash + T042a · Added the "Malicious-client suite (T042)" section (the eight-wire surface, the decode-stability rule and the spec-class index) and corrected the decode-order invariant to wire order, superseding the earlier semantic-checks-last wording by appending a dated note.
- 2026-11-20 (UTC) · opencode-go/deepseek-v4.1-flash + T042b-docs · Added the "Hostile-server suite (T042b)" section (the ten-decoder surface and its exclusions, the stability rule and a spec-class index of the four suites' tests) after the T042 section; no earlier section changed.
