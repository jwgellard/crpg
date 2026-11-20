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

## Godot bridge

## Test harness options

## Rules that bind it

## Agent log

- 2026-11-20 (UTC) · opencode-go/deepseek-v4.1-flash + T042c facts · Gathering the requirements, existing suites, refusal paths and constraints the T042c contract will be written from.
