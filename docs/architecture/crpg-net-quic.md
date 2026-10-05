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

Planned, each as its own `crpg-net-quic` child task paired with a `crpg-net`
wire task where needed:

| Task | Transport work |
|---|---|
| T023b (`crpg-server`) | The host adapter: credential policy, binding, pump, the T022 mapping (E§7), and the five driver-based QUIC conformance cases |
| T024 | A snapshot stream under a new ALPN or version |
| T025a | Resume/reconnect hello; seamless narrowing instead of a `SessionFenced` close |
| T026p | The movement DATAGRAM lane, and with it D23's vendored dedup patch and the native impairment probe |

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
| Close flush | 2 s | 2 s |
| QUIC stream receive window (`stream_window_bytes`) | 65,536 (floor 4,100) | 1,048,576 (floor 65,540) |
| QUIC connection receive window (`connection_window_bytes`) | 131,072 (≥ stream window) | 2,097,152 (≥ stream window) |
| QUIC send window (`send_window_bytes`) | 1,048,576 (floor 65,540) | 262,144 (floor 4,100) |

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
  server 1 bidi / 0 uni streams, client 0 / 0, no DATAGRAM extension, no
  migration. The windows are limits (T023c, ADR-0025 addendum): each
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

Credential policy, the `now_ms` clock, and the hold-or-drop choice on
ingest `QueueFull` belong to T023b. T023b's decisions are R4 (close with
`SessionFenced`/`Discard` on fencing) and R5 (hold one refused frame and
stop draining).

## Agent log

- 2026-10-04 (UTC) · claude-code + T023 · Opened the doc with the crate's position between `crpg-net` and `crpg-server`, the module flow, thread model, bounds and handshake as built, and the T023b/T024/T025a/T026p split, so later transport children extend it rather than restating the contract.
- 2026-10-05 (UTC) · claude-code + T023c · Added the three window limits per side to the bounds, narrowed the fixed-parameters sentence to what is still fixed, explained what bounds a stalled peer and passed the window limits and the T023b amendments on to T023b, so the doc matches the code after the windows stopped being constants.
