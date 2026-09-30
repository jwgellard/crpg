# Post-T018 specification queue

**2026-09-29 specification update:** [current readiness index](SPECIFICATION-READINESS.md)
supersedes the missing-exact-interface status below for T020, T021, T027a, T028,
T029a and T029b. Their exact appendices and ADRs 0017–0020 are filed; T030–T032
add independent preparation/tooling specifications. Historical draft BACKLOG
rows and verification records below are not current implementation completion.

## Status and authority

Documentation-only planning, updated 2026-09-28 under the user's delegation:
**[D01–D18 choices are now selected](POST-T018-DECISIONS.md)**. Their previous
option questions below are historical; the decision record governs. See
[agent assignments](POST-T018-HANDOFF.md) for what can proceed now.

T019 remains source-ready. Later tasks need exact-interface appendices, filed
ADRs where required, technical dependency evidence and prerequisite code; settling
a product choice does not supply those artifacts. No dependency integration,
merge or release approval is granted here. No placeholder tests discharge gates.

Primary input: [E017 Appendices A–B](E017-t018-interface-debt.md), especially
B2–B7; frozen reference: [T018a](T018a.md), [T018b](T018b.md),
[T018c](T018c.md), including completion records. [Common contract rules](POST-T018-RULES.md)
are incorporated into every child. Number allocation here is a proposed
maintainer index; BACKLOG is not edited by this session.

## Bookkeeping verification — report and maintainer drafts only

- GitHub PR #16 is closed/merged to `master`, `merged_at =
  2026-09-28T08:21:31Z`, merge commit
  `bc54896a6d3f9299751709deeb2f7949c75695d1`. Its implementation commit is
  `2fc54b77c813408152fba80b568136acc27a0b01`.
- Verified with `git show -s --format=fuller 2fc54b7` and
  `gh api "repos/jwgellard/crpg/commits/2fc54b7/pulls"`; remote master API
  returned the merge commit. Local checkout is `task/T018-net-protocol`;
  local master does not contain the implementation commit. Do not mistake
  that stale local ref for an unmerged PR. No refs were updated.
- BACKLOG Phase 4 still says `T018 | open`. T018 child completion headings
  retain their historical working-branch wording; preserve those records.
- S001 §6's postcard item is satisfied for the **implemented v1 vocabulary**:
  `codec.rs::decode_intent` caps 4 KiB, `decode_delta` caps 64 KiB before
  decoding; delta count is checked at 256 before push allocation;
  `take_ability` rejects strings over 256 bytes. Encode checks the same
  vocabulary bounds. The owned String is decoded **before** its 256-byte
  check, with allocation bounded by the already-checked 4 KiB input and
  postcard's available-input check. This is not a claim of pre-allocation
  enforcement of the 256-byte string ceiling. No arbitrary byte bag exists.
- `sim.rs::send_to` checks peer, then rate, fabric MTU (64 KiB), then checked
  per-peer/host frame AND byte accounting before staging. It does not decode
  or apply the 4 KiB intent-specific cap. `PeerState` is driver-owned;
  production admission is not implemented. Fault retry/dedup is in drivers,
  not automatic reliable delivery in the fabric.
- Evidence: T018a's 28 codec tests; T018b's final 25 transport tests;
  T018c's final 22 conformance tests and dual-native completion records.
  This session inspected source/history; it did not rerun these Rust suites.
- Preserve the recorded deviation: postcard is
  `{ version = "1", default-features = false }`; enabling defaults would
  restore the rejected atomic-polyfill chain. `deny.toml` stays unchanged.
- The 1-in-flight / 32-chunk / 5 s snapshot model is test-local, not a
  production snapshot transfer. S001 closure does not close that obligation.

Draft replacement BACKLOG row (maintainer applies):

```markdown
| T018 | done | 2026-09-28 | `crpg-net` lane-0 combat protocol, bounded postcard codec, simulated transport and conformance (T018a/b/c; PR #16, implementation `2fc54b7`, merge `bc54896`; 75 net tests per native target). Phase 4 host/QUIC/movement/reconnect remains open; follow-on queue: POST-T018.md. |
```

Draft replacement for the deferred postcard sentence in BACKLOG and closure
annotation beside S001 §6 (maintainer applies, with attribution):

> T018 `crpg-net` postcard caps are complete for v1 in PR #16 (`2fc54b7`):
> 4 KiB intent / 64 KiB delta frames, 256 delta operations and 256-byte
> strings, bounded decode/encode and staged ingress/egress. String allocation
> is frame-bounded before its field-length rejection. Future versioned
> vocabularies and snapshot reassembly inherit separate bound-before-allocation
> obligations. Data loader caps and persist compressed/decompressed save caps
> remain assigned to their owners.

## Ordered contracts and dependency chain

Rows are specification/handoff order, not permission to skip prerequisite work.
Each owner cell names exactly one implementation crate, or an explicit owner
gate. `blocked` includes design approval, not just a missing code dependency.

| Order | Contract | Sole owner | Phase | Prerequisites / status |
|---|---|---|---|---|
| 1 | [T019 same-pool spend](T019.md) | crpg-sim | 4 repair | Ready specification; existing public API |
| 2 | [T020 C1-sim](T020.md) | crpg-sim | 4 | T019; exact D08 wrapper API/ADR and new-golden contract |
| 3 | [T021 N-EVENTS-v2](T021.md) | crpg-net | 4 | T020; exact D09 projection/version contract |
| 4 | [T022 C4-consume](T022.md) | crpg-server | 4 | T020–21; exact D02/D03/D10 API and audit |
| 5a | [T023 N-QUIC specification](T023.md) | crpg-net | 4 | D04 carry-patch selected; exact source/patch/dependency evidence still required |
| 5b | [T023b host QUIC adapter](T023b.md) | crpg-server | 4 | T022 + executable, completed T023 |
| 6 | [T024 snapshot transfer](T024.md) | crpg-net | 4 | T021, T023b handoff; exact D11 transfer layout |
| 7a | [T025a N-RECONNECT mechanisms](T025a.md) | crpg-net | 4 | T024; exact D12 auth/resync wire |
| 7b | [T025b N-RECONNECT host](T025b.md) | crpg-server | 4 | T022, T023b, T025a; exact D12 API |
| 8 prerequisite | [T026p N-LANES-policy](T026p.md) | crpg-net | 4 | D13 authoritative movement/nav, T023; separate host adapter follows |
| 8 | [T026 N-PREDICT](T026.md) | crpg-godot | 5 | D13 movement/nav + T026p and host adapter; E012 |
| 9a | [T027a N-INTEREST facts](T027a.md) | crpg-sim | 4 | D14 authoritative area/perception facts |
| 9b | [T027b N-INTEREST projection](T027b.md) | crpg-net | 4 | T027a (or approved existing-facts proof), T021/T024 |
| 9c | [T027c N-INTEREST context](T027c.md) | crpg-server | 4 | T022, T027b, exact D03/D14 context API |
| 10 | [T028 N-LEGALITY](T028.md) | crpg-sim | before 9, earlier for UI | T019; D15 exact query/ordering/bounds |
| 11a | [T029a IR-SIGNATURES](T029a.md) | crpg-data | 8 or earlier MVP consumer | D16 declaration compatibility and limits |
| 11b | [T029b IR-BINDINGS](T029b.md) | crpg-script | 8 or earlier MVP consumer | T029a, E010, D17 continuation/budgets |

Critical chain: T019 → T020 → T021 → T022 → T023 → T023b → T024 →
T025a → T025b. D04 is decided; patch/dependency evidence still gates quinn code.
T026 is listed next as B7 requests; its phase-5 acceptance waits for the
phase-4 interest/movement facts. T027/T028/IR work is separately gated, not
an instruction to build client prediction before authority exists.

Draft additional BACKLOG rows (maintainer applies):

```markdown
| T019 | next | — | `crpg-sim` same-pool affordability repair; public regression and unchanged replay/goldens; POST-T018.md |
| T020 | blocked | — | C1-sim opt-in history wrapper selected; exact D08 API/ADR and T019 required |
| T021 | blocked | — | `crpg-net` N-EVENTS-v2 selected; exact D09 contract + T020; v1 compatibility |
| T022 | blocked | — | `crpg-server` C4 capture/checkpoint slice; D02/D03/D10 selected, exact API + T020/T021 required |
| T023 | blocked | — | `crpg-net` N-QUIC; carry-patch selected in D04, source/patch/dependency evidence and exact contract required |
| T023b | blocked | — | `crpg-server` QUIC adapter; exact API, T022 + completed T023 |
| T024 | blocked | — | `crpg-net` versioned bounded snapshot transfer; T021/T023b + D11 |
| T025a | blocked | — | `crpg-net` reconnect/resync mechanisms; T024 + D12 |
| T025b | blocked | — | `crpg-server` authenticated 30-second same-process grace; exact API + T025a |
| T026p | blocked | — | `crpg-net` N-LANES-policy/movement acknowledgments; authoritative movement + D13 |
| T026 | blocked | — | `crpg-godot` external movement prediction; T026p + host movement adapter + E012 |
| T027a | blocked | — | `crpg-sim` missing interest facts or approved sufficiency proof; D14 |
| T027b | blocked | — | `crpg-net` production interest projection; T027a/T021/T024 + D14 |
| T027c | blocked | — | `crpg-server` authoritative viewer context; exact D03/D14 API + T027b |
| T028 | blocked | — | `crpg-sim` shared legality query; T019 + D15; before AI |
| T029a | blocked | — | `crpg-data` versioned IR signature declarations; D16 |
| T029b | blocked | — | `crpg-script` trusted IR executable bindings; T029a + E010/D17 |
```

Retarget E003/E017's obsolete T018 blocker cells to `post-T018 reconciliation`;
record their delegated decision as selected, with ADR/spec reconciliation still
pending. Do not count a decision or new contract as merged implementation or
change throughput here.

## Single decision register — D01–D18 selected under delegation

The following table preserves the questions/options submitted before the user
delegated their resolution. **All selections and scope deferrals are now in
[POST-T018-DECISIONS.md](POST-T018-DECISIONS.md), keyed by these same ids.**
No D01–D18 product-option call remains awaiting the user for this queue.
Exact APIs/ADRs, dependency approval after audit and implementation verification
are still required. The approved 200-tick policy/window-1/cache defaults remain.

| ID | Human call and recommendation | Genuine alternative / output required |
|---|---|---|
| D01 | [C0](POST-T018-C0.md): retain net-local Transport, reconcile E003 with definitions-only contracts; supersede E017 by this queue | Reopen graph/contracts placement explicitly; or close E017 after all residuals have accepted owners. No contracts edits implied |
| D02 | C0: choose reusable `crpg-server` library + thin dedicated binary; Godot projects over crpg-godot | Another existing host crate after graph review. Must name package, API, ownership/threading, shutdown and replica query home |
| D03 | C0: server-issued scoped capabilities and separate privileged protocol, fail closed | Single authenticated multiplexed channel with equivalent enforcement. Decide credentials, renewal/revocation, offline trust roots and package stripping |
| D04 | [quinn brief](POST-T018-QUINN.md): evaluate carrying a pinned, reviewed dedup-window patch | Wait for independently verified upstream fix; exact source/version/features/deps and maintenance owner required before T023 can be written |
| D05 | [NAT brief](POST-T018-NAT.md): documented manual forwarding for first real server | Approve IGD dependency and mapping lifecycle; neither promises CGNAT traversal |
| D06 | [perf brief](POST-T018-PERF.md): controlled-runner per-target measurements; CLI wrapper after harness | Aspirational targets until measured; decide workloads, baseline policy, renderer ceiling vs LOD |
| D07 | [hygiene/native brief](POST-T018-HYGIENE-NATIVE.md): illustrative-only embedded examples, retrospective T004 record; defer native ABI until governance approved | T004 index annotation; explicit loader boundary and policy amendment or process-isolated extensions |
| D08 | T020: approve new history mode/producer/consumer/queue API and compatibility ADR | Host-only outcome history with no new sim events; do not auto-enable hashed emissions in legacy paths |
| D09 | T021: approve exact v2 event tags, fields, ordering/disclosure and version selection | Narrower event subset; no renumbering or silent v1 upgrade |
| D10 | T022: approve host admission/lifecycle/checkpoint API and retention caps | Process-restart session expiry vs persisted dedup continuation; real save backend requires persist-only prerequisite |
| D11 | T024: approve versioned transfer framing, chunk payload packing and snapshot/delta cutover | Independent transfer subprotocol vs later main version. 32 × 4 KiB cannot carry a 1 MiB snapshot; resolve without silently widening a hard cap |
| D12 | T025a/b: recommend 30 s grace, retain epoch/cache, full filtered resync; recommend frozen player input while authoritative turns/resources continue | AI takeover; choose cross-process policy, authentication proof, old-connection fencing, clock rebasing and expiry-boundary semantics |
| D13 | T026: authoritative movement/nav then lane-1 ack semantics then external prediction buffer | Defer movement/prediction; exact movement integration and bounds need separate nav/sim/net contracts; no invented MoveTo success |
| D14 | T027a/b/c: whole-area interest + approved perception facts, no within-area AOI | Whole-area-only disclosure policy if approved; decide fact ownership, viewer/field mask and revoke/cutover semantics |
| D15 | T028: read-only combat action enumeration sharing sim validation | On-demand legality check only; approve exact ActionOption, target enumeration/order, limit and diagnostics |
| D16 | T029a: versioned immutable declaration store with proposed A4 limits | Different explicit caps/version policy; decide ValueType checking and compatibility/migration rules |
| D17 | T029b: trusted immutable bindings, deterministic nested budgets, world-owned continuations | Synchronous no-wait first slice; decide exact context/errors/rollback, E010 numbers/strip list; sim continuation prerequisite if needed |
| D18 | Remaining ledger below: approve per-feature scope/owner before executable contracts | Defer feature explicitly; do not assign source signatures from roadmap nouns |

The recommendation-only briefs below are historical option analysis. Their
delegated-decision addenda supersede that status; they do not authorize source
from an unfinished exact-interface contract.

## Remaining backlog coverage / refused speculative contracts

The entire *named E017 evolution queue* is above, including snapshot transfer
and N-LANES-policy (D13). Broader BACKLOG/roadmap obligations remain visible
here; they are not silently represented as executable by the transport queue.
Their source records do not yet provide exact interfaces. D18 adopts the owner/
phase ordering and explicit deferrals; specification agents supply exact
one-crate contracts within those decisions:

| Obligation | Ordered sole owners and phase | Blocking specification / compatibility |
|---|---|---|
| N-LANES-policy and authoritative movement | crpg-nav navigation/bake → crpg-sim movement → crpg-net movement/acks → chosen host adapter, 4 | D04/D13; sim cannot import nav under ALLOWED: caller-supplied navigation results or separately approved graph decision. New lane needs version; v1 combat unchanged; no highest-seq-wins on lane 0 |
| Save backend, S001 decompression caps | crpg-persist → chosen host checkpoint adapter, 4/7 | D10; input AND decompressed limits, versioned save envelope, native save/load gate 10; persist cannot import net, so host owns session ledger |
| Product adapters and smoke | chosen host Windows/Linux dedicated → crpg-godot embedded adapter, 4/5 | D02; real artifacts and gate 12/13 smoke; one authority, Godot-free Linux |
| Replica query, proxy spawning, bulk scene sync, HUD/log | crpg-net replica mechanism if needed → crpg-godot presentation, 5 | E012/E022, T021/T024/T027; bulk snapshot API and property disclosure before engine objects |
| EditCommand/undo/validation, tree/forms/problems panel | crpg-edit headless command API → crpg-cli apply wrapper if approved → crpg-godot editor UI, 6 | E022 exact edits/identity/conflicts; E018 before live privileged edits; data migrations remain data-only |
| Graph execution/Lua, dialogue/quest/waits | crpg-data schema changes → crpg-sim persisted continuation state → crpg-script interpreter → crpg-edit graph tools, 8 | E010 + D17; no OS logic in sim, no wall-clock rule aborts; declaration is not executable binding |
| AI consumption/utility/encounter dry-run | crpg-ai → crpg-cli wrapper, 9 | T028 public query; scoring/budgets and movement facts first; no duplicated net legality |
| Multiplayer hardening/admin/party | chosen host → crpg-net new wire if needed → crpg-godot test launcher, 10 | E018/E022; decide host migration explicitly (recommend none); preserve embedded single-player |
| T0 extension/signing/packaging | owner gate E023, 12 | D07; no ABI/unsafe/loader approval from this queue |
| Perf harness/bench/bridge LOD | measured subsystem owner → crpg-cli wrapper; crpg-godot LOD separately, 13/earlier measurement | D06; ALLOWED is not a blanket testkit edge approval |
| E007 ADR wording; E013 diagram meaning | documentation-only decisions | Recommend append-only supersession and dependency-arrow clarification; no source authorization |
| Preflight scripts, ADR template, runner | tooling-only tasks after D18 scope | Preserve trusted lint gates; no fork PR self-hosting; crate docs due on opening |
| Data loader caps and filesystem proposal | crpg-data, then crpg-cli children when approved | Independent D0 work; pre-existing untracked `docs/reviews/D0_LIMITS_FILESYSTEM_PROPOSED.md` is not edited or approved here |
| MVP/content/authoring/distribution | scope decision then content/tooling tasks, 7/11/12 | PF2e and campaign content are not a crate-shaped networking implementation; no invented license/content approvals |

Remaining readiness boundary: D04 now permits N-QUIC specification, but not
unaudited dependency integration. Host owner is crpg-server; exact API work
remains. D08/D17 select history/budget policy without fabricating an implemented
API. Native ABI/loading is deferred. No hash workaround or roadmap-wide
placeholder success tests are authorized.

## Agent log

- 2026-09-28 (UTC) · opencode/gpt-6-astra + post-T018 queue and bookkeeping · Verified PR #16 and inspected the codec/fabric before drafting maintainer-only status closures. Ordered single-crate follow-ons and collected all unresolved decisions without treating roadmap sketches or historical completion headings as implementation authority.
- 2026-09-28 (UTC) · opencode/gpt-6-astra + queue completeness review · Added the explicit B3 N-LANES-policy prerequisite and literal maintainer BACKLOG row drafts. Kept movement authority and its host adapter behind their own owners instead of assigning them to prediction work.

## Verification addendum — documentation session

Supersedes only the first-pass statement above that suites had not been rerun:
after writing the contracts, the existing tree passed these checks on native
Windows/MSVC and genuine WSL Ubuntu 24.04 Linux/GNU, Rust 1.98.0
(`88d9e12ae178fab0fb5cc050a94da85685d449ea`), normal test profile/default features:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
```

Linux used `python3` for the same lint commands. Both full workspace test runs
exited 0, including net codec 28, transport 25 and conformance 22 and the native
combat_basic/combat_srd/replay_basic comparisons. Lint self-tests: 70 OK on each
target. `cargo deny check` passed on Windows (advisories/bans/licenses/sources).
Formatting used check-only to preserve this task's documentation-only scope.
These results validate the existing implementation, **not** the future test
targets named in T019–T029, which do not exist yet.

Windows `git diff --exit-code` confirmed no tracked changes, including all
source/manifests/lockfile/fixtures/goldens; new files are only this session's
24 task documents. The pre-existing untracked D0 proposal remains untouched.
No-index whitespace checks were run over the new docs (exit 1 from comparison
against /dev/null is an ordinary nonempty diff, not a whitespace error).
No commits/pushes/PRs or BACKLOG status application occurred.

- 2026-09-28 (UTC) · opencode/gpt-6-astra + documentation-session verification · Recorded fresh native Windows and Linux workspace/lint checks separately from future task acceptance. Confirmed that verification produced no tracked implementation or golden changes and preserved the maintainer-only bookkeeping boundary.

- 2026-09-28 (UTC) · opencode/gpt-6-astra + delegated decision alignment · Linked the settled D01–D18 record, assigned host children to crpg-server and removed the resolved quinn before-choice specification hold. Preserved historical alternatives and distinguished remaining exact-contract/audit work from pending user option decisions.
- 2026-09-29 (UTC) · opencode/gpt-6-astra + exact-contract frontier · Linked the nine-specification readiness pass without rewriting historical drafts or verification records. Future source and native acceptance remain distinct from the newly completed documentation.
