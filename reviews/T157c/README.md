# T157c mutant run

Branch `task/T157c-i1` at `a43214b`. Contract: `tasks/T157.md` Parts (T157c), Behaviour
S-C1..S-C4, Tests SC1-SC7. Driver: `run_mutants.py` (exact-text edit, one occurrence required,
originals restored in `finally`); rows in `results.jsonl` (fast command), survivors re-run on the
whole crate in `results_full.jsonl`; log in `run.log`. The session-102 reviewer ran all 30 rows
and the full re-run but ended before writing this file; the manager classified the survivors
from the recorded edits and the code (session 103).

## Result

30 mutants: 25 killed, 5 survived the fast and the full command (M07, M08, M12, M21, M30).
Per-row kills are in `results.jsonl`.

## Survivors

- **M07 (S-C1 place, real gap).** `InvalidEngage` moved before the S4 `NoEntrants` check.
  S-C1 says the engage check runs "after every existing start check". No test starts an
  encounter that fails both checks. Fix: a start with no entrants and a 0-distance rule
  refuses `NoEntrants`.
- **M08 (S-C1 place, real gap).** `InvalidEngage` moved after the T137 initiative rolls, so a
  refused start has already drawn from `world.rng_mut()`: "any failure leaves `world`
  unchanged" breaks. SC3 checks the bytes, but its fixture has no initiative rule, so nothing
  is drawn. Fix: SC3's refusal with an initiative rule set leaves the world byte-identical.
- **M12 (S-C1a, equivalent).** The `engage` term dropped from `serialize_struct`'s `len`. The
  length is a hint that `serde_json` ignores, and every crate path (`hash.rs`, `history.rs`)
  serializes through `serde_json`. Accepted, as for earlier `len` mutants.
- **M21 (S-C2, equivalent for the one rule kind).** The `fight_is_live` conjunct dropped from
  `engagement_due`. `Proximity` holds only for a pair of living, opposed combatants, and two
  such combatants are two live sides (faction-less combatants are each their own side, T157a
  M18), so `fight_is_live` is already true whenever a rule holds. Accepted. T158 adds trigger
  kinds that need no opposed pair; its contract must keep the conjunct and test it.
- **M30 (S-C3 order, real gap).** The `TurnBased` refusal moved before the `DeadCombatant`
  check. S-C3: "every other check and its order are unchanged". `a6_set_route_order_and_dead`
  pinned "DeadCombatant wins over TurnBased", but after T157c its fight is finished (no head),
  so `TurnBased` no longer fires there and the assertion stopped testing the order. Fix: a
  dead combatant in a live turn-based encounter that has a head (a three-combatant fight in
  which one dies and two fight on) refuses `DeadCombatant`.

## Agent log

- 2026-11-28 (UTC) · claude-code/opus-5.5 (manager) + T157c mutant run · Wrote the run's report from its recorded rows because the session-102 reviewer ended before writing it, classifying M12 and M21 as equivalent and M07, M08 and M30 as real gaps for fix round A.
