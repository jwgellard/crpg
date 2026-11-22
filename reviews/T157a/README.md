# T157a mutant run

Branch `task/T157a-i1` at `9e0c9d7`. Contract: `tasks/T157.md` Parts (T157a), Interface T157a,
Behaviour S-A1..S-A6, Tests SA1-SA11, Readings. Driver: `run_mutants.py` (exact-text edit,
one occurrence required, CRLF-aware, originals restored in `finally`); rows in `results.jsonl`
(fast command), survivors re-run on the whole crate in `results_full.jsonl`; log in `run.log`.

Baseline (unmutated), both pass: fast `cargo test -p crpg-sim --locked -j 2 --no-fail-fast --lib --test sides`
(48 lib + 13 sides) and the full `cargo test -p crpg-sim --locked -j 2 --no-fail-fast`.
`CARGO_INCREMENTAL=0`, timeout 900 s per mutant.

## Result

30 mutants: 28 killed, 2 survived (both also survive the full crate run).
M18 was a compile error on the first try (`Ulid::nil` does not exist) and was rewritten once
(`Ulid::from_u128(0)`); the table shows the rewrite.

| id | item | edit | result | killed by |
|---|---|---|---|---|
| M01 | S-A1 | Combatant.faction: skip_serializing_if dropped (null written when absent) | KILLED | sa1_no_faction_changes_nothing, sa6_faction_round_trips_and_hashes |
| M02 | S-A1 | CreatureState.faction: skip_serializing_if dropped (null written when absent) | KILLED | sa4_sides_of_combatants_and_records |
| M03 | S-A1 | Combatant.faction: never serialized (skip_serializing) | KILLED | sa6_faction_round_trips_and_hashes |
| M04 | S-A1 | CreatureState.faction: never serialized (skip_serializing) | KILLED | sa4_sides_of_combatants_and_records |
| M05 | S-A2 | prefab spawn: participant combatant takes no faction (Pending.faction = None) | KILLED | sa7_rollover_gates_count_sides, sa8_real_time_leave_counts_sides, sa4_sides_of_combatants_and_records (+8) |
| M06 | S-A2 | register_creature: record takes no faction | KILLED | sa9_join_refused_once_one_side_is_left, sa4_sides_of_combatants_and_records |
| M07 | S-A2 | start: Source::Enter record->combatant drops the faction | SURVIVED | - |
| M08 | S-A2 | start: member seat record->combatant drops the faction | KILLED | sa4_sides_of_combatants_and_records |
| M09 | S-A2 | join: joiner record->combatant drops the faction | KILLED | sa9_join_refused_once_one_side_is_left |
| M10 | S-A2 | exit_record: record takes no faction back (end and leave) | KILLED | sa4_sides_of_combatants_and_records |
| M11 | S-A3 | side_of: a combatant is always Alone (faction ignored) | KILLED | sa4_sides_of_combatants_and_records |
| M12 | S-A3 | side_of: a creature record is always Alone (record faction ignored) | KILLED | sa4_sides_of_combatants_and_records |
| M13 | S-A3 | opposed: `side_a != side_b` -> `==` | KILLED | sa4_sides_of_combatants_and_records |
| M14 | S-A3 | opposed: an entity with no side is opposed (`_ => false` -> `true`) | KILLED | sa4_sides_of_combatants_and_records |
| M15 | S-A4 | fight_is_live: `combat().is_some()` check removed | SURVIVED | - |
| M16 | S-A4 | fight_is_live: `>= 2` -> `>= 1` | KILLED | sa8_real_time_leave_counts_sides, sa7_rollover_gates_count_sides, sa2b_the_action_end_counts_sides (+4) |
| M17 | S-A4 | live_sides: dead filter removed (dead combatants count) | KILLED | sa10_real_time_validation_counts_sides, sa5_two_on_each_side_fight_on, sa2_one_side_left_ends_a_turn_based_action (+5) |
| M18 | S-A4 | live_sides: every faction-less combatant shares one side (zero faction) | KILLED | on_tick_deliveries_are_counted_exactly, round_and_turn_start_pools_refresh_exactly_once, e14_rt_and_boundary_leaves (+12) |
| M19 | S-A4 | live_sides: factions ignored (every combatant its own side = old living count) | KILLED | sa8_real_time_leave_counts_sides, sa7_rollover_gates_count_sides, sa2b_the_action_end_counts_sides (+6) |
| M20 | S-A5 join | S-A5 join_encounter J2 (RT terminal) counts the living | KILLED | sa9_join_refused_once_one_side_is_left |
| M21 | S-A5 admit-stop | S-A5 admit Stop (RT terminal gate) counts the living | KILLED | sa10_real_time_validation_counts_sides |
| M22 | S-A5 admit-use | S-A5 admit UseAbility (RT/Plan terminal gate) counts the living | KILLED | sa10_real_time_validation_counts_sides |
| M23 | S-A5 action-end | S-A5 perform_action action end counts the living | KILLED | sa2b_the_action_end_counts_sides |
| M24 | S-A5 advance-empty | S-A5 advance_tracked empty-timeline gate counts the living | KILLED | sa7_rollover_gates_count_sides |
| M25 | S-A5 advance-head | S-A5 advance_tracked head gate counts the living | KILLED | sa7_rollover_gates_count_sides |
| M26 | S-A5 rt_running | S-A5 rt_running counts the living | KILLED | sa3_one_side_left_ends_a_real_time_fight |
| M27 | S-A5 stage3-break | S-A5 stage-3 per-actor break counts the living | KILLED | sa11_stage_three_stops_when_one_side_is_left |
| M28 | S-A5 stage3-end | S-A5 stage-3 end report counts the living | KILLED | sa3_one_side_left_ends_a_real_time_fight |
| M29 | S-A5 leave-before | S-A5 remove_combatant_tracked 'before' count is the living count | KILLED | sa8b_a_finished_real_time_fight_ends_once |
| M30 | S-A5 leave-after | S-A5 remove_combatant_tracked 'after' count is the living count | KILLED | sa8_real_time_leave_counts_sides |

## Survivors

- **M07 (S-A2, real gap, not equivalent).** `combat.rs` `start_encounter_with_members_tracked`,
  `Source::Enter(entity)` arm: `let entered = enter_combatant(&definition, record, 0, member.initiative);`
  followed by `entered.faction = None;`. Survives `--lib --test sides` and the full crate. The
  `Source::Enter` arm is the *participant* whose placement already has a living creature record
  (non-respawning lifecycle); it is reachable. The contract's S-A2 lists "enter, member, joiner"
  as three copy points, but SA4(b), which is titled "Enter", starts R as a *member* (the seat
  arm, killed by M08), so no test in the contract or the suite drives the participant-enters-from-
  record arm. Fix is a test: register a record whose placement is a participant's placement with
  creature faction F1, `start_encounter_in_mode` it, assert the combatant's `faction() == Some(F1)`.
- **M15 (S-A4, equivalent).** `fight_is_live`: `world.combat().is_some() &&` removed. With no
  encounter `live_sides` is 0, because the loader refuses a combatant store without encounter
  state (`world.rs` "combatant store without encounter state") and `remove_combatant_tracked`
  clears `combat` when the store empties; no public path holds combatants with `combat() == None`.

## S-A5 sites, one mutant each (live_sides replaced by the old living count)

join `:2748` (M20, SA9), admit Stop `:3568` (M21, SA10), admit UseAbility `:3644` (M22, SA10),
action end `:3951` (M23, SA2b only), advance_tracked empty `:4303` (M24, SA7) and head `:4343`
(M25, SA7), rt_running `:4540` (M26, SA3), stage-3 break `:4612` (M27, SA11), stage-3 end `:4686`
(M28, SA3), `world.rs` leave "before" (M29) and "after" (M30). Every listed site is killed by
the test the contract assigns it. No extra site beyond the contract list exists (AGENTS.md and the
commit name none; a grep for living counts compared with 2 finds only the load-time coherence
check in `world.rs`, which the doc says stays a count and is not an end-of-fight rule).

Implementer's SA8b (known from the brief): M29 (`world.rs` "before" count replaced by the living
count) is killed by SA8b alone. The contract's SA8 does not reach it (SA8 kills M30). SA8b is
therefore load-bearing for that site; it is a contract-required behaviour (an RT fight already
down to one side must not journal a second `EncounterEnded` when someone leaves) that the
contract's SA8 omits.

## Assertion check against the contract

SA1-SA11 assert the contract's values where they were read (SA2/SA2b actives and `fight_is_live`,
SA4 sides/opposed/records/end copies, SA5 sequence, SA6 three `"faction"` occurrences, round trip,
hash differs). SA6's "no-faction `state_hash` equals the pre-change value" is not asserted
literally (the contract says SA1's bytes and the goldens are the evidence); SA1 asserts absence of
`"faction"` instead. Items that only one test reaches: M01..M04 (one or two tests each), M23 (SA2b),
M29 (SA8b). No contract item is reached by no test except the M07 arm above.

## Verdict

VERDICT: fix. One real gap (M07: participant enters from an existing record, S-A2 "enter"),
one equivalent (M15). 28/30 killed. Source needs no change; one test (above) is the fix.

## Fix round A

Branch `task/T157a-i1`, fix commit on top of `f953149`. Test only; no source file changed.

- Added `sa4c_participant_enters_from_record_keeps_faction` to `crates/crpg-sim/tests/sides.rs`: a record
  with faction F1 registered at participant C's placement (non-respawning lifecycle), then
  `start_encounter_in_mode`; asserts the combatant came from the record (`entered()`), its `faction()` is
  `Some(F1)` and its `side_of` is `Side::Faction(F1)`. Values come from S-A2.
- **M07 new outcome: KILLED** by `sa4c_participant_enters_from_record_keeps_faction` (re-run through
  `run_mutants.py M07`; the driver replaced M07's row in `results.jsonl`, the earlier `survived` outcome stays in
  `run.log` and git history). `git diff -- crates/crpg-sim/src` is empty after the run.
- Totals now: 29 killed, 1 survived (M15, equivalent, accepted). The table row for M07 above is the first-run result.

## Agent log
- 2026-11-22 (UTC) · claude-code/claude-sonnet-5-5 + T157a mutant run · Ran 30 single-edit mutants over the T157a source diff; 28 killed, M07 (participant-enters-from-record faction copy) survives, M15 is equivalent.
- 2026-11-22 (UTC) · claude-code/claude-sonnet-5-5 + T157a fix A · Added the participant-enters-from-record test and re-ran M07 through the driver, killed; the first-run table above is left as it was.
