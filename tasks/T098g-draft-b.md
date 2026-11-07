# T098g — reserve a slice of the close budget for CONNECTION_CLOSE (`crpg-net-quic`)

Status: **contract draft, not approved.** Answers D43a (a) from
`tasks/DECISIONS-D43.md:19`: a close still finishes within the budget, and a
fixed slice of that budget is reserved for CONNECTION_CLOSE. Sole crate:
`crpg-net-quic`. Adjudication input: this draft (b) plus the parallel draft
(a); a Claude architect picks the final contract.

Failure being fixed: `QuicClient::stop` fixes one shared deadline before
flushing (`crates/crpg-net-quic/src/client.rs:462`), `run_flush` may spend the
whole budget waiting for the FIN ack
(`crates/crpg-net-quic/src/queue.rs:961-985`), then `client_main` waits on the
same, already expired deadline
(`crates/crpg-net-quic/src/client.rs:680-685`), so the I/O thread ends before
the queued CONNECTION_CLOSE is transmitted and the peer only idles out at
60 s (`crates/crpg-net-quic/src/limits.rs:27`), past the 30 s test guard
(`crates/crpg-net-quic/tests/support/quic_rig.rs:33`).

## Decision

| # | Choice | Recommendation |
|---|---|---|
| D1 | Reserve = `min(200 ms, budget / 4)`, computed where the budget is fixed, as a private `const`, not a config field | Adopt. Fixed 200 ms covers a loopback close with margin; the `/4` branch keeps ≥75 % of small budgets for flushing; a new config field would be a public API change needing an ADR per `crates/crpg-net-quic/AGENTS.md:41-42`, so prefer none |
| D2 | Client `Flush`: flush phase ends at `stop.deadline − reserve`; CONNECTION_CLOSE always gets `[flush_end, stop.deadline]` | Adopt (B1). The total bound (`close_flush_timeout_ms`, `crates/crpg-net-quic/src/limits.rs:35`, range `0..=2_000` at `:126-131`) is unchanged |
| D3 | `server.shutdown`: same split, one shared overall deadline, per-connection flush ends | Adopt (B2). One slow connection cannot eat another's reserve because each connection's control task issues its own close (B2) |
| D4 | `QuicServer::close(.., Flush)` (single connection, `crates/crpg-net-quic/src/server.rs:596-640`): same split | Adopt (B3). Same helper as D2/D3 |
| D5 | No public-surface change: no field, variant, code, or range change | Adopt (§Interface, §What must not change) |
| D6 | Budget 0 or budget < reserve: reserve floors at 0, flush is skipped, the close is still issued and waited on to the (already past) overall deadline | Adopt (B4). Matches probe 1, where a spent budget still delivered the close (`tasks/T098.md:344-347`) |
| D7 | `Drop` paths keep today's shape; only their shared-deadline waits inherit the split where a flush exists | Adopt. Client `Drop` is `Discard` (`crates/crpg-net-quic/src/client.rs:519-529`); server `Drop` flushes nothing (`crates/crpg-net-quic/src/server.rs:765-783`) |

Rejected: (b) from D43a, giving the close its own extra wait after the flush
(a close could take 2× budget, and `shutdown`'s callers wait longer); a fixed
200 ms with no fraction branch (budgets ≤ 200 ms would flush nothing); a new
`close_reserve_ms` config field (public API change, ADR-gated, unjustified
for a constant).

## Interface

No public item changes. `CloseMode` (`crates/crpg-net-quic/src/close.rs:74-82`),
`CloseReason` (`:84-102`), `CloseReport`
(`crates/crpg-net-quic/src/client.rs:92-99`) and `ShutdownReport`
(`crates/crpg-net-quic/src/server.rs:186-195`) keep every field and meaning.

Private only:

```rust
// crates/crpg-net-quic/src/queue.rs (private)
impl FlushRequest { /* deadline: Instant becomes the flush-phase end */ }
pub(crate) const CLOSE_RESERVE_MS: u32 = 200; // flush reserve ceiling
pub(crate) fn split_deadline(budget_ms: u32, overall: Instant) -> (Instant /*flush_end*/, Instant /*overall*/);
```

`split_deadline` returns `(overall − min(200 ms, budget/4), overall)`,
floored at now when the subtraction underflows. `FlushRequest.deadline`
(`crates/crpg-net-quic/src/queue.rs:148-154`) carries the flush-phase end;
`Stop.deadline` (`:265-270`) keeps the overall end. `deadline_after`
(`:58-63`) still builds the overall deadline. `SYNC_GRACE`
(`crates/crpg-net-quic/src/queue.rs:32-34`) still bounds only the sync waiter,
never the wire.

## Behaviour

B1. Client `Flush` close after the fix, tied to the current code:
1. `stop` fixes the overall deadline exactly as today
   (`crates/crpg-net-quic/src/client.rs:462`) and derives
   `flush_end = split_deadline(budget_ms, deadline).0`.
2. The `FlushRequest` it installs (`:469-477`) carries `flush_end`, not the
   overall deadline.
3. `run_flush` waits for `writer_finished` (queue drained via `pop_outbound`
   at `crates/crpg-net-quic/src/queue.rs:439-447`, `FIN` + ack at `:877-881`)
   or peer close (`:970-985`) only until `flush_end`; then it records the
   close (`:986-993`, first-wins, discarding the rest via
   `discard_outbound` at `:449-460`), sets `close_done` (`:990`), and issues
   quinn's application close (`close_quinn`, `:106-109`) at `:997-998`.
4. `client_main` still calls `endpoint.close(..)` (`crates/crpg-net-quic/src/client.rs:680-683`)
   and then `timeout_at(overall_deadline, endpoint.wait_idle())` (`:684-685`).
   By construction `[flush_end, overall]` (≥ 0, typically 200 ms) remains for
   the queued CONNECTION_CLOSE to go out.
5. The sync waiter still joins to `guard_deadline = overall + SYNC_GRACE`
   (`:479-488`); the report still counts frames never handed to QUIC
   (`:492-495`).

B2. `server.shutdown` (`crates/crpg-net-quic/src/server.rs:700-759`):
1. The shared overall deadline is fixed exactly as today (`:706`); the
   per-connection `FlushRequest`s (`:722-731`) carry `flush_end`.
2. Each accepted connection's control task (`crates/crpg-net-quic/src/queue.rs:896-957`)
   runs its own `run_flush` and issues its own `close_quinn` (`:997-998`) at
   its flush end. A slow connection therefore cannot delay or consume
   another connection's close: closes are issued concurrently per
   connection, never from a serial loop.
3. Pending connections still close at once with `Shutdown`
   (`crates/crpg-net-quic/src/server.rs:719-721`).
4. The endpoint still closes and waits idle bounded by the shared overall
   deadline (`:862-864`), which now retains the reserve because no flush
   runs past `flush_end`.
5. `connections_closed` still counts accepted + pending closes (`:718`);
   per-connection discarded deltas still sum into the report (`:743-747`).

B3. `QuicServer::close(.., Flush)` (`crates/crpg-net-quic/src/server.rs:596-640`):
same split — the request (`:621-628`) carries `flush_end`; the waiter still
uses `guard_deadline` (`:629-635`); the report still counts
`discarded_since` (`:636-638`).

B4. Edge budgets. `budget_ms = 0` → reserve 0, `flush_end == overall`
(already past): `run_flush` records and closes at once, the endpoint close
is still issued (`crates/crpg-net-quic/src/client.rs:680-683`), matching
probe 1. `0 < budget < reserve ceiling` → reserve `budget/4`, flush keeps
`3/4`. `close_flush_timeout_ms` range and v1 default are unchanged
(`crates/crpg-net-quic/src/limits.rs:35`, `:126-131`).

B5. `CloseReport`/`ShutdownReport` counting is unchanged in kind:
`outbound_frames_discarded` counts only frames never handed to QUIC
(`crates/crpg-net-quic/src/queue.rs:449-460`,
`crates/crpg-net-quic/src/client.rs:492-493`). After the fix it may be
larger by the frames the reserve slice would have needed; that is honest
loss under a smaller flush window, and the peer-visible guarantee (a close
the peer can see) is worth it. `inbound_frames_discarded` still counts
queued-or-parked inbound (`crates/crpg-net-quic/src/client.rs:494`,
`crates/crpg-net-quic/src/server.rs:746-747`). No field or variant is added
or renamed.

B6. Untouched paths. `Discard` still records and closes at once
(`crates/crpg-net-quic/src/server.rs:613-619`,
`crates/crpg-net-quic/src/client.rs:489-491`). Client `Drop` still uses
`(Normal, Discard)` (`crates/crpg-net-quic/src/client.rs:519-529`). Server
`Drop` still sets a fresh `stop` deadline and joins without flushing
(`crates/crpg-net-quic/src/server.rs:765-783`). `finish_endpoint`
(`crates/crpg-net-quic/src/client.rs:688-696`) keeps its full-budget wait.

## Tests

Bounds: every new case reuses the 30 s `DEADLINE` guard
(`crates/crpg-net-quic/tests/support/quic_rig.rs:32-33`) as the failure
oracle. "Well before the idle timeout" means the peer's close is observed
inside that guard, i.e. ≥ 2× sooner than the 60 s idle timeout
(`crates/crpg-net-quic/src/limits.rs:27`). No wall-clock bound tighter than
the guard is asserted.

| # | Case (new; file `tests/quic.rs`) | Asserts exactly |
|---|---|---|
| T1 | `flush_close_reaches_peer_when_backlog_exceeds_budget` (client regression) | 128 × 4,096-byte frames queued (T098f probe shape, `tasks/T098.md:348-355`); server drains 10 then stops; client `close(Normal, Flush)`; server observes `Closed(Peer { code: 0 })` (mapping at `crates/crpg-net-quic/src/queue.rs:89-94`) before `DEADLINE`; `report.outbound_frames_discarded > 0` and `≤ 128 − 10`; no `IdleTimeout` |
| T2 | `shutdown_close_reaches_peer_when_backlog_exceeds_budget` (server regression) | Mirror of T1 from the server side: server queues 128 × 4,096-byte frames to one client that stops reading; `shutdown()`; client observes `Peer { code: 1 }` before `DEADLINE`; `report.outbound_frames_discarded > 0`; `connections_closed == 1` |
| T3 | `zero_budget_flush_still_closes` | `close_flush_timeout_ms = 0`, no backlog; peer sees `Peer { code: 0 }` before `DEADLINE`; `outbound_frames_discarded == 0` (guards the B4 floor) |
| T4 | `small_budget_flush_keeps_majority_for_flush` | `close_flush_timeout_ms = 100` with a 32-frame backlog and a non-reading peer (shape of `crates/crpg-net-quic/tests/quic_gaps.rs:228-256`); peer still sees the close before `DEADLINE`; discarded `> 0` (guards the `/4` branch without timing it) |

Controls (valid, must fail before the fix): T1/T2 run against the current
code must end with `IdleTimeout` (client case) or an unread close
past `DEADLINE` instead of `Peer { code }`, reproducing
`tasks/T098.md:348-355`. T3 passes before and after (probe 1,
`tasks/T098.md:344-347`), so it guards the edge without proving the fix.
T4 passes before and after (same family as
`crates/crpg-net-quic/tests/quic_gaps.rs:228-256`), guarding the small-budget
branch.

Anti-flake: backlog sizes are queue-cap-exact (128 = per-connection cap,
`docs/architecture/crpg-net-quic.md:107-108`), so no timing decides whether
the flush overruns — any scheduling still leaves undelivered frames at
`flush_end`; the only timing assertion is the 30 s guard, which is 2× below
the 60 s idle timeout and identical in kind to the existing case-13 "no
extras" wait (`crates/crpg-net-quic/tests/quic.rs:1062-1070`).

Case 13 (`ordering_preserved_under_concurrent_traffic`,
`crates/crpg-net-quic/tests/quic.rs:973-974`, with its drain wait at
`:1110-1114` and `discarded == 0` at `:1115-1116`) is not edited, relaxed,
retried, or moved.

## Definition of done

Run at the worktree root (crate `AGENTS.md:89-105` gates, `-j 2`):

```text
cargo fmt --all -- --check
cargo clippy -p crpg-net-quic --all-targets --locked -j 2 -- -D warnings
cargo test -p crpg-net-quic --test quic --locked -j 2
cargo test -p crpg-net-quic --test quic_gaps --locked -j 2
cargo test -p crpg-net-quic --test channel --locked -j 2
cargo test -p crpg-net --locked -j 2
cargo test --workspace --locked -j 2
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
cargo deny check
git diff --check
```

Plus the crate's scope grep (only the `#![forbid(unsafe_code)]` line may
match) and native runs on Windows/MSVC and genuine Linux/GNU. T1–T4 pass;
the full suite passes with no existing assertion edited or weakened.

Docs, signed in the same commit: `docs/architecture/crpg-net-quic.md`
(close-flush row + the new reserve), `crates/crpg-net-quic/AGENTS.md` (one
trap: the flush deadline is never the endpoint deadline), and a dated
amendment note in `tasks/T023.md` under the close-budget lines recording the
reserve (the T023 text itself is otherwise unchanged).

Size budget: `src/` diff ≤ ~60 lines (one const, one helper, three
call-site splits); tests ≤ ~150 lines; docs ≤ ~30 lines.

## Out of scope

- Any wire, ALPN, framing, cap, close-code, `Display`, or public-API change.
- Any `close_flush_timeout_ms` default/range change; any idle/keep-alive change.
- DATAGRAM, snapshot, resume, reconnect, migration, or impairment work.
- Host-adapter (`crates/crpg-server`), Godot, CI workflow, `deny.toml`,
  `Cargo.lock`, goldens/replays.
- Making a close reliable under a blackholed network: the reserve covers a
  spent flush budget, not packet loss past the overall deadline.

## What must not change

- Wire bytes (§4 of the T023 contract); ALPN; frame caps; close codes 0–9
  (`crates/crpg-net-quic/src/close.rs:8-32`, names at `:57-72`); `Display`
  texts.
- The T023 contract outside the close-budget lines; other close modes
  (`Discard` at `crates/crpg-net-quic/src/close.rs:74-82`); idle-timeout
  default 60 s (`crates/crpg-net-quic/src/limits.rs:27`); any golden.
- Case 13 and cases 28–30 semantics; no existing test edited or weakened
  (root rule; stop and say so if the build requires it).
- No new dependency or dev-dependency.

## Routing and size budget

One session, one crate (`crpg-net-quic`), implementer-grade but small: the
change is three deadline splits plus a helper and four tests. No S-set file
moves. Sizing above (§Definition of done). Landing order: T1/T2 red
(reproduce `IdleTimeout`), then `src/` split, then T1–T4 green, then docs.

## Unverified quinn details

Not checked against the vendored copy or `~/.cargo/registry` (no access
from this task); the implementation must confirm each before landing:

1. `Connection::close` queues (not transmits) the application
   CONNECTION_CLOSE, so a runtime dropped right after it can lose the close.
2. `Endpoint::close` + `wait_idle` transmits queued endpoint-level closes
   given poll time; `timeout_at` with an already-expired deadline returns
   without polling (the assumed loss mechanism at
   `crates/crpg-net-quic/src/client.rs:684-685`).
3. A 200 ms `wait_idle` on loopback reliably flushes one CONNECTION_CLOSE
   under CI load (margin claim behind D1's ceiling).

## Open questions for the user

1. Reserve shape: `min(200 ms, budget/4)` (D1) vs fixed `200 ms` clamped to
   the budget vs pure fraction (e.g. `budget/4`)? **Recommendation:
   `min(200 ms, budget/4)`** — bounded wall time at v1, graceful at small
   budgets, no config API.
2. Should the reserve ceiling live beside `V1_CLOSE_FLUSH_TIMEOUT_MS`
   (`crates/crpg-net-quic/src/limits.rs:35`) or in `queue.rs` next to
   `SYNC_GRACE` (`crates/crpg-net-quic/src/queue.rs:32-34`)?
   **Recommendation: `queue.rs`** — it is an internal split, not a limit,
   and keeps `limits.rs` public ranges untouched.
3. Should `server.shutdown` give every connection the same `flush_end`, or
   stagger slow connections? **Recommendation: same `flush_end`** (B2) —
   closes are already per-task and concurrent; staggering adds scheduling
   without isolation benefit.
4. Should budget 0 still attempt the flush wait, or close at once (B4)?
   **Recommendation: close at once** — matches probe 1 and today's
   `Discard` fast path.

## Agent log

- 2026-11-07 (UTC) · opencode-go/muse-spark-1.3-contributor + T098g draft · Drafted the blind contract (b) reserving min(200 ms, budget/4) of the close budget for CONNECTION_CLOSE across client Flush, server shutdown and single close, with per-connection close issue and four regression tests.
