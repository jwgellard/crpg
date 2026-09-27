# ADR-0014: explicit encounter release in `crpg-sim`

Date: 2026-09-27
Status: **Accepted**

## Context

T016 delivers the first end-to-end proof that authored ruleset data drives
combat, but its contract is one fight: T016b publishes an encounter and
plays it to terminal (`active = None`), and nothing releases it. The only
way out is despawning every combatant (T016e), which discards participants
without a retained outcome, and despawning the active combatant strands the
turn (`active = None` with scheduled alive combatants) with no specified
recovery. The remediation plan (work package A2) requires explicit
completion/reentry semantics: terminal retention for authoritative
consumers, release/cleanup, subsequent encounter initialization, and
identity reuse — without equating death with despawn or clearing unrelated
world state. The crate contract requires an ADR for the new public
operation; this file is the reviewed record, approved before implementation
(approval recorded in `tasks/T016f.md`).

## Decision

1. **New release operation.** `end_encounter(world) -> Result<
   EncounterSummary, CombatError>` releases the active encounter and
   returns its retained outcome. `combat = None` fails before any mutation
   as the existing `NoEncounter`; otherwise the operation is infallible
   (summary built first, then state stripped).
2. **Release retains results without retaining scheduling.** The summary
   carries the encounter/ruleset ULIDs, the closing round, and one
   `ParticipantResult { placement, entity, health, dead }` per combatant in
   authored participant order. Release removes every combatant timeline
   entry, clears the combatant store, and sets `combat = None`. Entities
   stay live: no despawn, no `Despawned` events, and no `Died` from release
   itself (deaths were emitted exactly once at their kills). Transforms,
   events, RNG, tick, interners, and outsider timeline entries are
   untouched.
3. **Release is allowed any time the encounter is active.** Aborting an
   in-progress fight is caller intent; the summary reflects the moment of
   release. No terminal-only gate and no new error variant.
4. **Despawn of the active combatant advances or terminates, never
   strands.** When `despawn` removes the holder of `combat.active` with
   combatants remaining, one shared crate-private helper runs the same rule
   as the `perform_action` tail: timeline nonempty with two or more alive
   sets the new head with one `TurnStart` refresh; timeline empty with two
   or more alive rolls the round over; otherwise terminal (`active =
   None`). Despawn of a non-active combatant leaves the turn untouched;
   despawn of the last combatant releases per T016e.
5. **Second encounters reuse content, never runtime identity.** After
   release, `start_encounter` runs unchanged on a rebuilt spec; new
   entities mint from the continuing arena with fresh ids. A second fight
   is a deterministic continuation, not a byte-repeat of the first.
6. **Load compatibility.** Released worlds are ordinary non-combat worlds
   under the T016e checks; `active = None` stays loadable for older saves.

## Consequences

- `World`'s serialized shape is unchanged; no migration, no re-baseline.
  The sim gains one public function and two public types; `CombatError`
  gains no variant and no existing signature changes.
- Turn logic now lives in `start_encounter`, `perform_action`, and the
  shared helper called from `World::despawn` — the helper owns the rule,
  `despawn` owns only the call. `tick` / `run_systems` / `end_turn` stay
  combat-free.
- T016c replays and goldens are unaffected (valid-input combat behavior is
  unchanged); lifecycle tests live beside them in `crpg-sim`.

## Agent log

- 2026-09-27 (UTC) · opencode/muse-spark + T016f approvals · Filed as the reviewed ADR for the encounter release operation, retained summary, despawn-active advance rule, and content-reusing reentry, after maintainer approval of the A2 API and this decision text.

(End of file - total 94 lines)
