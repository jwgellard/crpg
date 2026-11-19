# T042b — the hostile-server suite (`crpg-net`) — blind draft B (unapproved)

**Status: draft, unapproved. Another model drafts the same contract blind; nothing here is approved.**

## Task

Give `crpg-net` the hostile-server suite the client side needs before
player-hosted games (D67b in `tasks/DECISIONS-D67.md:14-21`, tied to D65b in
`tasks/DECISIONS-D65.md:11-16`): every decoder a server's bytes reach gets the
same hostile treatment T042a gives the server's eight (`tasks/T042.md:42`).
A malicious server or a middlebox bug controls exactly these bytes: the four
lane-0 delta decoders (v1–v4), the `0x81`/`0x82` snapshot bodies and
downstream frames, the `0x91` downstream resume frames, and the `0xA1`
downstream movement messages. Each case feeds hostile bytes into one of those
client-side decoders and pins the exact refusal, or the exact accepted
canonical bytes beside an accepted control. Test-only: if a case shows a
decoder bug, it is an open question, not a source change (S-1).

The plan splits it the same way T042 splits the client side: T042b is the
decode level in `crpg-net`; session, bridge and transport behaviour live
above it and are indexed, not re-tested (`docs/IMPLEMENTATION_PLAN.md:247`).
The welcome, the QUIC close and the bridge fault mapping are the three
indexed neighbours (§"Readings" 3, §"Behaviour" D-1).

## Parts

One part, one crate (`AGENTS.md` "One task = one crate").

| Part | Crate | Scope | Size (tests + docs) | S-set | Review |
|---|---|---|---|---|---|
| T042b | `crpg-net` | `tests/hostile_server.rs` (H1–H14); one architecture section; one trap | ~500 test lines + ~60 doc lines | no | a free first-pass review, then one OpenCode Go reviewer mutant run on the mutant table ("Definition of done"), one fix round |

T042b is not S-set (`docs/IMPLEMENTATION_PLAN.md:86–98`): it changes no
wire byte, no public item, no `World`, no tick order, no workspace file.

### T042b owns
- `crates/crpg-net/tests/hostile_server.rs` (new; the whole suite)
- `docs/architecture/crpg-net.md`: a new section "Hostile-server suite
  (T042b)" after "Malicious-client suite (T042)" (D-1)
- `crates/crpg-net/AGENTS.md`: one dated Known-traps entry (D-2) and an
  agent-log entry
- goldens: none new, none moved.

## Readings
Design decisions this contract makes. Each needs approval with the contract.

1. **The surface is the ten client-side decoders in `crpg-net`.** A hostile
   server controls only the bytes the client's decoders read:
   `codec::decode_delta` (`crates/crpg-net/src/codec.rs:421`),
   `codec_v2::decode_delta` (`crates/crpg-net/src/codec_v2.rs:595`),
   `codec_v3::decode_delta` (`crates/crpg-net/src/codec_v3.rs:1103`),
   `codec_v4::decode_delta` (`crates/crpg-net/src/codec_v4.rs:428`),
   `codec_transfer::decode_body` (`crates/crpg-net/src/codec_transfer.rs:97`)
   and `decode_down` (`crates/crpg-net/src/codec_transfer.rs:246`),
   `transfer_v3::decode_body` (`crates/crpg-net/src/transfer_v3.rs:258`) and
   `decode_down` (`crates/crpg-net/src/transfer_v3.rs:426`),
   `codec_resume::decode_down` (`crates/crpg-net/src/codec_resume.rs:205`)
   and `movement_v4::decode_down` (`crates/crpg-net/src/movement_v4.rs:318`).
   The upstream decoders (`decode_intent`, `decode_up`, `decode_request`)
   take a client's bytes, not a server's. *Rejected:* folding the QUIC
   welcome, the QUIC client close and the Godot bridge step into this suite
   (Reading 3).
2. **Snapshot bodies are in scope though they have no version or lane.**
   `decode_body` (both versions) reads exactly what a hostile server puts
   in a snapshot: entity lists, health facts, turn, encounter and timeline.
   A body has no first byte, so the first-byte and lane matrices do not
   apply to it; the short-input, truncation, bound and corruption passes do.
   *Rejected:* a frame-only scope (it would leave the largest hostile
   surface — a 1 MiB / 4 MiB reassembled body — to the per-codec suites,
   which never corrupt it byte by byte).
3. **The welcome, the close and the bridge step are indexed, not re-tested.**
   One crate per task (`AGENTS.md`): `Welcome::decode`
   (`crates/crpg-net-quic/src/handshake.rs:343`), the welcome-mismatch
   close (`crates/crpg-net-quic/src/client.rs:1064-1065`) with
   `CloseCode::ProtocolViolation`, `try_recv`/`close_reason`
   (`crates/crpg-net-quic/src/client.rs:450`, `:610`) and the Godot
   `Client::step` decode/replica mapping to 52/53
   (`crates/crpg-godot/src/core/client.rs:442-448`,
   `crates/crpg-godot/src/core/errors.rs:46-48`) live in other crates with
   their own suites; D-1 names the exact lines that pin them. *Rejected:* a
   three-crate part now (it breaks the one-crate rule and needs a
   `deps.py` review for every new test edge; filed as Open question 1).
4. **Stateless decoders have no replay, timing or ownership case.**
   A decoder is a function of one byte slice, so a replayed delta, a stale
   tick or a foreign entity has no byte-level form beyond a bad scalar.
   What exists at this level is indexed, not re-tested (D-1): the epoch is
   opaque to every decoder (W-13 proves it), receipt lanes are checked
   (`crates/crpg-net/src/codec.rs:319-321`), zero ids are `Malformed`
   (W-9). *Rejected:* per-decoder replay tests (pass by construction) and
   re-tests of the bridge fence.
5. **Corruption is exhaustive, and acceptance must be stable.** For each
   pinned frame, every byte is replaced by every other value; the result
   must be a refusal, or a frame whose canonical re-encoding is the
   corrupted bytes themselves. `postcard` accepts non-minimal varints
   (the T042a leniency, `tasks/T042.md:131-139`), and no pinned frame has
   a varint byte followed by `0x00` except a zero count, where the shifted
   value is refused — so no single-byte corruption reaches that leniency
   except through `stable`'s re-encode branch, which the seeded pass
   exercises. A seeded corpus adds multi-byte noise under the weaker rule
   the client needs: a decoded value re-encodes, and the re-encoding
   decodes back to itself. *Rejected:* `proptest` (a new `crpg-net`
   dev-dependency, needs asking, `AGENTS.md` "Do not add dependencies").
6. **The v3 body literal is encoder-derived, not hand-written.** The empty
   v3 body's encounter/timeline tail encoding was not hand-verified at
   draft time, so H1 pins `B82` byte-for-byte against `encode_body` on
   every run instead of against a hand-written literal: any encoder change
   fails H1 loudly, and the verdict properties (W-1–W-5, W-10) do not
   depend on the tail. *Rejected:* hand-writing the tail (risks pinning a
   wrong literal that a later task must "fix" by weakening the test).
7. **The "log line" at this level is the refusal and its fixed text.**
   `crpg-net` does no I/O; every refusal here is a `CodecError` whose
   `Display` is one of six fixed texts
   (`crates/crpg-net/src/codec.rs:72-83`), and the client layers map it to
   a fenced fault (52/53) or a close (D-1). Writing a log line is D67a
   (a): pinned here, owned by the program. *Rejected:* a logging library
   (a new dependency across client crates, D67a (b)).

## Interface
**Production code: none.** No public item, signature, variant, constant or
`Display` text of `crpg-net` changes; `src/` is untouched. Everything below
is test-local, in `crates/crpg-net/tests/hostile_server.rs`, with these
exact shapes (names of private helpers may not change: the review and later
tasks cite them).

```rust
//! T042b: the hostile-server suite (D67b/D65b).
//! Every client-side decoder, every hostile class, one table. No clock,
//! no thread, no I/O, no unseeded RNG, no `HashMap`/`HashSet`, no float.

/// One client-side decoder with its own encoder (Reading 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wire { V1, V2, V3, V4, C81, C82, J91, M1 }

/// Table order; every "for each wire" loop uses it.
const WIRES: [Wire; 8] = [Wire::V1, Wire::V2, Wire::V3, Wire::V4,
                          Wire::C81, Wire::C82, Wire::J91, Wire::M1];

impl Wire {
    /// 1, 2, 3, 4, 0x81, 0x82, 0x91, 0xA1.
    fn first_byte(self) -> u8;
    /// 1 for every wire (deltas: `[version, lane, ...]`; framed down wires:
    /// `[version, lane, tag/kind, ...]`).
    fn lane_at(self) -> usize;
    /// The lane byte: 0, 0, 0, 0, 2, 2, 3, 1.
    fn lane(self) -> u8;
    /// Decode with this wire's decoder; on `Ok(v)`, re-encode `v` with this
    /// wire's encoder. An encoder error on a decoded value panics with the
    /// wire, the input bytes and the error (the test fails: W-12, W-13).
    fn verdict(self, bytes: &[u8]) -> Verdict;
}

/// Bodies have no wire bytes: they are decoded bare.
enum Body { B81, B82 }

impl Body {
    /// Decode with this body's decoder; re-encode on `Ok` (same rule).
    fn verdict(self, bytes: &[u8]) -> Verdict;
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Verdict {
    /// The decoder refused the bytes with this error.
    Refused(CodecError),
    /// The decoder accepted them; the canonical re-encoding of the value.
    Accepted(Vec<u8>),
}

/// One pinned valid frame (the table in W-1).
struct Fixture { name: &'static str, wire: Wire, bytes: Vec<u8>, tag_at: usize }

/// The 10 framed fixtures of W-1, in table order, each built by its wire's
/// encoder from the value in that table (never from hand-written bytes).
fn fixtures() -> Vec<Fixture>;

/// The 2 body fixtures of W-1 (`B81` literal, `B82` encoder-derived).
fn bodies() -> Vec<(Body, Vec<u8>)>;

/// `bytes` with `bytes[at]` replaced by `value`; every other byte, the valid
/// remainder after `at` included, unchanged.
fn with_byte(bytes: &[u8], at: usize, value: u8) -> Vec<u8>;

/// `bytes` followed by `0x00` up to `len` bytes (`len >= bytes.len()`).
fn padded(bytes: &[u8], len: usize) -> Vec<u8>;

/// W-13's rule: `Refused(_)`, or `Accepted(c)` with `verdict(&c) == Accepted(c)`.
fn stable(wire: Wire, bytes: &[u8]) -> bool;

/// Same rule for bodies.
fn stable_body(body: Body, bytes: &[u8]) -> bool;

/// Marsaglia xorshift64 (shifts 13, 7, 17), wrapping `u64` arithmetic,
/// identical to T042a's (`tasks/T042.md:222-224`).
struct XorShift64(u64);
impl XorShift64 { fn next(&mut self) -> u64; }   // state ^= state << 13; state ^= state >> 7; state ^= state << 17; returns state
const SEED: u64 = 0x5EED_0000_0000_0042;
```

`Wire::verdict` calls exactly:

| Wire | Decoder | Encoder |
|---|---|---|
| `V1` | `codec::decode_delta` | `codec::encode_delta` |
| `V2` | `codec_v2::decode_delta` | `codec_v2::encode_delta` |
| `V3` | `codec_v3::decode_delta` | `codec_v3::encode_delta` |
| `V4` | `codec_v4::decode_delta` | `codec_v4::encode_delta` |
| `C81` | `codec_transfer::decode_down` | `codec_transfer::encode_down` |
| `C82` | `transfer_v3::decode_down` | `transfer_v3::encode_down` |
| `J91` | `codec_resume::decode_down` | `codec_resume::encode_down` |
| `M1` | `movement_v4::decode_down` | `movement_v4::encode_down` |

`Body::verdict` calls `codec_transfer::decode_body` / `encode_body`
(`B81`) and `transfer_v3::decode_body` / `encode_body` (`B82`).

## Behaviour
Normative. Each item names its tests. Every expected value below follows
from the cited code; the implementer does not adjust one to make a test
pass (S-1).

W-1. **The fixtures.** Every framed fixture is built by its wire's encoder
and equals its bytes (`verdict == Accepted(bytes)`, each canonical).
`EPOCH` = `[0x01, …, 0x10]` (`tasks/T042.md:23`).

| Fixture | Wire | Value | Bytes | Len | tag_at |
|---|---|---|---|---|---|
| `F1`–`F4` | `V1`–`V4` | `DeltaFrame { lane: 0, server_tick: 5, event_seq: 1, ops: vec![] }` | `[v] ++ [0x00, 0x05, 0x01, 0x00]` (postcard: `u8` raw, `u64` 5/1/0 as one byte) | 5 | count at 4, no tag |
| `B81` | body | `SnapshotBody { entities: vec![], turn: None }` | `[0x00, 0x00]` (count 0, `turn_present` false; `crates/crpg-net/src/codec_transfer.rs:58-76`) | 2 | — |
| `B82` | body | the empty `SnapshotBodyV3` (Reading 6) | `encode_body` output, pinned by H1 | — | — |
| `C81`, `C82` | `T81`/`T82` | `TransferDown::Cancel { epoch: EPOCH, transfer: tid(1), reason: Aborted }` | `[w, 0x02, 0x12] ++ EPOCH ++ [0x01, 0x02]` (`Aborted = 2`, `crates/crpg-net/src/protocol_transfer.rs:183-199`; layout `crates/crpg-net/src/transfer_v3.rs:399-419`) | 21 | 2 |
| `J91` | `R91` | `ResumeDown::Ticket { epoch: EPOCH, proof: [0xA5; 16] }` | `[0x91, 0x03, 0x30] ++ EPOCH ++ [0x10] ++ [0xA5; 16]` (proof len `u8`, `crates/crpg-net/src/codec_resume.rs:187-199`) | 36 | 2 |
| `A91` | `R91` | `ResumeDown::Answer(Rejected(Busy))` | `[0x91, 0x03, 0x32] ++ EPOCH ++ [0x02]` (`Busy = 2`, `crates/crpg-net/src/protocol_resume.rs:200-220`) | 20 | 2 |
| `M1` | `A1` | `MoveDown::Receipt(MoveReceipt { epoch: EPOCH, seq: 1, actor: net(7), processed_tick: 41, outcome: Accepted { start_tick: 42 } })` | `[0xA1, 0x01, 0x00] ++ EPOCH ++ [0x01, 0x07, 0x29, 0x00, 0x2A]` (`crates/crpg-net/src/movement_v4.rs:32-48`, `:275-295`) | 24 | kind at 2 |

Table order is `F1, F2, F3, F4, B81, B82, C81, C82, J91, A91, M1`;
`w` is the wire's first byte. *Tests:* H1.

W-2. **Short inputs, exhaustively.** For every framed wire and every input
of at most two bytes: `[]` → `Malformed` (empty checked explicitly on
`C81`/`C82`/`J91`/`M1`, `crates/crpg-net/src/codec_transfer.rs:251-253`,
`crates/crpg-net/src/transfer_v3.rs:430-432`,
`crates/crpg-net/src/codec_resume.rs:209-211`,
`crates/crpg-net/src/movement_v4.rs:322`; on deltas the version read fails:
`crates/crpg-net/src/codec.rs:426`,
`crates/crpg-net/src/codec_v2.rs:600`,
`crates/crpg-net/src/codec_v3.rs:1108`,
`crates/crpg-net/src/codec_v4.rs:433`); `[b]` → `Malformed` if `b` is the
wire's first byte, else `UnsupportedVersion`; `[b, c]` → `UnsupportedVersion`
if `b` is not the first byte, else `WrongDirection` if `c` is not the wire's
lane, else `Malformed`. For bodies: `[]` and every 1-byte input →
`Malformed`; every 2-byte input → `Malformed`, except `B81`'s own
`[0x00, 0x00]` (`Accepted`) and any input whose decoded count exceeds
`MAX_VISIBLE_ENTITIES` (`LimitExceeded`,
`crates/crpg-net/src/codec_transfer.rs:102-105`,
`crates/crpg-net/src/transfer_v3.rs:263-266`). No input this short reaches
a tag. *Tests:* H2.

W-3. **Oversize is refused first.** Per-wire caps, each checked before
anything else: deltas 65,536 (`MAX_DELTA_FRAME_BYTES`,
`crates/crpg-net/src/protocol.rs:64`); `C81`/`C82` 65,536
(`MAX_DOWN_FRAME_BYTES`, `crates/crpg-net/src/protocol_transfer.rs:46`);
`J91` 4,096 (`MAX_RESUME_FRAME_BYTES`,
`crates/crpg-net/src/protocol_resume.rs:40`); `M1` 1,024
(`MAX_MOVE_DOWN_BYTES`, `crates/crpg-net/src/movement_v4.rs:50`);
`B81` 1 MiB → `LimitExceeded`
(`crates/crpg-net/src/codec_transfer.rs:97-100`); `B82` 4 MiB →
`LimitExceeded` (`crates/crpg-net/src/transfer_v3.rs:258-261`). Per wire:
`[0xFF; cap + 1]` → the cap error (cap beats version); `[0xFF; cap]` →
`UnsupportedVersion` (framed) or `LimitExceeded` (bodies); each fixture
padded to `cap` → `Malformed` (trailing bytes), padded to `cap + 1` → the
cap error. *Tests:* H3.

W-4. **Every truncation is `Malformed`.** For every fixture and every
`n < len`, the prefix of length `n` → `Malformed`: a prefix keeps every
header value, so the version, lane and tag checks pass and the first read
past the end fails (`postcard` errors map to `Malformed`,
`crates/crpg-net/src/codec_transfer.rs:16-21`). A prefix never decodes an
over-bound length: fixture counts and proof lengths are minimal varints the
prefix keeps whole. *Tests:* H4.

W-5. **Trailing bytes are `Malformed`.** For every fixture, `f ++ [t]` for
`t` in `0x00, 0x01, 0xFF` → `Malformed` (empty-remainder checks:
`crates/crpg-net/src/codec.rs:445-447`,
`crates/crpg-net/src/codec_v2.rs:619-621`,
`crates/crpg-net/src/codec_v3.rs:1127-1129`,
`crates/crpg-net/src/codec_v4.rs:451-453`,
`crates/crpg-net/src/codec_transfer.rs:379-381`,
`crates/crpg-net/src/transfer_v3.rs:554-556`,
`crates/crpg-net/src/codec_resume.rs:226-229`,
`crates/crpg-net/src/movement_v4.rs:337-340`,
`crates/crpg-net/src/codec_transfer.rs:150-152`). *Tests:* H5.

W-6. **A foreign first byte is `UnsupportedVersion`.** (a) Every framed
fixture, given to each of the other seven wires → `UnsupportedVersion`
(the eight first bytes are distinct; every fixture is under every cap and
non-empty). (b) Every framed fixture with byte 0 replaced by each of the
255 other values → `UnsupportedVersion` from its own wire. Bodies have no
version byte: not applicable. *Tests:* H6.

W-7. **A foreign lane is `WrongDirection`.** Every framed fixture with the
byte at `lane_at` (1) replaced by each of the 255 other values →
`WrongDirection`; the lane is checked as read, right after the version
(`crates/crpg-net/src/codec.rs:430-433`,
`crates/crpg-net/src/codec_v2.rs:604-607`,
`crates/crpg-net/src/codec_v3.rs:1112-1115`,
`crates/crpg-net/src/codec_v4.rs:437-440`,
`crates/crpg-net/src/codec_transfer.rs:259-262`,
`crates/crpg-net/src/transfer_v3.rs:438-441`,
`crates/crpg-net/src/codec_resume.rs:217-220`,
`crates/crpg-net/src/movement_v4.rs:327-330`). *Tests:* H7.

W-8. **Tags, kinds and counts.** (a) Three-byte `[w, lane, t]` inputs: on
`C81`/`C82`, `t` in `0x10, 0x11, 0x12` → `Malformed` (the body is missing),
`0x20, 0x21` → `WrongDirection`
(`crates/crpg-net/src/codec_transfer.rs:388`,
`crates/crpg-net/src/transfer_v3.rs:563`), every other `t` →
`UnknownMessage`; on `J91`, `0x30, 0x31, 0x32` → `Malformed`,
`0x40` → `WrongDirection`
(`crates/crpg-net/src/codec_resume.rs:267`), every other `t` →
`UnknownMessage`; on `M1`, `0x00, 0x01` → `Malformed`, every other kind →
`UnknownMessage` (`crates/crpg-net/src/movement_v4.rs:332-336`). (b) Delta
op-count matrix: `F1`–`F4` with byte 4 := `c` for `c` in `1..=255` →
`Malformed` (the op tag read fails on the empty remainder); with the count
replaced by the two bytes `[0x81, 0x02]` (257, no ops) → `LimitExceeded`
(`crates/crpg-net/src/codec.rs:436-439 and its v2–v4 twins). (c) `A91`
with reason byte (19) := `0x01` → `Accepted` (`Denied`);
`:= 0x02` → `Accepted(f)`; every other value → `UnknownMessage`
(`crates/crpg-net/src/protocol_resume.rs:213-220`,
`crates/crpg-net/src/codec_resume.rs:257-259`). *Tests:* H8.

W-9. **Zero ids and sequences.** Each → `Malformed`: `C81`/`C82` transfer
byte (19) := 0 (`take_transfer_id`,
`crates/crpg-net/src/codec_transfer.rs:23-26`); `J91` proof-length byte
(19) := 0 (below 16, `crates/crpg-net/src/codec_resume.rs:30-41`); `M1`
`seq` byte (19) := 0 and `actor` byte (20) := 0
(`crates/crpg-net/src/movement_v4.rs:179-194`); `B81`/`B82` entity id 0
(`NetId::new`, `crates/crpg-net/src/protocol.rs:219-227`, applied at
`crates/crpg-net/src/codec_transfer.rs:108-110`,
`crates/crpg-net/src/transfer_v3.rs:271-272`). *Tests:* H9.

W-10. **Hostile lengths are bounded before their bodies.** (a) `B81` with
count `[0x81, 0x02]` (257, no body) → `LimitExceeded`; with `[0x80, 0x02]`
(256, no body) → `Malformed`
(`crates/crpg-net/src/codec_transfer.rs:102-105`). (b) `B82` the same two
inputs → `LimitExceeded` / `Malformed`
(`crates/crpg-net/src/transfer_v3.rs:263-266`). (c) `J91` with proof length
(19) := 129 → `LimitExceeded`; := 128 → `Malformed` (19 bytes follow);
:= 15 → `Malformed`
(`crates/crpg-net/src/codec_resume.rs:30-45`); claim count (37) := 5 →
`LimitExceeded`; := 0 → `Malformed`
(`crates/crpg-net/src/codec_resume.rs:47-57`). (d) Delta count 257 →
`LimitExceeded` (W-8 (b)). *Tests:* H10.

W-11. **Upstream frames fed downstream are refused.** Each built by its
upstream encoder, with a downstream control: `P81`/`R81`
(`tasks/T042.md:257-258`: `[0x81, 0x02, 0x20/0x21] ++ EPOCH ++ …`) to
`C81` → `WrongDirection`; `P82`/`R82` to `C82` → `WrongDirection`
(`crates/crpg-net/src/transfer_v3.rs:563`); the `R91` request
(`tasks/T042.md:259`: `[0x91, 0x03, 0x40] ++ EPOCH ++ …`) to `J91` →
`WrongDirection` (`crates/crpg-net/src/codec_resume.rs:267`); the `MT`
intent (`tasks/T042.md:260`: `[0xA1] ++ EPOCH ++ [0x01, 0x01, …]`) to `M1`
→ `UnknownMessage` (byte 1 is `EPOCH[0]` = `0x01` = the lane, byte 2 is
`EPOCH[1]` = `0x02` = no kind,
`crates/crpg-net/src/movement_v4.rs:327-336`). Every framed fixture, given
to each wire with another first byte → `UnsupportedVersion`. *Tests:* H11.

W-12. **Single-byte corruption is refused or stable.** For every fixture
`f` (wire or body `w`), every index `i < len` and every value `v != f[i]`
with `m = with_byte(&f, i, v)`, `w.verdict(&m)` is one of: `Refused(_)`
other than the cap error (`FrameTooLarge`/`LimitExceeded` from a cap, never
from a field bound the input did not cross); `Accepted(m)`; or
`Accepted(c)` with `c != m` only through canonical re-encoding and
`verdict(&c) == Accepted(c)`. Never a panic, never an encoder error. In
particular every `J91` epoch-byte (3–18) mutation is `Accepted(m)` exactly
(the epoch is opaque), and `M1`'s outcome byte admits no alias
(`Accepted(m)` only for the same outcome). *Tests:* H12.

W-13. **Seeded noise is stable.** One `XorShift64(SEED)` drives, in this
order: (a) for each wire in `WIRES` order, 1,024 inputs: `len = 1 +
(next() % 64)`, then `len` bytes each `next() as u8`, then byte 0 := the
wire's first byte and byte 1 := its lane; each must be `stable` for that
wire; (b) 1,024 body inputs of `len = next() % 65` bytes, each `stable_body`
for both bodies; (c) for each of the 5 epoch-carrying framed fixtures
(`C81, C82, J91, A91, M1`) in table order, 256 inputs: `i = 3 +
(next() % 16)`, `v = next() as u8`, `m = with_byte(f, i, v)`; the 16 epoch
bytes are opaque to every decoder, so each must be `Accepted(m)` exactly
(1,280 inputs, all accepted); (d) for each of the 10 framed fixtures, 256
inputs: `i = tag_at + (next() % (len − tag_at))`, `v = next() as u8`, each
`stable` for its own wire (2,560 inputs, a mix of refusals and accepted
re-encodings). In all 11,944 framed inputs plus 1,024 body inputs; the test
counts the `Accepted` verdicts of (c) and (d) together and asserts at least
1,280 (the floor is (c)'s, so the accepted branch of `stable` is proved
independently of the noise). The first three `next()` values from `SEED`
are `0x4B10_DA10_A18C_50C2`, `0x8788_B83B_1805_38E3`,
`0x18B5_5443_1CC3_6A52` (`tasks/T042.md:403`). *Tests:* H13.

W-14. **Refusal texts.** The six `CodecError` texts are exactly those of
"Errors"; every refusal any test sees is one of the six variants.
*Tests:* H14.

S-1. **Stop rule.** If any case fails against the part's base `src/`, the
implementer does not change a decoder, an expected value or a fixture: it
stops and reports the case, the input bytes and the actual verdict
(Reading 6, `AGENTS.md` "Do not weaken").

### Documentation (T042b)
D-1. **Section "Hostile-server suite (T042b)"** in
`docs/architecture/crpg-net.md`, after "Malicious-client suite (T042)": two
sentences on the surface (Reading 1) and the stability rule (Reading 5),
then exactly this table:

| Hostile input (server → client) | Decode level, `crpg-net` | Above the decoder |
|---|---|---|
| Oversized packets | `hostile_server.rs` H3 | `crpg-net-quic` length-prefix caps (opaque bytes, never decoded there) |
| Malformed frames | `hostile_server.rs` H2, H4–H10, H12–H13 | the per-codec suites |
| Hostile snapshot bodies | `hostile_server.rs` H2–H5, H9–H10, H12–H13 (B81/B82) | receivers apply only decoded values |
| Wrong-direction tags | `hostile_server.rs` H7–H8, H11 | — |
| Replay / stale ticks | none in a decoder (stateless); epochs opaque (H13 (c)) | T041; the bridge fence |
| Clean rejection text | every refusal is a `CodecError` with a fixed text (H14) | 52/53 fence (`crates/crpg-godot/src/core/client.rs:442-448`); QUIC close (`crates/crpg-net-quic/src/client.rs:1064-1065`) |

D-2. **Trap** in `crates/crpg-net/AGENTS.md`, dated: "*2026-11-19 (UTC),
T042b:* **A new client-side decoder is a new `Wire` row in
`tests/hostile_server.rs`** (with its fixtures), in the same task that adds
the decoder; the suite is the whole server-to-client byte boundary.
**Never relax `stable` or W-12's exactness to admit a decoder change**:
a decoded value that does not re-encode, or re-encodes to bytes that decode
differently, is a defect. Bodies have no version or lane: H6–H7 never apply
to them, and H2 states their rule separately."
D-3. **No `src/` edits.** This contract corrects no comment and no
invariant; if the implementer finds a doc mismatch, it stops and reports it
as an open question (S-1).

## Errors
No new error type or variant. Every refusal is an existing `CodecError`
(`crates/crpg-net/src/codec.rs:52-70`); its `Display` is the "log line"
material (Reading 7):

| Variant | `Display` (exact) | What the client does with it |
|---|---|---|
| `FrameTooLarge` | `frame exceeds its byte cap` | fence 52 (`crates/crpg-godot/src/core/errors.rs:46`) |
| `UnsupportedVersion` | `unsupported protocol version` | fence 52 |
| `WrongDirection` | `wrong lane or direction` | fence 52 |
| `UnknownMessage` | `unknown message discriminant` | fence 52 |
| `Malformed` | `malformed frame` | fence 52 |
| `LimitExceeded` | `wire bound exceeded` | fence 52 |

A decoded-but-inapplicable value is the replica's `ReplicaRejected` (53,
`crates/crpg-godot/src/core/errors.rs:48`); a bad welcome closes the
connection with `ProtocolViolation`
(`crates/crpg-net-quic/src/client.rs:1064-1065`). The welcome's own errors
are `HandshakeError` (`crates/crpg-net-quic/src/handshake.rs:362-371`):
`Malformed`, `UnsupportedWireVersion`, `CredentialLength`, displayed as
`"<Name> at quic/handshake"`
(`crates/crpg-net-quic/src/handshake.rs:416-425`) — indexed, not re-tested
(Reading 3).

A failing case is never answered by a new variant, a changed text or a
decoder change in T042b (S-1).

## Tests
`crates/crpg-net/tests/hostile_server.rs`, public API only. Every bad
case has a control that changes one input and flips the expectation;
each test's control is named. No clock, no sleep, no `#[should_panic]`
(a panic anywhere is a failure). Every assertion message names the wire,
the fixture and the input bytes (`{:02X?}`), so a failure is reproducible
from the log alone.

- **H1 `fixtures_are_pinned`** (W-1). For each of the 10 framed fixtures:
  its encoder's bytes equal the literal of W-1, its length and `tag_at`
  are the table's, and `verdict == Accepted(bytes)`. For `B81`: encoder
  bytes equal `[0x00, 0x00]`. For `B82`: encoder bytes are `stable_body`
  and decode to `Accepted`. The wire constants: `first_byte` equal
  `PROTOCOL_VERSION` 1–4, `TRANSFER_VERSION`, `TRANSFER_VERSION_V3`,
  `RESUME_VERSION`, `MOVEMENT_VERSION`
  (`crates/crpg-net/src/protocol.rs:25`,
  `crates/crpg-net/src/protocol_v2.rs:34`,
  `crates/crpg-net/src/protocol_v3.rs:40`,
  `crates/crpg-net/src/protocol_v4.rs:27`,
  `crates/crpg-net/src/protocol_transfer.rs:19`,
  `crates/crpg-net/src/transfer_v3.rs:39`,
  `crates/crpg-net/src/protocol_resume.rs:18`,
  `crates/crpg-net/src/movement_v4.rs:32`); `lane` equal `LANE_COMBAT`,
  `LANE_SNAPSHOT`, `LANE_SESSION`, `LANE_MOVEMENT`.
  *Control:* `M1`'s literal with byte 2 `0x01` is not `M1`'s encoding (it
  is a sample header, refused as a receipt body).
- **H2 `short_inputs_are_refused_exactly`** (W-2). Per-wire short-input
  tables of W-2, exhaustively (framed wires: 1 + 256 + 65,536 inputs each).
  *Control:* the rule's own branches (`[1]` vs `[2]` on `V1`: `Malformed`
  vs `UnsupportedVersion`; `[0x91, 3]` vs `[0x91, 4]` on `J91`: `Malformed`
  vs `WrongDirection`; `B81` `[0x00, 0x00]` vs `[0x00, 0x01]`: `Accepted`
  vs `Malformed`).
- **H3 `oversize_is_refused_first`** (W-3). Per-wire caps equal the named
  constants; per wire the three inputs of W-3 (framed wires), and the cap /
  cap+1 pair for bodies. *Control:* padded-to-cap vs padded-to-cap+1
  differ by one byte and flip `Malformed` to the cap error. *Fails if:* a
  cap is read after the version, or a body cap returns `FrameTooLarge`.
- **H4 `every_truncation_is_malformed`** (W-4). 12 fixtures (bodies
  included), every proper prefix. *Control:* the full fixture is
  `Accepted`.
- **H5 `trailing_bytes_are_malformed`** (W-5). 12 fixtures × 3. *Control:*
  the fixture without the extra byte.
- **H6 `foreign_first_bytes_are_unsupported`** (W-6). (a) 8 × 7
  cross-wire cases; (b) 10 × 255 version-byte cases. *Control:* each
  fixture to its own wire is `Accepted`.
- **H7 `foreign_lanes_are_wrong_direction`** (W-7). 10 × 255. *Control:*
  the lane value itself gives `Accepted(f)`.
- **H8 `tag_kind_and_count_matrix`** (W-8). (a) The 3-byte inputs for
  `C81`/`C82`/`J91`/`M1` against the table; (b) `F1`–`F4` count bytes
  1–255 plus the 257-count input; (c) `A91` reason bytes all 256 values.
  *Control:* the table's `Accepted` row (`A91` reason `0x02`,
  `F1`–`F4` count 0).
- **H9 `zero_ids_are_malformed`** (W-9). Every zero case `Malformed`.
  *Control:* the unmodified field is `Accepted(f)`. *Fails if:* a zero
  `NetId`, fence-adjacent length or `seq` check moves after the body.
- **H10 `hostile_lengths_are_bounded`** (W-10). (a)–(d), each exact.
  *Control:* (a) vs (b) differ only in the count (257 vs 256) and flip
  `LimitExceeded` to `Malformed`; `J91` proof length 16 is `Accepted`.
  *Fails if:* a bound waits for its body.
- **H11 `upstream_frames_sent_down_are_refused`** (W-11). The 4
  upstream-into-downstream cases with literal bytes, verdicts and
  downstream controls; then each framed fixture to every wire with another
  first byte → `UnsupportedVersion`.
- **H12 `single_byte_corruption_is_refused_or_stable`** (W-12). Every
  fixture, every index, every value. Also asserts, so both branches of the
  rule are exercised: at least one `Accepted` and one `Refused` verdict
  across the corpus; every `J91` epoch-byte mutation `Accepted(m)`;
  `M1` outcome byte mutations never alias. *Control:* `v == f[i]`
  (unmutated) is `Accepted(f)`. *Fails if:* a decoder accepts an alias
  outside canonical re-encoding, or accepts a value its encoder refuses.
- **H13 `seeded_noise_is_stable`** (W-13). The three pinned `next()`
  values first, then (a) 8 × 1,024 framed noise inputs `stable`, (b) 1,024
  body noise inputs `stable_body` for both bodies, (c) 1,280 epoch-byte
  inputs each `Accepted(m)`, (d) 2,560 body-byte inputs each `stable`; and
  the number of `Accepted` verdicts in (c) and (d) together is at least
  1,280 (so the re-encode branch of `stable` carries a fixed share of the
  inputs, not one control). *Control:* `stable(Wire::C81,
  &with_byte(&C81, 2, 0x12))` is `true` through the `Accepted` branch
  (W-1), and a second `XorShift64(SEED)` repeats the three pinned values
  and the same verdicts (the run is deterministic). *Fails if:* a decoder
  reads the epoch with a check, or an accepted value re-encodes to other
  bytes.
- **H14 `refusal_texts_are_fixed`** (W-14). The six texts of "Errors",
  `assert_eq!(CodecError::X.to_string(), "…")` each.

## What must not change
- Every file under `crates/crpg-net/src/`, byte for byte; every public
  item and `Display` text.
- Every existing test file and `tests/support/`, byte for byte, passing.
- `Cargo.toml`, `Cargo.lock`, `crates/crpg-net/Cargo.toml`; every golden
  (none is added or moved); every other crate.

## Dependency edges
None. The suite uses `crpg-net`'s public API only; no dev-dependency is
added, so `tools/lint/deps.py:54` (`ALLOWED["crpg-net"]`) and `deny.toml`
need nothing. No `proptest`, no fuzzer (Reading 5). The indexed neighbours
(welcome, close, bridge) are cited, never imported: a test edge into
`crpg-net-quic` or `crpg-godot` would need an `ALLOWED` review and the
user's approval, which is why this contract does not create one
(Reading 3, Open question 1).

## Permissions
- goldens: none.
- protected: none edited; `crpg-contracts`, `rust-toolchain.toml` untouched.
- sanctioned edits: the new test file; D-1's architecture section;
  D-2's trap entry in `crates/crpg-net/AGENTS.md` (dated, additive only).
- platform: native Windows/MSVC and Linux (WSL), like every crate
  (ADR-0012); the suite has no platform branch.

## Definition of done
On the part's branch, all green:
```
cargo fmt --all
cargo clippy -p crpg-net --all-targets --locked -- -D warnings
cargo test -p crpg-net --locked
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
git diff --check
git diff --exit-code <base> -- Cargo.toml Cargo.lock crates/crpg-net/Cargo.toml
git diff -U0 <base> -- crates/crpg-net/src | grep -E '^[+-][^+-]' | grep -vE '^[+-][[:space:]]*//'
```
The last command prints nothing (no `src/` change at all in T042b).
`git diff --name-only <base>` lists only the owned files; every W item has
its named test; docs carry a signed agent-log entry; the DoD count in the
trap is the new `cargo test -p crpg-net` pass count.

**Mutant table** (the reviewer plants each in a scratch copy, runs
`cargo test -p crpg-net --test hostile_server`, and expects the named
test to fail; a surviving mutant is a finding):

| # | Mutant | Killed by |
|---|---|---|
| MU1 | one framed decoder's cap `>` → `>=` | H3 |
| MU2 | one decoder's trailing-byte check removed | H5 |
| MU3 | one framed decoder's version check removed | H6 |
| MU4 | one framed decoder's lane check removed | H7 |
| MU5 | `codec_transfer::decode_down` maps `0x20`/`0x21` to `UnknownMessage` | H8, H11 |
| MU6 | `transfer_v3::decode_down` accepts `total_bytes` 0 | H9 (zero-transfer twin) |
| MU7 | `codec_resume::take_proof` bound `>` → `>=` | H10 |
| MU8 | `codec_resume::take_lanes` count bound removed | H10 |
| MU9 | `movement_v4::take_seq` accepts 0 | H9 |
| MU10 | `ResumeRejection::from_u8` maps 3 to `Busy` | H8 |

## Out of scope

TODO (draft B).

## Routing and size

TODO (draft B).

## Open questions (each with a recommendation)

TODO (draft B).

## Agent log

- 2026-11-19 (UTC) · opencode-go/muse-spark-1.3-contributor + T042b draft b · Opened the blind-draft skeleton; no behaviour decided yet.
