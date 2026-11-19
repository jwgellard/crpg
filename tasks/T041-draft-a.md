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

Normative. Each item names the tests that prove it. "Clean rejection" means a
`ReceiptStatus::Rejected(code)` with the pinned `code` and no host-state
change; never a panic. **First failure wins** in every ordered list. Codes
are `RejectionCode`'s frozen v1 values (`crates/crpg-net/src/protocol.rs:393`):
`RateLimited` 8, `SeqConflict` 11, `SeqGap` 12, `StaleSeq` 13, `StaleTick` 15,
`FutureTick` 16, `NotAuthorized` 17, `IllegalAction` 18.

### T041a — the harness

B-1. **`attach`.** Attaches `peer` through `LocalHost::attach` and sorts
`control` ascending; the replica and outbox start empty for `epoch`. An
`UnknownPeer` or `AlreadyAttached` `LocalError` propagates unchanged and
attaches nothing. *Tests:* H1.

B-2. **`drain` classification.** Frames are read in link order and applied by
first byte: `4` → `codec_v4::decode_delta` → `replica.apply_delta` (counted
`deltas`); `0xA1` → `movement_v4::decode_down` → `MoveDown::Receipt(r)` →
`outbox.receive(&r)` (counted `receipts`); any other first byte is a sample
datagram → decode entries → `replica.apply_sample` (counted `samples`). A
frame that fails decode or apply is `skipped` + 1 and the client state is
unchanged by that frame; `drain` never panics. A lane-1 receipt that
`outbox.receive` refuses (`WrongEpoch`/`Unexpected`/`WrongActor`) is
`skipped`, not retried. *Tests:* H2.

B-3. **`own_net`.** The client's actor `NetId` is the `net_id` of the first
replica row (ascending) whose disclosed entity equals `control[0]`'s
authoring `EntityId`; `None` before any delta discloses it, and `None` for an
empty `control`. *Tests:* H3.

B-4. **`drive_tick`.** One `host_loop.step(now_ms)` (intake → `drive_ai` →
`advance` → deliver → retire, `crates/crpg-server/src/run.rs:185–192`), then
each client's `drain()` in the slice order. Nothing else runs; no clock is
read. *Tests:* H1, H2, H3.

### T041b — the desync run

B-5. **The fixture.** Two combatants over an in-memory `Host` from
`crates/crpg-server/tests/fixtures/three_combatants.json` edited to two
participants (A, B; health 10/10) with the T148 movement block, started
`first-blood` through `crpg-sim` and wrapped `Host::from_history` (the
T037e shape, `crates/crpg-testkit/AGENTS.md:207–228`). Client A's
`DisclosureGrants` disclose A fully (health) and B presence-only; client B
the mirror. Both control their own actor. *Tests:* D1, D2.

B-6. **The 5,000-tick run.** `now_ms` runs 0, 50, …, 249,950. Each tick, in
order: (1) each client's scripted step — at every 50th tick a lane-0 attack
or `EndTurn` from whichever client holds the turn (the conformance cadence,
`crates/crpg-net/tests/conformance.rs:1304–1332`), plus one lane-1 `MoveTo`
at tick 60 and its `Halt` at tick 90 (so loss/dup exercises a recovery);
(2) `drive_tick` with `FaultSchedule { loss_every: Some(7), dup_every:
Some(11), reorder_depth: 2, seed: 99 }`; (3) at every 1,000th tick record
`local.host().authority_hash()`. No wall time, no RNG beyond the seeded
fault stream; two runs give equal hashes and equal canonical views. The loss
of a lane-0 frame is recovered by the host's retry/dedup, so the run reaches
5,000 ticks without a client fencing or a capture backpressure stall (the
script acknowledges captures each 1,000th tick if the host's admission
reports them). *Tests:* D1, D3.

B-7. **The desync check.** At every 1,000th tick, for each client,
`replica_view(client.replica) == expected_view(view, ctx)`, where `ctx.facts`
is the test's own disclosure table (A full, B presence-only, or the mirror)
derived from `local.host().host().world()`'s public combatant reads — never
from `take_delivery_entries`. A divergence means the host's projection and
the replica disagree, and the test fails naming the first differing `NetId`
and tick. *Tests:* D1.

B-8. **Identical visible state.** At the last tick,
`replica_view(&a.replica) == replica_view(&b.replica)`, and each equals the
projection (B-7). Because both clients share disclosure and only differ in
control, their canonical views coincide. *Tests:* D2.

B-9. **No panic, determinism.** The whole run is one test body with no
`#[should_panic]`; a panic anywhere fails. The run is repeated on a fresh
rig and produces byte-equal recorded hashes (same build). *Tests:* D3.

### T041c — the host-level malicious suite

B-10. **Ownership.** A well-formed lane-0 intent whose `actor` is a `NetId`
the client's `ControlGrant` does not name → `NotAuthorized` (17); the host
state, sequences, receipts and `last_combat_error` are unchanged. *Tests:*
M1.

B-11. **Timing.** An intent with `observed_tick` older than the freshness
window → `StaleTick` (15); ahead of `server_tick` → `FutureTick` (16); a
gameplay-illegal intent at the current tick (an out-of-turn attack, or a
`DeclareMove` whose actor is not the active head) → `IllegalAction` (18).
Each leaves the state unchanged. *Tests:* M2.

B-12. **Interest.** An intent that names an entity outside the client's
disclosure grant (a hidden or foreign id) → `NotAuthorized` (17). A revoked
visibility id is `NotAuthorized`, never resolved to a row. *Tests:* M3.

B-13. **Flooding.** A burst over the peer's rate budget → `RateLimited` (8)
for the over-budget intents, with the retained head re-admitted only after
the injected clock advances (the bucket replenishes stepwise,
`crates/crpg-net/src/sim.rs:157`). *Tests:* M4.

B-14. **Replay.** A same-`seq` retry whose canonical bytes equal the retained
intent returns the retained outcome (cached, never re-executed); a same-`seq`
retry with different canonical bytes → `SeqConflict` (11); a `seq` older than
the cache → `StaleSeq` (13); a `seq` skipping ahead → `SeqGap` (12), which
does not advance the next-expected sequence. *Tests:* M5.

B-15. **Canonical retry.** The same intent re-encoded with a non-minimal
varint (a length or `seq` byte re-encoded longer than minimal) or a ULID
case/`I`/`L`/`O` alias (`crates/crpg-core/src/ulid.rs:111–122`) that
`Host::canonical` re-encodes to the retained bytes is a cached retry, never a
`SeqConflict` (`crates/crpg-server/src/host.rs:1588–1590`, `:1666–1668`).
*Tests:* M6.

B-16. **Clean, never a panic.** Every case above is a `ReceiptStatus` with
the pinned code, no new `HostError` variant, no `Display` text change, and
the authority hash is byte-equal to the pre-intent hash after each rejection.
*Tests:* M1–M6.

### T041d — the gate 13 dedicated smoke

B-17. **Dedicated smoke.** Spawn the `crpg-server` binary (T040) with a
config naming an invitation with two control grants, connect two
`QuicClient`s over loopback (`crates/crpg-net-quic/src/client.rs:118`),
script move/fight, and assert both clients end with identical canonical
views (the harness's `replica_view` over each client's replica). The smoke
then writes `quit` and asserts exit 0 and the save. Windows dedicated and
Linux dedicated run the same binary shape (ADR-0012); the embedded leg is
B-8's in-process two-client run. *Tests:* S1.

B-18. **No placeholder.** The smoke drives the real program end to end; a
skipped or `todo!()` leg fails. *Tests:* S1.

## Errors

No new error type, variant or `Display` text in any crate. The harness and
suites return only existing errors:

| Item | Value |
|---|---|
| `attach` on an unbound/duplicate peer | `LocalError::{UnknownPeer, AlreadyAttached}` (`crates/crpg-server/src/run.rs`) |
| an outbox send at `SEQ_LAST`/full | `OutboxError::{Exhausted, Full}` (`crates/crpg-net/src/outbox_v4.rs`) |
| an outbox receipt out of order/epoch | `OutboxError::{WrongEpoch, Unexpected, WrongActor}` |
| a replica frame out of sequence | `ReplicaError::EventSeq` etc. (`crates/crpg-net/src/replica_v4.rs`), skipped by `drain`, not retried |
| a hostile intent (T041c) | the `ReceiptStatus::Rejected(code)` of Behaviour, codes pinned in `protocol.rs:393` |
| a hostile frame at the harness | `skipped` + 1, never a panic |

The D67a "log line" for a rejected client message is **not written here**: the
in-memory host has no logging, and D67a assigns writing the line to the server
program (T040's binary) with a log-destination decision (Open question 3). No
credential byte appears in any assertion message or error.

## Tests

Suites: `tests/desync.rs` (H1–H3, D1–D3), `tests/malicious_host.rs` (M1–M6),
`tests/dedicated_smoke.rs` (S1). Public API only; no `#[should_panic]` (a
panic anywhere fails); no clock/sleep/RNG. Every case names a control that
changes one input and a "fails if". Codes are the frozen `RejectionCode`
values (`crates/crpg-net/src/protocol.rs:393–435`), computed here and never
adjusted to make a test pass.

### `tests/desync.rs`

- **H1 `attach_sorts_control_and_errors`.** `attach(local, peer, epoch,
  [B, A], cap)` → `control == [A, B]`; replica `is_empty`, outbox `next_seq()
  == 1`. *Control:* `attach` the same peer again → `AlreadyAttached`; an
  unbound handle → `UnknownPeer`. *Fails if:* control order is not ascending,
  or a failed attach leaves a link.
- **H2 `drain_classifies_and_never_panics`.** With a link whose down queue
  holds, in order, one lane-0 v4 delta (`b[0] == 4`), one lane-1 receipt
  (`b[0] == 0xA1`), one sample datagram, and one `[0xFF]`: `drain()` returns
  `Drain { deltas: 1, receipts: 1, samples: 1, skipped: 1 }` and the replica,
  outbox and positions reflect the three applied frames. *Control:* the same
  queue minus the receipt → `receipts: 0`. *Fails if:* a foreign frame panics,
  or a receipt is retried instead of skipped.
- **H3 `own_net_resolves_from_replica`.** After `drain` of the client's first
  delta (which discloses its actor), `own_net() == Some(actor_net)`; before
  any drain it is `None`. *Control:* an empty `control` → `None` always.
  *Fails if:* resolution picks a row by insertion rather than by disclosed
  entity.
- **D1 `desync_checkpoints_match_the_projection`.** The B-6 run; at each
  1,000th tick, for each client, `replica_view(&client.replica) ==
  expected_view(&view, ctx)`. *Control:* a second client bound
  presence-only (health withheld) → its `expected_view` rows have `None`
  health, and a projection that leaked health would make the two disagree.
  *Fails if:* any checkpoint's canonical views differ, naming the `NetId`
  and tick.
- **D2 `two_clients_see_identical_state`.** The B-6 run; at tick 5,000,
  `replica_view(&a.replica) == replica_view(&b.replica)` and each equals the
  projection. *Control:* bind client B presence-only → its view differs from
  A's (health `Some` vs `None`), so the equality is not vacuous. *Fails if:*
  two equally-granted clients end with differing rows or turn facts.
- **D3 `run_is_deterministic`.** Two fresh rigs, same script and faults,
  produce byte-equal recorded `authority_hash()` sequences. *Control:* a
  different `FaultSchedule.seed` → a differing hash sequence (proves the
  comparison reads the recorded bytes). *Fails if:* any nondeterminism
  (clock, thread, unseeded RNG) leaks in.

### `tests/malicious_host.rs`

One `LocalHost` over the B-5 fixture with client A bound (control A, full
disclosure of A, presence-only of B). Each case sends one well-formed intent
through `client.link.send` + `drive_tick` and reads the receipt.

- **M1 `unowned_actor_is_not_authorized`.** An attack whose `actor` is B's
  `NetId` (not in A's control) → `ReceiptStatus::Rejected(NotAuthorized)`
  (`as_u8` 17). *Control:* the same intent for A → `Applied` or a gameplay
  refusal, never `NotAuthorized`. *Fails if:* a foreign actor is admitted or
  resolves.
- **M2 `illegal_times_are_rejected_exactly`.** `observed_tick` before the
  freshness window → `StaleTick` (15); ahead of `server_tick` → `FutureTick`
  (16); an out-of-turn attack (B's turn) → `IllegalAction` (18). *Control:*
  the on-turn attack is admitted. *Fails if:* any of the three returns a
  different code or mutates state.
- **M3 `out_of_interest_entity_is_not_authorized`.** An intent whose target
  is an entity outside A's disclosure → `NotAuthorized` (17). *Control:* the
  granted target B → admitted (presence-only still targets). *Fails if:* a
  hidden id resolves or is treated as a legal target.
- **M4 `over_rate_burst_is_rate_limited`.** A burst above the peer's rate
  bucket → `RateLimited` (8) for the over-budget intents, then, after
  `drive_tick` advances `now_ms` enough to replenish, the retained head is
  re-admitted. *Control:* the same intents spaced one per tick → never
  `RateLimited`. *Fails if:* the bucket counts lost-fault retries twice, or
  replenishment is not stepwise.
- **M5 `replay_codes_are_frozen`.** Retry the accepted intent at the same
  `seq` with identical bytes → the cached retained outcome (not re-executed;
  `executions()` unchanged); same `seq`, different bytes → `SeqConflict` (11);
  a `seq` older than the cache → `StaleSeq` (13); a `seq` skipping ahead →
  `SeqGap` (12) with `next_seq` unchanged. *Control:* a fresh `seq` → applied.
  *Fails if:* a cached retry re-executes, or a gap advances the sequence.
- **M6 `non_canonical_retry_is_a_cache_hit`.** Re-encode the accepted intent
  with a non-minimal varint (and separately a ULID `I`/`L`/`O` alias) and
  retry at the same `seq` → the cached retained outcome, never `SeqConflict`.
  *Control:* bytes whose canonical form genuinely differs → `SeqConflict`.
  *Fails if:* `Host::canonical` compares raw bytes instead of re-encodings.

### `tests/dedicated_smoke.rs`

- **S1 `dedicated_two_client_smoke`.** Spawn the `crpg-server` binary, connect
  two `QuicClient`s, script move/fight, assert identical canonical views,
  `quit` → exit 0 and the save reloads. *Control:* a client with a wrong pin
  → refused hello (a real failure the smoke must be able to observe, asserted
  in a sibling case or the control arm). *Fails if:* the smoke is skipped, a
  `todo!()`, or the clients do not reach identical visible state.

## What must not change

- Goldens, byte for byte (none added): every
  `crates/crpg-testkit/goldens/*` and `crates/crpg-sim/tests/goldens/*` file.
- Every existing test file, byte for byte and passing, in `crpg-testkit`
  (`tests/support/mod.rs` and `tests/support/combat.rs` included),
  `crpg-server`, `crpg-net`, `crpg-net-quic`, `crpg-sim` and `crpg-ai` —
  notably `crates/crpg-testkit/tests/host_movement.rs` and
  `tests/ai_mvp.rs`, and `crates/crpg-server/tests/run.rs`'s loop suite.
- `crates/crpg-testkit/src/` and `crates/crpg-testkit/Cargo.toml` (T041a–c);
  the public surface list in `crates/crpg-testkit/AGENTS.md:31–36` is
  unchanged — the harness is tests-only and adds no export.
- The wire, codecs, `Host` admission precedence, `RejectionCode` values,
  `LocalHost`/`LocalClient`/`HostLoop` surface, and every `HostError`/`Display`
  text; no crate's `src/` changes at all.
- `Cargo.toml` (workspace), `Cargo.lock` (T041a–c), `deny.toml`,
  `rust-toolchain.toml`, `crpg-contracts`, `tools/`, `.github/workflows/`.
- The T098c and QUIC case 13 suites (`tasks/BACKLOG.md:169`,
  `tasks/T098.md:312`): T041 runs no concurrent network test and no wall-time
  sleep, so it cannot make either flake worse.

## Dependency edges

| Edge | Section | `ALLOWED` |
|---|---|---|
| `crpg-testkit` → `crpg-net` | `[dev-dependencies]` (path) | already present (T037e, `crates/crpg-testkit/AGENTS.md:137`); `crpg-testkit: (None, {"crpg-godot"})` permits it (`tools/lint/deps.py:68`) |
| `crpg-testkit` → `crpg-server` | `[dev-dependencies]` (path) | already present (T037e, `:137`); permitted |
| `crpg-testkit` → `crpg-core` | `[dev-dependencies]` (path) | already present (T016c, `:133`); permitted |
| `crpg-testkit` → `crpg-net-quic` | `[dev-dependencies]` (path), **T041d only** | permitted by `tools/lint/deps.py:68` (`None`, not `crpg-godot`), but a **new edge** needing the user's approval (root `AGENTS.md:52`; Open question 2) |

No external crate and no new lockfile package. T041a–c add nothing to
`Cargo.toml`: the harness names `crpg-net` and `crpg-server` types through
edges that already exist, and `crpg-core` for `EntityId`. `deps.py` and
`deny.toml` are not edited.

T041d's `crpg-net-quic` edge is the one new candidate. **No-new-dependency
alternative** (if the user declines the edge): fold the dedicated smoke into
T070 (whose row is already `crpg-testkit (+ CI)` and lists T041 as a
prerequisite, `docs/IMPLEMENTATION_PLAN.md:335`), and keep T041d to the
in-process embedded leg (B-8), which needs no new edge — the real-QUIC
dedicated smoke then lands with T070, where the `crpg-net-quic` edge is
justified once. Either way `deps.py` already permits the edge; the question
is only whether it lands in T041 or T070.

## Out of scope

_(filled next edit)_

## Open questions

_(filled next edit)_

## Agent log

_(filled next edit)_
