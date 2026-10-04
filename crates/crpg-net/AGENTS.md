# crpg-net — agent contract

Read the root rules and [T018a](../../tasks/T018a.md) (exact public shapes,
frozen codes, wire hard maxima, operational policy defaults, codec error
precedence, `NetId` discipline) plus parent [T018](../../tasks/T018.md) and
[E017 Appendices A–B](../../tasks/E017-t018-interface-debt.md) (durable wire
versus provisional scope, per-lane seq, two-tier limits, filtered
projections, evolution queue). Architecture:
[crpg-net](../../docs/architecture/crpg-net.md). This document describes the
T018a lane-0 protocol v1 plus bounded `postcard` codec plus local
`Transport`, the T018b simulated transport, and the T018c conformance gate.

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
- `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]`. No `HashMap`/`HashSet`
  (use `BTreeMap`/`IndexMap`); no `f32`/`f64` in policy paths (no spatial
  wire in v1 lane 0); no `SystemTime`/threads/I-O/unseeded RNG
  (`DeterministicRng` only if needed; the codec path needs none).

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

## Definition of done for any change

```
cargo fmt --all
cargo clippy -p crpg-net --all-targets -- -D warnings
cargo test -p crpg-net --locked
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
cargo deny check
git diff --check
```

Any `proptest-regressions/` file a failure produces is **committed**, not
ignored: it is the shrunk counterexample, and losing it loses the regression.

## Known traps

- **Discriminants are explicit `u8` matches, not derived serde tags.**
  `postcard` encodes enum variant indices, which for `RejectionCode` would be
  `0..=19` instead of the frozen `1..=20`. The codec matches raw tag bytes,
  so `UnknownMessage` is exact and never inferred from a serde failure (every
  `postcard` decode error maps to `Malformed`).
- **No host invention.** Host-policy checks appear only as `POLICY_*`
  documented constants for T018b/c drivers. No auth, placement, lifecycle,
  perception, or capability API lives here.
- **No open bag, no transform op, no sim-enum serialization.** Undisclosed
  fields are omitted, never zero-filled. Spatial replication waits for the
  lane-1 movement channel.
- **`legal_actions` is not manufactured; `perform_action` precedence stays
  authoritative in sim.** No gameplay validation here beyond shape.
- **Never drain the authoritative queue.** Drivers observe a detached clone
  with a cursor for projection tests only; production retention/drain is C1
  work, not a codec concern.
- **Keep whole `EntityId`s internally.** Never serialize raw arena ids, never
  reuse sim queue gaps as client `seq`, preserve relative `(Tick, sim_seq)`
  order when assigning `event_seq`.
- **ULID parse leniency is inherited from core.** Display emits canonical
  26-character uppercase; parsing accepts case aliases. The length gate fires
  before parsing, so over-long text is `LimitExceeded`, not `Malformed`.
  v2 re-encodes canonically after accepting aliases.
- **Receipt `lane` is checked.** A receipt echoing a foreign lane is
  `WrongDirection`, matching the intent/frame lane rule.
- **Reorder must be provably bounded.** A forward windowed swap lets a
  datagram surf right without bound; the fabric rotates consecutive blocks
  of `reorder_depth + 1` instead, and the test pins displacement against the
  depth. Do not "simplify" it back to swaps.
- **Token buckets are milli-token exact.** Stepwise advances credit exactly
  what one jump does; backwards timestamps accrue nothing. Do not replace
  with float arithmetic.
- **Unknown `send_to` peers fail, unknown `recv_from` peers return `None`.**
  Sending needs an authenticated binding (`Unauthenticated`); receiving from
  nothing is ordinary absence, not an error.
- **Exhaustion beats staleness.** Once `next_new_seq` passes `SEQ_LAST`,
  non-cached admits report `SeqExhausted`, never `SeqGap`/`StaleSeq`: the
  client must rotate epochs, not retry. Cached retries still return retained
  outcomes so in-flight receipts recover.
- **Dropped failure responses are not lost outcomes.** An empty wire reply
  with a retained terminal status means the failure budget (not the command)
  ran out; the sender retries the same bytes after the injected clock
  advances. Applied receipts never spend failure budgets.
- **A passing double is not host integration passing.** Supplied tables are
  fixtures, not perception; E012/E018/E022 own the real host. Never drain
  the authoritative queue from a driver (detached clone plus cursor only).
- **No replay-format, hash-exclusion, golden, or target-selection change.**
  Rejected inputs leave complete `World`/RNG/event state byte-identical.
- *2026-10-04 (UTC), T023:* **Real QUIC lives in `crpg-net-quic`**
  (ADR-0024); never add I/O, threads, clocks or QUIC crates here.

## Agent log

- 2026-09-29 (UTC) · opencode/gpt-6-astra + T021 review fixes · Added overflow-regression and transport-backed retry requirements. The checks prevent wrapped length acceptance and keep exactly-once evidence tied to admission and delivery paths.

- 2026-09-28 (UTC) · opencode/muse-spark + T018a crate opening · Wrote the crate contract for lane-0 v1 (exact surface, decode precedence, bound-before-alloc, NetId/seq discipline, allowed deps with the postcard approval pointer) plus the trap list (explicit u8 tags, no host invention, no sim-enum serialization, no queue drain) before T018b/T018c build on it.
- 2026-09-28 (UTC) · opencode/muse-spark + T018b simulated transport · Extended the surface with the sim fabric (clock, caps, buckets, peers, faults), the fabric-versus-drivers admission split with its check order and bounded-reorder rule, and the provisional-snapshot caveat, keeping no-new-deps and no host API.
- 2026-09-28 (UTC) · opencode/muse-spark + T018c conformance · Extended the contract with the test-only driver/oracle vocabulary, the admission and disclosure rules, and the dev-only fixture edges, closing the T018 proof without claiming host completion.
- 2026-09-28 (UTC) · opencode/muse-spark + T018 review fixes · Enforced the specified failure-response budgets on the wire path (drops retain outcomes for retry), pinned egress enforcement to fabric staging with v1_egress depths, made SeqExhausted reachable with cache-first precedence, and bound driver peer binding to the 8-peer proof population.
- 2026-09-29 (UTC) · opencode/muse-spark + T021 v2 event protocol · Added the explicit protocol_v2/codec_v2/projection_v2 surface with the version/tag/disclosure rules above (bounded-before-allocation strings, full-u64 delta event_seq, ordered host-fed projection) and the six-case events_v2 suite against real T020 history, keeping v1 frozen and adding no dependency.
- 2026-10-04 (UTC) · claude-code + T023 · Added a dated Known-traps pointer to `crpg-net-quic` so no one adds QUIC, I/O, threads or clocks to this crate; doc-only.
