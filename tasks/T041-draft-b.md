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
No production item changes. Everything below is test-local in
`crates/crpg-testkit/tests/support/headless.rs` (included via
`#[path = "support/headless.rs"]`), with these exact shapes — helper names
may not change, so the review and later tasks cite them. Existing APIs are
used as documented: `LocalHost::new(host)` (`crates/crpg-server/src/run.rs:347`),
`attach(peer, capacity)` (`:361-365`), `HostLoop::new(hosting, start_ms)`
(`:127`), `Host::new(seed, config, now_ms)`
(`crates/crpg-server/src/host.rs:973`), `ReplicaWorld::apply_delta`
(`crates/crpg-net/src/replica.rs:188`), `MoveOutbox::send`
(`crates/crpg-net/src/outbox_v4.rs:93`).

```rust
//! T041: scripted headless clients over the host (Reading 1). Tests-only:
//! nothing here ships. No clock, no thread, no I/O, no unseeded RNG,
//! no `HashMap`/`HashSet`, no float.

// Marsaglia xorshift64 (T042 precedent, tasks/T042.md:220-224).
struct XorShift64(u64);
impl XorShift64 { fn next(&mut self) -> u64; }  // shifts 13, 7, 17
const SCRIPT_SEED: u64 = 0x5EED_0000_0000_0042;

/// One scripted headless client: a local link plus its replica view.
pub struct ScriptedClient {
    pub peer: PeerHandle,          // crpg_net::protocol's handle
    pub link: LocalClient,         // crates/crpg-server/src/run.rs:818
    pub view: ReplicaWorld,        // crates/crpg-net/src/replica.rs:121
    pub script: Vec<ScriptOp>,     // one entry per tick, in tick order
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptOp {
    Idle,                          // send nothing this tick
    Act { ability: u64, target: u64 },  // lane-0 intent by index into the fixture grants
    EndTurn,                       // lane-0 end-turn for the client's actor
}

/// Builds `n` clients on one host: `LocalHost::new`, one `attach` per peer,
/// each with a fresh `ReplicaWorld` and its script. Panics on `LocalError`.
pub fn attach_clients(host: Host, scripts: Vec<(PeerHandle, Vec<ScriptOp>)>) -> (LocalHost, Vec<ScriptedClient>);

/// One driven tick for every client, in ascending `PeerHandle` order
/// (InMemoryTransport's own order, crates/crpg-net/src/sim.rs:572):
/// send this tick's `ScriptOp` bytes, `HostLoop` one step at `now_ms`,
/// pump the sim fabric once, feed every due delta into each `view`.
/// Returns the host's `authority_hash` after the tick.
pub fn drive_tick(host: &mut HostLoop<LocalHost>, clients: &mut [ScriptedClient], fabric: &mut InMemoryTransport, faults: &FaultSchedule, now_ms: u64) -> [u8; 32];

/// The semantic comparison (Reading 4): the replica's visible state equals
/// the host's filtered projection for that peer — `project_snapshot` for
/// the join tick, `project_delta` after — compared op-for-op, never hashed.
pub fn assert_view_matches(host: &Host, client: &ScriptedClient);

/// Host-level probe: one raw client message against the host, returning the
/// host's verdict without panicking.
pub struct ProbeReport {
    pub rejections: Vec<Rejection>,  // T040a's type, tasks/T040.md:227-244
    pub authority_before: [u8; 32],
    pub authority_after: [u8; 32],
}
pub fn probe(host: &mut Host, bytes: &[u8], peer: PeerHandle) -> ProbeReport;
```
`PeerHandle`, `Rejection`, `FaultSchedule`, `InMemoryTransport` are used by
path, never re-exported or wrapped. Field docs are one line each in the
implementation (`#![warn(missing_docs)]` covers tests through clippy's
`--all-targets`, `crates/crpg-testkit/AGENTS.md:230-239`).

## Behaviour
Normative. Each item names the tests that prove it. "Unchanged" means
`authority_hash`, captures, executions and every delivery entry as before the
probe. **First failure wins** in every ordered list.

B-1. **A scripted pair connects, moves, fights and ends identical.**
`two_client`: two clients attach (ascending `PeerHandle`), run one scripted
fight to its terminal tick on `combat_basic` (Reading 7), and after every
tick `assert_view_matches` holds for both. *Tests:* D1.
B-2. **5,000-tick loss/jitter desync.** `headless_desync`: 5,000 driven ticks
at 20 Hz injected time (`now_ms = 50 * t`, the `HostLoop` grid,
`crates/crpg-server/src/run.rs:4`) through `FaultSchedule { loss_every: 7,
dup_every: 11, reorder_depth: 2 }` (Reading 3); sparse valid inputs (one
scripted `Act` every 50 ticks) plus one rejected input every 200 ticks; after
every tick both clients' views match the host projection, and the authority
hash is recorded every 1,000 ticks. *Tests:* D2.
B-3. **Reconnect within grace rejoins identical.** A client whose link drops
mid-fight and resumes with a live `SessionFence`
(`crates/crpg-net/src/resume.rs:74-84`) rejoins, replays its missed deltas,
and its view matches the other client's op-for-op at the next tick. A resume
past the fence is refused. *Tests:* D3.
B-4. **Gate-13 dedicated smoke.** `dedicated_smoke`: `QuicHost` started with
`start_resumable` on loopback, two scripted real-QUIC clients (Q1) connect,
move, fight and end with identical views; the suite passes at 1x contention
on Windows/MSVC and Linux/GNU with no wall-clock pacing — only the rig's own
deadline guards. `two_client` (D1) is the embedded leg. *Tests:* G1.
B-5. **Unowned intents are rejected, never applied.** An intent naming an
entity outside the sender's control grant is refused (`NotAuthorized`), the
host reports its `Rejection` with the fixed `Display`, and the authority is
unchanged. *Tests:* H1.
B-6. **Illegal-time intents are rejected.** An intent with a stale or future
`observed_tick` outside the admission window is refused; the same intent at
the current tick is admitted. *Tests:* H2.
B-7. **Floods are held, never silently lost.** An over-rate burst is held
with `QueueFull` and retried, never dropped without a receipt — the
`host_quic.rs` stall semantics (`crates/crpg-server/tests/host_quic.rs:1921-1940`).
Sustained refusal beyond the queue is a clean rejection, never a panic.
*Tests:* H3.
B-8. **Replays are cached, including non-canonical aliases.** A same-`seq`
retry with identical bytes is a cached retry; the same intent re-encoded
non-canonically (non-minimal varint or ULID alias, the T042 handover,
`tasks/T042.md:694-696`) is also a cached retry through `Host::canonical`,
never `SeqConflict`. *Tests:* H4.
B-9. **Out-of-interest requests are rejected.** A request for an entity
outside the peer's interest set is refused; a granted entity resolves
(`crates/crpg-net/src/interest.rs:215` is the wire-level rule T042 indexes,
`tasks/T042.md:443`). *Tests:* H5.
B-10. **Oversized and malformed frames fail clean at the host.** A frame over
the 4,096 cap or failing decode is refused with its `Rejection` and fixed
text (T042's six texts, `tasks/T042.md:507-514`); nothing panics, nothing is
half-applied. *Tests:* H6.
B-11. **Seeded reruns are identical.** The same seed twice gives identical
views and identical authority hashes; and the first three
`XorShift64(SCRIPT_SEED).next()` values are T042's pinned triple
(`0x4B10DA10A18C50C2`, `0x8788B83B180538E3`, `0x18B554431CC36A52`,
`tasks/T042.md:397-399`), proving the same generator. *Tests:* D4.
B-12. **Every rejection carries its log-line material.** Each B-5–B-10
rejection yields a `Rejection` whose `Display` is one of the six fixed texts,
and — where T040a's observer has landed — the intake observer fires once per
intake with that report (`tasks/T040.md:369-373`); otherwise the suite
asserts on the pump report directly. *Tests:* H1–H6.

## Errors
No new error type or variant in any crate. Every host-level refusal in B-5–
B-10 surfaces as T040a's `Rejection` (`tasks/T040.md:227-252`) with one of
T042's six fixed `Display` texts (`tasks/T042.md:507-514`), which the host
maps to receipt codes (`crates/crpg-server/src/host.rs:3959-3964`):

| Item | Value |
|---|---|
| intent for an entity outside the sender's grant | refused `NotAuthorized`; `Rejection` with its fixed text; authority unchanged (B-5) |
| intent outside the admission time window | refused with its fixed text; authority unchanged (B-6) |
| over-rate burst within the queue | held `QueueFull` and retried, never a rejection and never silent loss (B-7) |
| sustained flood beyond the queue | clean rejection with its fixed text, never a panic (B-7) |
| same-`seq` retry, canonical or alias encoding | cached retry through `Host::canonical`, never `SeqConflict` (B-8) |
| request outside the peer's interest set | refused with its fixed text (B-9) |
| oversized or undecodable frame | refused with its fixed text; nothing half-applied (B-10) |
| resume past the session fence | refused; the fence (`crates/crpg-net/src/resume.rs:74`) is the only input producing `Accepted` (B-3) |
| any host panic during a probe | the suite fails: a panic anywhere is a failure, never a verdict |

A failing case is never answered by a new variant, a changed text or a host
fix in T041 (the T042 S-1 rule, `tasks/T042.md:425-428`, applies at the host
level too): the implementer stops and reports the case with its input bytes.

## Tests
Public API only, plus the suite-local `headless.rs` shapes of "Interface".
Every case has a control that changes one input and flips the expectation;
each assertion message names the tick, the peer and the script index. No
clock, sleep, thread, `#[should_panic]` or unseeded RNG. No golden reads or
writes anywhere (Reading 8). Fixture: `combat_basic` through the T016c
loader with T037e's in-memory movement edit (Reading 7); `now_ms = 50 * t`.

- **D1 `scripted_pair_fights_to_identical_views`** (B-1, embedded gate-13
  leg). Two clients, scripted fight to the terminal tick: after every tick
  both views match the host projection op-for-op. *Control:* a third run
  with client B's script replaced by all-`Idle` ends with views that differ
  (the fight resolves differently). *Fails if:* any tick matches for one
  client but not the other, or order of `drive_tick` matters.
- **D2 `five_thousand_tick_loss_jitter_desync`** (B-2). 5,000 ticks under
  `{ loss_every: 7, dup_every: 11, reorder_depth: 2 }`, one scripted `Act`
  every 50 ticks (100 acts) and one rejected input every 200 ticks (25
  rejections); views match after every tick; 5 authority hashes recorded
  (every 1,000 ticks). *Control:* the same script with no faults gives the
  same 5 hashes and the same terminal views. *Fails if:* any tick's view
  differs, or the lossless control diverges from the impaired run's
  authority.
- **D3 `resume_within_grace_rejoins_identical`** (B-3). Client B's link drops
  at tick 1,000 of D2's script and resumes at 1,100 with a live fence: at
  tick 1,101 both views match op-for-op. *Control:* the same resume with an
  expired fence is refused and B's view stays behind. *Fails if:* the
  rejoined view skips or duplicates a delta.
- **D4 `seeded_rerun_is_identical`** (B-11). D2's script twice: identical
  views every tick and identical 5 hashes; plus the generator check —
  `XorShift64(SCRIPT_SEED)` yields T042's pinned triple in order. *Control:*
  seed `SCRIPT_SEED ^ 1` gives a different first value
  (`!= 0x4B10DA10A18C50C2`). *Fails if:* any tick differs between reruns.
- **H1 `unowned_intent_rejected_without_mutation`** (B-5, B-12). Client A
  sends an intent for B's actor: refused `NotAuthorized`, one `Rejection`
  with its fixed text, `authority_before == authority_after`. *Control:* A's
  intent for its own actor is admitted. *Fails if:* the authority moves or
  no `Rejection` surfaces.
- **H2 `illegal_time_intent_rejected`** (B-6, B-12). The same intent with
  `observed_tick` 500 behind and 500 ahead of admission: both refused.
  *Control:* at the current tick it is admitted. *Fails if:* either
  off-window intent applies.
- **H3 `flood_is_held_never_lost`** (B-7, B-12). A burst of 3x the queue
  capacity in one tick: the excess is held `QueueFull` and retried on later
  ticks — every sent frame ends admitted or cleanly rejected, none
  vanish. *Control:* the same burst at 1x is admitted at once. *Fails if:*
  any frame is neither admitted nor rejected after the drain.
- **H4 `noncanonical_retry_is_cached`** (B-8, B-12). A sent intent retried
  with identical bytes, then re-encoded with a non-minimal varint and with
  a ULID alias: all three are cached retries, never `SeqConflict`, and the
  authority applies the intent once. *Control:* a new `seq` with a different
  intent is admitted as new. *Fails if:* an alias retry conflicts or
  double-applies.
- **H5 `out_of_interest_request_rejected`** (B-9, B-12). A request naming an
  entity outside the peer's interest set is refused with its fixed text.
  *Control:* a granted entity resolves. *Fails if:* the ungranted entity is
  disclosed.
- **H6 `oversize_and_malformed_fail_clean`** (B-10, B-12). A 4,097-byte frame
  (`FrameTooLarge`: cap beats version, `tasks/T042.md:282-290`) and a
  one-byte `[0xFF]` frame against a v1 session: both refused with their
  fixed texts, authority unchanged. *Control:* the session's valid frame is
  admitted. *Fails if:* anything panics or half-applies.
- **G1 `dedicated_smoke_loopback_quic`** (B-4). `QuicHost` via
  `start_resumable` on `127.0.0.1:0`, two scripted real-QUIC clients connect,
  move, fight 200 ticks and end with identical views, at 1x contention with
  only the rig's deadline guards. *Control:* a third client with a wrong
  credential gets `AuthRefused` (`CloseCode` code 5, the T040 T1 shape,
  `tasks/T040.md:528-534`) plus its reject material. *Fails if:* the smoke
  needs any sleep, or flakes twice in a row at 1x.

**Mutant table** (the reviewer plants each in a scratch copy of
`tests/support/headless.rs`, runs the named suite, expects failure; a
survivor is a finding):

| # | Mutant | Killed by |
|---|---|---|
| MU1 | `drive_tick` feeds deltas to the first client only | D1, D2 |
| MU2 | `drive_tick` drives clients in script order, not ascending `PeerHandle` | D1 (control), D4 |
| MU3 | `probe` maps alias-encoded retries to `SeqConflict` | H4 |
| MU4 | `probe` drops `QueueFull`-held frames instead of retrying | H3 |
| MU5 | `attach_clients` accepts resumes past the fence | D3 (control) |
| MU6 | `assert_view_matches` compares entity counts only, not ops | H5, D2 |
| MU7 | `XorShift64::next` shifts 13, 7, 15 | D4 |

## What must not change
- Goldens, byte for byte (none added, none read): the six files under
  `crates/crpg-testkit/goldens/` (`replay_basic`, `combat_basic`,
  `srd_lite`, each `rust-1.98.0_x86_64-pc-windows-msvc` and
  `x86_64-unknown-linux-gnu`, `test-default`).
- Every existing test file and `tests/support/` in every crate, byte for
  byte and passing — including `crates/crpg-net/tests/conformance.rs` and
  `malicious_client.rs`, `crates/crpg-server/tests/host_quic.rs` and
  `crates/crpg-testkit/tests/host_movement.rs`.
- No `src/` change in any crate; no public item, error variant or `Display`
  text changes anywhere; `crates/crpg-testkit/src/` stays exactly as is.
- `crates/crpg-testkit/Cargo.toml` except the one conditional Q1 line;
  `Cargo.lock` grows by at most that edge entry; `deny.toml`, workflows,
  `rust-toolchain.toml`, `crpg-contracts`, `tools/` untouched.
- Every existing CI job, step and trigger; no new job (gate 13 runs inside
  the existing workspace-test matrix on both legs).

## Dependency edges
| Edge | Section | `ALLOWED` |
|---|---|---|
| (none new by default) | — | `crpg-testkit: (None, {"crpg-godot"})` (`tools/lint/deps.py:68`): every crate T041 touches — `crpg-server`, `crpg-net`, `crpg-nav`, `crpg-ai`, `crpg-data`, `crpg-rules`, `crpg-core`, `crpg-sim` — is already a normal or dev-dependency (`crates/crpg-testkit/Cargo.toml:11-39`), so the lockfile is untouched |
| `crpg-testkit` → `crpg-net-quic` (dev-only, path; only if Q1 approved) | `[dev-dependencies]` | permitted: same `ALLOWED` row; needs the user's yes (`AGENTS.md:52`, "Do not add dependencies") |

No external crate and no new lockfile package either way: `crpg-net-quic`
is a workspace member already in the graph through `crpg-server`
(`crates/crpg-testkit/AGENTS.md:141-143`). No cycle (`crpg-net-quic` may not
reach `crpg-testkit`). `deps.py` is not edited. The no-new-dependency
alternative is Reading 2's fallback: G1 deferred to T070's binary harness
and the dedicated leg proved at the library level only.

## Out of scope
- Spawning the T040 binary or writing log files/lines (T040b owns both,
  `tasks/T040.md:423-426`, `:433-452`); the T040a intake observer itself.
- Wire decoders, client receivers, `crpg-net-quic` framing — T042a owns the
  first (`tasks/T042.md:61`), T042b the hostile-server receivers
  (`tasks/BACKLOG.md:347`; `tasks/T042.md:680-687`).
- AI-driven play (`Host::drive_ai`, `tasks/T153.md:60-66`); the run loop and
  hooks (`Hosting::drive_ai`, `crates/crpg-server/src/run.rs:45`).
- Benchmarks and thresholds (T044), campaign-wide breadth and full §17.2
  acceptance (T070, `docs/IMPLEMENTATION_PLAN.md:335`), player-hosted games
  (D65b), any new golden or fixture.
- Any decoder, host or text fix: a failing case stops and reports (Errors).

## Open questions

## Agent log
- 2026-11-19 (UTC) · opencode-go/muse-spark-1.3-contributor + T041 draft b · Skeleton with all section headings.
