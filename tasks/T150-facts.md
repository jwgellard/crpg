# T150 facts — lane-1 movement transport in `crpg-net-quic`

Facts gathered for the T150 contract. Sources only; no design.

## Requirements

- `ADR-0036 gates T026p` (`docs/adr/0036-movement-and-navigation.md:14–16`) — T034 gates T036, T026p (lane-1 movement, S-set), T037 and the client tasks; the ADR is the movement spec D13 asks for.
- `ADR-0036 wire today` (`docs/adr/0036-movement-and-navigation.md:48–53`) — lane 0 is the only lane, lanes 1+ are reserved for movement (`crpg-net/src/protocol.rs:27-34`), and ADR-0004's spike sent per-tick `MoveTo` datagrams with latest-seq-wins.
- `L1` (`docs/adr/0036-movement-and-navigation.md:389–390`) — "**Lane 1 is movement**, with its own sequence domain per epoch (`SEQ_FIRST..=SEQ_LAST`). Lane 0 keeps window 1 (D13)."
- `L2` (`docs/adr/0036-movement-and-navigation.md:392–394`) — "**Upstream is a reliable ordered stream**: `MoveTo { seq, actor, destination: [i32; 3] }` (raw `Fx16_16`) and `Halt { seq, actor }`. There are no other fields (A1)."
- `L3` (`docs/adr/0036-movement-and-navigation.md:396–400`) — "**Reliable receipts**": one `MoveReceipt` per admitted intent, in `seq` order, with `Accepted`/`Superseded`/`Rejected`; `processed_seq` is the highest receipted `seq`.
- `L4` (`docs/adr/0036-movement-and-navigation.md:402–407`) — "**Samples are unreliable datagrams**": each `MoveSample` is self-contained and "within the transport's datagram limit"; "This is the first datagram lane, so the T030 dedup patch applies (D23)."
- `L5` (`docs/adr/0036-movement-and-navigation.md:409–411`) — samples carry only granted positions and revocation stops them at once.
- `ADR-0036 follow-on row` (`docs/adr/0036-movement-and-navigation.md:540`) — the T026p contract must pin L1–L6 and U7 policies; T037 pins H1–H5 and S3 and depends on T026p and T035.
- `ADR-0036 correction` (`docs/adr/0036-movement-and-navigation.md:612–614`) — "lane 1 is the separately versioned movement subprotocol `0xA1` (D13), beside lane-0 protocol version 4".
- `ADR-0004 dedup window` (`docs/adr/0004-quic-movement-spike.md:62–74`) — `quinn-proto`'s dedup is "a 129-packet-wide bitmask" that drops reordered but genuinely new packets, "including any DATAGRAM frame inside it"; the issue is quinn-rs/quinn#2710, closed unfixed upstream.
- `ADR-0004 datagram gate` (`docs/adr/0004-quic-movement-spike.md:95–107`) — "do not wire `crpg-net`'s snapshot channel straight to `quinn`'s `send_datagram`/`read_datagram` … without addressing #2710 first"; carrying a patch that widens `Window` is "a ~20-line, self-contained change".
- `ADR-0004 consequence` (`docs/adr/0004-quic-movement-spike.md:166–168`) — the eventual channel design ("snapshot = unreliable datagram") "needs a decision on #2710 before it is trusted at anything but loopback-quality reordering".
- `ADR-0024 pins` (`docs/adr/0024-separate-crpg-net-quic-crate.md:39–41`) — "The D23 pins are approved for `crpg-net-quic`, not `crpg-net`"; `crpg-server` takes no direct QUIC edge.
- `ADR-0024 row` (`docs/adr/0024-separate-crpg-net-quic-crate.md:60–63`) — the `ALLOWED` row is `"crpg-net-quic": ({"crpg-net"}, set())` and the D23 pins come with T023's audit.
- `ADR-0024 platform` (`docs/adr/0024-separate-crpg-net-quic-crate.md:64–65`) — "the crate is platform-neutral and is gated on both native targets like every other crate" (ADR-0012).
- `ADR-0024 lane split` (`docs/adr/0024-separate-crpg-net-quic-crate.md:81–82`) — "Each later lane splits into a `crpg-net` wire task and a `crpg-net-quic` transport task."
- `ADR-0025 item 1` (`docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:25–30`) — one client-initiated bidi stream carries hello, welcome and every lane-0 frame; "The DATAGRAM extension is off (`max_datagram_frame_size` is not advertised), so a peer's `send_datagram` fails".
- `ADR-0025 item 2` (`docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:31–36`) — ALPN `crpg-lane0/1` pins the framing, the hello/welcome layout, the stream mapping and the close codes; "A change to any of these needs a new ALPN".
- `ADR-0025 datagram alternative` (`docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:115–117`) — datagrams are rejected for lane 0; they "wait for the movement lane (T026p) and the dedup patch".
- `ADR-0025 consequence` (`docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:132–134`) — a change to the stream mapping, framing, hello/welcome layout or close codes needs a new ALPN and an ADR; "T026p (datagrams) each add their own".
- `ADR-0025 addendum item 4` (`docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:174–176`) — every other E§8.3 parameter "stays fixed: one client-initiated bidi stream, no uni streams, no DATAGRAM extension, no migration, `max_incoming = max_connections`, no 0-RTT."
- `ADR-0026 adapter` (`docs/adr/0026-quic-host-adapter.md:14–22`) — the adapter decides which connection becomes which host binding and "move[s] bytes both ways under bounded queues".
- `ADR-0026 edge` (`docs/adr/0026-quic-host-adapter.md:106–110`) — one path edge, `crpg-server → crpg-net-quic`; "No direct quinn, quinn-proto, rustls or tokio edge, and no new external crate".
- `D23 patch timing` (`tasks/DECISIONS-2026-09-30.md:83–88`) — the prepared `docs/reviews/T030-quinn/dedup-window.patch` "is applied, by the vendoring mechanism in the dossier, with the first unreliable-datagram lane (movement, T026p)"; "The T030 impairment probe must be re-run on native Windows at that point."
- `T026p task` (`tasks/T026p.md:48–49`) — lane 1 is new, "its own separately versioned subprotocol (first byte `0xA1`, D13)".
- `T026p split` (`tasks/T026p.md:59`) — T026p2 builds `movement_v4` (`MoveTo`/`Halt` intents, receipts, samples, `MoveOutbox`, the client's lane-1 sequence state).
- `T026p transport ownership` (`tasks/T026p.md:94–95`) — "`crpg-net-quic` (a new task, "Out of scope") carries lane 1's stream and datagrams."
- `T026p Reading 1` (`tasks/T026p.md:157–173`) — every lane-1 message starts with `MOVEMENT_VERSION` = `0xA1`; "v4 session" means lane-0 version 4 with movement version `0xA1`.
- `T026p Reading 6` (`tasks/T026p.md:209–211`) — "Samples are self-contained datagrams of at most 1,024 bytes and 38 entries"; QUIC guarantees 1,200-byte UDP payloads (RFC 9000 §14).
- `T026p lane-1 layout` (`tasks/T026p.md:596–609`) — the intent and downstream byte layouts; "the largest lane-1 intent is 54 bytes, the largest receipt 60, the largest sample 1,012 (24 + 38 × 26)".
- `M-7` (`tasks/T026p.md:741–747`) — `sample_datagrams` cuts entries into chunks of 38; "Every datagram is at most 1,012 bytes, and decoding them in order and concatenating their entries gives the input."
- `HM-4` (`tasks/T026p.md:988–991`) — "exactly one [receipt] per admitted intent, in `seq` order, on the lane-1 reliable stream".
- `HM-5` (`tasks/T026p.md:997–1001`) — "**Samples** go only as datagrams, never on a stream and never into the lane-0 `event_seq`"; split by `sample_datagrams`.
- `T026p out of scope` (`tasks/T026p.md:1471–1476`) — the new `crpg-net-quic` task owes "lane 1's reliable stream pair, the QUIC DATAGRAM extension for samples (with a maximum datagram size of at least `MAX_MOVE_DOWN_BYTES`, refused at connect otherwise), D23's vendored dedup patch, and the native Windows impairment probe re-run", citing this decisions file and `docs/architecture/crpg-net-quic.md:43`, `:142`.
- `T037 task` (`tasks/T037.md:49–50`) — the host speaks "lane 0 version 4 plus the lane-1 movement subprotocol `0xA1`, HM-1".
- `T037 out of scope` (`tasks/T037.md:1351–1355`) — "The `crpg-net-quic` lane-1 transport": the lane-1 reliable stream pair, the DATAGRAM extension, D23's dedup patch, "and the `quic.rs` route that binds v4 sessions and moves `ingest_move`, `take_move_receipts` and `take_samples` bytes. `quic.rs` is frozen here."
- `T037 D-1` (`tasks/T037.md:710–717`) — `ingest_move(peer, bytes, now_ms)` runs shared buckets, the 4,096-byte cap and ingress room before HM-2; nothing is staged on a refusal.
- `T037 D-9` (`tasks/T037.md:788–790`) — `take_move_receipts(peer)` returns the receipt queue's bytes in order and empties it, freeing its egress.
- `T037 C-6` (`tasks/T037.md:697–699`) — `take_samples(peer)` returns the queued sample datagrams and empties the queue, freeing its egress.
- `T037 Reading 23` (`tasks/T037.md:303–322`) — lane-1 bytes count toward the egress caps; a slot reserves one frame and `MAX_MOVE_RECEIPT_BYTES` (60) at admission, and a tick's samples "that do not fit are dropped whole (datagram loss, which the refresh repairs)".
- `T037 v4 binding` (`tasks/T037.md:437–441`) — `bind_v4_peer` is `bind_v3_peer` "with lane-0 version 4 and lane 1 (B-1)"; `session_wire` returns `4` for it (`:447–450`).
- `backlog row` (`tasks/BACKLOG.md:304`) — the lane-1 movement stream and datagram transport is "proposed", depends on T037d (D60).

## crpg-net-quic today

Public surface, one line per item (`lib.rs:40–53` re-exports these with no glob):

- `QuicServer` (`crates/crpg-net-quic/src/server.rs:200`) — the server endpoint handle; `bind` (`:224`), `bind_dual` (`:230`), `bind_resumable` (`:237`), `local_addr` (`:316`), `poll_event` (`:325`), `poll_notice` (`:354`), `wait` (`:378`), `accept` (`:396`), `accept_resume` (`:424`), `refuse` (`:449`), `try_recv` (`:471`), `try_recv_channel` (`:479`), `try_send` (`:532`), `try_send_channel` (`:549`), `framing` (`:586`), `close` (`:598`), `stats` (`:644`), `totals` (`:657`), `lane` (`:687`), `shutdown` (`:703`).
- `ConnectionId` (`server.rs:37`) — opaque `u64` connection id; `get` (`:41`).
- `ServerEvent` (`server.rs:52`) — `HelloReceived { conn, remote, hello }` (`:54–61`) and `Closed { conn, reason }` (`:64–69`).
- `ResumeReceived` (`server.rs:76`) — a resume opening's `conn`, `remote` and opaque `request` bytes.
- `ServerNotice` (`server.rs:97`) — the `poll_notice` resume-notice enum.
- `ServerError` (`server.rs:106`) — the server API error enum.
- `BindError` (`server.rs:139`) — bind/identity failure.
- `QueueTotals` (`server.rs:166`) — host-wide counters: `connections`, `pending`, inbound/outbound frames and bytes, `refused_handshakes`, `refused_at_limit`.
- `ShutdownReport` (`server.rs:189`) — what `shutdown` did.
- `ServerLane<'a>` (`server.rs:799`) — one connection as the net-local `Transport`; `send` (`:808`) and `recv` (`:813`) forward to `try_send`/`try_recv`.
- `QuicClient` (`crates/crpg-net-quic/src/client.rs:103`) — the client handle; `connect` (`:115`), `connect_framed` (`:125`), `connect_resume` (`:209`), `framing` (`:295`), `welcome` (`:302`), `local_addr` (`:307`), `try_send` (`:313`), `try_send_channel` (`:330`), `try_recv` (`:365`), `try_recv_channel` (`:386`), `wait` (`:400`), `stats` (`:416`), `close_reason` (`:440`), `close` (`:451`).
- `ConnectError` (`client.rs:43`) — the connect failure enum.
- `CloseReport` (`client.rs:95`) — `close`'s report of what was flushed.
- `CloseCode` (`crates/crpg-net-quic/src/close.rs:10`) — the ten codes 0–9 `Normal`..`SessionFenced` (`:12–31`); `as_u32` (`:36`), `from_u64` (`:41`), `name` (`:58`).
- `CloseMode` (`close.rs:76`), `CloseReason` (`close.rs:88`) — close modes and the recorded reason enum.
- `SendError` (`close.rs:108`), `ChannelSendError` (`close.rs:141`) — send failures, with the channel variant.
- `ConnectionStats` (`close.rs:163`) — per-connection inbound/outbound frames and bytes, `reader_parked`, `keepalive_pings_sent`.
- `ALPN_LANE0_V1` (`crates/crpg-net-quic/src/handshake.rs:12`), `ALPN_LANE0_V2` (`:16`) — `crpg-lane0/1` and `crpg-lane0/2`.
- `TLS_SERVER_NAME` (`handshake.rs:18`), `FRAME_HEADER_BYTES` (`:20`), `MIN_CREDENTIAL_BYTES` (`:22`), `MAX_CREDENTIAL_BYTES` (`:24`), `MAX_HELLO_BYTES` (`:26`), `WELCOME_BYTES` (`:28`).
- `frame_header` (`handshake.rs:43`), `parse_frame_header` (`:55`) — the 4-byte big-endian length prefix.
- `FrameError` (`handshake.rs:32`) — `Empty`, `TooLarge`, `UnknownChannel`.
- `Framing` (`handshake.rs:71`) — `V1`/`V2`; `alpn` (`:81`), `from_alpn` (`:89`).
- `Channel` (`handshake.rs:103`) — `Lane0`, `Transfer`, `Resume`; `as_u8` (`:115`), `from_u8` (`:120`).
- `channel_frame_header` (`handshake.rs:134`), `parse_channel_frame_header` (`:156`) — the `/2` header with the channel selector.
- `Credential` (`handshake.rs:186`) — opaque invitation bytes, redacted `Debug`; `new` (`:190`), `as_bytes` (`:201`), `ct_eq` (`:208`).
- `CredentialError` (`handshake.rs:228`), `HandshakeError` (`handshake.rs:319`) — format and handshake errors.
- `Hello` (`handshake.rs:237`) — `wire_version` + credential; `encode` (`:246`), `decode` (`:261`).
- `Welcome` (`handshake.rs:285`) — `wire_version` + 16-byte epoch; `encode` (`:294`), `decode` (`:303`).
- `CertificatePin` (`crates/crpg-net-quic/src/identity.rs:14`) — SHA-256 of the DER; `from_sha256` (`:18`), `of_certificate` (`:23`), `as_bytes` (`:28`), `to_hex` (`:33`), `from_hex` (`:44`).
- `ServerIdentity` (`identity.rs:76`) — certificate/key DER; `new` (`:83`), `pin` (`:91`).
- `ServerLimits` (`crates/crpg-net-quic/src/limits.rs:53`) — `v1` (`:87`), `validate` (`:105`).
- `ClientLimits` (`limits.rs:174`) — `v1` (`:207`), `validate` (`:225`).
- `ServerConfig` (`limits.rs:289`) — `bind`, `identity`, `limits`.
- `ClientConfig` (`limits.rs:300`) — `bind`, `pin`, `limits`.
- `ConfigError` (`limits.rs:311`) — an invalid limit or pin.

How lane 0 maps to streams:

- `lib.rs:6–12` — "One client-initiated bidirectional QUIC stream per connection carries a hello, a welcome, and then every lane-0 frame in both directions"; "There are no other streams and no DATAGRAM frames."
- `tls.rs:86–91` — the server's transport parameters are `bidi_streams: 1`, zero uni streams, plus the three windows; the client's are `bidi_streams: 0` (`:129–134`).
- `client.rs:747` — the client's connect calls `.open_bi()`; `server.rs:936` — the server's accept loop calls `.accept_bi()`.
- `queue.rs:800` `lane_reader` reads `[len][payload]` into the inbound queue; `queue.rs:876` `lane_writer` drains the outbound queue to `SendStream`; `queue.rs:928` `lane_control` runs the close/flush path; `queue.rs:734` `read_lane_frame` parses a header and checks the cap first.
- `server.rs:1180–1205` and `client.rs:652–676` spawn one reader, one writer and one control task per connection on the endpoint's current-thread runtime.

Limits (`limits.rs`):

- `ServerLimits` fields (`:53–80`): `idle_timeout_ms`, `keep_alive_ms`, `hello_timeout_ms`, `decision_timeout_ms`, `close_flush_timeout_ms`, `max_connections`, `inbound`/`outbound` (`QueueCaps`), and the three windows `stream_window_bytes`, `connection_window_bytes`, `send_window_bytes`.
- `ClientLimits` fields (`:174–201`): `idle_timeout_ms`, `keep_alive_ms`, `connect_timeout_ms`, `close_flush_timeout_ms`, four queue fields and the three windows.
- `validate` (`:105`, `:225`) checks fields in declaration order, `InvalidLimits` before `LoosensPolicy`; every limit is tightening-only.
- The v1 window values are `V1_SERVER_*`/`V1_CLIENT_*` at `:36–42`; the frame floors are `MIN_INTENT_WINDOW = 4_100` (`:44`) and `MIN_DELTA_WINDOW = 65_540` (`:46`), asserted against `MAX_INTENT_FRAME_BYTES`/`MAX_DELTA_FRAME_BYTES` plus the header (`:48–49`).
- No limit field exists for a datagram size, a datagram queue or a stream count other than the fixed `bidi_streams` in `tls.rs` (none found).

Close and teardown path:

- `close.rs:10–32` — codes 0–9, sent as QUIC varints with `name()` as the reason phrase.
- `queue.rs:108` `close_quinn` sends the application `CONNECTION_CLOSE`; `queue.rs:778` `local_close` records first and then calls it.
- `queue.rs:114` `Phase`, `queue.rs:160` `FlushClose` and `queue.rs:299` `Stop` hold the close state; `queue.rs:928` `lane_control` applies requested closes and maps `connection.closed()` to a `CloseReason`.
- `teardown.rs:9` `CLOSE_RESERVE_CAP_MS = 200`, `:12` `close_reserve_ms` (`R = min(200 ms, B / 4)`), `:18` `CloseDeadlines`, `:26` `close_deadlines`, `:42` `finish_endpoint_close` (yield, then `wait_idle` bounded by `overall`).
- `server.rs:703` `shutdown` and `Drop` close every connection with `Shutdown`; `client.rs:451` `close` takes `CloseMode`.
- The architecture doc states the close budget at `docs/architecture/crpg-net-quic.md:120–129` and the fixed transport parameters at `:141–147`.

Architecture doc:

- `docs/architecture/crpg-net-quic.md:43` — the T026p row: "The movement DATAGRAM lane, and with it D23's vendored dedup patch and the native impairment probe".
- `docs/architecture/crpg-net-quic.md:142` — "no DATAGRAM extension, no migration".
- `docs/architecture/crpg-net-quic.md:47–64` — the module flow (`lib`, `handshake`, `identity`, `limits`, `close`, `tls`, `queue`, `server`, `client`, `teardown`).
- `docs/architecture/crpg-net-quic.md:66–78` — the one-connection server flow: admit, TLS, `accept_bi`, hello, welcome, reader/writer/control tasks.
- `docs/architecture/crpg-net-quic.md:99–119` — the bounds table (read/send caps, queue caps, connections, timeouts, windows).
- `docs/architecture/crpg-net-quic.md:21–33` — what T023/T023c/T023v/T023d built and the 30-case count.

Dependencies and lint row:

- `crates/crpg-net-quic/Cargo.toml:7–19` — `crpg-net` (path) plus `quinn =0.11.12`, `quinn-proto =0.11.19`, `rustls =0.23.46`, `tokio =1.53.3`, all with default features off; no dev-dependencies.
- `tools/lint/deps.py:55–59` — the `crpg-net-quic` row is `({"crpg-net"}, set())`, commented as the real QUIC transport reachable only through `crpg-net`'s public byte protocol.

## The vendored quinn-proto and D23

- `Vendor directory` (`third_party/quinn-proto/`) — `crate/` (the unmodified 0.11.19 archive except the applied patches) and `patches/`; `VENDOR.md` documents source, hashes, patches, re-derivation and retirement.
- `Cargo.toml [patch.crates-io]` (`Cargo.toml:33–42`) — "T023v (user decision 2026-10-05, D23's vendoring mechanism, ADR-0027)"; `quinn-proto = { path = "third_party/quinn-proto/crate" }` is the last table, and the comment says it is "Not a workspace member".
- `VENDOR.md rules` (`third_party/quinn-proto/VENDOR.md:3–9`) — `crate/` is "unmodified except by the patches in `patches/`, applied in file-name order"; "Never edit `crate/` by hand, never run `rustfmt` on it, and never run cargo inside it in this repository."
- `VENDOR.md patch table` (`third_party/quinn-proto/VENDOR.md:26`) — exactly one patch, `0001-send-connection-close-when-congestion-blocked.patch` (upstream `e556fde`, #2787); "A change to the vendored code is a new numbered patch plus an updated table above" (`:67–70`).
- `T023v patch` (`tasks/T023v.md:1–7`) — the task is "vendored quinn-proto 0.11.19 with the `CONNECTION_CLOSE` fix", decided by T023c Q6 path b.
- `T023v dedup stacking` (`tasks/T023v.md:146`) — "D23 dedup patch stacking": `docs/reviews/T030-quinn/dedup-window.patch` "applies with `--dry-run` on top of 0001. Disjoint file (`spaces.rs`)".
- `T023v Windows gap` (`tasks/T023v.md:177`) — "No Windows run: this environment has no Windows host. Windows acceptance…".
- `D23 what the dedup patch must do` (`tasks/DECISIONS-2026-09-30.md:83–88`) — the prepared patch "stays ready"; it "is applied, by the vendoring mechanism in the dossier, with the first unreliable-datagram lane (movement, T026p)"; "The T030 impairment probe must be re-run on native Windows at that point."
- `Dedup patch width` (`docs/reviews/T030-quinn/README.md:14–15`) — the 129-packet window drops ages 129–2048 as duplicates at unit level; the patch is "2048 history bits (`[u64; 32]`) plus the separately stored highest packet → **effective window 2049**".
- `Dedup patch shape` (`docs/reviews/T030-quinn/README.md:77–92`) — it touches only `src/connection/spaces.rs`; `WINDOW_BITS = 2048`, `WINDOW_SIZE = 2049`, `insert`, `smallest_missing_in_interval` and three unit tests are rewritten width-generically.
- `Dedup patch memory` (`docs/reviews/T030-quinn/README.md:17`) — `Dedup` 32 → 264 bytes, "+696 bytes per connection, no heap allocation".
- `Dedup end-to-end result` (`docs/reviews/T030-quinn/README.md:18`) — "Did not reproduce. 0 dedup discards in all 26 real QUIC runs"; reorder depth at the relay was ≤ 29.
- `Dedup patch source` (`docs/reviews/T030-quinn/dedup-window.patch:1–31`) — the patch against `src/connection/spaces.rs`: `type Window = u128` becomes `[u64; WINDOW_WORDS]` with `WINDOW_WORDS = 32`, `WINDOW_BITS = 2048`, `WINDOW_SIZE = 1 + WINDOW_BITS`.
- `Vendored tree today` (`third_party/quinn-proto/crate/src/connection/spaces.rs:480–483`) — still `type Window = u128;` and `const WINDOW_SIZE: u64 = 1 + mem::size_of::<Window>() as u64 * 8;` (129), so the dedup patch is not applied on this branch.
- `Impairment probe harness` (`docs/reviews/T030-quinn/README.md:148–166`) — `probe/` (isolated crate with its own `[workspace]` and `Cargo.lock`) and `probe/run-matrix.sh`; a tokio UDP relay between a real quinn client and server applies `clean`, `reorder` (75 ms ± 15 ms) and `impaired` (plus 3% loss), with a raw-UDP control.
- `Impairment probe parameters` (`docs/reviews/T030-quinn/README.md:159–166`) — payload 1,000 bytes and one DATAGRAM per packet; rates 30/300/3,000 datagrams/s; `datagram_receive_buffer_size` 64 MiB, `datagram_send_buffer_size` 16 MiB; patched and unpatched builds compared through `--config patch.crates-io.quinn-proto.path=…`.
- `Impairment probe results` (`docs/reviews/T030-quinn/README.md:179–201`) — 0 dedup discards in every run on both builds; at ≥ 300/s the shortfall is sender-side loss/congestion response, which the patch does not change.
- `Impairment probe Windows gap` (`docs/reviews/T030-quinn/README.md:292–293`) — "**Native Windows/MSVC**: none of the suites, the harness or the impairment matrix have been run on Windows."
- `deny.toml` (`deny.toml:76–99`) — no `quinn-proto` entry of its own; its graph appears only in the comments explaining the `getrandom`, `rand` and `windows-sys@0.52.0` skips.

## The lane-1 codec in crpg-net

(pending)

## The host side

(pending)

## Existing tests to copy

(pending)

## Platform and dependency rules that bind it

(pending)

## Agent log

- 2026-11-15 (UTC) · deepseek-v4.1-flash + T150 facts · Opened the sheet with its section headings so the architect has a stable skeleton to fill.
