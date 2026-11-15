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

Module and wire facts. All items are implemented on this branch (T026p1–T026p5).

- `movement_v4` (`crates/crpg-net/src/movement_v4.rs:1`, declared `crates/crpg-net/src/lib.rs:50`) — "Lane-1 movement subprotocol (T026p2, ADR-0036 L1–L6, D13)".
- `outbox_v4` (`crates/crpg-net/src/outbox_v4.rs:1`, declared `crates/crpg-net/src/lib.rs:51`) — "The client's lane-1 sender state (T026p5, ADR-0036 L2–L3)".
- `subprotocol identity` (`movement_v4.rs:3–8`) — "Lane 1 is its own separately versioned subprotocol: every message starts with [`MOVEMENT_VERSION`] (`0xA1`)"; every lane-0 codec refuses lane-1 bytes with `UnsupportedVersion` and this codec refuses every other first byte the same way.
- `upstream/downstream shape` (`movement_v4.rs:10–17`) — upstream `0xA1, epoch[16], lane (1), seq, actor, tag`; downstream `0xA1, lane (1), kind`; a sample is "one self-contained datagram of at most [`MAX_MOVE_DOWN_BYTES`] bytes and [`MAX_SAMPLE_ENTRIES`] entries, with no `event_seq` and no epoch".
- `decode precedence` (`movement_v4.rs:19–21`) — "whole-message cap, empty input, version, header fields, body (each field checked when read), trailing-byte reject"; encode "never returns partial bytes".
- `MOVEMENT_VERSION` (`movement_v4.rs:32`) — `0xA1`.
- `LANE_MOVEMENT` (`movement_v4.rs:34`) — `1`.
- `MOVE_TAG_MOVE_TO` (`movement_v4.rs:36`) — `0`; `MOVE_TAG_HALT` (`movement_v4.rs:38`) — `1`.
- `DOWN_KIND_RECEIPT` (`movement_v4.rs:40`) — `0`; `DOWN_KIND_SAMPLE` (`movement_v4.rs:42`) — `1`.
- `OUTCOME_TAG_ACCEPTED`/`_SUPERSEDED`/`_REJECTED` (`movement_v4.rs:44–48`) — `0`, `1`, `2`.
- `MAX_MOVE_DOWN_BYTES` (`movement_v4.rs:50`) — `1_024`, "Largest lane-1 downstream message, receipt or sample".
- `MAX_SAMPLE_HEADER_BYTES` (`movement_v4.rs:53`) — `24`; `MAX_SAMPLE_ENTRY_BYTES` (`:55`) — `26`; `MAX_SAMPLE_ENTRIES` (`:57`) — `38`.
- `sample bounds assertion` (`movement_v4.rs:58–64`) — two `const` assertions pin 24 + 38 × 26 ≤ 1,024 < 24 + 39 × 26, so the entry and byte bounds change together.
- `POLICY_MOVE_INTENTS_PER_SEC` (`movement_v4.rs:67`) — `20`; `POLICY_MOVE_INTENT_BURST` (`:69`) — `40`; `POLICY_MOVE_IN_FLIGHT` (`:71`) — `8`; all "Policy, never wire".
- `MAX_MOVE_UP_BYTES` (none found) — `grep` over `crates/` finds no such constant; an intent is capped by `MAX_INTENT_FRAME_BYTES` (4,096, `crates/crpg-net/src/protocol.rs:58`) at `movement_v4.rs:229`, and the largest lane-1 intent is 54 bytes (`tasks/T026p.md:607`).
- `MAX_MOVE_RECEIPT_BYTES` (none in `crpg-net`) — the 60-byte receipt bound is defined only by the T037 contract (`tasks/T037.md:459`), not in this crate.
- `MoveBody` (`movement_v4.rs:76`) — `MoveTo { destination: Point }` (tag 0) and `Halt` (tag 1).
- `MoveIntent` (`movement_v4.rs:89`) — `epoch: [u8; 16]`, `seq: u64`, `actor: NetId`, `body: MoveBody`; "there is no `observed_tick`".
- `MoveOutcome` (`movement_v4.rs:102`) — `Accepted { start_tick }`, `Superseded`, `Rejected(RejectionCode)`.
- `MoveReceipt` (`movement_v4.rs:116`) — `epoch`, `seq`, `actor`, `processed_tick`, `outcome`; "Exactly one receipt per admitted lane-1 intent, in `seq` order."
- `SampleEntry` (`movement_v4.rs:131`) — `entity: NetId`, `point: Point`, `moving: bool`.
- `MoveSample` (`movement_v4.rs:143`) — `server_tick`, `processed_seq`, `entries: Vec<SampleEntry>`.
- `MoveDown` (`movement_v4.rs:154`) — `Receipt(MoveReceipt)` or `Sample(MoveSample)`.
- `encode_intent` (`movement_v4.rs:207`) — encodes one intent, first byte `0xA1`.
- `decode_intent` (`movement_v4.rs:228`) — refuses bytes over `MAX_INTENT_FRAME_BYTES` (`:229`), checks the version (`:234`) and the contract's field order.
- `encode_down` (`movement_v4.rs:273`) — encodes a receipt or a sample.
- `decode_down` (`movement_v4.rs:318`) — "Order: cap ([`MAX_MOVE_DOWN_BYTES`]), empty, version, lane, kind, the body" (`:316–319`).
- `check_sample` (`movement_v4.rs:432`) — M-6's encode checks: `processed_seq` above `SEQ_LAST`, more than 38 entries, non-ascending entity, in that order.
- `sample_datagrams` (`movement_v4.rs:447`) — M-7: cuts one tick's entries into chunks of 38, "Every datagram is at most 1,012 bytes" (`:440–446`).
- `MoveOutbox` (`outbox_v4.rs:54`) — "The client's lane-1 sender state for one epoch": `epoch: [u8; 16]`, `next_seq`, `pending: Vec<MoveIntent>`, `processed_seq`.
- `MoveOutbox::new` (`outbox_v4.rs:63`), `epoch` (`:73`), `next_seq` (`:78`), `processed_seq` (`:83`), `pending` (`:88`), `send` (`:93`), `receive` (`:113`).
- `OutboxError` (`outbox_v4.rs:17`) — `Exhausted`, `Full`, `WrongEpoch`, `Unexpected { expected, got }`, `WrongActor { seq }`; `Display` is `"<Variant> at movement/outbox"` (`:38–48`).
- `v4 subprotocol identifiers` (`crates/crpg-net/src/codec_v4.rs:6`) — "a first byte other than 4 (including 1, 2, 3, `0x81`, `0x82`, `0x91`, `0xA1`) is `UnsupportedVersion`".
- `lane table` (`tasks/T026p.md:166`) — lane 1 is `0xA1`; `tasks/T026p.md:173` records that the earlier reading which put lane 1 under byte 4 was rejected by review.
- `MAX_MOVE` grep (`crates/crpg-net/src/`) — the only matches are `MAX_MOVE_DOWN_BYTES` (`movement_v4.rs:50`) plus `MAX_MOVED_POINTS` (`protocol_v4.rs:41`, 64 points, lane 0's `Moved`), which is not a lane-1 constant.
- `lane-1 policy constants are host-enforced` (`crates/crpg-net/AGENTS.md`, T026p2 entry) — "The policy constants are host-enforced and never wire."

## The host side

`crates/crpg-server/src/quic.rs` as built (T023b/T024h2):

- `QuicHost` (`crates/crpg-server/src/quic.rs:430`) — "one [`Host`] plus one `crpg_net_quic::QuicServer`, driven by a synchronous, non-blocking [`QuicHost::pump`]" (`:1–3`).
- `module invariants` (`quic.rs:21–26`) — "This module reads no clock, spawns no thread, opens no socket"; `now_ms` is injected; `wait`, `shutdown` and `Drop` are the only blocking calls.
- `start` (`quic.rs:466`) — takes a host and a bound server; the server "must come from `QuicServer::bind` or `QuicServer::bind_dual`, never `QuicServer::bind_resumable`" (`:462–465`).
- `pump` (`quic.rs:501`) — preconditions (`Poisoned`, then `TimeRegression`), then `run_phases`.
- `run_phases` (`quic.rs:521–593`) — the seven phases: events, drain, admit (`host.pump`), reconcile epochs, deliver, retire closed peers, report.
- `on_hello` (`quic.rs:596`) — credential match (`:613–617`), wire version (`:618–622`), supersession (`:623–630`), framing from the negotiated ALPN (`:634`), bind (`:650–671`), welcome (`:687–692`), `server.accept` (`:694`), `Bound` record (`:696–710`).
- `bind by framing` (`quic.rs:650–659`) — `Framing::V2` calls `Host::bind_snapshot_peer`, `Framing::V1` calls `Host::bind_peer`; there is no other framing and no v4 branch.
- `drain` (`quic.rs:768–837`) — a held frame is retried first (`:774–787`), then up to `MAX_DRAIN_PER_CONNECTION` frames (`:56–58`) via `server.try_recv_channel` (`:790`).
- `channel routing` (`quic.rs:795–809`) — `Channel::Lane0` → `ingest_one`, `Channel::Transfer` → `ingest_transfer_one`, `Channel::Resume` → fence ("Unreachable (Q§8.2)").
- `ingest_one` (`quic.rs:852`) — calls `self.host.ingest(peer, frame, now_ms)` (`:862`) and applies the B§6 step-3 disposition; `ingest_transfer_one` (`:891`) is the transfer twin.
- `deliver` (`quic.rs:949`) — takes `host.take_delivery_entries(peer)` (`:958`), maps `DeliveryKind::Delta`/`Transfer` to `Channel::Lane0`/`Channel::Transfer` (`:994–997`) and writes with `server.try_send_channel` (`:998`).
- `acknowledge` (`quic.rs:1062`) — acknowledges delivery by the last accepted entry's ordinal (`:1039`); a departed peer's entries are discarded and acknowledged (`:973–989`).
- `closed` handling (`quic.rs:528–539`, `:567–585`) — `ServerEvent::Closed` marks the binding, then phase 6 unbinds and retires it once drained.
- `Wire version today` (`crates/crpg-server/src/host.rs:1195–1202`) — `session_wire` returns `3` for a v3 session else `config.protocol.as_u8()` (1 or 2); `ProtocolSelection` has only `V1` and `V2` (`host.rs:104–111`).
- `Frozen` (`tasks/T037.md:1338`) — `crates/crpg-server/src/quic.rs` is a frozen source in T037, and `:1355` says "`quic.rs` is frozen here."

The T037 surface the transport task must call. `tasks/T037.md` is an approved contract for code not yet written: `grep` finds no `bind_v4_peer`, `ingest_move`, `take_move_receipts` or `take_samples` in `crates/` today.

- `bind_v4_peer` (`tasks/T037.md:437–441`) — future: "Binds a protocol v4 session: `bind_v3_peer` with lane-0 version 4 and lane 1 (B-1)".
- `session_wire = 4` (`tasks/T037.md:447–450`) — future: `session_wire` returns `4` for a v4 session, and `DeliveryKind` gains "or v4".
- `ingest_move` (`tasks/T037.md:483–484`) — future: `pub fn ingest_move(&mut self, peer: PeerHandle, bytes: &[u8], now_ms: u64) -> Result<MoveAdmission, HostError>`.
- `MoveAdmission` (`tasks/T037.md:470–481`) — future: `Admitted { seq }`, `Rejected { seq, code }`, `Duplicate { seq, resent }`, `Refused { seq: Option<u64>, code }`.
- `ingest_move precedence` (`tasks/T037.md:710–717`, D-1) — future: closed, time, unbound, shared buckets, the 4,096-byte cap, ingress room, then HM-2 (`:719–741`); nothing is staged on a refusal.
- `take_move_receipts` (`tasks/T037.md:487`, `:788–790`) — future: returns "every released lane-1 receipt not yet taken, in release order"; taking them "hands them to the reliable stream".
- `take_samples` (`tasks/T037.md:462`, `:697–699`) — future: returns the queued sample datagrams and empties the queue.
- `MAX_MOVE_RECEIPT_BYTES` (`tasks/T037.md:459`) — future: `60`, "the largest receipt".
- `POLICY_SAMPLE_REFRESH_TICKS` (`tasks/T037.md:456`) — future: `20` ticks between full position refreshes.
- `lane-1 egress` (`tasks/T037.md:701–707`) — future: `V4State.egress` counts lane-1 frames and bytes and the host's `egress_bytes` includes them.
- `Transport obligations` (`tasks/T037.md:1351–1355`) — future, the T150 scope: "the `quic.rs` route that binds v4 sessions and moves `ingest_move`, `take_move_receipts` and `take_samples` bytes".

## Existing tests to copy

`crates/crpg-net-quic/tests/`:

- `quic.rs` (`crates/crpg-net-quic/tests/quic.rs`, 2,428 lines) — the 30-case endpoint suite (T023 1–23, T023c 24–27, T023v 28, T023d 29–30); 32 `#[test]` functions on this branch.
- `quic_gaps.rs` (`crates/crpg-net-quic/tests/quic_gaps.rs`, 282 lines, 7 tests) — the post-T023 gap cases.
- `channel.rs` (`crates/crpg-net-quic/tests/channel.rs`, 2,742 lines, 21 tests) — the T024q `/2` channel suite, including one impaired relay (`:1428`, `:1505`).
- `close_budget.rs` (`crates/crpg-net-quic/tests/close_budget.rs`, 197 lines, 3 tests) — the T098g flush reserve.
- `support/quic_rig.rs` (`crates/crpg-net-quic/tests/support/quic_rig.rs`) — shared helpers, `#[path]`-included by each suite.
- `fixtures/` (`crates/crpg-net-quic/tests/fixtures/`) — `server_a.cert.der`, `server_a.key.pk8.der`, `server_b.*`; `.gitattributes:7` marks them binary and `crates/crpg-net-quic/AGENTS.md` says they are test-only.

`support/quic_rig.rs` helpers:

- `loopback` (`quic_rig.rs:35`), `identity_a` (`:39`), `pin_a` (`:43`), `pin_b` (`:47`).
- `bind_server` (`quic_rig.rs:51`), `client_config` (`:60`), `credential` (`:68`), `hello` (`:72`), `epoch` (`:79`).
- `connect_async` (`quic_rig.rs:85`), `until` (`:95`, a 30 s guard), `next_event` (`:104`), `next_hello` (`:116`), `next_closed` (`:128`), `connect_accepted` (`:136`), `server_recv` (`:162`), `client_recv` (`:176`).
- `pattern` (`quic_rig.rs:191`), `cycling_size` (`:202`).
- `RawRuntime` (`quic_rig.rs:212`), `RawClient` (`:310`), `raw_connect` (`:317`), `raw_lane` (`:349`), `frame_bytes` (`:353`) — a real quinn connection that bypasses the crate API.
- `RelayConfig` (`quic_rig.rs:425`) — `drop_every`, `reorder_depth`, `seed`; `clean()` (`:432`).
- `UdpRelay` (`quic_rig.rs:454`) — "A client ↔ server UDP relay"; `set_impaired` (`:671`) and `set_blackhole` (`:701`); its doc (`:449–453`) says it "forwards every datagram in order until `set_impaired(true)`, so a test can complete its handshake on a clean path and only then measure loss and reordering" (T098d).
- `SplitMix64` (`quic_rig.rs:464`) and block rotation (`Direction::offer`, `:491`, `:517`) — "Block rotation: the first packet goes out last", so reorder depth stays bounded.

Impairment in the crate suite:

- `impaired()` (`crates/crpg-net-quic/tests/quic.rs:1174–1178`) — `drop_every: 13`, `reorder_depth: 4`, `seed: 23`.
- `connect_via_relay` (`quic.rs:1182`) — connect over the relay before impairment.
- `ordered_exactly_once_through_impaired_relay` (`quic.rs:1200`) — case 14; the handshake runs clean (T098d), then `relay.set_impaired(true)` (`:1212`), with `dropped`/`reordered` as controls (`:1221–1222`).
- `lane0_codec_frames_decode_after_impaired_delivery` (`quic.rs:1826`) — the impaired codec case, plus a clean positive control (`:1833–1834`).
- `datagrams_and_connection_limit_fail_closed` (`quic.rs:1652`) — the case that pins DATAGRAM frames as unsupported today.
- `transport_trait_round_trip` (`quic.rs:1708`) — `Transport` via `ServerLane`.

`crates/crpg-server/tests/`:

- `host_quic.rs` (`crates/crpg-server/tests/host_quic.rs`, 2,441 lines, 11 tests) — the T023b acceptance suite over real loopback; oracles are a mirror `HistoryWorld`, a per-client replica from received bytes, and a trace replay (`:1–19`).
- `host_quic_transfer.rs` (`crates/crpg-server/tests/host_quic_transfer.rs`, 1,901 lines, 9 tests) — the T024h2 `/2` adapter suite.
- `support/quic_host_rig.rs` (`crates/crpg-server/tests/support/quic_host_rig.rs`) — entity/ability fixtures (`eid`, `a`, `b`, `c`, `strike`, `smite`, `whiff`, `encounter`), `fixture` (`:99`), grants (`:108–157`), frame builders (`intent_header` `:195`, `declare` `:206`, `end_turn` `:223`), `TestClock` (`:541`, `now` `:550`, `advance` `:554`), `start_dual` (`:319`), `connect_framed_async` (`:330`), `connect_raw` (`:347`), `register` (`:592`), `pump` (`:602`).
- `fixtures/three_combatants.json` (`crates/crpg-server/tests/fixtures/three_combatants.json`) — the generated `HistoryWorld` fixture (seed 7, A/B/C health 10/10/3); `crates/crpg-server/AGENTS.md` warns it is generated, not hand-written.
- `both_versions!` (`host_quic.rs:47`) — runs a case body under both `ProtocolSelection`s.
- `exchange` (`host_quic.rs:144`), `mirror` (`:161`), `register` (`:170`), `settle` (`:198`), `replay` (`:701`), `three_clients` (`:813`), `lockstep_quic` (`:880`), `lockstep_memory` (`:910`), `IMPAIRED` (`:805–809`, the same 13/4/23 config).

Known flakes:

- `quic case 13` — `ordering_preserved_under_concurrent_traffic` (`crates/crpg-net-quic/tests/quic.rs:974`) is the case `tasks/T098.md` tracks; `tasks/T098.md:1–5` says "Status: **open, cause unknown**" and lists a Windows failure where a client's flush gave up with 8 frames undelivered.
- `T098f` (`tasks/BACKLOG.md:248`) — measured as product-side: a `Flush` close that spends the whole budget dropped the I/O runtime before `CONNECTION_CLOSE` left.
- `T098g`/`T098h` (`tasks/BACKLOG.md:249–250`) — the close reserve (R = min(200 ms, B/4)) and `FlushClose::flush_close` are the fixes; `crates/crpg-net-quic/AGENTS.md` forbids removing the exit yield.
- `T098d` (`tasks/BACKLOG.md:174`) — the relay now starts unimpaired and `set_impaired(true)` runs after the handshake; 20/20 alone and 16/16 at 8× contention.
- `host_quic quic_admitted_schedule_matches_in_memory` (`crates/crpg-server/tests/host_quic.rs:974`) — T147's measurement row (`tasks/BACKLOG.md:234`) records "deadline 7/30 on master, 4/30 on the batch; access violation 3 times under load, 0/60 quiet", and T147 is parked with temporary logging on `task/T147-m1` that "must not merge".

The impairment probe:

- `probe/` (`docs/reviews/T030-quinn/probe/`) — `Cargo.toml`, `Cargo.lock`, `src/main.rs`, `run-matrix.sh`; an isolated crate with its own `[workspace]` (`docs/reviews/T030-quinn/README.md:150–152`).
- `README §6` (`docs/reviews/T030-quinn/README.md:148–204`) — harness, parameters, the five findings and the interpretation.
- `README §10` (`docs/reviews/T030-quinn/README.md:290–302`) — the native Windows/MSVC blocker and the "end-to-end symptom … not reproduced" note.
- `results/` (`docs/reviews/T030-quinn/results/`) — `linux-impairment-summary.md`, `linux-impairment-matrix.jsonl` and the suite logs referenced at `README.md:176–177`.
- `grep impair` (`crates/`) — the only in-repo impairing relay is `UdpRelay` (`crates/crpg-net-quic/tests/support/quic_rig.rs:454`); the T030 probe is not part of the workspace and no `crpg-net-quic` test runs it.

## Platform and dependency rules that bind it

- `Platform invariant` (`AGENTS.md:6–19`) — `x86_64-pc-windows-msvc` "is primary for development, product, release gates, and behavioural baselines (client, editor, embedded single-player server, dedicated server, CLI)"; `x86_64-unknown-linux-gnu` "is fully supported for dedicated server, headless CLI/tooling, server-side extensibility, and tests".
- `Replay determinism` (`AGENTS.md:20–24`) — exact-build, target-scoped goldens, never cross-platform hashes; "no tolerance or missing-baseline skips".
- `Never import upward` (`AGENTS.md:28–37`) — "core <- data <- rules <- sim <- {net, ai, script} <- server", the `ALLOWED` table in `tools/lint/deps.py` is authoritative, and a crate missing from it is itself a failure.
- `unsafe` (`AGENTS.md:25–27`) — only `crpg-godot` may use it; every other crate root carries `#![forbid(unsafe_code)]` (`crates/crpg-net-quic/src/lib.rs:1` has it).
- `One task = one crate` (`AGENTS.md:57`) — "If a task needs two crates, stop and say so." T150's stated scope names both `crpg-net-quic` and the `crpg-server` `quic.rs` route (`tasks/T037.md:1351–1355`).
- `No new dependencies` (`AGENTS.md:52`) — "Do not add dependencies without being asked."
- `ADR-0012 Windows primary` (`docs/adr/0012-windows-primary-platform.md:17–24`) — Windows/MSVC is primary for development, product, release gating and behavioural baselines; Linux/GNU is fully supported, and "Platform-specific failures are defects, not best effort."
- `ADR-0012 one server` (`docs/adr/0012-windows-primary-platform.md:25–29`) — "One platform-neutral authoritative server implementation" serves Windows single-player and Windows/Linux dedicated hosts; OS-specific logic stays above core/rules/sim and "grants no dependency permission".
- `ADR-0012 exact-build` (`docs/adr/0012-windows-primary-platform.md:33–36`) — Windows and Linux compare independently generated target-scoped results; "Cross-platform hash equality is not required."
- `ADR-0012 targets` (`docs/adr/0012-windows-primary-platform.md:40–41`) — the gate targets are `x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu`.
- `deps.py layering` (`tools/lint/deps.py:46–72`) — the `ALLOWED` table: `crpg-net-quic` may import only `crpg-net` (`:55–59`), and `crpg-server` may import any crate but `crpg-godot` and `crpg-edit` (`:69`).
- `deps.py I/O-free` (`tools/lint/deps.py:13–15`, `:77–82`) — `crpg-core`, `crpg-data`, `crpg-rules`, `crpg-sim` and `crpg-net` "may not name tokio, quinn, quinn-proto, quinn-udp, rustls, ring, mio or socket2 in any section"; `IO_FREE_CRATES` and `IO_CRATES` hold the exact lists.
- `deps.py scans every section` (`tools/lint/deps.py:22–34`) — `[dependencies]`, `[dev-dependencies]`, `[build-dependencies]`, `[target.*]` blocks and renamed entries all count.
- `crate I/O permission` (`crates/crpg-net-quic/AGENTS.md:47–49`) — "I/O is allowed here, crate-wide … and nowhere below it. `crpg-net` stays I/O-free."
- `crate sync surface` (`crates/crpg-net-quic/AGENTS.md:50–53`) — every public call is non-blocking except `wait`, `connect`, `close`, `shutdown` and `Drop`, and no tokio, quinn or rustls type appears in a public signature.
- `no DATAGRAM without a new ALPN` (`crates/crpg-net-quic/AGENTS.md:125`) — "Never enable DATAGRAM frames or more streams without a new ALPN."
- `no dev-dependencies` (`crates/crpg-net-quic/AGENTS.md:85–87`) — "There are **no dev-dependencies** … Any new edge or feature needs a T018a-style record and approval."
- `D23 pins` (`crates/crpg-net-quic/AGENTS.md:73–83`) — the four pins, default features off, `quinn-proto` "vendored with patches, `third_party/quinn-proto/VENDOR.md`".
- `crate DoD` (`crates/crpg-net-quic/AGENTS.md:89–115`) — `clippy --all-targets --locked -D warnings`, the five test suites, `cargo deny check`, and the scope grep that "must print only the `#![forbid(unsafe_code)]` line"; run "on native Windows/MSVC (CI) and genuine Linux/GNU".
- `server I/O fence` (`crates/crpg-server/AGENTS.md:82–85`) — "network I/O exists **only** in `quic.rs`, only through `crpg-net-quic`'s API, which owns the socket, runtime and thread"; "No `HashMap`/`HashSet`."
- `server deps` (`crates/crpg-server/AGENTS.md:92–106`) — normal deps plus `crpg-persist` and the `crpg-net-quic` path edge, "and nothing else: there is no direct quinn, quinn-proto, rustls or tokio edge"; "never widen `deny.toml`."
- `server DoD` (`crates/crpg-server/AGENTS.md:108–122`) — `cargo clippy -p crpg-server --all-targets --locked -D warnings`, `host_quic`, the whole crate, both lints, `cargo deny check` and `git diff --check`.

## Agent log

- 2026-11-15 (UTC) · opencode-go/deepseek-v4.1-flash + T150 facts · Recorded the lane-1 requirements, the as-built `crpg-net-quic` surface and lane-0 stream mapping, the vendored quinn-proto state and D23's dedup-patch/probe obligation, the `movement_v4`/`MoveOutbox` codec, the `quic.rs` adapter and T037's future calls, the test rigs and the binding platform rules, so the T150 architect cites existing lines instead of re-deriving them.
