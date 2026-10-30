# T024q — blind contract draft (muse): transfer channel in `crpg-net-quic`

Status: **Proposed blind draft, not approved.** One of two independent
drafts for the T024q contract (T024 S§16 Q3 follow-on). An architect will
compare both and write the final contract. It approves nothing, changes no
code, and adds no dependency. H-contract approval of the final contract is
still required before any implementation.

One crate: `crpg-net-quic`. No other crate is touched.

## S§0. Inputs read

- Root `AGENTS.md`; `tasks/T024q-facts.md` (every `path:line` below on this
  branch exists there unless stated as read for this draft).
- `tasks/T024.md`: S§10 (receiver, cutover), S§11 (sender rules),
  S§16 Q3/Q5 (ordered-stream recommendation, follow-ons), approval D35.
- `docs/adr/0024-separate-crpg-net-quic-crate.md` (placement, `ALLOWED`
  `{"crpg-net"}`); `docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md`
  (wire/trust pinning); `docs/adr/0026-quic-host-adapter.md` (adapter
  decisions); `docs/adr/0027-vendored-crates-io-patches.md` (vendoring
  non-scope); `docs/adr/0028-snapshot-transfer-subprotocol.md` (S§11 host
  requirements pointer).
- `docs/architecture/crpg-net-quic.md`; `crates/crpg-net-quic/AGENTS.md`.
- `crates/crpg-net-quic/src/` (`handshake.rs`, `close.rs`, `queue.rs`,
  `server.rs`, `client.rs`, `tls.rs`, `limits.rs`, `lib.rs`) and `tests/`;
  `crates/crpg-net/src/protocol_transfer.rs`.
- The other lab's draft was not sought or read.

## S§1. Findings (measured, all cited)

1. One ALPN constant exists:
   `ALPN_LANE0_V1 = b"crpg-lane0/1"` (`crates/crpg-net-quic/src/handshake.rs:11-12`).
   No `crpg-lane0/2` constant exists (facts §8).
2. Both endpoints advertise only that ALPN:
   `crates/crpg-net-quic/src/tls.rs:81` (server) and `:122` (client).
   TLS 1.3 only, ring provider, no 0-RTT/resumption/client-cert, SNI
   `crpg.invalid` (facts §1; `handshake.rs:14`, `tls.rs:65,103`).
3. Server ALPN check `alpn_matches` compares `data.protocol` to
   `Some(ALPN_LANE0_V1)` (`crates/crpg-net-quic/src/server.rs:689`).
   ALPN failures never produce `HelloReceived`; `end_handshake` removes the
   record, decrements `live`, increments `refused_handshakes`
   (`crates/crpg-net-quic/src/server.rs:679-687`).
4. `foreign_alpn_refused` (`crates/crpg-net-quic/tests/quic.rs:692-708`)
   connects with `b"crpg-lane0/2"` and asserts refusal plus
   `refused_handshakes == 1`, with a right-ALPN control reaching
   `HelloReceived`.
5. Framing is 4-byte big-endian `u32` length + payload:
   `FRAME_HEADER_BYTES = 4` (`handshake.rs:16`), `frame_header(len, cap)`
   (`handshake.rs:36`), `parse_frame_header(header, cap)`
   (`handshake.rs:48`), `framed(payload, cap)` (`queue.rs:63`),
   `read_frame(recv, cap)` (`queue.rs:608`).
6. Caps are checked on the length prefix before allocation:
   `parse_frame_header` rejects zero (`FrameError::Empty`) and over-cap
   (`FrameError::TooLarge`) (`handshake.rs:48-60`); `read_frame` maps them
   to `CloseCode::ProtocolViolation` / `FrameTooLarge` (`queue.rs:618-621`).
7. Direction caps: client→server 4,096 `MAX_INTENT_FRAME_BYTES`, server→client
   65,536 `MAX_DELTA_FRAME_BYTES` (facts §2); enforced in
   `QuicServer::try_send` (`server.rs:361-362`), `QuicClient::try_send`
   (`client.rs:211-212`), server inbound cap (`server.rs:876-881`), client
   inbound cap (`client.rs:433-438`). Send precedence is documented on
   `try_send` (`server.rs:343-345`, `client.rs:199-200`); recv precedence on
   `try_recv` (`server.rs:316-318`, `client.rs:217`).
8. One client-initiated bidi stream per connection; zero uni; no DATAGRAM
   (ADR-0025 items 1, 8; arch doc `docs/architecture/crpg-net-quic.md:68,130`).
   Hello cap 130 (`handshake.rs:22`), welcome exactly 17 (`handshake.rs:24`).
   Zero-length is invalid (`handshake.rs:37-38`).
9. Ordering: within a connection and direction, frames deliver exactly once
   in `try_send` order (crate AGENTS.md invariant 6). No cross-connection
   order exists (facts §8).
10. Queues are bounded per connection and host-wide; `QueueFull` is
    retryable and changes nothing; a full inbound queue parks one frame and
    stops reading (crate AGENTS.md; `docs/architecture/crpg-net-quic.md:121-123`).
11. Public byte-movers: `QuicServer::try_recv` (`server.rs:319`),
    `QuicServer::try_send` (`server.rs:346`), `QuicClient::try_recv`
    (`client.rs:218`), `QuicClient::try_send` (`client.rs:201`),
    `QuicServer::lane` (`server.rs:462`). Transport trait
    (`crates/crpg-net/src/transport.rs:14-23`) is implemented via these
    (facts §2).
12. Close codes 0–9 (`close.rs:10-70`); `from_u64` returns `None` outside
    0..=9, peer codes kept raw (`close.rs:41-57`). `SendError`,
    `ServerError`, `ConnectError`, `FrameError`, `ReadFail`, `CloseReason`
    as in facts §4 (`close.rs:84-119`, `server.rs:66-79`,
    `client.rs:36-62`, `handshake.rs:27-33`, `queue.rs:598-603`).
    Reader closes via `local_close`, which `record_close`s first and only
    then calls quinn's `close` (`queue.rs:639-648`); first-wins keeps the
    first code.
13. Transfer wire (transport-relevant): first byte `TRANSFER_VERSION = 0x81`
    (`protocol_transfer.rs:19`), lane `LANE_SNAPSHOT = 2`
    (`protocol_transfer.rs:22`); caps `MAX_SEGMENT_BYTES = 4096`
    (`protocol_transfer.rs:40`), `MAX_CHUNK_BYTES = 32768`
    (`protocol_transfer.rs:44`), `MAX_DOWN_FRAME_BYTES = 65536`
    (`protocol_transfer.rs:46`), `MAX_UP_FRAME_BYTES = 4096`
    (`protocol_transfer.rs:48`), maxima 30/29/30 upstream (facts §3).
    Transfer frames fit existing direction caps: full chunk 32,815 ≤ 65,536
    downstream; upstream maxima 30 ≤ 4,096 (facts §3).
14. No selector exists today: the header carries only a 4-byte length
    (`handshake.rs:16-60`); payloads are opaque and never decoded here
    (crate AGENTS.md). No code references `protocol_transfer`,
    `TRANSFER_VERSION`, or transfer tags in this crate (facts §8).
15. T024 S§4: demultiplexing happens at transport/channel selection; no
    decoder guesses or falls back (`tasks/T024.md:288-291`). S§16 Q3
    (approved D35): carry transfer frames on the existing ordered stream
    with an explicit channel selector in the framing, not by inspecting
    payload bytes, under ALPN `crpg-lane0/2`; separate streams would need a
    cross-stream barrier (`tasks/T024.md:1192-1195`). S§11 item 3: in the
    peer's single total order a `Begin` precedes every delta above its
    cursor and chunks follow their `Begin` (`tasks/T024.md:896-897`).
    S§10.7: cutover correctness needs that shared total order; without it
    the receiver refuses (`BehindReplica`) rather than corrupts
    (`tasks/T024.md:831-834`).
16. ADR-0025: ALPN `crpg-lane0/1` pins framing, hello/welcome, stream mapping
    and close codes; any change needs a new ALPN (`docs/adr/0025-...:31-36`);
    privileged control streams need a new ALPN plus reviewed design
    (`docs/adr/0025-...:86-88`); change rule restated at `:132-134`.

## S§2. Decisions this contract makes

1. A new ALPN `crpg-lane0/2` versions the framing; dual-stack server,
   caller-selected client, no silent downgrade (S§4).
2. One ordered bidi stream is kept. A 1-byte channel octet opens every
   framed payload under the new ALPN only; values `0x01`/`0x02`/`0x81`;
   per-channel caps equal the direction caps (S§5).
3. Explicit `Channel` type threads through new `try_send`/`try_recv`
   overloads; the old signatures stay byte-identical on the old ALPN. One
   closed error enum gains `UnknownChannel`; every protocol error maps to
   a fixed close code (S§6, S§8).
4. The crate stays policy-free: it routes opaque tagged bytes and enforces
   caps/order/close only. All grant, id, cursor, deadline and publication
   policy stays in `crpg-server`/T024h (S§9).
5. A 10-case suite proves the channel, the order property, the close
   mapping, and the byte-identical legacy path (S§12).
6. T024h owns sender rules and seamless narrowing; T025a owns resume/resync;
   T025b owns grace/fencing. This contract owns only opaque transport
   (S§15).

## S§3. Files

New: none in this draft task (draft only).

Changed by the implementation task (not this task):
- `crates/crpg-net-quic/src/handshake.rs` (ALPN constant, channel octet,
  per-channel caps helpers), `src/queue.rs` (v2 reader/writer),
  `src/server.rs` + `src/client.rs` + `src/tls.rs` (negotiation, entry
  points), `src/close.rs` (error enum addition only if chosen),
  `src/limits.rs` (no new limits; documents that windows are unchanged),
  `src/lib.rs` (explicit re-exports, no glob), `tests/quic.rs` (S§12
  cases; existing 30 cases untouched), `crates/crpg-net-quic/AGENTS.md`
  (surface, invariants, traps), `docs/architecture/crpg-net-quic.md`
  (channel section), one ADR addendum under ADR-0025 (new ALPN record).

Not changed: `Cargo.toml`, `Cargo.lock`, `deny.toml`, `tools/`,
  `third_party/`, every `crpg-net` module, `crates/crpg-server/*`,
  hello/welcome layout, close-code numbers/names, v1 limits, window
  values.

## S§4. ALPN (item 1)

Pin: `pub const ALPN_LANE0_V2: &[u8] = b"crpg-lane0/2";` beside
`ALPN_LANE0_V1` (`handshake.rs:11-12`). It versions exactly: the 1-byte
channel octet, its value set, per-channel caps, and the new entry points.
Hello/welcome layout, stream mapping (one bidi, no uni, no DATAGRAM),
close codes 0–9, TLS profile, and v1 limits are unchanged.

- Option A (recommended): dual-stack server, caller-selected client.
  Server `tls.alpn_protocols` becomes `[ALPN_LANE0_V2, ALPN_LANE0_V1]`
  (extends `tls.rs:81`); client `connect` takes an `Alpn` selector
  (`V1` default-preserving vs `V2`), offering exactly one ALPN
  (extends `tls.rs:122`). Negotiated ALPN is recorded per connection and
  selects the framing reader/writer for that connection's lifetime.
  Consequence: old client (offers only `/1`, cf. `tls.rs:122`) to new
  server negotiates `/1` and runs the legacy path byte-identically (S§5.6);
  new client asking `/2` to an old server (advertises only `/1` per
  `tls.rs:81`) fails TLS with no overlap → client `ConnectError::Handshake`
  (`client.rs:48`), server counts nothing (no record exists to refuse).
  New client asking `/1` to a new server also runs legacy. No silent
  downgrade: a `/2` ask never yields `/1` bytes.
- Option B: dual-offer client (offers `[V2, V1]`, prefers V2). Consequence:
  fewer handshake failures, but the client must then handle either framing
  on return, the negotiated version becomes a runtime branch at every call
  site, and tests must cover the fallback matrix. More convenient, less
  explicit; downgrade risk if a caller ignores the negotiated value.
- Option C: flag day (only `/2` on both sides). Consequence: simplest code,
  but breaks every deployed `/1` peer and contradicts the byte-identical
  legacy requirement. Rejected.

Negotiation with current-ALPN-only peers (under A):
- old→new: negotiates `/1`; server runs `read_frame` with direction caps
  exactly as today (`queue.rs:608`, `server.rs:876-881`); channel octet
  absent; `frame_header`/`parse_frame_header` (`handshake.rs:36,48`)
  unchanged.
- new(`/2`)→old: TLS no-overlap failure. Client reports
  `ConnectError::Handshake` (`client.rs:48`); the old server records
  nothing (its `refused_handshakes` path at `server.rs:679-687` only covers
  connections that reached `end_handshake`, which never happens without a
  negotiated ALPN). No `HelloReceived`, no lane event either side.
- new(`/1`)→new: identical to old→old, byte for byte.

Mismatch handling: a server record whose negotiated ALPN is `/1` but that
receives a channel octet, or `/2` but receives a bare payload, is a
framing violation → `ProtocolViolation` (2) via `local_close`
(`queue.rs:639-648`), first-wins. A `/2` hello with unknown wire version
keeps today's hello-failure path (codes 2/3/4/6 per ADR-0025 item 3 and
arch doc handshake §); ALPN failure itself stays pre-hello
(`server.rs:679-689`).

*Recommendation:* approve Option A. It is the only option that keeps the
legacy path byte-identical, makes version selection explicit at `connect`,
and confines the new framing to connections that negotiated it.

## S§5. Channel selector, framing, caps, ordering (item 2)

Pin: under `/2` only, every lane frame payload is `channel-octet ||
inner-payload`, where the 4-byte length prefix covers the octet plus the
inner payload. The octet is logically the channel header and physically
the first payload byte, so `frame_header`/`parse_frame_header`
(`handshake.rs:36-60`) and `read_frame` (`queue.rs:608-625`) keep their
signatures; the v2 reader consumes one more byte after the length check
and before delivery.

- Where it sits — Option A (recommended): first byte of the framed
  payload (length covers it). Consequence: header parser untouched;
  allocation discipline unchanged (length checked against the direction
  cap before any read of octet or body, per `queue.rs:605-607`); legacy
  `/1` bytes unchanged. Cost: one payload byte per frame under `/2`.
- Option B: 5-byte wire header (4-byte length + 1-byte selector outside
  the length). Consequence: every header call site and the
  `FRAME_HEADER_BYTES` constant (`handshake.rs:16`) change; the exact-bytes
  test (`tests/quic.rs:72`) and the relay's frame splitter must learn two
  header widths; off-by-one risk at every boundary. Rejected: larger diff
  for no ordering gain.

Values (same under A or B):
- `0x01` lane-0 v1 inner bytes (wire version 1 payload, opaque here);
- `0x02` lane-0 v2 inner bytes (wire version 2 payload, opaque here);
- `0x81` transfer frame bytes (`TRANSFER_VERSION`,
  `protocol_transfer.rs:19`; `LANE_SNAPSHOT = 2` is inside the opaque
  inner payload, never inspected here).
Any other octet is `UnknownChannel`. The octet duplicates the inner first
byte by value but is never inferred from it: the reader branches on the
octet only (T024 S§4 no-guess rule, `tasks/T024.md:288-291`), then hands
the inner bytes to the caller undisclosed. Rationale for reusing the
values: a human reading a capture sees the same discriminants as
`crpg-net`, while the machine still checks the explicit field.

Caps per channel (all checked on the length prefix before allocation,
then re-checked as inner-length against the channel cap after stripping
the octet):
- downstream (server→client, read cap is the direction cap): lane-0 v1/v2
  inner ≤ 65,536; transfer inner ≤ 65,536 (`MAX_DOWN_FRAME_BYTES`,
  `protocol_transfer.rs:46`). Max observed transfer frame 32,815 stays
  legal (facts §3).
- upstream (client→server): lane-0 v1/v2 inner ≤ 4,096; transfer inner ≤
  4,096 (`MAX_UP_FRAME_BYTES`, `protocol_transfer.rs:48`). Max observed
  30 stays legal (facts §3).
- hello 130 / welcome 17 unchanged (`handshake.rs:22,24`); the channel
  octet never appears on hello/welcome.
Oversize (length prefix over the direction cap, or inner over the channel
cap) → `FrameTooLarge` (3); zero-length inner or zero total length →
`ProtocolViolation` (2), mirroring `handshake.rs:37-38` and
`queue.rs:618-621`.

Why one ordered stream keeps the single total order: the crate already
guarantees within-connection per-direction exact-once in-`try_send`-order
delivery (crate AGENTS.md invariant 6). Queuing lane-0 and transfer
inner frames through the same outbound queue and the same bidi stream
(the existing writer path) makes the peer's arrival order equal the
sender's `try_send` order across both channels. That is exactly the
property T024 S§11 item 3 (`tasks/T024.md:896-897`) and S§10.7
(`tasks/T024.md:831-834`) need: a `Begin` precedes every delta above its
cursor and chunks follow their `Begin`, so the receiver's cursor cutover
never applies a delta to the wrong base. Separate streams (or DATAGRAM)
would need a cross-stream barrier the crate has no mechanism for
(facts §8; ADR-0025 item 8, `docs/adr/0025-...:86-88`), plus a new ALPN
anyway. Keeping one stream also preserves the bounded-queue,
park-one-frame, first-wins-close machinery (`queue.rs:639-648`,
arch doc bounds) without a second accounting domain.

*Recommendation:* approve the 1-byte channel octet inside the framed
payload with values `0x01`/`0x02`/`0x81`, direction caps as channel caps,
single shared outbound/inbound queues per connection.

## S§6. Errors and close mapping (item 3)

Pin: existing `SendError`/`ServerError`/`ConnectError`/`CloseReason`
variants and `Display` texts (`"<VariantName> at quic/<area>"`) are frozen.
One additive variant only (needs its ADR line because public items need
an ADR per crate AGENTS.md): `UnknownChannel` on the send/recv error
path. No close-code number changes (codes 0–9, `close.rs:10-70`).

- Option A (recommended): extend the existing errors. `SendError` gains
  `UnknownChannel` (send-side: unknown octet on a caller-supplied
  channel; never sent on the wire); `ServerError` gains the same for
  `try_recv`/`lane`. `Display`: `UnknownChannel at quic/send`
  (respectively `quic/server`), following `close.rs:121-133` and
  `handshake.rs:242-251` patterns.
- Option B: a new closed `ChannelError` enum duplicating all variants plus
  `UnknownChannel`. Consequence: two parallel error types for one queue;
  every caller maps twice. Rejected.

Signatures (exact, additive; old ones retained and delegate on `/1`):
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel { Lane0V1, Lane0V2, Transfer }

pub fn channel_octet(channel: Channel) -> u8; // 0x01, 0x02, 0x81
pub fn channel_from_octet(raw: u8) -> Option<Channel>;

// Server (extends server.rs:319,346,462):
pub fn try_send_channel(&mut self, conn: ConnectionId, channel: Channel, inner: &[u8]) -> Result<(), SendError>;
pub fn try_recv_channel(&mut self, conn: ConnectionId) -> Result<Option<(Channel, Vec<u8>)>, ServerError>;
pub fn lane_channel(&mut self, conn: ConnectionId, channel: Channel) -> Result<ServerLane<'_>, ServerError>;

// Client (extends client.rs:201,218):
pub fn try_send_channel(&mut self, channel: Channel, inner: &[u8]) -> Result<(), SendError>;
pub fn try_recv_channel(&mut self) -> Result<Option<(Channel, Vec<u8>)>, CloseReason>;
```
`ServerLane` keeps its `Transport` impl on the lane-0 channel only; the
transfer channel is exposed only through the `_channel` entry points so
the `Transport` byte-pipe stays lane-0-pure.

Protocol-error → connection-close mapping (reader side, via `local_close`
at `queue.rs:639-648`, first-wins):
- length prefix over direction cap, or inner over channel cap →
  `FrameTooLarge` (3) (as today, `queue.rs:618-621`).
- zero total length, zero inner length, unknown channel octet, `/1`
  shape on a `/2` connection or vice versa, FIN mid-frame,
  `RESET_STREAM`/`STOP_SENDING` on the lane → `ProtocolViolation` (2)
  (as today, `queue.rs:619,627-633`, `close.rs:16`).
- ALPN/TLS failure → no lane event; server `refused_handshakes += 1`
  (`server.rs:679-687`); client `ConnectError::Handshake`
  (`client.rs:48`).
- idle → `CloseReason::IdleTimeout`; transport/crypto or transport-type
  peer close → `Transport` (`queue.rs:77-85`); stateless reset/lost →
  `Reset` (`queue.rs:78`); I/O thread end → `EndpointFailed`
  (`server.rs:166-168`). Unchanged.
- peer codes outside 0..=9 stay raw (`close.rs:41-57`, ADR-0025 item 7).

Send-side mapping (no close emitted; caller-visible only, precedence
`UnknownConnection → NotAccepted → Closed → EmptyFrame → FrameTooLarge
→ QueueFull` per `server.rs:343-345`, plus `UnknownChannel` first on bad
caller channel): `EmptyFrame` on empty inner; `FrameTooLarge` on inner
over the channel cap; `QueueFull` retryable with nothing queued.

*Recommendation:* approve Option A with the six signatures above and the
mapping table as written.

## S§7. Hard caps and policy limits

No cap changes. Direction caps, hello/welcome sizes, queue bounds,
connection counts, timeouts, and QUIC window limits keep their v1 values
(arch doc bounds table; crate AGENTS.md). The channel octet costs 1 byte
of the existing caps under `/2`; the largest transfer frame becomes
32,816 framed (32,815 + 1) ≤ 65,536, and the largest upstream transfer
frame 31 ≤ 4,096. `TransferLimits`/staging/pool budgets live in
`crpg-net` (T024 S§7) and are not re-stated here. Tightening-only
validation keeps its field order and `InvalidLimits`-before-`LoosensPolicy`
rule.

## S§8. Interface (exact public Rust, additive)

```rust
pub const ALPN_LANE0_V2: &[u8] = b"crpg-lane0/2";
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel { Lane0V1, Lane0V2, Transfer }
pub fn channel_octet(channel: Channel) -> u8;
pub fn channel_from_octet(raw: u8) -> Option<Channel>;
// plus the six S§6 methods; SendError::UnknownChannel / ServerError::UnknownChannel.
```
Every new item derives the crate's conventions (`Debug, Clone, Copy,
PartialEq, Eq` where value-like; `Display` as `<Variant> at quic/<area>`
with `std::error::Error`). No tokio/quinn/rustls type in any public
signature (crate AGENTS.md invariant 2). No `crpg-net` re-export (crate
trap). No `HashMap`/`HashSet`/`f32`/`f64`/`unsafe`/platform-`cfg`/logging
in `src` (crate AGENTS.md scope grep).

## S§9. Behaviour (transport only)

Under `/2`, per connection per direction: `try_send_channel` order is
arrival order across both channels; exactly-once; bounded queues shared
across channels (one accounting domain, so a transfer burst applies
backpressure to lane-0 and vice versa — intended, because the order
property requires one queue). A full inbound queue parks one complete
v2 frame (octet included) and stalls the stream (as today). After a
close, received-but-undrained tagged frames stay drainable with their
tags before `Closed` is published (arch doc bounds). Closes are
first-wins then published (invariant 7). The crate never decodes inner
bytes: lane-0 vs transfer routing is by octet only. It never mints
transfer ids, checks cursors, enforces deadlines, publishes snapshots, or
sends `Published`/`Resync` — those are `crpg-net` + `crpg-server`
(T024h) policy above it.

## S§10. Host-adapter changes (item 4, ADR-0026)

Needed in `crpg-server` (T024h implements; this contract only reserves
the transport shape so the adapter can be written without moving policy
down):

1. `QuicHost::pump` demultiplexes `try_recv_channel`: `(Lane0V1|Lane0V2,
   bytes)` → today's ingest path unchanged (invitation binding,
   `ingest`, one-frame hold, fencing per ADR-0026 items 2–7); `(Transfer,
   bytes)` → a new opaque transfer staging queue drained by the T024h
   sender/receiver wiring. No decoding, no grant/cursor check, no id
   allocation in `crpg-net-quic`.
2. Delivery demultiplexes the other way: the host's egress offers tagged
   frames; the adapter calls `try_send_channel` with the frame's channel.
   Acknowledgement stays on hand-off (ADR-0026 item 5) per tagged frame.
3. Backpressure: the existing one-frame hold (ADR-0026 item 6) stays a
   single shared hold across both channels, preserving the single total
   order. A per-channel hold is rejected: two holds can admit a delta
   past a held `Begin`.
4. Fencing/close paths (`SessionFenced` 9 `Discard`, slow-consumer fence,
   drain-then-unbind, D02 shutdown per ADR-0026 items 7–10) apply to the
   connection, not per channel; no new close code.
5. `transport_totals`/`connection_stats` gain per-channel counters only
   as additive fields; existing field semantics unchanged.

- Option A (recommended): tagged routing with shared hold, as above.
- Option B: separate transfer connection/stream. Consequence: needs a
  second stream type (ADR-0025 item 8 forbids without new ALPN + design),
  a cross-stream barrier for S§11 item 3, and duplicate queue accounting.
  Rejected.

*Recommendation:* approve Option A. Nothing in it moves host policy
(grants, credentials, epoch/session, id/cursor/deadline/publication,
revocation, narrowing) into `crpg-net-quic`; the crate still judges only
length, octet validity, and queue room.

## S§11. Tests (item 5, `tests/quic.rs` additions, public API only)

Existing 30 cases stay untouched (DoD suite count becomes 30 + 10).
Each new case has a positive control and states what fails without the
fix (valid controls, not stubs):

1. `v2_selector_round_trip`: `/2` connection; each channel (`0x01`,
   `0x02`, `0x81`) round-trips its inner bytes with the tag intact;
   max-size inner per direction accepted. Control: same bytes on `/1`
   lack the octet (S§12 case 6 proves it).
2. `v2_unknown_selector_refused`: octet `0x00`, `0x03`, `0x80`, `0x82`,
   `0xFF` each close with `ProtocolViolation` (2) via `local_close`
   (`queue.rs:639-648`); peer observes `Peer { code: 2 }`. Control: the
   three valid octets do not close.
3. `v2_oversize_per_channel`: inner 65,537 downstream and 4,097 upstream
   on every channel are `FrameTooLarge` on send and close 3 on raw
   receipt; exact-cap inner (65,536 / 4,096) accepted. Control: 32,815
   transfer chunk + octet (32,816) accepted downstream.
4. `v2_alpn_mismatch_matrix`: old→new negotiates `/1` (legacy frames
   flow); new(`/1`)→new flows legacy; new(`/2`)→old fails with
   `ConnectError::Handshake` (`client.rs:48`) and no server record;
   `foreign_alpn_refused` (`tests/quic.rs:692-708`) still passes.
5. `v2_interleaved_transfer_delta_order_preserved`: `/2` connection;
   sender interleaves lane-0 deltas D1..D6 with `Begin`/chunks
   (synthetic opaque payloads with the right octets, no transfer decode);
   receiver pops `(channel, bytes)` in exact send order, including
   `Begin`-before-deltas-above-cursor and chunks-after-`Begin`. Impaired
   relay variant (drop/reorder) still delivers exactly once in order.
   Negative control: shuffling the popped sequence breaks the T024
   cursor check in a `crpg-net` pool (cited, not re-implemented).
6. `v1_path_byte_identical`: `/1` handshake bytes, frame headers, and a
   fixed vector of lane-0 frames equal the pre-change bytes exactly
   (extends `frame_and_handshake_bytes_are_exact`, `tests/quic.rs:72`);
   `/1` never emits or accepts an octet; unknown-octet logic unreachable
   on `/1`.
7. `v2_empty_and_fin_midframe`: empty inner on send is `EmptyFrame`;
   zero-length wire frame and FIN mid-frame close with
   `ProtocolViolation` (2) (mirrors `lane_frame_caps_enforced_both_directions`,
   `tests/quic.rs:893`).
8. `v2_queues_shared_and_retryable`: filling outbound with transfer
   frames makes a lane-0 `try_send_channel` return `QueueFull` with
   nothing queued; retry after drain succeeds; inbound park-one-frame
   stalls a mixed-channel sender (mirrors `inbound_backpressure_at_exact_bounds`
   `tests/quic.rs:1240` and `outbound_queue_full_is_retryable_and_lossless`
   `tests/quic.rs:1369`).
9. `v2_close_mapping_first_wins`: raw over-cap header closes 3, zero
   header closes 2 (as in `tests/quic.rs:893`); a second violation keeps
   the first code (`local_close` first-wins, `queue.rs:639-648`).
10. `v2_no_decode_no_policy`: wrong-epoch / stale-id / bad-cursor
    transfer inners (hand-built, including `TRANSFER_VERSION` first byte)
    are delivered verbatim with tag `Transfer`; the crate emits no
    `Resync`, id, or cursor error. Control: the same bytes decode in
    `crpg-net` to the expected codec outcome (proves policy lives above).

*Recommendation:* approve all ten; the six BRIEF-mandated controls are
cases 1–6 (round-trip, unknown selector, oversize per channel, ALPN
mismatch, interleaved order, byte-identical regression).

## S§12. Docs and ADR record

- ADR-0025 addendum (new ALPN): records `ALPN_LANE0_V2`, the octet, values,
  caps, single-stream rationale, close mapping, and the dual-stack
  negotiation — in ADR-0025's shape, linking here as authoritative.
- `crates/crpg-net-quic/AGENTS.md`: new `Channel` items in Public surface;
  octet-is-opaque + single-shared-queue + no-policy in Invariants; two
  traps (never infer channel from payload bytes; never add a per-channel
  hold/queue that can reorder `Begin` past deltas).
- `docs/architecture/crpg-net-quic.md`: channel section (negotiation,
  framing diagram, caps table, close table, adapter handoff), suite count
  30 → 40.

## S§13. Definition of done (for the implementation task)

- [ ] Exactly the S§3 files changed; no manifest/lock/`deny.toml`/`tools/`
  diff; no `crpg-net`/`crpg-server` file edited.
- [ ] S§11 commands pass, then the crate DoD
  (`cargo test -p crpg-net-quic --test quic`, `cargo test -p crpg-net`,
  `cargo test --workspace`, clippy, `deps.py`, `determinism.py`, lint
  self-tests, `cargo deny check`, `git diff --check`) on Linux/GNU locally
  and Windows/MSVC through CI; scope grep prints only
  `#![forbid(unsafe_code)]`.
- [ ] Stopping command last: the workspace test run per crate AGENTS.md.

## S§14. Out of scope / boundary (item 6)

Left to T024h (`crpg-server` sender rules + seamless narrowing; input is
T024 S§11, `tasks/T024.md:885-910`, via ADR-0028 §Host requirements):
capturing bodies from the host view; allocating strictly-increasing per-epoch
transfer ids from 1; `plan_transfer` canonical framing; staging egress in
order (`Begin` before deltas above its cursor, chunks after `Begin`);
holding post-cutover deltas under existing caps; handling
`Published`/`Resync`; keeping `event_seq` unreset within an epoch;
`Cancel(Superseded|Aborted)` on supersession/revocation/overflow and fresh
snapshots after; replacing T022 narrowing-reset + `SessionFenced` with a
seamless snapshot (ADR-0026 items 6–7 as amended by T024h).

Left to T025a (`crpg-net` resume/resync): proof-bearing resume, fencing
generations, installing a snapshot across connections, reusing these
messages and the `ReassemblyPool` (T024 S§15, `tasks/T024.md:1147-1153`).

Left to T025b (`crpg-server` grace): authenticated 30-second same-process
grace and session fencing atop newest-wins + fresh-epoch (ADR-0026
consequences; T024q rows in BACKLOG).

Not in any of them here: compression, bulk package transfer, World/save
bytes, integrity digests, perf ceilings, v1/v2 changes, `RejectionCode`
changes, T018b tracker changes, host-resume claims from transport tests.

*Recommendation:* approve the boundary as written: this contract moves
opaque tagged bytes in one total order; every grant/id/cursor/deadline/
publication/narrowing/resume decision lives in T024h/T025a/T025b.

## S§15. Compatibility, dependency, S-set and invariant impact

- Wire: `/1` peers see byte-identical bytes (S§5.6/case 6). `/2` is a new
  ALPN with a 1-byte-per-frame cost and no change to hello/welcome, stream
  mapping, close codes, windows, or limits.
- Dependency: none added. `ALLOWED` row stays `{"crpg-net"}`
  (`tools/lint/deps.py:59`); D23 pins unchanged; no new edge or feature;
  no `deny.toml` change; vendored quinn-proto untouched (ADR-0027 §3
  non-scope). `cargo tree -p crpg-net -e normal,build` still names no
  tokio/quinn/rustls/ring/mio/socket2.
- S-set (`docs/IMPLEMENTATION_PLAN.md` §1.4: `crpg-contracts`, `World`/tick
  order, data schemas/migrations, GDExtension boundary, workspace
  `Cargo.toml`/`deny.toml`/workflows): nothing touches it.
- Invariants: platform-neutral source, no `cfg(target_os)`, gated on both
  native targets (ADR-0012); replay/`state_hash`/goldens unchanged;
  `bytes-are-opaque`, bounded-queues, first-wins-close, one-thread rules
  preserved; `#![forbid(unsafe_code)]` kept.

## S§16. Open questions for the user (each with a recommendation)

1. ALPN negotiation: dual-stack server + caller-selected single-ALPN
   client (S§4 Option A)? *Recommendation:* approve — only option with a
   byte-identical legacy path and no silent downgrade.
2. Selector placement/values: 1-byte octet inside the framed payload,
   values `0x01`/`0x02`/`0x81` (S§5)? *Recommendation:* approve — minimal
   diff, untouched header parser, explicit-not-sniffed routing.
3. Errors/signatures: extend `SendError`/`ServerError` with
   `UnknownChannel` and add the six `_channel` methods (S§6/S§8)?
   *Recommendation:* approve — one error family, additive surface.
4. Adapter: tagged routing with one shared hold, no policy in the crate
   (S§10 Option A)? *Recommendation:* approve — preserves the single total
   order; per-channel holds or streams need a barrier that does not exist.
5. Tests: the ten S§11 cases (six mandated + four close/queue/policy
   controls)? *Recommendation:* approve — each has a positive and a
   failing control; legacy cases untouched.
6. Boundary: transport-only here; sender rules → T024h, resume → T025a,
   grace → T025b (S§14)? *Recommendation:* approve — matches the
   D35-approved T024 S§16 Q5 filing.

## Agent log

- 2026-10-30 (UTC) · opencode-go/muse-spark-1.3-contributor + T024q draft · Blind transfer-channel contract draft (ALPN /2, channel octet, errors, adapter handoff, test list, boundary) from the fact sheet and cited sources, without reading the sibling draft.
