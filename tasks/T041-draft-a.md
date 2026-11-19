# T041 — scripted headless clients and the desync suite (`crpg-testkit`)

**Status: draft, 2026-11-19 (UTC). Not approved.** Blind draft A, written by
DeepSeek V4 Pro, session 79, slot 4, attempt 1. A second model is drafting the
same contract blind; a Claude architect writes the final contract from the two
drafts and the user approves it. Nothing here is approved.

Written from the T041 row (`docs/IMPLEMENTATION_PLAN.md:246`), ADR-0012
(`docs/adr/0012-windows-primary-platform.md`), D65a, D67a, D70 (approved; the
dedicated server program is specified in `tasks/T040.md`, not built yet) and
the root `AGENTS.md`. Testkit rules: root `AGENTS.md` (testkit is a one-way
integration consumer) and the `ALLOWED` table in `tools/lint/deps.py`. The
fact sheet `tasks/T041-facts.md` is the map; every fact relied on was
re-checked in code. Citation convention (T140's): a bare `:N` is a line of
the file cited earlier in the same item.

## Task

Give `crpg-testkit` the headless-client machinery the Phase 4 exit criteria
need (`docs/IMPLEMENTATION_PLAN.md:171–172`, spec
`docs/CRPG_ENGINE_SPEC.md:1488`, `:1587–1589`), all tests-only, no `src/`
change, no public export:

- **scripted headless clients**: one tests-only harness assembling the
  net-local modules a Godot-free client needs (`crates/crpg-net/src/lib.rs:66`)
  around the in-memory host link — `LocalHost`/`LocalClient`
  (`crates/crpg-server/src/run.rs:332`, `:818`), `ReplicaWorldV4`
  (`crates/crpg-net/src/replica_v4.rs:55`), `MoveOutbox`
  (`crates/crpg-net/src/outbox_v4.rs:54`) — so a test scripts two clients
  through attack/move/end-turn intents;
- **a 5,000-tick loss/jitter desync test**: run the authoritative host and two
  client replicas over the simulated network's seeded `FaultSchedule`
  (`crates/crpg-net/src/sim.rs:321`) for 5,000 ticks and assert, at fixed
  checkpoints, that each replica's visible state equals the host's filtered
  projection (semantic comparison, never hash equality — ADR-0009,
  `docs/adr/0009-determinism-scope.md:46`);
- **two clients with identical visible state**: both clients bound with equal
  disclosure grants converge to equal replicas after the scripted run
  (`docs/CRPG_ENGINE_SPEC.md:1527`);
- **the gate 13 dedicated smoke**: two clients over the real dedicated
  `crpg-server` binary move, fight and end with identical visible state
  (`docs/CRPG_ENGINE_SPEC.md:1589`);
- **the host-level malicious-client cases** the T042 row hands here
  (`docs/IMPLEMENTATION_PLAN.md:247`): the "Host level" column of
  `tasks/T042.md` D-1 — ownership, timing, interest, flooding, replay, and a
  same-`seq` retry with a non-canonical encoding treated as a cached retry,
  never a `SeqConflict` (`tasks/T042.md:693–696`).

Testkit is a one-way integration consumer (root `AGENTS.md:36–42`): everything
T041 builds lives under `crates/crpg-testkit/tests/`, never ships, and reaches
the host through the dev-only `crpg-server` edge T037e already opened
(`crates/crpg-testkit/AGENTS.md:137–140`).

## Parts

One crate (`AGENTS.md` "One task = one crate"), four parts, all tests-only in
`crates/crpg-testkit`, none S-set (`docs/IMPLEMENTATION_PLAN.md:86–98`): no
`World` field, tick order, wire byte, schema, workspace manifest or golden is
touched, and nothing in `src/` changes.

| Part | Crate | Scope | Size (tests + docs) | S-set | Review |
|---|---|---|---|---|---|
| T041a | `crpg-testkit` | the headless-client harness and the scripted driver in `tests/support/` | ~330 + ~40 | no | one OpenCode Go reviewer mutant run, one fix round |
| T041b | `crpg-testkit` | the 5,000-tick loss/jitter desync and two-client identical-visible-state tests | ~360 | no | one OpenCode Go reviewer mutant run, one fix round |
| T041c | `crpg-testkit` | the host-level malicious-client suite (T042's "Host level" column) | ~320 | no | one OpenCode Go reviewer mutant run, one fix round |
| T041d | `crpg-testkit` | the gate 13 dedicated smoke (real `crpg-server` binary + two `QuicClient`s) | ~220 | no | reviewer reads the smoke; the binary's own gates prove the rest |

Order: **T041a → T041b → T041c, serial** (T041b and T041c both consume
T041a's harness; T041c is independent of T041b and may follow either).
**T041d is cut after T040, T150c and T154d merge** (Open question 1); until
then it is specified but not buildable.

### T041a owns
- `crates/crpg-testkit/tests/support/headless_client.rs` (new: the
  `HeadlessClient` harness and the scripted two-client driver)
- `crates/crpg-testkit/tests/support/mod.rs` (unchanged; nothing added)
- `docs/architecture/crpg-testkit.md` (new section "Headless clients and the
  desync suite (T041)"), `crates/crpg-testkit/AGENTS.md` (public-surface note
  that the harness is tests-only, one new trap)
- goldens: none new, none moved.

### T041b owns
- `crates/crpg-testkit/tests/desync.rs` (new)
- `docs/architecture/crpg-testkit.md` (extends the T041a section)
- goldens: none new, none moved.

### T041c owns
- `crates/crpg-testkit/tests/malicious_host.rs` (new)
- `docs/architecture/crpg-testkit.md` (extends the T041a section)
- goldens: none new, none moved.

### T041d owns
- `crates/crpg-testkit/tests/dedicated_smoke.rs` (new)
- `crates/crpg-testkit/Cargo.toml` (one `[dev-dependencies]` line, the
  `crpg-net-quic` edge; `Cargo.lock` grows by edge entries only — Open
  question 2)
- `docs/architecture/crpg-testkit.md` (extends the T041a section)
- goldens: none new, none moved.

## Readings

Design decisions this contract makes. Each needs approval with the contract.

1. **A headless client is an in-memory link plus the net-local modules, not a
   `QuicClient`.** `LocalClient` is the peer end of a local link and
   implements net's `Transport` (`crates/crpg-server/src/run.rs:818`,
   `:834`); `replica`, `replica_v4`, `outbox_v4`, `movement_v4`, `interest`,
   `sim` and `transport` are net-local modules a headless client assembles
   without Godot (`crates/crpg-net/src/lib.rs:66`). So T041a builds a
   `HeadlessClient` that owns a `LocalClient` link, a `ReplicaWorldV4` and a
   `MoveOutbox`, and drives it over `LocalHost` with injected `now_ms`. The
   authority boundary holds: the client mutates nothing directly, only through
   `send` intents (`crates/crpg-server/AGENTS.md` invariant 1).
   *Rejected:* a `QuicClient`-based harness for the in-process tests (needs
   sockets/threads, which `tests/` harnesses must not own, testkit AGENTS
   invariant 7; and a two-crate part); a second `ClientSession` type in
   `crpg-net` (none exists, `tasks/T041-facts.md:34`, and T041 must not grow a
   production type — tests-only).

2. **The desync oracle is an independent projection, never a second pass over
   the delivery bytes.** ADR-0009 defines the cross-machine check as a
   *semantic* comparison: replica visible state versus the server's filtered
   projection (`docs/adr/0009-determinism-scope.md:46`). Applying the host's
   own delivery frames twice cannot catch a projection bug, so the oracle
   builds the expected view from the host's `World` through
   `crpg-net`'s `interest` module (`crates/crpg-net/src/interest.rs:1`,
   `project_snapshot` `:334`, `project_delta` `:225`) with the client's
   grants, and reduces the `ReplicaWorldV4` to the same canonical image
   (D-4). *Rejected:* re-applying `take_delivery_entries` output as the
   oracle (circular — the host's filter is the thing under test); a
   hash-equality check over the replica (ADR-0009 forbids hash equality as
   the desync check, and the replica holds no `state_hash`).

3. **Timing-sensitive cases stay deterministic: seeded `FaultSchedule` and
   injected `now_ms`, never wall time or `thread::sleep`.** The T098c and
   QUIC case 13 flakes must not be made worse (`tasks/BACKLOG.md:169`,
   `tasks/T098.md:312`). Every run steps `HostLoop::step(now_ms)` on a fixed
   50 ms grid (`crates/crpg-server/src/run.rs:154`) and pumps the fabric with
   `FaultSchedule { seed, loss_every, dup_every, reorder_depth }`, whose doc
   promises "same seed plus same send/tick sequence delivers the same byte
   sequence on the same build" (`crates/crpg-net/src/sim.rs:308–319`). The
   5,000-tick run uses `loss_every: Some(7), dup_every: Some(11),
   reorder_depth: 2` (T018c's values, `crates/crpg-net/tests/conformance.rs:1294–1299`);
   loss is the spec's `3%` floor's deterministic stand-in — periodic
   loss-every-7 drops ~14%, which is stricter than 3% and pinned, not random.
   *Rejected:* a PRNG loss ratio (`crpg-net`'s fabric has no ratio knob, only
   `loss_every`; adding one is a `crpg-net` change T041 may not make); wall
   time (nondeterministic, the flakes above).

4. **Two clients see identical state because they hold equal grants, and the
   check is semantic equality of the two replicas and of each against the
   projection.** "Both see the same world" (`docs/CRPG_ENGINE_SPEC.md:1527`)
   is proved three ways at the last checkpoint: `client_a.replica ==
   client_b.replica` (both reduced to the canonical image), and each equals
   the projected expected view (D-4). The two clients still differ in
   *control* (each acts only for its own actor, D-8) but share disclosure, so
   their visible rows coincide. *Rejected:* identical control sets (two
   clients acting for the same actors would fight over turn order and the
   run would not exercise the two-principal boundary); comparing only
   `authority_hash` (that is the host's own hash, `host_movement.rs:375`, not
   a client view — the fact sheet found no existing client-view equality,
   `tasks/T041-facts.md:67`).

5. **The 5,000-tick run reuses T018c's exercise shape but over the real host
   and real replicas.** T018c's `exercise_run` is a driver-double proof
   (`crates/crpg-net/tests/conformance.rs:1290`); T041b is the same shape —
   5,000 ticks, sparse valid and rejected inputs through loss/dup/reorder,
   hashes every 1,000 ticks — but the authority is `LocalHost` over a
   `Host` built from a fixture, and the receivers are `ReplicaWorldV4`s, so a
   divergence proves the host's projection, not the double's. *Rejected:* a
   fresh 5,000-tick loop design (the conformance exercise already pins the
   faults, the sparse-input cadence and the checkpoint cadence that make the
   run tractable and deterministic); moving the exercise into `crpg-net`
   (that crate owns the double; the host-level proof belongs here by E005).

6. **The malicious-client cases T041 covers are admission-level, not
   wire-level.** T042 already enumerates every byte-level refusal at the eight
   decoders (`tasks/T042.md:42–53`); the "Host level" column of its D-1 table
   is T041's case list (`tasks/T042.md:436–445`). Each case is a well-formed
   frame the host's admission path rejects cleanly with a pinned
   `RejectionCode`, never a panic: ownership (`NotAuthorized`, `:17`), timing
   (`StaleTick`/`FutureTick`/`IllegalAction`, `:15`/`:16`/`:18`), interest
   (`NotAuthorized` for an out-of-grant id), flooding (`RateLimited`, `:8`),
   replay (`StaleSeq`/`SeqConflict`/`SeqGap`, `:13`/`:11`/`:12`). The
   "clean rejection and a log line" cell is D67a's open item (Open question
   3), not a host check in this part. *Rejected:* re-testing the decoders
   (T042's, and `crates/crpg-server/tests/host_quic.rs`
   `quic_malicious_frames_rejected_without_mutation` already covers oversized
   and malformed at the adapter); a QUIC-adapter suite here (the adapter is
   `crpg-server`'s; testkit reaches the in-memory `Host`/`LocalHost` only).

7. **A same-`seq` retry with a non-canonical encoding is a cached retry, never
   a `SeqConflict`.** `Host::canonical` compares re-encoded bytes
   (`crates/crpg-server/src/host.rs:1588–1590`, `:1666–1668`), so a retry
   whose bytes re-encode to the retained canonical form (a non-minimal
   varint, or a ULID case/`I`/`L`/`O` alias, `crates/crpg-core/src/ulid.rs:111–122`)
   must hit the retry cache and return the retained outcome, while different
   canonical bytes at the same `seq` still `SeqConflict`
   (`tasks/T042.md:693–696`). T041c pins both. *Rejected:* treating
   byte-inequality as `SeqConflict` (breaks retry recovery for alias-encoding
   clients, the exact defect T042 flagged); teaching the decoders to
   canonicalize (the host owns `canonical`; decoders are stateless).

8. **The gate 13 smoke is the real binary with two `QuicClient`s; the embedded
   smoke is T041b's two-client test itself.** Gate 13 smokes "Windows
   embedded, Windows dedicated and Linux dedicated" (`docs/IMPLEMENTATION_PLAN.md:180–181`).
   The embedded leg is the in-process two-client run (T041b). The dedicated
   leg (T041d) spawns `crates/crpg-server` (T040's binary, D70) and connects
   two `QuicClient`s (`crates/crpg-net-quic/src/client.rs:104`, `:118`) over
   the loopback, scripting move/fight, and asserts identical visible state
   through the same harness's reduce. *Rejected:* a `QuicHost`-only smoke
   without the binary (does not smoke the program T040 builds, which is what
   gate 13 is for); folding the dedicated smoke into T070 (T070 is the full
   §17.2 acceptance and lists T041 as its prerequisite,
   `docs/IMPLEMENTATION_PLAN.md:335`; T041d is the earlier, smaller proof).

9. **No new dependency in T041a–c.** The harness uses `crpg-net` (already a
   dev-dependency, `crates/crpg-testkit/AGENTS.md:137`) and `crpg-server`
   (already a dev-dependency, `:137`), plus `crpg-core` for `EntityId`/`Ulid`
   (already dev-only, `:133`). T041d's `crpg-net-quic` edge is the one new
   candidate (Dependency edges, Open question 2); it is a workspace member,
   already reachable transitively through `crpg-server`'s test build. Give a
   std-only alternative there. *Rejected:* a `proptest`/fuzz dependency for
   the malicious suite (the admission cases are enumerable by hand, as T042's
   Reading 5 argued for the wire level); a logging crate for D67a's line
   (T042 Question 1 already chose to pin the text and defer the writer).

10. **The harness never ships.** Per testkit invariant 5
    (`crates/crpg-testkit/AGENTS.md:106–111`), nothing in `tests/` may grow a
    runtime caller; the `HeadlessClient` lives in `tests/support/` and is
    included by the suites via `#[path = "support/headless_client.rs"] mod
    headless_client;` (the T016c/T017e/T037e include shape,
    `crates/crpg-testkit/AGENTS.md:155`, `:183`, `:210`). No `src/` file
    changes, no public export. *Rejected:* a `src/` helper (would need an ADR
    for the public surface, and no binary needs a headless client today); a
    `tests/support/mod.rs` edit (the reserved shared support module stays
    byte-identical).

## Interface

Production code: none. No `src/` file, public item, signature or `Display` of
any crate changes; the harness lives in
`crates/crpg-testkit/tests/support/headless_client.rs` and is included by
each suite via `#[path = "support/headless_client.rs"] mod headless_client;`
(Reading 10). Types are named so the review and later tasks cite them; private
helpers are the implementer's. No `HashMap`/`HashSet`, no float, no clock,
thread, sleep, socket or RNG (testkit AGENTS invariants 6–7): `BTreeMap`,
`Fx16_16`/integers and injected `now_ms` only.

```rust
//! T041a: the scripted headless-client harness. Tests-only; never ships.

use std::num::NonZeroUsize;

use crpg_core::EntityId;
use crpg_net::outbox_v4::{MoveBody, MoveIntent, MoveOutbox, OutboxError};
use crpg_net::protocol::SessionEpoch;
use crpg_net::protocol_transfer::{HealthFact, TurnFact};
use crpg_net::replica_v4::ReplicaWorldV4;
use crpg_net::transport::Transport;
use crpg_server::host::PeerHandle;
use crpg_server::run::{LocalClient, LocalError, LocalHost};

/// One scripted headless client: the in-memory link plus the net-local
/// modules a Godot-free client assembles (Reading 1).
pub struct HeadlessClient {
    /// The peer end of the local link: intents up, delivery frames down.
    pub link: LocalClient,
    /// The client's copy of its filtered v4 session.
    pub replica: ReplicaWorldV4,
    /// The lane-1 intent sender for this session's epoch.
    pub outbox: MoveOutbox,
    /// The actors this client may act as (its control grant), ascending.
    pub control: Vec<EntityId>,
}

/// What one [`HeadlessClient::drain`] consumed, in the order it saw them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Drain {
    /// Lane-0 delta frames applied to the replica.
    pub deltas: usize,
    /// Lane-1 receipts applied to the outbox.
    pub receipts: usize,
    /// Lane-1 samples applied to the replica.
    pub samples: usize,
    /// Frames skipped because their kind did not apply (foreign or unknown
    /// first byte); never a panic.
    pub skipped: usize,
}

impl HeadlessClient {
    /// Attaches a fresh client for an already-bound `peer`, with an empty
    /// replica and outbox for `epoch`. `control` is sorted ascending here.
    pub fn attach(
        local: &mut LocalHost,
        peer: PeerHandle,
        epoch: SessionEpoch,
        control: Vec<EntityId>,
        capacity: NonZeroUsize,
    ) -> Result<Self, LocalError>;

    /// Drains the link's delivery frames and applies each. Classification by
    /// first byte (B-2): `b[0] == 4` → `codec_v4::decode_delta` →
    /// `replica.apply_delta`; `b[0] == 0xA1` → `movement_v4::decode_down` →
    /// `MoveDown::Receipt(r)` → `outbox.receive(&r)`; otherwise a sample
    /// datagram → decode its entries → `replica.apply_sample`. A frame that
    /// fails to classify or apply is counted in `skipped` and never panics.
    pub fn drain(&mut self) -> Drain;

    /// The client's own actor `NetId`, resolved from its replica rows: the
    /// `net_id` of the row whose disclosed entity equals `control[0]`'s
    /// authoring. `None` until the first delta discloses it.
    pub fn own_net(&self) -> Option<crpg_net::protocol::NetId>;
}
```

The two-client script driver (T041a, shared by T041b and T041c):

```rust
/// One scripted step for one client, applied at a tick. `Attack`, `EndTurn`
/// and `DeclareMove` are lane-0 intents encoded by the test through
/// `codec_v4::encode_intent`; `MoveTo`/`Halt` are lane-1 through
/// `outbox.send` + `movement_v4::encode_intent`.
pub enum ClientStep {
    /// Lane-0 `DeclareAction`/`EndTurn`/`DeclareMove` intent bytes, or the
    /// actor/body for the harness to encode.
    Lane0 { seq: u64, observed_tick: u64, body: Lane0Body },
    /// Lane-1 move for `actor` (a `NetId` this client controls).
    Move { actor: crpg_net::protocol::NetId, body: MoveBody },
}

/// Drives `loop.step(now_ms)` once and drains both clients; `now_ms` starts
/// at 0 and advances by 50 ms per tick (the `HostLoop` grid).
pub fn drive_tick(
    local: &mut LocalHost,
    host_loop: &mut HostLoop<LocalHost>,
    now_ms: u64,
    clients: &mut [&mut HeadlessClient],
);
```

The desync oracle (T041b), in `tests/desync.rs`:

```rust
/// The canonical visible state both a replica and a projection reduce to
/// (Reading 2, 4). `rows` maps each visible `NetId` to its disclosed health
/// (`None` = presence only); `turn` is the encounter's disclosed turn fact.
/// Equality is `BTreeMap` equality — order-insensitive, semantic (ADR-0009).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonView {
    pub rows: BTreeMap<crpg_net::protocol::NetId, Option<HealthFact>>,
    pub turn: Option<TurnFact>,
}

/// Reduces a replica to its canonical view: `v3().entities()` →
/// `(net_id, health)` and `v3().turn()`.
pub fn replica_view(replica: &ReplicaWorldV4) -> CanonView;

/// The independent oracle: builds an `interest::InterestView`, feeds it a
/// `ViewerContext` whose `facts` are the test's own disclosure table for the
/// client's grants (not the host's delivery), calls `project_snapshot`, and
/// reduces the `SnapshotBody` (`entities` → `(entity, health)`, `turn`).
pub fn expected_view(
    view: &mut InterestView,
    ctx: ViewerContext<'_>,
) -> CanonView;
```

`HostLoop<LocalHost>` and `LocalHost::attach` are the existing run surface
(`crates/crpg-server/src/run.rs:110`, `:361`); `codec_v4::encode_intent` /
`decode_delta`, `movement_v4::encode_intent` / `decode_down` and the sample
decode are the frozen codecs the harness calls by path and never re-implements.

## Behaviour

_(filled next edit)_

## Errors

_(filled next edit)_

## Tests

_(filled next edit)_

## What must not change

_(filled next edit)_

## Dependency edges

_(filled next edit)_

## Out of scope

_(filled next edit)_

## Open questions

_(filled next edit)_

## Agent log

_(filled next edit)_
