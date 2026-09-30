# Post-T018 — dispatch after delegated decisions

**Current dispatch update (2026-09-29):** consult
[SPECIFICATION-READINESS.md](SPECIFICATION-READINESS.md). T020, T021, T027a,
T028, T029a and T029b now have exact specification appendices; ADRs 0017–0020
are filed as delegated selections. T030–T032 specify independent preparation,
preflight and documentation work. The assignments below retain historical
context; their missing-exact-contract wording is superseded only for those tasks.

The [decision record](POST-T018-DECISIONS.md) settles D01–D18's option choices.
The next work is concrete implementation/specification, not another round of
the same user questions. No commits/pushes/PRs without separate instruction.

## Assignment 1 — implementation now

Implement **T019**, sole crate crpg-sim. Its focused/native stopping commands
and frozen replay/goldens remain binding. Do not implement history in the same
branch/task. Completion must precede T020 source work.

## Assignment 2 — next critical-path specification

Specify **T020** against D08, sole implementation owner crpg-sim. Inspect every
World mutation route, serialization validation and controller transition. Write
the exact HistoryWorld/HistoryEvent/transaction/read/ack/error/serde/hash contract,
native new-golden authoring path and a new ADR covering the opt-in API and full
wrapper hash. Existing SimEvent/World public shapes and all legacy bytes remain.

Required adversarial review: no mutable-inner-World escape; no duplicate or lost
legacy event; EndTurn/rollover/terminal/despawn/release ordering; capacity failure
has zero committed mutation; cursor exhaustion/load corruption; no net delivery
identity in sim; wrapper hash includes every authoritative field. Record the
server capture/projector as the real consumer. If another crate needs edits,
split an earlier task rather than broadening T020.

Deliverable is an exact executable T020 contract plus filed/reviewed ADR, not
source. ADR publication is outside the earlier tasks-only editing scope and
must be explicitly included when dispatching this documentation assignment.
No further history-mode product choice is needed unless the design conflicts
with a non-negotiable rule. Source starts only after this readiness record.

## Assignment 3 — C0 / host specification

Materialize D01–D03/D10 into new decision/architecture records and an exact
**T022** crpg-server contract. Pin type signatures, all errors/precedence,
authentication binding, serial execution, capacity reservations, capture/ack,
checkpoint parsing limits and new-session-on-restart semantics. Dedicated and
Godot adapters remain separate tasks. Reconcile historical spec wording via
new attribution; do not rewrite ADR history or claim E022's whole ledger done.

T021 exact v2 contract follows the stable T020 event interface. It consumes D09
tag/disclosure choices; no optional reinterpretation of v1.

## Assignment 4 — quinn dependency-preparation research

Resolve D04's technical source/audit evidence, not its already-selected
carry-versus-wait choice. Separate third-party patch ownership from net runtime
implementation. Report exact release/source checksums, minimal patch, all
width-sensitive operations, native regression/zero-loss controls, transitive
dependency/features/license/build/advisory/duplicate-policy results and future
patch maintenance/retirement procedure. No dependency integration or deny
widening without the resulting explicit approval record.

N-QUIC may now be specified under this selected strategy. It is not source-ready
until the source audit, dependency approvals, endpoint contract and native test
plan are complete. A private-fork link or passing simulated transport is not
the patch gate.

## Assignment 5 — independent data specification

Finish **T029a**'s exact constructor/lookup/validate/error/version/limits contract
under D16, sole owner crpg-data. Reuse actual ValueType/DataValue; no authored
schema changes or executable handlers. This specification can proceed while
sim work runs. Implementation waits for its exact readiness record.

## Later assignments

T024 uses D11's segmented transfer and fixed cutover; T025a/b use D12's exact
grace and fresh-session-on-restart rule. T026p/movement prerequisites use D13;
T027a/b/c use D14; T028 uses D15; T029b uses D17 synchronous scope. Each needs
its exact API appendix, existing required ADRs and own native commands before
source. E019 measurement and E021 hygiene can be separately tasked; E023/native
loading is deferred, not an open choice blocking this queue.

Parallel assignment is permissible in separate worktrees with non-overlapping
files. This record does not launch agents. Shared decision/index edits need
coordination; one implementation task still means one crate.

## Agent log

- 2026-09-28 (UTC) · opencode/gpt-6-astra + delegated decision handoff · Turned settled option choices into bounded agent assignments and named the remaining technical readiness artifacts. Kept implementation eligibility truthful rather than labeling policy decisions as completed API or dependency review.
- 2026-09-29 (UTC) · opencode/gpt-6-astra + specified-task handoff · Linked the completed exact contracts and new independent assignments while retaining historical dispatch context. Source readiness and dependency/native gates remain task-specific.
