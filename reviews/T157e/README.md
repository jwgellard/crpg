# T157e mutant run

Branch `task/T157e-i1` at `6f3b291`; driver `run_mutants.py`; fast = `cargo test -p crpg-godot --locked -j 2 --no-fail-fast --lib`, full = the same without `--lib`. Baselines (unmutated): fast 134 passed, full all binaries pass. Survivors of the fast run were re-run on the full command: all 9 survived again (`results_full.jsonl`).

**Result: 21 killed, 9 survived, 0 compile errors, 0 anchor errors (30 rows).**

| id | K | edit | status | killed by (first 2) |
|---|---|---|---|---|
| M01 | K-1 | WALK_RIGHTS.pause true -> false | killed | movement_readout_follows_the_pool, walk_open |
| M02 | K-1 | WALK_RIGHTS.unpause true -> false | killed | walk_open |
| M03 | K-1 | open_v4_walk_with binds PauseRights::default() instead of WALK_RIGHTS | killed | movement_readout_follows_the_pool, request_mode_starts_the_fight |
| M04 | K-1 | open_v4_walk_with own_pools true -> false | killed | movement_readout_follows_the_pool, walk_open |
| M05 | K-1 | grant: pools true -> false | killed | movement_readout_follows_the_pool, walk_open |
| M06 | K-1 | grant: effects false -> true | **SURVIVED** |  |
| M07 | K-1 | grants in descending EntityId order | **SURVIVED** |  |
| M08 | K-1 (other opens keep defaults) | open_v4_rt_with passes own_pools true | killed | walk_open |
| M09 | K-1 | open_v4_walk_with opens turn-based instead of real time | killed | the_goblin_waits_while_you_walk, the_host_engages_when_you_walk_near |
| M10 | K-1 | open_v4_walk_with uses NpcDriver::Peer instead of ServerAi | killed | request_mode_starts_the_fight, reach_is_one_and_a_half_metres |
| M11 | K-2 | table.movement_pool never assigned | killed | movement_readout_follows_the_pool, walk_open |
| M12 | K-2 | movement_pool is the action pool P instead of movement.turn.pool | killed | movement_readout_follows_the_pool, walk_open |
| M13 | K-2 | movement_pool is Some(P) when the ruleset declares no movement | **SURVIVED** |  |
| M14 | K-3 | request_mode: 0 and 1 swapped | killed | movement_readout_follows_the_pool, request_mode_starts_the_fight |
| M15 | K-3 | request_mode: other modes accepted as turn-based | killed | request_mode_precedence |
| M16 | K-3 | request_mode: last controlled actor instead of first | **SURVIVED** |  |
| M17 | K-3 | request_mode: 'no controlled actor' fault code NoSuchOption -> BadArgument | **SURVIVED** |  |
| M18 | K-4 | ClientV4::request_mode: outstanding-intent refusal removed | killed | request_mode_precedence |
| M19 | K-4 | ClientV4::request_mode sends EndTurn instead of RequestMode | killed | request_mode_starts_the_fight, reach_is_one_and_a_half_metres |
| M20 | K-5 | rebuild: pool match `fact.pool == wanted` -> `!=` | killed | movement_readout_follows_the_pool, walk_open |
| M21 | K-5 | rebuild: first disclosed pool taken regardless of id | killed | movement_readout_follows_the_pool, walk_open |
| M22 | K-5 | rebuild: movement pushes fact.max instead of fact.current | killed | movement_readout_follows_the_pool |
| M23 | K-5 | rebuild: movement_max pushes fact.current instead of fact.max | killed | movement_readout_follows_the_pool |
| M24 | K-5 | rebuild: withheld/absent rows push 0 instead of -1 | killed | movement_readout_follows_the_pool, walk_open |
| M25 | K-5 / invariant 6 | rebuild: movement dropped from the unchanged check | **SURVIVED** |  |
| M26 | K-5 | v2 client: movement column filled with 0 instead of -1 | **SURVIVED** |  |
| M27 | K-6 | data: goblin faction dropped (null) | killed | walk_open |
| M28 | K-6 | data: strike range 98304 -> 65536 | **SURVIVED** |  |
| M29 | K-6 | data: encounter engage distance 262144 -> 131072 | killed | the_host_engages_when_you_walk_near |
| M30 | K-6 | data: hero health 1310720 -> 655360 | **SURVIVED** |  |

## Survivors

- **M06** (K-1) - survived: real gap (host-side grant flags not asserted). Edit in `crates/crpg-godot/src/core/mod.rs`: `effects: false,                         stats: false,` -> `effects: true,                         stats: false,`
- **M07** (K-1) - survived: real gap / possibly equivalent (host may canonicalise; no test reads the grant order). Edit in `crates/crpg-godot/src/core/mod.rs`: `owned.sort_unstable();` -> `owned.sort_unstable_by(|a, b| b.cmp(a));`
- **M13** (K-2) - survived: equivalent (unreachable: only the walk open grants pools, and a no-movement ruleset cannot open a walk). Edit in `crates/crpg-godot/src/core/mod.rs`: `.map(|movement| movement.turn.pool);` -> `.map(|movement| movement.turn.pool)             .or_else(|| Some(Ulid::from_str("0000000000000000000000000P").unwrap()));`
- **M16** (K-3) - survived: real gap (both heroes are controlled, the host ignores the header actor; nothing captures the frame). Edit in `crates/crpg-godot/src/core/mod.rs`: `.controlled()             .first()` -> `.controlled()             .last()`
- **M17** (K-3) - survived: equivalent (defensive arm, no v4 session has an empty controlled list). Edit in `crates/crpg-godot/src/core/mod.rs`: `code: BridgeCode::NoSuchOption,                 detail: "no controlled actor".to_string(),` -> `code: BridgeCode::BadArgument,                 detail: "no controlled actor".to_string(),`
- **M25** (K-5 / invariant 6) - survived: equivalent in practice (movement changes only with an event/receipt, so `events.is_empty()` already fails). Edit in `crates/crpg-godot/src/core/client_v4.rs`: `&& movement == current.movement()             && movement_max == current.movement_max();` -> `&& movement_max == current.movement_max();`
- **M26** (K-5) - survived: real gap (no test reads movement() of a v2 session). Edit in `crates/crpg-godot/src/core/client.rs`: `movement: vec![-1; rows],` -> `movement: vec![0; rows],`
- **M28** (K-6) - survived: real gap (K10 asserts a strike, no distance; 1 m range still strikes). Edit in `campaigns/demo/adventure/abilities/strike.json`: `"range": 98304,` -> `"range": 65536,`
- **M30** (K-6) - survived: real gap (no test pins the hero's health of 20). Edit in `campaigns/demo/adventure/creatures/hero.json`: `"health": 1310720,` -> `"health": 655360,`

## Contract items no mutant or test reaches
- K-5 v2 client fills both columns with -1: no test reads `movement()`/`movement_max()` of a v2 session (M26). `movement_max` v2 fill was not mutated; it is equally unread.
- K-3 'fenced -> the fence code' and K-4 'link full -> 30, sequence unmoved' for `request_mode`: no test in `walk.rs` fences or fills the link around `request_mode` (no mutant row either; `send_intent` is old code).
- K-3 'first of controlled()': no test distinguishes hero 1 from hero 2 as the header actor (M16). K-3 'no controlled actor' (6): unreachable through the bridge (M17).
- K-1 'the other three flags false' and 'ascending order': no test inspects the host's fact grants (M06, M07).
- K-2 'None without movement' and 'v2 keeps None': unobservable at the bridge (M13); the K1 `open_v4_ai_with(&basic())` control reads -1 because pools are withheld, not because the pool is None.
- K-6 values `range 98304` and hero `health 20`: no assertion fixes them (M28, M30); K10 is named 'one and a half metres' but asserts only a strike and its order. The contract's K10 text has the same gap.
- K-5 invariant-6 clause for `movement` in the unchanged check (M25) is not separable from the event check by any scenario in the tests.

## Test assertions against the contract
Every assertion read in `walk.rs` (K1 to K10) carries the contract's values (rows, sides, rights, `[6, 6, -1]`, texts `bad mode`, `not a v4 session`, `an intent is outstanding`, codes 17/18/22, timing bounds). No assertion contradicts the contract. The 21 kills are by the intended tests (K1, K4, K7, K9 for the bridge and column rows, K3/K5 for the mode and driver, K9/K1 for data).

## Verdict
fix: add tests for the real gaps (v2 `movement()` = -1, `range`/`health` data pins, optionally the first-actor header and the grant flags), then re-run M06, M07, M16, M26, M28, M30. No source or data defect found; M13, M17 and M25 are equivalent.

## Fix round A

Four survivors with a real gap are now killed by added assertions only (no
source or data change):

| id | K | new outcome | killed by |
|---|---|---|---|
| M06 | K-1 | killed | `core::client_v4::tests::walk_open_grants_pools_and_nothing_else`: the player's disclosed replica rows carry `pools` and no `effects`, `stats` or `tags` (`walk.rs` unchanged for this one; the grant flags are only readable through the replica). |
| M07 | K-1 | **survived, equivalent** | The host canonicalises the grant order: `bind_v4_peer` calls `normalize_fact_grants` (`crates/crpg-server/src/host.rs:1391`), which does `facts.entities.sort_by_key(|grant| grant.entity);` (`host.rs:5971`) before the session stores or reads them. No seam returns the pre-sort list, so ascending and descending input are the same `FactGrants` to every reader. |
| M16 | K-3 | **survived, unobservable** | The actor is chosen at `crates/crpg-godot/src/core/mod.rs:358–362`; `ClientV4::request_mode` passes it to `send_intent` (`crates/crpg-godot/src/core/client_v4.rs:260–269`, `:294–317`), whose only effect is `link.send` of the encoded lane-0 frame (`:312`) — no recorded intent. `LocalClient`'s upstream queue is a private field (`crates/crpg-server/src/run.rs:819–826`). The host authorises `RequestMode` by the peer's `pause` right, not by the header actor (`crates/crpg-server/src/host.rs:6186`), and both heroes are controlled by the same peer (`:6163–6167`), so hero 1 and hero 2 produce the same receipt, event and state. |
| M26 | K-5 | killed | `core::authority::v4::walk_tests::v2_and_default_views_withhold_movement`: a v2 session's `movement()` and `movement_max()` are `-1` for every row; the default view's are empty. |
| M28 | K-6 | killed | `core::authority::v4::walk_tests::demo_data_pins_the_strike_range`: the loaded ability whose slug is `strike` has `range == Some(98304)` (1.5 m). |
| M30 | K-6 | killed | `core::authority::v4::walk_tests::walk_open`: the view's `health()` and `max_health()` are `[20, 20, 6]` (heroes 1310720 raw, goblin 393216 raw) and `health_known()` is `[1, 1, 1]`. |

The added tests are `walk_open_grants_pools_and_nothing_else` in
`crates/crpg-godot/src/core/client_v4/tests.rs` and
`v2_and_default_views_withhold_movement` plus `demo_data_pins_the_strike_range`
in `crates/crpg-godot/src/core/authority/v4/tests/walk.rs`; `walk_open` gained
the health assertions. No existing assertion was weakened or removed.

## Agent log
- 2026-11-28 (UTC) · claude-code/Sonnet 5.5 + T157e mutant run · Ran 30 single-point mutants over the T157e bridge, client columns and demo data; recorded 21 kills and 9 survivors with their K items.
- 2026-11-28 (UTC) · opencode-go/deepseek-v4.1-flash + T157e fix A · Killed M06, M26, M28 and M30 with tests only and recorded M07 (host sorts the grants) and M16 (no intent/frame seam and the host ignores the mode request's actor) as equivalent, so the fix round repairs every survivor that a test can observe.
