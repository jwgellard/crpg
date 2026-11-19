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

TODO (draft B): exact Rust types and signatures.

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
