# T108 draft check: draft-longcat.md

Claims about the engine in `draft-longcat.md`, checked against the code by
`opencode-go/beam-free` (`oc.py ask --role review`, brief `T108-check`,
2026-10-16). Only the claims that do not hold are listed. For the architect:
correct these before relying on the draft.

`CLAIMS: 95 checked · 91 holds · 3 wrong-place · 1 false · 0 unsupported`

| Draft line | Claim | Status | Evidence |
|---|---|---|---|
| 34 | Kernel exposes seven public modules (`lib.rs:13-20`) | wrong-place | `crates/crpg-rules/src/lib.rs:13-20` lists 8 modules including `error`; the seven are lines 13–14 and 16–20 |
| 93 | srd-lite has six attributes and health (`ruleset.json:33-53`) | **false** | `rulesets/srd-lite/ruleset.json:33-53` lists 5 stats: might, guile, resolve, ward, health |
| 102 | Sim `CombatDefinition` damage is `Vec<(Outcome,u32)>` per `AGENTS.md` | wrong-place | `docs/adr/0013-combat-simulation-state.md:48` says this; `crates/crpg-sim/AGENTS.md` has no such line |
| 155 | Multi-action turns via `ends_turn: false` (`focus.json:25`) | wrong-place | `rulesets/srd-lite/focus.json:24` |

## Agent log

- 2026-10-16 (UTC) · claude-code/opus-5.5 + T108 draft check · Filed the Beam check of the LongCat draft (95 claims, 4 not holding) so the architect pass starts from verified facts.
