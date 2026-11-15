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

Additions only, all proposed (the final architect pins them). Every public
item carries a doc comment (`#![warn(missing_docs)]` holds crate-wide). No
`HashMap`/`HashSet`, no `f32`/`f64`, no `unsafe`, no tokio/quinn/rustls
type in a public signature (the crate rule,
`crates/crpg-net-quic/AGENTS.md:47–53`), no new dependency.

### T150a — no API

The patch changes only `third_party/quinn-proto/crate/src/connection/spaces.rs`
(`type Window`, `WINDOW_WORDS`, `WINDOW_BITS`, `WINDOW_SIZE`, `insert`,
`smallest_missing_in_interval`, three unit tests rewritten
width-generically, `docs/reviews/T030-quinn/README.md:77–92`). No public
signature anywhere changes; the workspace `[patch.crates-io]` entry
(`Cargo.toml:33–42`) already routes `quinn-proto` at the vendored path.

### T150b — `lane1.rs` (new, `crpg-net-quic`)

```rust
/// ALPN for a connection carrying lane 1 (Reading 1). Lane-0-only peers
/// keep `ALPN_LANE0_V1` (`crates/crpg-net-quic/src/handshake.rs:12`).
pub const ALPN_LANE1_V1: &str = "crpg-lane1/1";
/// Refused at connect unless the peer advertises at least this
/// `max_datagram_frame_size` (Reading 3): `MAX_MOVE_DOWN_BYTES`
/// (`crates/crpg-net/src/movement_v4.rs:50`).
pub const LANE1_DATAGRAM_FLOOR: usize = 1_024;
/// Caps for the lane-1 reliable stream (Reading 5): one maximal intent
/// (54 bytes, `tasks/T026p.md:607–609`) plus the 4-byte header, rounded to
/// the lane-0 intent cap.
pub const LANE1_STREAM_READ_CAP: usize = 4_096;
pub const LANE1_STREAM_SEND_CAP: usize = 4_096;

/// What `QuicHost` (T150c) pumps on a lane-1 connection. Lane identity is
/// the stream, so no new `Channel` arm: lane-1 bytes are `movement_v4`
/// messages, never lane-0 frames.
impl QuicServer {
    /// Sends one lane-1 stream frame (an encoded intent upstream or receipt
    /// downstream); `QueueFull` is retryable and queues nothing.
    pub fn try_send_lane1(&self, conn: ConnectionId, bytes: &[u8]) -> Result<(), ChannelSendError>;
    /// Receives one lane-1 stream frame, in order, exactly once.
    pub fn try_recv_lane1(&self, conn: ConnectionId) -> Result<Vec<u8>, ServerError>;
    /// Sends one sample datagram (at most `LANE1_DATAGRAM_FLOOR` bytes);
    /// `QueueFull` is retryable and queues nothing, never blocking the
    /// reliable streams.
    pub fn try_send_sample(&self, conn: ConnectionId, bytes: &[u8]) -> Result<(), ChannelSendError>;
    /// Receives one sample datagram, if one arrived.
    pub fn try_recv_sample(&self, conn: ConnectionId) -> Result<Vec<u8>, ServerError>;
}
impl QuicClient {
    pub fn try_send_lane1(&self, bytes: &[u8]) -> Result<(), ChannelSendError>;
    pub fn try_recv_lane1(&self) -> Result<Vec<u8>, ServerError>;
    pub fn try_send_sample(&self, bytes: &[u8]) -> Result<(), ChannelSendError>;
    pub fn try_recv_sample(&self) -> Result<Vec<u8>, ServerError>;
}
```

`ConnectionId` (`crates/crpg-net-quic/src/server.rs:37`),
`ChannelSendError` (`crates/crpg-net-quic/src/close.rs:141`) and
`ServerError` (`crates/crpg-net-quic/src/server.rs:106`) are reused; no
new error enum. `ServerLimits`/`ClientLimits`
(`crates/crpg-net-quic/src/limits.rs:53`, `:174`) each gain one
tightening-only field `max_datagram_bytes: u64` (default: the floor;
`validate`, `:105`, `:225`, rejects below it as `InvalidLimits`), and
`tls.rs` advertises it as `max_datagram_frame_size` (today unadvertised,
`:86–91`, `:129–134`).

### T150c — `quic.rs` route (no new `Host` API)

T150c calls only the approved T037 surface, unchanged:

```rust
// host.rs (T037d/T037b/T037c; called, not changed)
pub fn bind_v4_peer(&mut self, control: ControlGrant, grants: DisclosureGrants,
    facts: FactGrants, rights: crpg_net::protocol_v3::PauseRights, now_ms: u64)
    -> Result<PeerHandle, HostError>;                                   // tasks/T037.md:437–441
pub fn ingest_move(&mut self, peer: PeerHandle, bytes: &[u8], now_ms: u64)
    -> Result<MoveAdmission, HostError>;                               // tasks/T037.md:483–484
pub fn take_move_receipts(&mut self, peer: PeerHandle) -> Result<Vec<Vec<u8>>, HostError>;
                                                                       // tasks/T037.md:487
pub fn take_samples(&mut self, peer: PeerHandle) -> Result<Vec<Vec<u8>>, HostError>;
                                                                       // tasks/T037.md:462
```

`quic.rs` gains a private `Lane1Binding { peer: PeerHandle }` per
connection plus two private pump phases (drain lane-1 stream frames via
`try_recv_channel`-analogous `try_recv_lane1` into `ingest_move`;
deliver `take_move_receipts` via `try_send_lane1` and `take_samples`
via `try_send_sample`). No `Host` signature, variant, `Display` text or
cap changes.

## Behaviour

Normative. Each item names the tests that prove it. "Refused" means the
stated close or error and no state change. **First failure wins** in every
list. B-1–B-8 are T150b, B-9 is T150a + T150b, B-10 is T150c.

- **B-1. Lane-1 stream open order.** After `accept` writes the welcome
  (`crates/crpg-server/src/quic.rs:687–694`), the client opens exactly one
  additional client-initiated bidi stream; the server accepts exactly one
  (`accept_bi` loop, `crates/crpg-net-quic/src/server.rs:936`). Lane-0
  bytes never arrive on it and lane-1 bytes never arrive on the lane-0
  stream. A second lane-1 open is closed with `ProtocolViolation` (2).
  *Tests:* L1, L2.
- **B-2. Datagram size negotiation and refusal at connect.** Both ends
  advertise `max_datagram_frame_size`; if either side's value is below
  `LANE1_DATAGRAM_FLOOR` (1,024), the connection closes with
  `VersionRefused` (6) before any hello is read: no `HelloReceived`
  event, no binding, `refused_handshakes` + 1, mirroring the
  over-cap/hello-timeout path
  (`docs/architecture/crpg-net-quic.md:165–177`). A floor of exactly
  1,024 connects. *Tests:* L3, L4.
- **B-3. Upstream order.** Lane-1 stream bytes arrive in order, exactly
  once, while the connection lives — the same guarantee lane 0 gets from
  QUIC (ADR-0026 item 5, `docs/adr/0026-quic-host-adapter.md:52–55`). The
  transport never decodes intent payloads (cf. ADR-0025 item 3,
  `docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:37–43): a
  54-byte intent-shaped payload is opaque bytes. *Tests:* L5.
- **B-4. Samples that do not fit never cross.** The codec emits at most
  1,012 bytes per datagram (24 + 38 × 26, `tasks/T026p.md:607–609`); a
  send of more than the advertised max fails at the API and nothing is
  queued; a received datagram over the cap is impossible by QUIC framing.
  *Tests:* L6.
- **B-5. Late, repeated or reordered samples pass through; D23 absorbs
  packet duplicates.** Application-level repeats are delivered (the
  replica keeps the newest per entity, L4); QUIC-level duplicates of one
  packet are absorbed by the widened window (effective 2049,
  `docs/reviews/T030-quinn/README.md:14–15`) instead of dropping
  genuinely new packets (the 129-packet bug,
  `docs/adr/0004-quic-movement-spike.md:62–74`). *Tests:* L7.
- **B-6. Backpressure and the lane-0 limits.** The lane-1 stream shares
  the connection's `QueueCaps` (128 frames / 2 MiB host-wide scale,
  `docs/architecture/crpg-net-quic.md:99–119`); `QueueFull` is retryable
  and queues nothing. Datagram sends never stall stream sends in either
  direction. No existing limit value moves, and the lane-0 suite's window
  cases (T023c C§1.2–C§1.3, `docs/architecture/crpg-net-quic.md:157–159`)
  pass unchanged. *Tests:* L8, every existing suite.
- **B-7. Lane-0 bytes are untouched.** Framing, hello/welcome layout,
  stream mapping, close codes and ALPN `crpg-lane0/1` behave byte for
  byte as ADR-0025 items 1–7
  (`docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:25–84`); a
  lane-0-only connection advertises no DATAGRAM extension (addendum,
  `:174–176`). *Tests:* every existing suite.
- **B-8. Close and teardown with lane 1 open.** A `Flush` close flushes
  the lane-1 stream first, then lane-0, and still delivers
  `CONNECTION_CLOSE` inside `B`: stops flushing at `B − R`,
  `R = min(200 ms, B / 4)`
  (`crates/crpg-net-quic/src/teardown.rs:9–18`). At `B = 2,000 ms`,
  `R = 200 ms`. `shutdown`/`Drop` close every connection with
  `Shutdown` (`crates/crpg-net-quic/src/server.rs:703`,
  `src/client.rs:451`). *Tests:* L9.
- **B-9. T150a patch behaviour.** The vendored `Window` is `[u64; 32]`
  with `WINDOW_BITS = 2048` (`docs/reviews/T030-quinn/dedup-window.patch:1–31`);
  `Dedup` grows 32 → 264 bytes (+696 per connection, no heap,
  `docs/reviews/T030-quinn/README.md:17`); the three rewritten unit tests
  pass; the patch applies with `--dry-run` on top of `0001`
  (`tasks/T023v.md:146`). *Tests:* P1.
- **B-10. The `quic.rs` route.** A connection on the lane-1 ALPN binds by
  `bind_v4_peer` (`tasks/T037.md:437–441`); every lane-1 stream frame is
  offered to `ingest_move(peer, bytes, now_ms)` in arrival order with the
  pump's injected `now_ms` (module rule,
  `crates/crpg-server/src/quic.rs:21–26`); `take_move_receipts` bytes
  leave on the lane-1 stream in release order; `take_samples` bytes leave
  as datagrams; a closed connection drains received frames first, then
  unbinds (drain-then-unbind, ADR-0026 items 9–10,
  `docs/adr/0026-quic-host-adapter.md:73–80`). *Tests:* S1, S2.

## Errors

No new close code and no new rejection code (R4; `tasks/T026p.md:185–193`).
The connect-time floor refusal surfaces as `VersionRefused` (6) on the
wire; every API error reuses the existing enums
(`crates/crpg-net-quic/src/server.rs:106`,
`src/client.rs:43`, `src/close.rs:108`, `:141`).

| Condition | Result |
|---|---|
| peer `max_datagram_frame_size` < 1,024 at connect | close `VersionRefused` (6), no event, no binding |
| second lane-1 stream opened on one connection | close `ProtocolViolation` (2) |
| lane-1 stream frame over 4,096 bytes | close `FrameTooLarge` (3), no body read (R5) |
| `try_send_lane1` / `try_send_sample` with a full queue | `QueueFull`, retryable, nothing queued |
| datagram send over the advertised max | API refusal, nothing queued |
| `ingest_move` / receipts / samples host refusals | exactly T037's table (`tasks/T037.md:828–841`) |
| lane-0 violations | unchanged codes 0–9 paths |

`Display`/`source()` of every existing variant are unchanged. T150c adds
no `HostError` variant.

## Tests

`crates/crpg-net-quic/tests/lane1.rs` (T150b), `host_quic_lane1.rs`
(T150c); T150a is proven by P1 plus the matrix L10. Where a case names no
separate control, its passing calls are the control. Each precedence list
is tested at every adjacent pair with both faults present.

- **P1 `vendored_window_is_2049`.** Applies
  `patches/0002-dedup-window.patch` with `--dry-run` on top of `0001`
  (control: `0001` alone still applies clean, `tasks/T023v.md:146`);
  asserts the crate's `WINDOW_BITS == 2048` and `WINDOW_SIZE == 2049`
  (was 129, `third_party/quinn-proto/crate/src/connection/spaces.rs:480–483`);
  runs the three rewritten unit tests. *Fails if:* the patch overlaps
  `0001`, or any constant differs.
- **L1 `lane1_stream_pair_opens_after_welcome`.** Full connect on
  `ALPN_LANE1_V1`; client opens the second bidi stream; server accepts
  exactly one. Sends a 54-byte intent-shaped payload (the largest lane-1
  intent, `tasks/T026p.md:607–609`) each way; expects exact bytes back in
  order. Control: the lane-0 hello/welcome round trip on the same
  connection first. *Fails if:* a second stream is needed, a byte is
  reordered, or lane-0 bytes leak across streams.
- **L2 `second_lane1_stream_is_refused`.** Control L1; client opens a
  second lane-1 stream; expects close `ProtocolViolation` (2).
  *Fails if:* the server accepts it or closes with another code.
- **L3 `datagram_floor_refused_at_connect`.** Raw client (the
  `RawClient`/`raw_connect` shape,
  `crates/crpg-net-quic/tests/support/quic_rig.rs:212–353`) advertising
  `max_datagram_frame_size = 512` on the lane-1 ALPN: expects close
  `VersionRefused` (6), no `HelloReceived`, `refused_handshakes` + 1.
  Control: the same client at 1,024 connects and is greeted. *Fails if:*
  512 connects, or the refusal carries another code.
- **L4 `datagram_floor_boundary`.** 1,023 refused exactly as L3; 1,024
  accepted exactly as the control. *Fails if:* either side flips.
- **L5 `lane1_stream_ordered_exactly_once_through_impaired_relay`.**
  Handshake clean, then `set_impaired(true)` (the T098d shape,
  `crates/crpg-net-quic/tests/quic.rs:1200–1212`); relay config
  `drop_every = 13`, `reorder_depth = 4`, `seed = 23` (`:1174–1178`).
  Sends 40 × 54-byte frames on the lane-1 stream; expects all 40 in
  order, exactly once. Control: the same 40 on a clean path.
  *Fails if:* any frame is lost, duplicated or reordered.
- **L6 `oversize_sample_never_crosses`.** `try_send_sample` of 1,012
  bytes (24 + 38 × 26, the largest sample, `tasks/T026p.md:607–609`)
  succeeds; of 1,025 bytes fails at the API with nothing queued.
  A 1,012-byte payload arrives whole. Control: a 0-byte... (no
  empty-datagram special case: 0-length stream frames are violations, so
  the control is the 1,012-byte success). *Fails if:* 1,025 is queued or
  1,012 is truncated.
- **L7 `duplicates_pass_through_dedup_holds`.** Sends one datagram 3
  times plus an older-tick-shaped datagram: expects 4 deliveries in send
  order at the receiver (application repeats are data; QUIC packet
  duplicates die in the 2,049 window). Control: reorder depth ≤ 29 as
  measured (`docs/reviews/T030-quinn/README.md:18`), far below both the
  old 129 and the new 2,049. *Fails if:* fewer than 4 arrive (dedup
  overreach) or order changes.
- **L8 `sample_pressure_never_blocks_receipts`.** Fills the outbound
  sample accounting to its cap; `try_send_lane1` of a 60-byte
  receipt-shaped payload (the largest receipt,
  `tasks/T037.md:459`) still succeeds immediately. Control: the same
  send on an idle connection. *Fails if:* the stream send blocks or
  fails while only datagram accounting is full.
- **L9 `close_with_lane1_open_flushes_both`.** Lane 1 open with a queued
  receipt and a queued sample; `close(Flush)` with `B = 2,000 ms`:
  expects both payloads delivered, then `CONNECTION_CLOSE`; the flush
  stops at `B − R = 1,800 ms` with `R = min(200, 2,000 / 4) = 200 ms`
  (`crates/crpg-net-quic/src/teardown.rs:9–18`). Control: the same close
  with lane 1 idle. *Fails if:* either payload is missing, the close
  code is wrong, or the flush overruns into the reserve.
- **L10 `windows_impairment_matrix_passes` (T150b acceptance, native
  Windows/MSVC).** Re-runs the T030 matrix
  (`docs/reviews/T030-quinn/README.md:148–166`: payload 1,000 bytes, one
  DATAGRAM per packet, rates 30/300/3,000 datagrams/s, 64 MiB receive /
  16 MiB send buffers, `clean` / `reorder` (75 ms ± 15 ms) / `impaired`
  (+ 3% loss), patched vs unpatched): expects 0 window-attributable
  discards at every rate (baseline: 0 in all 26 runs, `:179–201`), and
  shortfall at ≥ 300/s at most the unpatched baseline (sender-side, which
  the patch does not change). Results committed under
  `docs/reviews/T030-quinn/results/`. *Fails if:* any run shows a dedup
  discard, or the patched build underperforms the control.
- **S1 `v4_session_binds_over_lane1_alpn` (T150c).** Connect on the
  lane-1 ALPN with a registered invitation; expects `bind_v4_peer` to
  succeed and `session_wire == 4` (`tasks/T037.md:447–450`); the same
  credential on `crpg-lane0/1` binds a v3 session as today. Control: the
  T023b `host_quic` suite passes unchanged. *Fails if:* the ALPN binds
  the wrong version or the v3 path changes.
- **S2 `move_round_trip_through_route` (T150c).** Sends one encoded
  `MoveTo` (54 bytes at most) on the lane-1 stream; expects
  `ingest_move` → `Admitted`, one receipt on the lane-1 stream in order,
  and the next tick's `take_samples` bytes as datagrams; `Duplicate`
  resend returns `resent` without a second decision
  (`tasks/T037.md:758–764`). Control: mirror `HistoryWorld` with the
  same sim calls and `authority_hash` equality after acknowledge (the
  T037 test rule, `tasks/T037.md:844–857`). *Fails if:* a byte travels
  the wrong channel, order breaks, or the mirror diverges.

## What must not change

- The 30-case endpoint suite (`crates/crpg-net-quic/tests/quic.rs`),
  `quic_gaps.rs`, `channel.rs`, `close_budget.rs` and their
  `support/quic_rig.rs` helpers pass unchanged; no existing expectation
  moves. Lane-0 window cases keep their T023c values
  (`docs/architecture/crpg-net-quic.md:153–159`).
- The `crpg-server` suites `host_quic.rs` and `host_quic_transfer.rs`
  pass unchanged; every other `crates/crpg-server/tests/` file is
  untouched (the T037 frozen list, `tasks/T037.md:1408`).
- No `crpg-net` byte moves: `movement_v4` layouts and the 54 / 60 /
  1,012 bounds (`tasks/T026p.md:607–609`), and no T037 host signature,
  variant, `Display` text or cap (`tasks/T037.md:324–335`, `:817–825`).
- No `ALLOWED` change, no `deny.toml` change, no workspace
  `Cargo.toml` change (Dependency edges); no new external crate; no
  golden new or moved (brief Permissions: goldens none).
- `quic.rs` lane-0, transfer and resume paths behave byte for byte as
  today (`crates/crpg-server/src/quic.rs:768–809` drain/routing,
  `:949–998` deliver).

## Dependency edges

| Edge | Section | Part | `ALLOWED` change |
|---|---|---|---|
| (vendored patch: no manifest edge; `[patch.crates-io]` already routes `quinn-proto` at the vendored path, `Cargo.toml:33–42`) | — | T150a | none |
| `crpg-net-quic` → `crpg-net` (unchanged; datagrams use the pinned `quinn` 0.11 already in the graph, `crates/crpg-net-quic/Cargo.toml:7–19`) | `[dependencies]` | T150b | none: the row stays `({"crpg-net"}, set())` (`tools/lint/deps.py:55–59`) |
| `crpg-server` → `crpg-net-quic` (unchanged path edge) | `[dependencies]` | T150c | none: the row already allows every crate but `crpg-godot` and `crpg-edit` (`tools/lint/deps.py:69`) |

No new external crate and no new lockfile package in any part: the
DATAGRAM extension is a transport parameter of the pinned `quinn`, and
the patch reuses the T023v vendoring mechanism
(`third_party/quinn-proto/VENDOR.md:3–9`). `deps.py` and `ALLOWED` are
not edited. The I/O-free rule still holds below `crpg-net-quic`
(`tools/lint/deps.py:13–15`); `crpg-net` stays I/O-free
(`crates/crpg-net-quic/AGENTS.md:47–49`); `crpg-server` gains no direct
quinn/quinn-proto/rustls/tokio edge (ADR-0024 item 5,
`docs/adr/0024-separate-crpg-net-quic-crate.md:39–41`; ADR-0026 edge
row, `docs/adr/0026-quic-host-adapter.md:106–110`).

## Out of scope

- T026 prediction (G2, `docs/adr/0036-movement-and-navigation.md:453–458`);
  the `crpg-godot` bridge (T151: `ReassemblyPoolV4`, `ReplicaWorldV4`,
  `MoveOutbox`, T049/G1, T050/G3), owed by T026p
  (`tasks/T026p.md:1477–1479`).
- The nav revision on the wire (T037 Reading 4,
  `tasks/T037.md:136–160`); a `0x83` snapshot with positions (T026p
  Reading 8, `tasks/T026p.md:238–250`); new rejection codes (T026p
  Reading 3, `tasks/T026p.md:185–193`); route length on the wire.
- Any change to `crpg-net`'s codec or to the T037 host surface: T150c
  calls `ingest_move`, `take_move_receipts`, `take_samples` but owns
  none of them.
- T040 invitation generation and format; T025 resume/narrowing; grid
  snapping (S2–S5); terrain, facing, formations and the rest of the D46f
  list (`docs/adr/0036-movement-and-navigation.md:575–589`).
- Any code outside the owning lists; any other file.

## Open questions (each with a recommendation)

1. **Which close code refuses a missed datagram floor?** Reading 4
   proposes `VersionRefused` (6). *Recommend 6:* codes 0–9 are fixed and
   a new code needs its own registry change, while the floor is a
   version property the ALPN already versions.
2. **The lane-1 ALPN string?** Reading 1 proposes `crpg-lane1/1`.
   *Recommend it:* it sorts beside `crpg-lane0/1` and `/2`
   (`crates/crpg-net-quic/src/handshake.rs:12`, `:16`) and cannot
   collide with a future lane-0 version.
3. **One second bidi stream, confirmed?** Reading 2 proposes sharing one
   stream for intents up and receipts down. *Recommend yes:* one total
   order per direction is what the receipt logic needs; anything wider
   doubles the close paths.
4. **Where does the native Windows probe run, and does it gate?**
   Nothing in-repo has run on Windows (T030 gap,
   `docs/reviews/T030-quinn/README.md:292–293`; T023v gap,
   `tasks/T023v.md:177`). *Recommend:* re-run on a native Windows/MSVC
   host with the pinned toolchain, results committed, and T150b does not
   merge until L10 is green there; Linux/GNU is supporting evidence
   only (platform invariant, `AGENTS.md:6–19`).
5. **Sample queue caps: new values or mirrors?** The draft reuses the
   connection's `QueueCaps` with datagram bytes in the same accounting.
   *Recommend mirrors:* the host's lane-1 egress accounting already
   bounds sessions (`tasks/T037.md:701–707`); a second set of numbers
   needs its own T023c-style stall analysis.
6. **Does lane 1 need a `ServerLane`-style `Transport` impl?** The
   lane-0 path has one (`crates/crpg-net-quic/src/server.rs:799–813`,
   proven by `transport_trait_round_trip`,
   `crates/crpg-net-quic/tests/quic.rs:1708`). *Recommend yes:* the
   T150c tests should drive lane 1 through the same trait the in-memory
   fabric speaks.
7. **Limits field or endpoint constant for the floor?** The Interface
   proposes tightening-only `max_datagram_bytes` on both limits structs.
   *Recommend the field:* it follows the T023c precedent
   (`docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:152–176`) and
   keeps `v1()` reproducing exactly today's behaviour.

## Agent log

- 2026-11-15 (UTC) · opencode-go/muse-spark-1.3-contributor + T150 draft b · Opened the blind-draft skeleton with every contract section so each section can be filled and committed separately.
- 2026-11-15 (UTC) · opencode-go/muse-spark-1.3-contributor + T150 draft b · Filled the draft: three serial parts (vendored D23 patch, lane-1 stream pair plus DATAGRAMs with a 1,024 floor refused at connect, the `quic.rs` v4 route over the T037 surface), ten readings, proposed interfaces, ten behaviours with named tests, and seven open questions with recommendations; nothing here is approved.
