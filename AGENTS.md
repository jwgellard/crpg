# CRPG engine — agent rules

Rust workspace. Simulation core has NO game-engine dependency.

## Non-negotiable
- Only `crpg-godot` may depend on `godot`. Only `crpg-godot` may use `unsafe`;
  every other crate root carries `#![forbid(unsafe_code)]` and `deps.py` fails
  if one does not. Every crate root counts, `src/bin/*.rs` included.
- Dependency direction: core <- data <- rules <- sim <- {net, ai, script} <- server.
  Never import upward. Never create a cycle. That line is the spine, not the
  whole rule — the complete, enforced edge list is the `ALLOWED` table in
  `tools/lint/deps.py`, and a crate missing from it is itself a failure.
  Everything counts as an import: `[dependencies]`, `[dev-dependencies]`,
  `[build-dependencies]`, and the same three under any `[target.*]` block. A
  renamed dependency (`x = { package = "godot" }`) is the crate it names, not
  the key it is filed under.
- No `HashMap`/`HashSet` in `crpg-core`, `crpg-rules` or `crpg-sim` — use
  `IndexMap`/`BTreeMap`.
- No `f32`/`f64` in `crpg-core` or `crpg-rules` — use integers or `Fx16_16`.
  `crpg-sim` may use `f32` for spatial positions only (spec §2.4), never in a
  rules path. `f64` is banned in `crpg-sim` as well (E006-A); the determinism
  lint enforces it as `no-f64`.
- The two bans above apply to doctests as well as to `tests/` and `#[cfg(test)]`
  modules. A doctest is compiled and run; `determinism.py` scans inside doc
  fences for exactly that reason.
- Do not add dependencies without being asked.
- Do not modify `crpg-contracts` or `rust-toolchain.toml`.
- Do not weaken or delete an existing test to make a build pass. Stop and say so.

## Working rules
- One task = one crate. If a task needs two crates, stop and say so.
- Finish only when the task file's stated command passes.
- The first task that puts real code in a crate also writes
  `docs/architecture/<crate>.md` and the crate's `AGENTS.md`. Spec §15.6: a
  crate with no architecture doc is not ready for agent work. Later tasks in
  that crate extend both rather than starting new ones. See
  `docs/architecture/README.md` for what belongs in which file — an ADR is
  *why*, an architecture doc is *what*, an `AGENTS.md` is *how to work on it*,
  and copying between them is how they come to disagree.
- Before finishing, run every gate CI runs — `--all-targets` matters, because
  CI lints test code and a bare `-p <crate>` does not:

```
cargo fmt --all
cargo clippy -p <crate> --all-targets -- -D warnings
cargo test -p <crate>
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
```

- The lint self-tests are in that list on purpose. The lints are what enforce
  the non-negotiables above, so a change that defangs one has to fail
  somewhere, and this is where.
- Every agent edit to documentation signs itself in the same file and commit:
  `YYYY-MM-DD (UTC) · <harness/model> + <task or reason> · 1–2 sentence why`.
  Appends carry their own entry; an append without one fails the rule. Never
  rewrite or delete a prior entry; supersede by appending. Human edits need no
  entry; never forge a human entry.

## Note
- "Godot4" is available in PATH CLI

## Agent log

- 2026-09-05 · opencode/big-pickle · Established the documentation attribution rule: all agent edits to .md files must carry date/agent/reason signature to keep history auditable and prevent silent doc drift.
- 2026-09-06 (UTC) · opencode/muse-spark + E006-A · Banned `f64` in `crpg-sim` (`f32`-spatial only); the determinism lint enforces it as `no-f64`.
