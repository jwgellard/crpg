# T150-draft-a — lane-1 movement transport (`crpg-net-quic`, then `crpg-server`)

**Status: blind draft (nothing approved).** Written by the architect from the
T150 fact sheet (`tasks/T150-facts.md`), the approved T026p lane-1 items and
Out-of-scope (`tasks/T026p.md:967–1026`, `:1464–1476`), the approved T037
contract's Interface and Out-of-scope (`tasks/T037.md:1351–1355`), ADR-0036,
ADR-0024, ADR-0025, ADR-0026 and `tasks/DECISIONS-2026-09-30.md:83–88`. A
Claude architect writes the final contract; the user approves it. Nothing
here is approved.

Citation convention (T140's): a bare `:N` or `:N–M` is a line of the file
cited earlier in the same list item or sentence. Every citation was checked
against the branch `task/T150-draft-a-d1` (base `c4d32b9`, the fact sheet).

**`<base>`** (used in "Definition of done") is the commit a part's branch is
cut from: a descendant of `master` that holds T037d's code and the fact
sheet, plus the previous part's commits.

## Task

Carry lane 1 (movement) between a v4 host and its clients, as T026p left to
a new `crpg-net-quic` task and T037 froze out of `quic.rs`
(`tasks/T026p.md:1471–1476`, `tasks/T037.md:1351–1355`): lane 1's reliable
stream pair (intents upstream, receipts downstream), the QUIC DATAGRAM
extension for samples with a maximum datagram size of at least
`MAX_MOVE_DOWN_BYTES` refused at connect otherwise, D23's vendored dedup
patch, and the native Windows impairment probe re-run — all in
`crpg-net-quic` — and then the `crpg-server` `quic.rs` route that binds v4
sessions and moves `ingest_move`, `take_move_receipts` and `take_samples`
bytes. Today lane 0 is the only lane: one client-initiated bidi stream
carries hello, welcome and every lane-0 frame, and the DATAGRAM extension is
off (`crates/crpg-net-quic/src/tls.rs:86–91`, `:129–134`;
`docs/architecture/crpg-net-quic.md:141–147`). It is split into two
serialized parts, **T150a → T150b**, one crate each; T150a is S-set (a new
ALPN is a wire framing, and the dedup patch is a vendored dependency patch
plus the workspace `Cargo.toml`), T150b is not.

- **T150a — `crpg-net-quic` (S-set).** `Framing::V3` and ALPN
  `crpg-lane0/3`; the second client-initiated bidi stream for lane-1 intents
  and receipts; the DATAGRAM extension with the connect-time minimum and
  refusal; D23's `0002-widen-dedup-window` vendored patch; the native
  Windows impairment probe re-run.
- **T150b — `crpg-server`.** The `quic.rs` route: accept wire version 4 on
  `/3`, bind `bind_v4_peer`, and move lane-1 intents into `ingest_move`,
  receipts out of `take_move_receipts` and samples out of `take_samples`,
  traced in `HostCall`.

## Parts

Each part is one crate and one implementer run. Order: **serial**, T150a →
T150b. T150b needs T150a's `Framing::V3`, `try_recv_lane1`,
`try_send_lane1` and `try_send_datagram`.

| Part | Crate | Scope | S-set | Review |
|---|---|---|---|---|
| T150a | `crpg-net-quic` | V3 framing + lane-1 stream + datagrams + dedup patch + probe | **yes** | Claude reviewer mutant run, one fix round |
| T150b | `crpg-server` | the `quic.rs` v4 route | no | same |

**S-set** (`docs/IMPLEMENTATION_PLAN.md` §1.4, read as T140 read it): T150a
fixes wire bytes (a new ALPN = a new stream mapping and DATAGRAM
negotiation) and vendors a dependency patch (the workspace `Cargo.toml`
`[patch.crates-io]` and `third_party/quinn-proto/` are S-set). T150b fixes
no new byte: every wire byte it moves is already fixed by T026p (lane 1),
T140 (v3) and the T037 host surface; it maps `DeliveryKind` and host calls
to the transport API, so it is not S-set.

### T150a owns
- `crates/crpg-net-quic/src/handshake.rs` (`ALPN_LANE0_V3`, `Framing::V3`),
  `crates/crpg-net-quic/src/tls.rs` (the V3 transport parameters and the
  datagram buffers), `crates/crpg-net-quic/src/queue.rs` (the lane-1
  reader/writer and the datagram path), `crates/crpg-net-quic/src/server.rs`,
  `crates/crpg-net-quic/src/client.rs` (the lane-1 and datagram API),
  `crates/crpg-net-quic/src/limits.rs`, `crates/crpg-net-quic/src/close.rs`
  (a datagram send error), `crates/crpg-net-quic/src/lib.rs` (re-exports)
- `crates/crpg-net-quic/tests/lane1.rs` (new), `crates/crpg-net-quic/tests/lane1_datagram.rs` (new)
- `third_party/quinn-proto/patches/0002-widen-dedup-window.patch` (new),
  `third_party/quinn-proto/crate/src/connection/spaces.rs` (re-derived by the
  patch, never hand-edited), `third_party/quinn-proto/VENDOR.md` (patch table
  + tree digest), the root `Cargo.toml` (only if the `[patch.crates-io]`
  comment changes; the table itself already points at `third_party/quinn-proto/crate`)
- `docs/architecture/crpg-net-quic.md`, `crates/crpg-net-quic/AGENTS.md`
- goldens: none new, none moved.

### T150b owns
- `crates/crpg-server/src/quic.rs` (the `Framing::V3` arm, `Invitation`'s
  two new fields, the lane-1 drain/deliver steps, the `HostCall` variants)
- `crates/crpg-server/tests/host_quic_move.rs` (new)
- `docs/architecture/crpg-server.md`, `crates/crpg-server/AGENTS.md`
- goldens: none new, none moved.

## Readings

Design decisions this contract makes. Each needs approval with the contract.

1. **A new ALPN `crpg-lane0/3` = `Framing::V3`.** A second stream plus the
   DATAGRAM extension is a stream-mapping and framing change, which ADR-0025
   item 2 requires a new ALPN for
   (`docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:31–34`), and the
   crate trap forbids enabling either without one
   (`crates/crpg-net-quic/AGENTS.md:125`). `Framing::V3`
   (`crates/crpg-net-quic/src/handshake.rs:71`) pins: the `/2` channel
   framing on stream 1 (Lane0, Transfer, Resume), the second client-initiated
   bidi stream for lane 1, and the DATAGRAM extension with a minimum of
   `MAX_MOVE_DOWN_BYTES`. *Rejected:* enabling datagrams under `/1` or `/2`
   (the trap); a new `crpg-lane1/1` prefix (breaks the `crpg-lane0/N`
   framing-version namespace, which already carries the snapshot and resume
   channels under one ALPN, `:100–128`).
2. **Lane 1 is a second client-initiated bidi stream, not a `/2` channel.**
   `Channel` (`crates/crpg-net-quic/src/handshake.rs:103–128`) multiplexes
   the lane-0 stream by a selector byte; lane 1 is its own sequence domain
   per epoch (ADR-0036 L1,
   `docs/adr/0036-movement-and-navigation.md:389–390`) and its frames start
   with `0xA1` (`crates/crpg-net/src/movement_v4.rs:32`), not a selector.
   *Rejected:* a `Channel::Movement = 0xA1` on stream 1 (would make the
   selector depend on payload bytes, against the "never choose a channel from
   payload bytes" trap, and would couple lane-1 order to lane-0's one queue).
3. **The lane-1 stream reuses the 4-byte length framing.**
   `frame_header`/`parse_frame_header`
   (`crates/crpg-net-quic/src/handshake.rs:43–67`). Upstream cap
   `MAX_INTENT_FRAME_BYTES` (4,096, the same cap `decode_intent` enforces,
   `crates/crpg-net/src/movement_v4.rs:229`); downstream cap
   `MAX_MOVE_DOWN_BYTES` (1,024, above the 60-byte largest receipt,
   `tasks/T037.md:459`). *Rejected:* a channel-framed lane-1 stream (on its
   own stream lane 1 needs no selector); a 65,536 downstream cap (a receipt
   is ≤ 60 bytes; 1,024 keeps the two downstream shapes on one bound).
4. **The datagram minimum is exactly `MAX_MOVE_DOWN_BYTES` (1,024).** The
   client advertises `max_datagram_frame_size = Some(1_024)`; the server
   refuses, before writing the welcome, a client whose advertised size is
   absent or `< 1_024`, with `CloseCode::ProtocolViolation` (2). The largest
   sample is 1,012 bytes (24 + 38 × 26,
   `tasks/T026p.md:607–609`), so 1,024 admits every legal sample.
   *Rejected:* a minimum below 1,024 (could refuse a legal sample); a new
   close code (none is approved; 2 already means "the peer violated the
   pinned framing", and the ALPN pins the minimum).
5. **The dedup patch is `0002-widen-dedup-window.patch`**, applied after
   0001 in file-name order by the vendoring mechanism
   (`third_party/quinn-proto/VENDOR.md:3–9`, `:69–70`). It widens
   `Dedup`'s window from 129 to 2,049 packets
   (`docs/reviews/T030-quinn/README.md:14–15`, `:77–92`) so reordered-but-new
   datagrams are not dropped (quinn#2710). *Rejected:* keeping the 129-bit
   window (silent datagram loss under reordering; D23 requires the patch with
   the first datagram lane, `tasks/DECISIONS-2026-09-30.md:83–88`).
6. **Samples flow downstream only, as datagrams only.** The server advertises
   `max_datagram_frame_size = None` (so a client `send_datagram` fails) and a
   bounded `datagram_send_buffer_size`; the client advertises
   `Some(MAX_MOVE_DOWN_BYTES)` and a bounded `datagram_receive_buffer_size`.
   *Rejected:* the server advertising a receive size (accepts unsolicited
   upstream datagrams lane 1 has no codec for, `movement_v4` has no upstream
   datagram shape).
7. **Datagram backpressure is drop-whole, repaired by refresh.** A
   `try_send_datagram` that is `Blocked` (buffer full) or `Closed` is not
   retried by the adapter; the host's 20-tick refresh
   (`POLICY_SAMPLE_REFRESH_TICKS`, `tasks/T037.md:456`) replaces the lost
   sample. Lane-1 stream backpressure reuses the lane-0 queue caps. The
   datagram buffers are limits (below), not new unbounded queues.
   *Rejected:* retrying a refused datagram (a stale sample is worse than
   none, and ADR-0036 L4 makes samples lossy).
8. **The lane-1 stream opens after the welcome.** The client opens stream 1
   (hello) and, once the handshake completes, stream 2 (lane 1); the server's
   accept loop expects a second `accept_bi` on `/3`. The server reads lane-1
   intents only from a bound connection. *Rejected:* opening stream 2 before
   the hello is accepted (the server cannot distinguish lane 1 from lane 0
   traffic until the framing is fixed).
9. **The `quic.rs` v4 route binds only wire version 4 on `/3`, and facts and
   rights ride a separate v4 registration.** `on_hello`
   (`crates/crpg-server/src/quic.rs:596`) gains a `Framing::V3` arm that
   accepts wire version 4 and calls `bind_v4_peer`. Because `bind_v4_peer`
   builds the first snapshot from `FactGrants` and `PauseRights`
   (`tasks/T037.md:437–441`) and `FactGrants` has no `Default`
   (`crates/crpg-server/src/host.rs:244`), they are supplied by a new
   `register_v4_invitation` + `V4Invitation` (a `credential`, `control`,
   `grants`, `facts`, `rights`), leaving the existing `Invitation`
   (`crates/crpg-server/src/quic.rs:76`) and every struct-literal site
   untouched; the internal `InvitationRecord` gains
   `facts: Option<FactGrants>` and `rights: Option<PauseRights>` (`None` for
   a `/1`//`/2`-only invitation), and a `/3` hello whose record lacks them is
   refused `AuthRefused`. v3-over-QUIC is not part of T150 (Out of scope).
   *Rejected:* adding the fields to `Invitation` (breaks the
   `invitation()` helper and the `main.rs` wiring, an existing-test edit);
   empty defaults plus post-bind setters (the first delivery is already
   staged at bind).

## Interface

Additions only. Every public item has a doc comment (`#![warn(missing_docs)]`).
No `HashMap`/`HashSet`, float, clock or sim state in `crpg-net-quic` (crate
invariants 1–3, `crates/crpg-net-quic/AGENTS.md`). Bytes stay opaque: the
lane-1 reader checks only the length prefix, never the `0xA1` payload.

### T150a — `handshake.rs`, `close.rs`, `server.rs`, `client.rs`
```rust
// handshake.rs, after ALPN_LANE0_V2 (:16):
/// ALPN identifier: lane framing version 3 = the `/2` channel framing on
/// stream 1 plus the lane-1 stream and the DATAGRAM extension.
pub const ALPN_LANE0_V3: &[u8] = b"crpg-lane0/3";

// Framing gains (after V2, :74–76):
    /// `crpg-lane0/3`: as `V2` (Lane0/Transfer/Resume channels on stream 1),
    /// plus a second client-initiated bidi stream (lane 1) and the DATAGRAM
    /// extension with a minimum of `MAX_MOVE_DOWN_BYTES`.
    V3,
// Framing::alpn (:81) returns ALPN_LANE0_V3 for V3; from_alpn (:89) matches it.
```
```rust
// close.rs, after ChannelSendError (:159):
/// Why `try_send_datagram` queued nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatagramSendError {
    /// The peer did not negotiate DATAGRAM, or its advertised size is below
    /// the datagram. Unreachable after the connect refusal, kept total.
    UnsupportedByPeer,
    /// The datagram is above the peer's advertised maximum.
    TooLarge,
    /// The retryable `try_send` refusal for the same frame.
    Send(SendError),
}
```
```rust
// server.rs (QuicServer), beside try_recv_channel (:479) / try_send_channel (:549):
    /// One lane-1 intent frame from the connection's lane-1 stream, in send
    /// order; `None` when the queue is empty. `NotAccepted` before the hello.
    pub fn try_recv_lane1(&mut self, conn: ConnectionId) -> Result<Option<Vec<u8>>, ServerError>;
    /// Queues one lane-1 receipt frame on the lane-1 stream.
    pub fn try_send_lane1(&mut self, conn: ConnectionId, bytes: &[u8]) -> Result<(), SendError>;
    /// Queues one sample datagram; `TooLarge` above the peer's maximum,
    /// `UnsupportedByPeer` when DATAGRAM was not negotiated (both unreachable
    /// on an accepted `/3` connection).
    pub fn try_send_datagram(&mut self, conn: ConnectionId, bytes: &[u8]) -> Result<(), DatagramSendError>;
```
```rust
// client.rs (QuicClient), beside try_recv_channel (:386) / try_send_channel (:330):
    /// One lane-1 receipt frame, in send order.
    pub fn try_recv_lane1(&mut self) -> Result<Option<Vec<u8>>, CloseReason>;
    /// Queues one lane-1 intent frame on the lane-1 stream.
    pub fn try_send_lane1(&mut self, bytes: &[u8]) -> Result<(), SendError>;
    /// One sample datagram, in send order; `None` when none is queued.
    pub fn try_recv_datagram(&mut self) -> Result<Option<Vec<u8>>, CloseReason>;
```

### T150a — `tls.rs` (private), `limits.rs`
The V3 transport parameters, reached only when `Framing::V3` is negotiated
(the ALPN decides the config, as today
`crates/crpg-net-quic/src/tls.rs:69–101`):
- server (`:86–91`): `bidi_streams: 2` (was 1), `max_concurrent_uni_streams:
  0`, `datagram_receive_buffer_size(None)`, `max_datagram_frame_size(None)`
  (the server receives no datagram), `datagram_send_buffer_size(DATAGRAM_SEND_BUFFER_BYTES)`.
- client (`:129–134`): `bidi_streams: 0` (unchanged),
  `max_datagram_frame_size(Some(VarInt::from_u32(DATAGRAM_MIN_BYTES)))`,
  `datagram_receive_buffer_size(Some(DATAGRAM_RECV_BUFFER_BYTES))`,
  `datagram_send_buffer_size(0)`.

```rust
// limits.rs (private, beside the V1_* windows at :36–42):
/// The datagram minimum the client advertises and the server enforces at
/// accept: `crpg_net::movement_v4::MAX_MOVE_DOWN_BYTES`.
const DATAGRAM_MIN_BYTES: u32 = 1_024;
/// Server-side datagram send buffer (memory bound for samples awaiting send).
const DATAGRAM_SEND_BUFFER_BYTES: usize = 65_536;
/// Client-side datagram receive buffer (memory bound for samples awaiting read).
const DATAGRAM_RECV_BUFFER_BYTES: usize = 262_144;
const _: () = assert!(DATAGRAM_MIN_BYTES as usize == crpg_net::movement_v4::MAX_MOVE_DOWN_BYTES);
```

The lane-1 stream reuses `frame_header`/`parse_frame_header`
(`crates/crpg-net-quic/src/handshake.rs:43–67`) with the upstream cap
`crpg_net::protocol::MAX_INTENT_FRAME_BYTES` (4,096,
`crates/crpg-net/src/protocol.rs:58`) and the downstream cap
`DATAGRAM_MIN_BYTES` (1,024). Its inbound/outbound queues reuse the existing
`QueueCaps` per connection (128 frames; `crates/crpg-net-quic/src/limits.rs:53–80`).

### T150b — `quic.rs` (`crates/crpg-server/src/quic.rs`)
```rust
// V4Invitation, beside Invitation (:76):
/// An operator-provisioned v4 principal: the v1/v2 invitation plus the
/// fact grants and pause rights `bind_v4_peer` builds the first snapshot
/// from (T037 B-1). `Debug` redacts the credential as `Invitation` does.
#[derive(Debug, Clone)]
pub struct V4Invitation {
    /// Exactly [`INVITATION_CREDENTIAL_BYTES`] bytes of operator secret.
    pub credential: Credential,
    /// The actors this principal may act as.
    pub control: ControlGrant,
    /// What this principal may learn.
    pub grants: DisclosureGrants,
    /// The v4 fact grants, used at bind time.
    pub facts: FactGrants,
    /// The v4 principal's own pause rights.
    pub rights: crpg_net::protocol_v3::PauseRights,
}

impl QuicHost {
    /// As [`register_invitation`](Self::register_invitation), but the record
    /// also carries `facts` and `rights` for a `/3` v4 binding. Same
    /// precedence and errors.
    pub fn register_v4_invitation(&mut self, invitation: V4Invitation)
        -> Result<InvitationId, QuicHostError>;
}
```
`HostCall` (the trace enum, beside `BindSnapshotPeer` and `IngestTransfer`)
gains:
```rust
    /// `bind_v4_peer` for a `/3` hello naming wire version 4.
    BindV4Peer { conn: ConnectionId, invitation: InvitationId, result: Result<PeerHandle, HostError> },
    /// One `ingest_move` call, with its `MoveAdmission` result.
    IngestMove { conn: ConnectionId, peer: PeerHandle, frame: u64, len: usize, result: Result<MoveAdmission, HostError> },
    /// One `take_move_receipts` call; the result is the count of receipts.
    TakeMoveReceipts { peer: PeerHandle, result: Result<usize, HostError> },
    /// One `take_samples` call; the result is the count of datagrams.
    TakeSamples { peer: PeerHandle, result: Result<usize, HostError> },
```
`on_hello` (`:596`) gains a `Framing::V3` arm: the version check (`:619`)
becomes framing-aware — `/3` requires `wire_version == 4`, else
`VersionRefused`; the bind arm (`:650–659`) gains
`Framing::V3 => host.bind_v4_peer(control, grants, facts, rights, now_ms)`
with `facts`/`rights` taken from the record (a `/3` hello whose record lacks
them is refused `AuthRefused`), traced `HostCall::BindV4Peer`; the welcome
(`:689–692`) uses `session_wire(peer)` (4) as its `wire_version`. The
`drain` phase (`:768`) reads `try_recv_lane1` and traces `HostCall::IngestMove`;
the `deliver` phase (`:949`) writes `take_move_receipts` to `try_send_lane1`
and `take_samples` to `try_send_datagram` (traced
`TakeMoveReceipts`/`TakeSamples`).

## Behaviour

Normative. Each item names the tests that prove it. "Refused" means the
stated error and no state change. **First failure wins** in every list.

### T150a: the lane-1 transport

A-1. **ALPN and framing.** `ALPN_LANE0_V3` is exactly the bytes
   `crpg-lane0/3`; `Framing::from_alpn` maps it to `V3` and nothing else
   does; `Framing::V3.alpn()` returns it; `/1` and `/2` are unchanged. `bind`
   stays `/1`-only; `bind_dual` advertises
   `[crpg-lane0/3, crpg-lane0/2, crpg-lane0/1]` in that order;
   `bind_resumable` is unchanged (resume is out of scope, T025b). An accepted
   `/3` connection reports `framing() == Framing::V3`. *Tests:* A1.

A-2. **Stream open and close order.** On `/3` the client opens stream 1
   (hello, welcome, then lane-0 frames, exactly as `/2`) and, after `connect`
   returns, stream 2 (lane-1 intents upstream, receipts downstream). The
   server accepts stream 1 first and, for `/3`, stream 2 second; lane-1
   frames are read only from stream 2, and a lane-0 or `0x81`/`0x91` frame
   never rides stream 2. *Tests:* A2.

A-3. **Datagram negotiation and the refusal at connect.** The client
   advertises `max_datagram_frame_size = Some(1_024)`. The server, before
   writing the welcome to a `/3` connection, checks
   `connection.max_datagram_size()`; if it is `None` or `Some(n)` with
   `n < DATAGRAM_MIN_BYTES` (1,024), the connection is closed with
   `CloseCode::ProtocolViolation` (2), no welcome is written and no lane-1
   or lane-0 data is accepted. A `/1` or `/2` connection never enables
   datagrams and is never refused for this reason. *Tests:* A3.

A-4. **Lane-1 frames are opaque and ordered.** `try_recv_lane1` returns
   intent frames in send order; `try_send_lane1` queues receipt frames in
   call order. Each frame is `[len][payload]` with the upstream cap
   `MAX_INTENT_FRAME_BYTES` (4,096) and the downstream cap `DATAGRAM_MIN_BYTES`
   (1,024); a zero-length frame is `EmptyFrame`, an over-cap frame
   `FrameTooLarge`. The lane-1 reader never inspects the `0xA1` payload.
   *Tests:* A4.

A-5. **Samples are datagrams only.** `try_send_datagram` sends one datagram
   up to the peer's advertised maximum; above it is `TooLarge`, and
   `UnsupportedByPeer` when DATAGRAM was not negotiated (unreachable on an
   accepted `/3`). `try_recv_datagram` returns datagrams in send order. A
   sample never rides the lane-1 stream and a receipt never rides a
   datagram. *Tests:* A5.

A-6. **Backpressure and the lane-0 limits.** The lane-1 stream queues are
   bounded (the existing per-connection `QueueCaps`, 128 frames);
   `try_send_lane1` returns `QueueFull` when full, changes nothing and is
   retryable; a full inbound lane-1 queue parks one frame and stops reading
   (flow control), as lane 0. `try_send_datagram` returns `Send(QueueFull)`
   when the datagram send buffer is full; the adapter drops the sample and
   does not retry it (A-7's refresh repairs). *Tests:* A6.

A-7. **Close and teardown with lane 1 open.** A `Flush` close writes every
   queued lane-1 receipt (then FIN on stream 2) and every queued sample
   datagram before CONNECTION_CLOSE, inside the close budget
   (`crates/crpg-net-quic/src/teardown.rs:9–42`); a `Discard` close drops
   both at once. Datagrams have no FIN, so `Flush` sends what is queued then
   closes; a lane-1 receipt queued after the flush deadline is not sent.
   `CloseReason` stays first-wins. *Tests:* A7.

A-8. **D23's dedup patch is applied (S-set).**
   `third_party/quinn-proto/crate/src/connection/spaces.rs` holds
   `WINDOW_BITS = 2048` and `WINDOW_SIZE = 2049` (no `type Window = u128`),
   and `Dedup.window` is `[u64; 32]`; `VENDOR.md` lists the patch in the
   table with the re-derived tree digest. The patch applies on top of 0001
   with `patch --dry-run`, and the vendored crate's own suite (with the
   widened-window unit tests) passes. *Tests:* A8 (unit) + the VENDOR
   re-derivation script.

A-9. **The impairment probe's pass criteria.** Re-run the T030 probe
   (`docs/reviews/T030-quinn/probe/`, isolated, its own `[workspace]` and
   `Cargo.lock`) on native Windows/MSVC with the vendored patch applied, at
   the documented parameters: 1,000-byte payload, one DATAGRAM per packet,
   rates 30/300/3,000 datagrams/s, `clean`/`reorder` (75 ms ± 15 ms)/
   `impaired` (plus 3 % loss), `datagram_receive_buffer_size` 64 MiB,
   `datagram_send_buffer_size` 16 MiB, patched and unpatched builds compared
   through `--config patch.crates-io.quinn-proto.path=…`
   (`docs/reviews/T030-quinn/README.md:148–166`). **Pass:** 0 dedup discards
   in every run of the matrix on the patched build, with the Windows results
   recorded under `docs/reviews/T030-quinn/results/` (closing the Windows gap
   at `README.md:292–293`). *Tests:* A9 (a Definition-of-done step, not a
   cargo test).

### T150b: the `quic.rs` v4 route

B-1. **v4 binding.** A `/3` hello naming wire version 4 is accepted and
   `bind_v4_peer(control, grants, facts, rights, now_ms)` is called; the
   welcome's `wire_version` is 4 (from `session_wire(peer)`); the trace
   holds `BindV4Peer`. A `/3` hello naming version 1, 2 or 3, or a `/1`//`/2`
   hello naming version 4, is refused `VersionRefused`. *Tests:* B1.

B-2. **Lane-1 drain.** In the drain phase each lane-1 intent frame is
   `host.ingest_move(peer, bytes, now_ms)` and traced `IngestMove`; the
   `MoveAdmission` result is not fed into the lane-0 hold/disposition logic
   (no hold for a `Refused`). A held lane-0 frame does not block lane-1
   drain, and a lane-1 refusal does not fence the binding. *Tests:* B2.

B-3. **Receipt and sample delivery.** In the deliver phase,
   `take_move_receipts(peer)` bytes are sent in order via `try_send_lane1`
   and `take_samples(peer)` bytes via `try_send_datagram`, both traced
   `TakeMoveReceipts`/`TakeSamples`. A sample that does not fit
   (`Send(QueueFull)`) is dropped whole and not retried; a receipt that does
   not fit is retried next pump (the stream is reliable). *Tests:* B3.

B-4. **Trace completeness.** Every lane-1 host call
   (`BindV4Peer`, `IngestMove`, `TakeMoveReceipts`, `TakeSamples`) is in
   `QuicPumpReport.calls` in call order, so a replay against an in-memory
   host reproduces the authority hash. *Tests:* B4.

B-5. **Lane 0 is unchanged on `/3`.** A v4 session's lane-0 deltas and `0x82`
   transfers ride stream 1's channels, mapped from `DeliveryKind::Delta`/
   `Transfer` as today (`crates/crpg-server/src/quic.rs:994–997`); the
   lane-1 steps change no lane-0 byte. *Tests:* B5.

## Errors

One new error type and one re-used close code; no new `CloseCode` variant,
no new codec error, no new `RejectionCode`.

| Refusal | Result |
|---|---|
| `/3` peer did not advertise `max_datagram_frame_size ≥ 1_024` | closed `CloseCode::ProtocolViolation` (2) before the welcome; the adapter reports `HelloOutcome::Lost` |
| `try_send_datagram` above the peer's maximum | `DatagramSendError::TooLarge` |
| `try_send_datagram` when DATAGRAM was not negotiated | `DatagramSendError::UnsupportedByPeer` |
| `try_send_datagram` buffer full / closed / unknown / empty | `DatagramSendError::Send(SendError::QueueFull | Closed | UnknownConnection | EmptyFrame | …)` |
| `try_send_lane1` zero-length / over-cap / full / closed | `SendError::EmptyFrame` / `FrameTooLarge` / `QueueFull` / `Closed` (unchanged texts) |
| `try_recv_lane1` / `try_recv_datagram` on a closed connection | the existing `ServerError::Closed(reason)` / `CloseReason` |

`DatagramSendError`'s `Display` follows the crate pattern
(`crates/crpg-net-quic/src/close.rs:149–158`):
`"UnsupportedByPeer at quic/datagram"`, `"TooLarge at quic/datagram"`,
`"Send at quic/datagram"` (delegating to `SendError`); `source()` is `None`.
Every existing `SendError`, `ServerError`, `ChannelSendError`, `CloseCode`
and `CloseReason` text and variant is unchanged. `HostError` gains no variant
(the v4 route reuses the T037 host surface; a `bind_v4_peer` refusal maps to
the existing `CloseCode`s exactly as `on_hello` maps `bind_peer`'s
`crates/crpg-server/src/quic.rs:674–685`).

## Tests

Public API only, except A8 (unit). The suites reuse
`crates/crpg-net-quic/tests/support/quic_rig.rs` (`loopback`, `identity_a`,
`bind_server`, `client_config`, `credential`, `hello`, `connect_accepted`,
`RawRuntime`/`RawClient`/`raw_connect`, `UdpRelay` with
`set_impaired`/`set_blackhole`) and `crates/crpg-server/tests/support/quic_host_rig.rs`
(fixtures, `TestClock`, `pump_until`). Literal bytes below are the T026p wire
layout (`tasks/T026p.md:596–609`), computed by hand: a seq-1 `Halt` by
`n(1)` is 21 bytes (version 1 + epoch 16 + lane 1 + seq 1 + actor 1 + tag 1,
`tasks/T037.md:1175–1176`), the largest intent 54, the largest receipt 60,
the largest sample 1,012 (24 + 38 × 26), and `DATAGRAM_MIN_BYTES` = 1,024.

### T150a: `crates/crpg-net-quic/tests/lane1.rs`, `lane1_datagram.rs`
- **A1 `alpn_and_framing`.** `Framing::from_alpn(b"crpg-lane0/3") ==
  Some(Framing::V3)`, `Framing::V3.alpn() == b"crpg-lane0/3"`,
  `from_alpn(b"crpg-lane0/1") == Some(V1)`, `from_alpn(b"crpg-lane0/2") ==
  Some(V2)`, `from_alpn(b"crpg-lane0/4") == None`. A `bind_dual` server
  accepts a `/3` client and `framing(conn) == V3`; `bind` (`/1`-only)
  refuses a `/3` client. Control: the existing `/1`/`/2` mapping tests.
  *Fails if:* a wrong ALPN maps to `V3`, or `bind` accepts `/3`.
- **A2 `stream_open_order`.** Connect `/3`; the client sends the hello on
  stream 1 and, after `connect`, a 21-byte lane-1 `Halt` on stream 2; the
  server reads `HelloReceived`, `accept`s, then `try_recv_lane1` returns the
  21 bytes and `try_recv_channel` returns the lane-0 frames in order.
  Control: a `/2` client's stream-1 traffic is unchanged. *Fails if:* a
  lane-1 frame is read from stream 1, a lane-0 frame from stream 2, or the
  intent bytes differ.
- **A3 `datagram_refusal`.** `raw_connect` with
  `max_datagram_frame_size(None)`: the server closes `ProtocolViolation`
  (2), no welcome, the adapter sees `HelloOutcome::Lost`. `Some(1023)`:
  same. `Some(1024)`: accepted, welcome written. Control: a `/1` client (no
  datagram) is accepted normally. *Fails if:* a peer below 1,024 is
  accepted, or a 1,024-byte peer is refused.
- **A4 `lane1_frames_opaque_and_ordered`.** Client `try_send_lane1` three
  intents (21, 54, 21 bytes); server `try_recv_lane1` returns them in order,
  byte-identical. `try_send_lane1(&[0u8; 4097])` is `FrameTooLarge`;
  `try_send_lane1(&[])` is `EmptyFrame`. Server `try_send_lane1` a 60-byte
  receipt → client `try_recv_lane1` returns it. *Fails if:* order breaks, a
  cap is off by one, or a byte is altered.
- **A5 `samples_are_datagrams`.** Server `try_send_datagram` a 1,012-byte
  sample → client `try_recv_datagram` returns it byte-identical; a
  1,025-byte datagram is `TooLarge`. A sample never appears in
  `try_recv_lane1` and a receipt never in `try_recv_datagram`. *Fails if:* a
  sample rides the stream, a receipt rides a datagram, or the 1,024 bound is
  wrong.
- **A6 `backpressure`.** A server whose lane-1 outbound queue is full returns
  `QueueFull` from `try_send_lane1` and queues nothing; after the client
  drains it succeeds. A client with tightened windows makes
  `try_send_datagram` return `Send(QueueFull)` after a computable number of
  datagrams; the adapter drops and does not retry. Control: a `/2` session's
  `try_send_channel` `QueueFull` behaviour is unchanged. *Fails if:* lane 1
  escapes a cap or a `QueueFull` queues anything.
- **A7 `close_with_lane1_open`.** Open `/3`, queue two 60-byte receipts and
  one 1,012-byte sample, `close(Flush)`; the client receives both receipts
  and the sample before `Closed`. `close(Discard)` sends none. Control: the
  existing close-budget suite (`close_budget.rs`). *Fails if:* `Flush` drops
  a queued receipt or sample, or `Discard` sends one.
- **A8 `dedup_window` (unit).** The vendored `spaces.rs` defines
  `WINDOW_SIZE == 2049` (no `type Window = u128`); `VENDOR.md`'s patch table
  lists `0002-widen-dedup-window.patch` with a matching tree digest, and the
  re-derivation script prints `vendored tree reproduced`. *Fails if:* the
  patch is missing or the digest drifts.
- **A9 `impairment_probe`.** Not a cargo test: the Definition-of-done step
  runs `probe/run-matrix.sh` on native Windows/MSVC with the vendored patch
  and records 0 dedup discards under `docs/reviews/T030-quinn/results/`.

### T150b: `crates/crpg-server/tests/host_quic_move.rs`
- **B1 `bind_v4_over_quic`.** A `/3` `QuicHost` with an invitation carrying
  control/grants/facts/rights; a v4 client hello (wire version 4) binds
  `bind_v4_peer` (`BindV4Peer` `Ok`, `session_wire` 4), and the first
  delivery (a `Resync` plus the position page) reaches the client. A `/3`
  hello naming version 3 is `VersionRefused`. Control: T037's in-memory
  `bind_v4_peer` first delivery. *Fails if:* a v4 hello is refused or a
  non-4 hello binds.
- **B2 `lane1_drain`.** A v4 client sends one `MoveOutbox` `MoveTo` (54
  bytes); the pump drains it as `ingest_move` (`IngestMove`, `Admitted { seq:
  1 }`), then `advance`; the receipt reaches the client via
  `try_send_lane1`. A malformed intent traces `Refused` and does not fence.
  *Fails if:* an intent is not ingested or a refusal fences the binding.
- **B3 `receipt_and_sample_delivery`.** After a move, `take_move_receipts`
  and `take_samples` are drained each pump to the lane-1 stream and
  datagrams (traced `TakeMoveReceipts`/`TakeSamples`); a sample that does
  not fit (`Send(QueueFull)`) is dropped whole and not retried. *Fails if:*
  a receipt is reordered or a sample is retried.
- **B4 `trace_replay`.** Replaying `QuicPumpReport.calls` against the
  in-memory host reproduces the authority hash (as `tests/host_quic.rs`
  does for lane 0). *Fails if:* a lane-1 call is missing or mis-ordered in
  the trace.
- **B5 `lane0_unchanged_on_v3`.** A v4 session's lane-0 delta and `0x82`
  transfer ride stream-1 channels byte-for-byte as a v3 twin (T037 B-3's
  differential, `tasks/T037.md:981–990`). *Fails if:* a lane-1 change alters
  a lane-0 byte.

## What must not change

- **Goldens**, byte for byte (none is added; the lane-1 transport writes no
  hash golden):
  - `crates/crpg-testkit/goldens/{combat_basic,replay_basic,srd_lite}_rust-1.98.0_{x86_64-pc-windows-msvc,x86_64-unknown-linux-gnu}_test-default.golden`
  - `crates/crpg-sim/tests/goldens/history_v1.*`, `history_v2.*`,
    `history_v3.*`, `movement_v1.*`, `surface_v1.*` (two targets each)
- **Every existing `crpg-net-quic` suite**, byte for byte and passing:
  `tests/quic.rs` (32 cases), `tests/quic_gaps.rs`, `tests/channel.rs` (21
  cases), `tests/close_budget.rs`, and every `#[cfg(test)]` case in `src/`.
  The `datagrams_and_connection_limit_fail_closed` case
  (`crates/crpg-net-quic/tests/quic.rs:1652`) keeps pinning `/1`//`/2` as
  datagram-off; only `/3` enables them.
- **Every existing `crpg-server` test file** byte for byte and passing, and
  the frozen sources `quic.rs`'s pre-T150 paths, `save.rs`, `main.rs`,
  `capture.rs`, `checkpoint.rs` (their lane-0 behaviour). T150b is additive
  to `quic.rs`.
- **Bytes and versions:** every v1, v2, v3, `0x81`, `0x82` byte a session
  receives; `CloseCode` 0–9 and their names; the `/1` and `/2` ALPNs and
  their stream mapping; the lane-1 wire bytes fixed by T026p
  (`0xA1` intents, receipts, samples); the T037 host surface
  (`bind_v4_peer`, `ingest_move`, `take_move_receipts`, `take_samples`); the
  `MovementCodec`/`movement_v4` codec and every `crpg-net` type.
- **Public shapes:** every existing public item of `crpg-net-quic` and
  `crpg-server`, including `Framing::V1`/`V2`, `Channel`, `SendError`,
  `ServerError`, `HostCall`, `DeliveryKind`, `QuicPumpReport`, `Invitation`
  (its existing fields), and every `Display` text.
- **Vendoring:** `0001-send-connection-close-when-congestion-blocked.patch`
  and its effect (case 28 must keep passing); the `[patch.crates-io]` table
  already in the root `Cargo.toml` (`Cargo.toml:33–42`) is not re-pointed.
- **Existing-test edits: none**, in either part.

## Dependency edges

No new edge, no new lockfile package, no `ALLOWED` change.

| Edge | Section | Part | `ALLOWED` change |
|---|---|---|---|
| (none) | — | — | — |

- `crpg-net-quic` keeps its one workspace edge `crpg-net` (`ALLOWED` row
  `{"crpg-net"}`, `tools/lint/deps.py:55–59`) and its four D23 pins
  (`crates/crpg-net-quic/AGENTS.md:73–83`). The lane-1 API reuses
  `crpg_net::movement_v4::MAX_MOVE_DOWN_BYTES` and
  `crpg_net::protocol::MAX_INTENT_FRAME_BYTES` by path, which the existing
  edge already permits; `movement_v4` is already `pub` in `crpg-net`
  (`crates/crpg-net/src/lib.rs:50`).
- `crpg-server` keeps its existing `crpg-net-quic` path edge
  (`crates/crpg-server/AGENTS.md:92–106`) and its `crpg-net` normal edge; the
  v4 route calls `host.bind_v4_peer`/`ingest_move`/`take_move_receipts`/
  `take_samples` (already in the crate) and the new `crpg-net-quic` methods
  through the existing edge. `FactGrants` and `PauseRights` are already
  reachable (`crates/crpg-server/src/host.rs`, `crpg_net::protocol_v3`).
- The dedup patch is a vendored-patch change under `[patch.crates-io]`
  (`Cargo.toml:33–42`), not a new dependency; `deny.toml` is untouched.
- `tools/lint/deps.py`, `determinism.py` and their self-tests are not edited.

## Out of scope

- **T026 prediction**, and any `crpg-godot` work: the bridge's move to
  `ReassemblyPoolV4`/`ReplicaWorldV4`/`MoveOutbox` is T151 (after T037);
  T049 interpolation (G1), T026 prediction (G2) and T050 click-to-move (G3)
  are separate tasks. This task only carries the bytes.
- **The nav revision on the wire** (T037 Reading 4; a separate `crpg-net`
  task before T026).
- **A `0x83` snapshot with positions** (T026p Reading 8); new rejection
  codes (T026p Reading 3); route length on the wire.
- **Any change to `crpg-net`'s codec or to the T037 host surface.**
  `movement_v4`, `outbox_v4`, `positions_v4`, `projection_v4`, `pool_v4`,
  `replica_v4` and the T037 `host.rs`/`host_v4.rs` items are frozen here;
  this task consumes them, it does not edit them.
- **v3-over-QUIC**: the `/3` ALPN is movement; binding a plain v3 session
  over QUIC (a `/2` hello naming version 3) is not T150 and stays unbound.
- **Resume on `/3`** (T025b): `bind_resumable` and the `0x91` channel are
  unchanged here; `QuicHost::start` still takes `bind`/`bind_dual`.
- **Running the impairment probe or vendoring in this draft**: this is a
  doc-only slot; the probe re-run and the patch application are T150a's
  implementer Definition of done, described above, not done here.
- **Any other crate or file**; goldens are not re-blessed.

## Open questions

Each with a recommendation; nothing below is approved.

1. **Datagram buffer sizes.** `DATAGRAM_SEND_BUFFER_BYTES = 65_536`
   (server) and `DATAGRAM_RECV_BUFFER_BYTES = 262_144` (client) are picked
   to bound memory under congestion while absorbing the ~27 KB/s sample rate
   (worst case 27 × 1,024 B/s, `tasks/T037.md:292`). *Recommend:* these
   values, as fixed constants (not limits); a later task may promote them to
   tightening-only limits if operators need to tune them.
2. **Where the connect-time datagram refusal surfaces.** A-3 closes with
   `ProtocolViolation` in the server's accept path, so the adapter's
   `on_hello` sees `ServerError::Closed` and reports `HelloOutcome::Lost`
   (not a `Refused` code). *Recommend:* this path (it reuses the existing
   `Lost` handling and needs no new `CloseCode`); the alternative — a
   pre-hello refusal with no `HelloReceived` — is equally sound but touches
   more of the server loop.
3. **The v4 facts/rights registration.** Reading 9 adds
   `register_v4_invitation` + `V4Invitation` so the existing `Invitation`
   and its struct-literal sites are untouched. *Recommend:* the separate
   registration; the alternative (fields on `Invitation`) forces an
   existing-test edit, which this contract's "What must not change" forbids.
4. **`bind_dual` ALPN list order.** `[crpg-lane0/3, crpg-lane0/2,
   crpg-lane0/1]` keeps `/1` and `/2` clients working and prefers `/3`.
   *Recommend:* that order; `bind` stays `/1`-only and `bind_resumable` is
   unchanged.
5. **No new `CloseCode` for the datagram refusal.** A-3 reuses
   `ProtocolViolation` (2) because the ALPN pins the datagram minimum.
   *Recommend:* reuse; adding a `MovementUnsupported` code is an ADR-level
   wire change this contract deliberately avoids.

## Agent log

- 2026-11-15 (UTC) · opencode-go/deepseek-v4-pro + T150 draft a · Drafted the lane-1 movement transport contract (the `crpg-net-quic` stream pair, DATAGRAM extension, D23 dedup patch and impairment probe, then the `crpg-server` `quic.rs` v4 route) from the fact sheet and the T026p/T037 contracts, so the architect can review and finalize it.
