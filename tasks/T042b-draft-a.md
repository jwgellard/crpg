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
about 500 test lines (`tasks/BACKLOG.md:350`).

Every case feeds bytes a malicious server could send into a client-side
decoder and pins the exact `CodecError` it must produce, with an accepted
control beside it. Test-only: a case that shows a decoder bug is an open
question, never a source change (D67b `tasks/DECISIONS-D67.md:14`).

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
   non-minimal varints (postcard 1.1.3), but no pinned frame has a varint
   followed by `0x00` except an `actor`/`entity` before a tag, so no
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
   fixed texts (`codec.rs:68–79`). Writing the line to a file/console, and the
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

<!-- filled next -->

## Errors

<!-- filled next -->

## Tests

<!-- filled next -->

## What must not change

<!-- filled next -->

## Dependency edges

<!-- filled next -->

## Out of scope

<!-- filled next -->

## Open questions

<!-- filled next -->

## Agent log

<!-- filled next -->
