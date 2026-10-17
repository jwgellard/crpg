# Task backlog

The index of every numbered task. Derived from `docs/CRPG_ENGINE_SPEC.md` §24
(the first eighteen tasks) and §19.1 (the small backlog). One line per task;
detail lives in `tasks/TNNN.md`. Task files distinguish specification readiness
from implementation readiness; a specified successor may still await source
prerequisites. See [the current specification index](SPECIFICATION-READINESS.md).

Status: `done` · `on branch` · `in progress` · `next` · `open` · `blocked` · `human` (needs a
person, not an agent).

`done` means merged to `master`. **`on branch` means the work is finished and
green but not merged** — the Merged column names the branch instead of a date.
Recording finished-but-unmerged work as `done` with a merge date is how the
backlog and `docs/PROJECT_STATE.md` came to disagree with git.

Numbering: spec §24 calls the spikes T1–T3 and the build tasks T4–T18. Task
files are zero-padded (`T004.md`). A letter suffix means the spec's single task
was split into independently reviewable pieces — the split is recorded in the
ADR that motivated it.

---

## Phase 0 — Feasibility spikes

| Task | Status | Merged | Summary |
|---|---|---|---|
| T001 | done | 2026-09-03 | GDExtension rendering spike — go, ADR-0003 |
| T002 | done | 2026-09-03 | QUIC movement spike — conditional go, ADR-0004 |
| T002b | done | 2026-09-04 | T2's NAT leg — confirmed failure, cause attributed |
| T003 | done | 2026-09-03 | Lua sandbox spike — go, ADR-0005 |

## Phase 1 — Core skeleton and test harness

| Task | Status | Merged | Summary |
|---|---|---|---|
| T004 | done | 2026-09-03 | Workspace, 15 stub crates, CI on Linux + Windows; retrospective record [T004](T004.md) (2026-09-30, T032) |
| T005 | done | 2026-09-03 | Dependency-direction lint |
| T005b | done | 2026-09-03 | Determinism lint |
| T005c | done | 2026-09-04 | `deny.toml` + `cargo deny` in CI; narrow CI to `push: master` |
| T006a | done | 2026-09-04 | `CoreError`, `EntityId`, `GenerationalArena<T>` |
| — | done | 2026-09-04 | Review 1 follow-up: arena guard hole, lint gaps, lint self-tests in CI |
| — | done | 2026-09-04 | Review 2 follow-up: arena exhaustion boundary, lint blind spots, CI `--locked`, licences |
| T006b | done | 2026-09-05 | `Fx16_16` fixed point: saturating integer arithmetic, floor division, exact decimal display/parse, raw-integer serde |
| T006c | done | 2026-09-05 | `DeterministicRng`, PCG32 with named sub-streams |
| T006d | done | 2026-09-05 | `Tick`, `RoundCount`, `Ulid` |
| T006e | done | 2026-09-05 | `Interner`, `StatId`, `TagId` |
| T007 | done | 2026-09-06 | `crpg-sim`: `World` (with `EventQueue<SimEvent>`), `ComponentStore<T>`, spawn/despawn/query, `Timeline` container; generic event substrate per ADR-0008 (scoped core exception, see ADR) |
| T008a | done | 2026-09-06 | `state_hash`, fixed-step tick loop, `Timeline` advance rules (`crpg-sim`; scope ADR-0009) |
| T008b | done | 2026-09-06 | Hash-sequence harness + golden convention (`crpg-testkit`; scope ADR-0009) |
| — | done | 2026-09-06 | Review 3 follow-up: sim/core/testkit invariant hardening (finite hash guard, validated Timeline/World/EventQueue loading, truthful Mismatch enum) + ADR-0010/0011 |
| T009a | done | 2026-09-07 | Replay format/playback + genuine original Linux verification preserved; corrected by T009c's target policy |
| T009c | done | 2026-09-07 | Windows-primary/Linux-supported policy + independent native replay goldens (`crpg-testkit`; ADR-0012 supersedes only ADR-0009 Decision 3's Linux-only selection); required native Windows/MSVC and genuine WSL Ubuntu Linux/GNU gates passed and final audit complete; see [completion record](T009c.md) |
| T009b | done | 2026-09-07 | Thin `crpgc replay` wrapper (`crpg-cli`): `play_and_verify` over the T9a API, exit codes 0/1/2, provisional caller-owned reference intents, verify-only; real native-golden verification on both ADR-0012 targets |

Reported T009c results on each native target: 19 testkit tests (6 harness +
12 portable replay + 1 golden), 135 workspace tests (134 unit/integration + 1 doctest),
and 65 lint self-tests. Full required-gate results and provenance belong to
the [T009c completion record](T009c.md); both tasks merged to `master` on
2026-09-07.

T006a–e are spec §24's single T6, split per ADR-0006. T006a established
`Cargo.toml`, the module layout and `crpg-core/AGENTS.md`; T006b-T006e are
merged. T006e finishes the planned core primitives.

## Security hardening

From the security review, not spec §24. Detail lives in `tasks/S001.md`.

| Task | Status | Merged | Summary |
|---|---|---|---|
| S001 | done | 2026-09-17 | Fork-PR guard over `tools/lint/` and workflows, `build.rs` ban, secret scan, self-hosted-runner rule, `yanked = "deny"` (PR #2, all 9 checks green; branch protection human-done; revert PR #3 closed unmerged) |

## Phase 2 — Campaign data format

| Task | Status | Merged | Summary |
|---|---|---|---|
| **T010** | **done** | 2026-09-17 | `crpg-data`: entity/aggregate schemas, package ids, canonical writer, resolver/lock APIs, loader/index, tick-wait event IR |
| **T011a** | **done** | 2026-09-17 | `crpg-data`: deterministic positioned validation, `Diagnostic` model, 15-diagnostic `broken_references` snapshot + `expected.json` manifest (E004 split half) |
| **T011b** | **done** | 2026-09-17 | Thin `crpgc validate` wrapper (`crpg-cli`): read-only traversal, 0/1/2 exits, gate-8 fixture gate over the data-owned manifest (E004 split half) |
| **T012a** | **done** | 2026-09-18 | `crpg-data`: per-type migration chains, in-memory old-campaign loading, Item `v1 → v2` dummy edge, `migration_v1` golden + non-vacuous coverage gate (E004 split half) |
| **T012b** | **done** | 2026-09-18 | Thin `crpgc migrate` wrapper (`crpg-cli`): explicit source-save over the data loader/writer (E004 split half; PR #5) |
| **T013a** | **done** | 2026-09-18 | `crpg-data`: data-owned `explain_object` introspection prerequisite — canonical report, shared reference enumeration, subtree/ordering rules (prerequisite to CLI-only T013; merged PR #7, `e84b13e`) |
| **T013** | **done** | 2026-09-26 | Scaffolding/introspection CLI in `crpg-cli`: `new`/`schema`/`explain`/`fmt`/`lock`/`run` thin wrappers over landed data/harness APIs, five binary suites plus literal LLM acceptance (merged PR #9, `9a6304f`) |

## Phase 3 — Rules kernel

| Task | Status | Merged | Summary |
|---|---|---|---|
| T014 | **done** | 2026-09-26 | `crpg-rules`: stat/modifier kernel — typed stats, data-selected stacking, breakdowns, derived validation, symbolic persistence, kernel hooks; 122-row table, 1024-case properties, boundary coverage (merged PR #11, `18145d4`) |
| T015 | **done** | 2026-09-26 | `crpg-rules`: dice, outcome tables, and resolution — dice-valued stats, `DiceExpr`, roll/DC modifier targets, `BeforeRoll`/`AfterRoll`/`BeforeDamage`/`AfterDamage` hooks (merged PR #13, `76a48da`) |
| T016 | **done** | 2026-09-27 | `rulesets/minimal-d6` + headless combat — authored combat vocabulary, sim adapter/controller with lifecycle/release (ADRs 0013/0014), invariant repair, cross-layer replay with independent native goldens, thin `crpgc replay --campaign` wrapper (merged PR #14, `4686a73`) |
| T017 | **done** | 2026-09-27 | `rulesets/srd-lite` — the abstraction gate: versioned multi-pool/cost/defense/turn/effect vocabulary with migration edges, generalized sim controller (ADR-0016), cross-layer `combat_srd` replay with independent native goldens, CLI `end`-op proof (merged PR #14, `4686a73`) |

## Phase 4 — Server and networking

| Task | Status | Merged | Summary |
|---|---|---|---|
| T018 | done | 2026-09-28 | `crpg-net` lane-0 combat protocol, bounded postcard codec, simulated transport and conformance (T018a/b/c; PR #16, implementation `2fc54b7`, merge `bc54896`; 75 net tests per native target). Phase 4 host/QUIC/movement/reconnect remains open; follow-on queue: POST-T018.md. |
| T019 | done | 2026-09-29 | `crpg-sim` same-pool affordability repair; 11-test public regression, unchanged replay/goldens, dual-native green (PR #17, implementation `56931eb`, merge `f3d1560`); see [T019](T019.md) |
| T020 | done | 2026-09-30 | `crpg-sim` opt-in authoritative history: `HistoryWorld`, bounded read/ack journal, full-wrapper `history_hash`, per-target `history_v1` goldens (ADR-0017; PR #18, implementation `ea681c6`, merge `a991fe9`); dual-native verified per its completion record. Golden review closed and ADR-0017 accepted (D19); see [T020](T020.md) |
| T021 | done | 2026-09-30 | `crpg-net` explicit v2 event protocol beside frozen v1 (`protocol_v2`/`codec_v2`/`projection_v2`, `events_v2` suite; ADR-0019; PR #18, implementation `e25d0fd`, merge `a991fe9`). Retrospective completion record added from PR #18 dual-OS CI; ADR-0019 accepted (D19); see [T021](T021.md) |
| T022 | done | 2026-10-04 | `crpg-server` host capture/checkpoint slice (ADR-0022, D20): 31-case `host_capture` suite; four contract readings flagged in the completion record (view health bound 3, oldest-first retry eviction, outcome text from the journaled event, no staged reply for unreserved rejections) (PR #23, merge `9157250`; CI green on Windows/MSVC and Linux/GNU); see [T022](T022.md) |
| T023s | done | 2026-10-04 | `crpg-net-quic` setup in two steps (S001 guard): ALLOWED row `{crpg-net}`, I/O-crate manifest ban with 81 lint self-tests and ADR-0024 (PR #25), then the empty stub crate (PR #26); see [T023s](T023s.md) |
| T023 | done | 2026-10-04 | `crpg-net-quic` lane-0 QUIC transport: pinned-certificate TLS 1.3, invitation hello, length-prefixed single-stream framing, bounded queues, ten close codes, v1 limits 64 connections / 60 s idle / 15 s keep-alive; D23 pins, ADR-0025, ADR-0023 addendum; 23 loopback cases incl. impaired relay (PR #26; CI green on Windows/MSVC and Linux/GNU after a Windows relay-timing rig fix); see [T023](T023.md) |
| T023c | done | 2026-10-05 | `crpg-net-quic` tightening-only QUIC flow-control window limits (six fields, v1 unchanged), cases 24–27; C§8.5 case-17 mitigation withdrawn after it failed on Windows (PR #27, merge `efbfdd8`); see [T023c](T023c.md) |
| T023v | done | 2026-10-05 | Vendored quinn-proto 0.11.19 + upstream fix `e556fde` (CONNECTION_CLOSE sent when congestion blocked) under `third_party/` (ADR-0027), CI advisories re-check, case 17 restored, regression case 28 (fails 10/10 unpatched); Windows close-code failure resolved (PR #27, merge `efbfdd8`; Windows green on the PR run and one re-run). Upstream provenance verified 2026-10-06: archive + `e556fde` reproduces the vendored tree byte for byte; see [T023v](T023v.md) |
| T023d | done | 2026-10-06 | `crpg-net-quic`: a refused or `Discard`-closed pending connection now delivers its close when `Closed` is polled at once (release-then-remove, L§3); cases 29–30; negative control 6/6 `TimedOut` (about 10.2 s) with the fix reverted; Linux/GNU gates green (`quic` 30/30 ×5, cases 29–30 20/20, workspace 1,139 passed); Windows/MSVC pending at the PR tip; one PR with T023b, T023d first; see [T023d](T023d.md) Merged in PR #28 (`16f58c8`), CI green on both OSes. |
| T023b | done | 2026-10-06 | `crpg-server` QUIC host adapter (`quic` module, `QuicHost`) over a `crpg-net-quic` path edge (no direct QUIC crates; lock +1 line); ADR-0026 Accepted; invitation credentials (32 bytes, constant-time), one-frame hold, `SessionFenced` fencing, slow-consumer fence, drain-then-unbind, D02 shutdown; 25-case `host_quic` suite incl. T023's five conformance cases and the trace-replay equivalence proof, every A§4 count reproduced; case-9 blocker fixed by T023d (no pump workaround); seven readings approved 2026-10-06; Linux/GNU gates green (`host_quic` 25/25 ×5, workspace 1,164 passed); Windows/MSVC pending at the PR tip; one PR with T023d, T023d first; see [T023b](T023b.md) Merged in PR #28 (`16f58c8`), CI green on both OSes. |
| T024 | contract draft, awaiting approval | — | `crpg-net` snapshot transfer: separate subprotocol (byte 0x81, lane 2), 8 × 4 KiB segments per chunk, byte-level assembly plus a bounded receiver pool with atomic publication and a delivery-cursor cutover gate; 31 test cases; 5 open questions; see [T024](T024.md) |
| T025a | blocked | — | `crpg-net` reconnect/resync mechanisms; T024 + D12 |
| T025b | blocked | — | `crpg-server` authenticated 30-second same-process grace; exact API + T025a |
| T026p | blocked | — | `crpg-net` N-LANES-policy/movement acknowledgments; authoritative movement + D13; movement spec scheduled after T024 (D24); first datagram lane applies the T030 patch (D23) |
| T026 | blocked | — | `crpg-godot` external movement prediction; T026p + host movement adapter + E012 |
| T027a | done | 2026-09-30 | `crpg-sim` persisted single-area World identity (`new_in_area`/`area`/`area_of`), `AreaMismatch`, atomic noncombat `transfer_entity`/`transfer_history_entity` (ADR-0020); 11-test suite, legacy bytes/goldens unchanged (PR #20, merge `c1bc6ab`; CI green on Windows/MSVC and Linux/GNU); see [T027a](T027a.md) |
| T027b | blocked | — | `crpg-net` production interest projection; T027a/T021/T024 + D14 |
| T027c | blocked | — | `crpg-server` authoritative viewer context; exact D03/D14 API + T027b |
| T028 | done | 2026-09-30 | `crpg-sim` shared `validate_action`/`legal_actions` (ADR-0018): single validation path, ULID×EntityId order, EndTurn last, whole-failure 4096 bound; 8-test suite, legacy goldens unchanged (PR #20, merge `c1bc6ab`; CI green on Windows/MSVC and Linux/GNU); see [T028](T028.md) |
| T029a | done | 2026-09-30 | `crpg-data` immutable `ActionSignatureStore` with content-derived `ActionBundleIdentity`, strict bundle reader/writer and bounded `validate_call`/`read_call`/`write_call`; 11-test suite (PR #20, merge `c1bc6ab`; CI green on Windows/MSVC and Linux/GNU); see [T029a](T029a.md) |
| T029b | done | 2026-10-04 | `crpg-script` synchronous trusted bindings (D17, D21): startup validation, staged all-or-nothing dispatch, 18 tests plus 4 doctests (PR #23, merge `9157250`; CI green on Windows/MSVC and Linux/GNU); see [T029b](T029b.md) |
| T038 | done | 2026-10-04 | `crpg-persist` versioned zstd save envelope with S001 input and decompressed caps, and atomic file store (D27b): 34 tests, both format vectors reproduced (PR #23, merge `9157250`; CI green on Windows/MSVC and Linux/GNU); see [T038](T038.md) |
| T039 | done | 2026-10-05 | `crpg-server` host checkpoint save adapter (`save` module over `crpg-persist`), gate 10 active on the T022 fixture authority (1,000-step file save/load continuation, both wire versions), 20 tests (PR #27, merge `efbfdd8`); see [T039](T039.md) |

## Phase 6 — Editor

| Task | Status | Merged | Summary |
|---|---|---|---|
| T058a | done | 2026-10-04 | `crpg-data` public RFC 6901 pointer edit (`edit_document`, `pointer_tokens`) and in-memory structural index rebuild (`campaign_index`), 32 tests, fixture equivalence with the loader; §6 erratum approved by the user (PR #24; CI green on Windows/MSVC and Linux/GNU); see [T058a](T058a.md) |
| T058 | done | 2026-10-04 | `crpg-edit` headless document/command/undo/validation API: nine closed commands, atomic batches, byte-exact bounded undo/redo, `crpg-data`-only validation, pure save plan; 49 tests incl. 2×256-case property tests; rename-onto-lock erratum approved (PR #25; CI green on Windows/MSVC and Linux/GNU); see [T058](T058.md) |

## Independent preparation and tooling

| Task | Status | Merged | Summary |
|---|---|---|---|
| T030 | done | 2026-09-30 | quinn 0.11.12 / quinn-proto 0.11.19 investigation dossier in `docs/reviews/T030-quinn/` (PR #20, merge `c1bc6ab`): verified sources, 2049-packet dedup patch with independent-model reproduction, end-to-end symptom not reproduced. T023 stays blocked on its open decisions (windows-sys bans, dependency approval) and its Windows probe run; see [T030](T030.md) |
| T031 | done | 2026-09-30 | `tools/preflight.sh` / `tools/preflight.ps1` gate runners (PR #20, merge `c1bc6ab`). Native Windows run of `tools/tests` supplied by the D26 CI step: `lint-selftest (windows-latest)` green on PR #21 (merge `ac137ac`, 2026-10-01); see [T031](T031.md) |
| T032 | done | 2026-09-30 | Documentation reconciliation: E003/E007/E012/E021 resolved, E013 spec part done (README pending), retrospective `tasks/T004.md`, `docs/adr/0000-template.md` (PR #20, merge `c1bc6ab`); see [T032](T032.md) |
| T033 | done | 2026-10-01 | Documentation (PR #21, merge `ac137ac`, all nine checks green): remaining E-task wording (E010, E013 README, E017, E018, E019, E022) reconciled with D01–D18 and the as-built code; E023 deferred; see [T033](T033.md) |

The delegated path from here to spec Phase 12 — work-package kinds, waves,
concurrency limits and human gates — is [docs/IMPLEMENTATION_PLAN.md](../docs/IMPLEMENTATION_PLAN.md).
Its T034+ ids are reserved places in that order, not written contracts.

---

## Carried decisions and blockers

- **quinn dedup window (quinn-rs/quinn#2710).** ADR-0004 flags it unresolved.
  It does not block T018, which uses the in-memory transport — it blocks the
  *quinn* transport that follows T018. Decide (carry a patch, or wait on
  upstream) before that task is written.
- **NAT traversal.** T002b confirmed a server behind an unmodified home router
  is unreachable, as spec §7.7 anticipated. Scoped future work: a UPnP/IGD
  client (`igd` crate) or documented manual port forwarding for operators.
  Not on the critical path until there is a real server to reach.
- **ADR-0006 Accepted** on 2026-09-04. Its four decisions (generational arena
  in `crpg-core`, `Fx16_16` saturating/floor, PCG32 sub-streams in a
  `BTreeMap`, interned ids runtime-only) govern T006a–e, T007, T008a/b and T014.
  No longer a blocker.
- **Deferred decision tasks (E-series).** Human-decision, doc-only; detail
  lives in `tasks/ENNN.md`. Status: `done` · `open` · `deferred` (a recorded decision to postpone, not pending).

| Task | Status | Blocks | Summary |
|---|---|---|---|
| E001 | done | — | Event ownership → A′ (ADR-0008) |
| E002 | done | — | Single `EntityId` in core (spec §2.4 fix) |
| E003 | done | — | Contracts are definitions only; `Transport` permanently net-local (D01, option B); spec reconciled 2026-09-30 by T032. E017 residuals stay open |
| E004 | done | — | One-task-one-crate → split (T008a sim / T008b testkit; later splits at Stage 2) |
| E005 | done | — | Testkit is a one-way integration consumer; lower-layer integration tests live there |
| E006 | done | — | `f64`-in-sim → banned (E006-A: `no-f64` lint for sim) |
| E007 | done | — | ADR policy: Decision never rewritten; supersede by new ADR or dated appended note; exact status vocabulary (T032, 2026-09-30; template `docs/adr/0000-template.md`). Architecture README pointer is a follow-up |
| E008 | done | — | Instruction (not wall-clock) event budget |
| E009 | done | — | ADR-0008 residue (sketch, diagrams, §24 text) |
| E010 | done | — | Budgets selected in D17 (1024 nodes, 100,000 units per slice, depth 32, 2 MiB Lua state, no wall clock); loader strip list corrected; spec reconciled by T033 (2026-09-30) |
| E011 | done | — | Determinism-scope ADR-0009 (replay, not lockstep) |
| E012 | done | — | Naming/placement: `crpg-server` host library + dedicated binary, client/editor Godot projects over `crpg-godot` (D02; spec reconciled by T032, 2026-09-30). E022 interfaces and capability-gated smoke obligations stay open |
| E013 | done | — | Spec (T032) and README (T033) diagrams match `ALLOWED`; "core" vs "simulation stack" defined (2026-09-30) |
| E014 | done | — | `World: Serialize` vs interned-handle caveat (skeleton-only serde) |
| E015 | done | — | Replica/prediction model + `Timeline` owner (buffer outside sim) |
| E016 | done | — | Entity/aggregate documents, lock authorities, package ids, tick waits |
| E017 | done | — | Direction decided in D01 and the T018/T021 as-built wire; spec §§5.2, 6.2, 7.2–7.6 reconciled by T033 (2026-09-30). The named evolution queue (T019–T029b) carries the residual work |
| E018 | done | — | Capability model selected in D03 (invitation credentials, pinned certificates, explicit grants, separate fail-closed control protocol, allowlisted client manifest); spec reconciled by T033 (2026-09-30). Implementation belongs to the owning tasks |
| E019 | done | — | D06: measure before enforcing ceilings; perf rows marked aspirations, 1000-entity/60 fps/no-LOD combination dropped; bench row added below (T033, 2026-09-30) |
| E020 | done | — | Gates 7–13 activate with capabilities; T009c supersedes its Linux-only T009a gate assignment |
| E021 | done | — | §15.1 example labelled illustrative-only; retrospective `tasks/T004.md` backfilled from git evidence (T032, 2026-09-30) |
| E022 | done | — | API-shape ledger added below (owner, prerequisite, phase per shape) with a spec §11.3 pointer (T033, 2026-09-30). The shapes themselves stay unspecified until their tasks |
| E023 | deferred | native extensions | Native loading/ABI and signing/packaging deferred by D07; portable data and sandboxed scripting are the extension path for current milestones |

## API-shape ledger (E022)

Owners, prerequisites and phases for the server/editor/bridge shapes named in
spec §§9–11. A row is a place in the queue, not a design; each shape is pinned
by its owning task's contract. Added 2026-09-30 by T033 from D01–D18.

| Shape | Owning crate(s) | Prerequisite | Phase |
|---|---|---|---|
| Host sessions, epochs, grants, capture/checkpoint | `crpg-server` | T022 (ADR-0022) | 4 |
| Network handshake: pinned certificate + invitation credential | `crpg-net-quic` (T023; ADR-0024), `crpg-server` (T023b) | T022, T023s, T030 evidence, D03 | 4 |
| Intent rate/queue policy and rejection codes | `crpg-net` (wire), `crpg-server` (policy) | T018 as built; T022 | 4 |
| Replica storage, delta application, engine-neutral read queries | `crpg-net` | T024, T027b | 5 |
| Engine-facing replica/presentation objects, `net_id`↔`EntityId` view | `crpg-godot` | replica queries above | 5 |
| Interpolation buffer and own-movement prediction | `crpg-godot` | T026p, movement spec (D24) | 5 |
| `EditCommand`, receipts, diagnostics, undo (headless first) | `crpg-edit` | data APIs (T010–T013, T029a) | 6 |
| Privileged control protocol (GM/admin), live-edit conflicts | `crpg-net` + `crpg-server` | T022, T023, D03 | 6 |
| Optional `crpgc apply` over the edit command API | `crpg-cli` | `EditCommand` above | 6 |
| FFI surface (§11.3) object types, errors, threading | `crpg-godot` | edit and replica APIs above | 5–6 |
| Client presentation-manifest export / import, strip leak test | `crpg-data` export, host distribution, client import | D03 | 7 |
| Native extension signing, trust root, loading | — | deferred (D07, E023) | later |

Measured performance work (E019/D06): subsystem-owned deterministic workload
APIs (minimal-d6 combat, srd-lite combat, eight-peer filtered replication,
snapshot assembly) first, then a thin `crpgc bench` wrapper in `crpg-cli`;
controlled-runner baselines per target/toolchain/profile/hardware before any
threshold becomes a gate.

## Not yet numbered

Small items from spec §19.1 that do not yet have a task file, roughly in the
order they become available: canonical JSON writer (§19.1 #11, part of T010),
`crpgc new` (#10, T013), `DiceExpr` (#15, T015), `OutcomeTable` (#16, T015),
`ResourcePool` (#17, T015), the Lua sandbox module (#18, `crpg-script`), codec
round-trip (#19, T018), simulated transport (#20, T018), Recast bake (#21),
Godot proxy spawning (#22), interpolation buffer (#23), editor tree (#24),
generated property form (#25), problems panel (#26).

Follow-up to review (user, 2026-10-05, T039 Q4): T039 loads a save only into the exact campaign version that produced it. Revisit whether saves should load into newer compatible campaign versions; that needs a save-migration rule, which is its own task.

Deferred input-hardening requirements (S001; land with the tasks that own
them, not here): T010 `crpg-data` loader caps — campaign JSON file size and
nesting-depth ceilings (`serde_json` recursion is a stack-overflow DoS); T018
`crpg-net` postcard decode caps — message size and `Vec`/`String` length
ceilings (a hostile length field is an OOM), complete for the implemented v1
vocabulary in PR #16 (`2fc54b7`: 4 KiB intent / 64 KiB delta frames, 256 delta
operations and 256-byte strings; string allocation is frame-bounded before its
field-length rejection; future vocabularies inherit separate
bound-before-allocation obligations); `SnapshotBackend`
(`crpg-persist`) decompression-bomb cap on `zstd` saves — input size *and*
decompressed-size ceiling.

Missing scaffolding from the workflow plan §15 checklist, none of it blocking:
`tools/preflight.ps1` (+ `.sh`) (on branch via T031), `docs/adr/0000-template.md`
(on branch via T032), per-crate
`AGENTS.md` beyond the five that exist (`crpg-core`, `crpg-sim`,
`crpg-testkit`, `crpg-cli`, `crpg-data`), and a self-hosted runner for the
slow CI layer.

`docs/architecture/` now exists, with its README and five docs
(`crpg-core.md`, `crpg-sim.md`, `crpg-testkit.md`, `crpg-cli.md`,
`crpg-data.md`). Spec
§15.6's rule — "if a crate has no architecture doc, it is not ready for agent
work" — is a readiness gate rather than a documentation quota, so the remaining
ten docs are **due with the first task that puts real code in each crate**,
not written up front against designs that have not been decided.
`docs/architecture/README.md` holds the index and the per-crate status.

**Every task that opens a crate carries this in its definition of done**,
alongside the crate's `AGENTS.md`:

```
- [ ] `docs/architecture/<crate>.md` written (or extended, if it exists),
      and its row in `docs/architecture/README.md` updated
- [ ] Agent log entry added (date · agent · 1–2 sentence why)
```

Spec §14 also lists `docs/contracts/` and `docs/guides/`, which still do not
exist. Unlike the architecture docs, no stated gate depends on them: contracts
are meaningful once `crpg-contracts` has traits in it, and authoring guides
once there is a campaign format to author against. They stay on the scaffolding
list above rather than being a rule the project is failing to keep.

## Integration gate activation

E020 rejects skipped green placeholders. The unavailable spec §15.4 gates
remain backlog obligations and become mandatory with these capabilities:

| Step | Activates with |
|---|---|
| 7 schema drift | T010 schema generation |
| 8 fixture/ruleset validation | T011's per-crate data + CLI split |
| 9 golden replay | T009a implementation corrected by T009c: independent Windows/MSVC and Linux/GNU comparisons in the existing workspace-test matrix |
| 10 save/load equivalence | T039 (on branch): `crates/crpg-server/tests/host_save.rs` file-backed host save/load continuation cases (`file_save_load_equivalence`, `repeated_restarts_match_uninterrupted_run`, `older_save_resumes_from_its_own_point`, each V1 and V2) in the existing `cargo test --workspace --locked` job on Windows/MSVC and Linux/GNU; per-target, in-process equivalence on the T022 fixture authority, no golden. Campaign-wide breadth (spec §8 "every fixture campaign") comes with T040/T041, the first tasks that build a host from campaign data |
| 11 performance | E019 benchmark task |
| 12 product builds | Each real binary task: Windows client/editor/server and Linux headless server artifacts; no placeholder jobs |
| 13 integrated smoke | Post-T018 real capabilities: Windows embedded-server and dedicated-server smoke tests plus Linux dedicated-server smoke test; no placeholder jobs |

---

## Throughput

Workflow plan §13 asks for tasks merged per week and cost per merged task.
Record it here, one line per week.

| Week ending | Merged | Notes |
|---|---|---|
| 2026-09-06 | 16 | T001–T005c plus T006a–e, T007, T008a and T008b are merged. Two review follow-ups merged alongside T006a and are not counted, being fixes rather than numbered tasks. Cost per merged task not tracked yet. |
| 2026-09-07 | 19 | T009a (typed replay), T009c (Windows-primary/Linux-supported native goldens, ADR-0012) and T009b (thin `crpgc replay` wrapper) merged. Three review follow-ups merged alongside T006a and are not counted, being fixes rather than numbered tasks. Cost per merged task not tracked yet. T010 campaign data is next. |
| 2026-09-17 | 22 | T010 (campaign data format and schema gate), T011a (positioned validation in `crpg-data`) and T011b (thin `crpgc validate` wrapper) merged and pushed to `origin`. The T011a slug/graph coverage follow-up merged alongside T011a and is not counted, being a fix rather than a numbered task. Cost per merged task not tracked yet. T012 migration framework is next. |
| 2026-09-18 | 25 | T012a (data-half migration framework, with the T012/T012b Stage-2 specs), T012b (thin `crpgc migrate` wrapper, PR #5), and T013a (data-owned introspection prerequisite, PR #7) merged. The T012a review-fix follow-up merged alongside T012a and the T013a review fix alongside T013a, neither counted, being fixes rather than numbered tasks. Cost per merged task not tracked yet. T013 is next. |
| 2026-09-26 | 26 | T013 (scaffolding/introspection CLI: six thin wrappers, five binary suites, literal LLM acceptance, PR #9) merged. The trial-input line-ending follow-up merged alongside T013 and is not counted, being a fix rather than a numbered task. Cost per merged task not tracked yet. T014 rules kernel is next. |
| 2026-09-26 | 27 | T014 (stat/modifier kernel in `crpg-rules`: 122-row table, 1024-case properties, boundary coverage, PR #11) merged. The review follow-up closing the enum/tag-literal/capacity boundary and reversal-occurrence gaps merged alongside T014 and is not counted, being a fix rather than a numbered task. Cost per merged task not tracked yet. T015 dice/resolution is next. |
| 2026-09-26 | 28 | T015 (dice, outcome tables, and resolution in `crpg-rules`, PR #13, `76a48da`) merged. Cost per merged task not tracked yet. T016 headless combat is next; its T016a–d children plus the T016e invariant repair are implemented and verified in the working tree, unmerged, so none is counted here. |
| 2026-09-27 | 30 | T016 (headless combat with `rulesets/minimal-d6`: content, sim adapter/controller, lifecycle/release, replay goldens, CLI proof) and T017 (second-ruleset abstraction proof with `rulesets/srd-lite`: multi-pool/effect/defense/turn generalization, replay goldens, CLI proof) merged (PR #14, `4686a73`). Cost per merged task not tracked yet. T018 `crpg-net` protocol is next. |
| 2026-09-28 | 31 | T018 (`crpg-net` lane-0 combat protocol, bounded postcard codec, simulated transport and conformance: T018a/b/c, PR #16, `bc54896`) merged. Cost per merged task not tracked yet. T019 `crpg-sim` same-pool repair is implemented and dual-native verified in the working tree, unmerged, so it is not counted here. |
| 2026-09-29 | 32 | T019 (`crpg-sim` same-pool affordability repair, PR #17, `f3d1560`) merged. Cost per merged task not tracked yet. T020 history (specification review) is next. |
| 2026-09-30 | 34 | T020 (`crpg-sim` opt-in history) and T021 (`crpg-net` v2 event protocol) merged in PR #18 (`a991fe9`) together with the post-T018 specification frontier (ADRs 0018–0022, T022–T032 contracts). Cost per merged task not tracked yet. T022 host needs specification inputs; T027a, T028, T029a and T030–T032 are specified and implementable. |
| 2026-09-30 | 40 | T027a, T028, T029a (sim/data), T030 (quinn dossier), T031 (preflight runners) and T032 (documentation reconciliation) merged in PR #20 (`c1bc6ab`), CI green on Windows/MSVC and Linux/GNU. Cost per merged task not tracked yet. Next: T022 host and T029b bindings, both waiting on dependency approval. |
| 2026-10-01 | 41 | T033 (documentation: E-task wording reconciled) merged in PR #21 (`ac137ac`) with decisions D19–D26, ADR-0023 and the D26 preflight CI step; all nine checks green including the first native Windows preflight-test run. Cost per merged task not tracked yet. Next: T022 host and T029b bindings, both ready; plan of record in `docs/IMPLEMENTATION_PLAN.md`. |
| 2026-10-04 | 44 | T022 (`crpg-server` host), T029b (`crpg-script` bindings) and T038 (`crpg-persist` save envelope) merged in PR #23 (`9157250`), CI green on both OSes; D27 recorded; T058 contract approved, blocked on T058a. Cost per merged task not tracked yet. Next: T058a and T023 contracts (drafting). |
| 2026-10-04 | 48 | T058a (PR #24), T058 and T023s step 1 (PR #25), T023 and T023s step 2 (PR #26) merged, CI green on both OSes. Cost per merged task not tracked yet. Next: T023b host QUIC adapter and T039 checkpoint adapter contracts. |
| 2026-10-05 | 51 | T039 (gate 10), T023c (window limits) and T023v (vendored quinn-proto close fix) merged in PR #27 (`efbfdd8`); `master` Windows CI red since #26 is resolved by T023v. Cost per merged task not tracked yet. Next: amend and implement T023b. |
| 2026-10-06 | 53 | T023d (`crpg-net-quic` close delivery on an immediate `Closed` poll) and T023b (`crpg-server` QUIC host adapter, ADR-0026) merged in PR #28 (`16f58c8`), CI green on both OSes; T023v's four Windows runs complete. Cost per merged task not tracked yet. Next: T024 snapshot-transfer contract (drafted, awaiting approval). |

---

## Agent log

- 2026-09-05 (UTC) · opencode/muse-spark + agent-attribution rule · Added the agent-log checkbox to the crate-opening Definition of Done so future crate tasks sign their own doc edits; no change to the readiness gate.
- 2026-09-05 (UTC) · opencode/muse-spark + E002/E008/hygiene batch · Marked not-yet-written task rows as intentional (Stage 2) so absence reads as design, not omission.
- 2026-09-05 (UTC) · opencode/muse-spark + E001/ADR-0008 · Assigned event pieces: substrate + SimEvent → T007 (scoped core exception, see ADR), IR types → T010, hooks → T014.
- 2026-09-06 (UTC) · opencode/muse-spark + spec-gap triage · Filed E009–E022 from the full-spec gap review and indexed all E-tasks in the blockers table; no spec, README, or source changes per file-only scope.
- 2026-09-06 (UTC) · opencode/muse-spark + E006-A/E009/E014/E015 + hygiene · Marked T006e merged (it landed in 8f2e38b; the "on branch" row was stale), corrected the throughput count to 13, locked the T007/T008 scope split (Timeline container vs advance rules), and recorded the four T007-unblocking decisions as done.
- 2026-09-06 (UTC) · opencode/muse-spark + T007 merged · Marked T007 done, T008 next, throughput at 14.
- 2026-09-06 (UTC) · opencode/muse-spark + E004 decided (Option A) · Split T008 into T008a (sim, next) and T008b (testkit); E011 now blocks T008a, E005 retargeted to T009, `crpgc run` wrapper transferred to T013. Later multi-crate tasks split at Stage 2.
- 2026-09-06 (UTC) · opencode/muse-spark + E011 filed · ADR-0009 accepted (replay-not-lockstep scope); T008a/T008b/T009 rows cite it, E011 done.
- 2026-09-06 (UTC) · opencode/muse-spark + T008a merged · Marked T008a done, T008b next, throughput at 15.
- 2026-09-06 (UTC) · opencode/muse-spark + T008b merged · Marked T008b done, T009 next (blocked: E005 + E020), throughput at 16.
- 2026-09-06 (UTC) · opencode/muse-spark + review 3 follow-up · Recorded the sim/core/testkit hardening and ADR-0010/0011 as done (fixes, not counted in throughput per the review 1/2 precedent); T009 still next, still blocked on E005 + E020.
- 2026-09-06 (UTC) · opencode/gpt-5.6-sol + E005/E016/E020 decisions · Marked all three decisions done, split T009 into testkit and CLI tasks, expanded T010/T013 ownership, and indexed capability-based activation for integration gates 7–13.
- 2026-09-06 (UTC) · opencode/muse-spark + T009a merged · Marked T009a done (replay format/playback, canonical-Linux golden generated and green on genuine Rust 1.98.0 Linux, CI step 9 live), T009b next, throughput at 17.
- 2026-09-06 (UTC) · opencode/gpt-5.6-sol + T009c platform correction plan · Corrected the prior unmerged-as-done record and made T009c the next priority: Windows/MSVC becomes primary while Linux/GNU retains an independently enforced server replay baseline; T009b waits for that policy correction.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c · Kept T009a unmerged, T009c next/in progress pending native verification, T009b blocked, and merged throughput unchanged. Indexed shared-host ownership and open E023 native-extension governance with capability-gated product checks.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c verification alignment · Recorded the reported native Windows and WSL Ubuntu gate passes and per-target counts, linking the completion record. T009a/T009c remain uncommitted and unmerged, T009c stays next for review/merge, T009b waits for landing, and throughput remains 16.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c count correction · Corrected the active workspace total to 135 (134 unit/integration + 1 doctest), matching the reported breakdown of core 91, sim 24, testkit 19, and core doctest 1. Verification status and merged throughput are unchanged.
- 2026-09-07 (UTC) · opencode/big-pickle + T009a/T009c merged · Marked T009a and T009c done (merged to `master` in `bb9a702`), T009b next, throughput at 18.
- 2026-09-07 (UTC) · opencode/big-pickle + T009b merged · Marked T009b done (merged to `master`), T010 next, throughput at 19.
- 2026-09-17 (UTC) · opencode/muse-spark + T010/T011a/T011b merged · Marked T010, T011a and T011b done (pushed to `origin` in `15ac9fb`, `2f801b5`/`ab7d208` and `d45f1b1`), T012 next, throughput at 22; the slug/graph coverage follow-up is a fix, not a counted task.
- 2026-09-18 (UTC) · opencode/muse-spark + T012a merged/doc-status catch-up · Marked T012a done (E004 data half; T012b next), split the single T012 row into the T012a/T012b pair, refreshed the scaffolding counts to the five live crate docs, and moved throughput to 23; the review-fix follow-up is a fix, not a counted task.
- 2026-09-17 (UTC) · opencode/muse-spark + S001 deferred input caps · Recorded the three deferred input-cap requirements with their owning tasks, so loader/postcard/snapshot hardening lands where the code lives.
- 2026-09-18 (UTC) · opencode/muse-spark + S001 merged · Marked S001 done (PR #2 merged 2026-09-17, all 9 checks green, branch protection human-done, revert PR #3 closed unmerged); S-series, so throughput unchanged per the fix-not-counted precedent.
- 2026-09-18 (UTC) · opencode/muse-spark + T012b merged/T013a on-branch catch-up · Marked T012b done (merged PR #5), added the T013a on-branch row for the implemented and dual-native verified data prerequisite (`8d2ea32`), clarified T013 consumes the landed API, and moved throughput to 24 with T013a landing next.
- 2026-09-18 (UTC) · opencode/muse-spark + T013a landed · Marked T013a done (merged PR #7, `e84b13e`), moved throughput to 25 with T013 next; the review-added stale-index test is a fix, not a counted task.
- 2026-09-26 (UTC) · opencode/muse-spark + T013 landed · Marked T013 done (merged PR #9, `9a6304f`), moved throughput to 26 with T014 next; the trial-input line-ending fix is a fix, not a counted task.
- 2026-09-26 (UTC) · opencode/muse-spark + T014 landed · Marked T014 done (merged PR #11, `18145d4`), moved throughput to 27 with T015 next; the review-added boundary/reversal tests are a fix, not a counted task.
- 2026-09-27 (UTC) · opencode/muse-spark + A0 status reconciliation · Marked T015 done (merged PR #13, `76a48da`, 2026-09-26), moved T016 to in progress with its T016a–e working-tree states and A2/merge outstanding, and moved throughput to 28; unmerged work is never counted.
- 2026-09-27 (UTC) · opencode/muse-spark + T017/B1 specification · Recorded the approved T017 milestone contract and the specified B1 data contract on the open T017 row; neither implementation nor merge is claimed.
- 2026-09-27 (UTC) · opencode/muse-spark + T016/T017 landed · Marked T016 and T017 done (merged PR #14, `4686a73`), moved throughput to 30 with T018 `crpg-net` protocol next.
- 2026-09-28 (UTC) · opencode/gpt-6-astra + T018 parent and child contracts · Split open T018 into lane-0 protocol/codec (T018a), simulated transport (T018b), and conformance (T018c), all single-crate `crpg-net` per E004, on approved E017 Appendix B with recommended defaults; no implementation, merge, or throughput change.
- 2026-09-28 (UTC) · opencode/muse-spark + T018 landed/post-T018 queue · Marked T018 done (merged PR #16, `bc54896`, 2026-09-28), added the T019–T029b follow-on rows from the POST-T018 maintainer drafts (T019 as `on branch`: implemented and dual-native verified in the working tree per its completion record, unmerged — a truthful deviation from the draft's `next`), retargeted E003/E017 blocker cells to post-T018 reconciliation, annotated the S001 postcard closure for the v1 vocabulary, and moved throughput to 31; review/merge of T019 and T020 specification remain outstanding.
- 2026-09-30 (UTC) · opencode/muse-spark + T019 landed · Marked T019 done (merged PR #17, `f3d1560`, 2026-09-29), noted the T019 dependency met on the T020 row, and moved throughput to 32 with T020 specification review next.
- 2026-09-29 (UTC) · opencode/gpt-6-astra + specification frontier · Indexed six exact implementation contracts and three independent preparation/tooling contracts, with four new decision records. Kept specification readiness distinct from unmet implementation/dependency gates and left merged status/throughput unchanged.
- 2026-09-30 (UTC) · opencode/muse-spark + review-fix pass · Corrected the T019 row's stale branch pointer (`task/T018-net-protocol` → `task/T019-same-pool`); implementation/merge status and throughput unchanged.
- 2026-09-30 (UTC) · claude-code + T020/T021 landed bookkeeping · Marked T020 and T021 done (merged in PR #18, `a991fe9`, after the T019 bookkeeping in PR #19 had already recorded them as blocked) and moved throughput to 34; kept the T020 golden-artifact review, the missing T021 completion record, and the still-"Selected" ADR-0017/0019 statuses visible as open items rather than implying approval.
- 2026-09-30 (UTC) · claude-code + T028 implementation · Moved T028 to `on branch` with Linux-only verification, so the unrun Windows/MSVC gate and review/merge stay explicit instead of reading as done.
- 2026-09-30 (UTC) · claude-code + T027a implementation · Moved T027a to `on branch` with Linux-only verification, keeping the Windows/MSVC gate and review/merge explicit.
- 2026-09-30 (UTC) · claude-code + T029a implementation · Moved T029a to `on branch` with Linux-only verification, keeping the Windows/MSVC gate and review/merge explicit.
- 2026-09-30 (UTC) · claude-code + T032 documentation reconciliation · Closed E003/E007/E012/E021 with scope notes and kept E013 open for its README diagram, so the backlog does not claim debt outside T032's allowed files is resolved.
- 2026-09-30 (UTC) · claude-code + T031 implementation · Moved T031 to `on branch` and annotated the scaffolding checklist for the preflight runners and ADR template, keeping the native Windows run open.
- 2026-09-30 (UTC) · claude-code + T030 dossier · Moved T030 to `on branch` with its evidence location and the three blockers that keep T023 blocked.
- 2026-09-30 (UTC) · claude-code + PR #20 landed · Marked T027a/T028/T029a/T030/T031/T032 done (merged `c1bc6ab`, dual-OS CI green) and moved throughput to 40, keeping T031's missing native Windows test run and T023's open decisions explicit.
- 2026-09-30 (UTC) · claude-code + D19–D24 · Opened T022 and T029b, narrowed T023 to its own contract, noted D24/D23 on T026p, and closed the T020/T021 review items per the user-approved decisions.
- 2026-09-30 (UTC) · claude-code + T033 E-task reconciliation · Closed E010/E013/E017/E018/E019/E022 against D01–D18 and the as-built code, marked E023 deferred (with a new `deferred` status meaning), added the E022 API-shape ledger and the D06 bench row, and added the T033 row.
- 2026-10-01 (UTC) · claude-code + PR #21 landed / implementation plan · Marked T033 done and T031's Windows test evidence supplied (PR #21, `ac137ac`), moved throughput to 41, and linked the new delegation plan without filing its reserved T034+ ids as tasks.
- 2026-10-04 (UTC) · claude-code + T038 contract · Added the T038 row as `open` with its contract draft awaiting approval, so the first Wave 1 spec output is indexed without implying readiness or approval.
- 2026-10-04 (UTC) · claude-code + T038 approval · Marked T038 ready after the user approved its contract and dependency request.
- 2026-10-04 (UTC) · claude-code + T058 contract · Added a Phase 6 — Editor table with the T058 row as `open`, its contract draft awaiting approval and blocked on the `crpg-data` prerequisite T058a, so the second Wave 1 spec output is indexed without implying readiness or approval.
- 2026-10-04 (UTC) · claude-code + T038 implementation · Moved T038 to `on branch` with Linux-only verification, keeping the Windows/MSVC gate and review/merge explicit.
- 2026-10-04 (UTC) · claude-code + T022/T029b implementation · Moved T022 and T029b to `on branch` with Linux-only verification, keeping the Windows/MSVC gate, T022's flagged contract readings and review/merge explicit.
- 2026-10-04 (UTC) · claude-code + T058 decisions · Recorded T058's contract approval while keeping it blocked on T058a.
- 2026-10-04 (UTC) · claude-code + PR #23 landed · Marked T022, T029b and T038 done (merge `9157250`, dual-OS CI green) and moved throughput to 44.
- 2026-10-04 (UTC) · claude-code + T058a contract · Added the T058a row above T058 in the Phase 6 — Editor table as `open`, its contract draft awaiting approval and blocking T058, so the prerequisite is indexed without implying approval.
- 2026-10-04 (UTC) · claude-code + T058a link · Pointed the T058 row's prerequisite note at the new T058a contract instead of the stale "no task file yet".
- 2026-10-04 (UTC) · claude-code + T058a approval · Marked T058a ready after the user approved its contract.
- 2026-10-04 (UTC) · claude-code + T023 contract · Noted on the T023 row that its exact contract is drafted and awaiting approval, and that its measured audit adds a `cargo deny` blocker, so the row no longer reads as waiting only on contract writing.
- 2026-10-04 (UTC) · claude-code + T058a implementation · Moved T058a to `on branch` with Linux-only verification, keeping the Windows/MSVC gate, the open §6 duplicate-id decision and review/merge explicit.
- 2026-10-04 (UTC) · claude-code + T058a erratum · Replaced the row's open-contradiction note with the user-approved erratum.
- 2026-10-04 (UTC) · claude-code + PR #24 landed · Marked T058a done (dual-OS CI green) and T058 ready now that its prerequisite is merged.
- 2026-10-04 (UTC) · claude-code + T023 re-contract · Added the T023s setup row and moved the T023 row to the new `crpg-net-quic` crate with its approved decisions and re-contract status. Also pointed the T023b row and the E022 handshake ledger row at that crate, so the index matches the Q12 split.
- 2026-10-04 (UTC) · claude-code + backlog tidy · Removed the stale `on branch` T022 row that duplicated its `done` row after the PR #23 bookkeeping.
- 2026-10-04 (UTC) · claude-code + T023/T023s approval · Marked T023s ready and recorded T023's approved re-contract, gated on T023s.
- 2026-10-04 (UTC) · claude-code + T058 implementation · Moved T058 to `on branch` with Linux-only verification, keeping the Windows/MSVC gate, the open rename-onto-lock decision and review/merge explicit.
- 2026-10-04 (UTC) · claude-code + T023s implementation · Moved T023s to `on branch` with Linux-only verification, keeping the Windows/MSVC gate, the H-review of the lint row and review/merge explicit.
- 2026-10-04 (UTC) · claude-code + T058 erratum · Noted the user-approved rename-onto-lock erratum; status unchanged.
- 2026-10-04 (UTC) · claude-code + T023s split · T023s lands in two PRs (lint/docs, then the stub crate) because the trusted-lint guard judges with the merge-base lint.
- 2026-10-04 (UTC) · claude-code + PR #25 landed · Marked T058 done and T023s step 1 merged (PR #25, dual-OS CI green), with the stub crate re-added on branch as step 2.
- 2026-10-04 (UTC) · claude-code + T023 implementation · Moved T023 to `on branch` with Linux-only verification, keeping the Windows/MSVC gate and review/merge explicit.
- 2026-10-04 (UTC) · claude-code + T023 integration · Restored the two-step T023s row that the T023 cherry-pick's older copy had overwritten.
- 2026-10-04 (UTC) · claude-code + PR #26 landed · Marked T023 and T023s done (PRs #25/#26, dual-OS CI green) and moved throughput to 48.
- 2026-10-05 (UTC) · claude-code + T039 contract · Added the T039 row as `open` with its contract draft awaiting approval, so the Wave 3 persistence adapter is indexed without implying readiness, approval or gate-10 activation.
- 2026-10-05 (UTC) · claude-code + T023b contract · Moved the T023b row to `open` with its exact contract drafted and awaiting approval, since T022 and T023 are done, so the index shows the remaining gate is contract approval, not a prerequisite.
- 2026-10-05 (UTC) · claude-code + T039 decisions · Marked T039 ready and recorded the user's review-later follow-up on cross-version save loading.
- 2026-10-05 (UTC) · claude-code + T039 implementation · Moved T039 to `on branch` with Linux-only verification and activated the gate-10 row on the T022 fixture authority, carrying campaign-wide breadth to T040/T041 as the user decided.
- 2026-10-05 (UTC) · claude-code + T023b decisions · Marked T023b ready after the user approved its contract.
- 2026-10-05 (UTC) · claude-code + T023c filed · Added T023c ahead of T023b per the user's choice and marked T023b blocked on it.
- 2026-10-05 (UTC) · claude-code + T023c contract · Moved the T023c row to contract draft awaiting approval, with its scope and question count, so the index shows the remaining gate before T023b can resume.
- 2026-10-05 (UTC) · claude-code + T023c decisions · Marked T023c ready after the user approved its contract.
- 2026-10-05 (UTC) · claude-code + T023c implementation · Moved T023c to `on branch` with Linux-only verification and noted on T023b which cases to amend once T023c lands, so the index shows the Windows gate and merge as the remaining steps before T023b resumes.
- 2026-10-05 (UTC) · claude-code + T023v filed · Filed T023v after T023c's case-17 mitigation failed on Windows, per the user's choice to patch quinn-proto.
- 2026-10-05 (UTC) · claude-code + T023v contract · Moved the T023v row to contract draft awaiting approval, with its two-part split, the upstream fix it recommends and its question count, so the index shows what must be approved before PR #27 can go green on Windows.
- 2026-10-05 (UTC) · claude-code + T023v decisions · Marked T023v ready after the user approved its contract and dependency change.
- 2026-10-05 (UTC) · claude-code + T023v-a · Moved T023v to "T023v-a on branch" with its Linux/GNU results and the open upstream provenance check, so the index shows H-review and T023v-b as the next steps.
- 2026-10-05 (UTC) · claude-code + T023v-b · Recorded T023v-b on branch with its Linux results and noted on T023c that C§8.5 is withdrawn, so the index shows the four Windows runs at the PR tip as the remaining gate for both.
- 2026-10-05 (UTC) · claude-code + PR #27 landed · Marked T039, T023c and T023v done (merge `efbfdd8`), kept T023v's upstream-provenance check open, and moved throughput to 51.
- 2026-10-06 (UTC) · claude-code + T023b amendment · Marked T023b "blocked → amendment drafted, awaiting approval", because its stall-case amendment on T023c windows now waits only for the user's approval.
- 2026-10-06 (UTC) · claude-code + T023b amendment decisions · Marked T023b ready after the user approved the stall-case amendment.
- 2026-10-06 (UTC) · claude-code + T023v provenance check · Closed T023v's open upstream-provenance item on its row after the byte-for-byte check passed.
- 2026-10-06 (UTC) · claude-code + T023d filed · Added T023d as a contract draft and marked T023b blocked on it, after T023b's implementer found the close lost on an immediate `Closed` poll and the user chose a separate `crpg-net-quic` task.
- 2026-10-06 (UTC) · claude-code + T023d decisions · Marked T023d ready after the user approved its contract.
- 2026-10-06 (UTC) · claude-code + T023d implementation · Moved T023d to on branch with its Linux/GNU results and negative control, so the index shows the Windows check at the PR tip and the merge with T023b as the remaining steps.
- 2026-10-06 (UTC) · claude-code + T023b · Moved T023b to on branch (done, unmerged) with its Linux/GNU results, ADR-0026 and T023d as the case-9 fix, so the index shows the Windows check at the PR tip and the merge with T023d as the remaining steps.
- 2026-10-06 (UTC) · claude-code + PR #28 Windows · PR #28 (T023d + T023b) is green on both OSes; Windows ran `quic` 30/30 and `host_quic` 25/25, the fourth Windows run for T023v.
- 2026-10-06 (UTC) · claude-code + T024 contract · Moved T024 from blocked to contract draft awaiting approval, with its design in one line and five open questions, since T021 and T023b are finished and the remaining gate is the user's approval.
- 2026-10-06 (UTC) · claude-code + PR #28 landed · Marked T023d and T023b done (merge `16f58c8`) and moved throughput to 53, so the index shows T024's contract as the next gate.
