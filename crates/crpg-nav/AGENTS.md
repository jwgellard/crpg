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

## Agent log

- 2026-11-08 (UTC) · claude-code/sonnet-5.5 (implementer) + T035a · Opened the file with the integer-only, `i128`, no-panic, error-order and canonical-version rules for the crate.
