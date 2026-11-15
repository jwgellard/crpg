# crpg-ai — agent contract

Read the root rules and [T056](../../tasks/T056.md) (the part you were given:
T056a for the decision, T056b for the behaviour suite), then spec §6. Reference
[ADR-0036](../../docs/adr/0036-movement-and-navigation.md) for every distance,
point and route the AI uses. Architecture:
[crpg-ai](../../docs/architecture/crpg-ai.md). This file holds only how-to-work
rules.

## Definition of done for any change

```
cargo fmt --all
cargo clippy -p crpg-ai --all-targets -- -D warnings
cargo test -p crpg-ai
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
```

## Rules and traps

- **Integer-only and deterministic by contract.** `tools/lint/determinism.py`
  does not scan this crate yet (the manager adds it separately), so the rule is
  the reviewer's: no `f32`/`f64` or float literal, no `HashMap`/`HashSet`, no
  clock, thread or RNG, no interior mutability. All arithmetic stays in `u64`
  or `i128` through sim's U4/U5 functions (`seg_len`, `point_at`,
  `route_measure`), never a local distance formula.
- **`legal_actions` is the only legality source.** Every `Decision::Act` is an
  element of its list; a move is checked with the host's own `host_route` and
  with `validate_move`/`validate_route`. Never re-derive affordability or range
  from the definition, and never dry-run an action on a clone.
- **The AI decides; it never executes or mutates.** `decide` takes `&World`,
  reads only, and calls no host operation. Purity is part of the contract
  (M-9): same inputs, same decision, and `state_hash` unchanged.
- **A move is a destination, never a route.** The host's operations take a
  destination and compute the route themselves; `host_route` must stay the same
  computation as `plan_route` in `crpg-server` (`crates/crpg-server/src/nav.rs`),
  including the snap, the goal and the tag.
- **Refusal orders are contract.** `Sides::new`'s least duplicate,
  `host_route`'s `NavUnbound` → `AbsentEntity` → `NotMover` → `Nav`, and
  `decide`'s M-1 → M-9 first-failure order are observable behaviour. Reordering
  a check changes results.
- **Hostility is the caller's table.** Sim has no sides; never add one here or
  infer it from placement, generation or store order. An unlisted entity is
  hostile to no one.
- **Literal expected values in tests come from the contract.** If correct code
  disagrees with one, stop and report it; never edit the expected value.
- **One call is bounded:** one `legal_actions`, at most one `validate_action`
  per listed ability, at most two `find_path`s. When to call it is the driver's.

## Agent log

- 2026-11-15 (UTC) · opencode-go/deepseek-v4.1-flash + T056a · Opened the file with the integer-only, one-legality-source, read-only and refusal-order rules, the route-equals-`plan_route` and caller-supplied-`Sides` boundaries, and the bounded-call budget, so a later AI change extends the MVP rather than reinterpreting it.
