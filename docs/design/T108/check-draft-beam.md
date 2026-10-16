# T108 draft check: draft-beam.md

Claims about the engine in `draft-beam.md`, checked against the code by
`opencode-go/beam-free` (`oc.py ask --role review`, brief `T108-check`,
2026-10-16). The checker is the draft's own model: LongCat, the other-lab
checker, failed and is out of routing. Only the claims that do not hold are
listed. The two `invariant` rows matter most: the draft says two
recommendations stay clear of the S-set when they do not.

`CLAIMS: 48 checked · 42 holds · 4 wrong-place · 2 invariant`

| Draft line | Claim | Status | Evidence |
|---|---|---|---|
| 246 | A per-encounter mode on `CombatState` does not change `World` | **invariant** | `docs/IMPLEMENTATION_PLAN.md:93` puts the `World` struct in the S-set, and `CombatState` is `World`-owned (`crates/crpg-sim/AGENTS.md`) |
| 247 | Pause stays out of `HistoryWorld`, so no history-schema or persistence change | **invariant** | The draft itself adds `TimelineMode`, the round holder and initiative order to the snapshot (`docs/architecture/crpg-sim.md:253-256`) |
| 20 | Public surface re-exported at crate root (`lib.rs:13-20`) | wrong-place | `crates/crpg-rules/src/lib.rs:13-20` are `pub mod` declarations; re-exports are at 22–39 |
| 172 | Advance policy lives in tick/`end_turn` (arch sim:67) | wrong-place | `docs/architecture/crpg-sim.md:57-58` says advance policy is T008a; 66–67 is the tick module |
| 279 | T051 waits on T108 and T104 at plan:243 | wrong-place | `docs/IMPLEMENTATION_PLAN.md:244` is T051; 243 is T050 |
| 94 | Literal-exclusion gate in `AGENTS.md`'s limits section | wrong-place | `crates/crpg-rules/AGENTS.md`: under Determinism traps; Limits is lines 98–108 |

## Agent log

- 2026-10-16 (UTC) · claude-code/opus-5.5 + T108 draft check · Filed the Beam check of the Beam draft (48 claims; two S-set understatements) so the architect pass starts from verified facts.
