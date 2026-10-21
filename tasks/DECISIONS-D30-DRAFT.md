# Decision D30 — draft

Status: **Draft; not yet put to the user.** Non-blocking questions collected
by manager sessions under `docs/AGENT_TEAM_PLAN.md` §6, each with a
recommendation. Nothing here is decided until the user answers.

## D30a — case 23: handshake through a clean relay?

T098c measured `lane0_codec_frames_decode_after_impaired_delivery` failing
only at 8× contention (3/320; 0/20 alone, 0/80 at 4×). The server's 5 s
hello timeout fires because the handshake runs through the impaired relay
(`crates/crpg-net-quic/tests/quic.rs:1190`), and the test's 30 s guard then
trips. Verdict: test-side.

- **Recommend:** (a) in `codec_frames_through` and `ordered_exactly_once_through_impaired_relay` (`crates/crpg-net-quic/tests/quic.rs`), connect over a clean relay and switch impairment on only after `HelloReceived`, so the tests measure frame delivery under loss, which is what they assert. Cost: the handshake is no longer tested under drop/reorder; add a separate handshake-under-loss case only if wanted, with a wait on observable state rather than the 5 s timeout.
- Alternative: (b) keep the handshake impaired and accept the rare flake under heavy load (8× contention, about 1 in 100), recorded as a known flake in T098.

## D30b — A0-8, the `crpg-cli` `AGENTS.md` trim

Branch `task/A0-8-cli-agents-trim` (`af2a66f`, 2026-10-17, unpushed) moves
389 lines of `crates/crpg-cli/AGENTS.md` into `docs/architecture/crpg-cli.md`.
A0-8 needs the user's approval per crate.

- **Recommend:** (a) approve it, then the manager applies D29k on top and adds both to the next batch PR.
- Alternative: (b) reject or revise; D29k is then applied to the current file.

## D30c — ADR-0029 (T108) acceptance

ADR-0029 "Kernel range and time model" (Proposed),
`docs/adr/0029-kernel-range-and-time-model.md`, needs the user's H-accept.
Its nine open questions with the architect's recommendations:

1. a mode switch applies at the next round boundary;
2. T109 runs in data only after follow-on contracts 3 and 4, and "+ Lua" is dropped;
3. approve follow-on contracts 1 and 2 now, defer exploding dice and glitch rules;
4. multiplayer pause rights: the session owner holds both, guests only by grant, dedicated worlds operator-only;
5. auto-pause: three triggers on in single player, all off in multiplayer;
6. commands still run while paused;
7. one encounter per area, with fights merging;
8. roll transparency versus ADR-0019's ban on rolls on the wire: a ruleset may opt in to showing a player their own roll and modifiers, DCs stay hidden;
9. AI runs outside the tick order, amending spec §10 stage 5.

- **Recommend:** accept the ADR with all nine recommendations.
- Alternative: answer them one by one.

## Agent log

- 2026-10-21 (UTC) · opencode-go/muse-spark-1.3-contributor + S11 bookkeeping · Started the D30 draft with the case-23 relay, A0-8 trim and ADR-0029 acceptance questions, so the user can answer them in one batch.
