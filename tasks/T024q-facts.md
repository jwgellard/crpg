# T024q facts — `crpg-net-quic` transfer channel (T024 S§16 Q3 follow-on)

Fact sheet only. Every `path:line` below exists on this branch. No
recommendations, proposed types, or opinions.

## 1. ALPN(s): how chosen and checked, and where

- Single ALPN constant: `ALPN_LANE0_V1 = b"crpg-lane0/1"` (`crates/crpg-net-quic/src/handshake.rs:11-12`).
- It is the only ALPN advertised by both endpoints' TLS configs
  (`tls.alpn_protocols = vec![ALPN_LANE0_V1.to_vec()]`):
  `crates/crpg-net-quic/src/tls.rs:81` (server), `:122` (client).
- TLS 1.3 only, ring provider, no 0-RTT, no session resumption, no client
  cert; SNI is `crpg.invalid` (`TLS_SERVER_NAME`, `handshake.rs:14`;
  `tls.rs:65,103`).
- Server-side ALPN check after TLS handshake:
  `alpn_matches` compares `data.protocol` to `Some(ALPN_LANE0_V1)`
  (`crates/crpg-net-quic/src/server.rs:689`).
- Connections that fail ALPN (and other pre-hello failures) never produce
  `HelloReceived`; `end_handshake` removes the record, decrements `live`,
  increments `refused_handshakes`
  (`crates/crpg-net-quic/src/server.rs:679-687`).
- Test `foreign_alpn_refused` (`crates/crpg-net-quic/tests/quic.rs:692-708`)
  connects with `b"crpg-lane0/2"` and asserts the handshake is refused and
  `refused_handshakes == 1`; a control connection with the right ALPN
  reaches `HelloReceived`.
- ADR-0025 item 2: ALPN `crpg-lane0/1` pins the framing, hello/welcome
  layout, stream mapping and close codes; any change needs a new ALPN (example
  given: `crpg-lane0/2`) (`docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:31-36`).
- ADR-0025 item 8: privileged control streams are not available; adding one
  needs a new ALPN and its own reviewed design (`docs/adr/0025-...:86-88`).
- No constant for `crpg-lane0/2` exists in the crate (searched
  `crates/crpg-net-quic/src` and `tests`: only `ALPN_LANE0_V1`).

## 2. The ordered stream: framing, caps, reader/writer signatures

- One client-initiated bidirectional QUIC stream per connection; server
  permits one bidi stream and zero uni streams; DATAGRAM extension off
  (ADR-0025 items 1 and 8; `docs/architecture/crpg-net-quic.md:68,130`).
- Framing: 4-byte big-endian `u32` length prefix + payload. Constants and
  functions: `FRAME_HEADER_BYTES = 4`
  (`crates/crpg-net-quic/src/handshake.rs:16`);
  `frame_header(len, cap)` (`handshake.rs:36`);
  `parse_frame_header(header, cap)` (`handshake.rs:48`);
  `framed(payload, cap)` (`crates/crpg-net-quic/src/queue.rs:63`);
  `read_frame(recv, cap)` (`queue.rs:608`).
- Caps checked before any allocation: `parse_frame_header` rejects zero
  (`FrameError::Empty`) and over-cap (`FrameError::TooLarge`) lengths
  (`handshake.rs:48-60`); `read_frame` maps these to
  `CloseCode::ProtocolViolation` and `CloseCode::FrameTooLarge`
  (`queue.rs:618-621`).
- Per-direction frame caps:
  - client → server: 4,096 bytes = `MAX_INTENT_FRAME_BYTES`
    (`crates/crpg-net/src/protocol.rs:58`).
  - server → client: 65,536 bytes = `MAX_DELTA_FRAME_BYTES`
    (`crates/crpg-net/src/protocol.rs:64`).
- Enforced in transport code:
  - `QuicServer::try_send` rejects `frame.len() > MAX_DELTA_FRAME_BYTES`
    (`crates/crpg-net-quic/src/server.rs:361-362`).
  - `QuicClient::try_send` rejects `frame.len() > MAX_INTENT_FRAME_BYTES`
    (`crates/crpg-net-quic/src/client.rs:211-212`).
  - server inbound reader cap = `MAX_INTENT_FRAME_BYTES`
    (`server.rs:876-881`); client inbound reader cap =
    `MAX_DELTA_FRAME_BYTES` (`client.rs:433-438`).
- Hello cap 130 bytes = `MAX_HELLO_BYTES` (`handshake.rs:22`); welcome is
  exactly 17 bytes = `WELCOME_BYTES` (`handshake.rs:24`).
- Zero-length frame is invalid (`FrameError::Empty`, `handshake.rs:37-38`).
- Ordering: within a connection and direction, frames are delivered exactly
  once in `try_send` order (AGENTS.md invariant 6;
  `crates/crpg-net-quic/AGENTS.md:50-51`).
- Queues: every queue bounded in frames and bytes per connection and
  host-wide; `QueueFull` is retryable and changes nothing
  (`crates/crpg-net-quic/AGENTS.md:47-49`). A full inbound queue parks one
  complete frame and stops reading; parked frames are let in as `try_recv`
  frees room (`docs/architecture/crpg-net-quic.md:121-123`).
- Transport trait: `Transport` with `send`/`recv` and associated `Error`
  (`crates/crpg-net/src/transport.rs:14-23`).
  `ServerLane` implements it via `QuicServer::try_send`/`try_recv`
  (`crates/crpg-net-quic/src/server.rs:572-590`); `QuicClient` implements it
  (`crates/crpg-net-quic/src/client.rs:339-347`).
- Reader/writer functions (crate-private): `lane_reader(shared, id,
  connection, recv, cap, wake)` (`queue.rs:605` signature at `:608`);
  `lane_writer` (imported in `server.rs:23` and `client.rs:27`); `framed`,
  `read_frame` as above. Public entry points that move bytes:
  `QuicServer::try_recv` (`server.rs:319`), `QuicServer::try_send`
  (`server.rs:346`), `QuicClient::try_recv` (`client.rs:218`),
  `QuicClient::try_send` (`client.rs:201`), `QuicServer::lane`
  (`server.rs:462`).

## 3. T024 transfer subprotocol as it bears on transport

- Separate subprotocol: first byte `TRANSFER_VERSION = 0x81`
  (`crates/crpg-net/src/protocol_transfer.rs:19`); lane
  `LANE_SNAPSHOT = 2` (`protocol_transfer.rs:22`).
- Downstream tags `0x10` Begin, `0x11` Chunk, `0x12` Cancel; upstream
  `0x20` Published, `0x21` Resync
  (`protocol_transfer.rs:25-33`).
- Caps (transport-relevant): `MAX_SEGMENT_BYTES = 4096`
  (`protocol_transfer.rs:40`); `MAX_SEGMENTS_PER_CHUNK = 8`
  (`protocol_transfer.rs:42`); `MAX_CHUNK_BYTES = 32768`
  (`protocol_transfer.rs:44`); `MAX_DOWN_FRAME_BYTES = 65536`
  (`protocol_transfer.rs:46`); `MAX_UP_FRAME_BYTES = 4096`
  (`protocol_transfer.rs:48`); `MAX_ENCODED_BODY_BYTES = 22552`
  (`protocol_transfer.rs:50`); `MAX_SNAPSHOT_BYTES = 1_048_576`
  (`crates/crpg-net/src/protocol.rs:89`); `MAX_SNAPSHOT_CHUNKS = 32`
  (`protocol.rs:95`); `MAX_VISIBLE_ENTITIES = 1024` (`protocol.rs:92`).
- Compile-time checks: `MAX_CHUNK_BYTES * MAX_SNAPSHOT_CHUNKS == MAX_SNAPSHOT_BYTES`
  and `MAX_SEGMENT_BYTES * MAX_SEGMENTS_PER_CHUNK == MAX_CHUNK_BYTES`
  (`protocol_transfer.rs:56-57`).
- T024 S§4: demultiplexing happens at transport/channel selection; a caller
  picks the codec for the channel a frame arrived on; no decoder guesses or
  falls back (`tasks/T024.md:288-291`).
- T024 S§16 Q3 (approved, D35): correctness assumes a peer's transfer and
  delta frames share one total order; a `Begin` precedes every delta above
  its cursor and a transfer's chunks follow its `Begin`; updates leave through
  one FIFO queue (`tasks/T024.md:831-834,896-897,1192-1195`).
- T024 S§11 item 3: in the peer's single total order, a `Begin` precedes
  every delta above its cursor, and a transfer's chunks follow its `Begin`
  (`tasks/T024.md:896-897`).
- T024 S§15 out of scope: the real transport channel carrying transfer
  frames in one total order with lane-0 frames under a new ALPN
  (`tasks/T024.md:1147-1150`).
- T024 S§16 Q3 recommendation: carry transfer frames on the existing ordered
  stream with an explicit channel selector in the framing (not by inspecting
  payload bytes) under ALPN `crpg-lane0/2`; separate streams would need a
  cross-stream barrier (`tasks/T024.md:1192-1195`).
- ADR-0025: any change to stream mapping, framing, hello/welcome or close
  codes needs a new ALPN (`docs/adr/0025-...:132-134`).
- ADR-0028: filed Accepted by T024; follow-on tasks T024q (QUIC channel) and
  T024h (host sender rules) (`docs/adr/0028-snapshot-transfer-subprotocol.md:47`).
- Transfer frames fit existing caps: full chunk max 32,815 bytes ≤ 65,536
  downstream; upstream maxima 30 bytes ≤ 4,096
  (`tasks/T024.md:121-122,155`).
- T024 S§10.7: cutover correctness needs the transport/host property that a
  peer's transfer and delta frames share one total order; without it the
  receiver refuses (`BehindReplica`) rather than corrupts
  (`tasks/T024.md:831-834`).

## 4. Errors and close codes in `crpg-net-quic`

- Close codes 0–9 with names (`crates/crpg-net-quic/src/close.rs:10-70`):
  `Normal` 0, `Shutdown` 1, `ProtocolViolation` 2, `FrameTooLarge` 3,
  `HelloTimeout` 4, `AuthRefused` 5, `VersionRefused` 6, `ServerBusy` 7,
  `AuthTimeout` 8, `SessionFenced` 9.
- `CloseCode::from_u64` returns `None` for values outside 0..=9; peer codes
  are kept raw (`close.rs:41-57`; ADR-0025 item 7 `docs/adr/0025-...:70-72`).
- `CloseReason` variants: `Local(CloseCode)`, `Peer { code: u64 }`,
  `IdleTimeout`, `Transport`, `Reset`, `EndpointFailed`
  (`close.rs:84-102`).
- `SendError`: `UnknownConnection`, `NotAccepted`, `Closed(CloseReason)`,
  `EmptyFrame`, `FrameTooLarge`, `QueueFull` (`close.rs:104-119`).
- `ServerError`: `UnknownConnection`, `NotPending`, `NotAccepted`,
  `InvalidWelcome`, `Closed(CloseReason)` (`server.rs:66-79`).
- `ConnectError` defined at `crates/crpg-net-quic/src/client.rs:36-62`.
- `FrameError`: `Empty`, `TooLarge` (`handshake.rs:27-33`).
- Mapping from framing to close:
  - length prefix over the direction cap → `FrameTooLarge` (3):
    `read_frame` maps `FrameError::TooLarge` to
    `ReadFail::Code(CloseCode::FrameTooLarge)` (`queue.rs:618-621`);
    `QuicServer::try_send`/`QuicClient::try_send` return
    `SendError::FrameTooLarge` (`server.rs:361-362`, `client.rs:211-212`).
  - zero-length frame / empty frame → `ProtocolViolation` (2):
    `FrameError::Empty` → `ReadFail::Code(CloseCode::ProtocolViolation)`
    (`queue.rs:619`); `SendError::EmptyFrame` on send (`server.rs:358-359`,
    `client.rs:208-209`).
  - FIN mid-frame, `RESET_STREAM`, `STOP_SENDING` on the lane →
    `ProtocolViolation` (`queue.rs:627-633`).
  - ALPN/TLS failure → no lane event; counted in `refused_handshakes`
    (`server.rs:679-687,689`).
  - idle timeout → `CloseReason::IdleTimeout` (`reason_from`,
    `queue.rs:77`).
  - transport/crypto error or transport-type peer close →
    `CloseReason::Transport` (`queue.rs:82-85`).
  - peer stateless reset / connection lost → `CloseReason::Reset`
    (`queue.rs:78`).
  - I/O thread ended unexpectedly → `CloseReason::EndpointFailed`
    (`server.rs:166-168`).
- Protocol error to connection close: the reader calls `local_close`
  (`queue.rs:639-648`), which records the close in shared state first
  (`record_close`) and only then calls quinn's `close` with the code and its
  name as reason phrase (`close_quinn`, `queue.rs:89-92`); if a close was
  already decided elsewhere the later call keeps the first code.
- AGENTS.md trap: record a local close (`record_close`) before calling
  quinn's `close` (`crates/crpg-net-quic/AGENTS.md:118-119`).

## 5. Host adapter entry points that would carry the new channel

`QuicServer` (`crates/crpg-net-quic/src/server.rs`):
- `bind` (`server.rs:174`), `poll_event` (`:239`), `wait` (`:262`),
  `accept` (`:278`), `refuse` (`:300`), `try_recv` (`:319`),
  `try_send` (`:346`), `close` (`:372`), `stats` (`:419`),
  `totals` (`:432`), `lane` (`:462`), `shutdown` (`:476`),
  `local_addr` (`:233`).

`QuicClient` (`crates/crpg-net-quic/src/client.rs`):
- `connect` (`client.rs:105`), `welcome` (`:190`), `local_addr` (`:195`),
  `try_send` (`:201`), `try_recv` (`:218`), `wait` (`:232`),
  `stats` (`:248`), `close_reason` (`:272`), `close` (`:282`).

Adapter `QuicHost` (`crates/crpg-server/src/quic.rs`):
- `start` (`quic.rs:424`), `pump` (`:459`), `wait` (`:1027`),
  `register_invitation` (`:1034`), `revoke_invitation` (`:1074`),
  `set_invitation_grants` (`:1090`), `set_invitation_control` (`:1144`),
  `disconnect` (`:1174`), `tick` (`:1183`),
  `acknowledge_captures` (`:1191`), `host` (`:1211`),
  `binding` (`:1216`), `invitation_binding` (`:1222`),
  `local_addr` (`:1236`), `transport_totals` (`:1241`),
  `connection_stats` (`:1246`), `shutdown` (`:1251`).

Host delivery (`crates/crpg-server/src/host.rs`):
- `take_delivery` (`host.rs:1423`), `acknowledge_delivery` (`:1436`),
  `try_resync` (`:1403`, private), `chunk_frames` (`:1379`, private),
  `full_state_ops` (`:2032`, private).

## 6. Existing tests pinning framing, ALPN and caps

`crates/crpg-net-quic/tests/quic.rs` (30-case suite per AGENTS.md DoD):
- `frame_and_handshake_bytes_are_exact` (`quic.rs:72`): handshake bytes,
  frame header, `ALPN_LANE0_V1 == b"crpg-lane0/1"`, `MAX_HELLO_BYTES == 130`,
  `WELCOME_BYTES == 17`, `FRAME_HEADER_BYTES == 4`, hello decode precedence.
- `foreign_alpn_refused` (`quic.rs:692`): `crpg-lane0/2` fails handshake;
  right ALPN reaches `HelloReceived`.
- `lane_frame_caps_enforced_both_directions` (`quic.rs:893`): client send
  cap 4,096 and server send cap 65,536; `FrameTooLarge`/`EmptyFrame` leave
  queues unchanged; raw over-cap header closes with `FrameTooLarge` (3); zero
  header closes with `ProtocolViolation` (2).
- `limits_validate_and_refuse_loosening` (`quic.rs:347`): tightening-only
  validation.
- `credential_bounds_and_redaction` (`quic.rs:243`): credential 16..=128.
- `inbound_backpressure_at_exact_bounds` (`quic.rs:1240`) and
  `outbound_queue_full_is_retryable_and_lossless` (`quic.rs:1369`):
  bounded queues.
- `transport_trait_round_trip` (`quic.rs:1707`): `Transport` trait round trip
  with cap enforcement.
- `ordered_exactly_once_through_impaired_relay` (`quic.rs:1199`): ordering
  under impairment.

Known flake record:
- T098 (`tasks/T098.md`): open; flake in
  `ordering_preserved_under_concurrent_traffic` (case 13, `quic.rs:973`).
  Observed intermittent failures under contention: client thread panics at
  `quic.rs:1112` with `outbound_frames_discarded` 46 or 68 vs expected 0;
  server then sees `Closed(Peer { code: 0 })` at `quic.rs:1037`. Hypothesis
  is a 2,000 ms `close_flush_timeout_ms` flush bound under load; cause not
  established. Not Windows-only (Linux reproduction recorded
  `tasks/T098.md:47-52`). T098 forbids changing the case assertions or v1
  limits (`tasks/T098.md:66-74`).

## 7. Constraints that bind T024q

- ADR-0024: `crpg-net-quic` `ALLOWED` row is `{"crpg-net"}` only
  (`tools/lint/deps.py:59`; `docs/adr/0024-...:26-27,60-61`). It has no
  dev-dependencies (`crates/crpg-net-quic/AGENTS.md:71-73`). `crpg-core`,
  `crpg-data`, `crpg-rules`, `crpg-sim` and `crpg-net` are in
  `IO_FREE_CRATES` and may not name QUIC crates (`tools/lint/deps.py:81`,
  `:252`).
- ADR-0025: stream mapping (one bidi stream, no uni, no DATAGRAM), framing,
  hello/welcome, close codes and ALPN are pinned; any change needs a new ALPN
  and an ADR (`docs/adr/0025-...:25-36,70-88,132-134`).
- ADR-0027: vendored crates live under `third_party/` with `VENDOR.md`,
  numbered patches, `crate/` archive verified by checksum; `[patch.crates-io]`
  path entry; no git source, no fork URL, no `deny.toml` widening;
  `.gitattributes` sets `third_party/** -text -whitespace`
  (`docs/adr/0027-...:31-43,55-75`). quinn-proto is vendored with upstream
  close fix (`docs/adr/0027-...:17-23`).
- `crates/crpg-net-quic/AGENTS.md`: I/O allowed crate-wide; host side stays
  synchronous and non-blocking except named calls; never touch simulation
  state; bytes are opaque and never decoded here; caps checked on length
  prefix before allocation; every queue bounded; closes recorded first-wins
  then published; one endpoint/thread/runtime; no `HashMap`/`HashSet`/`f32`/
  `f64`/`unsafe`/platform `cfg` in `src` (`AGENTS.md` scope grep line 94);
  crate root has `#![forbid(unsafe_code)]` (`src/lib.rs:1`).
- ADR-0012 platform invariant: `x86_64-pc-windows-msvc` primary;
  `x86_64-unknown-linux-gnu` supported for server/CLI/tests; no OS-specific
  branches in core/rules/sim; select gates at compile time
  (`docs/adr/0012-windows-primary-platform.md` referenced by root AGENTS.md).
- ADR-0026: host adapter decisions T024q must respect: one adapter owns the
  whole host; invitation is principal; constant-time matching; newest
  connection wins; delivery acknowledged on hand-off; backpressure holds one
  frame; fencing closes with `SessionFenced` (9) in `Discard` mode; narrowing
  does not call `Host::set_grants`; slow-consumer fence 10,000 ms; drain then
  unbind; D02 shutdown order; synchronous/injected
  (`docs/adr/0026-quic-host-adapter.md:29-86`).
- T024 contract status: approved by user 2026-10-24 (D35); ADR-0028 filed
  Accepted (`tasks/T024.md:83`; `docs/adr/0028-...:4`).

## 8. Open facts (looked for, not found)

- No ALPN constant for `crpg-lane0/2` exists in `crates/crpg-net-quic`
  (searched `src` and `tests`); only `ALPN_LANE0_V1` (`handshake.rs:12`).
- No channel selector exists in the current framing: the frame header carries
  only a 4-byte length (`FRAME_HEADER_BYTES`, `handshake.rs:16-60`); lane
  payloads are treated as opaque bytes and are never decoded in this crate
  (`AGENTS.md:44-46,104`).
- No code in `crates/crpg-net-quic` references `protocol_transfer`,
  `TransferDown`, `TransferUp`, `TRANSFER_VERSION`, `LANE_SNAPSHOT` or the
  transfer tags (searched `src` and `tests`).
- No `crpg-net-quic` test exercises a transfer channel; the 30-case suite is
  lane-0 only (`tests/quic.rs`).
- No host-side handling of `TransferDown`/`TransferUp` in `QuicHost` or
  `Host` (searched `crates/crpg-server/src/quic.rs` and `host.rs`); T024
  S§15 lists host wiring as out of scope / follow-on T024h
  (`tasks/T024.md:1142-1146`).
- No cross-stream or cross-connection ordering mechanism exists: ordering is
  per connection per direction only (`AGENTS.md:50-51`); there is one bidi
  stream per connection and no uni streams (ADR-0025 item 1).
- No second stream type or DATAGRAM path exists in the crate (searched
  `src`; `max_datagram_frame_size` not advertised per ADR-0025 item 1).

## Agent log

- 2026-10-25 (UTC) · opencode-go/beam-free + T024q-facts · Fact sheet for the T024q contract covering ALPN, ordered-stream framing and caps, T024 transfer subprotocol transport facts, errors/close codes, host adapter entry points, pinning tests and flake record, binding constraints, and open facts.