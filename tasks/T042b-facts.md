# T042b facts — hostile-server suite for the client's own decoders

Fact sheet for T042b (`crpg-net`): a hostile-server suite mirroring T042a,
about 500 test lines (`tasks/BACKLOG.md:350`). Facts only; no design.

## Requirements

Spec (`docs/CRPG_ENGINE_SPEC.md`):
- `docs/CRPG_ENGINE_SPEC.md:1488` — `crpg-net` malicious-client tests as a first-class suite: oversized packets, malformed frames, intents for entities not owned, intents at illegal times, flooding, replay of old packets, requesting entities outside the interest set; clean rejection and a log line, never a panic.
- `docs/CRPG_ENGINE_SPEC.md:1586` — Phase 4 deliverables include the malicious-client suite.
- `docs/CRPG_ENGINE_SPEC.md:1587` — Tests: desync over 5,000 ticks; every malicious-client case; reconnection.
- `docs/CRPG_ENGINE_SPEC.md:1703` — §19.2 item 21: the malicious-client test suite.
- `docs/CRPG_ENGINE_SPEC.md:1774` — risk: build the simulated-network transport and the malicious-client suite in Phase 4.

ADRs (decode):
- `docs/adr/0022-host-session-capture-retirement.md:56` — unknown delta tags proved by decoder tests, not by host capture.
- `docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:46` — the transport never decodes intent or delta payloads.
- `docs/adr/0031-host-snapshot-sender-and-snapshot-sessions.md:68` — host order: unbound → not-snapshot-peer → rate spend → decode → epoch.
- `docs/adr/0032-resume-subprotocol-and-connection-fencing.md:75` — decode with `decode_request`, then build the `ResumeDecision`.
- `docs/adr/0033-godot-bridge.md:152` — bridge step: `codec_v2::decode_delta` then apply to replica (T046).
- `docs/adr/0035-client-replica.md:12` — bridge decodes frames; replica applies whole decoded values; codec faults stay code 52.
- `docs/adr/0035-client-replica.md:80` — T047 calls `apply_delta` with every decoded lane-0 frame and treats `Err` as fatal.

Implementation plan:
- `docs/IMPLEMENTATION_PLAN.md:247` — T042 row: wire-level malicious-client suite; host-level cases live in T041.

`tasks/T042.md` (lines naming T042b or the server-to-client direction):
- `tasks/T042.md:42` — the wire level is the server's byte boundary: eight decoders a client's bytes reach.
- `tasks/T042.md:90` — pools, replicas and `MoveOutbox` are the client's receivers (input is the server's bytes).
- `tasks/T042.md:99` — the first rejected cell is a downstream decoder (v2 `Health.dead`).
- `tasks/T042.md:375` — `C81`: `codec_transfer::decode_down` control.
- `tasks/T042.md:376` — `C82`: `transfer_v3::decode_down` control.
- `tasks/T042.md:377` — `J91`: `codec_resume::decode_down` control.
- `tasks/T042.md:378` — `Z1`–`Z4`: `decode_delta` control.
- `tasks/T042.md:379` — `K1`: `movement_v4::decode_down` receipt control.
- `tasks/T042.md:651` — out of scope: the client's decoders and receivers.
- `tasks/T042.md:667` — D67b answer: T042b filed, tied to player-hosted games (D65b).
- `tasks/T042.md:680` — Question 2: should the client's own decoders get the suite.
- `tasks/T042.md:685` — T042b, same shape, about 500 test lines.

Decisions:
- `tasks/DECISIONS-D65.md:11` — D65b: joining a running campaign possible by design, built later.
- `tasks/DECISIONS-D67.md:3` — D67b answered: (a) now plus a backlog row for (b).
- `tasks/DECISIONS-D67.md:14` — D67b: client decodes what the server sends; a hostile server or bug could send bad bytes.
- `tasks/DECISIONS-D67.md:19` — (b) T042b, same shape, about 500 test lines.
- `tasks/DECISIONS-D67.md:21` — recommend (a) now, plus a backlog row for (b) tied to player-hosted games (D65b).

Backlog:
- `tasks/BACKLOG.md:350` — T042b row.

## The T042a pattern

Test file: `crates/crpg-net/tests/malicious_client.rs` (1598 lines, 14 tests).

Helpers and case groups (one line each):
- `EPOCH: [u8; 16]` (`crates/crpg-net/tests/malicious_client.rs:27`)
- `ability() -> Ulid` (`crates/crpg-net/tests/malicious_client.rs:32`)
- `net(n: u64) -> NetId` (`crates/crpg-net/tests/malicious_client.rs:36`)
- `tid(n: u64) -> TransferId` (`crates/crpg-net/tests/malicious_client.rs:40`)
- `enum Wire { V1, V2, V3, V4, T81, T82, R91, A1 }` (`crates/crpg-net/tests/malicious_client.rs:46`)
- `WIRES: [Wire; 8]` (`crates/crpg-net/tests/malicious_client.rs:58`)
- `Wire::first_byte / lane_at / lane / verdict` (`crates/crpg-net/tests/malicious_client.rs:71` / `crates/crpg-net/tests/malicious_client.rs:85` / `crates/crpg-net/tests/malicious_client.rs:93` / `crates/crpg-net/tests/malicious_client.rs:105`)
- `reencoded<T>(wire, bytes, decoded, encode) -> Verdict` (`crates/crpg-net/tests/malicious_client.rs:159`)
- `Verdict { Refused(CodecError), Accepted(Vec<u8>) }` (`crates/crpg-net/tests/malicious_client.rs:178`)
- `Fixture { name, wire, bytes, tag_at }` (`crates/crpg-net/tests/malicious_client.rs:186`)
- `fixtures() -> Vec<Fixture>` (`crates/crpg-net/tests/malicious_client.rs:195`)
- `h / declare / end_turn / transfer_up` builders (`crates/crpg-net/tests/malicious_client.rs:443` / `crates/crpg-net/tests/malicious_client.rs:450` / `crates/crpg-net/tests/malicious_client.rs:458` / `crates/crpg-net/tests/malicious_client.rs:464`)
- `pinned() -> Vec<Fixture>` (`crates/crpg-net/tests/malicious_client.rs:471`)
- `with_byte(bytes, at, value) -> Vec<u8>` (`crates/crpg-net/tests/malicious_client.rs:602`)
- `padded(bytes, len) -> Vec<u8>` (`crates/crpg-net/tests/malicious_client.rs:609`)
- `stable(wire, bytes) -> bool` (`crates/crpg-net/tests/malicious_client.rs:617`)
- `XorShift64(u64)` with `next(&mut self) -> u64` (`crates/crpg-net/tests/malicious_client.rs:628` / `crates/crpg-net/tests/malicious_client.rs:631`)
- `SEED: u64` (`crates/crpg-net/tests/malicious_client.rs:639`)
- `fixture<'a>(all, name) -> &'a Fixture` (`crates/crpg-net/tests/malicious_client.rs:641`)
- `tag_expected(name, t, original, mutated) -> Verdict` (`crates/crpg-net/tests/malicious_client.rs:944`)

Case groups (tests):
- `fixtures_are_pinned` (`crates/crpg-net/tests/malicious_client.rs:652`)
- `short_inputs_are_refused_exactly` (`crates/crpg-net/tests/malicious_client.rs:707`)
- `oversize_is_refused_first` (`crates/crpg-net/tests/malicious_client.rs:765`)
- `every_truncation_is_malformed` (`crates/crpg-net/tests/malicious_client.rs:815`)
- `trailing_bytes_are_malformed` (`crates/crpg-net/tests/malicious_client.rs:842`)
- `foreign_first_bytes_are_unsupported` (`crates/crpg-net/tests/malicious_client.rs:872`)
- `foreign_lanes_are_wrong_direction` (`crates/crpg-net/tests/malicious_client.rs:916`)
- `tag_matrix` (`crates/crpg-net/tests/malicious_client.rs:986`)
- `zero_ids_and_field_order` (`crates/crpg-net/tests/malicious_client.rs:1015`)
- `hostile_lengths_are_bounded` (`crates/crpg-net/tests/malicious_client.rs:1127`)
- `downstream_frames_sent_up_are_refused` (`crates/crpg-net/tests/malicious_client.rs:1208`)
- `single_byte_corruption_is_refused_or_exact` (`crates/crpg-net/tests/malicious_client.rs:1361`)
- `seeded_noise_is_stable` (`crates/crpg-net/tests/malicious_client.rs:1460`)
- `refusal_texts_are_fixed` (`crates/crpg-net/tests/malicious_client.rs:1579`)

Count: 14 tests; corruption pass runs 128,265 single-byte inputs; seeded pass runs 17,920 inputs.

## Client-side decoders today

`crpg-net` decoders for bytes a server sends to a client (each `pub fn`, error type in signature):
- `crates/crpg-net/src/codec.rs:421` `pub fn decode_delta(bytes: &[u8]) -> Result<DeltaFrame, CodecError>` (v1)
- `crates/crpg-net/src/codec_v2.rs:595` `pub fn decode_delta(bytes: &[u8]) -> Result<DeltaFrame, CodecError>` (v2)
- `crates/crpg-net/src/codec_v3.rs:1103` `pub fn decode_delta(bytes: &[u8]) -> Result<DeltaFrame, CodecError>` (v3)
- `crates/crpg-net/src/codec_v4.rs:428` `pub fn decode_delta(bytes: &[u8]) -> Result<DeltaFrame, CodecError>` (v4)
- `crates/crpg-net/src/codec_transfer.rs:97` `pub fn decode_body(bytes: &[u8]) -> Result<SnapshotBody, CodecError>` (0x81 snapshot body)
- `crates/crpg-net/src/codec_transfer.rs:246` `pub fn decode_down(bytes: &[u8]) -> Result<TransferDown, CodecError>` (0x81 downstream)
- `crates/crpg-net/src/transfer_v3.rs:258` `pub fn decode_body(bytes: &[u8]) -> Result<SnapshotBodyV3, CodecError>` (0x82 snapshot body)
- `crates/crpg-net/src/transfer_v3.rs:426` `pub fn decode_down(bytes: &[u8]) -> Result<TransferDown, CodecError>` (0x82 downstream)
- `crates/crpg-net/src/codec_resume.rs:205` `pub fn decode_down(bytes: &[u8]) -> Result<ResumeDown, CodecError>` (0x91)
- `crates/crpg-net/src/movement_v4.rs:318` `pub fn decode_down(bytes: &[u8]) -> Result<MoveDown, CodecError>` (0xA1 receipts/samples)

QUIC welcome (server → client):
- `crates/crpg-net-quic/src/handshake.rs:343` `pub fn Welcome::decode(bytes: &[u8]) -> Result<Welcome, HandshakeError>`
- `crates/crpg-net-quic/src/handshake.rs:348` `pub(crate) fn Welcome::decode_for(bytes: &[u8], movement: bool) -> Result<Welcome, HandshakeError>`

## Client sessions today

- `crates/crpg-godot/src/core/client.rs:399` `pub(crate) struct Client` (replica, table, view, pending, next_seq, ended).
- `crates/crpg-godot/src/core/client.rs:434` `pub(crate) fn step(...)` drains delivered frames; `decode_delta(&bytes)` failure → `Fault { code: BridgeCode::FrameUndecodable, ... }`; `replica.apply_delta` failure → `Fault { code: BridgeCode::ReplicaRejected, ... }`.
- `crates/crpg-net-quic/src/client.rs:104` `pub struct QuicClient`.
- `crates/crpg-net-quic/src/client.rs:983` welcome body decoded with `Welcome::decode_for`; error → `invalid_welcome(&connection)` (closes with `ProtocolViolation`).
- `crates/crpg-net-quic/src/client.rs:450` / `crates/crpg-net/tests/malicious_client.rs:471` / `crates/crpg-net/tests/malicious_client.rs:509` / `crates/crpg-net/tests/malicious_client.rs:523` `try_recv*` → `Err(CloseReason)` once closed and drained.
- `crates/crpg-net-quic/src/client.rs:610` `pub fn close_reason(&self) -> Option<CloseReason>`.

## Limits and caps

`crates/crpg-net/src/protocol.rs`:
- `MAX_INTENT_FRAME_BYTES` = 4096 (`crates/crpg-net/src/protocol.rs:58`)
- `MAX_DELTA_FRAME_BYTES` = 65536 (`crates/crpg-net/src/protocol.rs:64`)
- `MAX_DELTA_OPS` = 256 (`crates/crpg-net/src/protocol.rs:70`)
- `MAX_WIRE_COLLECTION` = 256 (`crates/crpg-net/src/protocol.rs:73`)
- `MAX_WIRE_STRING_BYTES` = 256 (`crates/crpg-net/src/protocol.rs:80`)
- `MAX_WIRE_BYTES_FIELD` = 4096 (`crates/crpg-net/src/protocol.rs:83`)
- `MAX_SNAPSHOT_BYTES` = 1048576 (`crates/crpg-net/src/protocol.rs:89`)
- `MAX_VISIBLE_ENTITIES` = 1024 (`crates/crpg-net/src/protocol.rs:92`)
- `MAX_SNAPSHOT_CHUNKS` = 32 (`crates/crpg-net/src/protocol.rs:95`)

`crates/crpg-net/src/protocol_transfer.rs`:
- `MAX_SEGMENT_BYTES` = 4096 (`crates/crpg-net/src/protocol_transfer.rs:40`)
- `MAX_SEGMENTS_PER_CHUNK` = 8 (`crates/crpg-net/src/protocol_transfer.rs:42`)
- `MAX_CHUNK_BYTES` = 32768 (`crates/crpg-net/src/protocol_transfer.rs:44`)
- `MAX_DOWN_FRAME_BYTES` = 65536 (`crates/crpg-net/src/protocol_transfer.rs:46`)
- `MAX_UP_FRAME_BYTES` = 4096 (`crates/crpg-net/src/protocol_transfer.rs:48`)
- `MAX_ENCODED_BODY_BYTES` = 22552 (`crates/crpg-net/src/protocol_transfer.rs:50`)

`crates/crpg-net/src/protocol_resume.rs`:
- `MAX_RESUME_FRAME_BYTES` = 4096 (`crates/crpg-net/src/protocol_resume.rs:40`)
- `MIN_RESUME_PROOF_BYTES` = 16 (`crates/crpg-net/src/protocol_resume.rs:46`)
- `MAX_RESUME_PROOF_BYTES` = 128 (`crates/crpg-net/src/protocol_resume.rs:49`)
- `MAX_RESUME_LANES` = 4 (`crates/crpg-net/src/protocol_resume.rs:52`)

`crates/crpg-net/src/protocol_v3.rs`:
- `MAX_FACT_POOLS` = 16 (`crates/crpg-net/src/protocol_v3.rs:96`)
- `MAX_FACT_EFFECTS` = 16 (`crates/crpg-net/src/protocol_v3.rs:98`)
- `MAX_FACT_STATS` = 32 (`crates/crpg-net/src/protocol_v3.rs:100`)
- `MAX_FACT_TAGS` = 8 (`crates/crpg-net/src/protocol_v3.rs:102`)
- `MAX_FACT_NAME_BYTES` = 32 (`crates/crpg-net/src/protocol_v3.rs:104`)
- `MAX_TIMELINE_ENTRIES` = 1024 (`crates/crpg-net/src/protocol_v3.rs:106`)
- `MAX_CLOCK_ELAPSED` = 2147483647 (`crates/crpg-net/src/protocol_v3.rs:108`)
- `MAX_DELTA_HEADER_BYTES_V3` = 24 (`crates/crpg-net/src/protocol_v3.rs:115`)
- `MAX_FRAME_OP_BYTES_V3` = 65512 (`crates/crpg-net/src/protocol_v3.rs:119`)

`crates/crpg-net/src/protocol_v4.rs`:
- `COORD_LIMIT_RAW` = 1 << 29 (`crates/crpg-net/src/protocol_v4.rs:38`)
- `MAX_MOVED_POINTS` = 64 (`crates/crpg-net/src/protocol_v4.rs:41`)

`crates/crpg-net/src/movement_v4.rs`:
- `MAX_MOVE_DOWN_BYTES` = 1024 (`crates/crpg-net/src/movement_v4.rs:50`)
- `MAX_SAMPLE_HEADER_BYTES` = 24 (`crates/crpg-net/src/movement_v4.rs:53`)
- `MAX_SAMPLE_ENTRY_BYTES` = 26 (`crates/crpg-net/src/movement_v4.rs:55`)
- `MAX_SAMPLE_ENTRIES` = 38 (`crates/crpg-net/src/movement_v4.rs:57`)

`crates/crpg-net/src/transfer_v3.rs`:
- `MAX_SNAPSHOT_BYTES_V3` = 4194304 (`crates/crpg-net/src/transfer_v3.rs:43`)
- `MAX_SNAPSHOT_CHUNKS_V3` = 128 (`crates/crpg-net/src/transfer_v3.rs:47`)
- `MAX_ENCODED_BODY_BYTES_V3` = 2847788 (`crates/crpg-net/src/transfer_v3.rs:52`)

## Rules that bind it

- `tools/lint/deps.py:54` — `ALLOWED["crpg-net"] = ({"crpg-core", "crpg-data", "crpg-sim"}, set())`.
- Root `AGENTS.md:43`–`crates/crpg-net/tests/malicious_client.rs:48` — no `HashMap`/`HashSet` in `crpg-core`/`crpg-rules`/`crpg-sim`/`crpg-ai`; no `f32`/`f64` in `crpg-core`/`crpg-rules`/`crpg-ai`; `crpg-sim` `f32` spatial only.
- Root `AGENTS.md:49`–`crates/crpg-net/tests/malicious_client.rs:51` — the two bans apply to doctests too.
- Root `AGENTS.md:52` — do not add dependencies without being asked.
- Root `AGENTS.md:54` — do not weaken or delete an existing test.
- Root `AGENTS.md:66`–`crates/crpg-net/tests/malicious_client.rs:76` — CI gates (`fmt`, `clippy --all-targets`, `test`, `deps.py`, `determinism.py`, lint self-tests).
- Root `AGENTS.md:81`–`crates/crpg-net/tests/malicious_client.rs:85` — every agent doc edit signs itself.
- `crates/crpg-net/AGENTS.md:1` — crate agent contract (how-to-work rules, known traps, decode-order discipline).

## Agent log

- 2026-11-19 (UTC) · opencode-go/beam-free + T042b facts · Fact sheet for the hostile-server suite contract (client-side decoders, sessions, caps, rules), citations checked on branch.