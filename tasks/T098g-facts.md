# T098g fact sheet — lost QUIC CONNECTION_CLOSE (D43a)

Facts only. Paths are relative to the repository root on branch
`task/T098g-facts-b1`. Line numbers are current.

## 1. `tasks/DECISIONS-D43.md` and `tasks/T098.md` §"T098f measurement"

Measured failure (D43): Master CI failed once after PR #78 merged in QUIC test
`ordering_preserved_under_concurrent_traffic` (case 13); a rerun passed
(`tasks/DECISIONS-D43.md:7`). Cause is in the product:

> When a client closes with `Flush`, it gets one time budget, `close_flush_timeout_ms` (2 s by default). In that time it sends its queued frames, then it sends CONNECTION_CLOSE, the "goodbye" packet that tells the server the connection is over. (`tasks/DECISIONS-D43.md:9`)

> If sending the queued frames uses the whole budget, the client's network thread shuts down before the goodbye packet goes out (`crates/crpg-net-quic/src/client.rs:462` sets the budget, `:680-685` waits on the same, already expired, deadline). (`tasks/DECISIONS-D43.md:10`)

> The server never hears the goodbye. It only learns the client is gone when the idle timeout fires, 60 s later. The test waits 30 s, so it fails. (`tasks/DECISIONS-D43.md:11`)

> The worker reproduced this on purpose: 128 queued frames and a `Flush` close gave the server `IdleTimeout` at 60.0 s instead of the client's normal close. (`tasks/DECISIONS-D43.md:13`)

D43 also notes `server.shutdown` uses the same one-shared-deadline shape
(`tasks/DECISIONS-D43.md:15`) and cites
`tasks/T023.md` lines 379–381 and 494–496.

T098f measurement (`tasks/T098.md:310-388`): case 13 failed on Windows `check`
in master run 41544530333 after PR #78 at
`crates/crpg-net-quic/tests/support/quic_rig.rs:170` ("deadline: server frame");
the run took 40 s (`tasks/T098.md:312-318`). Environment: Windows 10.0.26300,
`x86_64-pc-windows-msvc`, `rustc 1.98.0`, debug profile, 16 logical CPUs;
contention = N copies of the built `quic` binary (`tasks/T098.md:322-325`).
Client-side drain waiting (T098b) is already in place
(`tasks/T098.md:319-320`).

Reproduction probe 2 (`tasks/T098.md:348-355`): default v1 limits; one client
queues 128 × 4,096-byte frames; server drains 10 then stops; client closes
`(Normal, Flush)`. Client spent the whole 2,000 ms flush budget, discarded
48 frames, and `close_quinn` logged the close request. Server then drained its
queue (`reader_parked: false`, inbound/outbound 0) and never saw
`Peer { code: 0 }`: at 60.0 s it published `Closed(IdleTimeout)`.
`v1.idle_timeout_ms` is 60,000 (`crates/crpg-net-quic/src/limits.rs:27`).
Probe 1 (`tasks/T098.md:344-347`): `close_flush_timeout_ms = 0` gave the
server `Closed(Peer { code: 0 })` 1.7 µs later; a spent budget alone is not
enough.

Mechanism (`tasks/T098.md:357-370`, paraphrased; T098's own wording uses
`src/...` paths from its tree, current equivalents are
`crates/crpg-net-quic/src/...`):

`QuicClient::stop` fixes `close_flush_timeout_ms` as one shared budget before
flushing: `let deadline = deadline_after(...)` at
`crates/crpg-net-quic/src/client.rs:462`, stored in
`Stop { code, deadline }` after the flush wait. `run_flush`
(`crates/crpg-net-quic/src/queue.rs:961`) may use the entire 2 s waiting for
the FIN ack; it then records the close, discards what is left and calls
`close_quinn` (queuing the CONNECTION_CLOSE). `client_main` then runs
`endpoint.close(...)` followed by
`timeout_at(Instant::from_std(stop.deadline), endpoint.wait_idle())`
(`crates/crpg-net-quic/src/client.rs:680-685`) with a deadline that has already
expired. The wait returns at once, the I/O thread ends and the runtime is
dropped before the queued CONNECTION_CLOSE is transmitted.

`frame_tx.connection_close` read 0 in both probes, including probe 1
(`tasks/T098.md:374-375`).

T098 cites these current code locations: `crates/crpg-net-quic/src/client.rs:462`,
`:680-685`; `crates/crpg-net-quic/src/queue.rs` `run_flush`, `pop_outbound`,
`discard_outbound`, `lane_writer` finish;
`crates/crpg-net-quic/src/limits.rs:27`, `:35`;
`crates/crpg-net-quic/tests/quic.rs:974`, `:1016-1058`, `:1074-1113`,
`:1078`, `:1110`, `:1111`.

## 2. `tasks/T023.md` passages on close

CloseMode (`tasks/T023.md:378-385`):

> ```text
> pub enum CloseMode {
>     /// Write every queued outbound frame, FIN, wait for the peer's ack up to
>     /// `close_flush_timeout_ms`, then CONNECTION_CLOSE.
>     Flush,
>     /// Discard queued outbound frames not yet handed to QUIC; close at once.
>     Discard,
> }
> ``` (`tasks/T023.md:378-385`)

CloseReason (`tasks/T023.md:387-401`): `Local(CloseCode)`, `Peer { code: u64 }`,
`IdleTimeout`, `Transport`, `Reset`, `EndpointFailed`.

CloseReport (`tasks/T023.md:463-467`, `:532`):

> ```text
> pub struct CloseReport { pub outbound_frames_discarded: usize, pub inbound_frames_discarded: usize }
> ``` (`tasks/T023.md:532`)

`server.shutdown` (`tasks/T023.md:489-496`):

> Graceful: stop accepting, close pending with `Shutdown`, flush accepted
> connections (`Flush`, one shared deadline), close all with `Shutdown`,
> wait for the endpoint to go idle (bounded by the same deadline), join.
> (`tasks/T023.md:489-492`)

> Abrupt: every connection closes with `Shutdown` in `Discard` mode, then the
> endpoint is given up to `close_flush_timeout_ms` to transmit the
> CONNECTION_CLOSE frames (`wait_idle`), then the thread is joined.
> (`tasks/T023.md:494-496`)

Idle timeout and keep-alive (`tasks/T023.md:812-852`): idle timeout v1 30,000
ms (revised as built to 60,000, `crates/crpg-net-quic/src/limits.rs:27`),
valid range `1,000..=60,000`; keep-alive v1 15,000 ms, range 0 (off) or
`100..idle_timeout_ms`; `close_flush_timeout_ms` v1 2,000, range `0..=2,000`
(`tasks/T023.md:821`, `:822`, `:826`; `crates/crpg-net-quic/src/limits.rs:27`,
`:29`, `:35`, `:126-131`, `:240-245`). Effective idle timeout is the minimum
of the two peers' values (`tasks/T023.md:852`).

Server observes peer close vs idle timeout (`tasks/T023.md:799-803`):

> quinn's `ConnectionError` maps as follows: `ApplicationClosed` →
> `Peer { code }`; `TimedOut` → `IdleTimeout`; `Reset` → `Reset`;
> `LocallyClosed` → `Local(code used)`; `TransportError`, `ConnectionClosed`,
> `VersionMismatch` and `CidsExhausted` → `Transport`. (`tasks/T023.md:800-803`)

Close codes 0–9 (`tasks/T023.md:783-799`); `Normal = 0`, `Shutdown = 1`.

As-built notes (`tasks/T023.md:1973-1985`): close is recorded first-wins and
becomes visible only once the lane reader has stopped adding frames; a parked
reader keeps waiting after a close and then reads what QUIC already buffered;
this is what makes "frames fully received before any close stay drainable"
hold for a peer's `Flush` close.

`QuicServer::close(.., Flush)` blocks for up to `close_flush_timeout_ms`
(`tasks/T023.md:1990-1993`); sync waits add a 1 s liveness guard
(`SYNC_GRACE`, `crates/crpg-net-quic/src/queue.rs:34`).

## 3. Client close path end to end

- `QuicClient::close` (`crates/crpg-net-quic/src/client.rs:447-455`): calls
  `stop`, then joins the I/O thread.
- `QuicClient::stop` (`crates/crpg-net-quic/src/client.rs:457-500`):
  - report initialised with zeros (`:458-461`);
  - budget computed `let deadline = deadline_after(self.limits.close_flush_timeout_ms);`
    at `crates/crpg-net-quic/src/client.rs:462`;
  - if the connection is still open and mode is `Flush` (`:464-468`,
    `:469`), sets `record.closing = Some(code)` and
    `record.flush = Some(FlushRequest { code, deadline, started: false })`
    (`:470-476`), notifies `record.wake.control` (`:477`);
  - waits on the condvar up to `guard_deadline = deadline + SYNC_GRACE`
    (`:479-488`) for `record.close_done || record.control_done`;
  - otherwise (closed or `Discard`) calls `state.record_close(LANE, CloseReason::Local(code))`
    (`:489-491`);
  - fills `report.outbound_frames_discarded = record.discarded.saturating_sub(before)`
    (`:493`) and `report.inbound_frames_discarded = record.held_inbound_frames()`
    (`:494`);
  - stores `state.stop = Some(Stop { code, deadline })` (`:496`);
  - notifies endpoint wake (`:498`).
- Flush execution: `lane_control` (`crates/crpg-net-quic/src/queue.rs:896-957`)
  takes `conn.flush` when `!started` and calls `run_flush`
  (`crates/crpg-net-quic/src/queue.rs:933`).
- `run_flush` (`crates/crpg-net-quic/src/queue.rs:961-1000`):
  - notifies writer (`:968`);
  - waits in a loop until `conn.writer_finished || conn.reason.is_some()` or
    `sleep_until(deadline)` fires (`:970-985`);
  - records the close with `state.record_close(id, CloseReason::Local(flush.code))`
    (`:988`), sets `conn.close_done = true` (`:990`);
  - calls `close_quinn(connection, flush.code)` (`:998`), which issues
    quinn's application CONNECTION_CLOSE (`crates/crpg-net-quic/src/queue.rs:107-109`).
  - `discard_outbound` runs inside `record_close`
    (`crates/crpg-net-quic/src/queue.rs:465-479`, `:449-460`), incrementing
    `record.discarded`.
- Writer drain: `lane_writer`
  (`crates/crpg-net-quic/src/queue.rs:844-892`) pops outbound frames via
  `pop_outbound` (`crates/crpg-net-quic/src/queue.rs:439-447`), writes each
  with `send.write_all` (`:868`); on empty queue while flushing calls
  `send.finish()` then `send.stopped().await` (`:877-881`) and sets
  `writer_finished = true` (`:887`).
- I/O thread exit (`crates/crpg-net-quic/src/client.rs:671-686`):
  - `race(lane_control(...), wait_stop(&shared))` (`:671-675`);
  - `endpoint.close(quinn::VarInt::from_u32(stop.code.as_u32()), stop.code.name().as_bytes())`
    (`:680-683`);
  - `tokio::time::timeout_at(tokio::time::Instant::from_std(stop.deadline), endpoint.wait_idle())`
    (`:684-685`);
  - thread joins in `close()` (`crates/crpg-net-quic/src/client.rs:451-453`).
- Drop path (`crates/crpg-net-quic/src/client.rs:519-529`): `stop(Normal, Discard)`,
  then join.

## 4. Server side `shutdown` end to end and peer-close vs idle timeout

`QuicServer::shutdown` (`crates/crpg-net-quic/src/server.rs:700-759`):

- report initialised with zeros (`:701-705`);
- `let deadline = deadline_after(self.limits.close_flush_timeout_ms);` (`:706`);
- locks state, sets `state.stopping = true` (`:708`);
- collects connection ids (`:709`);
- for each connection:
  - skips `Phase::Hello` or already-closing (`:715`);
  - increments `report.connections_closed` (`:718`);
  - pending: `record.quinn_close = Some(Shutdown)` and
    `state.record_close(id, CloseReason::Local(Shutdown))` (`:719-721`);
  - accepted: sets `record.closing = Some(Shutdown)` and
    `record.flush = Some(FlushRequest { code: Shutdown, deadline, started: false })`,
    notifies `record.wake.control`, records `(id, record.discarded)` in
    `flushing` (`:722-731`);
- waits on condvar up to `guard_deadline = deadline + SYNC_GRACE`
  (`:733-741`) until every flushing record has `close_done || control_done`;
- if the wait returns, adds per-connection discarded deltas to
  `report.outbound_frames_discarded` (`:743-745`), sums
  `held_inbound_frames` into `report.inbound_frames_discarded` (`:746-747`),
  sets `state.stop = Some(Stop { code: Shutdown, deadline })` (`:748-751`);
- notifies endpoint wake (`:754`), joins thread (`:755-757`).

`QuicServer::Drop` (`crates/crpg-net-quic/src/server.rs:765-783`): sets
`state.stopping = true` and `state.stop = Some(Stop { code: Shutdown, deadline })`
if none, notifies endpoint wake, joins thread. No flush of queued frames.

I/O thread exit (`crates/crpg-net-quic/src/server.rs:855-865`):
`close_quinn_endpoint(&endpoint, stop.code)` (`:862`),
`tokio::time::timeout_at(tokio::time::Instant::from_std(stop.deadline), endpoint.wait_idle())`
(`:863-864`).

Per-connection close via `QuicServer::close`
(`crates/crpg-net-quic/src/server.rs:596-640`): pending or `Discard` mode
records close and sets `record.quinn_close` (`:613-619`); accepted `Flush`
sets `record.flush` and waits on condvar up to `guard_deadline`
(`:621-639`), then returns discarded delta.

Peer close vs idle timeout:
- `reason_from` (`crates/crpg-net-quic/src/queue.rs:89-104`): maps
  `ConnectionError::ApplicationClosed` to `CloseReason::Peer { code }`,
  `ConnectionError::TimedOut` to `CloseReason::IdleTimeout`.
- `lane_control` (`crates/crpg-net-quic/src/queue.rs:936-944`): on
  `connection.closed()` returning, calls
  `state.record_close(id, reason_from(&error))` (`:940`).
- `ServerEvent::Closed { conn, reason }`
  (`crates/crpg-net-quic/src/server.rs:61-68`) is emitted by `try_publish`
  (`crates/crpg-net-quic/src/queue.rs:483-502`) once the reader stops adding
  frames. Idle timeout value comes from `ServerLimits.idle_timeout_ms`
  (`crates/crpg-net-quic/src/limits.rs:55`, `:27`, `:107-112`).

## 5. Config

Fields:
- `ServerLimits`: `idle_timeout_ms: u32` (`:55`), `keep_alive_ms: u32` (`:57`),
  `hello_timeout_ms: u32` (`:59`), `decision_timeout_ms: u32` (`:61`),
  `close_flush_timeout_ms: u32` (`:63`), `max_connections: usize` (`:65`),
  `inbound: QueueCaps` (`:67`), `outbound: QueueCaps` (`:69`),
  `stream_window_bytes` (`:73`), `connection_window_bytes` (`:76`),
  `send_window_bytes` (`:79`).
- `ClientLimits`: `idle_timeout_ms` (`:176`), `keep_alive_ms` (`:178`),
  `connect_timeout_ms` (`:180`), `close_flush_timeout_ms` (`:182`),
  `inbound_frames` (`:184`), `inbound_bytes` (`:186`),
  `outbound_frames` (`:188`), `outbound_bytes` (`:190`),
  `stream_window_bytes` (`:194`), `connection_window_bytes` (`:197`),
  `send_window_bytes` (`:200`).
- Defaults: `ServerLimits::v1()` (`:87-101`), `ClientLimits::v1()` (`:207-221`).
  `close_flush_timeout_ms` v1 = `V1_CLOSE_FLUSH_TIMEOUT_MS = 2_000`
  (`:35`, `:93`, `:212`); idle v1 = `V1_IDLE_TIMEOUT_MS = 60_000`
  (`:27`, `:89`, `:209`); keep-alive v1 = `V1_KEEP_ALIVE_MS = 15_000`
  (`:29`, `:90`, `:210`).
- Validation: `ServerLimits::validate` (`:105-169`), `ClientLimits::validate`
  (`:225-284`), `bounded` (`:340-353`), `keep_alive` (`:356-361`),
  `queue` (`:386-417`). `close_flush_timeout_ms`: floor 0, ceiling v1 2,000
  (`:126-131`, `:240-245`). Idle: floor 1,000, ceiling 60,000
  (`:107-112`, `:227-232`). Keep-alive: 0 or 100..idle
  (`:113`, `:233`). Validation order declaration order; first failure wins;
  `InvalidLimits` before `LoosensPolicy` (`:103-104`, `:223-224`, `:339-353`).

## 6. Existing tests

Close/flush/shutdown/idle/case 13 (file, test, line, assertion, helper):

- `crates/crpg-net-quic/tests/quic.rs:973` `#[test]`
  `ordering_preserved_under_concurrent_traffic` (`:974`): 4 clients, 1,000
  frames each way; client drains `outbound_frames == 0` then `close(Normal, Flush)`
  (`:1111`); asserts `outbound_frames_discarded == 0` (`:1116`); server
  expects 1,000 in order then `Closed(Peer { code: 0 })` with no extras
  (`:1064-1070`). Helpers: `client_traffic`, `bind_server`, `connect_async`,
  `next_hello`, `server_recv`, `cycling_size`, `pattern`, `until`-style waits.
- `crates/crpg-net-quic/tests/quic.rs:1458` `#[test]`
  `graceful_close_flushes_and_reports_codes` (`:1459`): server `close(Normal, Flush)`
  with 3 queued → client gets 3 then `Peer { code: 0 }`, report 0 discarded
  (`:1467-1475`); server `close(SessionFenced, Discard)` with N queued →
  in-order prefix + `Peer { code: 9 }`, `k + discarded <= N`
  (`:1494-1520`); client `close(Normal, Flush)` after 3 sends → server gets 3
  then `Closed(Peer { code: 0 })` (`:1526-1546`).
- `crates/crpg-net-quic/tests/quic.rs:1548` `#[test]`
  `server_shutdown_and_drop_close_every_connection` (`:1549`): 3 clients with
  5 queued frames each; `shutdown()` → each gets 5 then `Peer { code: 1 }`,
  `connections_closed == 3`, `outbound_frames_discarded == 0`
  (`:1559-1568`); dropped server without shutdown → client sees `Peer { code: 1 }`,
  not idle timeout (`:1570-1575`).
- `crates/crpg-net-quic/tests/quic.rs:1577` `#[test]`
  `peer_abrupt_close_keeps_complete_frames_only` (`:1578`): 2 complete frames
  then half of third, `close(0x99)` → `Closed{Peer{code:0x99}}`, try_recv
  yields 2 then closed (`:1603-1615`).
- `crates/crpg-net-quic/tests/quic.rs:1617` `#[test]`
  `idle_timeout_closes_silent_peer` (`:1618`): idle 1,000 / keep-alive 300
  through `UdpRelay`; blackhole → server `Closed{IdleTimeout}` and client
  `Err(Closed(IdleTimeout))` (`:1645-1649`); positive control with keep-alives
  past idle period (`:1633-1644`). Helpers: `UdpRelay::start`, `RelayConfig`,
  `connect_via_relay`.
- `crates/crpg-net-quic/tests/quic.rs:752` `#[test]`
  `hello_timeout_closes_silent_client` (`:753`): no hello within 5 s →
  `Closed{Local(HelloTimeout)}` etc.
- `crates/crpg-net-quic/tests/quic.rs:2228` `#[test]`
  `discard_close_reaches_peer_when_congestion_blocked` (`:2229`).
- `crates/crpg-net-quic/tests/quic.rs:2320` `#[test]`
  `refusal_reaches_client_when_closed_is_polled_at_once` (`:2321`).
- `crates/crpg-net-quic/tests/quic.rs:2345` `#[test]`
  `pending_discard_close_reaches_client_when_polled_at_once` (`:2346`).
- `crates/crpg-net-quic/tests/quic.rs:2362` `#[test]`
  `closed_record_kept_until_try_recv_reports_closed` (`:2363`).
- `crates/crpg-net-quic/tests/quic.rs:2405` `#[test]`
  `closed_without_held_frames_forgotten_on_poll` (`:2406`).
- `crates/crpg-net-quic/tests/quic_gaps.rs:17` `#[test]`
  `inbound_parked_frame_delivered_before_peer_close` (`:19`): parked inbound
  frame delivered before peer close; `Closed` not published while parked
  (`:37-57`).
- `crates/crpg-net-quic/tests/quic_gaps.rs:228` `#[test]`
  `graceful_close_exceeding_flush_timeout_reports_discard` (`:230`): 100 ms
  flush budget, non-reading client, many queued frames →
  `outbound_frames_discarded > 0` (`:247-254`).
- `crates/crpg-net-quic/tests/quic_gaps.rs:258` `#[test]`
  `shutdown_with_parked_inbound_frames_reports_discarded` (`:260`): 2 queued +
  1 parked inbound → `inbound_frames_discarded == 3` (`:274-282`).
- `crates/crpg-net-quic/tests/channel.rs:929` `#[test]`
  `v2_reader_close_codes` (`:930`).
- `crates/crpg-net-quic/tests/channel.rs:1716` `#[test]`
  `v2_close_keeps_tagged_frames_drainable` (`:1717`).
- `crates/crpg-net-quic/tests/channel.rs:1995` `#[test]`
  `resume_opening_close_codes` (`:1996`).
- `crates/crpg-net-quic/tests/channel.rs:2274` `#[test]`
  `resume_answer_then_close_reaches_client` (`:2275`).
- `crates/crpg-net-quic/tests/channel.rs:2544` `#[test]`
  `v2_client_reader_close_codes` (`:2545`).

Helpers/harness: `bind_server`, `client_config`, `connect_accepted`,
`connect_async`, `next_hello`, `next_closed`, `next_event`, `server_recv`,
`client_recv`, `until`, `raw_lane`/`raw_connect`, `UdpRelay`, `RelayConfig`,
`pattern`, `epoch`, `credential`, `hello`, plus `SYNC_GRACE` and the 30 s
`DEADLINE` guard. `crates/crpg-net-quic/tests/support/quic_rig.rs:1-199`.

## 7. Timing constants

- `crates/crpg-net-quic/tests/support/quic_rig.rs:33`: `DEADLINE = 30 s`
  guard.
- `crates/crpg-net-quic/src/queue.rs:34`: `SYNC_GRACE = 1 s`.
- `crates/crpg-net-quic/src/limits.rs:27`: idle timeout v1 = 60,000 ms.
- `crates/crpg-net-quic/src/limits.rs:29`: keep-alive v1 = 15,000 ms.
- `crates/crpg-net-quic/src/limits.rs:31`: hello timeout v1 = 5,000 ms.
- `crates/crpg-net-quic/src/limits.rs:32`: decision timeout v1 = 5,000 ms.
- `crates/crpg-net-quic/src/limits.rs:33`: connect timeout v1 = 10,000 ms.
- `crates/crpg-net-quic/src/limits.rs:35`: close flush timeout v1 = 2,000 ms.
- `crates/crpg-net-quic/tests/support/quic_rig.rs:544`: `HOLD = 2 ms`
  relay reorder hold.
- `crates/crpg-net-quic/tests/support/quic_rig.rs:581`: relay read timeout = 2 ms.
- `crates/crpg-net-quic/tests/support/quic_rig.rs:111`: server wait 20 ms in
  `next_event`/`next_hello`/`next_closed`.
- `crates/crpg-net-quic/tests/support/quic_rig.rs:171`: server wait 20 ms in
  `server_recv`.
- `crates/crpg-net-quic/tests/support/quic_rig.rs:185`: client wait 20 ms in
  `client_recv`.
- `crates/crpg-net-quic/tests/quic_gaps.rs:39`: 100 ms loop in
  `inbound_parked_frame_delivered_before_peer_close`.
- `crates/crpg-net-quic/tests/quic.rs:1056`, `:1107`, `:1113`: 1 ms waits in
  case 13 traffic/drain loops.
- Test-only config overrides: `close_flush_timeout_ms = 100`
  (`crates/crpg-net-quic/tests/quic_gaps.rs:232`); idle/keep-alive 1,000/300
  (`crates/crpg-net-quic/tests/quic.rs:1619-1624`); connect timeout 100
  (`crates/crpg-net-quic/tests/quic_gaps.rs:222`); 1-frame/65,536-byte inbound
  queues in `graceful_close_exceeding_flush_timeout_reports_discard`
  (`crates/crpg-net-quic/tests/quic_gaps.rs:236-239`).

## 8. Architecture doc and crate AGENTS

`docs/architecture/crpg-net-quic.md`:
- `:31` T023v vendors quinn-proto with upstream's `CONNECTION_CLOSE` fix.
- `:60` module flow: `server.rs` `QuicServer: bind, accept loop, ... shutdown/Drop, ServerLane`.
- `:71` writer task: `outbound queue → SendStream (Flush: drain, FIN, ack)`.
- `:72` control task: `requested closes, Flush, connection.closed() → reason`.
- `:88` sync waiters (`wait`, `close(Flush)`, `shutdown`) use a `Condvar`.
- `:113` `| Close flush | 2 s | 2 s |`.
- `:124-128` after a close, data QUIC already received stays readable; `Closed`
  published only once the reader has nothing more; a host that stops draining
  a closed connection holds back that connection's `Closed` event.
- `:166` `QuicServer`: events, accept/refuse, try_recv/try_send, close modes,
  stats, totals, shutdown.
- `:180-183` close codes reach the peer under a full congestion window
  (T023v).

`crates/crpg-net-quic/AGENTS.md`:
- `:18` public surface includes `ShutdownReport`.
- `:51` host side stays synchronous, except `wait`, `connect`, `close`,
  `shutdown` and `Drop`.
- `:133` a parked reader must keep waiting after a close; otherwise a peer's
  `Flush` close would lose data.

## 9. Mentions of T098g, D43 or "case 13"

One line each (path:line, quote):

- `tasks/DECISIONS-D43.md:1` `# Decision D43 — a QUIC close that never reaches the peer (T098f)`
- `tasks/DECISIONS-D43.md:3` Status answered D43a (a), reserve a slice; QUIC case 13 stays a known flake.
- `tasks/DECISIONS-D43.md:7` Master CI failed once after PR #78 in `ordering_preserved_under_concurrent_traffic` (case 13).
- `tasks/DECISIONS-D43.md:17` `## D43a — What the close budget covers`
- `tasks/DECISIONS-D43.md:23` manager has a short contract (T098g, one crate: `crpg-net-quic`).
- `tasks/DECISIONS-D43.md:27` Drafted D43 from the T098f measurement, lost CONNECTION_CLOSE when Flush spends whole budget.
- `tasks/DECISIONS-D43.md:28` Recorded user's answer (a), keep close bound and reserve slice for CONNECTION_CLOSE.
- `tasks/T098.md:2` Find out why `ordering_preserved_under_concurrent_traffic` (T023 case 13) failed.
- `tasks/T098.md:124` `| case 13 alone | 20 | 0 | — |`
- `tasks/T098.md:126` `| case 13, 4× parallel contention | 80 | 3 | outbound_frames_discarded = 46, 68, 68 |`
- `tasks/T098.md:331` `| case 13 alone (cargo test, filtered) | 20 | 0 | — (3.76–5.33 s each) |`
- `tasks/T098.md:392` Recorded the one Windows CI failure of QUIC case 13.
- `tasks/T098.md:393` Added the second failure of case 13, on Linux.
- `tasks/T098.md:394` Added three more failures of case 13 on PRs #51 and #52.
- `tasks/T098.md:397` Measured case 13's final close wait and isolated with two probes.
- `tasks/T023.md:2005` case 13's sizes cycle over `1..=cap` with prime stride (7,919).
- `tasks/T023b.md:1892` burst, and case 13 is covered above.
- `tasks/T023b.md:2033` Lane-0 recovery stays a retry by seq, which case 13's resend of seq 81 ...
- `tasks/T023b.md:2041` would contradict case 13, which requires `RateLimited` drops.
- `tasks/T023b.md:2074` The case 13 guard is approved as a clarification.
- `tasks/T023b.md:2354` Recovery is a client retry by seq (case 13).
- `tasks/T024q.md:968` case (case 13 of `quic.rs` is the T098 flake; nothing here copies its ...
- `tasks/T024q.md:1007` as opaque, and case 13 uses non-resume bytes on purpose.
- `tasks/T024q-facts.md:239` `ordering_preserved_under_concurrent_traffic` (case 13, `quic.rs:973`).
- `tasks/T024h.md:340` `next_ordinal + count < u64::MAX` (S§7 step 5; S§11 case 13).
- `tasks/T024h.md:1196` and new unit case 13; S§6.2 now states where each ordinal check sits.
- `tasks/T025a.md:1737` resolved ... pinned in case 13.
- `tasks/BACKLOG.md:248` T098f measurement only: case 13 failure is product-side.
- `tasks/BACKLOG.md:249` T098g fix the lost close per D43a.
- `tasks/BACKLOG.md:571` Added T098f, T098g and T051 rows.
- `docs/PROJECT_STATE.md:6` In flight: T098f measurement, D43 (answered: (a)).
- `docs/PROJECT_STATE.md:7` T098g contract per D43a ... then implementation.
- `docs/PROJECT_STATE.md:712` Updated handover CI line with three PR failures of case 13.
- `docs/AGENT_TEAM_PLAN.md:191` (T098, QUIC case 13) counts the same way: rerun, do not charge.
- `docs/agents/RESEARCH-2026-10-16.md:26` T098, QUIC case 13 adopted as a rule.
- `docs/agents/MANAGER.md:71` failure in test with open flake record (T098, QUIC case 13).
- `crates/crpg-net-quic/tests/quic.rs:1073` `/// One client's half of case 13: sends 1..=1,000 ...`
- `tools/tests/fixtures/brief/T023b.md:1892` burst, and case 13 is covered above.
- `tools/tests/fixtures/brief/T023b.md:2033` Lane-0 recovery stays a retry by seq, which case 13's resend of seq 81 ...
- `tools/tests/fixtures/brief/T023b.md:2041` would contradict case 13, which requires `RateLimited` drops.
- `tools/tests/fixtures/brief/T023b.md:2074` The case 13 guard is approved as a clarification.
- `tools/tests/fixtures/brief/T023b.md:2354` Recovery is a client retry by seq (case 13).

## Agent log

- 2026-11-07 (UTC) · opencode-go/beam-free + T098g facts · Fact sheet for the T098g contract: repository facts on the lost QUIC CONNECTION_CLOSE, close paths, config, tests, and mentions.