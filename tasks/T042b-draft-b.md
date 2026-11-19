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

TODO (draft B): numbered, each testable.

## Errors

TODO (draft B): each exact variant and when.

## Tests

TODO (draft B): named cases, each with a one-input control, exact expected values, fails-if.

## What must not change

TODO (draft B): existing goldens and suites.

## Dependency edges

TODO (draft B): every new edge, ALLOWED status, no-new-dependency alternative.

## Permissions

TODO (draft B).

## Definition of done

TODO (draft B).

## Out of scope

TODO (draft B).

## Routing and size

TODO (draft B).

## Open questions (each with a recommendation)

TODO (draft B).

## Agent log

- 2026-11-19 (UTC) · opencode-go/muse-spark-1.3-contributor + T042b draft b · Opened the blind-draft skeleton; no behaviour decided yet.
