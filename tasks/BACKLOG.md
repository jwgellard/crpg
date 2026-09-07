# Task backlog

The index of every numbered task. Derived from `docs/CRPG_ENGINE_SPEC.md` §24
(the first eighteen tasks) and §19.1 (the small backlog). One line per task;
detail lives in `tasks/TNNN.md`. Rows with no file yet (T007 and later) are
intentional — task files are written when the task is specified (Stage 2).

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
| T004 | done | 2026-09-03 | Workspace, 15 stub crates, CI on Linux + Windows |
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
| T009a | in progress | uncommitted/unmerged local `master` working tree | Replay format/playback + genuine original Linux verification preserved; T009c correction verified, awaiting review/merge |
| **T009c** | **next** | uncommitted/unmerged working tree | Windows-primary/Linux-supported policy + independent native replay goldens (`crpg-testkit`; ADR-0012 supersedes only ADR-0009 Decision 3's Linux-only selection); required native Windows/MSVC and genuine WSL Ubuntu Linux/GNU gates passed, awaiting review/merge; see [completion record](T009c.md) |
| T009b | blocked | T009c landing | Thin `crpgc replay` wrapper (`crpg-cli`; blocked pending T009c review/merge) |

Reported T009c results on each native target: 19 testkit tests (6 harness +
12 portable replay + 1 golden), 135 workspace tests (134 unit/integration + 1 doctest),
and 65 lint self-tests. Full required-gate results and provenance belong to
the [T009c completion record](T009c.md); passing gates does not imply landing.

T006a–e are spec §24's single T6, split per ADR-0006. T006a established
`Cargo.toml`, the module layout and `crpg-core/AGENTS.md`; T006b-T006e are
merged. T006e finishes the planned core primitives.

## Security hardening

From the security review, not spec §24. Detail lives in `tasks/S001.md`.

| Task | Status | Merged | Summary |
|---|---|---|---|
| S001 | open | — | Fork-PR guard over `tools/lint/` and workflows, `build.rs` ban, secret scan, self-hosted-runner rule, `yanked = "deny"` |

## Phase 2 — Campaign data format

| Task | Status | Merged | Summary |
|---|---|---|---|
| T010 | open | — | `crpg-data`: entity/aggregate schemas, package ids, canonical writer, resolver/lock APIs, loader/index, tick-wait event IR |
| T011 | open | — | Validation and positioned diagnostics, `crpgc validate --json` |
| T012 | open | — | Migration framework |
| T013 | open | — | Scaffolding/introspection CLI, including thin `crpgc lock` and run wrappers |

## Phase 3 — Rules kernel

| Task | Status | Merged | Summary |
|---|---|---|---|
| T014 | open | — | `crpg-rules`: stats and the modifier pipeline; kernel hook event types per ADR-0008 |
| T015 | open | — | Dice, outcome tables, resolution |
| T016 | open | — | `rulesets/minimal-d6` + headless combat |
| T017 | open | — | `rulesets/srd-lite` — the abstraction gate |

## Phase 4 — Server and networking

| Task | Status | Merged | Summary |
|---|---|---|---|
| T018 | open | — | `crpg-net` protocol v1 + simulated-network transport |

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
  lives in `tasks/ENNN.md`. Status: `done` · `open`.

| Task | Status | Blocks | Summary |
|---|---|---|---|
| E001 | done | — | Event ownership → A′ (ADR-0008) |
| E002 | done | — | Single `EntityId` in core (spec §2.4 fix) |
| E003 | open | T018 | Contracts placement (Transport trait home) |
| E004 | done | — | One-task-one-crate → split (T008a sim / T008b testkit; later splits at Stage 2) |
| E005 | done | — | Testkit is a one-way integration consumer; lower-layer integration tests live there |
| E006 | done | — | `f64`-in-sim → banned (E006-A: `no-f64` lint for sim) |
| E007 | open | — | ADR immutability wording |
| E008 | done | — | Instruction (not wall-clock) event budget |
| E009 | done | — | ADR-0008 residue (sketch, diagrams, §24 text) |
| E010 | open | script | Script budgets + sandbox-strip alignment |
| E011 | done | — | Determinism-scope ADR-0009 (replay, not lockstep) |
| E012 | open | bridge/server | Binary/crate naming and shared authoritative host package placement with E022 |
| E013 | open | — | Diagram direction + "core" meaning |
| E014 | done | — | `World: Serialize` vs interned-handle caveat (skeleton-only serde) |
| E015 | done | — | Replica/prediction model + `Timeline` owner (buffer outside sim) |
| E016 | done | — | Entity/aggregate documents, lock authorities, package ids, tick waits |
| E017 | open | T018 | T018 interface debt (intents, registry, caps) |
| E018 | open | server | Privileged-channel capability model |
| E019 | open | CI | Perf measurability + `crpgc bench` task |
| E020 | done | — | Gates 7–13 activate with capabilities; T009c supersedes its Linux-only T009a gate assignment |
| E021 | open | — | Embedded-contract hygiene + T004 file |
| E022 | open | post-T018 | Server/editor/bridge API-shape ledger; one authoritative host for Windows embedded/dedicated and Linux dedicated adapters with E012 |
| E023 | open | native extensions | T0 target-specific artifacts, ABI/loading and packaging decisions for Windows/MSVC and Linux/GNU; reconcile unsafe governance before implementation |

## Not yet numbered

Small items from spec §19.1 that do not yet have a task file, roughly in the
order they become available: canonical JSON writer (§19.1 #11, part of T010),
`crpgc new` (#10, T013), `DiceExpr` (#15, T015), `OutcomeTable` (#16, T015),
`ResourcePool` (#17, T015), the Lua sandbox module (#18, `crpg-script`), codec
round-trip (#19, T018), simulated transport (#20, T018), Recast bake (#21),
Godot proxy spawning (#22), interpolation buffer (#23), editor tree (#24),
generated property form (#25), problems panel (#26).

Missing scaffolding from the workflow plan §15 checklist, none of it blocking:
`tools/preflight.ps1` (+ `.sh`), `docs/adr/0000-template.md`, per-crate
`AGENTS.md` beyond `crpg-core`'s, and a self-hosted runner for the slow CI
layer.

`docs/architecture/` now exists, with its README and `crpg-core.md`. Spec
§15.6's rule — "if a crate has no architecture doc, it is not ready for agent
work" — is a readiness gate rather than a documentation quota, so the remaining
fourteen docs are **due with the first task that puts real code in each crate**,
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
| 10 save/load equivalence | Persistence implementation task |
| 11 performance | E019 benchmark task |
| 12 product builds | Each real binary task: Windows client/editor/server and Linux headless server artifacts; no placeholder jobs |
| 13 integrated smoke | Post-T018 real capabilities: Windows embedded-server and dedicated-server smoke tests plus Linux dedicated-server smoke test; no placeholder jobs |

---

## Throughput

Workflow plan §13 asks for tasks merged per week and cost per merged task.
Record it here, one line per week.

| Week ending | Merged | Notes |
|---|---|---|
| 2026-09-06 | 16 | T001–T005c plus T006a–e, T007, T008a and T008b are merged. T009a and T009c remain uncommitted/unmerged in the local working tree awaiting review/merge after native verification passed. Two review follow-ups merged alongside T006a and are not counted, being fixes rather than numbered tasks. Cost per merged task not tracked yet. |

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
