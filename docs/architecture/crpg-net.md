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

## Agent log

- 2026-09-29 (UTC) · opencode/gpt-6-astra + T021 review fixes · Documented checked string-length overflow and transport-backed cache/retry conformance. These close malformed-length acceptance and replace codec-only redelivery evidence with actual dropped/duplicate delivery checks.

- 2026-09-28 (UTC) · opencode/muse-spark + T018a crate opening · Opened the doc with the lane-0 v1 protocol/codec/local-Transport shape, the frozen-wire and no-gameplay-execution boundaries, and the T018b/c plus evolution-queue plan, so later tasks extend rather than restart it.
- 2026-09-28 (UTC) · opencode/muse-spark + T018b simulated transport · Extended the scope and module flow with the injected-time fabric (rate → size → queue staging, seeded bounded faults, driver-owned seq/cache state) and the bytes-only fabric versus admission-drivers split, leaving T018c conformance and the evolution queue planned.
- 2026-09-28 (UTC) · opencode/muse-spark + T018c conformance · Extended the scope with the sim-backed driver, replica oracle, and four suites (rejection-without-mutation, filtered convergence with negative controls, 5,000-tick exercise, receipt liveness), closing the T018 proof with the evolution queue still planned.
- 2026-09-28 (UTC) · opencode/muse-spark + T018 review fixes · Recorded enforced failure-response budgets with wire drops, egress staging under v1_egress depths, reachable SeqExhausted precedence, and the bound driver peer table.
- 2026-09-29 (UTC) · opencode/muse-spark + T021 v2 event wire · Recorded the as-built explicit v2 modules (tagged history events beside frozen v1, bounded-before-allocation codec, host-fed ordered projection) and the six-case acceptance suite, so T022 consumes version-selected codecs plus ordered conformance evidence.
