# Decision D43 — a QUIC close that never reaches the peer (T098f)

Status: **Answered by the user, 2026-11-03: D43a (a), reserve a slice for the goodbye so a close still finishes within the budget.** Drafted by the manager in session 35 (2026-11-03). Nothing else waits on it: T048 (Godot scene sync) is in progress separately. Until it is fixed, QUIC case 13 stays a known flake (rerun, do not charge the worker).

## What happened

Master CI failed once after PR #78 merged, in the QUIC test `ordering_preserved_under_concurrent_traffic` (case 13). A rerun passed. T098f measured it (`tasks/T098.md` §"T098f measurement"). The cause is in the product, not the test:

- When a client closes with `Flush`, it gets one time budget, `close_flush_timeout_ms` (2 s by default). In that time it sends its queued frames, then it sends CONNECTION_CLOSE, the "goodbye" packet that tells the server the connection is over.
- If sending the queued frames uses the whole budget, the client's network thread shuts down before the goodbye packet goes out (`crates/crpg-net-quic/src/client.rs:462` sets the budget, `:680-685` waits on the same, already expired, deadline).
- The server never hears the goodbye. It only learns the client is gone when the idle timeout fires, 60 s later. The test waits 30 s, so it fails.

The worker reproduced this on purpose: 128 queued frames and a `Flush` close gave the server `IdleTimeout` at 60.0 s instead of the client's normal close. This could happen to a real player on a slow link: the server would show a departed player as connected for a minute.

`tasks/T023.md` (`CloseMode::Flush`, lines 379–381) says "write every queued frame … wait up to `close_flush_timeout_ms`, then CONNECTION_CLOSE". It does not say whether the goodbye still counts against the same 2 s. `server.shutdown` uses the same one-shared-deadline shape (`tasks/T023.md:494-496`), so it may have the same gap.

## D43a — What the close budget covers

- **(a) Recommended: keep "a close finishes within the budget", and reserve a slice for the goodbye.** The flush stops sending at budget minus a small fixed reserve (for example 200 ms, or less if the budget is smaller), and the goodbye always gets that reserve. A close still takes at most 2 s, as callers expect today. The cost is that a flush on a slow link may discard a few more frames, and that is already reported in `CloseReport::outbound_frames_discarded`.
- (b) The flush keeps the whole budget, and the goodbye then gets its own extra wait (up to the budget again). Nothing extra is discarded, but a close can take up to twice the budget (4 s by default). Code that waits on a close, such as `server.shutdown` and the host's shutdown, waits longer.
- Either way, the fix covers both the client close and `server.shutdown`, and adds a test: a large queued backlog plus a `Flush` close, and the server must see the client's own close well before the idle timeout.

**Next step after you answer:** the manager has a short contract (T098g, one crate: `crpg-net-quic`) written to your choice, then sends it to an OpenCode worker with a Claude review, because a close-semantics bug is the kind a test can miss.

## Agent log

- 2026-11-03 (UTC) · claude-code/opus-5.5 (manager) + session 35 · Drafted D43 from the T098f measurement, which found a lost CONNECTION_CLOSE when a `Flush` close spends its whole budget, so the user can choose what the close budget covers before a fix is contracted.
- 2026-11-03 (UTC) · claude-code/opus-5.5 (manager) + session 35 + D43 answered · Recorded the user's answer: (a), keep the close bound and reserve a slice for CONNECTION_CLOSE.
