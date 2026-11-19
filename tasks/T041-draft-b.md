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
Design decisions this contract makes. Each needs approval with the contract.

1. **The client harness is a tests-only assembly, not a new type.** A
   scripted client is `LocalClient` (`crates/crpg-server/src/run.rs:818`,
   the peer end of a local link and `net`'s `Transport`,
   `:834`) plus the client-local net modules T037e already assembles —
   `ReplicaWorld` (`crates/crpg-net/src/replica.rs:121`) or `ReplicaWorldV4`
   (`crates/crpg-net/src/replica_v4.rs:55`) fed by `apply_delta` (`:188`,
   `:96`) and `MoveOutbox` (`crates/crpg-net/src/outbox_v4.rs:54`) — wired
   together in `tests/support/headless.rs`, which never ships
   (`crates/crpg-testkit/AGENTS.md:104-111`). No `src/` change, no public
   export: the crate's public API changes only with an ADR (`:29-36`).
   *Rejected:* a shipped `HeadlessClient` type (a public surface with no
   runtime caller; T037e proves the tests-only shape suffices).
2. **Correctness suites run in-process; only the smoke touches real QUIC.**
   The desync, two-client, malicious and reconnect suites drive `LocalHost`
   (`crates/crpg-server/src/run.rs:332`: one `Host` plus in-memory local
   links) with `HostLoop` stepped by hand (`:110`) on injected `now_ms`
   (`crates/crpg-server/src/run.rs:4`), exactly as T037e drives the host —
   deterministic, no sockets. The gate-13 smoke drives the dedicated adapter
   itself, `QuicHost` (`crates/crpg-server/src/quic.rs:528`) started with
   `start_resumable` (`:578`) on loopback, with scripted real-QUIC clients;
   that needs the Q1 edge. *Rejected:* real sockets everywhere (imports the
   T098/QUIC-13 flake class into 5,000-tick assertions); no real QUIC at all
   (gate 13 names the dedicated adapter, `docs/IMPLEMENTATION_PLAN.md:181`).
3. **Determinism is seeded fabric plus injected time, never wall time.**
   Loss/jitter/reorder come from `InMemoryTransport`
   (`crates/crpg-net/src/sim.rs:396`) pumped by `tick`
   (`:572`) under `FaultSchedule` (`:321-327`: `loss_every`, `dup_every`,
   `reorder_depth`), with the T018c exercise's values (`loss_every` 7,
   `dup_every` 11, `reorder_depth` 2,
   `crates/crpg-net/tests/conformance.rs:1293-1298`). Script randomness is a
   hand-rolled xorshift64 (the T042 `XorShift64` precedent,
   `tasks/T042.md:220-224`: shifts 13, 7, 17; seed `0x5EED_0000_0000_0042`),
   std-only. No clock, thread, sleep or unseeded RNG in any suite: T098c's
   8x-only failure got a test-side verdict (`tasks/BACKLOG.md:173`), and
   QUIC case 13 died on a 30 s wall guard (`tasks/T098.md:312-316`). *Rejected:*
   `proptest`/new RNG crates (a new dependency for what a dozen lines do;
   `AGENTS.md:52`); wall-clock pacing; 8x contention.
4. **Desync comparison is semantic, never hash equality.** The check is the
   spec's: "the replica's visible state matches the server's filtered
   projection" (`docs/CRPG_ENGINE_SPEC.md:1488`), i.e. each replica's view
   against `InterestView::project_snapshot`/`project_delta` for its own
   `ViewerContext` (`crates/crpg-net/src/interest.rs:74`, `:155`, `:225`,
   `:334`) — the ADR-0009 cross-machine rule
   (`tasks/T041-facts.md:18`). Authority equality stays authority-only, as
   the host suite pins it (`crates/crpg-testkit/tests/host_movement.rs:375`).
   *Rejected:* comparing `authority_hash` with a replica hash (different
   domains; a filter gap would pass exactly when it should fail).
5. **Host-level means everything past the byte boundary.** T042 owns the
   eight decoders (`tasks/T042.md:80-89`); T041 owns its D-1 "Host level"
   column (`tasks/T042.md:431-446`): ownership (`NotAuthorized`), timing,
   host-side rate, seq replay, interest, plus the non-canonical same-intent
   retry T042 hands over (`tasks/T042.md:694-696`: `Host::canonical` must see
   a cached retry, not `SeqConflict`). The log line itself is observed, not
   written: each rejection must surface as its `Rejection` with the fixed
   `Display` T040a defines (`tasks/T040.md:227-254`), and where T040a's
   observer exists the suite asserts it fires (`tasks/T040.md:256-269`).
   Writing the line to stderr/file is T040b's (`tasks/T040.md:423-426`).
   *Rejected:* re-proving wire decoders (T042a owns them); testkit writing
   logs (D67a assigns that to the server program).
6. **One part, tests-only, in `crpg-testkit`.** The integration consumer
   owns cross-layer proofs (`AGENTS.md:36-42`); no second crate is touched.
   *Rejected:* a `crpg-server` suite part (the binary-side smoke is T040b's
   `server_binary.rs` pattern, `tasks/T040.md:567-628`, owned there).
7. **The fixture is `combat_basic` with T037e's in-memory movement edit.**
   Production `load_campaign` through the T016c loader, T148's `movement`
   block edited in memory, `first-blood` started through `crpg-sim` — the
   `host_movement.rs` shape, so no new fixture, replay or golden file.
   *Rejected:* a new checked-in campaign (a second fixture to bless for no
   new behaviour).
8. **No goldens out of T041.** With `goldens: none` (this brief, §
   Permissions) every expectation is relational (client A view == client B
   view == host projection) or structural (counts, codes, pinned RNG
   values), with fixed seeds. *Rejected:* blessing new per-target goldens
   (a reviewed re-baseline event each, for values another run cannot
   reproduce bit-for-bit across bleeding paths).

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
