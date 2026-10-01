# ADR-0020: Persisted single-area World identity and explicit entity transfer

Date: 2026-09-29 (UTC)
Status: Selected under recorded D14 delegation for specification; T027a
implementation/native verification remains open.

## Context

World owns one loaded area's simulation, but current public state does not retain
area identity. Encounter adaptation checks PlacementAndArea values then drops
them. Whole-area presence cannot be inferred from health, placement or NetId.

## Decision

Adopt [T027a's specification revision](../../tasks/T027a.md#specification-revision--2026-09-29)
as the normative exact API/error/serde/transition contract. New bound constructors
persist an immutable optional area ULID on World. Legacy unbound constructors
omit it and retain exact bytes. Presence is liveness in the bound World; death
does not remove presence. Bound encounter initialization verifies authored area.

Explicit two-World transfer stages both authorities and moves only the supported
noncombat, unscheduled EntityMeta/optional-Transform shape, allocating a new
destination identity. A HistoryWorld pair operation journals source removal and
destination spawn atomically. Unsupported combat migration fails; no partial
component-copy success. Cross-World runtime identity includes area plus full
EntityId because arena ids can collide between Worlds.

## Consequences

No net/account/viewer types or permissions enter sim. Host resolves authored
areas, owns viewer grants, retires actionable mappings and cancels stale
snapshots. No within-area AOI, LOS, automatic navigation or mid-combat transfer
is claimed. Future components must explicitly participate in transfer or make
it fail, not disappear. Area fields hash in full when present; legacy unbound
goldens are unchanged. No dependency/core/contract change.

## Agent log

- 2026-09-29 (UTC) · opencode/gpt-6-astra + T027a/D14 specification · Recorded persisted area membership and its narrow atomic producer without contradicting one-area-per-World ownership. Kept permissions, network handoff and unsupported combat migration out of the sim facts contract.

## Status update — 2026-09-30 (UTC)

**Accepted** — approved by the user on 2026-09-30 ("Approved.", decision D19). Implemented by T027a and merged in PR #20 with its suite green on Windows/MSVC and Linux/GNU CI. See [tasks/DECISIONS-2026-09-30.md](../../tasks/DECISIONS-2026-09-30.md).

- 2026-09-30 (UTC) · claude-code + D19 status update · Appended the user-approved status change as a dated section instead of editing the original status line, per the E007 append-only policy.
