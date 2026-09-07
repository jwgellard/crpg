# CRPG engine — agent rules

Rust workspace. Simulation core has NO game-engine dependency.

## Non-negotiable
- Platform invariant ([ADR-0012](docs/adr/0012-windows-primary-platform.md)):
  `x86_64-pc-windows-msvc` is primary for development, product, release gates,
  and behavioural baselines (client, editor, embedded single-player server,
  dedicated server, CLI). `x86_64-unknown-linux-gnu` is fully supported for
  dedicated server, headless CLI/tooling, server-side extensibility, and tests;
  platform-specific failures are defects, not best effort. Linux GUI builds
  are not promised; Linux headless support must not depend on Godot.
- One platform-neutral authoritative server implementation serves Windows
  embedded single-player and Windows/Linux dedicated hosts. In-memory
  transport preserves the authority boundary: clients cannot mutate
  authoritative state directly.
- Keep OS-specific process, filesystem, service, and presentation logic above
  `crpg-core`, `crpg-rules`, and `crpg-sim`; no OS-specific branches in those
  crates. This architectural rule grants no platform dependency permission.
- Replay determinism is exact-build, not cross-platform lockstep. Windows and
  Linux must compare their own independently generated target-scoped goldens
  with the pinned toolchain, normal test profile, and default features, never
  each other's hashes. Select gates at compile time, never by runtime OS;
  no tolerance or missing-baseline skips. Re-baselines require review.
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
- `crpg-testkit` is an integration consumer, not a dependency of the layers it
  tests. It may depend down through `crpg-sim`; `crpg-core`, `crpg-data`,
  `crpg-rules`, and `crpg-sim` must not depend on it, including for tests.
  Their cross-layer integration tests live in `crpg-testkit`. A higher crate
  may dev-depend on testkit only when every normal dependency testkit brings is
  already a legal dependency of that crate and testkit does not depend back on
  it. `tools/lint/deps.py`'s explicit `ALLOWED` table remains authoritative.
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
- T009a (typed replay) and T009c (Windows-primary/Linux-supported native
  golden policy, ADR-0012) are merged on `master` as of 2026-09-07; native
  Windows/MSVC and genuine Linux/GNU in WSL Ubuntu 24.04 gates passed and the
  final audit is complete. T009b (thin `crpgc replay` wrapper) is next;
  see the [completion record](tasks/T009c.md).

## Agent log

- 2026-09-05 · opencode/big-pickle · Established the documentation attribution rule: all agent edits to .md files must carry date/agent/reason signature to keep history auditable and prevent silent doc drift.
- 2026-09-06 (UTC) · opencode/muse-spark + E006-A · Banned `f64` in `crpg-sim` (`f32`-spatial only); the determinism lint enforces it as `no-f64`.
- 2026-09-06 (UTC) · opencode/gpt-5.6-sol + E005 decision · Ratified testkit as a one-way integration consumer: lower simulation layers keep self-contained tests, while their cross-layer tests live in testkit.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c documentation alignment · Added ADR-0012's platform, authority, portability, and replay invariants without granting dependencies or OS-specific substrate logic. These are policy requirements, not a claim that T009c gates have been verified.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c verification status · Recorded the reported passing native gates and linked the completion record without changing platform invariants. Both tasks remain uncommitted, T009c retains priority before T009b, and review/merge and final audit remain outstanding.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c final audit · Recorded the reported final audit completion and completed working-tree implementation/verification without changing platform invariants. Both tasks remain uncommitted, review/merge is outstanding, and T009c remains the priority before T009b.
- 2026-09-07 (UTC) · opencode/big-pickle + T009a/T009c merged · Updated the note to the merged state: both tasks landed on `master` in `bb9a702` with native gates and final audit complete; T009b is next.
