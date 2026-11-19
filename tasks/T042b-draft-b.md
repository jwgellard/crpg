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

TODO (draft B): numbered readings, each with the alternative rejected and why.

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
