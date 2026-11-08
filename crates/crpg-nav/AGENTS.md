# crpg-nav — agent contract

Read the root rules and [T035](../../tasks/T035.md) (the part you were
given), then [ADR-0036](../../docs/adr/0036-movement-and-navigation.md).
Architecture: [crpg-nav](../../docs/architecture/crpg-nav.md). This file
holds only how-to-work rules.

## Definition of done for any change

```
cargo fmt --all
cargo clippy -p crpg-nav --all-targets -- -D warnings
cargo test -p crpg-nav
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
```

## Rules and traps

- **Integers only, tests and doctests included.** No `f32`, `f64` or float
  literal anywhere in the crate; the determinism lint (T035v) scans
  `crpg-nav` with the full float ban and fails if the crate is missing.
- **The float-literal lint flags nested tuple access such as `a.0.1`.**
  Destructure instead.
- **`i128` for every cross product and distance**, on raw `to_raw()` values.
  Take magnitudes with `unsigned_abs()`, never `abs()` (it overflows on
  `i32::MIN`).
- **No panic on caller input.** Every public method accepts any
  `[Fx16_16; 3]`, any ids and any bytes; out-of-range ids return `None`.
- **Validation and error orders are contract.** `NavMesh::new`, the decoder
  and (later) the bake and path errors return the *first* failure in the
  contract's order. Reordering a check changes observable behaviour.
- **Never change the canonical layout without raising `NAV_FORMAT_VERSION`.**
  Its bytes are hashed into a `NavRevision` by the host.
- **No dependency beyond `crpg-core`,** no `HashMap`/`HashSet` (use
  `BTreeMap`/`BTreeSet`), no `unsafe` (the root keeps
  `#![forbid(unsafe_code)]`).
- **Literal expected values in tests come from the contract.** If correct
  code disagrees with one, stop and report it; never edit the expected value.

- **Query and bake tie-breaks are contract.** `nearest_on_mesh` keeps the
  first *strictly* smaller `d2` (lowest polygon, then lowest edge); the edge
  candidate tries `(fx,fz) (fx,cz) (cx,fz) (cx,cz)` in that order and keeps
  the first on a tie; the none-contained fallback takes `a` when
  `d2(a) <= d2(b)`. Use `div_euclid`/`rem_euclid`, never `/` or `%`, on a
  signed product. `bake_box` checks `TooManyHoles` before `OutOfBounds`
  before `EmptyOuter`, and per hole empty before outside.
- **Rounding tests need searched cases.** A hand-picked query rarely reaches
  the ceiling, negative-floor, order or tie rules; `tests/query.rs`
  `b5_rounding_rules` holds cases found against an independent integer model.
- **The A* open-set order is contract.** The key is `(f, h, PolyId)` and a
  neighbour's cost changes only on a strict improvement; changing either moves
  the corridor. `tests/path.rs` `c2_ring_tie_break`, `c2b_*` and `c2c_*` each
  fail under a different wrong tie-break, so keep them when editing.
- **Funnel and portals follow B-P4/B-P5 exactly.** Left is the shared edge's
  end vertex and right its start, as the polygon being left holds it. Path
  points drop only `x`/`z`-equal repeats; `y` is carried, never compared.
- **The V5 oracle, not a reading of the code, proves a path is inside the
  mesh.** Extend `c8_inside_the_mesh`'s oracle rather than loosening it, and
  take every cross product in `i128`.

## Agent log

- 2026-11-08 (UTC) · claude-code/sonnet-5.5 (implementer) + T035a · Opened the file with the integer-only, `i128`, no-panic, error-order and canonical-version rules for the crate.
- 2026-11-08 (UTC) · claude-code/sonnet-5.5 (implementer) + T035b · Added the bake and query tie-break traps (strict-less keeps the first, floor not truncation, error order) and the note that rounding rules need searched test cases.
- 2026-11-08 (UTC) · claude-code/sonnet-5.5 (implementer) + T035c · Added the A* open-set, portal and funnel traps and the rule that the V5 oracle in `tests/path.rs` is the proof of containment.
