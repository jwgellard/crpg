# crpg-net-quic architecture

## Purpose and position

`crpg-net-quic` is the real lane-0 transport. It carries `crpg-net`'s
lane-0 intent and delta bytes between a host and its clients over QUIC.
- It sits **above `crpg-net`** (its only workspace edge; `ALLOWED` row
  `{"crpg-net"}`) and **below `crpg-server`**, which takes a path edge on it
  in T023b.
- It exists apart from `crpg-net` so that crates needing only the protocol,
  the codecs or the in-memory fabric never link tokio, quinn or rustls.
  `crpg-net` keeps its no-I/O rule with no exception.

Placement: [ADR-0024](../adr/0024-separate-crpg-net-quic-crate.md). Wire and
trust: [ADR-0025](../adr/0025-lane0-quic-mapping-and-pinned-trust.md).
Dependencies: D22/D23 ([decisions](../../tasks/DECISIONS-2026-09-30.md)) and
the [ADR-0023](../adr/0023-windows-sys-duplicate-skip.md) skip with its T023
addendum. Exact API and behaviour: the
[T023 re-contract](../../tasks/T023.md#re-contract--2026-10-04-utc-the-crpg-net-quic-crate).

## What exists (T023) and what is planned

As built: a server endpoint (`QuicServer`), a client endpoint
(`QuicClient`), the hello/welcome handshake with SHA-256 certificate
pinning, bounded queues with backpressure, close codes and modes, and a
public endpoint suite (`tests/quic.rs`, 23 cases). The suite runs over IPv4
loopback, including a UDP relay that drops and reorders packets.

T023c made the QUIC flow-control windows tightening-only limits (v1 equal
to T023's fixed values) and added cases 24–27, so the suite has 27 cases.
T023v vendors quinn-proto with upstream's `CONNECTION_CLOSE` fix
(`third_party/quinn-proto/VENDOR.md`) and adds case 28, so the suite has
28 cases. T023d adds cases 29–30, so the suite has 30 cases.

Planned, each as its own `crpg-net-quic` child task paired with a `crpg-net`
wire task where needed:

| Task | Transport work |
|---|---|
| T023b (`crpg-server`) | The host adapter: credential policy, binding, pump, the T022 mapping (E§7), and the five driver-based QUIC conformance cases |
| T024 | A snapshot stream under a new ALPN or version |
| T025a | Resume/reconnect hello; seamless narrowing instead of a `SessionFenced` close |
| T026p | Built as the movement endpoint, T150b — see "The movement endpoint (T150b)" below (the vendored D23 dedup patch and its native Windows probe are T150a's) |

## Module flow

```text
lib.rs        façade: crate docs, private mods, explicit pub use (no glob)
handshake.rs  ALPN/SNI/size constants, frame_header/parse_frame_header,
              Credential (redacted Debug, ct_eq), Hello, Welcome
identity.rs   CertificatePin (SHA-256 of DER, hex), ServerIdentity
limits.rs     ServerLimits/ClientLimits (v1, validate), configs, ConfigError
close.rs      CloseCode 0–9, CloseMode, CloseReason, SendError, ConnectionStats
tls.rs        (private) rustls/quinn configs, transport parameters,
              PinnedServerVerifier
queue.rs      (private) shared State behind one Mutex, Condvar + Notify
              wakeups, bounded inbound/outbound accounting, and the lane
              reader / writer / control tasks both endpoints run
server.rs     QuicServer: bind, accept loop, handshake/hello/decision per
              connection, sync API, shutdown/Drop, ServerLane
client.rs     QuicClient: connect sequence, sync API, close/Drop
teardown.rs   (private) close budget split (R = min(200 ms, B / 4)), the
              flush_end/overall instants, and the I/O-thread exit sequence
```

One connection, server side:

```text
UDP ─ quinn Endpoint ─ admit (max_connections, else CONNECTION_REFUSED)
     └─ TLS 1.3 (ring, ALPN crpg-lane0/1) ─ accept_bi ─ hello (cap 130, hello timer)
        └─ HelloReceived ─ decision timer ─ accept → welcome (17 B) written first
           ├─ reader task: [len][payload] (cap 4,096) → inbound queue or park
           ├─ writer task: outbound queue → SendStream (Flush: drain, FIN, ack)
           └─ control task: requested closes, Flush, connection.closed() → reason
```

The client mirrors this with one connection: the read cap is 65,536, the
send cap is 4,096, and its queue caps double as its host-wide caps.

## Runtime and thread model

- Each endpoint owns exactly one OS thread (`crpg-quic-server` or
  `crpg-quic-client`) running one tokio current-thread runtime (`rt`,
  `net`, `time`, `sync`; no macros, no multi-thread runtime).
- The caller's thread binds the `std::net::UdpSocket`, and the I/O thread
  wraps it in a quinn endpoint. A `sync_channel(1)` reports readiness, so
  `bind` and `connect` return synchronously.
- The sync side and the I/O side share one `std::sync::Mutex<State>`:
  - the mutex is never held across `.await`;
  - sync waiters (`wait`, `close(Flush)`, `shutdown`) use a `Condvar`;
  - async tasks are woken through `tokio::sync::Notify`.
- If the I/O thread ends without a clean stop, a drop guard marks every
  connection `EndpointFailed` and sets a failure flag. Later calls then
  report `CloseReason::EndpointFailed`, and the caller never panics.
- The transport touches no `World`, RNG, event queue or simulation clock.
  Arrival timing across connections is real-network nondeterminism. Within a
  connection, order is exact.

## Bounds

The defaults are the v1 values. Every limit is tightening-only, and
`validate` reports `InvalidLimits` before `LoosensPolicy`, in declaration
order.

| Bound | Server | Client |
|---|---|---|
| Read cap per lane frame | 4,096 | 65,536 |
| Send cap per lane frame | 65,536 | 4,096 |
| Inbound queue | `QueueCaps::v1()` (128 frames / 256 KiB per connection; 1,024 / 2 MiB host-wide) | 128 frames / 2 MiB |
| Outbound queue | `QueueCaps::v1_egress()` (128 / 2 MiB per connection; 16 MiB host-wide) | 128 frames / 256 KiB |
| Parked inbound frame | ≤ 1 per connection | ≤ 1 |
| Connections | 64, pending included | 1 |
| Idle / keep-alive | 60 s / 15 s | 60 s / 15 s |
| Hello / decision / connect | 5 s / 5 s / — | — / — / 10 s |
| Close flush[^flush] | 2 s | 2 s |
| QUIC stream receive window (`stream_window_bytes`) | 65,536 (floor 4,100) | 1,048,576 (floor 65,540) |
| QUIC connection receive window (`connection_window_bytes`) | 131,072 (≥ stream window) | 2,097,152 (≥ stream window) |
| QUIC send window (`send_window_bytes`) | 1,048,576 (floor 65,540) | 262,144 (floor 4,100) |

- **Close budget.** The close flush row's `B = close_flush_timeout_ms`
  splits at `R = min(200 ms, B / 4)`. Every `Flush` close, including
  `QuicServer::close`, stops its flush at `flush_end = now + (B − R)`; the
  endpoint stops waiting at `overall = now + B`. `FlushClose::flush_close`
  (in `queue.rs`) is the single constructor of a `Flush` close's request and
  its `overall` deadline; the request's fields are private, so no call site
  can build its own request or pick the wrong deadline (T098h). The I/O thread's exit closes the endpoint, yields to the scheduler once, then
  waits for `wait_idle` bounded by `overall`: the yield makes every driver
  woken by the close run at least once even when `overall` has already
  passed, and a budget of 0 stays best effort with that one pass (T098g).
- **Outbound full** → `SendError::QueueFull`. It is retryable and nothing is
  queued. A frame leaves the accounting when it is handed to QUIC, and
  quinn's own buffer is bounded by `send_window_bytes`.
- **Inbound full** → the reader parks one complete frame and stops reading,
  so QUIC flow control stalls the peer. Parked frames are let in, in
  ascending connection order, as `try_recv` frees room.
- **After a close**, data QUIC already received stays readable. The reader
  keeps feeding it in as room frees, and `Closed` is published, through the
  event and `try_recv`'s error, only once the reader has nothing more. A
  host that stops draining a closed connection therefore holds back that
  connection's `Closed` event.
- The transport parameters other than the windows are fixed (E§8.3):
  server 1 bidi / 0 uni streams, client 0 / 0, no DATAGRAM extension on
  `/1` and `/2` (downstream-only on `/3`, T150b), no migration. The windows
  are limits (T023c, ADR-0025 addendum): each
  endpoint advertises its own receive windows to the peer and enforces its
  own send window locally, and `v1()` reproduces T023's fixed values
  exactly. A window floor is one maximal frame of its direction plus the
  4-byte header.
- **What bounds a stalled peer** is its own receive windows, not the
  sender's send window: quinn releases acknowledged data, and a peer that
  stops reading still acknowledges. With one-frame queues on both ends and
  no window update (the stalled side has read fewer than `W / 8` bytes),
  a server's `try_send` to a stalled client accepts exactly
  `⌊(min(W, C) − 21) / F⌋ + 2` frames of framed size `F` before a lasting
  `QueueFull`: about 1 MiB of frames at the client's v1 windows (40,331
  frames of 22 bytes), 2,521 at its floor. Toward a stalled server the 21
  welcome bytes become the 22-byte hello. A server cannot impose a client's
  windows, so against a slow production client it relies on its outbound
  queue caps, its send window (memory) and T023b's slow-consumer fence
  (T023c C§1.2–C§1.3).

[^flush]: A `Flush` close stops its flush at `B − R` and keeps the last
    `R = min(200 ms, B / 4)` for the CONNECTION_CLOSE, so the goodbye still
    goes out inside `B`.

## The movement endpoint (T150b)

`QuicServer::bind_movement` serves exactly `crpg-lane0/3` (`/3`) with the
movement transport parameters; `bind`, `bind_dual` and `bind_resumable` keep
their ALPN lists and parameters byte for byte, and `/1`/`/2` connections
never see a second stream or a datagram (Reading 1). A `/3` connection maps
as follows (Readings 2–4):

| Stream / frame | Direction | Contents | Cap |
|---|---|---|---|
| Stream 1 (bidi) | both | the `/2` lane stream — hello, welcome, lane-0, `0x81` and `0x91` frames — framed exactly as `/2`, so `framing() == V2` | as `/2` |
| Stream 2 (bidi) | up / down | lane 1: intents up, receipts down, 4-byte length header | 4,096 up / 1,024 down |
| DATAGRAM | down only | position samples | ≤ 1,024 bytes |

Lane 1 is the client's second `open_bi`, visible to the server only once the
client writes its first lane-1 frame; the server's lane-1 writer exists from
then on, frames queued before that wait for it, and a `Flush` close in that
window discards them (Reading 3). Lane 1 has its own queues, its own parked
slot and its own sequence domain, with no order relation to lane 0. Its
queues are per-connection constants, not limits, and are host-wide only
through `max_connections` (Reading 8):

| Bound | Server | Client |
|---|---|---|
| Lane-1 frame cap | 4,096 up / 1,024 down | 4,096 up / 1,024 down |
| Lane-1 inbound queue | 64 frames / 65,536 bytes | 128 frames / 131,072 bytes |
| Lane-1 outbound queue | 128 frames / 131,072 bytes | 64 frames / 65,536 bytes |
| Datagram direction | send only | receive only |
| Datagram size | ≤ 1,024 | ≤ 1,024 |
| Datagram send buffer | 65,536 bytes | 0 (none) |
| Datagram receive buffer | `Some(0)` | `Some(65_536)` (advertised 65,535) |
| Client datagram queue | — | newest 64 |

Datagrams go downstream only (Reading 4): the server's quinn
`datagram_receive_buffer_size` is `Some(0)`, so it advertises
`max_datagram_frame_size = 0`; a peer's non-empty `send_datagram` then fails
with `TooLarge` before anything is sent, and an empty one makes the server's
quinn close with a transport `PROTOCOL_VIOLATION` (`CloseReason::Transport`).
The client's receive buffer is `Some(65_536)` and its send buffer 0.

The connect-time floor is the server's sendable size (Reading 5): right after
TLS on a `/3` connection, `max_datagram_size()` of `None` or below 1,024
closes with `ProtocolViolation` (2) before the hello, with no event and
`refused_handshakes` + 1. quinn computes it as `min(advertised − 9, MTU −
(1 + CID + 4 + 16) − 9)` with 8-byte CIDs and an MTU floor of 1,200, so only
the client's advertisement can fail it. **Recorded quinn property, not
changed here:** a quinn receiver closes the connection on a datagram whose
length plus 32 exceeds its own receive buffer, while its advertisement lets a
sender use the buffer less 9 — so a third-party client with a receive buffer
of 1,033..=1,055 passes the floor but cannot receive a 1,024-byte datagram.
This crate's client uses 65,536; a third-party client must use at least
1,056.

`try_send_datagram` is a direct, synchronous, non-blocking hand-off to
quinn's `send_datagram`, outside the state lock (Reading 6): there is no
crate datagram queue on the server, `Ok` means only "handed to QUIC" (which
may still drop it), and it never returns `QueueFull`. The client keeps the
newest 64 datagrams, dropping and counting the oldest when the queue is full
(Reading 7); a 0-byte datagram closes with `ProtocolViolation`, one above
1,024 with `FrameTooLarge`.

Closes are per connection, not per lane (Reading 9): a `Flush` close drains
and FINs both writers and waits for both acknowledgements inside the one
budget; a close is published only after both readers stop, frames either
reader received stay drainable, and the record-lifetime rule applies per
lane — the record is forgotten once its `Closed` is polled and each lane that
held frames at publication has reported `Closed` through its own call
(`try_recv`/`try_recv_channel` for lane 0, `try_recv_lane1` for lane 1).
Datagrams have no flush: unsent ones die with `CONNECTION_CLOSE`. A server
`Flush` while the lane-1 accept is still pending waits for lane 0 alone and
discards the queued lane-1 frames. `/3` hellos and welcomes carry wire
version 4 only (Reading 10); the public `Hello`/`Welcome` `encode`/`decode`
keep the `/1`/`/2` rule.

### The resumable movement endpoint (T155b)

`QuicServer::bind_movement_resumable` is `bind_movement` with the resume
opening on: the same ALPN (`[crpg-lane0/3]`), transport parameters and
datagram floor. The opening reader is the `/2` one, which already takes
`resumable` and `movement` separately: a first header byte `0x91` is a resume
request, queued as `ServerNotice::Resume`; anything else is a hello decoded
version 4 only. `bind_movement` still parses a `0x91` opening as a hello and
closes it (T155 Reading 5); no new ALPN, close code or constant exists.

`QuicClient::connect_movement_resume(config, server, session, request)` is
`connect_resume` on `/3`: TLS offering `/3` alone, the `[0x91][len24]`
request, the answer checked for form only, then the lane-1 stream opened, all
under the one `connect_timeout_ms`. It returns `(client, answer)` with
`movement() == true`, `framing() == V2` and `welcome() == session`; `session`
must be version 4 and `request` 1..=4,096 bytes, checked before any socket is
bound. `accept_resume` on a movement connection claims the lane-1 reader as
`accept` does, so a close is not published while lane-1 frames may still be
read (Reading 6). Lane 1 carries no resume progress: the client resends its
unanswered intents after an accepted resume (T155 Reading 2).

## Handshake

The handshake follows E§5 / ADR-0025.
1. The server refuses at `max_connections` before any crypto.
2. TLS 1.3 runs with the pinned verifier on the client.
3. The server reads one hello under `hello_timeout_ms`.
   - The close codes are 3 (over 130 bytes), 2 (malformed, zero length or
     FIN), 6 (unknown version) and 4 (timeout).
   - These failures produce no event and count in `refused_handshakes`.
4. `HelloReceived` starts the decision timer. Lane frames the client sent
   early stay in QUIC's buffer until `accept`.
5. `accept` writes the welcome before any lane frame. `refuse` closes with
   the given code. Timer expiry closes with `AuthTimeout` (8).

## What T023b inherits

- `QuicServer`: events, `accept`/`refuse`, `try_recv`/`try_send`, close
  modes, stats, totals, shutdown.
- `QuicClient` for headless test clients.
- The `(remote, wire_version, Credential)` triple, answered with
  `Welcome { wire_version, epoch }`.
- `Credential::ct_eq`.
- The informative T022 mapping in E§7.
- The record-lifetime rule: a connection is forgotten once its `Closed`
  event is polled **and**, if it held frames, `try_recv` has returned
  `Closed`.
- The window limits (T023c): a test client with tightened
  `stream_window_bytes` / `connection_window_bytes` stalls the server's
  `try_send` within a computable number of frames. The resulting
  amendments to T023b's cases 10, 12, 14 and 16 are in
  [T023c C§10](../../tasks/T023c.md#c10-downstream-amending-t023b-after-t023c-lands).
- Close codes reach the peer under a full congestion window (T023v): a
  `SessionFenced`/`Discard` close arrives as `Peer { code: 9 }`, not as a
  `Reset`, even when the server's congestion window is full (case 28,
  ADR-0025's T023v addendum).
- A refused or `Discard`-closed connection delivers its code to the peer
  even if `Closed` is polled in the same pump (T023d, cases 29–30). The
  record-lifetime rule is unchanged for the API: the connection is unknown
  to every call from that poll on.

Credential policy, the `now_ms` clock, and the hold-or-drop choice on
ingest `QueueFull` belong to T023b. T023b's decisions are R4 (close with
`SessionFenced`/`Discard` on fencing) and R5 (hold one refused frame and
stop draining).

## Agent log

- 2026-10-04 (UTC) · claude-code + T023 · Opened the doc with the crate's position between `crpg-net` and `crpg-server`, the module flow, thread model, bounds and handshake as built, and the T023b/T024/T025a/T026p split, so later transport children extend it rather than restating the contract.
- 2026-10-05 (UTC) · claude-code + T023c · Added the three window limits per side to the bounds, narrowed the fixed-parameters sentence to what is still fixed, explained what bounds a stalled peer and passed the window limits and the T023b amendments on to T023b, so the doc matches the code after the windows stopped being constants.
- 2026-10-05 (UTC) · claude-code + T023v-b · Recorded the vendored quinn-proto close fix and case 28 (28 cases) and told T023b that close codes now reach the peer under a full congestion window, so T023b's fencing cases can rely on the code instead of tolerating a `Reset`.
- 2026-10-06 (UTC) · claude-code + T023d · Told T023b that a refused or `Discard`-closed connection delivers its code even when `Closed` is polled in the same pump, with the API's record-lifetime rule unchanged, and raised the suite count to 30, so T023b's adapter can poll `Closed` at once without a workaround.
- 2026-11-07 (UTC) · opencode-go/deepseek-v4.1-flash + T098g · Added `teardown.rs` to the module flow, footnoted the Close flush row and added the close-budget bound (flush at `B − R`, endpoint at `B`, one exit yield), so the doc states the reserve every `Flush` close now keeps for CONNECTION_CLOSE.
- 2026-11-07 (UTC) · opencode-go/deepseek-v4-pro + T098h · Named `FlushClose::flush_close` as the single constructor of a `Flush` close's reserve'd request and its `overall` deadline, so the doc records that no call site builds its own request (D44c (a)).
- 2026-11-15 (UTC) · opencode-go/deepseek-v4-pro + T150b · Added the movement endpoint: the `/3` stream/datagram mapping, the lane-1 queue and datagram bounds, the connect-time datagram floor with quinn's recorded receive-buffer property, the per-lane close and record-lifetime rules, replaced the T026p planned row with a pointer to it, and amended the fixed-parameters line's "no DATAGRAM extension" to "none on `/1`/`/2`, downstream-only on `/3`", so the doc states what `bind_movement`/`connect_movement` build and keep separate from `/1`/`/2`.
- 2026-11-21 (UTC) · claude-code/sonnet-5.5 (implementer) + T155b · Added the resumable movement endpoint (`bind_movement_resumable`, `connect_movement_resume`, the lane-1 claim in `accept_resume`) under the movement endpoint, so the doc states what T155b builds and that `bind_movement` still refuses a resume opening.
