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

## Design (moved to architecture doc)

Design, module flow, public surface, invariants, and allowed dependencies
live in `docs/architecture/crpg-net.md` (§Scope, §Module flow, §Public
surface, §Invariants, §Allowed dependencies, §Conformance, §v2 event wire,
§Snapshot transfer, §Resume and reconnect (T025a)). The resume modules
(`protocol_resume`, `codec_resume`, and `resume` in T025a2) and the T027b
`interest` module follow that pointer; so does the T046 `replica` module.
This file keeps only how-to-work rules.

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
  perception, or capability API lives here. `SessionFence` orders connection
  generations and authorises nothing: it checks epoch, fence order and shape,
  and the host's explicit `ResumeDecision` is the only input that produces
  `Accepted`.
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
- *2026-10-25 (UTC), T024:* Segment length checked before body; never queue
  a delta above an in-flight cursor; never reset `event_seq` within an epoch;
  the full 1 MiB claim belongs to the byte layer (S§1.4); the T018b tracker
  test stays. DoD count: 119 passed (88 + 31; the other suites grew after the contract was written).
- *2026-10-30 (UTC), T025a1:* The resume subprotocol (`0x91`, lane 3) is a
  separate codec beside v1/v2/`0x81`. Never put a fence on a lane-0 or
  transfer frame; never rotate the epoch on resume; never remove and re-add a
  retained receiver (use `rebind_receiver`, T025a2); never re-create a
  `SessionFence` for a live session and never add a restore constructor;
  `ResumeProof` never gains `PartialEq`, `Eq`, `Hash`, `Ord`, `Display` or a
  logging path (case 9's probe fails if it does); a resume decoder never
  falls back to another codec. DoD count: 175 passed (165 + 10), then 186
  (T025a2).
- *2026-11-02 (UTC), T027b:* **Interest projection is one view per session
  epoch.** Never add a view restore constructor, never build a second
  `InterestView` for one epoch, never store a retired-id/grave table
  (retirement is derived from `next_id`), and never add a `Spawned`/`Died`
  path to `InterestView` — notices stay in `project_events`, and a removal is
  exactly one `EntityLeave`/`Despawned`. DoD count: 191 passed (186 + 5
  `interest` unit tests, I15 among them).
- *2026-11-02 (UTC), T046:* **The replica applies, it never buffers or
  re-derives.** Never buffer or skip a frame, never add an in-band reset, a
  restore constructor or a `Serialize` path, never add a retired-id table
  (retirement is the `high_water` mark), never let a `Died` notice set `dead`,
  and keep the conformance `Replica` an independent oracle. A failed call
  leaves the replica equal to its pre-call clone; live rows may reach 2,048
  mid-page (D8). DoD count: 226 passed (the base suites plus 17 `replica`
  unit tests).
- *2026-11-10 (UTC), T140a:* **Protocol v3** (`protocol_v3`, `codec_v3`; see
  architecture §"Protocol v3"). A v3 tag is never reused or moved: tags
  11..=13 stay reserved for the fact ops (T140b) and are `UnknownMessage`
  until then. v3 never wraps v2 in `Legacy`: tags 0..=10 are native, because
  a receipt's code must reach 21..=30 and v1's `RejectionCode` is frozen at
  1..=20. `Clock` carries no tick; the frame header does, and `Clock` is the
  only carrier of pause state. A withheld value is `None` (a `00` flag),
  never a zero.
- *2026-11-10 (UTC), T140b:* **Fact ops** (tags 11..=13, `Encounter`,
  `Timeline`, `EntityFacts`; supersedes the T140a "reserved" note above).
  Never check timeline `NetId` order: the authority orders ties by
  `EntityId`, the client sees `NetId`s, so the codec checks only
  non-decreasing keys and distinct entities. A v3 page is not a frame: split
  with `codec_v3::frame_spans`, never by op count (1,024 full `EntityFacts`
  are about 2.8 MB; ops-only chunking builds frames `encode_delta` refuses).
  Over-limit families are withheld by the host, never truncated or refused
  here. The encode checks are the public `check_*` functions; keep them and
  the decoder in the same order.

- *2026-11-10 (UTC), T140c:* **Fact filter** (`facts_v3`; see architecture
  §"T140c"). `FactView` never mints: it resolves every `NetId` through the one
  `InterestView` of the epoch, and it is driven only through `project_page`
  (never mutate it, or the view, around that call). No controller input exists;
  the order grant is the host's (`fields.order` only for the controlling peer).
  Withheld is `None`, never a disclosed nothing: `Some(None)` means "disclosed,
  none", and an unchanged-looking image under a new `NetId` is still a new
  subject, so remembered rows are keyed by `(EntityKey, NetId)`. Build the two
  host ops only with `clock_op` / `host_facts_op`.

- *2026-11-10 (UTC), T140d:* **`0x82` snapshot** (`transfer_v3`; see
  architecture §"T140d"). `0x82` frames are `0x81` layouts with the v3 caps
  (4 MiB, 128 chunks): never fork the layout, and keep T024's checks in
  T024's order. Never call the `0x81` codec or `BeginTransfer::validate` for a
  v3 Begin or Chunk (they refuse the wider values): use `check_begin` and
  `chunk_len`. Never raise the `protocol` v1 caps for v3; they are frozen
  pins. Compute the chunk count in `usize` before narrowing to `u8` (a count
  that wraps can look valid). `encode_body` and `decode_body` check in the
  same order, so they agree on every refusal; keep them so, and never add a
  host-fact field to the body.

- *2026-11-10 (UTC), T140e:* **v3 receiver sizing** (`reassembly_v3`; see
  architecture §"T140e"). A v3 transfer is never sized by `TransferLimits` or
  assembled by `TransferAssembly`: both keep the v1 caps (1 MiB, 32 chunks,
  a `u32` bitmap) and refuse a 4 MiB Begin; use `TransferLimitsV3` and
  `TransferAssemblyV3`. The host paces `0x82` frames (H-7): the largest plan
  is 88 frames and 2,851,124 bytes, above the 2 MiB per-peer egress bytes, so
  the egress caps do not grow and a plan is never admitted whole. The v3
  limits are read by path from `protocol` and `transfer_v3`; never copy a
  number into them, and keep `validate` in field order, `InvalidLimits`
  before `LoosensPolicy`.

- *2026-11-13 (UTC), T140r1:* **v3 receiver gate** (`pool_v3`; see
  architecture §"T140r1"). `pool_v3` is a transliteration of
  `reassembly.rs`'s pool: a fix to one is a fix to both, in one task. Never
  add a V3 arm to `DeltaWire`, `GatedDelta` or `ReplicaUpdate`: the
  `crpg-server` tests match them exhaustively, and `Publication.body` is the
  `0x81` body. A v3 receiver never touches `codec_transfer`,
  `TransferAssembly` or `TransferLimits` (they refuse v3 sizes): use
  `transfer_v3`, `TransferAssemblyV3` and `TransferLimitsV3`.

- *2026-11-13 (UTC), T140r2:* **v3 client replica** (`replica_v3`; see
  architecture §"T140r2"). **Fact references are checked when the op
  applies, never re-checked later**: a stored order target, timeline entry or
  `turn.active` may name a removed id, and only a new op naming it is
  `UnknownEntity`. **`apply_snapshot` keeps `pause` and `host_facts`**: they
  are host state, are not in the `0x82` body, and the host re-sends them.
  **The replica re-runs no codec value check** (counts, `current ≤ max`,
  ordering, duplicates): those belong to the decoder. **Every v3 refusal is an
  existing `ReplicaError`: add no variant**, or the bridge's fatal-53 mapping
  and T046's `Display` strings change.

## Agent log

- 2026-09-29 (UTC) · opencode/gpt-6-astra + T021 review fixes · Added overflow-regression and transport-backed retry requirements. The checks prevent wrapped length acceptance and keep exactly-once evidence tied to admission and delivery paths.

- 2026-09-28 (UTC) · opencode/muse-spark + T018a crate opening · Wrote the crate contract for lane-0 v1 (exact surface, decode precedence, bound-before-alloc, NetId/seq discipline, allowed deps with the postcard approval pointer) plus the trap list (explicit u8 tags, no host invention, no sim-enum serialization, no queue drain) before T018b/T018c build on it.

- 2026-09-28 (UTC) · opencode/muse-spark + T018b simulated transport · Extended the surface with the sim fabric (clock, caps, buckets, peers, faults), the fabric-versus-drivers admission split with its check order and bounded-reorder rule, and the provisional-snapshot caveat, keeping no-new-deps and no host API.

- 2026-09-28 (UTC) · opencode/muse-spark + T018c conformance · Extended the contract with the test-only driver/oracle vocabulary, the admission and disclosure rules, and the dev-only fixture edges, closing the T018 proof without claiming host completion.

- 2026-09-28 (UTC) · opencode/muse-spark + T018 review fixes · Enforced the specified failure-response budgets on the wire path (drops retain outcomes for retry), pinned egress enforcement to fabric staging with v1_egress depths, made SeqExhausted reachable with cache-first precedence, and bound driver peer binding to the 8-peer proof population.

- 2026-09-29 (UTC) · opencode/muse-spark + T021 v2 event protocol · Added the explicit protocol_v2/codec_v2/projection_v2 surface with the version/tag/disclosure rules above (bounded-before-allocation strings, full-u64 delta event_seq, ordered host-fed projection) and the six-case events_v2 suite against real T020 history, keeping v1 frozen and adding no dependency.

- 2026-10-04 (UTC) · claude-code + T023 · Added a dated Known-traps pointer to `crpg-net-quic` so no one adds QUIC, I/O, threads or clocks to this crate; doc-only.

- 2026-10-25 (UTC) · opencode-go/beam-free + T024 · Added transfer subprotocol modules, byte-layer assembly, bounded reassembly pool and cutover gate; 31-case suite.

- 2026-10-25 (UTC) · opencode-go/beam-free + S21-bookkeeping · Updated DoD count to 119 passed (88 + 31).

- 2026-10-25 (UTC) · opencode-go/beam-free + T024-docs · Closed S§13 doc gaps: added transfer decode precedence and the S§10.7 cutover barrier to Invariants, traps, and ADR-0028 host requirements.

- 2026-10-25 (UTC) · opencode-go/beam-free + A0-8 · Trimmed AGENTS.md to how-to-work by moving Public surface, Invariants, and Allowed dependencies to the architecture doc; no rules changed.

- 2026-10-30 (UTC) · opencode-go/deepseek-v4-pro + T025a1 · Added the resume subprotocol modules (`protocol_resume`, `codec_resume`) to the architecture pointer, the SessionFence-orders-and-authorises-nothing note on the No-host-invention trap, and the T025a1 traps with the DoD count 175.

- 2026-11-01 (UTC) · opencode-go/deepseek-v4.1-flash + T025a2 · Landed the resume mechanisms (`resume`: `SessionFence`, `ResumeDecision`, `check_result`; additive `reassembly::rebind_receiver`) behind the same architecture pointer, with cases 11–21 in `reconnect_wire` and the DoD count 186. The existing T025a1 traps already state the fence/epoch/rebind/proof rules; this entry records that they are now enforced by code and tests.

- 2026-11-02 (UTC) · opencode-go/deepseek-v4.1-flash + T027b · Added the `interest` pointer and the per-epoch-view/derived-retirement/notice-ownership traps, with the DoD count 191 (five `interest` unit tests, I15 included).

- 2026-11-02 (UTC) · opencode-go/deepseek-v4.1-flash + T046 · Added the `replica` pointer and the apply-only/no-reset/no-grave/`Died`-is-a-notice traps, with the DoD count 226 (the base suites plus 17 `replica` unit tests).

- 2026-11-10 (UTC) · claude-code/opus-5.5 (manager) + T140a · Signed for the Sonnet implementer, who added the protocol v3 entry with its three traps (no tag reuse, no `Legacy` wrap of v2, `Clock` carries no tick), so agents extending v3 in T140b–e keep the wire stable.

- 2026-11-10 (UTC) · claude-code/claude-sonnet-5-5 + T140b · Added the fact-op entry with its two traps (never check timeline `NetId` order; a v3 page is not a frame, split with `frame_spans`), superseding the T140a note that tags 11..=13 are reserved.

- 2026-11-10 (UTC) · claude-code/claude-sonnet-5-5 + T140c · Added the fact-filter entry with its three traps (`FactView` never mints and is driven only through `project_page`; no controller input, the order grant is the host's; withheld is `None`, never a disclosed nothing).

- 2026-11-10 (UTC) · claude-code/claude-sonnet-5-5 + T140d · Added the `0x82` snapshot entry with its traps (v3 Begin/Chunk go through `check_begin`, never the `0x81` codec or `validate`; the v1 caps stay frozen; the chunk count is computed in `usize`; encode and decode check in one order).

- 2026-11-10 (UTC) · claude-code/claude-sonnet-5-5 + T140e · Added the v3 receiver-sizing entry with its two traps (a v3 transfer is never sized by `TransferLimits` or assembled by `TransferAssembly`; the host paces `0x82` frames and the egress caps do not grow).

- 2026-11-13 (UTC) · opencode/deepseek-v4.1-flash-free + T140r1 · Added the v3 receiver-gate entry with its three traps (one transliteration fixed in both pools in one task; no V3 arm on the frozen v1/v2 types; a v3 receiver uses only the v3 codecs, assembly and limits). P9 remains unwritten because the contract's `G(1)` expectation contradicts P-5 step 4; reported to the manager.
- 2026-11-13 (UTC) · opencode/deepseek-v4.1-flash-free + T140r2 · Added the v3 client-replica entry with its four traps (fact references checked only when the op applies; `apply_snapshot` keeps `pause` and `host_facts`; no codec value check is re-run; every v3 refusal is an existing `ReplicaError`, add no variant).
