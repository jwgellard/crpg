# ADR-0022: Host sessions, disclosure revocation, and capture retirement

Date: 2026-09-29 (UTC)
Status: Selected under recorded D02/D03/D10 delegation for specification;
source, dependency approval, independent review, and native acceptance remain
T022 work. No independent human review is claimed. This ADR supersedes
ADR-0021's incomplete contract; where the two disagree, ADR-0022 and the
normative [T022 appendix](../../tasks/T022.md#specification-revision--2026-09-29-review-corrections)
govern.

## Context

ADR-0021 pinned the host/capture/checkpoint shape against the T020/T021
public APIs, but review found eight load-bearing gaps: peer-wide disclosure
flags could not express per-field permissions, narrowed grants left queued
secrets deliverable, the never-retired capture journal halted the host,
terminal rejections bypassed reservation, one host-wide epoch made
correlation ambiguous across peers, retry storage could not reproduce
receipt ticks, capture post-state and observation had no API, and V1
projection of richer history was undefined. Implementation under ADR-0021
would have invented host policy during coding.

## Decision

Adopt the corrected T022 appendix as the normative exact
API/error/cap/checkpoint/acceptance contract:

1. **Per-entity disclosure, not peer-wide categories.** `EntityDisclosure`
   plus disclosed ability/encounter vectors express presence, health,
   lifecycle, per-end action fields, and turn facts per whole `EntityId`.
   Unknown entities and fields are denied; control (who may act) stays
   separate from visibility (who may learn).
2. **Revocation resynchronizes delivery.** Narrowing grants returns
   `RebindRequired`, discards unacknowledged frames for that peer, resets
   delivery to a fresh filtered state, and relies on explicit adapter
   fencing for already-taken bytes. Already published facts are not
   retractable; widening preserves continuity.
3. **Sessions own epochs.** Each binding's epoch derives from the host
   incarnation and its never-reused handle. Correlation is therefore
   globally unique, sequence conflicts fence only the offending session,
   and receipts go to the originator while events/state fan out.
4. **Captures retire through a trusted consumer.** Monotonic `capture_seq`,
   bounded `read_captures`, and `acknowledge_captures` bound evidence
   without halting the host; the watermark persists across checkpoints.
   Records retain the exact history range plus involved-entity views, so the
   first post-state survives later mutations.
5. **Terminal rejections reserve before committing.** Freshness,
   authorization, and gameplay rejections determine their reason, reserve
   cache plus receipt-delivery room, then commit sequence/cache/delivery.
   Backpressure is retryable head-of-line blocking in every path.
6. **Retries reproduce the original receipt.** The cache retains status plus
   the original processed tick; `take_delivery` is idempotent until
   acknowledged and re-issues the identical receipt after acknowledgment.
7. **V1 and V2 projections differ explicitly.** V1 delivers receipts plus
   legacy state only; V2 adds ordered event ops. Acceptance oracles are
   version-specific; unknown delta tags are proved by decoder tests, not by
   ingesting delta bytes as commands.

## Consequences

No new dependency is approved by this ADR; the implementation still needs
human-approved normal edges on `crpg-core`/`crpg-sim`/`crpg-net` plus
workspace `serde`/`serde_json`, each with a T018a-style audit record, and a
`[lib]` target beside the thin dedicated binary. Checkpoint honesty is
enforced by a max-shape measurement test against the 16 MiB cap (now
including the 16 KiB capture-record bound for retained events and views).
True `u64::MAX` counter exhaustion stays unreachable-by-counting with pinned
dispatch order and explicit residuals. `docs/architecture/crpg-server.md`
and `crates/crpg-server/AGENTS.md` remain due with the first source task.
The `crpg-persist` backend, authentication/entropy/TLS evidence (T030), and
production interest facts (T027a/c supply values for these shapes, not type
changes without a new ADR) remain prerequisites for product use, not for
this slice's in-memory contract.

## Agent log

- 2026-09-29 (UTC) · opencode/muse-spark + T022 review corrections · Filed the superseding session/disclosure/retirement rationale, keeping the normative contract in tasks/T022.md and naming the persist/auth/exhaustion residuals as open gates rather than silent choices.
