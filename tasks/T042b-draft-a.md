# T042b — the wire-level hostile-server suite (`crpg-net`)

**Status: blind draft (attempt a), not approved. A Claude architect writes the final contract; the user approves it. Nothing here is approved.**

Written from `tasks/T042b-facts.md` (the fact sheet), `tasks/T042.md` (T042a's form and pattern), the root `AGENTS.md` and `crates/crpg-net/AGENTS.md`, and the code in `crates/crpg-net/src/`. Every fact was re-checked in code.

Citation convention (T140's): a bare `:N` or `:N–M` is a line of the file cited earlier in the same item or sentence; another branch is `<branch>:<path>:<line>`.

## Task

Give `crpg-net` the hostile-server mirror of T042a's malicious-client suite:
the client's own decoders, as one table-driven file. A hostile server (or a
server bug) controls only the bytes it sends to a client; in `crpg-net` those
reach the ten client-side decoders: the four lane-0 delta decoders
(`codec::decode_delta` `:421`, `codec_v2::decode_delta` `:595`,
`codec_v3::decode_delta` `:1103`, `codec_v4::decode_delta` `:428`), the two
transfer downstream decoders (`codec_transfer::decode_down` `:246`,
`transfer_v3::decode_down` `:426`), the resume downstream decoder
(`codec_resume::decode_down` `:205`), the movement downstream decoder
(`movement_v4::decode_down` `:318`), and the two snapshot bodies
(`codec_transfer::decode_body` `:97`, `transfer_v3::decode_body` `:258`).
Each is tested today in its own suite with its own fixtures; no suite treats
them as one surface, none sends one wire's frame to another's decoder, none
corrupts every byte with every value, and none checks that a decoded value
re-encodes stably — which the client's replica applies whole
(`docs/adr/0035-client-replica.md:12`) and the bridge re-applies
(`docs/adr/0033-godot-bridge.md:152`). T042b adds that suite as one file,
about 500 test lines (`tasks/BACKLOG.md:355`).

Every case feeds bytes a malicious server could send into a client-side
decoder and pins the exact `CodecError` it must produce, with an accepted
control beside it. Test-only: a case that shows a decoder bug is an open
question, never a source change (D67b `tasks/DECISIONS-D67.md:16`).

## Parts

One part, one crate (`AGENTS.md` "One task = one crate").

| Part | Crate | Scope | Size (tests) | S-set | Review |
|---|---|---|---|---|---|
| T042b | `crpg-net` | `tests/malicious_server.rs` (S1–S14; the whole suite) | ~500 test lines | no | a free first-pass review, then one OpenCode Go reviewer mutant run ("Definition of done"), one fix round |

T042b is not S-set (`docs/IMPLEMENTATION_PLAN.md:86–98`): it changes no wire
byte, no public item, no `World`, no tick order, no workspace file.

### T042b owns
- `crates/crpg-net/tests/malicious_server.rs` (new; the whole suite)
- goldens: none new, none moved.

## Readings

Design decisions this contract makes. Each needs approval with the contract.

1. **The surface is the ten client-side decoders.** A hostile server controls
   only the bytes it sends; in `crpg-net` those reach the four delta decoders,
   the `0x81`/`0x82` downstream, the `0x91` downstream, the `0xA1` downstream,
   and the two snapshot bodies (all cited in "Task"). *Rejected:* the QUIC
   `Welcome` decoder (`crates/crpg-net-quic/src/handshake.rs:343`) and the
   client session's reaction (`crates/crpg-godot/src/core/client.rs:434`) —
   both are other crates, and "one task = one crate" (`AGENTS.md`). The pools,
   replicas and `apply_delta` are *consumers of decoded values*, not decoders
   (`docs/adr/0035-client-replica.md:80`), so they are out of scope too.
2. **Three decoder shapes, one table.** The eight framed decoders (deltas and
   `decode_down`s) all read `version → lane` first, so they are one `Wire`
   enum; the two bodies read `count → entities` with no version or lane, so
   they are a second `Body` enum sharing `Verdict` and the helpers. *Rejected:*
   one `Wire` enum of ten (the bodies have no `first_byte`/`lane`, so the
   uniform methods of T042a's `Wire` would be sentinels), and a per-decoder
   gap-fill like the draft suits that treat them as separate suites.
3. **Stateless decoders have no replay, timing, flooding, ownership or
   interest case.** A decoder is a function of one byte slice (T042a Reading
   3, `tasks/T042.md:101`). The client-session equivalents (rate, fence,
   interest) live in `crpg-net-quic`/`crpg-godot` and stay out of scope
   (Reading 1). *Rejected:* per-decoder replay or rate cases (pass by
   construction).
4. **The code's decode order is the contract.** Each decoder checks each field
   as it reads it: frame cap → version → lane → header scalars → count/bounds
   before any body → each tag/op then its fields → trailing bytes. The delta
   decoders read `server_tick` then `event_seq` with no range check on either
   (`codec.rs:434–435`, `codec_v2.rs:608–609`, `codec_v3.rs:1116–1117`,
   `codec_v4.rs:441–442`); the bodies return `LimitExceeded` (not
   `FrameTooLarge`) for oversize (`codec_transfer.rs:98–99`,
   `transfer_v3.rs:259–260`). *Rejected:* changing the code to match a
   different order (it would move frozen receipt bytes, T042a Reading 4).
5. **Corruption is exhaustive and acceptance must be stable.** For each pinned
   fixture, every byte is replaced by every other value; the result is a
   refusal or a frame whose canonical re-encoding is the corrupted bytes
   themselves, except inside ULID text where core accepts case and `I`/`L`/`O`
   aliases (`crates/crpg-core/src/ulid.rs:111–122`). `postcard` accepts
   non-minimal varints (postcard 1.1.3), but the only pinned frames where a
   varint is followed by `0x00` are the delta op count (before its op tag) and
   the Begin `transfer` (before `base_tick`); in both, a corrupted continuation
   byte shifts a later field into a value the decoder refuses, so no
   single-byte corruption reaches that leniency. *Rejected:* `proptest` (a new
   dev-dependency, `AGENTS.md` "Do not add dependencies") and sampling instead
   of enumerating.
6. **Stability is checked through the encoder, not `PartialEq`.** `ResumeDown`
   holds a `ResumeProof`, which never gains `PartialEq`
   (`crates/crpg-net/src/protocol_resume.rs:94–97`); comparing re-encoded
   bytes works for all ten decoders and is exactly what the client's replica
   round-trips. *Rejected:* `PartialEq` on decoded values.
7. **The "log line" at this level is the refusal's fixed `Display` text.**
   `crpg-net` does no I/O; every refusal is a `CodecError` with one of six
   fixed texts (`codec.rs:72–83`). Writing the line to a file/console, and the
   client's close code (`BridgeCode::FrameUndecodable` 52, or QUIC
   `ProtocolViolation`), are other crates' jobs (Reading 1). *Rejected:* a
   logging dependency, and pinning the client close in a `crpg-net` test.
8. **T042b fixes nothing in a decoder.** If a case fails against unchanged
   `src/`, the implementer stops and reports the input and actual verdict
   (S-1). *Rejected:* a source fix folded into a test task.

## Interface

**Production code: none.** No public item, signature, variant, constant or
`Display` text of `crpg-net` changes. Everything below is test-local, in
`crates/crpg-net/tests/malicious_server.rs`, with these exact shapes (names of
private helpers may not change: the review and later tasks cite them).

```rust
//! T042b: the wire-level hostile-server suite (the mirror of T042a).
//! Every client-side decoder, every hostile class, one table. No clock,
//! no thread, no I/O, no unseeded RNG, no `HashMap`/`HashSet`, no float.

/// One client-facing framed decoder with its own encoder (Reading 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wire { V1, V2, V3, V4, T81, T82, R91, A1 }

/// Table order; every "for each wire" loop uses it.
const WIRES: [Wire; 8] = [Wire::V1, Wire::V2, Wire::V3, Wire::V4,
                          Wire::T81, Wire::T82, Wire::R91, Wire::A1];

impl Wire {
    /// 1, 2, 3, 4, 0x81, 0x82, 0x91, 0xA1.
    fn first_byte(self) -> u8;
    /// Index of the lane byte: 1 for all eight (Reading 2).
    fn lane_at(self) -> usize;
    /// The lane byte: 0, 0, 0, 0, 2, 2, 3, 1.
    fn lane(self) -> u8;
    /// Decode with this wire's decoder; on `Ok(v)`, re-encode `v` with this
    /// wire's encoder. An encoder error on a decoded value panics with the
    /// wire, the input bytes and the error (the test fails: W-12, W-13).
    fn verdict(self, bytes: &[u8]) -> Verdict;
}

/// One client-facing body decoder: no version byte, no lane (Reading 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Body { B81, B82 }
const BODIES: [Body; 2] = [Body::B81, Body::B82];
impl Body {
    /// Decode with this body's decoder; on `Ok(v)`, re-encode with its
    /// encoder. Same panic rule as `Wire::verdict`.
    fn verdict(self, bytes: &[u8]) -> Verdict;
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Verdict {
    /// The decoder refused the bytes with this error.
    Refused(CodecError),
    /// The decoder accepted them; the canonical re-encoding of the value.
    Accepted(Vec<u8>),
}

/// One pinned valid frame (the table in W-1). `wire` and `body` are mutually
/// exclusive; `tag_at` is the body/kind/op-tag byte index for W-8/W-12.
struct Fixture { name: &'static str, wire: Option<Wire>, body: Option<Body>,
                 bytes: Vec<u8>, tag_at: Option<usize> }

/// The 13 fixtures of W-1, in its table order, each built by its decoder's
/// encoder from the value in that table (never from hand-written bytes).
fn fixtures() -> Vec<Fixture>;

/// `bytes` with `bytes[at]` replaced by `value`; every other byte unchanged.
fn with_byte(bytes: &[u8], at: usize, value: u8) -> Vec<u8>;

/// `bytes` followed by `0x00` up to `len` bytes (`len >= bytes.len()`).
fn padded(bytes: &[u8], len: usize) -> Vec<u8>;

/// W-13's rule: `Refused(_)`, or `Accepted(c)` with `verdict(&c) == Accepted(c)`.
fn stable(wire: Wire, bytes: &[u8]) -> bool;
fn stable_body(body: Body, bytes: &[u8]) -> bool;

/// Marsaglia xorshift64 (shifts 13, 7, 17), wrapping `u64` arithmetic.
struct XorShift64(u64);
impl XorShift64 { fn next(&mut self) -> u64; }
const SEED: u64 = 0x5EED_0000_0000_0042;
```

`Wire::verdict` calls exactly:

| Wire | Decoder | Encoder |
|---|---|---|
| `V1` | `codec::decode_delta` | `codec::encode_delta` |
| `V2` | `codec_v2::decode_delta` | `codec_v2::encode_delta` |
| `V3` | `codec_v3::decode_delta` | `codec_v3::encode_delta` |
| `V4` | `codec_v4::decode_delta` | `codec_v4::encode_delta` |
| `T81` | `codec_transfer::decode_down` | `codec_transfer::encode_down` |
| `T82` | `transfer_v3::decode_down` | `transfer_v3::encode_down` |
| `R91` | `codec_resume::decode_down` | `codec_resume::encode_down` |
| `A1` | `movement_v4::decode_down` | `movement_v4::encode_down` |

`Body::verdict` calls exactly:

| Body | Decoder | Encoder |
|---|---|---|
| `B81` | `codec_transfer::decode_body` | `codec_transfer::encode_body` |
| `B82` | `transfer_v3::decode_body` | `transfer_v3::encode_body` |

The wire constants come from `protocol::PROTOCOL_VERSION` (`protocol.rs:25`),
`protocol_v2::PROTOCOL_VERSION` (`protocol_v2.rs:34`),
`protocol_v3::PROTOCOL_VERSION` (`protocol_v3.rs:40`),
`protocol_v4::PROTOCOL_VERSION` (`protocol_v4.rs:27`),
`protocol_transfer::TRANSFER_VERSION` (`protocol_transfer.rs:19`),
`transfer_v3::TRANSFER_VERSION_V3` (`transfer_v3.rs:39`),
`protocol_resume::RESUME_VERSION` (`protocol_resume.rs:18`),
`movement_v4::MOVEMENT_VERSION` (`movement_v4.rs:32`), and the lanes
`LANE_COMBAT` (`protocol.rs:34`), `LANE_SNAPSHOT` (`protocol_transfer.rs:22`),
`LANE_SESSION` (`protocol_resume.rs:21`), `LANE_MOVEMENT` (`movement_v4.rs:34`).

## Behaviour

Normative. Each item names its tests. Every expected value below follows from
the cited code; the implementer does not adjust one to make a test pass (S-1).
Notation: `EPOCH` = `[0x01, 0x02, …, 0x10]` (16 bytes); `net(n)` =
`NetId::new(n).unwrap()`; `tid(n)` = `TransferId::new(n).unwrap()`;
`ABILITY` = `Ulid::from_parts(0x0123_4567_89AB, 0x0CDE_F012_3456_789A)`
(`crates/crpg-core/src/ulid.rs:27`), text `014D2PF2DB0000SQQG28T5CY4T`.

W-1. **The fixtures.** The delta header `D(v)` = `[v, 0x00, 0x05, 0x01]`
(version, lane 0, `server_tick` 5, `event_seq` 1); then `op_count`, then the
op. Every fixture, built by its decoder's encoder, equals its bytes, and
`verdict(bytes) == Accepted(bytes)` (each is canonical).

| Fixture | Wire/Body | Value | Bytes | Len | `tag_at` |
|---|---|---|---|---|---|
| `D1`–`D4` | `V1`–`V4` | the version's `DeltaFrame { lane: 0, server_tick: 5, event_seq: 1, ops: [EntityEnter { entity: net(9) }] }` | `[v, 0x00, 0x05, 0x01, 0x01, 0x00, 0x09]` | 7 | 5 |
| `B81` | `T81` | `TransferDown::Begin(BeginTransfer { epoch: EPOCH, transfer: tid(1), base_tick: 0, cursor: 0, total_bytes: 4096, entity_count: 0, segments_per_chunk: 1, chunk_count: 1 })` | `[0x81, 0x02, 0x10] ++ EPOCH ++ [0x01, 0x00, 0x00, 0x80, 0x20, 0x00, 0x01, 0x01]` | 27 | 2 |
| `C81` | `T81` | `TransferDown::Cancel { epoch: EPOCH, transfer: tid(1), reason: Aborted }` | `[0x81, 0x02, 0x12] ++ EPOCH ++ [0x01, 0x02]` | 21 | 2 |
| `B82` | `T82` | the same Begin through `transfer_v3` | `[0x82, 0x02, 0x10] ++ EPOCH ++ [0x01, 0x00, 0x00, 0x80, 0x20, 0x00, 0x01, 0x01]` | 27 | 2 |
| `C82` | `T82` | the same Cancel through `transfer_v3` | `[0x82, 0x02, 0x12] ++ EPOCH ++ [0x01, 0x02]` | 21 | 2 |
| `Q91` | `R91` | `ResumeDown::Ticket(ResumeTicket { epoch: EPOCH, proof: ResumeProof::new(vec![0xA5; 16]).unwrap() })` | `[0x91, 0x03, 0x30] ++ EPOCH ++ [0x10] ++ [0xA5; 16]` | 36 | 2 |
| `R1` | `A1` | `MoveDown::Receipt(MoveReceipt { epoch: EPOCH, seq: 1, actor: net(7), processed_tick: 41, outcome: Accepted { start_tick: 42 } })` | `[0xA1, 0x01, 0x00] ++ EPOCH ++ [0x01, 0x07, 0x29, 0x00, 0x2A]` | 24 | 2 |
| `S1` | `A1` | `MoveDown::Sample(MoveSample { server_tick: 5, processed_seq: 0, entries: [SampleEntry { entity: net(7), point: Point::new([1, 2, 3]).unwrap(), moving: false }] })` | `[0xA1, 0x01, 0x01, 0x05, 0x00, 0x01, 0x07, 0x02, 0x04, 0x06, 0x00]` | 11 | 2 |
| `Y81` | `B81` | `SnapshotBody { entities: [SnapshotEntity { entity: net(1), health: None }], turn: None }` | `[0x01, 0x01, 0x00, 0x00]` | 4 | — |
| `Y82` | `B82` | `SnapshotBodyV3 { entities: [SnapshotEntityV3 { entity: net(1), health: None, facts: FactSet::default() }], turn: None, encounter: EncounterFacts::default(), timeline: None }` | `[0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]` | 13 | — |

Table order is `D1, D2, D3, D4, B81, C81, B82, C82, Q91, R1, S1, Y81, Y82`.
Layouts are the encoders' (`codec.rs:395–414`, `codec_transfer.rs:160–244`,
`transfer_v3.rs:349–422`, `codec_resume.rs:185–202`, `movement_v4.rs:273–312`,
`codec_transfer.rs:32–95`, `transfer_v3.rs:187–252`). *Tests:* S1.

W-2. **Short inputs, exhaustively.** For every framed wire and every input of
at most two bytes: `[]` → `Malformed`; `[b]` → `Malformed` if `b` is the
wire's first byte, else `UnsupportedVersion`; `[b, c]` → `UnsupportedVersion`
if `b` is not the first byte, else `WrongDirection` if `c` is not the lane,
else `Malformed`. (The lane is at index 1 for all eight, so no wire tests
emptiness explicitly; the empty input fails the version read.) For the bodies,
`[]` and every one-byte `[b]` (0..=255) → `Malformed` (the `turn`/`encounter`
read past the end). *Tests:* S2.

W-3. **Oversize is refused first.** The cap of `V1`–`V4` and `T81`/`T82` is
65,536 (`MAX_DELTA_FRAME_BYTES` `protocol.rs:64`; `MAX_DOWN_FRAME_BYTES`
`protocol_transfer.rs:46` equals it), of `R91` 4,096 (`MAX_RESUME_FRAME_BYTES`
`protocol_resume.rs:40`), of `A1` 1,024 (`MAX_MOVE_DOWN_BYTES`
`movement_v4.rs:50`), of `B81` 1,048,576 (`MAX_SNAPSHOT_BYTES`
`protocol.rs:89`) and of `B82` 4,194,304 (`MAX_SNAPSHOT_BYTES_V3`
`transfer_v3.rs:43`), checked before anything else. For each framed wire:
`[0xFF; cap + 1]` → `FrameTooLarge` (cap beats version); `[0xFF; cap]` →
`UnsupportedVersion`. For each body: `[0xFF; cap + 1]` → `LimitExceeded` (the
bodies map oversize to `LimitExceeded`, not `FrameTooLarge`,
`codec_transfer.rs:98–99`, `transfer_v3.rs:259–260`); `[0xFF; cap]` →
`Malformed` (a huge `count` varint cannot terminate). *Tests:* S3.

W-4. **Every truncation is `Malformed`.** For every fixture and every `n <
len`, `verdict(&f[..n]) == Refused(Malformed)`: a prefix keeps the version,
lane and tag/kind, so the first read past the end fails. *Tests:* S4.

W-5. **Trailing bytes are `Malformed`.** For every fixture, `f ++ [t]` for `t`
in `0x00, 0x01, 0xFF` → `Malformed`. *Tests:* S5.

W-6. **A foreign first byte is `UnsupportedVersion`.** Framed wires only. (a)
Every fixture, given to each of the seven other framed wires →
`UnsupportedVersion` (the eight first bytes are distinct). (b) Every fixture
with byte 0 replaced by each of the 255 other values → `UnsupportedVersion`
from its own wire. There is no negotiation or fallback. *Tests:* S6.

W-7. **A foreign lane is `WrongDirection`.** Framed wires only. Every fixture
with the byte at index 1 replaced by each of the 255 values other than the
lane → `WrongDirection`; the lane is read right after the version
(`codec.rs:430–433`, `codec_v2.rs:604–607`, `codec_v3.rs:1112–1115`,
`codec_v4.rs:437–440`, `codec_transfer.rs:259–262`, `transfer_v3.rs:438–441`,
`codec_resume.rs:217–220`, `movement_v4.rs:327–330`). *Tests:* S7.

W-8. **The tag/kind/op byte, all 256 values.** For a framed fixture `f` with
`m` = `with_byte(&f, tag_at, t)`:

| Fixture | `t` → verdict |
|---|---|
| `D1` (`V1`) | 0..=4 → `Accepted` (entity = 9); 5, 6, 7 → `Malformed` (more fields than remain); 8..=255 → `UnknownMessage` |
| `D2` (`V2`) | 0..=4 → `Accepted`; 5..=10 → `Malformed`; 11..=255 → `UnknownMessage` |
| `D3` (`V3`) | 0..=4, 14, 16, 17, 18 → `Accepted`; 5..=13, 19, 20 → `Malformed`; 15, 21..=255 → `UnknownMessage` |
| `D4` (`V4`) | 0..=4, 14, 16, 17, 18 → `Accepted`; 5..=13, 19..=22 → `Malformed`; 15, 23..=255 → `UnknownMessage` |
| `B81`, `B82` | `0x10` → `Accepted(f)`; `0x11`, `0x12` → `Malformed` (fields of the other tag); `0x20`, `0x21` → `WrongDirection`; else → `UnknownMessage` |
| `C81`, `C82` | `0x12` → `Accepted(f)`; `0x10`, `0x11` → `Malformed`; `0x20`, `0x21` → `WrongDirection`; else → `UnknownMessage` |
| `Q91` | `0x30` → `Accepted(f)`; `0x31`, `0x32` → `Malformed`; `0x40` → `WrongDirection`; else → `UnknownMessage` |
| `R1` | `0` → `Accepted(f)`; `1` → `Malformed`; else → `UnknownMessage` |
| `S1` | `1` → `Accepted(f)`; `0` → `Malformed`; else → `UnknownMessage` |

The delta `Accepted` set is exactly the ops whose fields are the one remaining
byte `0x09`: the five entity ops (0..=4) plus v3/v4's single-`NetId` notices
(16 `OrderEnded`, 17 `Joined`, 18 `Left`) and the scalar-only 14 `RoundStarted`
(a single `u64` = 9); the `UnknownMessage` at tag 15 is `ModeChanged` reading a
mode byte 9 that is neither 0 nor 1 (`codec_v3.rs:189–192`).

The delta op-tag sets are frozen per version (tags 0..=7 v1, 0..=10 v2,
0..=20 v3, 0..=22 v4; `codec.rs:334–389`, `codec_v2.rs:519–556`,
`codec_v3.rs:929–1035`, `codec_v4.rs:347–363`). A body `tag_at` is `None`; the
bodies have no discriminant byte. *Tests:* S8.

W-9. **Zero ids and field order.** Each → `Malformed`: `entity` byte := 0
(index 6 of `D1`–`D4`, index 1 of `Y81`/`Y82`, index 6 of `S1`); `transfer`
byte := 0 (index 19 of `B81`, `C81`, `B82`, `C82`); `actor` byte := 0 (index
20 of `R1`); `seq` byte := 0 (index 19 of `R1`); proof length := 0 (index 19
of `Q91`). Precedence (Reading 4): the tag/kind and header scalars are read
and checked before these fields, so a zero id is `Malformed`, never an
`UnknownMessage` tag first. *Tests:* S9.

W-10. **Hostile lengths are bounded before their bodies.** (a) Delta op count:
`D(v) ++ [0x81, 0x02]` (count 257) → `LimitExceeded` for all four (checked
before any op is read, `codec.rs:436–439` and the v2/v3/v4 twins); `D(v) ++
[0x80, 0x02]` (count 256, no ops) → `Malformed`. (b) Transfer Begin
(`B81`/`B82`): `total_bytes` := 0 → `Malformed`, := 1,048,577 (`0x81`) or
4,194,305 (`0x82`) → `LimitExceeded`; `entity_count` := 1,025 →
`LimitExceeded`; `segments_per_chunk` := 0 → `Malformed`, := 9 →
`LimitExceeded`; `chunk_count` := 0 → `Malformed`, := 33 (`0x81`) or 129
(`0x82`) → `LimitExceeded`; a `chunk_count` that is not
`div_ceil(total_bytes, segments_per_chunk * 4096)` → `Malformed`
(`codec_transfer.rs:265–321`, `transfer_v3.rs:444–499`).
(c) Resume proof (byte 19 of `Q91`): := 15 → `Malformed`, := 128 → `Malformed`
(16 bytes follow), := 129 → `LimitExceeded` (`codec_resume.rs:30–45`). (d)
Movement sample (byte 5 of `S1`): count := 39 → `LimitExceeded`
(`MAX_SAMPLE_ENTRIES` `movement_v4.rs:57`); `processed_seq` := `u64::MAX` →
`Malformed` (over `SEQ_LAST`). (e) Body count (byte 0): := 1,025 →
`LimitExceeded`; on `Y82`, a fact family count over its cap (pools 16, effects
16, stats 32, tags 8, timeline 1,024) → `LimitExceeded`, an order `target`,
`turn.active` or timeline `entity` not one of the entities → `Malformed`
(`transfer_v3.rs:297–330`). *Tests:* S10.

W-11. **Upstream frames sent downstream are refused.** Each built by its
encoder, with its control:

| Frame | Built as | Bytes | To | Verdict | Control |
|---|---|---|---|---|---|
| `P81` | `codec_transfer::encode_up(&TransferUp::Published { epoch: EPOCH, transfer: tid(1) })` | `[0x81, 0x02, 0x20] ++ EPOCH ++ [0x01]` | `T81` | `WrongDirection` | `codec_transfer::decode_up` gives it |
| `R81` | `codec_transfer::encode_up(&TransferUp::Resync { epoch: EPOCH, after: None, reason: Expired })` | `[0x81, 0x02, 0x21] ++ EPOCH ++ [0x00, 0x01]` | `T81` | `WrongDirection` | `decode_up` gives it |
| `P82`, `R82` | the same through `transfer_v3::encode_up` | version byte `0x82` | `T82` | `WrongDirection` | `transfer_v3::decode_up` gives it |
| `Req91` | `codec_resume::encode_request(&ResumeRequest { epoch: EPOCH, fence: ConnectionFence::FIRST, proof: ResumeProof::new(vec![0xA5; 16]).unwrap(), claims: vec![LaneClaim { lane: 0, receipted_through: 0 }] })` | `[0x91, 0x03, 0x40] ++ EPOCH ++ [0x01, 0x10] ++ [0xA5; 16] ++ [0x01, 0x00, 0x00]` | `R91` | `WrongDirection` | `codec_resume::decode_request` gives it |
| `I1` | `movement_v4::encode_intent(&MoveIntent { epoch: EPOCH, seq: 1, actor: net(7), body: Halt })` | `[0xA1] ++ EPOCH ++ [0x01, 0x01, 0x07, 0x01]` | `A1` | `UnknownMessage` | `movement_v4::decode_intent` gives it |

The `A1` case is `UnknownMessage`, not `WrongDirection`: the intent's byte 1
is `EPOCH[0]` = `0x01`, which happens to equal `LANE_MOVEMENT`, so the lane
check passes and the kind byte `EPOCH[1]` = `0x02` fails as unknown
(`movement_v4.rs:331–336`). Every one of these 7 frames, given to each framed
wire whose first byte differs from its own, → `UnsupportedVersion`. *Tests:* S11.

W-12. **Single-byte corruption is refused or exact.** For every fixture `f`,
every index `i < len` and every value `v != f[i]`, with `m = with_byte(&f, i,
v)`, `verdict(&m)` is one of: `Refused(_)` other than `FrameTooLarge`;
`Accepted(m)`; or, only inside ULID text, `Accepted(c)` where `c` equals `m`
except at `i` (an alias re-encoded canonically, Reading 5). Never a panic,
never an encoder error. *Tests:* S12.

W-13. **Seeded noise is stable.** One `XorShift64(SEED)` drives, in order:
(a) for each framed wire in `WIRES` order, 1,024 inputs: `len = 1 + (next() %
64)`, then `len` bytes each `next() as u8`, then byte 0 := the wire's first
byte and byte 1 := its lane; each must be `stable`; (b) 1,024 inputs of `len =
next() % 65` bytes, each `next() as u8`, each `stable` for all eight framed
wires and `stable_body` for both bodies. The first three `next()` values from
`SEED` are `0x4B10_DA10_A18C_50C2`, `0x8788_B83B_1805_38E3`,
`0x18B5_5443_1CC3_6A52` (the same `XorShift64` as T042a). Then two
fixture-seeded passes over the epoch-carrying fixtures (`B81`, `C81`, `B82`,
`C82`, `Q91`, `R1`; deltas and bodies carry no epoch): (c) 256 inputs each:
`i = 3 + (next() % 16)`, `v = next() as u8`, `m = with_byte(f, i, v)`; the 16
epoch bytes are opaque to every decoder, so each must be `Accepted(m)` exactly;
(d) for each of the 13 fixtures, 256 inputs: `i = tag_at + (next() % (len −
tag_at))` (for bodies, `i = 0 + (next() % len)`), `v = next() as u8`, each
`stable` for its own wire/body. The test counts the `Accepted` verdicts of (c)
and (d) and asserts at least 1,536 (so the accepted branch of `stable` carries
a fixed share). *Tests:* S13.

W-14. **Refusal texts.** The six `CodecError` texts are exactly those of
"Errors"; every refusal any test sees is one of the six variants. *Tests:* S14.

S-1. **Stop rule.** If any case fails against `<base>`'s `src/`, the
implementer does not change a decoder, an expected value or a fixture: it
stops and reports the case, the input bytes and the actual verdict (Reading 8,
`AGENTS.md` "Do not weaken").

## Errors

No new error type or variant. Every refusal is an existing `CodecError`
(`crates/crpg-net/src/codec.rs:56–70`); its `Display` is the "log line"
material (Reading 7):

| Variant | `Display` (exact) |
|---|---|
| `FrameTooLarge` | `frame exceeds its byte cap` |
| `UnsupportedVersion` | `unsupported protocol version` |
| `WrongDirection` | `wrong lane or direction` |
| `UnknownMessage` | `unknown message discriminant` |
| `Malformed` | `malformed frame` |
| `LimitExceeded` | `wire bound exceeded` |

T042a's "receipt code the host sends" column does not apply: on the client
side there is no host receipt; the bridge maps a decode failure to
`BridgeCode::FrameUndecodable` (52) and the QUIC client closes with
`ProtocolViolation`, both in other crates (Reading 1). A failing case is never
answered by a new variant, a changed text or a decoder change in T042b (S-1).

## Tests

`crates/crpg-net/tests/malicious_server.rs`, public API only. Every bad case
has a control that changes one input and flips the expectation; each test's
control is named. No clock, no sleep, no `#[should_panic]` (a panic anywhere
is a failure). Every assertion message names the wire/body, the fixture and
the input bytes (`{:02X?}`), so a failure is reproducible from the log alone.

- **S1 `fixtures_are_pinned`** (W-1). For each of the 13 fixtures: its
  encoder's bytes equal the literal of W-1, its length and `tag_at` are the
  table's, and `verdict == Accepted(bytes)`. The wire constants (`first_byte`,
  `lane_at`, `lane`) equal the eight version and four lane constants of
  "Interface". *Control:* `D1`'s literal with byte 5 `0x01` is not `D1`'s
  encoding. *Fails if:* an encoder writes a non-canonical form.
- **S2 `short_inputs_are_refused_exactly`** (W-2). 65,793 inputs × 8 wires
  plus 257 × 2 bodies, each against W-2's rule. *Control:* the rule's own
  branches (`[1]` vs `[2]` on `V1`: `Malformed` vs `UnsupportedVersion`;
  `[0x81, 2]` vs `[0x81, 3]` on `T81`: `Malformed` vs `WrongDirection`).
  *Fails if:* a decoder reads the version with a fallback, or a body accepts
  a short input.
- **S3 `oversize_is_refused_first`** (W-3). The six cap constants equal their
  values; per wire/body the inputs of W-3. *Control:* `padded(f, cap)` vs
  `padded(f, cap + 1)` differ by one byte and flip `UnsupportedVersion` (or
  `Malformed` for bodies) to `FrameTooLarge` (or `LimitExceeded`). *Fails if:*
  a cap is `>=`, or version/count is read first.
- **S4 `every_truncation_is_malformed`** (W-4). 13 fixtures, every proper
  prefix. *Control:* the full fixture is `Accepted`.
- **S5 `trailing_bytes_are_malformed`** (W-5). 39 inputs. *Control:* the
  fixture without the extra byte.
- **S6 `foreign_first_bytes_are_unsupported`** (W-6). (a) 77 cross-wire cases
  over the 11 framed fixtures; (b) 11 × 255 version-byte cases. *Control:* each
  fixture to its own wire is `Accepted`.
- **S7 `foreign_lanes_are_wrong_direction`** (W-7). 11 × 255. *Control:* the
  lane value itself gives `Accepted(f)`.
- **S8 `tag_matrix`** (W-8). 11 framed fixtures × 256 values, each against the
  table. *Control:* the table's `Accepted(f)` row. *Fails if:* a delta op tag
  outside the version's set is accepted, or an upstream tag is not
  `WrongDirection`.
- **S9 `zero_ids_and_field_order`** (W-9). Every zero case `Malformed`. On
  `D1`–`D4`: tag `0xFF` → `UnknownMessage`, tag `0xFF` + `entity` 0 →
  `Malformed` (the entity is checked before the tag). *Control:* the
  unmodified field is `Accepted(f)`. *Fails if:* an id check moves after the
  tag or the trailing check.
- **S10 `hostile_lengths_are_bounded`** (W-10). (a)–(e) each exact. *Control:*
  (a) 257 vs 256 flip v2–v4 from `LimitExceeded` to `Malformed`; `Q91` proof
  length 16 is `Accepted`. *Fails if:* a bound waits for the body.
- **S11 `upstream_frames_sent_down_are_refused`** (W-11). The 7 frames:
  literal bytes, the downstream verdict, and the upstream control; then each
  frame to every framed wire with another first byte → `UnsupportedVersion`.
- **S12 `single_byte_corruption_is_refused_or_exact`** (W-12). Every fixture ×
  every index × every other value. Also asserts at least one `Accepted` and one
  `Refused` per fixture. *Control:* `v == f[i]` (unmutated) is `Accepted(f)`.
  *Fails if:* a decoder accepts an alias outside ULID text, or a value its
  encoder refuses.
- **S13 `seeded_noise_is_stable`** (W-13). The three pinned `next()` values
  first, then the inputs of (a)–(d): (a), (b) `stable`; (c) each `Accepted(m)`;
  (d) each `stable`; the `Accepted` count in (c) and (d) is at least 1,536.
  *Control:* a second `XorShift64(SEED)` repeats the pinned values and the same
  verdicts. *Fails if:* a decoder reads the epoch with a check, or an accepted
  value re-encodes to other bytes.
- **S14 `refusal_texts_are_fixed`** (W-14). The six texts of "Errors",
  `assert_eq!(CodecError::X.to_string(), "…")` each.

## What must not change

- Every file under `crates/crpg-net/src/`, byte for byte; every public item
  and `Display` text.
- Every existing test file and `tests/support/`, byte for byte, passing.
- `Cargo.toml`, `Cargo.lock`, `crates/crpg-net/Cargo.toml`; every golden
  (none is added or moved); every other crate.

## Dependency edges

None. The suite uses `crpg-net`'s public API and `crpg_core::Ulid`
(`crpg-core` is a normal dependency, `crates/crpg-net/Cargo.toml:11`); no
dev-dependency is added (`:15–23` unchanged), so `tools/lint/deps.py:54`
(`ALLOWED["crpg-net"] = ({"crpg-core", "crpg-data", "crpg-sim"}, set())`) and
`deny.toml` need nothing. No `proptest`, no fuzzer (Reading 5). No new
external crate, so no user approval is needed.

## Out of scope

- The QUIC `Welcome` decoder and the client's connection close
  (`crpg-net-quic`, `crates/crpg-net-quic/src/handshake.rs:343`): a different
  crate (Reading 1, `AGENTS.md` "One task = one crate").
- The client session's reaction to a decode error
  (`crpg-godot/src/core/client.rs:434`), and the replica/pool `apply_delta`
  consumers (ADR-0035).
- The "log line" itself (Reading 7, T042a Question 1).
- Any decoder fix (S-1), any wire, cap, tag or code change, any new dependency.

## Open questions

1. **Does T042b stay `crpg-net`-only, or grow a second part for the QUIC
   `Welcome` decoder in `crpg-net-quic`?** The `Welcome::decode`/
   `decode_for` functions are the same hostile-input surface but live in
   another crate, which "one task = one crate" forbids a single task from
   touching. *Recommend:* keep T042b `crpg-net`-only now and file a
   `crpg-net-quic` mirror (T042c) for the welcome/close surface, tied to
   player-hosted games (D65b).
2. **Should the client session's reaction to a decode error (the
   `BridgeCode::FrameUndecodable` fault and the QUIC `ProtocolViolation`
   close) be pinned here?** Those are in `crpg-godot`/`crpg-net-quic`, not
   `crpg-net`. *Recommend:* leave them to their own suites; T042b pins only
   the decoder's exact `CodecError`, which is what the session consumes.
3. **Is the body decoder's oversize-`LimitExceeded` (versus the framed
   decoders' `FrameTooLarge`) intended?** The code returns `LimitExceeded` for
   an oversized body (`codec_transfer.rs:98–99`, `transfer_v3.rs:259–260`),
   which W-3 pins. *Recommend:* pin it as-is (test-only; no source change), and
   flag it to the architect as a possible inconsistency to raise with the user.

## Agent log

- 2026-11-19 (UTC) · opencode-go/deepseek-v4-pro + T042b draft a · Wrote the blind hostile-server contract as the mirror of T042a's form over the ten client-side decoders (four deltas, the `0x81`/`0x82`/`0x91`/`0xA1` downstreams, and the two snapshot bodies), re-checking each decoder, cap, version, lane, tag and error against `crates/crpg-net/src/`, with the bodies' oversize-`LimitExceeded` and the movement intent's `UnknownMessage`-not-`WrongDirection` mirror pinned as exact values.
