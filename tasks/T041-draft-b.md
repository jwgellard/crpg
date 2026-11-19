# T041 — scripted headless clients and desync suite (`crpg-testkit`)

**Status: draft, 2026-11-19 (UTC). Not approved.**

## Task
Give `crpg-testkit` the Phase 4 headless-client proof the M4 exit criteria
demand (`docs/IMPLEMENTATION_PLAN.md:168-174`): scripted clients that connect,
move and fight; a 5,000-tick loss/jitter desync with semantic comparison; two
clients ending with identical visible state; and the gate 13 dedicated smoke
(`docs/IMPLEMENTATION_PLAN.md:181-183`). It also takes the host-level half of
the malicious-client suite: T042 is the wire level in `crpg-net`, and
"host-level cases live in T041" (`docs/IMPLEMENTATION_PLAN.md:247`;
`tasks/T042.md:39`), indexed by T042's D-1 "Host level" column
(`tasks/T042.md:431-446`) plus the non-canonical-retry case T042 hands over
(`tasks/T042.md:694-696`) and the D67a server-log-line backlog row
(`tasks/BACKLOG.md:348`; `tasks/DECISIONS-D67.md:5-12`).

One part, **T041**, in `crpg-testkit`, tests only: no `src/` change, no public
export, no golden. Not S-set (`docs/IMPLEMENTATION_PLAN.md:86-98`): the T041
row carries no S marker (`docs/IMPLEMENTATION_PLAN.md:246`), and nothing below
touches `World`, tick order, wire bytes, checkpoints, history versions,
schemas or manifests. Prerequisites: T025b, T027c, T037, T039 (the plan row).
T040 is not built on this base (the binary still refuses to run,
`crates/crpg-server/src/main.rs:10`), so the smoke drives the library
adapters, not the binary; T153's `Host::drive_ai` is specified
(`tasks/T153.md:60-66`) but not built here (only the run-loop hook
`Hosting::drive_ai(now_ms)` exists, `crates/crpg-server/src/run.rs:45`), so
the fight is scripted intents, not AI decisions (Q4).

## Parts
| Part | Crate | Scope | Size (tests + docs) | S-set | Review |
|---|---|---|---|---|---|
| T041 | `crpg-testkit` (tests only) | `tests/headless_desync.rs`, `tests/two_client.rs`, `tests/host_malicious.rs`, `tests/dedicated_smoke.rs`, `tests/support/headless.rs`, docs | ~700 test + ~80 doc lines | no | one OpenCode Go reviewer mutant run on the mutant table ("Tests"), one fix round |

Order: one part. It runs after T025b, T027c, T037 and T039 merge; it may run
beside T040 (no shared files: T041 touches no `crpg-server` source).

### T041 owns
- `crates/crpg-testkit/tests/headless_desync.rs` (new), `crates/crpg-testkit/tests/two_client.rs` (new), `crates/crpg-testkit/tests/host_malicious.rs` (new), `crates/crpg-testkit/tests/dedicated_smoke.rs` (new)
- `crates/crpg-testkit/tests/support/headless.rs` (new; included from the four suites via `#[path = "support/headless.rs"]`, the T016c pattern, `crates/crpg-testkit/AGENTS.md:152-156`)
- `crates/crpg-testkit/Cargo.toml` (at most one `[dev-dependencies]` line, only if Q1 is approved)
- `docs/architecture/crpg-testkit.md` (one new section "Headless clients and desync (T041)"), `crates/crpg-testkit/AGENTS.md` (one new section plus traps)
- goldens: none new, none moved.

## Readings

## Interface

## Behaviour

## Errors

## Tests

## What must not change

## Dependency edges

## Out of scope

## Open questions

## Agent log
- 2026-11-19 (UTC) · opencode-go/muse-spark-1.3-contributor + T041 draft b · Skeleton with all section headings.
