# ADR-0034: Interest projection machinery

Date: 2026-11-02 (UTC)
Status: **Accepted** — filed by T027b (D41a). Authoritative contract:
`tasks/T027b.md` §"Exact contract".

## Context

`crpg-net` needed the H-filter decision that decides what each player's client
may receive. T027b implements it as a net-owned, per-viewer projection driven
by host-supplied facts: the host keeps frames, chunking, `server_tick` and
`event_seq`, while `InterestView` turns one viewer's context into existing v1
`DeltaOp` values or one T024 `SnapshotBody`. No wire form is added. The host
still owns authentication, grants, area derivation and every gameplay rule.

## Decision

1. **New module `interest`; output is ops, not frames.** Frames, chunking,
   `server_tick` and `event_seq` stay with the host because the same sequence
   numbers receipts and history.
2. **Identity is `(area, full EntityId)`.** Two areas may mint equal `EntityId`s;
   keying on the id alone would alias a viewer's old `NetId` onto a different
   entity after an area change, with no leave and no enter.
3. **Net applies D14's same-area rule and invents no value.** A fact is visible
   when its area equals the viewer's and its presence grant is set. A foreign
   row is suppressed, not an error, so a host mistake fails closed. One view is
   one viewer in one area; there is no multi-area or visible-to oracle here.
4. **Disclosure needs a value and a per-row grant.** `FieldMask::default()`
   denies everything; plain `bool` fields (`presence`, `health`) match the
   host's per-entity grants and reach no wire, so `from_bits`/`union` would be
   dead API. There is no `action_result` bit: action results stay per-field on
   `EventCandidate`.
5. **Each op has exactly one owner.**

   | Op | Owner |
   |---|---|
   | `EntityEnter`, `Health`, `Turn`, `EntityLeave`, `Despawned` | `InterestView` |
   | `Spawned`, `Died`, `ActionResolved`, `TurnStarted`, `EncounterEnded` | `project_events` |
   | `Receipt` | host |

   `Spawned` and `Died` stay in history because their journal position is
   pinned. `Despawned` belongs to the view so a removal is exactly one op:
   leave or notice, never both.
6. **A page has a fixed layout and a history slot.** `EntityEnter`s first, then
   `Health`, at most one `Turn`, then removals; each section ascending `NetId`.
   `StateProjection::enters` marks where history goes.
7. **Retirement is derived, not stored.** Ids are minted monotonically and only
   for disclosed keys, so an id is retired exactly when it is below `next_id`
   and not live. There is no grave table to bound, overflow or force a resync.
8. **`project_snapshot` is the only replacement primitive.** Narrowing,
   `Resync` and resume all replace state through it; `revoke`, `reseed` and a
   grant generation are rejected because a mapping change with no op and no
   snapshot desynchronises the client.
9. **Turn enters by key and leaves by `NetId`.** The active actor may be
   entering the same page, so the host cannot know its id first. A hidden
   active actor renders `active: None`, matching the host and its pinned oracle.
10. **`ViewCursor` is view state, never wire.** `revision` orders projections
    within one tick and saturates; `tick` is the context tick. T024's
    `BeginTransfer.cursor` stays the host's last `event_seq`, and area clocks
    need not match.
11. **No limits parameter.** Caps are `MAX_VISIBLE_ENTITIES` and the derived
    `MAX_PAGE_OPS` (`3 * 1024 + 1`); a host-supplied limit would be a loosening
    path.
12. **Two one-page resend flags preserve T022 frames.** A command page resends
    unchanged `Health` and `Turn` for involved rows, so a change-only diff
    alone would break pinned host tests.
13. **Errors are checked in order before any mutation.** `WrongEpoch`,
    `TooManyFacts`, `DuplicateFact`, `IdExhausted`; an `Err` changes nothing.
    Ids run `1..=u64::MAX - 1`; exhaustion's remedy is a new `SessionEpoch` and
    a fresh view, never an id reuse.

## Authority and affected surface

- Owning crate: `crates/crpg-net`, module `interest`, reached at the crate root
  as `pub mod interest`.
- Contract: `tasks/T027b.md` §"Exact contract" (D1–D12, items 1–17).
- Host consumers arrive with T027c; nothing calls `InterestView` in production
  until then.

## Compatibility, hash and dependency impact

- No new dependency, `unsafe`, float, `HashMap`/`HashSet`, manifest or wire
  edit. Every output is an existing v1 `DeltaOp` or the existing T024 body;
  v1/v2/`0x81` bytes, fixtures, goldens and `state_hash` are untouched.
- `InterestView` is deliberately not `Serialize`: a saved view would restore
  stale authorization.

## Consequences

- T027c supplies `epoch`, `tick`, area, rows, despawns, turn, grants and resend
  flags, then composes `[Receipt][enters][history][rest]` and chunks frames.
- T046 may generate input with `InterestView` but must not use it as its
  oracle; the wire, not the view, is its subject.

## Agent log

- 2026-11-02 (UTC) · opencode-go/deepseek-v4.1-flash + T027b · Filed ADR-0034 Accepted from the approved contract's D1–D12, recording the interest projection surface, owner split, layout and retirement rule.
