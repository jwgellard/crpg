# T024q (draft): QUIC transfer channel — exact contract (beam)

Date: 2026-10-25 (UTC)
Status: **Proposed (draft by opencode-go/beam-free)** — blind contract draft for
the `crpg-net-quic` transfer channel (T024 S§16 Q3 / D35 follow-on). A Claude
architect will compare this with an independent draft and write the final
contract. This draft makes its own investigation from `tasks/T024q-facts.md`
and the cited code.

## S§0. Inputs read

- Root `AGENTS.md` (non-negotiables, platform/replay invariants).
- `tasks/T024q-facts.md` (fact sheet, all `path:line` citations below).
- `tasks/T024.md` S§10 (transport property), S§11 (sender rules, esp. item 3
  total order), S§15 (out of scope), S§16 Q3/Q5.
- `docs/adr/0024-separate-crpg-net-quic-crate.md`,
  `docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md`,
  `docs/adr/0026-quic-host-adapter.md`,
  `docs/adr/0028-snapshot-transfer-subprotocol.md`.
- `docs/architecture/crpg-net-quic.md`.
- `crates/crpg-net-quic/AGENTS.md`, `src/lib.rs`, `src/handshake.rs`,
  `src/server.rs`, `src/client.rs`, `src/queue.rs`, `src/tls.rs`,
  `src/close.rs`.
- `crates/crpg-net/src/protocol.rs`, `src/protocol_transfer.rs`,
  `src/transport.rs`.
- `tools/lint/deps.py` (ALLOWED row, IO_FREE_CRATES).

## S§1. Findings (measured / code)

- Single ALPN today: `ALPN_LANE0_V1 = b"crpg-lane0/1"` (`handshake.rs:11-12`);
  advertised by both TLS configs (`tls.rs:81,122`). Server accepts only it
  (`server.rs:685-690`). A client presenting `b"crpg-lane0/2"` is refused and
  counted in `refused_handshakes` (`quic.rs:692-708`, `server.rs:679-687`).
  No `crpg-lane0/2` constant exists (`facts §1, §8`).
- One client-initiated bidi stream per connection; server permits one bidi,
  zero uni, DATAGRAM off (ADR-0025 items 1, 8; `architecture:68,130`).
- Framing today is a 4-byte big-endian `u32` length + payload, caps checked
  before allocation (`FRAME_HEADER_BYTES = 4`, `handshake.rs:16-60`;
  `read_frame`, `queue.rs:608-625`). Per-direction caps:
  client→server 4,096 (`MAX_INTENT_FRAME_BYTES`, `protocol.rs:58`),
  server→client 65,536 (`MAX_DELTA_FRAME_BYTES`, `protocol.rs:64`).
  Enforced in `try_send` (`server.rs:361-362`, `client.rs:211-212`) and
  inbound readers (`server.rs:876-881`, `client.rs:433-438`).
- Transfer subprotocol: first byte `TRANSFER_VERSION = 0x81`, lane
  `LANE_SNAPSHOT = 2` (`protocol_transfer.rs:19,22`); downstream tags
  0x10/0x11/0x12, upstream 0x20/0x21 (`protocol_transfer.rs:25-33`).
  Caps: `MAX_UP_FRAME_BYTES = 4,096`, `MAX_DOWN_FRAME_BYTES = 65,536`
  (`protocol_transfer.rs:46-48`); a full chunk max 32,815 ≤ 65,536
  (`T024.md:121-122,155`). Transfer frames already fit the existing caps.
- No channel selector exists in the current framing; the header carries only
  the length (`facts §8`). No `crpg-net-quic` code references
  `protocol_transfer` (`facts §8`).
- Ordering is per connection per direction, exact `try_send` order
  (`AGENTS.md:50-51`). There is no cross-stream or cross-connection ordering
  (`facts §8`).
- `crpg-net-quic` `ALLOWED` row is `{"crpg-net"}` only; no dev-dependencies
  (`deps.py:59`; `AGENTS.md:71-73`). `crpg-core/data/rules/sim/net` are
  IO-free (`deps.py:81,252`).
- Close codes 0–9 are frozen (ADR-0025 item 7; `close.rs:10-70`). A new
  code needs a new ALPN.
- Host adapter: `QuicHost` (`crates/crpg-server/src/quic.rs`) and
  `Host::take_delivery`/`acknowledge_delivery` (`host.rs:1423,1436`). Delivery
  is one frame per lane payload; backpressure holds one frame
  (ADR-0026 items 5–6).
- T024 requires, for cutover correctness, that a peer's transfer and delta
  frames share one total order; otherwise the receiver refuses
  (`BehindReplica`) rather than corrupts (`T024.md:831-834,896-897`).

## S§2. Decisions this contract makes

1. **A new ALPN `crpg-lane0/2`**, negotiated alongside `crpg-lane0/1`, not a
   replacement. Both endpoints advertise both, preferring v2. A connection's
   framing mode is fixed by the negotiated ALPN for its lifetime.
2. **The current ALPN path stays byte-identical.** When `crpg-lane0/1` is
   negotiated, the frame header remains 4 bytes and no selector is added;
   `framed`/`read_frame` output is unchanged. This is a regression gate
   (S§9 item 7).
3. **One ordered stream, extended header.** Under v2 ALPN the frame header
   grows from 4 to 5 bytes by appending a 1-byte channel selector after the
   length prefix. No second stream, no DATAGRAM, no cross-stream barrier.
4. **Channel selector values** mirror the wire protocol identity they carry:
   `0x01` = lane-0 v1, `0x02` = lane-0 v2, `0x81` = transfer
   (`TRANSFER_VERSION`). Demultiplexing is by this byte, never by payload
   bytes (T024 S§4).
5. **Caps stay per direction, not per channel.** Every channel in a direction
   uses that direction's existing cap (4,096 client→server; 65,536
   server→client). Transfer already fits (`T024.md:121-122,155`). No new cap
   and no tightening-only limit is added in this crate.
6. **`try_send`/`try_recv` carry the channel explicitly.** The transport owns
   framing; the host owns policy. A frame sent on a channel the negotiated
   ALPN cannot carry is refused with `SendError::UnsupportedChannel`.
7. **No new dependency and no S-set touch.** `crpg-net-quic` already edges
   `crpg-net`, whose `protocol_transfer` module supplies the transfer
   constants. No manifest, lock, deny.toml or tools change.

## S§3. Files

New: none (no new module; constants/types live in existing modules).

Changed (all in `crates/crpg-net-quic` unless noted):
- `src/handshake.rs`: `ALPN_LANE0_V2`; `Channel` enum; `FrameError::UnknownChannel`;
  `channel_frame_header`/`parse_channel_frame_header`; `CHANNEL_SELECTOR_BYTES`,
  `CHANNEL_FRAME_HEADER_BYTES`.
- `src/queue.rs`: `framed_channel`, `read_channel_frame`.
- `src/tls.rs`: advertise both ALPNs (v2 preferred) in both configs.
- `src/server.rs`: `alpn_matches` accepts V1 or V2; `ServerEvent::HelloReceived`
  gains `alpn`; `try_send`/`try_recv` signatures; `alpn()` accessor.
- `src/client.rs`: `try_send`/`try_recv` signatures; `negotiated_alpn()`.
- `src/close.rs`: `SendError::UnsupportedChannel`.
- `src/lib.rs`: re-export new items.
- `docs/architecture/crpg-net-quic.md`, `crates/crpg-net-quic/AGENTS.md`:
  signed entries (S§12).
- Host adapter `crates/crpg-server/src/quic.rs` and `host.rs`: call-site
  updates and routing (S§8); these are `crpg-server` work, listed as
  required, not performed by this crate's edit.

Not changed: `Cargo.toml`, `Cargo.lock`, `deny.toml`, `tools/`, any other
crate, and the v1 `framed`/`read_frame`/`frame_header`/`parse_frame_header`
functions.

## S§4. ALPN: constant, negotiation, mismatch

### S§4.1 Constant

```
pub const ALPN_LANE0_V2: &[u8] = b"crpg-lane0/2";   // handshake.rs (new)
```

`ALPN_LANE0_V1` (`handshake.rs:11-12`) is unchanged.

### S§4.2 Advertisement (both endpoints)

`tls.rs:81` (server) and `tls.rs:122` (client) change from

```
tls.alpn_protocols = vec![ALPN_LANE0_V1.to_vec()];
```

to

```
tls.alpn_protocols = vec![ALPN_LANE0_V2.to_vec(), ALPN_LANE0_V1.to_vec()];
```

Preference order is v2 then v1 on both sides, so a peer supporting both gets
v2 and a v1-only peer gets v1. No other protocol version is advertised.
TLS 1.3 only, ring, no 0-RTT, no resumption, SNI `crpg.invalid` are
unchanged (ADR-0025 items 2, 5).

### S§4.3 Selection and per-connection mode

After the TLS handshake, the server derives the negotiated ALPN from
`connection.handshake_data()` (as `alpn_matches` does today,
`server.rs:685-690`) and stores it on the connection record. The client
reads `connection.handshake_data()` after connect. A helper maps the bytes to
a framing mode:

```
pub enum FramingVersion { V1, V2 }
impl FramingVersion { pub fn from_alpn(alpn: &[u8]) -> Option<Self>; }
```

- `crpg-lane0/1` → `V1` (4-byte header, no selector, lane-0 v1/v2 payloads).
- `crpg-lane0/2` → `V2` (5-byte header with channel selector).
- anything else → `None` (treated as mismatch).

The negotiated value never changes after the connection is accepted.

### S§4.4 Mismatch handling

- **No common ALPN:** TLS negotiation fails. The server increments
  `refused_handshakes` and never emits `HelloReceived`, exactly as today for
  `foreign_alpn_refused` (`server.rs:679-687`, `quic.rs:692-708`). The
  client's `connect` returns `ConnectError::Handshake` (existing variant;
  `client.rs:47`), since ALPN failure is a TLS handshake failure.
- **A v1-only client against a v2-capable server:** negotiates V1 and
  completes the handshake; the server sees `FramingVersion::V1`. No refusal.
- **A v2-only client against a v1-only server:** TLS fails; refused on the
  server side, `Handshake` on the client side. This is the expected behaviour
  for a peer that cannot carry transfer frames.
- **`alpn_matches`** (`server.rs:685-690`) becomes "protocol is
  `Some(ALPN_LANE0_V1)` or `Some(ALPN_LANE0_V2)`", not only V1.

### S§4.5 Options considered

- **Option A (recommended): advertise both, v2 preferred.** Keeps v1 peers
  connectable and byte-identical, while v2 peers gain the transfer channel.
- **Option B: advertise v2 only.** Breaks "current ALPN path byte-identical
  for peers on the current ALPN" (v1 peers could not connect at all).
- **Option C: version negotiation inside the v1 hello.** Rejected by
  ADR-0025 item 2: the ALPN pins the framing; a framing change needs a new
  ALPN, and the header size changes here.

## S§5. Channel selector on the existing ordered stream

### S§5.1 Values

```
pub enum Channel { Lane0V1 = 0x01, Lane0V2 = 0x02, Transfer = 0x81 }
```

`Channel::from_u8(raw: u8) -> Option<Self>` returns `None` for any other
byte. `Transfer` equals `TRANSFER_VERSION` (`protocol_transfer.rs:19`), so a
frame's selector is the same byte a transfer payload would have led with, but
it is now an explicit framing field rather than a payload guess (T024 S§4).

### S§5.2 Where it sits

Under V2 ALPN the frame header is 5 bytes:

```
[len: u32 BE][channel: u8]
```

i.e. the 4-byte length prefix is unchanged in position and meaning, and the
selector is appended immediately after it, before the payload. `cap` is
checked against `len` before the channel byte or any body is read, exactly as
today (`read_frame` checks `parse_frame_header` before `vec![0; len]`,
`queue.rs:612-623`).

Constants:

```
pub const CHANNEL_SELECTOR_BYTES: usize = 1;
pub const CHANNEL_FRAME_HEADER_BYTES: usize = 5; // 4 + 1
```

Under V1 ALPN the header stays 4 bytes and no selector is written or read.

### S§5.3 Caps per channel

Caps are per direction and apply to every channel in that direction:

| Channel | client → server | server → client |
|---|---|---|
| `Lane0V1` | 4,096 (`MAX_INTENT_FRAME_BYTES`, `protocol.rs:58`) | 65,536 (`MAX_DELTA_FRAME_BYTES`, `protocol.rs:64`) |
| `Lane0V2` | 4,096 | 65,536 |
| `Transfer` | 4,096 (`MAX_UP_FRAME_BYTES`, `protocol_transfer.rs:48`) | 65,536 (`MAX_DOWN_FRAME_BYTES`, `protocol_transfer.rs:46`) |

A length over the direction cap is `FrameTooLarge` (3) regardless of channel;
an unknown selector is `ProtocolViolation` (2). No channel-specific cap is
added, because T024's transfer frames already fit these caps
(`T024.md:121-122,155`) and a second, transport-level cap would duplicate
the wire caps in `protocol_transfer`.

### S§5.4 Why one ordered stream keeps the single total order

ADR-0025 item 1 pins one client-initiated bidi stream and zero uni streams;
T024 S§11 item 3 requires that, in the peer's single total order, a `Begin`
precedes every delta above its cursor and a transfer's chunks follow its
`Begin`. QUIC delivers bytes on one stream in `try_send` order, exactly once
(`AGENTS.md:50-51`; `ordered_exactly_once_through_impaired_relay`,
`quic.rs:1199`). Adding a selector byte to the header does not create a
second ordering domain: every frame, whatever its channel, is sequenced by
the same stream offset and arrives in send order. The host stages transfer
and delta frames through the same outbound queue, so their relative order is
the order the host enqueued them. Separate streams (or DATAGRAMs) would each
have an independent order and would need a cross-stream barrier to recover
the total order T024's cutover gate assumes; that is exactly what
S§16 Q3 rejects (`T024.md:1192-1195`). The selector therefore carries
*identity* (which codec to use, per T024 S§4) while the stream carries
*order*.

## S§6. Read/write function signatures

### S§6.1 Framing functions (crate-private, `queue.rs`/`handshake.rs`)

Existing (unchanged signatures):

```
pub const FRAME_HEADER_BYTES: usize = 4;                 // handshake.rs:16
pub fn frame_header(len: usize, cap: usize) -> Result<[u8; 4], FrameError>; // handshake.rs:36
pub fn parse_frame_header(header: [u8; 4], cap: usize) -> Result<usize, FrameError>; // handshake.rs:48
pub(crate) fn framed(payload: &[u8], cap: usize) -> Result<Vec<u8>, FrameError>; // queue.rs:63
pub(crate) async fn read_frame(recv: &mut RecvStream, cap: usize)
    -> Result<Option<Vec<u8>>, ReadFail>;                // queue.rs:608
```

New (V2 only):

```
pub const CHANNEL_SELECTOR_BYTES: usize = 1;             // handshake.rs (new)
pub const CHANNEL_FRAME_HEADER_BYTES: usize = 5;         // handshake.rs (new)
pub fn channel_frame_header(len: usize, channel: Channel, cap: usize)
    -> Result<[u8; 5], FrameError>;                     // handshake.rs (new)
pub fn parse_channel_frame_header(header: [u8; 5], cap: usize)
    -> Result<(usize, Channel), FrameError>;             // handshake.rs (new)
pub(crate) fn framed_channel(payload: &[u8], channel: Channel, cap: usize)
    -> Result<Vec<u8>, FrameError>;                     // queue.rs (new)
pub(crate) async fn read_channel_frame(recv: &mut RecvStream, cap: usize)
    -> Result<Option<(Channel, Vec<u8>)>, ReadFail>;    // queue.rs (new)
```

Precedence in `parse_channel_frame_header`: cap check on `len` first
(`TooLarge`), then `Empty` for zero, then unknown selector
(`UnknownChannel`). Body allocation happens only after both checks pass.

### S§6.2 Public entry points

Server (`server.rs`):

```
pub fn try_send(&mut self, conn: ConnectionId, channel: Channel,
                frame: &[u8]) -> Result<(), SendError>;   // was (conn, frame)
pub fn try_recv(&mut self, conn: ConnectionId)
    -> Result<Option<(Channel, Vec<u8>)>, ServerError>;   // was Option<Vec<u8>>
pub fn alpn(&self, conn: ConnectionId) -> Option<&'static [u8]>; // new
```

Client (`client.rs`):

```
pub fn try_send(&mut self, channel: Channel, frame: &[u8])
    -> Result<(), SendError>;                            // was (frame)
pub fn try_recv(&mut self)
    -> Result<Option<(Channel, Vec<u8>)>, CloseReason>;  // was Option<Vec<u8>>
pub fn negotiated_alpn(&self) -> Option<&'static [u8]>;   // new
```

`ServerEvent::HelloReceived` gains a field:

```
HelloReceived {
    conn: ConnectionId,
    remote: SocketAddr,
    hello: Hello,
    alpn: &'static [u8],   // new
}
```

Host adapter reads `alpn` to learn the framing mode at accept time.

### S§6.3 Precedence (server `try_send`)

`UnknownConnection` → `NotAccepted` → `Closed` → `UnsupportedChannel`
(channel not allowed under the negotiated ALPN) → `EmptyFrame` →
`FrameTooLarge` → `QueueFull`. The channel is validated before the empty/cap
checks, so an unsupported channel is refused without inspecting the frame.

Client `try_send`: `Closed` → `UnsupportedChannel` → `EmptyFrame` →
`FrameTooLarge` → `QueueFull`.

## S§7. Closed error enum and close-code mapping

### S§7.1 `FrameError` (`handshake.rs:26-33`)

Add one variant:

```
pub enum FrameError { Empty, TooLarge, UnknownChannel }
```

Display: `UnknownChannel at quic/frame`.

### S§7.2 `SendError` (`close.rs:104-119`)

Add one variant:

```
pub enum SendError {
    UnknownConnection, NotAccepted, Closed(CloseReason), EmptyFrame,
    FrameTooLarge, QueueFull, UnsupportedChannel,   // new
}
```

Display: `UnsupportedChannel at quic/send`.

### S§7.3 Mapping: framing/protocol error → connection close code

Applied by the reader via `local_close` (`queue.rs:637-647`), which records
first and then calls quinn's `close` (trap: record before close,
`AGENTS.md:118-119`):

| Source | Close code | Where |
|---|---|---|
| `FrameError::Empty` (zero-length frame) | `ProtocolViolation` (2) | `read_frame` maps `Empty` → `ReadFail::Code(ProtocolViolation)` (`queue.rs:619`) |
| `FrameError::TooLarge` (over direction cap) | `FrameTooLarge` (3) | `queue.rs:620` |
| `FrameError::UnknownChannel` (bad selector) | `ProtocolViolation` (2) | `read_channel_frame` (new) |
| FIN mid-frame / `RESET_STREAM` / `STOP_SENDING` on the lane | `ProtocolViolation` (2) | `read_fail` (`queue.rs:627-633`) |
| ALPN mismatch (no common ALPN) | no lane close code; `refused_handshakes++` (server) / `ConnectError::Handshake` (client) | `server.rs:679-687`, `client.rs:47` |
| idle timeout | `CloseReason::IdleTimeout` | `reason_from` (`queue.rs:77`) |
| transport / crypto error | `CloseReason::Transport` | `queue.rs:82-85` |
| peer stateless reset / connection lost | `CloseReason::Reset` | `queue.rs:78` |
| I/O thread ended unexpectedly | `CloseReason::EndpointFailed` | `server.rs:166-168` |

Payload codec errors (`CodecError` from `crpg-net`, including its
`FrameTooLarge`/`UnsupportedVersion`/`WrongDirection`/`UnknownMessage`/
`Malformed`/`LimitExceeded`) are **not** decoded by this crate (invariant 4,
`AGENTS.md:44-46`). If the host closes because of one, it should use
`CloseCode::ProtocolViolation` (2) for malformed/unknown/limit/version errors
and `CloseCode::FrameTooLarge` (3) for an oversized encoded frame; this crate
provides only the framing. `SessionFenced` (9) and the other host-level codes
remain the host's per ADR-0026.

## S§8. Host-adapter (ADR-0026) changes required

These are `crpg-server` changes; they are listed here as the contract's
input to T024h. No host policy moves into `crpg-net-quic`.

1. **Call-site updates for the new signatures.** `QuicHost` (`quic.rs`) and
   any test client call `QuicServer::try_send(conn, channel, frame)` and
   `QuicClient::try_send(channel, frame)` instead of the old single-argument
   form. The held frame in the one-frame backpressure buffer
   (ADR-0026 item 6) stores its `Channel` and is retried with the same
   channel.
2. **Receive routing by channel, never by payload.** Where `QuicHost` calls
   `try_recv` and today treats the bytes as a lane-0 frame, it now receives
   `(Channel, Vec<u8>)` and dispatches:
   - `Channel::Lane0V1` → `crpg_net::codec::decode_intent`/`decode_delta`
     (v1);
   - `Channel::Lane0V2` → `crpg_net::codec_v2::decode_intent`/`decode_delta`
     (v2);
   - `Channel::Transfer` → `crpg_net::codec_transfer::decode_down`/
     `decode_up`.
   This is the demultiplexing point T024 S§4 requires; no decoder inspects
   bytes to guess.
3. **ALPN gating at accept time.** `QuicHost` reads `alpn` from
   `ServerEvent::HelloReceived` (or calls `QuicServer::alpn(conn)`) to set a
   per-binding framing mode. Transfer frames are staged only to bindings whose
   ALPN is `crpg-lane0/2`; a v1-only peer never receives a `Channel::Transfer`
   frame (and `try_send(..., Transfer, ...)` would return
   `UnsupportedChannel` anyway).
4. **Negotiated ALPN on the client side.** A client that needs transfer
   frames checks `QuicClient::negotiated_alpn() == Some(b"crpg-lane0/2")`
   before using `Channel::Transfer`; otherwise it falls back to deltas.
5. **Stats/totals unchanged in shape.** Frames are counted by the existing
   queue accounting regardless of channel; no new counter is required in this
   crate. If per-channel counters are wanted, that is `crpg-server` policy.

## S§9. Numbered test list

All in `crates/crpg-net-quic/tests/quic.rs` (existing 30-case suite) unless
noted; each refusal has a positive control and leaves queues/state unchanged
where applicable. New cases are appended (31+), not renumbering existing ones.

1. `alpn_lane0_v2_constant_and_advertisement`: `ALPN_LANE0_V2 == b"crpg-lane0/2"`;
   server and client configs advertise `[v2, v1]` in that order; `ALPN_LANE0_V1`
   unchanged; `frame_and_handshake_bytes_are_exact` (case 1) still passes.
2. `alpn_v1_peer_still_connects`: a client advertising only
   `crpg-lane0/1` against a v2-capable server negotiates V1 and reaches
   `HelloReceived`; `alpn == b"crpg-lane0/1"`.
3. `alpn_mismatch_refused`: a client advertising only an unknown ALPN is
   refused (`refused_handshakes == 1`, no `HelloReceived`); a control client
   with a supported ALPN reaches `HelloReceived` (extends
   `foreign_alpn_refused`, `quic.rs:692`).
4. `channel_selector_round_trip`: for each `Channel` variant, `framed_channel`
   output parses back through `parse_channel_frame_header` with the same
   channel and length; `read_channel_frame` round-trips all three over IPv4
   loopback.
5. `channel_selector_unknown_rejected`: a header whose 5th byte is not in
   `{0x01, 0x02, 0x81}` yields `FrameError::UnknownChannel`, which
   `read_channel_frame` maps to `ReadFail::Code(ProtocolViolation)` (2); the
   connection closes with `ProtocolViolation` and queued frames are unchanged.
6. `oversize_per_channel`: for each channel, a frame one byte over the
   direction cap (4,097 client→server; 65,537 server→client) returns
   `SendError::FrameTooLarge` and leaves outbound queues unchanged; a raw
   over-cap header closes with `FrameTooLarge` (3). Control: a frame exactly
   at the cap is accepted.
7. `interleaved_transfer_and_delta_order_preserved`: server sends a sequence
   mixing `Channel::Transfer` and `Channel::Lane0V2` frames; the client
   receives them in the same order with the same channel tags. Proves the
   single total order across channels.
8. `unsupported_channel_on_v1_connection`: on a V1-negotiated connection,
   `try_send(conn, Channel::Transfer, ...)` returns
   `SendError::UnsupportedChannel`; `try_send(..., Channel::Lane0V1, ...)` is
   accepted (positive control).
9. `v1_alpn_path_byte_identical` (regression): under V1 ALPN, `framed` output
   and `read_frame` bytes are byte-identical to the pre-change output (4-byte
   header, no selector); asserted against the existing
   `frame_and_handshake_bytes_are_exact` golden (`quic.rs:72`) and a captured
   baseline. No selector byte appears.

## S§10. Boundary: left to T024h, T025a, T025b

Left to **T024h** (`crpg-server`, host sender rules and seamless narrowing):
- Allocating strictly increasing per-epoch transfer ids and capturing snapshot
  bodies from the host view (T024 S§11 items 1, 6).
- Staging transfer frames in the host egress log and handing them to
  `QuicServer::try_send(conn, Channel::Transfer, frame)` in canonical order
  (S§5.4, S§8.1).
- Holding post-cutover deltas and releasing them through the FIFO queue after
  publication; never resetting `event_seq` within an epoch
  (T024 S§10.7, S§11 item 2).
- Handling `TransferUp::Published`/`Resync` and sending
  `TransferDown::Cancel` on revocation; making narrowing seamless in place of
  `SessionFenced` (T024 S§10.9, S§11 items 4–5, 7–8).
- Per-binding ALPN gating from S§8.3.

Left to **T025a** (`crpg-net`, reconnect/resync primitives) and
**T025b** (`crpg-server`, grace): proof-bearing resume, fencing generations,
installing a snapshot across connections, 30-second same-process grace. Both
need T024q and T024h first (T024 S§16 Q5). T024q provides only the transport
channel; it does not define resume or host wiring.

Left to **T024** (already approved): the transfer wire codec and reassembly
pool are unchanged by this task; T024q only carries their frames.

## S§11. S-set, dependency and invariant impact

- **S-set:** no. Nothing here touches `World`, tick order, schema versions or
  migrations in `crpg-data`, `crpg-contracts` signatures, or the GDExtension
  boundary. The channel selector is a transport envelope byte, not simulation
  state.
- **Dependency:** none added. `crpg-net-quic` already edges `crpg-net`
  (`deps.py:59`), which exposes `protocol_transfer` (`protocol_transfer.rs`).
  No manifest, lock, `deny.toml` or `tools/` change. `cargo tree -p crpg-net`
  still names no tokio/quinn/rustls/ring/mio/socket2.
- **Invariants:** no `unsafe`, no `HashMap`/`HashSet`, no `f32`/`f64`, no
  platform `cfg`, no new thread or runtime. Replay determinism is unaffected
  (transport bytes are not part of `state_hash` or any golden). The V1 ALPN
  path is byte-identical (S§9 item 9), so existing baselines and goldens
  remain valid without re-baseline.

## S§12. Docs

- `docs/architecture/crpg-net-quic.md`: add a "Transfer channel (T024q)"
  section naming the V2 ALPN, the 5-byte header, the `Channel` values, the
  per-direction caps, and the total-order rationale; signed entry.
- `crates/crpg-net-quic/AGENTS.md`: add `ALPN_LANE0_V2`, `Channel`,
  `CHANNEL_FRAME_HEADER_BYTES` to the public surface; add the total-order and
  no-payload-inspection invariants; signed entry.
- `docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md` may gain an
  addendum noting that `crpg-lane0/2` carries the channel selector, or a
  new ADR may be filed (S§14 question 7). No ADR is filed by this draft.

## S§13. Definition of done

- [ ] This contract approved (H-contract).
- [ ] Exactly the S§3 files changed in `crates/crpg-net-quic`; host-adapter
      call sites updated in `crates/crpg-server` (S§8).
- [ ] `cargo fmt --all` and `cargo fmt --all -- --check` clean.
- [ ] `cargo clippy -p crpg-net-quic --all-targets --locked -- -D warnings` clean.
- [ ] `cargo test -p crpg-net-quic --test quic --locked` passes with the
      existing 30 cases plus the 9 new cases of S§9.
- [ ] `cargo test -p crpg-net --locked` unchanged (119 passed).
- [ ] `cargo tree -p crpg-net -e normal,build --target all` names no
      tokio/quinn/rustls/ring/mio/socket2.
- [ ] `python tools/lint/deps.py`, `python tools/lint/determinism.py`,
      `python -m unittest discover -s tools/lint -p "test_*.py"` pass.
- [ ] `cargo deny check` and `git diff --check` clean.
- [ ] Native Windows/MSVC (CI) and genuine Linux/GNU runs recorded.
- [ ] Completion record appended to `tasks/T024q.md` (to be created) and the
      BACKLOG row updated.

## S§14. Open questions for the user

1. **ALPN advertisement and fallback.** Option A (recommended): advertise
   both `crpg-lane0/2` and `crpg-lane0/1` on both endpoints, preferring v2,
   so v1-only peers still connect and keep the byte-identical path. Option B:
   advertise v2 only, which breaks v1 peers. Option C: negotiate the version
   inside the v1 hello, which ADR-0025 item 2 forbids because the framing
   changes. *Recommendation: Option A.*

2. **Placement and size of the channel selector.** Option A (recommended): a
   5-byte header under V2 ALPN — the existing 4-byte length prefix followed
   by one selector byte — checked cap-first before any allocation. Option B:
   put the selector as the first payload byte, which is "inspecting payload
   bytes" and violates T024 S§4's channel-selection rule. Option C: a 2-byte
   selector for future-proofing, which wastes a byte on a stream shared with
   deltas and has no current use. *Recommendation: Option A.*

3. **Selector values.** Option A (recommended): `0x01` lane-0 v1, `0x02`
   lane-0 v2, `0x81` transfer, mirroring the protocol version / first byte
   each channel carries. Option B: `0x00`/`0x01`/`0x02`, which collides
   conceptually with lane bytes and makes a transfer frame's selector differ
   from its `TRANSFER_VERSION`. *Recommendation: Option A.*

4. **Caps per channel.** Option A (recommended): the existing per-direction
   caps (4,096 / 65,536) apply to every channel; transfer already fits and no
   new cap or limit is added. Option B: transport-enforced per-channel caps
   (e.g. 32,815 downstream for transfer) that duplicate `protocol_transfer`'s
   wire caps and can drift from them. *Recommendation: Option A.*

5. **`try_send`/`try_recv` shape.** Option A (recommended): add a `Channel`
   parameter to `try_send` on both endpoints and return `(Channel, Vec<u8>)`
   from `try_recv`, so framing and channel identity stay in this crate.
   Option B: keep opaque byte frames and have the host prepend the selector,
   which leaks framing into the host and breaks the "bytes are opaque" rule.
   Option C: separate methods `try_send_lane`/`try_send_transfer`, which
   duplicate the queue path and the backpressure buffer. *Recommendation:
   Option A.*

6. **Exposing the negotiated ALPN to the host.** Option A (recommended): add
   `alpn: &'static [u8]` to `ServerEvent::HelloReceived` plus
   `QuicServer::alpn(conn)` and `QuicClient::negotiated_alpn()`, so the host
   gates transfer frames at accept time. Option B: let the host infer support
   from the first received `Channel`, which is too late for send-side gating.
   *Recommendation: Option A.*

7. **ALPN record.** Option A (recommended): append an addendum to
   ADR-0025 noting that `crpg-lane0/2` carries the channel selector and that
   ADR-0025 item 2's "new ALPN for framing changes" is satisfied. Option B:
   file a new ADR (ADR-0029) dedicated to the transfer channel. Either is
   acceptable; this draft leans to an addendum to keep the wire/trust record
   in one place. *Recommendation: Option A (addendum), unless the architect
   prefers a standalone ADR.*

8. **Unsupported channel on a V1 connection.** Option A (recommended):
   `try_send(..., Channel::Transfer, ...)` on a V1-negotiated connection
   returns `SendError::UnsupportedChannel` (fail closed). Option B: silently
   drop the frame, which hides a host bug. *Recommendation: Option A.*

## Agent log

- 2026-10-25 (UTC) · opencode-go/beam-free + T024q draft · Drafted the exact contract for the `crpg-net-quic` transfer channel: V2 ALPN negotiated alongside V1 with byte-identical V1 path, 5-byte header with a 3-valued channel selector, per-direction caps, explicit channel in try_send/try_recv, host-adapter routing requirements, 9 numbered tests, and boundary list for T024h/T025a/T025b. No S-set touch, no new dependency.