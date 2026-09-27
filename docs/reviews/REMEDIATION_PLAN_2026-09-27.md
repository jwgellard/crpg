# Project gap remediation plan

Date: 2026-09-27 (UTC)

Status: proposed delivery plan, grounded in the current working tree. This is
a planning document, not implementation approval or a claim that the gaps are
closed. Work-package labels below are local to this plan; assign numbered task
files when each package is specified rather than preempting the task backlog.

## 1. Goal and delivery strategy

Deliver the project's actual promise: a creator can author a small campaign,
run it through the same authoritative implementation in embedded or dedicated
hosting, play it, save it, and resume it reliably. Preserve the Godot-free
simulation, data-driven rules, exact-build replay, and Windows-primary/Linux
headless support policies.

The remediation strategy is to prove integration earlier while retaining the
existing subsystem checks:

1. Close current combat correctness holes and reconcile T016 completion evidence.
2. Prove a genuinely different ruleset through production data and simulation.
3. Connect one authoritative action end to end, including a restart from disk.
4. Add the smallest complete authoring/gameplay loop and test it with a creator.
5. Establish package reuse and measurable operating limits before publication.

Authoring-write safety and input bounds can progress independently of the
gameplay sequence. Performance measurement starts with existing capabilities;
it does not wait for the entire product or require premature optimization.

## 2. Baseline and T016 recheck

Reviewed base: `76a48da` (T015 merge), plus the existing uncommitted T016a–d
working-tree implementation. Preserve that work and its historical records.

The current [T016d record](../../tasks/T016d.md) reports dual-native completion
and a 2026-09-27 correction from runtime early returns to compile-time test
selection. [T016b](../../tasks/T016b.md) and [T016c](../../tasks/T016c.md) also
carry review clarifications. Those improvements do not close the simulation
invariant findings below. The parent [T016](../../tasks/T016.md) still says
completion is pending, while its children report implementation/verification;
neither wording is sufficient evidence of merge or closure of new findings.

### Evidence checked for this plan

- Re-read `World` loading/despawn, the combat controller, persistence tests,
  T016 child review records, existing decision tasks, and dependency policy.
- Ran `cargo test -p crpg-sim --test combat_persistence --locked` on Windows:
  all three existing tests passed.
- Re-ran an isolated temporary Rust probe against the workspace APIs. It
  reproduced accepted saves missing the required roll tag, a next-action panic,
  acceptance of zero-health/alive state, and failure to reload a world after
  despawning all combatants. The probe is outside the repository; task closure
  must use checked-in regressions, not depend on that temporary artifact.
- The preceding project review passed Windows workspace tests, Clippy, format,
  dependency/determinism lints, 70 lint self-tests, and cargo-deny. Those are
  historical observations, not fresh full-gate or Linux verification here.

### Gap register

| Gap | Current evidence | Required closure |
|---|---|---|
| G1: combat lifecycle/save invariants | [World loading/despawn](../../crates/crpg-sim/src/world.rs) checks references but misses behavioral relationships; failures reproduced above | A: invalid saves fail at load, valid lifecycle transitions remain reloadable |
| G2: ruleset flexibility not integrated | [Combat data](../../crates/crpg-data/src/combat.rs) has one pool and Int-only declarations; [sim](../../crates/crpg-sim/src/combat.rs) uses the first ability, empty modifier contexts, actor-attribute DC, no natural selector, and automatic turn completion | B: materially different rulesets through the production path |
| G3: end-to-end critical path incomplete | Server/net/nav/script/persist/edit remain scaffolding; [E017](../../tasks/E017-t018-interface-debt.md) and [E022](../../tasks/E022-server-editor-api-shapes.md) identify missing interfaces | C and F: host-to-view-to-save slice, then minimal creator loop |
| G4: destructive authoring writes | [CLI writer](../../crates/crpg-cli/src/main.rs) truncates originals; its documented partial-write contract permits data loss | D: crash-safe file replacement and explicit migration recovery |
| G5: package reuse unproved | [Resolver](../../crates/crpg-data/src/package.rs) selects supplied metadata; combat fixture copies standalone ruleset documents | E: two campaigns load one verified local package |
| G6: acquisition bounds absent | CLI/replay use whole-file reads; [backlog](../../tasks/BACKLOG.md) still assigns deferred loader caps to completed T010 | D: bounded acquisition and bounded decoding |
| G7: performance targets not operational | [E019](../../tasks/E019-perf-measurability.md) remains open; no benchmark harness in the workspace | P: representative workloads, baseline policy, regression measurements |
| G8: status/contracts drift | README/PROJECT_STATE/BACKLOG still describe T015 as future work; testkit's “never ships” wording conflicts with its CLI runtime consumer | A0: concise, source-grounded status and ownership reconciliation |

## 3. Execution rules

- One implementation task changes one crate. A milestone spanning crates is
  split into ordered children with explicit allowed paths and stopping commands.
  The tables below group delivery work; they do not authorize multi-crate edits.
- Each child must first specify API/wire shapes, errors, limits, dependencies,
  acceptance tests, and out-of-scope work. Obtain the required ADR for public
  API changes. Open a crate with its architecture doc and `AGENTS.md`; extend
  existing docs instead of duplicating decisions.
- No dependency, new crate, ALLOWED-table change, `unsafe` exception, toolchain
  change, or `crpg-contracts` edit is authorized by this plan. Record required
  approvals in the owning task. Keep filesystem/process logic above core/rules/sim.
- Testkit remains a one-way integration consumer. Its current normal
  dependencies reach sim; do not add host/persist/net normal dependencies to
  it merely to share test helpers. Such changes can make higher-crate dev-edges
  illegal. Host integration tests should normally live in the host-owning crate.
- Do not weaken or delete tests to make a fix pass. If an approved behavior
  change conflicts with a current test, stop and obtain a separately scoped
  contract decision. Existing goldens are read-only; a necessary rebaseline
  requires review and independent generation on each supported native target.
- Preserve history and append signed documentation changes. Commit/PR/merge
  actions remain separately requested work.

## 4. Workstream A — close T016 correctness and status gaps

Priority: P0 for A1; P1 for lifecycle expansion. Dependency: current T016 tree.

### A0 — reconcile status and review closure (documentation only)

Update the parent/child completion ledger, BACKLOG, PROJECT_STATE, README, and
relevant ownership wording in a scoped documentation task. Record T015 as
merged, T016's actual working-tree/review state, remaining findings, and evidence
links. Do not replace prior agent logs or turn reported verification into a new
claim. Reconcile testkit's CLI-consumption wording without relocating code.

Acceptance: the same next milestone and merge state appear in every active
status summary; outstanding findings have an owner and no “all done” ambiguity.

### A1 — combat invariant repair (`crpg-sim`)

Specify one coherent invariant for loading and normal public lifecycle
operations. Audit the assumptions behind combat `expect`/assert sites against
what a loaded World can actually contain.

Required regression cases:

- Missing required roll tag or selected attribute: reject before publishing World.
- Contradictory health/dead/max-health state: reject; define legal zero-health
  and terminal-state semantics explicitly.
- Inconsistent pool/definition identities, costs, attribute declarations,
  duplicate placement identities, or active/timeline relationships: reject.
- Active participants must be alive and scheduled consistently; scheduled combat
  entries must be valid for the encounter. Define treatment of preexisting
  noncombat timeline entries instead of assuming a fresh empty world.
- Despawn active/inactive/dead/final participants; serialize and reload after
  every valid transition. In particular, despawning all participants must not
  produce a save rejected by the same implementation.
- Ordinary rejected actions retain byte-identical World/RNG/event state.

Use table-driven corrupt-save cases and bounded operation-sequence properties.
Round-trip validity must be checked after operations, not just at initial state.
Retain existing successful combat traces and goldens unless an independently
approved behavior change makes that impossible.

Focused commands:

```text
cargo test -p crpg-sim --test combat --locked
cargo test -p crpg-sim --test combat_persistence --locked
cargo test -p crpg-sim --test world --locked
```

Exit: checked-in regressions cover every reproduced failure and the full sim
gate passes on Windows/MSVC and Linux/GNU. This is the first implementation task.

### A2 — explicit encounter completion/reentry (`crpg-sim`)

After A1, decide the API and persistence semantics for terminal encounter
retention, release/cleanup, subsequent encounter initialization, and identity
reuse. Retain death results long enough for authoritative consumers; do not
equate death with despawn or clear unrelated world state.

Acceptance: start → fight → terminal → save/load → release → second encounter
works in the same world. Active-participant removal has a specified next-turn
or terminal outcome. Repeated cleanup is defined and cannot duplicate death or
refresh events. Invalid transitions fail without mutation. This is a reviewed
API extension, not an implicit expansion of T016's original one-fight contract.

## 5. Workstream B — make T017 a production-path abstraction gate

Priority: P1. Dependencies: A1 and approved T017 milestone contract; A2 for
multi-encounter coverage. Reuse T017 rather than creating a competing milestone.

Design backward from a small `srd-lite` encounter. Pin hand-calculated examples
before changing schemas. It must differ behaviorally from minimal-d6, not just
in die size or names. Specify the paired campaign/content arrangement, including
which references vary by ruleset and which scenario facts stay identical.

| Child | Sole crate | Deliverable and acceptance |
|---|---|---|
| B1 | `crpg-data` | Versioned authored shapes for the actual second-ruleset requirements: target defense, multiple pools/costs/abilities, natural-face selection, and the selected modifier/effect example. Preserve existing content through migrations and independent oracles. |
| B2, conditional | `crpg-rules` | Only primitives demonstrated missing by the two concrete cases. First try existing stats/modifiers/resolution/resources; record every required kernel change as an abstraction finding. No generic DSL or speculative spell system. |
| B3 | `crpg-sim` | Map authored definitions into real stat/modifier contexts; select target defense, use declared natural rules, retain multiple abilities/pools, and separate action completion from turn completion. Own modifier/effect lifetime in authoritative state. |
| B4 | `crpg-testkit` | Production-load both rulesets; independently assert expected actions, resource balances, target defenses, modifier breakdowns, natural outcomes, and save/load continuation. Add reviewed native goldens only after semantic oracles pass. |
| B5 | `crpg-cli` | Thin selection/run or replay surface over the landed APIs; black-box proof that both rulesets run outside the repository working directory. |

B1/B2 order is finalized at specification time against existing kernel APIs;
B3 waits for both required surfaces. B4 consumes all lower layers one-way.

Required scenario coverage:

1. Actor modifier affects a roll against a target-derived defense.
2. At least two usable abilities and two independently accounted resource pools.
3. An accepted action can leave the turn active; an explicit completion policy
   advances it. Invalid actions spend neither resources nor randomness.
4. Apply and remove one modifier-bearing effect with a bounded lifetime; save
   midway and preserve expiration and stacking behavior after reload.
5. A selected raw die invokes an authored natural-face rule.
6. Both rulesets use the same generic runtime path, without package-name branches.

Do not introduce Lua solely to satisfy this gate if these examples are fully
expressible as data and generic operations. Scripted extension behavior remains
an explicit later acceptance requirement, not something a data-only test proves.

Exit: both rulesets pass semantic and native replay tests. If adding the second
requires redesign, resolve that now and record it before protocol/editor APIs
are stabilized. A text search for game-specific names is supporting evidence,
not a substitute for this behavior test.

## 6. Workstream C — authoritative action, view, and restart slice

Priority: P1. Scope: one area, one encounter, minimal presentation. Architecture
decisions may be settled while B runs; runtime work consumes B's accepted model.

### C0 — resolve existing interface decisions (documentation only)

Close or scope the prerequisite portions of
[E003](../../tasks/E003-contracts-placement.md),
[E012](../../tasks/E012-binary-crate-naming.md),
[E017](../../tasks/E017-t018-interface-debt.md),
[E018](../../tasks/E018-privileged-channel-capabilities.md), and
[E022](../../tasks/E022-server-editor-api-shapes.md).

Decisions required before dependent implementations:

- Shared authoritative host placement, lifecycle, ownership, shutdown, and errors.
  Recommended starting point: evaluate a library target in `crpg-server`; this
  is a proposal, not a package-placement decision or dependency permission.
- Intent identity/sequence/tick semantics, duplicate/retry policy, validation
  outcomes, payload limits, entity binding, and ownership/authority checks.
- Which action/turn/damage/death results become events, their correlation and
  ordering, retention/drain policy, and per-client disclosure rules. A replay
  adapter's discarded return value cannot be the only presentation record.
- Snapshot/delta/replica query contracts; a client never receives an unrestricted
  authoritative World or unrestricted save bytes as its replication format.
- Minimal player/GM/admin capability policy and package filtering, with privileged
  operations denied until their explicit model is implemented.
- Campaign-level versus area-level persistent state, version/content identity,
  and ownership of future quest/script continuation data.

### C implementation packages

| Child | Sole crate | Depends on | Observable result |
|---|---|---|---|
| C1 | `crpg-sim` | B3, C0 | Required authoritative outcome events have real consumers/tests, deterministic order, and enough permitted facts for combat presentation. |
| C2 (T018) | `crpg-net` | C0, C1 contracts | Bounded codec and in-memory transport; malformed, duplicated, stale, oversized, and unauthorized intents cannot mutate state. |
| C3 | `crpg-persist` | A1/A2, C0, approved codec/compression dependencies | Versioned durable snapshot read/write with engine/content compatibility policy, symbolic state, bounded decompression, and recovery behavior. |
| C4 | Host crate selected by C0 | C1–C3 | One authority implementation accepts commands through transport, emits filtered results, and saves/restarts. Test embedded and dedicated adapters through this implementation. |
| C5 | `crpg-godot` | C4, approved bridge dependencies | A minimal Windows view submits an intent and renders its authoritative result; no gameplay authority or direct World mutation. |
| C6 | Host crate selected by C0 | C4/C5 interfaces | End-to-end headless acceptance plus embedded/dedicated lifecycle and restart tests. Keep transport-local fault tests in net. |

The host crate is unresolved only until C0; its task cannot start with an
unassigned owner. C3 may progress alongside C1/C2 after its state contract is
stable. Persistence/host integration tests live in their owning higher crate;
lower simulation layers never dev-depend upward on them or testkit.

Slice acceptance:

1. Load authored campaign, initialize the reusable host, and connect a client
   using in-memory transport; send one valid and one rejected action.
2. Assert authoritative outcome, permitted replica state, and visible event
   order independently. Hidden fields never reach the client.
3. Save at a nonterminal point, stop, create a new host, load from disk, replay
   the remaining inputs, and match the uninterrupted native hash sequence.
4. Exercise real Windows embedded and dedicated adapters and Linux dedicated
   hosting as they become available. Headless Linux has no Godot dependency.
5. Demonstrate the action in the minimal Windows view. This human smoke test
   supplements the headless assertion; it is not replaced by a hash-only test.

Real-network follow-on: implement QUIC in a separate `crpg-net` task, then
dedicated listener/reconnection integration in the host crate. Resolve the
ADR-0004 quinn datagram/reordering decision before depending on raw datagrams;
pin protocol limits, loss/jitter/retry tests, and documented initial port
forwarding behavior. The MVP two-client separate-server gate remains required;
the in-memory slice is an earlier milestone, not its replacement.

## 7. Workstream D — protect authored content and bound acquisition

Priority: P1 before external creator use or downloaded-content hosting.
Independent of B except for keeping the accepted schema inventory current.

| Child | Sole crate | Work and acceptance |
|---|---|---|
| D0 | Documentation only | Specify byte/count/depth limits and error precedence; approve filesystem replacement/durability strategy for Windows and Linux. Distinguish per-file atomic replacement from multi-file recovery. |
| D1 | `crpg-data` | Enforce supplied-document bytes, aggregate campaign bytes/counts, and parsing/collection bounds before expensive transformations. Preserve duplicate-key, migration, and diagnostic guarantees. |
| D2 | `crpg-cli` | Enforce bounded reads and directory traversal during acquisition, including cumulative limits and files changing while read. Use bounded catalog reads too. A metadata-length check alone is insufficient. |
| D3 | `crpg-testkit` | Bound replay/golden acquisition and decoding; reject excessive payload/list/string sizes before unbounded allocation. Existing post-decode input-count validation is not an acquisition bound. |
| D4 | `crpg-cli` | Replace truncate-in-place saves with same-filesystem staged writes and approved atomic replacement/recovery semantics; preserve source-drift, symlink, hard-link, no-op, and error guarantees. |
| D5 | `crpg-cli` | Define interrupted multi-file migration recovery with backup/journal or a staged transaction protocol. Recovery is deterministic, repeatable, and explained to authors. |

D1 precedes D2's policy integration; D4 precedes D5. D3 consumes the separately
specified replay limits, not campaign-specific assumptions. Obtain dependency
approval if portable safe filesystem primitives are insufficient; do not add
unsafe code outside the existing permitted crate.

Acceptance includes largest-valid/first-invalid inputs, many small files,
oversized strings, bounded nesting, interrupted writes, disk/write/sync/replace
failures, and process restart. Failed replacement preserves the prior complete
file according to the approved contract; interrupted multi-file work can be
recovered without guessing which versions were written. Test behavior on both
native targets, not only mocked I/O. Do not weaken existing tests whose contract
must be superseded; stop for the explicit contract-change task first.

Future editor writes must use a reviewed equivalent storage boundary. Do not
make `crpg-edit` depend upward on the CLI or persistence crate to reuse I/O code;
resolve reuse/adapter ownership in its opening task under the actual ALLOWED
table. Keep its document and command model filesystem-independent where possible.

## 8. Workstream E — reusable local content packages

Priority: P2, before declaring the authoring/package interface stable.
Dependencies: accepted ruleset schema (B), input limits (D1/D2).

E0 (documentation): distinguish the immediate local composition milestone from
later archive publishing/signing. Specify package identity/version/checksum
authority, canonical bytes, reference scope/collision policy, override policy,
engine compatibility, and content identity recorded in saves/replays.

| Child | Sole crate | Acceptance |
|---|---|---|
| E1 | `crpg-data` | Pure composition over caller-supplied verified inputs; deterministic lookup and diagnostics for collisions, missing packages, conflicting versions, and cross-package references. |
| E2 | `crpg-cli` | Bounded filesystem acquisition and actual package-byte verification against the lock; resolve local locations explicitly, never treat a declared checksum as verification. |
| E3 | `crpg-testkit` | Two small campaigns consume the same ruleset package without copied ruleset documents; both run through production composition and sim. |
| E4 | Host crate selected by C0 | Runtime content binding rejects a missing/tampered/incompatible package before authority starts; save/replay mismatch errors distinguish content incompatibility from simulation divergence. |

Acceptance: change a package byte without changing its lock and loading fails;
upgrade a package through the documented workflow and both compatible campaigns
still work; identities survive source-file moves. Preserve old fixture meaning.
If replay envelope changes are required, specify a separate testkit task after
E0 rather than placing replay semantics in CLI or host code.

Before distributing archives, separately task safe extraction, final manifest
verification, and server-to-client content filtering. Test absolute/parent
paths, symlinks, case collisions, compression bombs, and hidden logic/assets.
A registry, signing UI, and online publishing are not prerequisites for E1–E4.

## 9. Workstream F — minimal creator loop and MVP completion

Priority: P2 after C's action/restart slice; design the creator exercise early.
Use the spec's one-area/one-NPC/one-quest scope, not the larger Greenhollow demo.

Specify these missing prerequisites as separate one-crate tasks:

| Package | Sole crate | Required proof |
|---|---|---|
| F1 | `crpg-nav` | Approve a supported bake/query implementation and format; same baked mesh serves Windows client and Windows/Linux headless queries. No Godot dependency or new unsafe exception. |
| F2 | `crpg-sim` | Authoritative movement/interaction/trigger transitions consume permitted inputs; navigation orchestration ownership must respect sim's lack of an allowed nav edge. |
| F3 | `crpg-data` | Only authored dialogue/quest/command shapes actually missing from the existing schemas; version/migrate changes. |
| F4 | `crpg-script` | Minimal dialogue-choice/quest transition execution with world-owned serializable continuation state where needed. Resolve E010 before Lua: deterministic budgets, explicit global stripping, and nested-call failure semantics. |
| F5 | Host crate selected by C0 | Wire movement, interactions, script results, and quest persistence; one death transitions the quest exactly once, including across save/reload. |
| F6 | `crpg-edit` | Headless edit/undo/redo/validate commands create the tiny campaign; random command sequences preserve document invariants and undo correctness. |
| F7 | `crpg-cli` | Thin authoring command interface over F6 if required for the creator exercise; no second editor semantics. |
| F8 | `crpg-godot` | Minimal editor shell/property forms and Play, reusing F6 and C's host boundary; movement/dialogue/quest/combat views display authoritative results. |

F1/F2 require an explicit orchestration decision before implementation; do not
solve it by adding an upward dependency. F3 is conditional on a concrete schema
gap. If script-owned quest transitions require additional sim operations or
state, land those in a separate sim prerequisite before F4/F5. Similarly,
persist new campaign/continuation state through separately scoped owner tasks.

Creator acceptance: a person unfamiliar with the JSON creates the campaign,
places the actors, edits a dialogue and a combat value, diagnoses one invalid
reference, uses Play, starts/completes the quest, saves/quits/loads, and repeats
against a separate server with two clients. Record task time, assistance,
confusing diagnostics, and lost-work incidents. Fix observed blockers before
expanding editor breadth. Add a short authoring guide based on this exercise.

## 10. Workstream P — measure before optimizing

Priority: P2; begin after A1, expand with B/C/F. Close
[E019](../../tasks/E019-perf-measurability.md) rather than adding another policy.

- P0 (documentation): define reproducible seeds/content, active versus static
  versus visible populations, hardware/build profile, warm-up, repetitions,
  variance handling, and absolute versus relative budgets. The 200-character
  successful spike and 43.7 FPS at 1,000 visible characters do not establish
  60 FPS at every proposed load or on older hardware.
- P1 (`crpg-sim`): representative deterministic workloads for current combat
  and tick operations, reporting workload results so an empty loop cannot
  masquerade as meaningful throughput. Add owner-local benches in rules/nav/net
  later as separate tasks when those workloads exist.
- P2 (`crpg-cli`): a benchmark/reporting command over approved workload APIs;
  clocks remain above core/rules/sim. Measure sim separately from JSON hashing,
  serialization, and I/O so regression attribution is useful.
- P3 (`crpg-godot`, after C5): reproduce the bulk-snapshot bridge measurement
  with actual scene/animation costs and defined visible populations. Reconcile
  the LOD/ceiling decision with measurements before promising the ceiling.
- P4 (CI-only task): activate reporting first, then the approved regression
  gate when the runner/baseline policy is repeatable. Maintain Windows primary
  measurements and genuine Linux headless measurements independently.

Do not demand hypothetical AI/pathfinding/bandwidth measurements before those
features exist. Activate each budget with a representative capability; no
skipped green placeholder gate and no optimization without a measured cause.

## 11. Ordering and milestone exit criteria

| Milestone | Dependencies | Exit evidence |
|---|---|---|
| M0: trustworthy current combat | A0, A1; A2 before multi-encounter claims | Regression cases, complete native checks, consistent status record |
| M1: credible rules abstraction | B1–B5 and required A2 lifecycle | Two production-loaded rulesets, independent semantic oracles, native replays |
| M2: smallest runtime product slice | C0–C6, M1, snapshot bounds | Intent → authority → permitted view → disk save → new host → identical continuation |
| M3: safe authoring/content reuse | D, E | Crash recovery, bounded inputs, two campaigns using one verified package |
| M4: tiny creator-authored MVP | F, M2/M3, real-network follow-on | Creator exercise and spec §17.2 embedded/dedicated two-client acceptance |

Performance P runs alongside these milestones; its capability-specific gates
join the owning milestone once measurable. D is not deferred until M3 if
creators or downloaded content arrive earlier. M0/M1 gate downstream runtime
API stabilization, but C0 decisions and bounded independent documentation work
can proceed while they are implemented.

Immediate execution queue:

1. Specify A1 in `crpg-sim`, with checked-in regressions for the reproduced cases.
2. Reconcile A0 status, then specify A2's lifecycle ADR/API extension.
3. Specify the T017 acceptance scenario and B child boundaries.
4. Specify D0–D4, starting acquisition and write protection as separate tasks.
5. Resolve C0's minimum existing E-series decisions and task the action/restart
   slice. Avoid designing the entire eventual editor or extension ABI now.

## 12. Verification and completion records

Each executable child states a literal focused test command and a final
`cargo test -p <owning-crate> --locked` stopping command. Proposed future suite
names are not existing gates; establish them in the child's contract. Run all
applicable root/crate rules and current CI checks, including:

```text
cargo fmt --all
cargo clippy -p <crate> --all-targets -- -D warnings
cargo test -p <crate>
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo deny check
git diff --check
```

These placeholders are expanded in implementation task files. Inspect format
diffs for unrelated changes. Record native Windows/MSVC and Linux/GNU results
with the pinned toolchain and required features/profile; Linux GUI is not a
release promise. Golden comparison uses compile-time target selection and
independent target baselines, never cross-target equality. Record remote-only
guard/secret-scan results from CI, not from a local Cargo success.

Each closure record includes revision/tree state, commands, target, outcome,
new regressions, artifact provenance where relevant, unresolved decisions, and
merge status separately. “Specified,” “implemented,” “verified,” and “merged”
are distinct states. A previously green suite is not closure evidence for a
newly discovered invariant hole.

## 13. Scope boundary

This plan closes reviewed risks and orders existing MVP work. Full PF2e, a
universal rules DSL, online package registry, matchmaking/relay services, and
native extension loading remain later work. In particular,
[E023](../../tasks/E023-native-extension-loading-and-packaging.md) must resolve
native ABI/loading and unsafe governance before that feature is implemented;
it is not a prerequisite for portable data-driven campaigns.

Success is measured by reliable completed creator/player workflows, supported
by the existing strict tests—not by crate count, task count, or additional
abstractions.

## 14. Progress ledger

Work-package status with executable task links. "Specified,"
"implemented," "verified," and "merged" are distinct; merge status is
recorded separately from implementation status. Evidence links point at the
owning task's completion record, which holds the native gate detail.

| Package | Task file | Owning crate | State | Evidence / blockers |
|---|---|---|---|---|
| A1 combat invariant repair | [T016e](../../tasks/T016e.md) | `crpg-sim` | Landed 2026-09-27 (PR #14, `4686a73`) | 63 sim tests (incl. 4 new persistence regressions), full workspace + deny green on Windows/MSVC and genuine Linux/GNU; one approved test-only alignment in `tests/combat.rs` (intent preserved) |
| A0 status reconciliation | [A0](../../tasks/A0-status-reconciliation.md) | Documentation only | In progress | T015 marked merged (PR #13); T016 family headers aligned with their records; testkit wording split; this ledger |
| A2 encounter completion/reentry | [T016f](../../tasks/T016f.md) | `crpg-sim` | Landed 2026-09-27 (PR #14, `4686a73`) | ADR-0014 (release op + summary, despawn-active advance, reentry) approved in-session; 72 sim tests (9 new), full workspace + deny green on Windows/MSVC and genuine Linux/GNU |
| B second-ruleset proof (T017) | [T017](../../tasks/T017.md) | Per child | Landed 2026-09-27 (PR #14, `4686a73`), all 9 checks green | B1–B5 landed: [T017a](../../tasks/T017a.md) data vocabulary + srd-lite content, [T017b](../../tasks/T017b.md) sim compat (ADR-0015, retired), [T017c](../../tasks/T017c.md) CLI consumer, [T017d](../../tasks/T017d.md) generalization (ADR-0016), [T017e](../../tasks/T017e.md) replay + native goldens, [T017f](../../tasks/T017f.md) CLI proof; B2 vacuous, legacy goldens identical |
| D authoring safety + input bounds | To specify (D0–D5) | Per child | Specified (plan §7), pending | D0 filesystem/limit strategy first; independent of gameplay sequence |
| C action/view/save slice | To specify (C0–C6) | Per child + host TBD by C0 | Specified (plan §6), pending | Blocked on C0's E003/E012/E017/E018/E022 decisions; runtime work consumes B |
| E local package reuse | To specify (E0–E4) | Per child | Specified (plan §8), pending | Depends on B's schema and D1/D2 limits |
| F creator loop / MVP | To specify (F1–F8) | Per child | Specified (plan §9), pending | Depends on C's slice, M3, and the real-network follow-on |
| P performance measurement | To specify (P0–P4) | Per child | Specified (plan §10), pending | P0 policy first; P1 workloads after A1 (this ledger's A1 row is that gate) |

## Agent log

- 2026-09-27 (UTC) · opencode/gpt-6-astra + project gap remediation plan · Rechecked the visible T016 review updates and reproduced the remaining combat invariant failures before sequencing remediation. Defined per-crate work packages, existing decision dependencies, milestone acceptance, and evidence requirements to connect subsystem quality to a usable creator/player workflow.
- 2026-09-27 (UTC) · opencode/muse-spark + A1 ledger entry · Recorded T016e's implemented-and-dual-native-verified repair (unmerged) with its evidence link and the approved test-only alignment; A2/B/C/D/E/F/P remain specified-pending with the plan's dependency ordering unchanged.
- 2026-09-27 (UTC) · opencode/muse-spark + A2 ledger entry · Recorded T016f's implemented-and-dual-native-verified completion/reentry work (ADR-0014 approved in-session, unmerged) with its evidence link; B/C/D/E/F/P remain specified-pending with the plan's dependency ordering unchanged.
- 2026-09-27 (UTC) · opencode/muse-spark + T017/B1 ledger entry · Recorded the approved T017 milestone contract (hand-calculated srd-lite content, vacuous B2 with kernel mapping) and the specified B1 data contract; B1 implementation is next.
- 2026-09-27 (UTC) · opencode/muse-spark + B1/T017b/T017c ledger resolution · Recorded B1 implemented and dual-native-verified with T017b sim compat (ADR-0015 Accepted boundary) and the T017c CLI consumer fix, full workspace plus deny green on both natives with legacy goldens identical, all unmerged; B3 generalization next with its own ADR.
- 2026-09-27 (UTC) · opencode/muse-spark + T016/T017 landed · Marked work packages A1/A2/B landed (PR #14, `4686a73`, all 9 checks green) with B2 confirmed vacuous; C/D/E/F/P remain specified-pending with the plan's dependency ordering unchanged.
