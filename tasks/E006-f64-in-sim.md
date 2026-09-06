## Task
Decide whether `f64` is permitted in `crpg-sim`, and align the determinism lint
scope with that decision. Human-decision task (ADR/AGENTS.md).

> **Decided 2026-09-06 as Option A** (tighten lint: `no-f64` scoped to sim,
> `f32`-spatial-only). Implemented; see Agent log.

## Why this is deferred

Root `AGENTS.md:19-21` allows `f32` in `crpg-sim` for spatial positions only. But the
determinism lint (`tools/lint/determinism.py`) exempts `crpg-sim` from ALL float
bans (`no-float-crates` only covers core + rules). That means `f64` is currently
legal in sim despite the AGENTS.md stance. (Citation correction 2026-09-05: this
task previously cited `crates/crpg-sim/AGENTS.md`, which does not exist — only
`crates/crpg-core/AGENTS.md` does; the enforced rule lives in root `AGENTS.md`.) This is a gap, but closing it two ways
is possible and the choice affects gameplay code:

- **Option A — tighten lint**: add a `no-f64-crates` scope covering core + rules
  + sim, so `f64` is flagged in sim but `f32` is allowed. Matches the strict
  reading of the sim AGENTS.md.
- **Option B — relax doc**: keep `f64` legal in sim (for e.g. economy/stat
  overflow math) and amend the sim AGENTS.md wording to say spatial position is
  `f32`, non-positional simulation math may use `f64`.

## Deliverable
- A decision recorded in an ADR or the sim AGENTS.md.
- If Option A: `tools/lint/determinism.py` gains a `no-f64` rule scoped to sim
  (core/rules already ban all floats) plus self-tests.
- If Option B: only a doc amendment.

## Constraints
- Determines a lint-policy change → human sign-off required.
- Do not touch crpg-sim sources in this task; it is not yet implemented.

## Agent log

- 2026-09-05 (UTC) · opencode/muse-spark + citation fix · Corrected the evidence citation from non-existent crates/crpg-sim/AGENTS.md to root AGENTS.md:19-21; the f64-in-sim policy gap itself is unchanged and still awaits decision.
- 2026-09-06 (UTC) · opencode/muse-spark + E006-A decided with maintainer · Option A: `f64` banned in `crpg-sim`; `tools/lint/determinism.py` enforces `no-f64` for sim (unsuffixed literals stay allowed — they infer to `f32`), self-tests pin all three float scopes, root `AGENTS.md` updated.