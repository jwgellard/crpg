# Task backlog

The index of every numbered task. Derived from `docs/CRPG_ENGINE_SPEC.md` §24
(the first eighteen tasks) and §19.1 (the small backlog). One line per task;
detail lives in `tasks/TNNN.md`. Rows with no file yet (T007 and later) are
intentional — task files are written when the task is specified (Stage 2).

Status: `done` · `on branch` · `next` · `open` · `blocked` · `human` (needs a
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
| **T008** | **next** | — | `state_hash` + the fixed-step tick loop + `Timeline` advance rules |
| T009 | open | — | Replay record/playback harness |

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
| T010 | open | — | `crpg-data`: schema types, canonical writer, loader; event-IR graph types per ADR-0008 |
| T011 | open | — | Validation and positioned diagnostics, `crpgc validate --json` |
| T012 | open | — | Migration framework |
| T013 | open | — | Scaffolding and introspection CLI |

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
  `BTreeMap`, interned ids runtime-only) govern T006a–e, T007, T008 and T014.
  No longer a blocker.
- **Deferred decision tasks (E-series).** Human-decision, doc-only; detail
  lives in `tasks/ENNN.md`. Status: `done` · `open`.

| Task | Status | Blocks | Summary |
|---|---|---|---|
| E001 | done | — | Event ownership → A′ (ADR-0008) |
| E002 | done | — | Single `EntityId` in core (spec §2.4 fix) |
| E003 | open | T018 | Contracts placement (Transport trait home) |
| E004 | open | T008+ | One-task-one-crate splits for multi-crate tasks |
| E005 | open | T008 | Testkit dev-cycle ownership |
| E006 | done | — | `f64`-in-sim → banned (E006-A: `no-f64` lint for sim) |
| E007 | open | — | ADR immutability wording |
| E008 | done | — | Instruction (not wall-clock) event budget |
| E009 | done | — | ADR-0008 residue (sketch, diagrams, §24 text) |
| E010 | open | script | Script budgets + sandbox-strip alignment |
| E011 | open | T008 | Determinism-scope ADR (spec orders it) |
| E012 | open | bridge | Binary/crate naming (`crpg-client`, bridge) |
| E013 | open | — | Diagram direction + "core" meaning |
| E014 | done | — | `World: Serialize` vs interned-handle caveat (skeleton-only serde) |
| E015 | done | — | Replica/prediction model + `Timeline` owner (buffer outside sim) |
| E016 | open | T010 | Campaign envelope contradictions |
| E017 | open | T018 | T018 interface debt (intents, registry, caps) |
| E018 | open | server | Privileged-channel capability model |
| E019 | open | CI | Perf measurability + `crpgc bench` task |
| E020 | open | T009 | Gate steps 7–13 + testkit ownership |
| E021 | open | — | Embedded-contract hygiene + T004 file |
| E022 | open | post-T018 | Server/editor/bridge API-shape ledger |

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

---

## Throughput

Workflow plan §13 asks for tasks merged per week and cost per merged task.
Record it here, one line per week.

| Week ending | Merged | Notes |
|---|---|---|
| 2026-09-06 | 14 | T001–T005c plus T006a–e and T007, all on `master`. Two review follow-ups merged alongside T006a and are not counted, being fixes rather than numbered tasks. Cost per merged task not tracked yet. |

---

## Agent log

- 2026-09-05 (UTC) · opencode/muse-spark + agent-attribution rule · Added the agent-log checkbox to the crate-opening Definition of Done so future crate tasks sign their own doc edits; no change to the readiness gate.
- 2026-09-05 (UTC) · opencode/muse-spark + E002/E008/hygiene batch · Marked not-yet-written task rows as intentional (Stage 2) so absence reads as design, not omission.
- 2026-09-05 (UTC) · opencode/muse-spark + E001/ADR-0008 · Assigned event pieces: substrate + SimEvent → T007 (scoped core exception, see ADR), IR types → T010, hooks → T014.
- 2026-09-06 (UTC) · opencode/muse-spark + spec-gap triage · Filed E009–E022 from the full-spec gap review and indexed all E-tasks in the blockers table; no spec, README, or source changes per file-only scope.
- 2026-09-06 (UTC) · opencode/muse-spark + E006-A/E009/E014/E015 + hygiene · Marked T006e merged (it landed in 8f2e38b; the "on branch" row was stale), corrected the throughput count to 13, locked the T007/T008 scope split (Timeline container vs advance rules), and recorded the four T007-unblocking decisions as done.
- 2026-09-06 (UTC) · opencode/muse-spark + T007 merged · Marked T007 done, T008 next, throughput at 14.
