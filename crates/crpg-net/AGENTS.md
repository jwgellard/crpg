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
§Snapshot transfer). This file keeps only how-to-work rules.

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
- *2026-10-25 (UTC), T024:* Segment length checked before body; never queue
  a delta above an in-flight cursor; never reset `event_seq` within an epoch;
  the full 1 MiB claim belongs to the byte layer (S§1.4); the T018b tracker
  test stays. DoD count: 119 passed (88 + 31; the other suites grew after the contract was written).

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
