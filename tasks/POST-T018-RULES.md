# Standing rules for post-T018 contracts

**Readiness update (2026-09-29):** the date-specific “Only T019” and missing-exact-
contract statements below are superseded by
[SPECIFICATION-READINESS.md](SPECIFICATION-READINESS.md) for the nine specified
tasks. Existing source prerequisites, ADR/API review, dependency approvals and
native acceptance rules are unchanged.

## Scope and readiness

Current decision authority: [POST-T018-DECISIONS.md](POST-T018-DECISIONS.md).
The user delegated D01–D18 selections after this rules file was drafted. Exact
API/ADR/dependency evidence is still required; historical "owner gate" now
means crpg-server, and the historical quinn before-choice hold is resolved.

Incorporated by reference into every T019–T029 child. Read root AGENTS,
the owning crate's AGENTS/architecture, E004, E017 A–B and T018a/b/c completion
records before work. Frozen shapes are cited, not restated here.

Only T019 is evidence-complete today. A blocked contract must receive an
append-only readiness record containing the approved exact Rust signatures,
field/tag/serde layout, errors and precedence, resource bounds, prerequisites,
ADR identifiers and test names before implementation. Recommendations and
candidate signatures are not approval. No placeholder implementations/tests.
Host contracts deliberately have an owner gate: selecting a package alone is
insufficient; replace their conditional commands with literal approved package
commands in that readiness record. N-QUIC is a specification hold, not an
executable contract, until the carried decision is made.

## Constraints (all children)

- E004: one implementation crate per child. If another crate needs source,
  fixtures, manifests or tests, stop and write an ordered child; no implicit
  mechanical downstream exception. Docs supporting that crate are allowed.
- No crpg-contracts or rust-toolchain changes. Preserve root unsafe/Godot,
  deterministic containers/numbers and platform-neutral sim constraints.
- ALLOWED in tools/lint/deps.py is a ceiling, never approval. Any dependency
  (including a new dev edge or feature expansion) requires a T018a-style
  record: exact package/version/features/source, purpose, graph edge, license,
  build-script/advisory/duplicate audit, explicit human approval, lock impact
  and clean `cargo deny check`. Never widen deny.toml to pass.
- Testkit stays an integration consumer, not a dependency of core/data/rules/
  sim. Net/host conformance stays in its owner, not testkit runtime code.
- No replay-format, hash-exclusion, golden or target-selection change without
  its own reviewed ADR. New history goldens require explicit ADR coverage;
  existing B4/legacy artifacts remain byte-identical. No blanket rebaseline.
- Preserve exact-build determinism and compile-time target-scoped selection.
  Windows/MSVC and genuine Linux/GNU gates are mandatory, pinned toolchain,
  normal test profile, default features. Never substitute cross-compilation,
  cross-target hashes, tolerance or missing-baseline skips. Godot presentation
  acceptance is Windows-primary; Linux headless dependency gates remain real.
- Every new acceptance assertion enters through public APIs; every rejection
  has a valid positive control and complete-state unchanged evidence. Replica
  tests use independent permitted-view oracles with drop/corrupt negative
  controls, not full World hash equality or production projection as oracle.
- T018 source/completion details govern as-built behavior: fabric rate → size
  → queue, driver retry/dedup, cache-before-exhaustion, test-only visibility
  and snapshot tracker. Do not turn them into an alleged production host.
- Preserve v1 fixtures/semantics; additive closed variants require reviewed
  versions and fixtures. No raw SimEvent serialization. Per-client ids and
  sequences must not disclose hidden world ids, sequence gaps or event counts.
- No existing test is weakened/deleted. Stop on a conflicting contract.

## Test and Definition of done (mandatory additions to every child)

Run the child's literal focused command, its crate clippy/test commands and
these commands on both native targets (shell syntax appropriate to each):

```text
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
cargo deny check
git diff --check
rustc -vV
```

The child's final stopping command is run after the focused and common gates.
Record commands, environment, results/counts and limitations honestly; running
the command against zero tests is not acceptance. Capability gates 10–13
activate with real persistence/perf/product/smoke implementations, never stubs.

- [ ] Required ADR/API/dependency approvals linked; single owner respected.
- [ ] Focused, full crate, workspace and lint gates pass on both native targets.
- [ ] Public-API positive/negative controls and compatibility evidence retained.
- [ ] Owning architecture and AGENTS extended; first-code task writes both
      and updates architecture README. Every doc edit appends attribution.
- [ ] Completion record states next contract's usable output and open gates.
- [ ] No commits, pushes or PRs without separate instruction; retain any
      generated regression artifacts for later authorized commit.

## Agent log

- 2026-09-28 (UTC) · opencode/gpt-6-astra + successor contract rules · Centralized mandatory native gates and readiness/compatibility requirements so blocked contracts cannot grant implicit dependencies, multi-crate edits or golden changes.
- 2026-09-28 (UTC) · opencode/gpt-6-astra + delegated decisions precedence · Recorded the selected decision record as the current authority without weakening exact-interface, dependency, ADR or native verification gates.
- 2026-09-29 (UTC) · opencode/gpt-6-astra + readiness alignment · Linked current completed specifications while preserving the standing review, dependency and verification gates. Historical readiness claims no longer hide the new exact task appendices.
