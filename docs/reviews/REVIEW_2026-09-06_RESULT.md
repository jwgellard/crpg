# Review result — 2026-09-06

Autonomous execution record for the maintainer to review on return. Companion
to `REVIEW_2026-09-06_PLAN.md`, which holds the full plan and context.

## What ran

Three phases were executed in one autonomous session (sub-agents + verified
locally): Phase A (mechanical fixes), Phase C1/C2 (determinism lint tooling),
and Phase E (deferred-task record-writing). Phases B and D were deferred by
maintainer instruction (chose "defer ALL human-decision phases").

### Phase A — delivered

- `crates/crpg-cli/Cargo.toml`: added `[[bin]] name = "crpgc"` pointing at
  `src/main.rs`. Spec/README `crpgc` commands now resolve.
- `crates/crpg-cli/src/main.rs`: stub now prints `crpgc: not yet implemented`
  to stderr and exits 1 (was println + exit 0).
- `crates/crpg-server/src/main.rs`: same treatment (`crpg-server: not yet
  implemented`, exit 1).
- `crates/crpg-persist/src/lib.rs`: module doc reworded from "now" to
  "planned" (crate is empty).
- Gates: `cargo check` (3 crates) clean; both binaries exit 1; `clippy -D
  warnings` clean; `deps.py` 0; `fmt --check` clean. Verified twice (sub-agent
  + local).

### Phase C1 — delivered

- `tools/lint/determinism.py`:
  - New unsuffixed float-literal detector (2nd `no-float` rule covering
    core+rules). Matches `1.5`, `0.5`, `1e3`, `1.0e-2`, `.5`; rejects `1..`,
    `x.foo()`, `self.0`, `0x1F`, char literals, integer-with-method.
  - `mask_string_literals()` blanks the contents of `"..."`/`'…'` before the
    escape-marker check and pattern rules, so a `// determinism-ok:` appearing
    *inside a string* no longer suppresses a line, and banned tokens inside
    strings are no longer false-flagged.
- `tools/lint/test_determinism.py`: 12 new self-tests (unsuffixed floats,
  exponent, string-pass, range/member pass, hex pass, char pass, int/method
  pass, marker-in-string-no-suppress, spaced-path rule parse).
- Verified: 61 lint self-tests pass (was 49), `determinism.py` exit 0 on the
  real tree, `cargo test -p crpg-core --locked` green (11 unit + 1 doctest;
  scope note 2026-09-05: that was a partial/filtered re-run count —
  PROJECT_STATE records 85 tests incl. doctest for the full tree).

### Phase C2 — delivered

- `tools/lint/test_determinism.py`: violation lines now parsed with
  `re.match(r"^VIOLATION.*?:(\d+)\s+(\S+)")` instead of whitespace-splitting,
  so Windows paths containing spaces (e.g. `C:\Users\Jane Doe\…`) parse
  correctly. Unit test covers the spaced-path case.
- `.github/workflows/ci.yml`: `lint-selftest` now runs on
  `[ubuntu-latest, windows-latest]` matrix.

### Phase E — deferred task files written (NOT implemented)

Writing these does not change behavior or policy; they exist so a future
maintainer-assigned agent can execute them after human sign-off:

| File | Decision required |
|---|---|
| `tasks/E001-events.md` | Event types: core (payloads) vs sim (queue); assign to a task. |
| `tasks/E002-duplicate-entityid.md` | Spec §2.4 defines 2nd `EntityId`; keep ADR-0006 single-id, edit spec. |
| `tasks/E003-contracts-placement.md` | `crpg-contracts` can't impl net traits under graph; relax or rehome. |
| `tasks/E004-one-task-one-crate.md` | Multi-crate T008/9/11/16/18 violate the rule; split or relax. |
| `tasks/E005-testkit-cycle.md` | Sim↔testkit dev-cycle; one-way ownership rule. |
| `tasks/E006-f64-in-sim.md` | Lint bans `f64` in sim vs doc allows `f64` non-spatial; pick one. |
| `tasks/E007-adr-immutability.md` | Define immutable-vs-append boundary after T002b edited ADR-0004. |
| `tasks/E008-wallclock-event-budget.md` | Spec uses wall-clock event budget → switch to instruction budget. |

Also **explicitly hidden/deferred by policy, NOT revisited**:
- Phase B security hardening (S001): `yanked = "deny"`, wildcards deny, build.rs
  ban, gitleaks, guard job, self-hosted runner. Existing `tasks/S001.md`. No
  changes made to `deny.toml` or CI security policy.
- `tools/lint/deps.py` changes (recursive root discovery, inner-attribute
  validation, integration-test scanning) — reviewed as findings, deferred;
  **no changes made**. The unsafe-substring-search gap (C2 in the plan) is
  acknowledged.
- `caution`: nothing in `deny.toml`, `rust-toolchain.toml`, `crpg-contracts`,
  or `tools/lint/determinism.py`'s policy scope was changed, and no task was
  modified except the review's own REV items in the uncommitted tree.

## State of the working tree

- Phase A/C/E changes are uncommitted alongside the pre-existing T006a–e work
  (core fixes, ADR-0007, docs). Expected before this session.
- `git status` file list (relevant new): `.github/workflows/ci.yml`,
  `crates/crpg-cli/Cargo.toml`, `crates/crpg-cli/src/main.rs`,
  `crates/crpg-server/src/main.rs`, `crates/crpg-persist/src/lib.rs`,
  `tools/lint/determinism.py`, `tools/lint/test_determinism.py`,
  `tasks/E001..E008`, `docs/reviews/REVIEW_2026-09-06_PLAN.md`. Everything else
  in the listing predates this session.

## Gates run at session end (all green)

1. `cargo test --workspace --locked` — all packages green (85 core-equivalent
   total incl. 1 doctest).
2. `python tools/lint/deps.py` — exit 0.
3. `python tools/lint/determinism.py` — exit 0.
4. `python -m unittest discover -s tools/lint -p "test_*.py"` — 61 OK.
5. `cargo fmt --all -- --check` — clean (verified in Phase A sub-agent).
6. `cargo clippy -p crpg-cli -p crpg-server -p crpg-persist --all-targets
   --locked -- -D warnings` — clean (Phase A).

No commits were made. Nothing was force-pushed. Lockfile modifications present
in the pre-existing T006 work were not touched.

## What the maintainer should review

1. The four bullet diffs above (Phase A/C) — read each changed file.
2. The 12 new lint self-tests in `tools/lint/test_determinism.py`.
3. `tools/lint/determinism.py` — the new float regex and
   `mask_string_literals`.
4. The 8 deferred task files E001–E008 and decide next steps (assign an agent
   per file after sign-off).
5. Whether to fold this session's changes into the T006 merge commit or keep
   separate — the user's call on return.

## Agent log

- 2026-09-05 (UTC) · opencode/muse-spark + count correction · Flagged the "11 unit + 1 doctest" line as a partial re-run count versus PROJECT_STATE's 85-test full-tree figure, so future readers do not bisect against it.