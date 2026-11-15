# T150-draft-b — lane-1 movement transport (`crpg-net-quic` + `crpg-server` route)

**Status: draft, blind (slot 2, session 60). Nothing here is approved.**
Draft of the exact contract for T150, from the approved T026p contract
(`tasks/T026p.md:1471–1476`), the approved T037 contract
(`tasks/T037.md:1351–1355`), ADR-0036, ADR-0024, ADR-0025, ADR-0026 and
`tasks/DECISIONS-2026-09-30.md:83–88`, built on `tasks/T150-facts.md`.
A Claude architect writes the final contract; the user approves it.

## Task

Carry lane 1 over QUIC. `crpg-net-quic` gains a second reliable ordered
stream pair for lane-1 intents upstream (`MoveTo`/`Halt`) and receipts
downstream, the QUIC DATAGRAM extension for `MoveSample`s with a negotiated
floor of `MAX_MOVE_DOWN_BYTES` (1,024, refused at connect otherwise), D23's
vendored dedup patch through the T023v mechanism, and the native Windows
impairment-probe re-run as evidence. The `crpg-server` `quic.rs` route binds
v4 sessions and pumps lane-1 bytes through the T037 surface it calls:
`bind_v4_peer`, `ingest_move`, `take_move_receipts`, `take_samples`
(`tasks/T037.md:437–441`, `:483–484`, `:487`, `:462`).

Built after T037d, so the T037 Interface is the host surface it calls;
`quic.rs` is otherwise frozen (`tasks/T037.md:1338`, `:1351–1355`). One task
= one crate per part (`AGENTS.md:57`; the scope names two crates,
`tasks/T037.md:1351–1355`), so three serial parts: **T150a** the vendored
patch (S-set), **T150b** the transport (not S-set), **T150c** the server
route (not S-set). S-set here is `docs/IMPLEMENTATION_PLAN.md:1.4`'s
serialized set: a vendored dependency patch, `deny.toml` and the workspace
`Cargo.toml` are S-set; nothing else in this draft is.

## Parts

Order is serial: **T150a → T150b → T150c**. Each part is one crate and one
implementer run. T150b needs T150a (patched vendored code) and the merged
T026p/T037d surfaces; T150c needs T150b and T037d.

| Part | Crate | Scope | S-set | Review |
|---|---|---|---|---|
| T150a | vendored `quinn-proto` | apply D23 dedup patch | **yes** (vendored dependency patch) | reviewer checks `--dry-run` stacking + `VENDOR.md` |
| T150b | `crpg-net-quic` | lane-1 stream pair + DATAGRAMs + Windows probe | no | Claude reviewer, one fix round |
| T150c | `crpg-server` | `quic.rs` lane-1 route | no | same |

### T150a owns

- `third_party/quinn-proto/patches/0002-dedup-window.patch` (new: the
  `docs/reviews/T030-quinn/dedup-window.patch:1–31` content, retargeted at
  `third_party/quinn-proto/crate`; touches only
  `src/connection/spaces.rs`, replacing `type Window = u128` with
  `[u64; 32]`, `WINDOW_WORDS = 32`, `WINDOW_BITS = 2048`,
  `WINDOW_SIZE = 1 + WINDOW_BITS`, i.e. effective window 2049, +696 bytes
  per connection, no heap allocation,
  `docs/reviews/T030-quinn/README.md:14–17`, `:77–92`)
- `third_party/quinn-proto/VENDOR.md` (one patch-table row; the table rule
  is `third_party/quinn-proto/VENDOR.md:26`, the numbering rule `:67–70`;
  `crate/` stays hand-untouched, `:3–9`)
- `docs/reviews/T030-quinn/results/` (new Windows matrix files only)
- goldens: none new, none moved.

### T150b owns

- `crates/crpg-net-quic/src/lane1.rs` (new: `ALPN_LANE1_V1`, floor
  constant, lane-1 stream + datagram API)
- `crates/crpg-net-quic/src/handshake.rs` (ALPN constant beside
  `ALPN_LANE0_V1`/`ALPN_LANE0_V2`, `:12`, `:16`), `src/tls.rs`
  (transport parameters: 2 client-initiated bidi streams, DATAGRAM
  extension on; today 1/0/off, `:86–91`, `:129–134`),
  `src/limits.rs` (datagram floor field, tightening-only), `src/queue.rs`
  (second-stream reader/writer tasks beside `:800`, `:876`; datagram
  accounting), `src/server.rs`, `src/client.rs`, `src/lib.rs` (re-export
  beside `:40–53`)
- `crates/crpg-net-quic/tests/lane1.rs` (new),
  `crates/crpg-net-quic/tests/support/quic_rig.rs` (helpers appended)
- `docs/architecture/crpg-net-quic.md`,
  `crates/crpg-net-quic/AGENTS.md`
- goldens: none new, none moved.

### T150c owns

- `crates/crpg-server/src/quic.rs` (the lane-1 route only: v4-session
  binding on the lane-1 ALPN, lane-1 drain/deliver phases; every lane-0,
  transfer and resume line frozen)
- `crates/crpg-server/tests/host_quic_lane1.rs` (new)
- `docs/architecture/crpg-server.md`, `crates/crpg-server/AGENTS.md`
- goldens: none new, none moved.

## Readings

Design decisions this draft takes. Each needs approval with the contract.

1. **A new ALPN for lane 1: `crpg-lane1/1`.** ADR-0025 pins ALPN to the
   framing, hello/welcome layout, stream mapping and close codes
   (`docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:31–36`), and its
   consequence row says datagrams each add their own ALPN (`:132–134`; the
   addendum keeps lane 0 at no DATAGRAM, `:174–176`). `crpg-lane0/2` is
   taken by snapshot sessions (ADR-0026 addendum,
   `docs/adr/0026-quic-host-adapter.md:161–179`). *Rejected:* reusing the
   lane-0 ALPN (a lane-0-only peer would then have to carry datagram
   state); bumping the lane-0 ALPN for everyone (forces a flag day on the
   30-case lane-0 suite for no lane-0 change).
2. **One second client-initiated bidi stream carries intents up and
   receipts down.** It preserves the single total order per direction the
   receipt logic relies on (the same reason ADR-0025 keeps one stream per
   direction, `docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:115–120`).
   *Rejected:* two uni
   streams (doubles the close paths in `queue.rs:928` for no ordering
   gain); sharing the lane-0 stream (breaks lane-1 isolation, HM-10,
   `tasks/T037.md:299–301`).
3. **The datagram floor is negotiated, never downgraded.** The endpoint
   advertises `max_datagram_frame_size >= 1,024` or the connect is refused;
   the codec guarantees every sample fits in 1,012 bytes
   (`tasks/T026p.md:607–609`). *Rejected:* clamping or fragmenting
   (samples must stay self-contained datagrams, L4,
   `docs/adr/0036-movement-and-navigation.md:402–407`); downgrading
   (the host selects one version and refuses others,
   `docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:31–36`).
4. **Refusal for a missed floor closes with `VersionRefused` (6).** Close
   codes 0–9 are fixed (`docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:70–85`)
   and a new code needs its own ALPN
   (`docs/adr/0026-quic-host-adapter.md:127–128`). *Rejected:* a new code
   10+ (registry churn for a condition the ALPN already versions);
   `AuthRefused` (5) (lies about credentials). Open question Q1 confirms.
5. **The lane-1 stream reuses `[len][payload]` framing, cap 4,096.** It
   reuses `frame_header`/`parse_frame_header`
   (`crates/crpg-net-quic/src/handshake.rs:43–55`) and the
   reader/writer/control shape (`crates/crpg-net-quic/src/queue.rs:800`,
   `:876`, `:928`); the intent decoder refuses over-cap bytes anyway
   (`crates/crpg-net/src/movement_v4.rs:228–229`). *Rejected:* a new
   2-byte header (a second framing to pin under R1's ALPN for 54-byte
   intents at most).
6. **The transport keeps no sample cache and reads no sample.** QUIC
   enforces the advertised max; late or repeated datagrams pass through
   and the replica keeps the newest per entity (L4,
   `docs/adr/0036-movement-and-navigation.md:402–407`;
   `apply_sample` never fails, `tasks/T026p.md:544–554`). Packet-level duplicates die in the D23 window inside
   quinn-proto. *Rejected:* transport-side dedup or newest-wins (duplicates
   `project_samples`/`MoveOutbox` rules in the wrong layer).
7. **Backpressure: the lane-1 stream shares the connection's `QueueCaps`;
   samples never block it.** `QueueFull` stays retryable with nothing
   queued (the bounds table,
   `docs/architecture/crpg-net-quic.md:99–119`); a tick's samples that do
   not fit the session's egress room are dropped whole, which the refresh
   repairs (`tasks/T037.md:303–322`, C-7 `:701–707`). *Rejected:* blocking
   datagram sends (unreliable traffic must not head-block the reliable
   streams); an unbounded sample queue (a stalled adapter would hold bytes
   outside every cap, the case Reading 23 exists to close).
8. **Close order: lane-1 stream first, then lane-0, then
   `CONNECTION_CLOSE`, inside the same budget `B` with reserve
   `R = min(200 ms, B / 4)`** (`crates/crpg-net-quic/src/teardown.rs:9–18`).
   Untaken receipts/samples die with the session (D-10,
   `tasks/T037.md:792–795`); a resent intent re-queues its receipt via
   `Duplicate { resent: true }` (`:758–764`), and samples refresh every 20
   ticks (`:284–292`). *Rejected:* draining lane 1 before closing (a peer
   that will not read stalls the close; `teardown.rs:26` bounds the wait
   regardless).
9. **The probe re-run is T150b's acceptance evidence, not a fourth part.**
   D23 ties patch + probe to the first datagram lane
   (`tasks/DECISIONS-2026-09-30.md:83–88`); results land under
   `docs/reviews/T030-quinn/results/`. *Rejected:* a codeless evidence-only
   part (one task = one crate leaves it nowhere to own).
10. **Three parts, not one.** The T037 out-of-scope item names both the
    transport and the `quic.rs` route (`tasks/T037.md:1351–1355`), and one
    task = one crate (`AGENTS.md:57`) forces the split. *Rejected:* a
    single two-crate task (the brief's stop-and-report rule).

## Interface

TODO (fill)

## Behaviour

TODO (fill)

## Errors

TODO (fill)

## Tests

TODO (fill)

## What must not change

TODO (fill)

## Dependency edges

TODO (fill)

## Out of scope

TODO (fill)

## Open questions (each with a recommendation)

TODO (fill)

## Agent log

- 2026-11-15 (UTC) · opencode-go/muse-spark-1.3-contributor + T150 draft b · Opened the blind-draft skeleton with every contract section so each section can be filled and committed separately.
