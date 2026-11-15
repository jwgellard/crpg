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

(pending)

## The vendored quinn-proto and D23

(pending)

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
