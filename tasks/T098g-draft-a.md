# T098g — lost QUIC `CONNECTION_CLOSE` (D43a)

## Exact contract

Status: **blind contract draft; not approved**. Sole implementation crate:
`crpg-net-quic`. This contract implements D43a (a): keep the existing
`close_flush_timeout_ms` total budget and reserve part of it for sending the
application `CONNECTION_CLOSE`, rather than extending close time. The measured
failure is a client `Flush` that spends the full budget before the endpoint's
wait on that same expired deadline; the peer then observes idle timeout
instead (`tasks/T098.md:348-369`; `crates/crpg-net-quic/src/client.rs:462,680-685`).

### Decision

**D1 — Reserve policy (recommendation: fixed cap plus a fraction).** Define
`B = close_flush_timeout_ms`, `R = min(200 ms, floor(B / 4))`, and
`F = B - R`. `F` is the maximum flush/FIN-ack phase; `R` is the remaining
connection-close transmit window. This is a private implementation constant,
not a config field. At the default 2,000 ms budget, reserve 200 ms and flush
for at most 1,800 ms. At 100 ms, reserve 25 ms and flush for at most 75 ms.
At 1–3 ms, integer rounding gives no positive reserve; at 0, both phases have
zero wait and the close request is best effort. This cannot promise delivery
with a zero total budget. The current public client/server limit is
`close_flush_timeout_ms` (`crates/crpg-net-quic/src/limits.rs:62-63,181-182`),
valid from 0 to 2,000 ms (`crates/crpg-net-quic/src/limits.rs:126-131,240-245`).

Options and consequences:

| Option | Consequence | Recommendation |
|---|---|---|
| Fixed `min(200 ms, B)` reserve | Predictable at default; for `B <= 200 ms`, no flush time remains; zero still cannot reserve positive time. | Do not choose: overly penalizes small budgets. |
| Fraction, e.g. `B / 10` | Preserves 90% for flush; at default also yields 200 ms; reserve scales down to zero with budget. | Viable; leaves no fixed minimum at smaller budgets. |
| `min(200 ms, floor(B / 4))` | Default is 200 ms; smaller budgets preserve at least 75% for flush; 0–3 ms has no positive reserve. | **Choose**: explicit cap, bounded flush loss, no public option. |
| Add reserve to both public limits | Lets callers tune it, but changes the public `ClientLimits`/`ServerLimits` struct surface and their construction sites. | Reject unless tuning is required; prefer a private constant. |

**D2 — Close sequence.** Let `D = close_start + B` be the unchanged absolute
total deadline and `C = close_start + F` the new flush cutoff. Use monotonic
`Instant`s and saturating/checked arithmetic consistent with
`deadline_after` (`crates/crpg-net-quic/src/queue.rs:58-63`).

1. For an open client `Flush`, store `C` in its flush request and notify its
   control task. The writer continues to take queued frames in FIFO order and
   write them; once the queue is empty it sends FIN and waits for the peer's
   acknowledgement (`crates/crpg-net-quic/src/client.rs:462-478`;
   `crates/crpg-net-quic/src/queue.rs:842-892`).
2. Stop waiting when the writer finishes, the connection is already closed,
   or `C` expires. At that point record `Local(code)` first-wins, discard only
   frames still queued in this crate, then call `connection.close` for
   `CONNECTION_CLOSE` (`crates/crpg-net-quic/src/queue.rs:465-478,961-999,106-109`).
   Do not wait for the FIN acknowledgement past C.
3. Signal the synchronous caller that this connection's flush/close request
   has been issued. The client then requests endpoint close and waits on
   `endpoint.wait_idle()` only until D; it must not wait on C, nor create a
   second budget. The I/O thread exits when that wait completes or D expires;
   `close()` joins it (`crates/crpg-net-quic/src/client.rs:447-455,479-499,
   671-686`). Retain the existing `SYNC_GRACE` liveness guard for the
   synchronous condition-variable wait, not as extra network-transmit time
   (`crates/crpg-net-quic/src/queue.rs:32-34`).
4. If the flush finishes before C, issue the close immediately and allow the
   endpoint to become idle; do not intentionally sleep until C. If B is zero,
   issue the close immediately and allow no endpoint wait. A non-zero reserve
   is best effort, not an assertion that a packet must reach the peer.

Today, `stop` creates D before the flush and stores it in `FlushRequest`
(`crates/crpg-net-quic/src/client.rs:457-478`); `run_flush` waits on that same
deadline and calls `close_quinn` only afterward (`crates/crpg-net-quic/src/queue.rs:961-999`).
Then `client_main` passes D to `timeout_at(endpoint.wait_idle())`
(`crates/crpg-net-quic/src/client.rs:680-685`). The change splits the flush
cutoff from the existing endpoint deadline; it does not extend D.

**D3 — `server.shutdown`.** Compute one D and one C at shutdown entry and
apply the same C to every accepted connection's `FlushRequest`; start all
flushes before waiting, not one connection at a time. Pending connections
still close immediately with `Shutdown`; hello-stage and already-closing
records remain skipped. Each accepted connection follows D2. Once every
accepted connection has issued its close (or its control task has ended),
sum discarded deltas and held inbound frames, set `Stop { code: Shutdown,
deadline: D }`, close the endpoint, wait idle only through D, and join.
Current code already has the one shared D, schedules all accepted flushes in
the connection loop, waits for all `close_done || control_done`, and uses D
for `wait_idle` (`crates/crpg-net-quic/src/server.rs:697-758,834-865`).
This contract leaves `QuicServer::close(Flush)` unchanged: it performs a
per-connection flush but does not stop the server endpoint or start its
endpoint-wide `wait_idle` deadline (`crates/crpg-net-quic/src/server.rs:621-639,834-865`).

No one slow connection receives a sequential flush budget or consumes another
connection's flush allowance: all use the same absolute C and their async
control/writer tasks run independently (`crates/crpg-net-quic/src/queue.rs:842-892,896-957`).
The endpoint wait and runtime are shared, however, so scheduling, congestion,
or one connection keeping the endpoint non-idle can use the shared remaining
time. D43a guarantees a bounded *attempt window*, not per-connection delivery
or fairness under congestion. The shutdown test below includes a blocked
connection and a healthy control connection to guard against starvation.

**D4 — Reports and public surface.** Keep `CloseReport` and
`ShutdownReport` fields/variants unchanged; add or rename none. The report's
`outbound_frames_discarded` remains the number still in the crate's outbound
queue when `record_close` discards it, measured as the per-operation delta.
Frames already popped and handed to QUIC do not count; `record_close` clears
the queued frames and increments `discarded`
(`crates/crpg-net-quic/src/queue.rs:437-460,465-478`); client and server
reports subtract the pre-close count (`crates/crpg-net-quic/src/client.rs:468,492-495`;
`crates/crpg-net-quic/src/server.rs:608-639,730-747`). Reserve-induced earlier
cutoff may increase this count, never reduce it by excluding queued frames.
Inbound report semantics do not change. Existing report definitions are
`crates/crpg-net-quic/src/client.rs:94-99` and
`crates/crpg-net-quic/src/server.rs:188-194`.

### Interface

No public Rust API, config, variant, wire byte, close code, or dependency is
added or renamed. The private timing helper/constant may be introduced in
`crpg-net-quic`; no other crate or golden changes. Current `CloseMode::Flush`
documents drain, FIN, acknowledgement up to `close_flush_timeout_ms`, then
close, while `Discard` closes at once (`crates/crpg-net-quic/src/close.rs:74-82`).
Amend only the `Flush` budget wording for `QuicClient::close` and
`server.shutdown` to say that the flush phase ends at `B - R` and the close
gets the remaining `R` within total B. `QuicServer::close(Flush)` retains its
current full flush budget because its endpoint remains live; `Discard` is
unchanged.

### Behaviour

1. **Bounded budget:** flush ends no later than C; close transmission is
   attempted in the remaining reserve; total endpoint waiting ends by D.
   `B=0` is immediate/best-effort, not guaranteed delivery. (Tests 1, 2, 5.)
2. **Client `Flush`:** preserve FIFO frames handed to QUIC, finish/await FIN
   only through C, record local close and discard queued remainder, issue
   `CONNECTION_CLOSE`, then endpoint-wait through D and exit. Early completion
   closes early. (Tests 1, 3, 4.)
3. **Server shutdown:** apply the same C and D concurrently to every accepted
   connection; close pending connections immediately; the report counts
   discarded queued frames; one slow connection gets no sequential priority
   or extra budget. (Tests 2, 4.)
4. **Reports:** `outbound_frames_discarded` counts only frames still queued at
   the close cutoff; retain the existing fields and inbound accounting.
   (Tests 1–4.)
5. **Other close modes:** `Discard`, Drop, close codes, first-wins publication,
   and already-received frames remaining drainable do not change. (Test 6 and
   the existing close suite.)
6. **Concurrent ordering:** do not change or weaken case 13's four clients,
   1,000 ordered frames each way, no extras, peer code 0, and zero discarded
   outbound frames. Its success remains required. (`crates/crpg-net-quic/tests/quic.rs:973-1070,1110-1117`.)
7. **No protocol drift:** no wire/API/dependency/golden changes. (Review and
   existing public-surface, wire, and golden gates.)

### Tests

Add real loopback integration regressions in
`crates/crpg-net-quic/tests/quic.rs` or
`crates/crpg-net-quic/tests/quic_gaps.rs`; no sleeps as timing oracles. Use
the existing rig's 30 s waits only as failure guards
(`crates/crpg-net-quic/tests/support/quic_rig.rs:32-33,93-112`). The default
idle timeout is 60,000 ms
(`crates/crpg-net-quic/src/limits.rs:27,87-100,207-221`); for
peer-close assertions use a generous 25 s upper bound measured from just
before initiating close, leaving a wide CI margin while distinguishing a
`Peer` close from idle timeout. Poll through `wait` in short intervals, with
the deadline as a guard. Do not use `frame_tx.connection_close` as an oracle:
the measurement found it read zero both when close arrived and when it did not
(`tasks/T098.md:374-375`).

1. **`client_flush_backlog_reaches_peer_before_idle_timeout` (D2/B1–B2).**
   Connect with v1 limits; queue 128 × 4,096-byte client frames; have the
   server receive exactly 10 then stop draining. Call client
   `close(Normal, Flush)` with the default 2 s budget. Assert the server's
   `Closed` event is `Peer { code: 0 }`, and the close/error observed after
   queued inbound frames are drained is also peer code 0; assert elapsed time
   from close start is `<25 s`. Assert
   `1 <= report.outbound_frames_discarded <= 128`, and that delivered frames
   plus reported discards do not exceed 128. This setup matches the measured
   128-frame reproduction (`tasks/T098.md:348-355`). **Baseline control:** run
   this exact case against the pre-fix code; it must miss the 25 s bound (the
   measured peer instead became `IdleTimeout` at 60 s), proving the test
   detects the product defect, not a test-only wait change.
2. **`shutdown_reserves_close_for_each_connection` (D3/B1/B3).** Use two
   accepted clients. Set the slow client's inbound queue to 1 frame / 65,536
   bytes and its stream and connection windows to 65,540 bytes each; queue a
   full server-side backlog (32 × 65,536-byte frames); keep the control
   client's outbound queue empty. Call
   `server.shutdown()`. Assert `connections_closed == 2`, both clients observe
   `Peer { code: 1 }` (the server's `Shutdown`) within `<25 s` of shutdown
   start, and the control connection is not delayed to its idle timeout.
   Assert `0 < outbound_frames_discarded <= successfully_queued_frames` for
   the deliberately stalled connection's backlog, with total report bounded
   by frames successfully queued on both connections; assert
   `inbound_frames_discarded == 0`. Drain any received frames before checking
   each client's close error. The current shutdown gives every flush the
   whole shared D and then waits on that same expired D
    (`crates/crpg-net-quic/src/server.rs:706-751,862-865`), so the saturated
    slow path is the pre-fix control; use the same 25 s peer-close assertion.
3. **`flush_that_finishes_early_does_not_wait_for_reserve` (D2/B2).** Queue a
   small known number of frames on each side in separate subcases, close with
   `Flush`, and assert all frames arrive in order, zero are reported
   discarded, and the peer sees the correct `Peer` code. Use a generous
   `<25 s` outer bound, not a tight assertion that the reserve was consumed.
   This guards against implementing the reserve as a fixed sleep.
4. **`discard_report_is_the_unhanded_queue_delta` (D4/B4).** In the backlog
   cases, bound the report by frames accepted into the local queue and require
   a positive client (and stalled server) discard count. Also retain the
   successful small-flush assertion of zero. Frames delivered or handed to
   QUIC are not counted as discarded; no new report field is expected.
5. **`close_reserve_arithmetic_boundaries` (D1/B1).** Unit-test the private
   budget split exactly: 2,000 ms → (F=1,800, R=200); 100 ms → (75,25);
   3 ms → (3,0); 0 ms → (0,0). Verify `F + R == B`, both are nonnegative,
   and the close deadline remains D. These deterministic arithmetic cases
   need no wall-clock timing.
6. **Existing close-mode and ordering controls (B5–B7).** Keep all current
   assertions in `graceful_close_flushes_and_reports_codes`,
   `server_shutdown_and_drop_close_every_connection`, and especially
   `ordering_preserved_under_concurrent_traffic` unchanged. In particular do
   not relax its exact 1,000-frame order/no-extras/peer-code/zero-discard
    expectations
    (`crates/crpg-net-quic/tests/quic.rs:1458-1575,973-1070,1110-1117`). Existing
   `Discard`/idle-timeout and parked-reader tests remain in the acceptance
   suite.

Keep the existing `DEADLINE` guard; timing assertions use `<25 s` versus the
60 s idle timeout, not milliseconds or the measured 1.7 μs event. The rig's
20 ms polling waits are pacing/guard mechanics, not the assertion oracle
(`crates/crpg-net-quic/tests/support/quic_rig.rs:103-112,161-186`).

### Definition of done

One crate only, no new dependency. Update `docs/architecture/crpg-net-quic.md`
with the flush/reserve/endpoint timeline and unchanged total budget;
`crates/crpg-net-quic/AGENTS.md` with the new close/report invariants and test
traps; and add a dated amendment note under the close contract in
`tasks/T023.md`. Sign each edited documentation file in that file's agent log.

Run the crate gates with `-j 2`, and run on native Windows/MSVC and genuine
Linux/GNU (the crate's agent contract requires both;
`crates/crpg-net-quic/AGENTS.md:89-113`):

```text
cargo fmt --all -- --check
cargo clippy -p crpg-net-quic --all-targets --locked -j 2 -- -D warnings
cargo test -p crpg-net-quic --test quic --locked -j 2
cargo test -p crpg-net-quic --test quic_gaps --locked -j 2
cargo test -p crpg-net-quic --test channel --locked -j 2
cargo test -p crpg-net --locked -j 2
cargo tree -p crpg-net -e normal,build --target all
cargo test --workspace --locked -j 2
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
cargo deny check
git diff --check
```

Also pass the `crpg-net-quic/AGENTS.md` scope grep; it must print only the
crate-root `#![forbid(unsafe_code)]` line. **Size budget:** no more than 100
production LOC, 250 test LOC, and 1,000 added bytes to each of the two crate
docs and the T023 amendment; this contract remains below its 25,000-byte
draft limit.

### Out of scope

Changing any other crate, QUIC transport parameters, idle timeout, connection
codes, close modes other than the `Flush` budget split, endpoint architecture,
thread/runtime model, server shutdown policy, or the T023 contract beyond its
close-budget wording and dated amendment. No attempt to repair case 13 by
loosening its assertions or waits.

### What must not change

- Wire format, framing, ALPN, protocol bytes, close codes or dependency graph.
- T023 semantics except clarifying how `Flush` shares its existing close
  budget; no other T023 wording or behavior is amended.
- `Discard`, Drop, idle-timeout defaults/ranges, unrelated test or timeout
  behavior, and any golden.
- Case 13's existing order, count, close-reason and discard assertions.

### Routing and size budget

Route implementation to one `crpg-net-quic` worker after this draft is
adjudicated with the independent draft. No code is authorized by this draft.
Implementation must not add dependencies or modify files outside the crate
and the three documentation locations listed above. Contract cap: 25,000
bytes; recommended implementation diff cap: 100 production LOC + 250 test
LOC + at most 1,000 bytes per named documentation amendment.

## Unverified quinn details

- The registry source for `quinn` 0.11.12 was not readable in this session and
  the repository's vendored copy is `quinn-proto`, not quinn's endpoint crate.
  Therefore the precise `Endpoint::close`/`wait_idle` packet-scheduling and
  completion guarantees are **unverified** here. The proposed contract treats
  `wait_idle` as a bounded best-effort transmission window; implementation
  must verify its exact pinned API behavior before relying on stronger claims.

## Open questions for the user

1. Approve `R = min(200 ms, floor(B/4))` as the private reserve policy?
   **Recommendation: yes**; it gives a 200 ms default reserve without taking
   more than one quarter from small positive budgets.
2. Accept that a zero budget (and 1–3 ms after integer rounding) cannot provide
   a positive reserved window or guarantee peer delivery? **Recommendation:
   yes**; preserve the existing valid zero limit and define the close as
   immediate, best effort. If delivery is required at zero, the budget rule
   itself must change.
3. Is `<25 s` against the 60 s idle timeout acceptable for both backlog
   regressions? **Recommendation: yes**; it leaves 35 s separation from the
   idle event and is far less brittle than tight wall-clock limits.
4. Approve the no-public-field/no-new-dependency constraint and the 100/250
   LOC implementation budget? **Recommendation: yes**; the reserve is a
   policy detail, not caller configuration.
5. Confirm that per-connection `QuicServer::close(Flush)` keeps its full flush
   allowance? **Recommendation: yes**; it does not tear down the live server
   endpoint, unlike the two shared-deadline paths this fix targets.

## Agent log

- 2026-11-07 (UTC) · opencode-go/gpt-6-luna + T098g draft · Drafted the D43a contract from the measured code paths and repository evidence, separating the total deadline from its flush cutoff while retaining API, wire, and close-mode invariants for independent adjudication.
