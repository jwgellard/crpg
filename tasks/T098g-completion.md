# T098g completion — close budget reserve (D43a)

Route: opencode-go/deepseek-v4.1-flash · slot 2 · branch `task/T098g-i1` ·
2026-11-07 (UTC). Sole crate `crpg-net-quic`; native Windows/MSVC plus
Linux/GNU via WSL Ubuntu-24.04.

## Landed

- `src/teardown.rs` (new): `CLOSE_RESERVE_CAP_MS = 200`, `close_reserve_ms`,
  `close_deadlines` (one `Instant::now()`), `finish_endpoint_close`
  (`Endpoint::close`, one `yield_now`, bounded `wait_idle`).
- `src/queue.rs`: `FlushRequest.deadline` → `flush_end`; `run_flush` sleeps
  to it; `Stop.deadline` holds `overall`.
- `src/client.rs`: `stop` builds both instants once, the `FlushRequest` and
  `Stop` use them, the exit uses `finish_endpoint_close`.
- `src/server.rs`: `close(Flush)` and `shutdown` build the same split once;
  `Drop` unchanged; `close_quinn_endpoint` deleted; `server_main` exit uses
  the helper. `deadline_after` now appears only in the import and `Drop`.
- `src/close.rs`: `CloseMode::Flush` rustdoc only.

## Evidence

### Test 2 mutation control (recorded, then restored)

Body of `finish_endpoint_close` replaced with the pre-fix pair
(`endpoint.close`, then `timeout_at(overall, wait_idle)`, no yield):

```
cargo test -p crpg-net-quic --lib --locked -j 2 goodbye_sent_after_the_endpoint_deadline_passed
EXIT=101  FAILED
panicked at crates\crpg-net-quic\src\teardown.rs:186:9:
assertion `left == right` failed
  left: None
 right: Some(Peer { code: 0 })
```

Restored; the same command then passes (`EXIT=0`, 2 lib tests pass).

### Test 3 pre-fix count (5 runs)

Pre-fix state emulated by `close_reserve_ms` returning 0 and removing the
exit yield: `cargo test -p crpg-net-quic --test close_budget --locked -j 2
client_flush_close_over_budget_reaches_peer`, once per run.

| Run | Exit | Failure |
|---|---|---|
| 1 | 101 | `deadline: server frame` |
| 2 | 101 | `deadline: server frame` |
| 3 | 101 | `deadline: server frame` |
| 4 | 101 | `deadline: server frame` |
| 5 | 101 | `deadline: server frame` |

Count: **5/5** `deadline: server frame` failures.

### Test 4 and test 5 pre-fix (guards)

Same pre-fix state, one run each:

- `shutdown_over_budget_reaches_every_client`: `EXIT=0`, `1 passed`.
- `single_close_flush_keeps_the_reserve`: `EXIT=0`, `1 passed`.

Both match D5: these are guards, not fail-first tests.

### Case 13, 10 filtered Windows runs

`cargo test -p crpg-net-quic --test quic --locked -j 2
ordering_preserved_under_concurrent_traffic`, ten consecutive runs:
all `EXIT=0`, `1 passed; 0 failed; 31 filtered out`, 3.44–3.53 s each.
No failure, so no panic line to report.

### Acceptance (native Windows/MSVC, `-j 2`, `--locked`)

| Command | Exit |
|---|---|
| `cargo fmt --all -- --check` | 0 |
| `cargo clippy -p crpg-net-quic --all-targets --locked -j 2 -- -D warnings` | 0 |
| `cargo test -p crpg-net-quic --lib --locked -j 2` (2 tests) | 0 |
| `cargo test -p crpg-net-quic --test close_budget --locked -j 2` (3) | 0 |
| `cargo test -p crpg-net-quic --test quic --locked -j 2` (32) | 0 |
| `cargo test -p crpg-net-quic --test quic_gaps --locked -j 2` (7) | 0 |
| `cargo test -p crpg-net-quic --test channel --locked -j 2` (21) | 0 |
| `cargo test -p crpg-net --locked -j 2` | 0 |
| `cargo tree -p crpg-net -e normal,build --target all` (no tokio/quinn/rustls/ring/mio/socket2) | 0 |
| `cargo test --workspace --locked -j 2` | 0 |
| `python tools/lint/deps.py` | 0 |
| `python tools/lint/determinism.py` | 0 |
| `python -m unittest discover -s tools/lint -p "test_*.py"` (81) | 0 |
| `cargo deny check` (advisories, bans, licenses, sources) | 0 |
| `pwsh -NoProfile -File tools/preflight.ps1 -Crate crpg-net-quic` | 0 |
| `git diff --check` | 0 |

Environment note: the first `cargo test --workspace` attempt failed while
linking (`LNK1318`/`LNK1108`) because `C:` had 0 bytes free. `cargo clean`
removed 31.6 GiB of this worktree's build artifacts (nothing else), and the
rerun with `CARGO_INCREMENTAL=0` passed with exit 0; no source changed.

Scope greps:

- `grep -rnE "HashMap|HashSet|f32|f64|unsafe|cfg\(target_os|cfg\(windows|println|eprintln|tracing|log::|tokio::main" crates/crpg-net-quic/src`
  prints only `src/lib.rs:1:#![forbid(unsafe_code)]`.
- `wait_idle` in `src/`: `teardown.rs` (code), `client.rs`'s
  `finish_endpoint` (unchanged, Behaviour 7), and the pre-existing `Drop`
  rustdoc line in `server.rs`.
- `deadline_after` in `src/server.rs`: the import and the `Drop` line only.

### Linux/GNU (WSL Ubuntu-24.04, `~/.cargo/bin`, `CARGO_TARGET_DIR=$HOME/t098g-target`)

Script run through `wsl -d Ubuntu-24.04 -- bash -lc`; all exit 0:

- `cargo test -p crpg-net-quic --lib --locked -j 2` — 2 passed
- `cargo test -p crpg-net-quic --test close_budget --locked -j 2` — 3 passed
- `cargo test -p crpg-net-quic --test quic --locked -j 2` — 32 passed
- `cargo test -p crpg-net-quic --test quic_gaps --locked -j 2` — 7 passed
- `cargo test -p crpg-net-quic --test channel --locked -j 2` — 21 passed
- `cargo fmt --all -- --check` — exit 0
- `cargo clippy -p crpg-net-quic --all-targets --locked -j 2 -- -D warnings` — exit 0

## Docs

- `docs/architecture/crpg-net-quic.md`: `teardown.rs` in the module flow,
  footnoted Close flush row, close-budget Bounds bullet, signed.
- `crates/crpg-net-quic/AGENTS.md`: two teardown test commands, the yield /
  flush-deadline trap, tokio pin corrected to `=1.53.3`, signed.
- `tasks/T023.md`: appended amendment (no earlier line touched), signed.

## Agent log

- 2026-11-07 (UTC) · opencode-go/deepseek-v4.1-flash + T098g · Recorded the mutation control, the pre-fix 5/5 test-3 failures, the test-4/5 guard runs, ten case-13 runs and every gate exit code, so the reserve's evidence is auditable without re-running the pre-fix state.
