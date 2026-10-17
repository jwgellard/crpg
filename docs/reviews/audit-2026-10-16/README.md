# Crate audit 2026-10-16

At the user's request ("put a few beam agents to the task of finding any gaps
in the project, missing tests, missing assumptions, and do checks of all
implementation"), run while the Go 5-hour cap held the paid models.

## Method

- **Pass 1, find.** Twenty-four read-only reviews on `opencode-go/beam-free`,
  two per crate with code (`crpg-core`, `-data`, `-rules`, `-sim`, `-net`,
  `-net-quic`, `-server`, `-persist`, `-script`, `-edit`, `-testkit`, `-cli`):
  lens `tests` (contract items no test would catch; tests that pass for the
  wrong reason) and lens `assumptions` (reachable panics, unchecked overflow and
  casts, ordering that determinism depends on, trusted input, doc mismatches).
  Every finding cites the code and the contract line. All 24 reported: 103
  findings, 11 high, 61 medium, 31 low, $0.00, 1–3 minutes each. The full table
  is `findings.md`, **unverified**.
- **Pass 2, verify.** Each high finding that claims a defect (not a missing
  test) went to a separate Beam run told that most audit claims are wrong, to
  trace the call path, look for guards and tests, and give a reproduction.
- **Manager check.** The manager read the code for the two with the widest
  blast radius (sim, net). The `execute` tool was denied for every run
  (`docs/agents/MODELS.md` §1 item 15).

## Confirmed defects

| # | Crate | Where | Defect | Severity (manager) | Checked by |
|---|---|---|---|---|---|
| D1 | crpg-sim | `src/combat.rs:2020-2057` | An ability that kills its own user (`allow_self_target`, lethal damage): dead combatants leave the timeline before the turn branch, so with `ends_turn` the `end_turn` pop skips the next living combatant's turn, and without it the `debug_assert` at :2052 panics in debug builds and release sets `active` to the dead actor. Breaks `crates/crpg-sim/AGENTS.md:76-80` | high | Beam ×2, manager read |
| D2 | crpg-data | `src/inventory.rs:402-412` | Object references nested in `list`/`map` tagged values are positioned without the `/value` segment (`.../deep/k/0/value` instead of `.../deep/value/k/value/0/value`), so `validate` and `explain` report wrong pointers. Breaks `docs/architecture/crpg-data.md:59-60` | high | Beam ×2 |
| D3 | crpg-net-quic | `src/server.rs:239-254` | `poll_event` releases a closed connection's record once its inbound queue is drained, even if `try_recv` never returned `Closed`; a later `try_recv` gets `UnknownConnection`. Breaks `docs/architecture/crpg-net-quic.md:172-174` | high | Beam ×2 |
| D4 | crpg-server | `src/checkpoint.rs:205-265`, `src/host.rs:489-504` | `load_checkpoint` never bounds `capture_acknowledged`; with `captured: []` and `u64::MAX`, `restore` sets `last_capture = u64::MAX`, the value the accept path reserves (`host.rs:1342`). Breaks "loading validates everything", `docs/architecture/crpg-server.md:85-86` | high | Beam ×2 |
| D5 | crpg-net | `src/sim.rs:519-527`, `:556-559` | The simulated transport pushes fault duplicates into `ready` uncounted, but `recv_from` releases accounting for every pop (saturating), so after duplication the per-peer and host staging caps under-count and stop holding. Test transport only | medium | Beam ×2, manager read |

## High test gaps (pass 1 only; cheap to close)

- crpg-sim: the non-finite `Transform` guards are tested on 3 of 6 fields in
  `state_hash` (`src/hash.rs:38`), 2 of 6 in the area-transfer preflight
  (`src/area.rs:111`), and not at all in `history_hash` (`src/history.rs:678`).
- crpg-net-quic: no test parks an inbound reader and then closes the peer
  (`src/queue.rs:438`).
- crpg-edit: manifest protection is tested only with absolute pointers, never an
  object-relative one (`tests/commands.rs:452`, `tasks/T058.md:391`).
- crpg-testkit: no test covers a malformed golden line after the run ends
  (`actual: None`, `src/harness.rs:269`).

## Next

- D1–D5 become fix tasks, one per crate, each with a regression test written
  from the reproduction above. D1 touches combat turn order: check it against
  the S-set rule on tick order before routing.
- The test gaps and the 61 medium rows feed a per-crate test-writer pass. The
  medium rows are unverified; give each crate's rows to its test writer as
  leads, and have the writer report which were false.

## Status (2026-10-17)

Every confirmed defect has a fix PR with a regression test, implemented by `opencode-go/beam-free` and reviewed by Beam plus, where noted in each PR, a free model of another lab: D1 → T117 (PR #41), D2 → T113 (PR #38), D3 → T114 (PR #39), D4 → T115 (PR #36), D5 → T116 (PR #40). Test-writer pass: crpg-edit → T118 (PR #42; #92 reported unreachable), crpg-sim → T119 (PR #43), crpg-net-quic → T120 (branch `task/T120-quic-test-gaps`, under review). No lead in those three crates turned out to be a defect. Remaining: crpg-testkit's high gap and the medium rows of the other crates.

## Agent log

- 2026-10-16 (UTC) · claude-code/opus-5.5 + crate audit · Recorded the method, the five defects confirmed by a second Beam pass (two also read by the manager) and the high test gaps, so fix and test tasks can be cut from them.
- 2026-10-17 (UTC) · claude-code (manager, Opus 5.5) + audit follow-up · Recorded which PR fixes each confirmed defect and how far the test-writer pass got, so the remaining crates can be picked up from here.
