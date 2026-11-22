# T049a mutant run

Branch `task/T049a-i1` at `f0c5335`. Contract `tasks/T049.md`. Driver
`run_mutants.py` (exact-text anchors, CRLF-aware, bytes restored in a
`finally`), results `results.jsonl`, raw output `run.log`.
Command per mutant: `cargo test -p crpg-godot --locked -j 2 --lib --test
source_rules_smooth` with `CARGO_INCREMENTAL=0`. Baseline (unmutated): 111 lib
tests + 1 SR1 test, all pass. No compile errors, no timeouts.

**Result: 32 mutants, 28 killed, 4 survived, all 4 equivalent.**

| id | item | edit | result (killed by) |
|---|---|---|---|
| S01 | B-S1 | offset `min` -> `max` | killed (C1, S5) |
| S02 | B-S1 | offset drops `x TICK_MS` | killed (S2, S3, S5, C1, C3 ...) |
| S03 | B-S1 | later sample overwrites offset | killed (S5) |
| S04 | B-S1/S4 | `observe_clock` call removed | killed (C1, C3, E1) |
| S05 | B-S1/S4 | `observe_clock(.., 0)` | killed (C1, C3) |
| S06 | B-S2 | append `<` -> `<=` | killed (S6) |
| S07 | B-S2 | bound `>` -> `>=` | killed (S6) |
| S08 | B-S2 | bound pops back | killed (S6, E1, rt_move_to_walks) |
| S09 | B-S2 | same-tick replace is a no-op | killed (S6) |
| S10 | B-S2 | restart `clear()` removed | killed (S6) |
| S11 | B-S2 | `retain` (revocation) removed | killed (S6, C2) |
| S12 | B-S2 | `moving` always false | killed (S4, S7) |
| S13 | B-S4 | `observe` removed from loop | killed (C1, C2, C3, E1) |
| S14 | B-S4 | `observe` moved after loop | killed (C1, C3) |
| S15 | B-S3(2) | hold `>=` -> `>` | SURVIVED, equivalent |
| S16 | B-S3(2) | hold `> newest_at + 1000` | SURVIVED, equivalent |
| S17 | B-S3(3) | oldest `<=` -> `<` | SURVIVED, equivalent |
| S18 | B-S3(3) | before-oldest returns newest | killed (S2, S3, E1) |
| S19 | B-S3 | delay `-` -> `+` | killed (S2, S3, S4, C1, E1) |
| S20 | B-S3 | offset ignored | killed (S2, S3, S4, C1) |
| S21 | B-S3(1) | no clock returns `[0;3]` | killed (S1, E1, rt_open) |
| S22 | B-S3(4) | pair `a_at <=` -> `<` | killed (E1 only) |
| S23 | B-S3(4) | rest start `max` -> `min` | killed (S4) |
| S24 | B-S3(4) | `a.moving` negated | killed (S4, S7) |
| S25 | B-S3(4) | `div_euclid` (floor) | killed (S3) |
| S26 | B-S3(4) | `span = c_at - a_at` | killed (S4) |
| S27 | B-S3 | `(tick * TICK_MS) as i128` | killed (S7) |
| S28 | B-S4 | unsmoothed fallback returns zeros | SURVIVED, equivalent |
| S29 | B-S4 | rows emitted reversed | killed (C1, E1, rt_open) |
| S30 | B-S5 | `last_poll_ms` not recorded | killed (E1, rt_move_to_walks) |
| S31 | B-S5 | `OpenV4` arm passes 0 | killed (E1, rt_move_to_walks) |
| S32 | B-S5 | `OpenV4` arm returns unsmoothed view | killed (E1) |

## Survivors (exact edits)
- **S15** `smooth.rs` `if render_at >= newest_at {` -> `> newest_at {`.
  Equivalent: at `R == newest_at` the oldest test and the pair search
  (`R < c_at` never true for the last pair) fall to the final fallback, which
  returns `newest.point`.
- **S16** the same line -> `if render_at > newest_at + 1000 {`. Confirms the
  implementer's judgement: equivalent. For any `R >= newest_at` the oldest
  test is false (`R > oldest_at`, or for a one-sample buffer `R > oldest_at`),
  the pair search finds no pair, and the same `newest.point` fallback returns.
  The hold rule is therefore only observable through the fallback; the
  fallback comment says so.
- **S17** `if render_at <= oldest_at {` -> `< oldest_at {`. Equivalent: at
  `R == oldest_at` the first pair returns `a.point` (moving: `R <= start`;
  rest: `start >= a_at`).
- **S28** `None => out.extend_from_slice(&positions[3*index..])` -> `&[0,0,0]`.
  Equivalent in practice: the fallback is reached only for `position_known ==
  0` rows, whose view positions are already `[0,0,0]` (`client_v4.rs:493`);
  a known row always has a buffer, because `observe` runs after every frame.
  The contract's "otherwise `positions()[3i..]`" branch is therefore
  unreachable and untestable.

## Contract vs tests
All asserted values match the contract: S1-S7 (including the 650/700/725/749/
750/900 table, floor-vs-truncate 132, the 8-sample bound, restart, revocation),
S8, C1-C3, E1 (`held` is the `h_k` series, `V` is every returned view's
`positions()[0]`), E2, SR1 with its control. No assertion differs.

## Items no mutant or test reaches
- **Middle-pair boundary `a_at <= R`** (S22) is killed by E1 alone. Contract
  S2-S4 use two samples, where `R == ms(a)` is caught by the oldest rule, so
  no unit test pins the boundary between two interior samples.
- **E2's fenced arm** has no mutant in this table (e.g. a `Fenced` arm that
  returns an empty Vec was not run); the test asserts the contract value.
- **The constants** (`TICK_MS`, `RENDER_DELAY_MS`, `SAMPLES_PER_ENTITY` values)
  were not mutated directly; S8 asserts them, and S02/S19 reach the same
  arithmetic.
- **SR1** guards source tokens, not behaviour; no mutant applies.

## Verdict
`ready-for-pr`: no non-equivalent survivor. The one weak spot (S22, interior
pair boundary only in E1) is a test-strength note, not a contract violation.

## Agent log
- 2026-11-22 (UTC) · claude-code/sonnet-5.5 + T049a mutant run · Designed and ran 32 mutants over the new and changed non-test source lines; 28 killed, 4 equivalent survivors, source untouched.
