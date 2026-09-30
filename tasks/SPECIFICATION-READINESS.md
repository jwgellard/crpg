# Specification readiness — 2026-09-29

## Scope and authority

The user requested specifications for as many tasks as can be specified without
being blocked by other tasks. This pass uses the recorded D01–D18 delegation,
actual source interfaces and the existing single-crate/compatibility rules.
It writes specifications, not implementations. Historical logs remain evidence
of their dates; current readiness below supersedes their “exact API missing”
statements only where a new appendix is linked.

An implementation dependency is not automatically a specification dependency:
once its exact public contract is pinned, dependent interfaces can be specified
against it, while source acceptance still requires the real predecessor. New
ADRs record selected-under-delegation designs, not invented independent review
or completed native acceptance. New ADRs are filed for review; the existing
ADR/API review gate still precedes source. No new dependency is approved by this index.

## Nine completed specifications

| Task | Sole implementation owner | Contract now pinned | Remaining implementation gates |
|---|---|---|---|
| [T020](T020.md#specification-revision--2026-09-29) | crpg-sim | HistoryWorld, six events, transaction/read/ack/errors, strict journal load, full hash and new native golden authoring; [ADR-0017](../docs/adr/0017-opt-in-authoritative-history.md) | T019 regression passing on implementation base; new artifacts independently generated/reviewed before completion |
| [T021](T021.md#specification-revision--2026-09-29) | crpg-net | Explicit v2 codecs/tags and bounded per-field disclosure projection; [ADR-0019](../docs/adr/0019-event-protocol-v2.md) | Real T020 history for integration acceptance; no new dependency |
| [T027a](T027a.md#specification-revision--2026-09-29) | crpg-sim | Persisted one-area World identity, bound encounter validation and atomic supported-shape transfer; [ADR-0020](../docs/adr/0020-authoritative-area-membership.md) | T020 for HistoryWorld integration; same-crate work serialized |
| [T028](T028.md#specification-revision--2026-09-29) | crpg-sim | Shared pure validation, exact option order/4096 bound/error precedence; [ADR-0018](../docs/adr/0018-read-only-combat-legality.md) | T019 regression passing on implementation base |
| [T029a](T029a.md#specification-revision--2026-09-29) | crpg-data | Immutable declarations, derived bundle identity, strict readers/writers and exact call budgets/errors | None beyond ordinary implementation gates |
| [T029b](T029b.md#specification-revision--2026-09-29) | crpg-script | Exact trusted binding/startup/context/proposal/rollback API, synchronous legacy-World scope | T029a + T028; explicit proposed dependency-edge approval |
| [T030](T030.md) | isolated dependency preparation | Exact source/patch/audit/native evidence deliverables and reproduction controls | Can begin now; any audit failure remains a T023 blocker |
| [T031](T031.md) | tooling | Native preflight CLI, command order, failure semantics and subprocess tests | Can begin now |
| [T032](T032.md) | documentation | Selected authority wording, historical T004 evidence, ADR template | Can begin now |

T019 was already specified and implemented/verified according to its existing
completion record; this pass neither reimplements it nor changes its merge
status. T020/T028 can use that current implementation after their stated
regression gate; on an older base they must first acquire T019. Task scheduling
must preserve one implementation crate per task and avoid simultaneous sim edits.

## Remaining frontier — concrete missing inputs

| Task / obligation | Why an exact executable contract is not completed in this pass | Next specification input |
|---|---|---|
| T022 host | Typed history/projection inputs are now supplied, but authentication credential/verifier/entropy dependency evidence, host admission/checkpoint limits and exact lifecycle API are not yet pinned | Host security/dependency preparation and C0 exact interface work; no arbitrary library selection |
| T023 QUIC | Selected patch strategy is not a verified release/patch/runtime/TLS dependency graph | T030 dossier and explicit dependency approval, then endpoint/stream contract |
| T023b host QUIC | Depends on concrete host and endpoint APIs | T022 + T023 exact contracts |
| T024 snapshots | Transfer segmentation is selected, but production replica snapshot encoding/storage and host cutover ownership are not pinned | Host/replica and real-channel contracts; retain 1 MiB/32×8×4096 bounds |
| T025a/b reconnect | Requires concrete transfer/authentication/session ledger and connection-fencing interfaces | T022/T023b/T024 |
| T026p / T026 movement | No authoritative navigation/path request or movement stepping contract exists | Separate nav then sim contracts; no sim→nav dependency; lane semantics follow |
| T027b/c interest | Sim area facts are now specified, but production replica membership/field-clear/snapshot generation and host viewer-binding APIs are not | T022/T024 and replica mechanism contract; consume area+full-EntityId identity |
| T029b history integration / later interpreter | Synchronous legacy-World dispatch is specified; no HistoryWorld batch API, persistent continuations or Lua dependency exists | Separate sim batch/continuation and interpreter tasks; D17 budgets are selected, not still a user-option question |
| persistence / disk adapter | Full host checkpoint representation and actual backend/compression dependency audit absent | T022 checkpoint contract, then persist-only backend and server adapter |
| performance / CLI bench | Initial combat workload families selected; exact measured workload/report API and controlled-runner baseline policy absent; replication workloads also need real mechanisms | Subsystem-specific workload specification, dependency record as needed, measurement before thresholds |
| editor / bridge / AI / content | Live editor command conflicts, replica query/presentation APIs and AI scoring are not specified; AI legality now has T028's contract | Feature-specific owner contracts after the now-pinned prerequisites; no roadmap-noun placeholders |
| native loading / signing / PF2e expansion | Explicitly deferred by D07/D18 | Later product-scope decision, not a hidden blocker on the nine contracts above |
| D0 loader/filesystem proposal | Independent pre-existing proposal has not been approved by association | Its own decision/review; this pass leaves it untouched |

These rows are engineering inputs, not a reopening of settled D01–D18 choices.
Do not turn them into green tests by substituting supplied fixture permissions,
byte round trips for disk persistence, or a simulated transport for real QUIC.
The new preparation tasks create bounded work where no product code is ready.

## Verification record

Verification of this documentation pass:

- Native Windows/MSVC and genuine WSL Ubuntu 24.04 Linux/GNU both used Rust
  1.98.0 (`88d9e12ae178fab0fb5cc050a94da85685d449ea`). Normal test profile and
  default features, existing target-scoped golden comparisons unchanged.
- `cargo fmt --all` on Windows made no source changes; format check passed on
  both targets. `cargo clippy --workspace --all-targets --locked -- -D warnings`
  and `cargo test --workspace --locked` passed on both native targets, including
  T019's 11-test combat_costs suite and existing replay goldens.
- `python tools/lint/deps.py`, `python tools/lint/determinism.py` and
  `python -m unittest discover -s tools/lint -p "test_*.py"` passed on Windows;
  the same commands with python3 passed on Linux (70 lint self-tests each).
- `cargo deny check` passed on Windows with cargo-deny 0.20.2. The Linux
  invocation could not run because cargo-deny is not installed there; this is
  an explicit verification limitation, not a claimed native pass. The initial
  WSL rustc lookup also required adding the existing `$HOME/.cargo/bin` to the
  process PATH; subsequent native Rust gates passed without toolchain changes.
- `git diff --check` passed. Explicit no-index whitespace checks cover the
  edited/new untracked task and ADR files; ordinary git diff alone omits them.
  `git diff --exit-code -- crates Cargo.toml Cargo.lock rust-toolchain.toml
  campaigns rulesets schemas tools .github AGENTS.md` confirmed no tracked
  implementation, dependency, fixture, golden or rule changes.

Future test names in task appendices are required implementation acceptance,
not claims those suites already exist or passed. No source, manifest, fixture,
golden, root/crate rule or toolchain edit is part of this pass.

## Agent log

- 2026-09-29 (UTC) · opencode/gpt-6-astra + specification-readiness audit · Distinguished specification dependencies from implementation prerequisites, indexed nine complete contracts and named concrete inputs for the remaining queue. Preserved the user's existing working files and historical completion/approval provenance.
- 2026-09-29 (UTC) · opencode/gpt-6-astra + documentation verification · Recorded fresh native workspace/clippy/lint passes and Windows dependency audit, with Linux cargo-deny absence explicitly retained as a limitation. Confirmed source/fixtures remain untouched and kept the new specifications' future acceptance separate from existing-code verification.
