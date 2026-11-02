# ADR-0035: Client replica storage and application

Date: 2026-11-02 (UTC)
Status: **Accepted** — filed by T046. Authoritative contract:
`tasks/T046.md` §"Exact contract".

## Context

The Godot bridge (ADR-0033) needs an engine-neutral client copy of one filtered
v2 session: something must hold what the wire disclosed, in order, and refuse
frames that break the T027b rules N1–N9. The conformance `Replica` is a test
oracle, not a shipped type. The bridge decodes frames; this module only applies
whole decoded values, so codec faults stay code 52 and replica faults code 53.

## Decision

1. **One module `replica`, one type `ReplicaWorld`.** The spec's "no mutating
   methods except `apply_delta`" means the apply family: `apply_delta` and
   `apply_snapshot` are the only `&mut self` methods. The test double is not
   reused or replaced.
2. **Storage is one `Vec<ReplicaEntity>`, strictly ascending `NetId`.** T047
   makes one bulk ascending pass, so a slice is the natural query. Because a
   delta enter is always above every stored id, it is a push; lookup and
   removal are a binary search. No `HashMap`/`HashSet`.
3. **A row is presence plus `Option<HealthFact>`.** `None` is unknown, never
   zero. `dead` lives only inside `HealthFact`: `Died` does not set it because
   death travels with the health grant and notices are not state. The turn is
   one `Option<TurnFact>`; `active: None` means no disclosed actor.
4. **Inputs are whole values.** `apply_delta` takes `&protocol_v2::DeltaFrame`;
   `apply_snapshot` takes `&reassembly::Publication`. A caller cannot pair one
   page's ops with another page's sequence, or one body with another
   transfer's cursor.
5. **Strict sequence law, no in-band reset.** A frame is accepted only at the
   next expected `event_seq`. A restart at 1 after progress is an error: the
   host answers a narrowing reset with `RebindRequired`, the adapter fences the
   connection, and the caller builds a new replica for the new session.
6. **Reuse is caught by a high-water mark.** `high_water` is the largest
   `NetId` ever stored. A delta enter at or below it is a reuse; a snapshot row
   at or below it that is not live just before the snapshot is a reuse. This
   enforces N3 with no retired-id table, relying on rising, once-only id
   minting (T027b item 4, K1).
7. **Every op is validated, notices and events included.** Each named `NetId`
   must be live at that point, so N1 is enforced once here. `Receipt.epoch` is
   the only epoch a delta carries and must equal the replica's.
8. **Live rows may reach `2 * MAX_VISIBLE_ENTITIES` mid-page.** A T027b page
   puts all enters before all removals and may split at any op boundary, so a
   1,024-entity swap briefly holds 2,048 rows. This is a client bound, not wire.
9. **Atomicity: validate and apply on a working copy, then commit.** `Err`
   leaves the replica equal to a pre-call clone; `Ok` commits rows, turn,
   sequence, tick and high-water together.
10. **One closed fatal error enum, small payloads.** Payloads name the
    offending id or sequence for `last_error`. The bridge maps any `Err` to
    fatal code 53 and never skips; the replica itself stays usable after `Err`
    (fencing is bridge policy).
11. **v2 only.** T047 binds a V2 session; N9's "V1 receives no notices" is
    vacuous, and a legacy client is a wrapping adapter in its own task.
12. **No allocation in queries, no events kept.** The bridge collects events
    and receipts itself, owns `ended()`, and diffs `net_ids` between revisions.
    `apply_delta` returns `()`.

## Authority and affected surface

- Owning crate: `crates/crpg-net`, module `replica`, reached as
  `pub mod replica`. New public items: `MAX_REPLICA_ENTITIES`,
  `ReplicaEntity`, `ReplicaError`, `ReplicaWorld` and its queries.
- Contract: `tasks/T046.md` §"Exact contract" (items 1–15, R1–R19).
- Host callers arrive with the bridge and the first narrowing session; the
  slice has none.

## Compatibility, hash and dependency impact

- No new dependency, `unsafe`, float, `HashMap`/`HashSet`, manifest or wire
  edit; no tag, field, constant, fixture or golden changes. Every input is an
  existing T021/T024 value; the module encodes, decodes and mints nothing.
- `ReplicaWorld` is deliberately not `Serialize`: a saved replica would restore
  stale authorization. There is no `Default` and no restore constructor.

## Consequences

- T047 calls `apply_delta` with every decoded lane-0 frame and treats `Err` as
  fatal 53, builds arrays from `entities()`, and reads `server_tick()` for
  `observed_tick`. `turn().active` may name a non-live id; a missing proxy is
  no highlighted actor, not an error.

## Agent log

- 2026-11-02 (UTC) · opencode-go/deepseek-v4.1-flash + T046 · Filed ADR-0035 Accepted from the approved contract's D1–D12, recording the replica's storage, strict sequence and high-water rules, mid-page bound and atomicity.
