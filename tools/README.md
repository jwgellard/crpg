# tools

Repository tooling. Nothing here is a Rust crate or a CI workflow.

- `lint/` — the dependency-direction (`deps.py`) and determinism
  (`determinism.py`) lints with their self-tests. CI runs them; so does
  preflight.
- `preflight.sh` / `preflight.ps1` — local native gate runners (T031).
- `tests/` — subprocess tests for the preflight runners.

## Preflight

Runs the repository's existing local gates, in order, stopping at the first
failure:

```text
bash tools/preflight.sh [--crate <package>] [--check-only]          # Linux (and other POSIX)
pwsh -NoProfile -File tools/preflight.ps1 [-Crate <package>] [-CheckOnly]  # Windows
```

1. `rustc -vV` — prints the native host and toolchain actually used.
2. `cargo metadata --no-deps --format-version 1 --locked`, validated; in crate
   mode the package must exactly match a **workspace member** name.
3. `cargo fmt --all` (skipped by check-only), then `cargo fmt --all -- --check`.
4. `cargo clippy --workspace --all-targets --locked -- -D warnings`
   (crate mode: `-p <package>` instead of `--workspace`).
5. `cargo test --workspace --locked` (crate mode: `-p <package>`).
6. `python3 tools/lint/deps.py` (`python` from PowerShell).
7. `python3 tools/lint/determinism.py`.
8. `python3 -m unittest discover -s tools/lint -p "test_*.py"`.
9. `cargo deny check` — always the workspace graph, even in crate mode.
10. `git diff --check`.

Before step 1 the runner checks that `Cargo.toml`, `rust-toolchain.toml` and
the lint scripts exist under the root it resolves from its own location
(any caller directory works, paths with spaces included), that `rustc`,
`cargo`, the Python launcher and `git` are on `PATH`, and that Python is
3.11 or newer. Every tool is invoked with an argument array; nothing is
`eval`ed and no environment change outlives the run.

Exit codes: `0` every gate ran and passed (the last line says so); `1` a gate
failed — the failed command and its native exit code are printed and no later
gate runs; `2` invalid arguments, a missing prerequisite, an unknown or
non-member package, or malformed metadata — reported before any formatting or
other modifying gate. `--help` / `-Help` prints usage and runs nothing.

### Limits

- **Local checks, not CI.** A pass is not a remote CI result and does not
  replace GitHub's protected-path, secret-scan or fork-guard jobs.
- **Native only.** The runners never pass `--target`; they check whatever
  host they run on. Task acceptance that names both native targets
  (Windows/MSVC and Linux/GNU) needs a run on each.
- **Crate mode is partial.** A `--crate` / `-Crate` run prints "crate gates"
  and does not satisfy any full-workspace or second-native-target acceptance
  (POST-T018 standing rules).
- **Check-only is only about formatting.** It skips the rewriting
  `cargo fmt --all`; every other gate still runs. It is not a dry run.
- The runners provision nothing: no toolchain, package, WSL distribution,
  schema or golden generation. Missing tools are reported, not installed.

### Tests

```text
python3 -m unittest discover -s tools/tests -p "test_preflight.py"   # Linux
python -m unittest discover -s tools/tests -p "test_preflight.py"    # Windows
```

The suite launches each runner as a real process inside a temporary fake
repository (whose path contains spaces) with fake `rustc`/`cargo`/`python`/
`git` executables alone on `PATH`, then checks the exact argv sequence, the
working directory of every call, exit codes, first-failure stopping, help and
invalid-argument paths, membership validation (including a plausible
non-member package) and the success line. Bash runs on POSIX hosts;
PowerShell runs wherever `pwsh` is found and is required on Windows.

## Agent log

- 2026-09-30 (UTC) · claude-code + T031 implementation · Documented the preflight runners' fixed gate order, exit codes and limits (local, native-only, partial crate mode) so a green local run is not mistaken for CI or dual-native acceptance.
