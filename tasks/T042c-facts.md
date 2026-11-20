# T042c facts: hostile-server follow-up (`crpg-net-quic` and the Godot bridge)

Fact sheet for `T042c`. Facts only: no recommendations, no proposed types, no opinions.
Each entry cites an existing `path:line`.

## Requirements

- `crpg-net hostile-input mandate` (`docs/CRPG_ENGINE_SPEC.md:1488`): malicious-client tests are a first-class suite; every case must give a clean rejection, never a panic, and the desync test runs 5,000 ticks. This sentence names the server side only.
- `crpg-net deliverable` (`docs/CRPG_ENGINE_SPEC.md:1586`): the `crpg-net` deliverable includes the simulated-network transport and the malicious-client suite.
- `crpg-net tests` (`docs/CRPG_ENGINE_SPEC.md:1587`): the test list includes every malicious-client case and reconnection.
- `top-risk 21` (`docs/CRPG_ENGINE_SPEC.md:1703`): the malicious-client test suite is item 21 of the risk mitigation list.
- `one authoritative server` (`docs/CRPG_ENGINE_SPEC.md:969`): one platform-neutral authoritative server runs in-process on Windows single-player; it "cannot mutate authoritative state directly" (`docs/CRPG_ENGINE_SPEC.md:971`).
- `server-side refusal has no spec client-side twin` (`docs/CRPG_ENGINE_SPEC.md:1488`): no other binding sentence in the spec requires a client to refuse hostile server bytes; "none found" for `Welcome`, bridge faults and the client close.
- `ADR-0024 placement` (`docs/adr/0024-separate-crpg-net-quic-crate.md:58`): the QUIC crate owns ALPN, framing, pinning and close codes; the wire record is ADR-0025.
- `ADR-0025 no negotiation` (`docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:35`): the server accepts the one chosen wire version or refuses it; nothing is negotiated.
- `ADR-0025 welcome layout` (`docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:43`): the welcome is exactly 17 bytes, and its version must equal the hello's (`docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:50`).
- `ADR-0025 pinned trust` (`docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:56`): a certificate chain is refused; the pin is the only trust input.
- `ADR-0025 close codes` (`docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:70`): application close codes 0–9, each sent with its name; `VersionRefused` is 6 (`docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:81`).
- `ADR-0026 auth before version` (`docs/adr/0026-quic-host-adapter.md:46`): a bad credential and a wrong-length credential are `AuthRefused` (5), checked before `VersionRefused` (6) — server side.
- `ADR-0026 fencing closes` (`docs/adr/0026-quic-host-adapter.md:63`): unbind, revocation and narrowing grants close the connection with `SessionFenced` (9) in `Discard` mode — server side.
- `ADR-0033 fatal table` (`docs/adr/0033-godot-bridge.md:341`): 52 `FRAME_UNDECODABLE` is a `CodecError` on a delivered frame, fatal, "stop; show text"; 53 `REPLICA_REJECTED` (`docs/adr/0033-godot-bridge.md:342`) is T046 refusing a frame; 54 `UNEXPECTED_TRANSFER` (`docs/adr/0033-godot-bridge.md:343`) is a `DeliveryKind::Transfer` in the slice.
- `ADR-0033 BR-8` (`docs/adr/0033-godot-bridge.md:641`): a `Transfer` entry routes to fatal 54.
- `ADR-0033 BR-13` (`docs/adr/0033-godot-bridge.md:646`): a corrupt frame on the link gives fatal 52, the host is closed, and every later `poll` repeats it.
- `ADR-0035 fault split` (`docs/adr/0035-client-replica.md:13`): the replica applies whole decoded values, so codec faults stay code 52 and replica faults code 53.
- `T042 out of scope` (`tasks/T042.md:653`): T042a excludes "`crpg-net-quic` framing (ADR-0025's length prefix and close codes)".
- `T042b out of scope` (`tasks/T042b.md:1375`): the QUIC `Welcome` decode and client close (`crates/crpg-net-quic`), and the Godot bridge's fault mapping and replica refusals (`crates/crpg-godot`), are the follow-up filed as T042c (`tasks/T042b.md:1377`).
- `T042b case tables` (`tasks/T042b.md:391`): the delta op-tag table; (`tasks/T042b.md:410`) the header/op-count rows; (`tasks/T042b.md:438`) the Health/Receipt rows; (`tasks/T042b.md:593`) the ActionResolved rows; (`tasks/T042b.md:715`) the Moved/Position rows.
- `T042b mutant tables` (`tasks/T042b.md:1300`): T042b1's MA1–MA12; (`tasks/T042b.md:1315`) MB1–MB10; (`tasks/T042b.md:1330`) MC1–MC10; (`tasks/T042b.md:1345`) MD1–MD10.
- `D65b` (`tasks/DECISIONS-D65.md:11`): "open my campaign to others" is built later, after the demo; the run loop must let one `Host` serve an in-memory local player and QUIC players at once (`tasks/DECISIONS-D65.md:15`).

## T042b today

- `hostile_delta.rs` (`crates/crpg-net/tests/hostile_delta.rs:1`): T042b1, the four lane-0 delta decoders on `EntityEnter`, `Health` and `Receipt`; private `Wire` enum (`:28`) and `Fixture` (`:84`).
- `hostile_delta_ops.rs` (`crates/crpg-net/tests/hostile_delta_ops.rs:1`): T042b2, the same four decoders on the v2 event op, the v3 fact ops and the v4 movement ops; private `Wire` (`:36`) and `Fixture` (`:92`).
- `hostile_transfer.rs` (`crates/crpg-net/tests/hostile_transfer.rs:1`): T042b3, the `0x81`/`0x82` downstream frames and the two snapshot bodies; private `Subject` (`:33`) and `Fixture` (`:112`).
- `hostile_session.rs` (`crates/crpg-net/tests/hostile_session.rs:1`): T042b4, the `0x91` resume downstream and the `0xA1` movement downstream; private `Subject` (`:30`) and `Fixture` (`:78`).
- `rig: Verdict` (`crates/crpg-net/tests/support/hostile.rs:40`): `Refused(CodecError)` or `Accepted(canonical bytes)`.
- `rig: reencoded` (`crates/crpg-net/tests/support/hostile.rs:50`): turns a decode result into a `Verdict`, panicking if the encoder refuses a decoded value.
- `rig: with_byte / padded / cat / edit` (`crates/crpg-net/tests/support/hostile.rs:69`): the four literal- and row-building helpers; `edit` (`:95`) applies ascending `(at, remove, insert)` edits.
- `rig: Want / Row / row / assert_row` (`crates/crpg-net/tests/support/hostile.rs:114`): expected verdicts `Refused`, `Same` (`Accepted(m)`) and `Orig` (`Accepted(f)`); `assert_row` (`:152`) prints both byte strings on failure.
- `rig: XorShift64 / SEED / stable` (`crates/crpg-net/tests/support/hostile.rs:168`): the Marsaglia xorshift64 PRNG, the fixed seed `0x5EED_0000_0000_0042` (`:180`), and `stable` (`:183`: refusal, or an accepted canonical form that re-encodes to itself).
- `rig: Counts / check_corruption` (`crates/crpg-net/tests/support/hostile.rs:191`): every byte against every other value must be a refusal other than `FrameTooLarge`, the same bytes, a ULID alias, or a shorter stable re-encoding; anything else panics.
- `rig: fixtures are encoder-built and pinned` (`crates/crpg-net/tests/hostile_delta.rs:248`): `fixtures()` builds each value through its wire's encoder, `pinned()` (`:302`) holds the literal; HA1 (`:371`) asserts the two are equal and decode canonically.
- `HA2 short_inputs_are_refused_exactly` (`crates/crpg-net/tests/hostile_delta.rs:411`): every input of at most two bytes gives `Malformed`, `UnsupportedVersion` or `WrongDirection` exactly.
- `HA3 oversize_is_refused_first` (`crates/crpg-net/tests/hostile_delta.rs:470`): 65,537 bytes is `FrameTooLarge` before the version; 65,536 is `UnsupportedVersion`; a padded fixture is `Malformed`.
- `HA4 every_truncation_is_malformed` (`crates/crpg-net/tests/hostile_delta.rs:522`): every strict prefix is `Malformed`.
- `HA5 trailing_bytes_are_malformed` (`crates/crpg-net/tests/hostile_delta.rs:549`): one `0x00`, `0x01` or `0xFF` after a fixture is `Malformed`.
- `HA6 foreign_first_bytes_are_unsupported` (`crates/crpg-net/tests/hostile_delta.rs:579`): a fixture given to another wire, and byte 0 replaced by each other value, is `UnsupportedVersion`.
- `HA7 foreign_lanes_are_wrong_direction` (`crates/crpg-net/tests/hostile_delta.rs:624`): byte 1 replaced by each non-zero value is `WrongDirection`.
- `HA8 op_tag_matrix` (`crates/crpg-net/tests/hostile_delta.rs:683`): all 256 op-tag values per wire land on the table's `Accepted(m)`, `Malformed` or `UnknownMessage`.
- `HA9 header_scalars_and_op_count` (`crates/crpg-net/tests/hostile_delta.rs:723`): 257 ops is `LimitExceeded`; `u64`/varint overflows are `Malformed`; `server_tick` and `event_seq` accept the full `u64` range.
- `HA10 health_and_receipt_fields` (`crates/crpg-net/tests/hostile_delta.rs:828`): Health's bool and `u32` bounds, Receipt's lane, `seq` range and version-scoped rejection codes.
- `HA11 upstream_intents_sent_down_are_refused` (`crates/crpg-net/tests/hostile_delta.rs:1056`): an upstream intent on its own delta wire is `WrongDirection`, on another wire `UnsupportedVersion`.
- `HA12 single_byte_corruption_is_refused_or_exact` (`crates/crpg-net/tests/hostile_delta.rs:1105`): 40,800 single-byte mutants, each refused or exact/stable.
- `HA13 seeded_noise_is_stable` (`crates/crpg-net/tests/hostile_delta.rs:1212`): four seeded passes over the same wires, with the first three PRNG values pinned.
- `HB1..HB11` (`crates/crpg-net/tests/hostile_delta_ops.rs:498`): fixtures pinned; truncation/trailing/foreign byte/foreign lane refused; ActionResolved, EntityFacts, Encounter/Timeline/Clock/HostFacts and Moved/Position rows; upstream frames refused; corruption and seeded noise stable.
- `HC1..HC13` (`crates/crpg-net/tests/hostile_transfer.rs:480`): fixtures pinned; short/oversize/truncated/trailing/foreign frames refused; tag matrix; Begin/Chunk/Cancel fields and built chunks; body fields; upstream frames refused; corruption and seeded noise stable.
- `HD1..HD13` (`crates/crpg-net/tests/hostile_session.rs:307`): fixtures pinned; short/oversize/truncated/trailing/foreign frames refused; tag and kind matrix; resume proof/answer fields; movement receipt/sample fields; upstream frames refused; corruption and seeded noise stable.
- `architecture index` (`docs/architecture/crpg-net.md:323`): "Hostile-server suite (T042b)" names the support module and the four files (`:325`), the ten-decoder surface (`:332`) and the excluded QUIC/Godot layers (`:336`), the precedence and stability rule (`:341`), and the spec-class table (`:354`).

## crpg-net-quic client

- `Welcome` (`crates/crpg-net-quic/src/handshake.rs:320`): server→client first frame after acceptance, `wire_version: u8` and `epoch: [u8; 16]`.
- `Welcome::encode` (`crates/crpg-net-quic/src/handshake.rs:329`): `[wire_version][epoch: 16]`, `WELCOME_BYTES` long; `encode_for` (`:334`) applies the ALPN version rule.
- `Welcome::decode` (`crates/crpg-net-quic/src/handshake.rs:343`): length ≠ 17 → `Malformed`, then an unknown version → `UnsupportedWireVersion`; the epoch is copied opaquely.
- `Welcome::decode_for` (`crates/crpg-net-quic/src/handshake.rs:348`): the same checks with the connection's ALPN rule.
- `wire_version_ok` (`crates/crpg-net-quic/src/handshake.rs:375`): `/3` carries exactly version 4; `/1` and `/2` carry 1 or 2.
- `HandshakeError` (`crates/crpg-net-quic/src/handshake.rs:364`): `Malformed`, `UnsupportedWireVersion`, `CredentialLength` — the only handshake refusals.
- `ConnectError::InvalidWelcome` (`crates/crpg-net-quic/src/client.rs:64`): the welcome, or a resume answer, is malformed or names another version; the client closes with `ProtocolViolation`.
- `welcome header length check` (`crates/crpg-net-quic/src/client.rs:977`): a 4-byte length other than 17 returns `InvalidWelcome`.
- `welcome body decode` (`crates/crpg-net-quic/src/client.rs:983`): any `HandshakeError` from `Welcome::decode_for` maps to `InvalidWelcome`.
- `welcome version must equal the hello's` (`crates/crpg-net-quic/src/client.rs:984`): a mismatch returns `InvalidWelcome`.
- `resume answer header` (`crates/crpg-net-quic/src/client.rs:1024`): byte 0 must be `Channel::Resume` (`0x91`), else `InvalidWelcome`; `parse_answer_len` (`:1027`) accepts only 1..=65,536.
- `read_welcome_part` (`crates/crpg-net-quic/src/client.rs:1052`): a lost connection is `lost_error`, any other read failure is `invalid_welcome` (`:1060`).
- `invalid_welcome` (`crates/crpg-net-quic/src/client.rs:1064`): `close_quinn(connection, CloseCode::ProtocolViolation)` then `ConnectError::InvalidWelcome`.
- `lost_error` (`crates/crpg-net-quic/src/client.rs:1070`): peer `ApplicationClosed` → `ConnectError::Refused { code }`, otherwise `ConnectError::Lost` (`:1075`).
- `handshake_error` (`crates/crpg-net-quic/src/client.rs:1080`): `PinMismatch`, `EndpointFull` on `CONNECTION_REFUSED`, `Refused`, `TimedOut`, `Reset`, `Lost(Local(Normal))` for local close, and `Handshake` for the rest.
- `negotiated-ALPN check` (`crates/crpg-net-quic/src/client.rs:945`): under `/2` or `/3` a negotiated protocol other than the one offered closes with `ProtocolViolation` and returns `Handshake`.
- `frame_error_code` (`crates/crpg-net-quic/src/queue.rs:946`): zero length or an unknown channel is `ProtocolViolation`; an over-cap length is `FrameTooLarge` with no body read.
- `read_lane_frame` (`crates/crpg-net-quic/src/queue.rs:956`): reads the 4-byte header and body; `/2` upstream rejects a `0x91` opening as `ProtocolViolation` (`:977`).
- `read_fail` (`crates/crpg-net-quic/src/queue.rs:988`): a FIN mid-frame or `RESET_STREAM` is `ProtocolViolation`; any other read error is `Lost`.
- `datagram_reader` (`crates/crpg-net-quic/src/queue.rs:1151`): an empty datagram closes `ProtocolViolation` (`:1157`), one above `MAX_DATAGRAM_BYTES` closes `FrameTooLarge` (`:1161`).
- `try_recv` (`crates/crpg-net-quic/src/client.rs:450`): `Ok(None)` when empty; `Err(CloseReason)` only once closed **and** drained; a transfer or resume frame at the head still returns `Ok(None)`.
- `try_recv_channel` / `try_recv_lane1` / `try_recv_datagram` (`crates/crpg-net-quic/src/client.rs:471`, `:509`, `:523`): the same drained-then-error rule per queue.
- `close_reason` (`crates/crpg-net-quic/src/client.rs:610`): `EndpointFailed` if the I/O thread failed, else the record's first close reason.
- `close` (`crates/crpg-net-quic/src/client.rs:621`): takes `CloseCode` and `CloseMode`, runs `stop`, joins the I/O thread, returns `CloseReport`.
- `stop` (`crates/crpg-net-quic/src/client.rs:629`): builds the flush request with `FlushClose::flush_close` (`:634`); `Flush` waits for the record's close (`:641`); `Discard` records `Local(code)` (`:661`); then sets `Stop { code, deadline }` (`:667`).
- `Drop` (`crates/crpg-net-quic/src/client.rs:693`): closes with `Normal` in `Discard` mode (`:698`).
- `CloseCode` (`crates/crpg-net-quic/src/close.rs:10`): `Normal` 0, `Shutdown` 1, `ProtocolViolation` 2, `FrameTooLarge` 3, `HelloTimeout` 4, `AuthRefused` 5, `VersionRefused` 6, `ServerBusy` 7, `AuthTimeout` 8, `SessionFenced` 9; `from_u64` (`:41`) returns `None` outside 0..=9.
- `CloseMode` (`crates/crpg-net-quic/src/close.rs:76`): `Flush` writes queued frames, FINs, waits for the ack, then `CONNECTION_CLOSE`; `Discard` drops unqueued frames and closes at once.
- `CloseReason` (`crates/crpg-net-quic/src/close.rs:88`): `Local(CloseCode)`, `Peer { code: u64 }`, `IdleTimeout`, `Transport`, `Reset`, `EndpointFailed`.
- `v2_client_reader_close_codes` (`crates/crpg-net-quic/tests/channel.rs:2545`): a raw quinn server writes valid lane-0/transfer/resume frames, then one bad 4-byte header per case; asserts the client returns `Local(ProtocolViolation)` (code 2) for a zero length, an unknown channel or a wrong direction, `Local(FrameTooLarge)` (code 3) for an over-cap length, and the peer sees the same code; frames before the bad one still drain.
- `v2_client_reader_close_codes, FIN mid-frame` (`crates/crpg-net-quic/tests/channel.rs:2587`): a short body followed by FIN makes `try_recv_channel` return `Local(ProtocolViolation)` and the peer see code 2.
- `connect_resume_answer_checks_and_timeout` (`crates/crpg-net-quic/tests/channel.rs:2654`): six resume answers — a `/1` welcome, a foreign channel byte, a zero length, a 0x010001 length, a truncated body with FIN — each gives `ConnectError::InvalidWelcome` and the peer sees close 2 (`:2689`).
- `frame_and_handshake_bytes_are_exact` (`crates/crpg-net-quic/tests/quic.rs:72`): direct `Welcome::decode` checks — 16 bytes, 17 zeros, a 17-byte trailing input are `Malformed`; version 3 is `UnsupportedWireVersion` (`:153`–`:164`). It does not feed a live client.
- `none found` (`crates/crpg-net-quic/tests/channel.rs:2515`): no test drives `QuicClient::connect` against a raw server that sends a malformed 17-byte welcome, a welcome whose version differs from the hello's, or a header length other than 17; `raw_v2_connect` always sends a valid welcome (`:2530`).

## Godot bridge

- `BridgeCode` (`crates/crpg-godot/src/core/errors.rs:12`): the closed ADR-0033 §11 table; `is_fatal` is exactly 50..=55 (`:88`).
- `52 / 53 / 54` (`crates/crpg-godot/src/core/errors.rs:46`): `FrameUndecodable` (52), `ReplicaRejected` (53), `UnexpectedTransfer` (54), each fatal.
- `Fault` (`crates/crpg-godot/src/core/errors.rs:95`): one code plus the diagnostic `detail` string.
- `host_fault` (`crates/crpg-godot/src/core/errors.rs:106`): `HostError::Closed` → 50, every other host failure → 51.
- `v2 decode fault` (`crates/crpg-godot/src/core/client.rs:443`): `decode_delta` failure → `FrameUndecodable` with the codec's `Display`.
- `v2 replica fault` (`crates/crpg-godot/src/core/client.rs:447`): `ReplicaWorld::apply_delta` failure → `ReplicaRejected`; the view stays at its previous revision (`:433`).
- `v2 submit link full` (`crates/crpg-godot/src/core/client.rs:654`): a full link is `IntentRefused`, not fatal.
- `transfer in a hand-off` (`crates/crpg-godot/src/core/authority.rs:64`): a `DeliveryKind::Transfer` entry → `UnexpectedTransfer` (54).
- `v4 lane split` (`crates/crpg-godot/src/core/client_v4.rs:100`): `0x82` bytes go to `offer_transfer`; `0xA1` bytes to `decode_down`; everything else to `offer_delta`.
- `v4 transfer reply faults` (`crates/crpg-godot/src/core/client_v4.rs:112`): a reply that does not encode or does not fit the link → `ReplicaRejected` (53).
- `v4 movement decode fault` (`crates/crpg-godot/src/core/client_v4.rs:124`): `decode_down` failure → `FrameUndecodable` (52).
- `v4 outbox refusal` (`crates/crpg-godot/src/core/client_v4.rs:130`): `MoveOutbox::receive` failure → `ReplicaRejected` (53).
- `v4 sample` (`crates/crpg-godot/src/core/client_v4.rs:137`): `apply_sample` returns nothing and never faults.
- `v4 pool offer` (`crates/crpg-godot/src/core/client_v4.rs:141`): `offer_delta` maps through `transfer_fault`; the drained update goes through `apply_update` (`:145`).
- `v4 snapshot refusal` (`crates/crpg-godot/src/core/client_v4.rs:305`): `apply_snapshot` failure → 53.
- `v4 delta refusal` (`crates/crpg-godot/src/core/client_v4.rs:313`): `apply_delta` failure → 53; the ops are then applied to the view (`:318`).
- `transfer_fault` (`crates/crpg-godot/src/core/client_v4.rs:549`): `TransferError::Codec` → 52, every other `TransferError` → 53.
- `poll guard` (`crates/crpg-godot/src/core/mod.rs:248`): `catch_unwind` around `poll_guarded`; a panic becomes 55 (`:934`) and fences.
- `v2 poll fault` (`crates/crpg-godot/src/core/mod.rs:732`): a client or authority `Fault` goes to `fail_fault`, which fences on a fatal code (`:896`).
- `v4 client fault` (`crates/crpg-godot/src/core/mod.rs:774`): a `ClientV4::step` `Fault` fences through `fail_fault` (`:778`).
- `v4 loop end` (`crates/crpg-godot/src/core/mod.rs:817`): a panicked runner → 55, a hosting error → `host_fault` (50/51), a time regression → 51, a read error/link close/stop → 51.
- `fail / last_error` (`crates/crpg-godot/src/core/mod.rs:887`): a non-fatal result only records `"<NAME>: <detail>"`; `fail_fault` (`:896`) fences a fatal code and keeps that text.
- `every_code_matches_the_adr_table` (`crates/crpg-godot/src/core/errors/tests.rs:11`): asserts all 20 codes' numbers, names and fatality, including 52/53/54.
- `host_failures_split_on_closed` (`crates/crpg-godot/src/core/errors/tests.rs:55`): only `HostError::Closed` is 50; every other host failure is 51.
- `bad_frames_are_fatal_and_never_skipped` (`crates/crpg-godot/src/core/client/tests.rs:496`): `[0xff]` → 52; an `event_seq` gap → 53 with the view `Rc` unchanged; a control frame at the next seq is accepted.
- `a_leave_shrinks_every_column_and_a_retired_id_cannot_return` (`crates/crpg-godot/src/core/client/tests.rs:662`): a retired id re-entering is 53 (`:718`), the view keeps revision 4.
- `bad_frames_are_fatal` (`crates/crpg-godot/src/core/client_v4/tests.rs:607`): a short `0xA1` frame → 52; a v2 delta on a v4 session → 52; a `Position` op before its row → 53; each leaves revision 0.
- `lane1_receipts_complete_the_outbox_in_order` (`crates/crpg-godot/src/core/client_v4/tests.rs:399`): a receipt for seq 2 before seq 1 is 53, detail `"Unexpected at movement/outbox"` (`:477`), and the view is unchanged.
- `non_codec_transfer_error_is_replica_rejected` (`crates/crpg-godot/src/core/client_v4/tests.rs:1349`): two buffered deltas with one seq and different bytes → 53; the same bytes twice is a harmless duplicate.
- `hand_off_stops_at_the_first_full_link` (`crates/crpg-godot/src/core/authority/tests.rs:127`): a `Transfer` entry → 54; a full link stops at the last queued ordinal.
- `corrupt_frame_fences_and_close_resets` (`crates/crpg-godot/src/core/tests.rs:825`): `[0xff]` downstream → `poll` 52, fenced, revision kept, `FRAME_UNDECODABLE` prefix, every later `poll`/`submit_option` repeats 52; `close` resets to 0 and the text survives; `open` clears it.
- `submit_precedence_pairs` (`crates/crpg-godot/src/core/tests.rs:673`): on a fenced session a valid revision returns 52 while a negative argument still returns 3 (`:693`).
- `panics_are_caught_at_every_boundary` (`crates/crpg-godot/src/core/tests.rs:928`): a teardown panic during a 52 fence keeps 52 and `FRAME_UNDECODABLE` (`:966`).
- `a_refused_frame_clears_the_pending_submit` (`crates/crpg-godot/src/core/tests.rs:1213`): 80 malformed upstream frames give `IntentRefused` (30), not a fence; the pending seq is cleared so the next submit sends.

## Test harness options

## Rules that bind it

## Agent log

- 2026-11-20 (UTC) · opencode-go/deepseek-v4.1-flash + T042c facts · Gathering the requirements, existing suites, refusal paths and constraints the T042c contract will be written from.
