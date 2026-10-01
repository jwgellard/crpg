# Implementation plan to project completion

Status: **plan of record for delegation, 2026-10-01 (UTC).** It orders the
work from the current frontier (Phase 4, after PR #21) to the end of spec
§18 Phase 12. It does not approve any contract, dependency, ADR or golden.
Every work package still needs its own exact task file and the human gates
named below.

Authority and inputs: `docs/CRPG_ENGINE_SPEC.md` §§17–19 (MVP, roadmap,
milestone backlog), `docs/AGENTIC_WORKFLOW_PLAN.md` (roles, pipeline,
parallelism, never-delegated list), `tasks/POST-T018.md` (ordered contracts),
decisions D01–D18 (`tasks/POST-T018-DECISIONS.md`) and D19–D26
(`tasks/DECISIONS-2026-09-30.md`), the E022 API-shape ledger in
`tasks/BACKLOG.md`, and `tools/lint/deps.py` `ALLOWED`. Where this plan and
one of those disagree, those win and this plan is corrected by an appended
note.

---

## 1. How work is delegated

### 1.1 Work-package kinds

Every row in §3 is one of these. Kinds map to the four roles of the workflow
plan (§3 there).

| Kind | Who | Produces | Stops when |
|---|---|---|---|
| **A** — architecture | Architect agent (no code) | ADR draft in `docs/adr/`, status *Proposed* | Draft written; human accepts or returns it |
| **C** — contract | Spec agent (no code) | Exact one-crate contract appended to `tasks/TNNN.md`: Interface (exact signatures), Behaviour, Constraints, Test (exact command), Out of scope, dependency audit if any | Contract written; human approves it |
| **I** — implementation | Implementer agent | Branch + PR, one crate, completion record in the task file | The task file's Test command and every CI gate pass locally |
| **G** — grunt | Low-cost agent | Content/data/docs/bookkeeping | `crpgc validate` (data) or the doc rule (docs) passes |
| **H** — human gate | The user | A recorded decision, approval, review or sign-off | — |

A package moves `contract-needed → contract-review (H) → ready → in progress →
on branch → done`. Only `ready` packages are handed to an implementer. A
package whose Interface cannot be written yet goes back to an A package, not
forward to an implementer (workflow plan §7 Stage 2).

### 1.2 Standing rules every agent receives

These are restated from `AGENTS.md`, not new:

- One task, one crate. Two crates means split the task.
- Branch `task/TNNN-<slug>` in its own git worktree. Never share a checkout.
- Run `bash tools/preflight.sh --crate <crate>` (or `tools/preflight.ps1`) before declaring done. CI
  runs Windows/MSVC and Linux/GNU; both must be green.
- **Stop and report, do not work around**, on any of:
  - an ambiguous contract;
  - a second crate;
  - a new dependency not already approved in a decision record;
  - a golden hash that changes;
  - `unsafe` outside `crpg-godot`;
  - an edit to `crpg-contracts` or `rust-toolchain.toml`;
  - a test that has to be weakened.
- The first task with real code in a crate writes
  `docs/architecture/<crate>.md` and `crates/<crate>/AGENTS.md`, and updates
  the index row in `docs/architecture/README.md`.
- Doc edits carry an agent-log entry. No model identifiers in repository
  artifacts.

### 1.3 Implementer prompt (template)

```text
Read AGENTS.md, crates/<crate>/AGENTS.md (if it exists), tasks/POST-T018-RULES.md
and tasks/<TNNN>.md. Implement exactly that task in <crate> only, on branch
task/<TNNN>-<slug> in a fresh worktree from master. Stop when
`<task Test command>` and `bash tools/preflight.sh --crate <crate>` pass. Write the
completion record in tasks/<TNNN>.md. Open a PR; do not merge it. If the task
is ambiguous, needs a second crate or an unapproved dependency, would change a
golden, or would weaken a test: stop and report why.
```

The contract (C) and architecture (A) prompts are the same shape. They say
"write, do not implement", and they give the decision records and ADRs to cite.

### 1.4 Concurrency

Following workflow plan §9, up to **two implementers and one spec/architect
agent** run at once. Serialize anything that touches these, one at a time and
human-reviewed:

- `crpg-contracts`;
- the `World` struct or tick order in `crpg-sim`;
- schema versions and migrations in `crpg-data`;
- the GDExtension boundary in `crpg-godot`;
- the workspace `Cargo.toml`, `deny.toml` or `.github/workflows/`.

Packages marked **S** in §3 are in this serialized set.

### 1.5 Human gates that stay human

These come from the workflow plan §12 and spec §17. An agent may draft any of
them. Only the user decides.

| Gate | When |
|---|---|
| H-contract | Approve each C/A output before its I package starts |
| H-dep | Approve each new third-party dependency (batch requests; see §2) |
| H-golden | Accept any new or changed golden, with native provenance |
| H-unsafe | Read every `unsafe` block in `crpg-godot` |
| H-sandbox | Read every line of the Lua deny-list and budget enforcement |
| H-filter | Read client intent validation and per-client filtering changes |
| H-merge | Decide a PR is finished. Proposed delegation in §1.6 |
| H-play | Phase 5: play `combat_basic` end to end at 60 fps on Windows |
| H-author | Phase 6: someone who has never seen the JSON builds the MVP campaign in the editor |
| H-legal | Phase 11: ORC licence review **before** any PF2e content |
| H-release | Publish any public build |

### 1.6 Merge delegation (proposed, needs the user's decision)

The workflow plan keeps "the decision that a task is finished" human.

*Proposal:* an orchestrating agent may merge a PR on its own only when **all**
of these hold:

- both OS checks are green on the head commit;
- the PR touches one crate plus its own docs and task file;
- no golden, `Cargo.lock`, `deny.toml`, workflow, `crpg-contracts`,
  `crpg-sim` `World` or tick-order change;
- no existing test assertion modified or removed;
- no `unsafe`;
- the task's contract was H-approved.

Everything else waits for the user. Until the user approves this in a decision
record, every merge is H-merge.

---

## 2. Decisions to batch ahead of the waves

Agents stall on missing decisions, not missing code. Ask for these as one
batch (proposed **D27**) before the wave that needs them. Each is an H gate.

| # | Decision | Needed by | Recommendation to put to the user |
|---|---|---|---|
| a | §1.6 merge delegation policy | Wave 1 | Adopt, with the listed exclusions |
| b | `crpg-persist` compression dependency (`zstd` or none) | T038 | `zstd` with input-size and decompressed-size caps (S001); or uncompressed v1 if the dependency is refused |
| c | Navigation approach: in-house grid/navmesh vs a Recast binding | T035 | In-house pure-Rust navmesh/grid first (no C build, deterministic); revisit Recast only on measured need |
| d | `godot` (gdext) crate and Godot engine version pin | T047 | Pin the versions ADR-0003's spike used, or the current stable 4.x, exact pins, `crpg-godot` only |
| e | `mlua` with vendored Lua 5.4 | T055 | Adopt per ADR-0005; `crpg-script` only; Lua not needed for the MVP (it uses an IR hook) |
| f | MVP scope confirmations: IR quest hook instead of Lua; minimal AI in `crpg-ai` | T052–T056 | Confirm as written in spec §17.3 |

---

## 3. Milestones and work packages

New task ids start at **T034**. A row is a place in the order, not a design.
Each I row needs its C row approved first unless it says **ready**.

### M4 — Authoritative server and networking (spec Phase 4)

**Exit criteria** (spec §18 Phase 4, ADR-0012). All must hold:

1. Two headless scripted clients connect over real QUIC to a dedicated server,
   move, fight, and end with identical visible state.
2. A 5,000-tick desync test with loss and jitter passes.
3. The malicious-client suite passes.
4. Reconnection within the 30-second grace works.
5. Save/load equivalence holds (gate 10).
6. Windows and Linux dedicated-server artifacts build (gate 12).
7. Smoke tests pass for Windows embedded, Windows dedicated and Linux
   dedicated (gate 13).

**4A — host and transport (critical chain)**

| Id | Kind | Crate | Prerequisites | Notes |
|---|---|---|---|---|
| T022 | I **ready** | crpg-server | — | Host capture/checkpoint slice (ADR-0022, D20). Opens the crate: writes the architecture doc and `AGENTS.md`, plus the dependency audit |
| T023 | C → I, S | crpg-net | T022 API on branch | Endpoint, stream framing, backpressure and close; D03 pinned certificate plus invitation handshake; D23 pins; ADR-0023 skip in `deny.toml` |
| T023b | C → I | crpg-server | T022, T023 | QUIC host adapter: bind, start, pump, shutdown, errors |
| T024 | C → I | crpg-net | T021, T023b | D11 versioned bounded snapshot transfer |
| T025a | C → I | crpg-net | T024 | D12 resume/resync wire mechanisms |
| T025b | C → I | crpg-server | T022, T023b, T025a | 30-second same-process grace and session fencing |
| T043 | C → I | crpg-server | T022 | Intent rate/queue policy and rejection codes (E022 ledger). H-filter |

**4B — interest management**

| Id | Kind | Crate | Prerequisites | Notes |
|---|---|---|---|---|
| T027b | C → I | crpg-net | T027a, T021, T024 | D14 whole-area baseline plus private-field grants; leak test. H-filter |
| T027c | C → I | crpg-server | T022, T027b | Authenticated viewer context |

**4C — movement (D24: specified after T024)**

| Id | Kind | Crate | Prerequisites | Notes |
|---|---|---|---|---|
| T034 | A | — | T024 merged | Movement and navigation ADR: D13 path format, movement rules, numeric limits, and how sim consumes caller-supplied nav results (sim may not import nav) |
| T035 | C → I | crpg-nav | T034, D27c | Navigation v1 (spec §19.1 #21). Opens the crate |
| T036 | C → I, S | crpg-sim | T034 | Authoritative movement; `World`/tick-order touch; new goldens (H-golden) |
| T026p | C → I, S | crpg-net | T023, T036 | Lane-1 movement and acknowledgments. Vendors the T030 dedup patch (D23); re-runs the impairment probe on native Windows |
| T037 | C → I | crpg-server | T026p, T035 | Movement host adapter |

**4D — persistence**

| Id | Kind | Crate | Prerequisites | Notes |
|---|---|---|---|---|
| T038 | C → I | crpg-persist | D27b | Versioned save envelope with input and decompressed caps (S001). Opens the crate |
| T039 | C → I | crpg-server | T022, T038 | Checkpoint/save adapter plus gate 10 native save/load equivalence |

**4E — products and acceptance**

| Id | Kind | Crate | Prerequisites | Notes |
|---|---|---|---|---|
| T040 | C → I, S | crpg-server | T023b | Dedicated binary (Windows plus Godot-free Linux); CI artifact jobs for gate 12 |
| T041 | C → I | crpg-testkit | T025b, T027c, T037, T039 | Scripted headless clients; 5,000-tick loss/jitter desync; two-client identical visible state; gate 13 dedicated smoke |
| T042 | C → I | crpg-net | T024 | Wire-level malicious-client suite (spec §19.2 #21); host-level cases live in T041 |
| T044 | C → I | per subsystem, then crpg-cli | — (parallel filler) | D06 workload APIs, then a thin `crpgc bench`. Thresholds only after baselines |

### M5 — Client (spec Phase 5, Windows primary)

**Exit:** H-play. A human plays `combat_basic` end to end at 60 fps on
Windows/MSVC, through the embedded server.

| Id | Kind | Crate | Prerequisites | Notes |
|---|---|---|---|---|
| T045 | A | — | M4 4A done | Bridge ADR: §11.3 FFI surface, threading, embedded host via `crpg-server`, error mapping, `net_id`↔`EntityId` view, `client/` project layout (E012/E022) |
| T046 | C → I | crpg-net | T024, T027b | Replica storage, delta application, engine-neutral read queries |
| T047 | C → I, S | crpg-godot | T045, T046, D27d | Bridge skeleton: gdext extension, `client/` project, embedded host, proxy spawn from a bulk snapshot (§19.1 #22), Windows client build (gate 12). Opens the crate. H-unsafe |
| T048 | C → I | crpg-godot | T047 | Scene sync and proxy lifecycle (§19.2 #12) |
| T049 | C → I | crpg-godot | T047 | Interpolation buffer (§19.1 #23) |
| T026 | C → I | crpg-godot | T026p, T037, T049 | Own-movement prediction and reconciliation |
| T050 | C → I | crpg-godot | T048 | Camera, click-to-move, selection |
| T051 | C → I | crpg-godot | T048 | HUD, combat log, `SimEvent`-driven VFX/audio hooks |

### M5′ — MVP gameplay (parallel with M5; needed by spec §17.2)

| Id | Kind | Crate | Prerequisites | Notes |
|---|---|---|---|---|
| T029b | I **ready** | crpg-script | — | Synchronous trusted bindings (D17, D21). Opens the crate |
| T052 | C → I, S | crpg-sim | D27f | Persisted continuation state for dialogue, quests and waits; `World` touch |
| T053 | C → I | crpg-script | T029b, T052 | Event IR interpreter with serializable `Wait` continuations (§19.2 #19) and D17 budgets |
| T054 | C → I | crpg-script | T053 | Dialogue engine and quest state machine over the existing `crpg-data` dialogue/quest documents |
| T056 | C → I | crpg-ai | T028, T036 | MVP AI: move to nearest hostile and attack, consuming `legal_actions` only. Opens the crate |
| T057 | C → I | crpg-godot | T051, T054 | Dialogue UI and quest journal |

### M6 — Editor v1 (spec Phase 6; headless command API first, per D18)

**Exit:** H-author. The MVP campaign (spec §17.2) is built entirely in the
editor by someone who has never seen the JSON.

| Id | Kind | Crate | Prerequisites | Notes |
|---|---|---|---|---|
| T058 | C → I | crpg-edit | — (data APIs exist) | `EditCommand`, document, undo, validation, receipts; random-command-sequence property test (§19.2 #15). Opens the crate. **Can start in Wave 2** |
| T059 | C → I | crpg-cli | T058 | `crpgc apply` over the command API |
| T060 | C → I | crpg-godot | T047, T058 | `editor/` shell: read-only campaign explorer (§19.1 #24), problems panel (#26) |
| T061 | C → I | crpg-godot | T060 | Schema-generated property forms (#25) |
| T062 | C → I | crpg-godot | T061 | Placement tools: select, move, rotate, snap, duplicate, delete (§19.2 #16) |
| T063 | C → I, S | crpg-data, then crpg-godot | T062 | Terrain heightfield data (schema version, H review), then sculpt/paint (§19.2 #17). Two tasks |
| T064 | C → I | crpg-godot | T035, T062 | Navmesh bake in the editor with a walkable overlay (§19.2 #18) |
| T065 | C → I | crpg-godot | T061 | Creature editor |
| T066 | C → I | crpg-godot | T061 | Dialogue editor with text round-trip |
| T067 | C → I | crpg-godot | T061 | Quest editor |
| T068 | C → I | crpg-godot | T047, T060 | Play: in-process server plus client from the editor (§17.2 #6) |

### M7 — MVP integration and hardening (spec Phase 7)

**Exit:** all twelve spec §17.2 items demonstrated, with gates 10, 12 and 13
green on their targets. Then H sign-off.

| Id | Kind | Crate | Prerequisites | Notes |
|---|---|---|---|---|
| T069 | G | `campaigns/mvp` | M6 | MVP campaign content, `crpgc validate` loop |
| T070 | C → I, S | crpg-testkit (+ CI) | T069, T041 | Automated §17.2 acceptance (headless where possible); smoke on Windows embedded, Windows dedicated and Linux dedicated with two clients |
| T071 | G | docs | M6 | Authoring guide (`docs/guides/`) and `docs/contracts/` |
| T072 | C → I | crpg-data, then host/client | D03 | Client presentation-manifest export/import and strip leak test (E022 ledger, Phase 7) |
| T07x | — | as found | — | Integration fixes, filed as they appear (Phase 7 exists for this) |

### M8 — Event graphs, dialogue depth, quests; Greenhollow (spec Phase 8, §17.4)

| Id | Kind | Crate | Prerequisites | Notes |
|---|---|---|---|---|
| T055 | C → I | crpg-script | D27e, T053 | `mlua` sandbox: deny-list and budget enforcement (§19.1 #18). H-sandbox |
| T073 | C → I | crpg-script | T053 | Graph compiler: graph documents to IR |
| T074 | C → I | crpg-godot | T073, T060 | Visual graph editor |
| T075 | C → I | crpg-data | — | Quest reachability analysis |
| T076 | C → I, S | crpg-data → crpg-sim | T052 | Triggers, doors/keys, containers, traps. Split per crate at contract time |
| T077 | C → I, S | crpg-data → crpg-sim | T076 | Shops and loot tables |
| T078 | G | rulesets/srd-lite | — | Levels 1–2 |
| T079 | G + I | campaigns, crpg-testkit | T076–T078 | Greenhollow, plus two-player acceptance. Its first H-author session is the honest editor usability test |

### M9 — AI (spec Phase 9)

| Id | Kind | Crate | Notes |
|---|---|---|---|
| T080 | C → I | crpg-ai | Utility scorer with data-driven weights (§19.2 #20) |
| T081 | C → I, S | crpg-data | AI profiles as data (schema change) |
| T082 | C → I | crpg-ai | Behaviour trees for schedules |
| T083 | C → I | crpg-ai | Party AI |
| T084 | C → I | crpg-ai, then crpg-cli | Encounter dry-run simulator (§19.2 #23) and its CLI wrapper |

### M10 — Multiplayer hardening (spec Phase 10)

| Id | Kind | Crate | Notes |
|---|---|---|---|
| T085 | A, then C → I | crpg-net, then crpg-server | Privileged GM/admin control protocol (D03, separate fail-closed protocol), live-edit conflicts |
| T086 | C → I | crpg-server | Party management |
| T087 | C → I | crpg-godot / tools | Multi-client test launcher with network-condition simulation (§19.2 #24) |
| T088 | C → I | crpg-server / crpg-cli | Admin tooling |
| T089 | C → I, S | CI | Public Windows/MSVC and Linux/GNU dedicated-server release builds. H-release |
| T090 | G | docs | Operator port-forwarding guide (D05). UPnP/IGD only by later decision |
| — | H | — | Record "no multiplayer host migration" (D18) as an ADR |

### M11 — PF2e (spec Phase 11; its own sub-roadmap)

| Id | Kind | Notes |
|---|---|---|
| — | H-legal | ORC compliance review first. Nothing below starts without it |
| T091 | A | PF2e sub-roadmap ADR: action economy, conditions, levels 1–5 and four classes, expressed without kernel changes. A required kernel change is an abstraction-test failure and goes back to the Architect |
| T092+ | G, batched | Class/feat/spell content in `rulesets/pf2e/` through the `crpgc validate` loop, with rules-suite cases per batch |

### M12 — Modding, packaging, distribution (spec Phase 12)

| Id | Kind | Notes |
|---|---|---|
| — | H | Decide E023 (native loading, ABI, signing trust root) or keep deferring it |
| T093 | C → I | `.crpg` packaging with hashing and signing (§19.2 #22), `crpgc pack`, malicious-archive tests |
| T094 | C → I | Campaign publishing and dependency resolution |
| T095 | A | Third-party server extension story (WASM only if needed) |

### M13 — Polish, performance, platforms (ongoing)

- Turn D06 baselines into gates per target.
- Bridge LOD.
- Platform polish.
- Filed as measured needs, not up front.

---

## 4. Waves (what can run at once)

A wave starts when its prerequisites merge, not on a date. "Spec" is the one
spec/architect slot; "Impl" is the two implementer slots.

| Wave | Impl slots | Spec slot | H gates in the wave |
|---|---|---|---|
| **1 (now)** | T022, T029b | T038 contract; then T058 contract | D27 batch; approve the T038 and T058 contracts |
| **2** | T023 (after its contract), T038 or T058 | T023 contract (after the T022 API is on branch), T043, T052 | Approve the T023 contract (S); T029b/T022 merge review |
| **3** | T023b, T039, T058/T059, T052 | T024, T027b | Golden review for T052 if hashes move |
| **4** | T024, T043, T053 | T025a, T034 (movement ADR) | Accept the movement ADR |
| **5** | T025a, T027b, T035, T054 | T036, T026p, T042, T040 | Approve the nav dependency (D27c) |
| **6** | T036 (S), T025b, T027c, T042 | T037, T041, T045 (bridge ADR) | H-golden for movement |
| **7** | T026p (S), T037, T040 (S), T056 | T046, T047 | Accept the bridge ADR; godot pin (D27d) |
| **8** | T041 (M4 exit), T046, T047 (S) | M5/M6 contracts | **M4 exit review**; H-unsafe |
| **9+** | M5, M5′ and M6 packages as their contracts clear | ahead by one wave | H-play, H-author |
| later | M7 → M13 in milestone order | — | MVP sign-off, H-legal, H-release |

Filler for an idle implementer slot: T044 workloads, T042, T075, and per-crate
documentation catch-up.

---

## 5. Cross-cutting tracks

| Track | Kind | Owner | Notes |
|---|---|---|---|
| Bookkeeping after each merge | G | docs | BACKLOG row, `docs/PROJECT_STATE.md`, `AGENTS.md` note, throughput line |
| Merge queue | H | repo settings | Workflow plan §7 Stage 4. Catches pass-alone/fail-together branches once two implementers run |
| Windows self-hosted runner | H | infrastructure | Slow layer (goldens, perf, product builds, smoke). S001 rule: never `pull_request`-triggered |
| Data loader caps / D0 filesystem proposal | H then C → I | crpg-data | Independent. The proposal is not approved by this plan |
| Throughput and cost | G | `tasks/BACKLOG.md` | Workflow plan §13 |

---

## 6. Completion

The project is complete for this plan when M12's packages are merged with both
native gates green. Phase 13 is open-ended and has no completion line.

The MVP sign-off at the end of M7 is the milestone to measure progress
against. Spec §0.3's honest-timeline warning applies: M5, M6 and M11 are where
estimates slip.

---

## Agent log

- 2026-10-01 (UTC) · claude-code + implementation plan · Laid out the delegated path from the post-PR #21 frontier to Phase 12, with work-package kinds, waves, concurrency limits and the human gates the workflow plan keeps human; merge delegation is a proposal pending the user's decision, not an approval.
