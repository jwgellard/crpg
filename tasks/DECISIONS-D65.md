# Decision D65 — session 63: who ticks the world, joining a campaign, and the T151/T153 questions

Status: **D65a and D65b answered by the user, 2026-11-16 (UTC). D65c–D65h open.** Drafted by the manager in session 63. T151 and T153 wait on these; nothing in flight does.

## D65a — the server run loop owns the clock in both modes (answered)

The T151 contract (`task/T151-c1` `28c0550`, Reading 6 and its Question 4) had the Godot bridge call `Host::advance` from its frame poll in v4 sessions, because no server run loop exists in either mode (`crates/crpg-server/src/main.rs` is a 12-line stub; only tests call `Host::advance`). The user: "Single player should be exactly the same as multiplayer, just the server is running in the background."

**Answer:** file a `crpg-server` run-loop task. The loop owns the 20 Hz clock and the per-tick order (ingest, AI driver from T153, advance, deliver). The dedicated server and the embedded single-player host both run it; in single-player it runs in the background (own thread recommended; the contract decides). Revise T151 so the bridge only sends commands and renders what the server sends; it never ticks. T151's Question 4 is withdrawn. T153's `drive_ai` (Reading 1) plugs into this loop.

## D65b — joining a running campaign: possible by design, built later (answered)

Single-player uses the in-memory transport (ADR-0012: same protocol bytes, same authority boundary). Today the embedded host has no network listener, so nobody can join.

**Answer:** the run loop must be designed so one `Host` can serve an in-memory local player and QUIC players at once; the "open my campaign to others" switch is built later, after the demo. The architect also confirms that a single-player save loads into a dedicated server.

## Open (each with the architect's recommendation; plain-language versions in the session 63 transcript)

- **D65c (T151 Q1)** — network play for the Godot client in T151: recommend (a) not in T151; in-memory pipe now, QUIC client later. **Answered (a) by the user, 2026-11-16 ("Yes on that one").**
- **D65d (T151 Q2)** — keep v2 beside v4: recommend (a) keep both; retiring v2 later needs approval to delete its tests. **Answered (a) by the user, 2026-11-16: keep v2 beside v4.** New features target v4 only; each new protocol version extends the previous one on client and server (never a per-version copy); retiring v1/v2 is optional later cleanup, not planned work.
- **D65e (T151 Q3)** — the game may name a destination (turn-based) and, in real time, the creature by NetId; the server decides: recommend (a) yes. **Answered (a) by the user, 2026-11-16: "move requests need to be sent."**
- **D65f (T153 Q1)** — the `crpg-server` → `crpg-ai` edge (already in `ALLOWED`, `tools/lint/deps.py:69`; no new package): recommend (a) yes. **Answered (a) by the user, 2026-11-16.**
- **D65g (T153 Q2)** — the AI acts while the host is paused (fights and loaded saves start paused): recommend (a) yes.
- **D65h** — file a small `crpg-cli` task so `crpgc play` uses `crpg_ai::decide` for enemy turns (fastest terminal AI demo; needs a `crpg-cli` → `crpg-ai` edge): recommend yes.

## Agent log
- 2026-11-16 (UTC) · claude-code/opus-5.5 (manager) + D65 · Recorded the user's answers on the server run loop and on joining a campaign, and the six T151/T153 questions still open.
- 2026-11-16 (UTC) · claude-code/opus-5.5 (manager) + D65c · Recorded the user's yes to D65c (in-memory pipe now, network client later).
- 2026-11-16 (UTC) · claude-code/opus-5.5 (manager) + D65d · Recorded the user's answer to keep v2 beside v4, with new work on v4 only and versions built by extension.
- 2026-11-16 (UTC) · claude-code/opus-5.5 (manager) + D65e · Recorded the user's yes: the client sends move requests (destination; in real time also the creature), and the server decides.
- 2026-11-16 (UTC) · claude-code/opus-5.5 (manager) + D65f · Recorded the user's yes to the `crpg-server` → `crpg-ai` edge.
