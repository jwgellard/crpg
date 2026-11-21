# crpg-testkit — architecture

Shared test machinery, built once so it cannot fork across crates.

**State:** the hash-sequence harness is complete (T008b): scripted
scenario runs, golden-file write/compare, and the convention later tasks
use. The versioned replay is merged (T009a + T009c, 2026-09-07):
opaque-payload record/parse/validate/playback through public sim APIs, an
exact-tick divergence report reusing the harness mismatch, and per-target
checked-in replay goldens under the two-target policy below. Required native
Windows/MSVC and genuine WSL Ubuntu Linux/GNU gates passed (see the
[T009c completion record](../../tasks/T009c.md)); T009b's thin
`crpgc replay` wrapper in `crpg-cli` is merged (2026-09-07; see
[tasks/T009b.md](../../tasks/T009b.md)). Fixture
campaigns, contract conformance suites and further helpers are planned,
each owned by the task that first needs it.

Decisions: [ADR-0009](../adr/0009-determinism-scope.md) for the scope the
harness operates under; [ADR-0010](../adr/0010-testkit-mismatch-shape.md) for
the truthful mismatch shape; E005 for its one-way dependency direction.
ADR-0012/T009c supersedes only ADR-0009 Decision 3's Linux-only selection.
Working contract: [`crates/crpg-testkit/AGENTS.md`](../../crates/crpg-testkit/AGENTS.md).

---

## Position

`crpg-testkit` may reach any simulation crate except the Godot bridge — the
one crate in the ALLOWED table with an exclusion instead of a closed set.
Under E005's topology, a testkit that pulled the engine would pull it into
every higher crate using the helpers — which is why the bridge exclusion
exists. Today it
reaches `crpg-sim` (normal) and `crpg-core`, `crpg-data`, `crpg-rules`,
`crpg-server`, `crpg-net` and `crpg-nav` (tests only), and `crpg-cli` takes
its normal edge at runtime. Later helpers add edges with their tasks: each
justified in its task file.

The direction is one-way by E005: testkit reaches down through sim, while
core, data, rules, and sim never reach back, including through a
dev-dependency. Their self-contained tests stay in their own crates and their
cross-layer integration tests live here. This is an architecture boundary,
not a claim that Cargo cannot compile dev-only cycles.

## Modules

- **`harness` — `run_hash_sequence`, goldens, errors.** The fixed
  script→tick→hash interleaving, the line-hex golden format with `#` scope
  headers, and exact-tick mismatch reports carrying only present sides
  (ADR-0010). T009's replay and T016's combat are named future consumers;
  the module serves them, it does not anticipate them.
- **`replay` — `Replay`, playback, divergence.** The versioned `.replay`
  schema (format 1: seed, campaign/engine identity, tick count, ordered
  inputs with opaque `serde_json::Value` payloads), validation before
  playback in fixed check order, the input→tick→hash interleaving that
  keeps replay hashes comparable with harness hashes, and
  `play_and_verify`: the end-to-end gate that reuses `verify_golden` and
  attaches replay identity to the `Mismatch` as a boxed `ReplayDivergence`.
  Payload meaning stays with the caller (`ApplyInput` over public `World`
  APIs); no game-specific input vocabulary lives here. T009b's `crpgc
  replay` is the thin CLI consumer; T016's combat is the next behavioural
  one.

## Today versus planned

| Exists (T008b + T009a) | Planned (owner) |
|---|---|
| Hash sequences, golden write/compare, error shape | Fixture campaigns (first data task needing one) |
| Replay format + validation + playback + typed divergence | Contract conformance suites (spec §15.3, first backend) |
| Determinism/tamper/shape/replay-behaviour tests | — |
| Scope-header convention and independent Windows/MSVC and Linux/GNU golden gates (T009c, step 9; merged 2026-09-07); T009b `crpgc replay` wrapper (merged 2026-09-07) | T016 headless combat (next replay consumer) |

## Target-scoped replay policy

Windows `x86_64-pc-windows-msvc` is the primary development, product,
release-gating, and behavioural-baseline target. Linux
`x86_64-unknown-linux-gnu` is fully supported for dedicated servers, headless
CLI/tooling, server-side extensibility, and testing; failures are defects.
Windows supports client, editor, embedded single-player server, dedicated
server, and CLI. Linux GUI client/editor support is not promised, and Linux
headless support must not depend on Godot.

The portable input remains `crates/crpg-testkit/fixtures/replay_basic.replay`.
Under `crates/crpg-testkit/goldens/`, the independent baselines are:

- `replay_basic_rust-1.98.0_x86_64-pc-windows-msvc_test-default.golden`
- `replay_basic_rust-1.98.0_x86_64-unknown-linux-gnu_test-default.golden`

Real `play_and_verify` comparisons select their paths at compile time using
`cfg(all(target_os = "windows", target_arch = "x86_64", target_env = "msvc", debug_assertions))`
and
`cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu", debug_assertions))`,
respectively. Other targets/profiles retain portable parse/play repeatability
and shape coverage without reading either golden. Runtime platform selection,
tolerance, and cross-platform equality assertions are forbidden. Each target
must match its own baseline exactly; a missing baseline is `Io(NotFound)`,
not a skipped green check.

The pinned Rust 1.98.0 toolchain, complete target triple, normal test profile,
and default features define each scope; changing any element requires a
reviewed re-baseline. Each baseline is independently generated on its genuine
native target through `read_replay` -> `play_replay` -> `write_golden`, never
copied from the other. The genuine T009a Linux record is preserved; renaming
that artifact is not regeneration. T009c native re-verification passed as
recorded in its completion record.
Neither Windows' primary baseline nor Linux's server regression baseline
promises cross-platform lockstep. Replay API, format, comparison semantics,
and ADR-0009 hash-exclusion governance remain unchanged.

One reusable platform-neutral authoritative server implementation must serve
Windows embedded single-player, Windows dedicated, and Linux dedicated hosts.
In-process transport preserves the client/server authority boundary: clients
never mutate authoritative state directly. E012/E022 own the unresolved
package/API placement; E023 owns native-extension ABI/loading governance.
Platform-specific hosting stays above core/rules/sim. Future real product
builds and smoke tests activate with their capabilities under E020, not as
placeholder jobs.

## Combat replay (T016c, tests-only)

The first behavioural replay consumer after T009b's CLI is T016c's
headless combat suite. `tests/support/combat.rs` (included directly from
`tests/combat_replay.rs`, never from `src/`) owns the production content
loader — repo-root `campaigns/fixtures/combat_basic/` through
`load_campaign` with engine `0.1.0`, asserting the 14-file map, the 11-id
index, empty `validate`, and canonical-source parity with
`rulesets/minimal-d6/` — and the caller-owned combat adapter that decodes
the versioned `{"combat": 1, "op": "init" | "attack"}` grammar onto
`start_encounter` / `perform_action` with strict unknown-field rejection.
`EncounterSpec` reference maps are assembled transiently per initialization
from the owned `LoadedCampaign`, never as self-referential state; authored
placement ULIDs bind to runtime entities by walking `world.combatants()` in
insertion order against `encounter.participants` in authored order.
Observation is behavior-free: shared `&World` reads plus read-only
`serde_json::to_vec(&world)` snapshots after every accepted attack and
detached `World` clones for terminal, trailing-tick suffix, and death-event
assertions (the `EventQueue` exposes no read iterator). The portable input
is `fixtures/combat_basic.replay`; the independent native baselines are
`goldens/combat_basic_rust-1.98.0_x86_64-pc-windows-msvc_test-default.golden`
and `goldens/combat_basic_rust-1.98.0_x86_64-unknown-linux-gnu_test-default.golden`,
each generated on its genuine native target through the production
`read_replay` → `play_replay` → `write_golden` path. The generic harness
and replay stay payload-agnostic; T016d writes its own CLI-owned adapter
against the same grammar.

## Second-ruleset replay (T017e, specified before source)

The second behavioural replay consumer is T017e's srd suite.
`tests/support/srd.rs` (included directly from `tests/srd_replay.rs`,
never from `src/`) owns the `combat_srd` production content loader —
repo-root `campaigns/fixtures/combat_srd/` through `load_campaign` with
engine `0.1.0`, asserting the 17-file map, the 14-id index, empty
`validate`, and canonical-source parity with `rulesets/srd-lite/` — and the
caller-owned srd adapter that decodes the versioned `{"combat": 1}` grammar
extended with the `end` op onto `start_encounter` / `perform_action` with
strict unknown-field rejection. Per-attack trace entries carry faces,
modified totals, margins, outcomes, damage, both pool balances, attach
state, and the turn holder; `end` inputs advance with no trace entry.
Observation stays behavior-free (shared reads, read-only snapshots,
detached clones). The portable input is `fixtures/combat_srd.replay` (a
pinned ascending-probe seed with `K <= 12` actions plus `T = 4` trailing
ticks); the independent native baselines are the two
`goldens/srd_lite_rust-1.98.0_*_test-default.golden` files, each generated
on its genuine native target. B5 writes its own CLI-owned proof against the
same grammar.

As-built: seed 285 with `K = 9` and `TOTAL_TICKS = 14`; the 13-test
`srd_replay` suite pins the artifact, oracles, continuation, terminal, and
per-target goldens; `combat_basic` artifacts stay byte-identical.

## Authored movement through the host (T037e, tests-only)

`tests/host_movement.rs` is the first suite to take the host as a
dev-dependency: it proves T037a–d's movement surface end to end on real
authored content with no source change anywhere. It includes
`tests/support/combat.rs` for `combat_basic` and gives the loaded ruleset
T148's `movement` block in memory before the start (speed and step 65,536
raw, one primary-pool unit per step, plus `round_ticks: 4` for the real-time
case), starts `first-blood` through `crpg-sim`, and wraps a `HistoryWorld` in
`Host::from_history`. K1 binds the loaded `Area.bounds` with
`bind_area_bounds` and checks the revision is nav's bake of exactly those
bounds; K2 moves the goblin (contract erratum, session 74: initiative 9 acts
first) by `DeclareMove`, pinning the page's receipt, `Moved` notice and spent
primary pool; K3 walks the hero through a lane-1 `MoveTo` sent by
`MoveOutbox` at the authored speed, taking the receipt and each tick's
sample into `ReplicaWorldV4`; K4 saves a history-version-3 `Moved`, loads it
at incarnation 2 and re-binds the same bounds, which clear nothing and leave
the hash equal. The oracle is a clone of the started `HistoryWorld` driven by
the same sim calls, with every route computed by calling `crpg_nav` directly.
The three host-side edges (`crpg-server`, `crpg-net`, `crpg-nav`) are
`[dev-dependencies]` only; no normal edge changed.

## MVP AI decision suite (T056b, tests-only)

`crpg-ai` may not read `crpg-data`, so its own U1-U8 suite cannot start an
encounter (T056 Reading 13); the behaviour suite lives here instead.
`tests/ai_fixture/mod.rs` is a private fixture module included by
`tests/ai_mvp.rs` through `mod ai_fixture;` (in a subdirectory, so it is not
a test target of its own). Its first line is `#![allow(dead_code)]` because
each includer uses only part of it. The fixture re-authors sim's own
documents value for value — `Fixture::standard()` from
`crates/crpg-sim/tests/support/mod.rs` and `move_fixture()` from
`tests/support/movement_schedule.rs` — parsed through `crpg-data`'s
production types, then edits them with typed functions (`ranged`, `far`,
`near`, `trio`, `lethal`, `shared`, `twin`, `tune`), plus the contract's
`m()`/`ms()` bakes, `rev`/`bind`, `sides`, and `apply_move` (turn-based
`perform_move` over `host_route`'s route; real-time `set_route`).
`start`/`start_rt` start the encounter from those documents and assert B
acts first in turn-based play.

`tests/ai_mvp.rs` is G1-G22 plus the fix round's G23-G34: each decision is
pinned to the contract's hand-computed destination, balance or `Decision`
variant, applied through `perform_action`, `perform_move` and `set_route`, and
every case carries a control that changes one input and flips the expectation.
The first suite proves M-1 to M-9 over real turn-based and real-time encounters
with a real bound mesh; M-8's two validation calls stay declared defence in
depth, because no fixture can reach them. G23-G34 close the mutant review's
gaps in the ordered rules: the nearest-then-`EntityId` approach target and
least-ULID intent, the real-time `Stop`/`Halt`/`Hold` precedence, the
saturating standoff and floored step budget, the full-range hold band, the
`nearest_on_mesh` goal, the summed reserve, the moverless `u64::MAX` distance
and the one-raw walk.

The two dev-only edges this suite adds are `crpg-ai` (the decision under
test) and `crpg-nav` (the mesh the host routes on), approved in D62;
`ALLOWED` already permits testkit -> anything but `crpg-godot`. No `src/`
change, no public export, and no golden.

## Headless clients and the desync suite (T041, tests-only)

The M4 exit-criteria proofs (scripted headless clients, the 5,000-tick
loss/jitter desync, two clients with identical visible state, the
host-level malicious table, and the gate-13 embedded smoke) live entirely
in `tests/`, included per suite with
`#[path = "support/headless.rs"] mod headless;`. `tests/support/headless.rs`
is the shared rig (T041a): the `combat_basic` `Content` loader and
`moved_ruleset` copied from `host_movement.rs`, a `start` that wraps a
`HistoryWorld` in `Host::from_history` and binds the area bounds, the two
`Grants` profiles, and a QUIC-shaped simulated network — `Channel`s whose
`Faults::SPEC` (3 % loss, 2-cycle latency, 0–4 jitter, 4-cycle rto, 1 %
duplication) delay reliable frames and drop/duplicate/reorder datagrams,
with every draw from `DeterministicRng` so a run is a pure function of
`(seed, name, calls)`. A `HeadlessClient` speaks protocol v4 exactly as the
Godot bridge will (`0x82` snapshots through `ReassemblyPoolV4`, a
`ReplicaWorldV4`, lane-0 intents, a lane-1 `MoveOutbox`, samples) and runs a
`Script` that reads only its own replica. The `Arena` drives the real
`HostLoop<LocalHost>`; the simulated network sits on the client side of the
link, so the client never acknowledges on the host's behalf. The desync
oracle is a twin session bound right after each faulted client with
identical grants, plus an independent `Mirror` — a clone of the started
`HistoryWorld` driven with the same sim calls — so `authority_hash` must
equal the mirror's `history_hash` and `canon` must agree across twins.
`canon`/`world_view` leave out the ticks and event sequences that
legitimately differ per session. `tests/headless_harness.rs` is the H1–H8
suite proving the network, the arena, the scripts, the settle/resume cycle
and the comparison. T041b adds `tests/desync.rs`. D1
(`five_thousand_ticks_under_loss_and_jitter`) runs L1 — two patrolling
players and an observer that intrudes every 250 cycles, each with a
faultless twin — as five blocks of 1,000 `Faults::SPEC` steps, asserting
`authority() == mirror_hash()` after every 100th step and the full
settle/checkpoint before resuming. As built, the run's counts are A 135 and
B 136 applied `DeclareMove`s, 19 intrusions (all `NotAuthorized`), 83
reliable losses, and 46 lost plus 19 duplicated datagrams; D1 takes about
2.6 s in the debug test profile. D2 (`seeded_runs_repeat`) pins the
authority hashes, `sent` frames and receipt cycles of two 1,000-step runs
against `NET_SEED` and requires `NET_SEED + 1` to change A's receipt
cycles. D3 (`a_frame_lost_for_good_is_caught`) drops one down-0 frame for
good and requires A to diverge from its twin while B, O and the authority
stay converged. D4 (`lane_one_walks_under_loss`) walks both players on lane
1 in real time under `Faults::SPEC` and pins the receipts to exactly `1..=k`,
all `Accepted`, with `k >= 20`. D5 (`two_clients_move_fight_and_agree`) is
the gate-13 embedded leg: both clients move once, strike to one death, and
agree with each other and with the mirror's combatant health. The extra
case `walk_waits_while_moving` pins B-11's moving gate directly — a sample
at the previous destination that is still moving must not send the next leg
— because the harness suite's `outbox.pending()` check cannot see a leg
whose clean receipt clears in the same step. T041c adds
`tests/host_malicious.rs` and the `crpg-net.md` index cells. It runs `Host`
directly (Reading 12), because `LocalHost` discards the pump summary and a
decode failure or an ingress refusal sends no receipt, so the clean
rejection is visible only in `IngestDisposition` and `AdmissionResult`. The
rig is B-21's: `start(TurnBased, None)` with `A` (hero, `Full`), `B` (goblin,
`Full`) and `P` (goblin; the hero present with health only, no
`action_target`) bound in that order through `bind_v4_peer`, each then
published with T037e's `bind_client` steps; the goblin (initiative 9) holds
the first turn and every session maps the hero to `n(1)` and the goblin to
`n(2)`. Each case builds a fresh rig, asserts the exact `AdmissionResult` or
`IngestDisposition`, that `state` (authority hash and `executions`) is
unchanged, that the session is still bound (M9 aside), and carries a control
that flips one input and is applied. The index: M1 unowned actor
(`NotAuthorized`); M2 illegal times (`FutureTick` at `tick + 1`, `StaleTick`
at `tick − 201`, applied at `tick − 200` after `idle(210)`); M3 out of turn
(`IllegalAction`, `last_combat_error`'s `OutOfTurn { actor: hero, active:
Some(goblin) }`); M4 out-of-interest targets (`NotAuthorized` for an unknown
`n(99)` and for `P`'s ungranted `action_target`); M5 a 100-frame flood (the
80-frame burst staged, 20 ingress `RateLimited`, the pump's 80
`UnsupportedVersion`, recovered at 12 s); M6 oversize and truncated frames
(`FrameTooLarge` with nothing staged, then `Malformed`, then the whole seq 1
applied); M7 identical replay (cached `Applied`, no new execution); M8 a seq
gap (`SeqGap`, so seq 1 is still applied) and 257 `FutureTick` seqs that
evict seq 1 (its bytes then `StaleSeq`) while seq 2 stays a cached
`FutureTick`; M9 a conflicting seq 1 (`SeqConflict`, the epoch fenced to
`UnknownPeer`); M10 the non-canonical retry (a non-minimal seq varint, the
lowercase `k` ability alias and the `O`/`0` alias each decode to the first
frame and are cache hits). No source, fixture, manifest or golden changes.

## What consumers inherit

- **Comparable sequences.** Same interleaving everywhere means a hash at
  index *n* means the same thing in every task's test: the world after the
  *n*-th scripted step and tick.
- **Reviewable goldens.** Line-hex under scope headers diffs cleanly; a
  one-tick behaviour change shows as one changed line plus the report naming
  it.
- **Recordable behaviour.** A replay (seed + ordered opaque inputs) plays to
  the same hash sequence on the same build, so any consumer — T009b's CLI,
  T016's combat, future sweeps — regresses behaviour through one artifact
  and one gate instead of a private dialect each.
- **No engine in the test graph.** Depending on testkit never drags Godot,
  windowing, or audio into a headless test — the exclusion that keeps CI
  fast is structural, not careful.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark + T008b · Wrote the crate doc for the harness: testkit's position and one-way direction, the module map, today-vs-planned with T009/T016/E020 as named consumers, and what consumers inherit.
- 2026-09-06 (UTC) · opencode/muse-spark + testkit mismatch hardening · Recorded ADR-0010, corrected the E005 dev-dependency claim to proposed-not-fact, and noted truthful mismatch sides.
- 2026-09-06 (UTC) · opencode/gpt-5.6-sol + E005 decision · Recorded the ratified lower-layer test ownership boundary and removed the incorrect Cargo-cycle justification.
- 2026-09-06 (UTC) · opencode/gpt-5.6-sol + E020 decision · Assigned canonical-Linux enforcement to T009a under the existing workspace-test job instead of planning a placeholder CI job.
- 2026-09-06 (UTC) · opencode/muse-spark + T009a · Recorded the replay module (opaque payloads, input→tick→hash ordering, boxed divergence), the checked-in canonical-Linux fixture pair, and CI step 9 going live with T009b/T016 as the named consumers.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c · Aligned the architecture with Windows-primary and fully supported Linux headless hosting and independent exact-build golden scopes. Distinguished unmerged implementation from pending native verification and deferred host/extension implementation decisions to their owners.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c verification alignment · Replaced active pending-verification wording with the reported native gate passes and linked the completion record. Kept T009c the next review/merge priority and T009b blocked on landing, without claiming either replay task committed or merged.
- 2026-09-07 (UTC) · opencode/big-pickle + T009a/T009c merged · Updated the state and table rows to the merged replay and two-target golden gates; T009b is next.
- 2026-09-07 (UTC) · opencode/big-pickle + T009b merged · Updated the policy paragraph and the Exists/Planned table to the merged `crpgc replay` wrapper; the next replay consumer is T016.
- 2026-09-26 (UTC) · opencode/muse-spark + T016c crate opening · Recorded the tests-only combat adapter, artifact, observation, and independent native golden ownership actually built; generic harness and replay stay payload-agnostic.
- 2026-09-27 (UTC) · opencode/muse-spark + T017e crate opening · Recorded the tests-only srd adapter, extended grammar with the end op, per-attack trace shape, and independent native golden ownership before source implementation.
- 2026-09-27 (UTC) · opencode/muse-spark + T017e implementation · Aligned the module with the as-built srd replay (seed 285, K = 9, per-target goldens, combat_basic untouched).
- 2026-11-19 (UTC) · opencode-go/deepseek-v4.1-flash + T037e · Recorded the authored-movement suite and its three tests-only host edges (`crpg-server`, `crpg-net`, `crpg-nav`), so the crate doc names what `host_movement.rs` proves and why no normal edge moved. No source, fixture, or golden changed.
- 2026-11-15 (UTC) · opencode-go/deepseek-v4.1-flash + T056b · Recorded the tests-only MVP AI decision suite: the private fixture's provenance and typed edits, the G1-G22 shape with its controls, and the two dev-only edges, so the crate doc names the suite's owner and its boundary.
- 2026-11-15 (UTC) · opencode-go/deepseek-v4.1-flash + T056b fix round 1 · Added G23-G34 and the fixture edits they need (`reversed_abilities`, `step_cost`, `extra_cost`, `place_c`, a member registered without movement) to pin the mutant review's ordered rules exactly, so the suite fails on each named bug rather than passing by coincidence.
- 2026-11-19 (UTC) · opencode-go/deepseek-v4-pro + T041a · Recorded the tests-only headless-client rig (`tests/support/headless.rs`): the `combat_basic` start, the QUIC-shaped simulated network, the v4 `HeadlessClient`, the `Arena` over `HostLoop<LocalHost>`, the twin-session and independent-mirror oracles, and the H1–H8 harness suite; no source, fixture, manifest or golden changed.
- 2026-11-20 (UTC) · opencode-go/deepseek-v4.1-flash + T041b · Recorded the as-built D1 counts (135/136 applied moves, 19 intrusions, 83 reliable losses, 46 lost and 19 duplicated datagrams, about 2.6 s), the D2–D5 shape, and the extra `walk_waits_while_moving` case that pins B-11's moving gate; no source, fixture, manifest or golden changed.
- 2026-11-20 (UTC) · opencode-go/deepseek-v4.1-flash + T041c · Recorded the host-level malicious table (`tests/host_malicious.rs`, M1–M10) and its B-21 rig on `Host` directly, so the M-table index and the exact dispositions each case pins are named in one place; no source, fixture, manifest or golden changed.
