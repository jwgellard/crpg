# Post-T018 — delegated decision record

Date: 2026-09-28 (UTC).
Status: **Selected under user delegation; documentation/specification authority.**

## Authority and effect

After receiving the D01–D18 register and the distinction between ready and
gated tasks, the user instructed: **“I want you to go ahead with the decisions.”**
The selections below are the agent's exercise of that delegation, not a claim
that the user individually reviewed these exact designs or that an ADR was
already filed. They supersede the corresponding unresolved choices in the
post-T018 briefs and contracts. Prior logs and alternative analyses remain history.

This resolves product/architecture choices and authorizes **specification work**
within them. It does not waive exact-interface contracts, ADR publication,
dependency audits, single-crate boundaries or native verification. No source,
manifest, lock, fixture, golden, root-rule or ADR edit is performed in this
session. Dependency versions/features still require the existing explicit
approval record after audit; no unrestricted dependency permission is granted.

Specification agents may now fill precise types/errors/precedence and test
matrices consistent with these selections without resubmitting the same option
questions. If evidence makes a selection impossible under existing rules,
report the concrete conflict; do not silently change it. Task readiness must
list missing technical artifacts, rather than still saying “needs user choice.”

## D01 — contracts and E017 disposition: selected

- Select E003 B: contracts contains definitions, not cross-crate implementations.
  Transport stays local to net permanently; higher consumers may define narrow
  adapters. No net→contracts edge, contracts edit or graph relaxation.
- Supersede E017 as the **T018-blocking** decision with its executed lane-0
  result and the named T019–T029 evolution queue. Do not close the residual work.
- Spec wording reconciliation is documentation follow-through, not another
  placement decision. BACKLOG row application remains maintainer-owned.

Why: the implemented boundary already works within ALLOWED; moving it would
add graph risk without creating a consumer benefit.

## D02 — shared host and product mapping: selected

- `crpg-server` owns the reusable platform-neutral **library target** and thin
  dedicated binary. T022/T023b/T025b/T027c each have this sole owner.
- Windows embedded single-player uses that same library through a separate
  `crpg-godot` adapter. Client/editor are Godot projects, not new Rust packages.
- Net owns filtered replica storage/application and engine-neutral read queries;
  bridge owns engine-facing query/presentation objects. Neither gets mutable
  authoritative World access.
- One serial authority owner processes admitted commands and simulation steps.
  I/O tasks communicate through bounded queues; no concurrent World mutation.
  Shutdown stops admissions, completes or explicitly refuses admitted work,
  checkpoints at a consistent boundary, then closes transports. Exact API and
  timeout/error shapes belong to the host contract, not sim.
- Headless Linux never imports Godot. Real native dedicated/embedded artifact
  and smoke obligations remain capability-gated.

Why: this uses the existing package/graph and avoids a second authority or new
crate solely for packaging. E022's editor/FFI ledger remains separately scoped.

## D03 — privilege, authentication and client content: selected

- No account service in the first host. Use operator-provisioned, revocable
  invitation credentials bound server-side to principal, campaign/session,
  allowed actors and grants. Credential entropy is host-supplied secure entropy,
  never simulation RNG. Credentials are not printed in receipts/logs.
- For real network connections, verify a server certificate pinned through an
  out-of-band operator invitation, then authenticate the invitation over TLS.
  No trust-on-first-use auto-accept, anonymous player authority or LAN bypass.
  Exact TLS/entropy/verifier library choices remain the dependency audit's job.
- Ordinary player, scoped GM and administrative grants are explicit sets of
  allowed operations/resources, checked by the host immediately before execution.
  Revocation invalidates pending privileges and resume credentials before the
  next command. Role names alone are not an all-access wildcard.
- Privileged traffic has a separate typed control protocol/stream from gameplay;
  it may share the authenticated QUIC connection. No v1 intent flag elevates a
  command. Privileged endpoint implementation is deferred to its own task;
  until then it is unavailable/fail-closed.
- Embedded sessions receive the same host-issued binding, not direct mutation
  access. No general editor privilege is inferred from being in the process.
- Client distribution is a generated allowlisted presentation manifest, not a
  server package with a few files deleted. Server Lua, event handlers, hidden
  gameplay data, native binaries and credentials are excluded. Unknown content
  categories fail the export. Data/export, host distribution and client import
  tests remain separate owning tasks; malicious assets are not “trusted” merely
  because the manifest is allowed.
- T0 native signing/loading is deferred by D07. No fabricated trust-root or
  signed-plugin claim accompanies portable data distribution. Offline operation
  uses explicit local trust configuration, never fallback acceptance.

Why: a small self-hosted authentication model is sufficient without weakening
the authority boundary or implementing an accounts platform.

## D04 — quinn: carry a patch, with an auditable preparation gate

Select **project-maintained dedup-window patch**, rather than waiting upstream.
Evidence checked in this session:

- [Issue #2710](https://github.com/quinn-rs/quinn/issues/2710) is closed
  `not_planned`; its author says they carry their own fix and are not pursuing
  upstream. The report separately identifies #2711 loss-threshold interaction.
- Upstream main at `74b6a84ee31fe30974aa12bc208ba5c9b68cdda2`
  (commit date 2026-09-27) still has `type Window = u128` and
  `WINDOW_SIZE = 1 + size_of::<Window>() * 8` in
  `quinn-proto/src/connection/spaces.rs`.
- That current implementation also contains explicit `u128` masks and `128`
  assumptions in missing-packet queries. This is **not** approval to replace
  one typedef and call the patch complete.

The before-choice barrier to **specifying** N-QUIC is now removed. Before an
executable dependency pin is finalized, an isolated dependency-preparation
assignment must select the exact release, reproduce the window defect, author
or audit the patch, and verify duplicate/reorder/boundary behavior and all
width-dependent missing-packet queries. Initial target: a 2048-bit history
bitset with the separate highest packet retained (2049-packet effective window
if that representation is retained). Measure per-packet-space/connection memory;
do not repeat the issue's cost as a measured project result.

Use a checksum-pinned local vendored release plus explicit reviewed patch as
the proposed integration mechanism. `deny.toml` prohibits unknown git sources;
do not add a fork URL or widen the allowlist. Vendoring must receive a scoped
third-party ownership/dependency record and must not be hidden inside a
crpg-net-only change. If this cannot pass current source/license/build/advisory/
duplicate rules, return the concrete blocker before integration.

Maintain the patch as part of dependency upgrades, with upstream retirement
criteria and native tests. No private fork is trusted by reference and no patch
is claimed tested here. Do not add #2711 or alter congestion/ACK policy without
separate evidence; include it in measurement interpretation. Keep lane-0
reliable ordered; movement datagrams are a distinct versioned lane.

## D05 — NAT: manual forwarding first

Document the dedicated server's chosen UDP port/firewall and manual forwarding
procedure, with two-network external reachability acceptance. No `igd` dependency
or automatic router changes in the first real-server slice. CGNAT/relay/punching
are explicitly deferred. This becomes operational work when a server exists.

## D06 — performance: measure before enforcing ceilings

- Use versioned deterministic headless workloads and separately recorded
  elapsed-time measurements. Initial workload families: minimal-d6 combat,
  srd-lite combat, eight-peer filtered replication and snapshot assembly.
- Each workload pins seed/content, admitted schedule, entity/system counts,
  interest policy and semantic checksum; time is never part of replay state.
- Subsystem-owned workload APIs first, then a thin `crpg-cli` bench wrapper.
  Do not create an upward testkit dependency to obtain a timer.
- Store controlled-runner baselines per target/toolchain/profile/features and
  hardware. Shared runners verify report/workload correctness only. The 8 ms
  target and 20% threshold remain aspirations until measured baseline policy
  and noise envelopes exist; there is no fake green performance gate.
- Plan bulk scene synchronization and animation LOD as separate bridge work;
  drop the unsupported 1000-entity/60-fps/no-LOD acceptance combination. Do not
  substitute an invented lower supported entity count before measuring it.

## D07 — hygiene and native extensions: selected

- E021: mark the embedded spec contract illustrative-only with authoritative
  pointers; backfill a one-page retrospective T004 file from git evidence,
  explicitly distinguishing evidence from unknown historical test results.
- E023: **defer native dynamic loading/ABI and native signing/packaging** beyond
  this queue. Portable data and sandboxed scripting are the extension path for
  these milestones. Do not select IPC, FFI, stable Rust ABI or an unsafe exception
  prematurely. Native-extension design remains a later explicit project feature
  decision, not a blocker on T019–T029.
- Keep only-crpg-godot-unsafe and Godot-free headless policy unchanged.

## D08 — C1: opt-in, sim-owned transactional history

Select richer authoritative history, with **legacy World APIs and bytes intact**.
Use an opt-in sim-owned history wrapper around the existing World/controller,
not unconditional emissions into every legacy World and not a second rules
implementation. Working name `HistoryWorld`; exact signatures/serde/errors must
be pinned in the T020 readiness appendix and a new ADR before code.

- The wrapper owns World privately; callers get read-only World queries and
  typed transactional operations, never `&mut World` or `events_mut` on the
  authority it owns. Operations reuse existing controllers on staged state and
  publish only after history/capacity validation. Rejection leaves both unchanged.
- Keep the existing three-variant `SimEvent` closed API unchanged to avoid
  forcing downstream exhaustive-match edits. Define a separate sim-owned,
  core-closed `HistoryEvent` vocabulary: Spawned, Despawned, Died and the three
  richer variants from T020. No nested SimEvent/rules values or interned handles.
- History producers live at the wrapper's real transition boundaries. It
  consumes the existing queue through its public drain mechanism after each
  successful staged operation and retains the events in the new journal. There
  is no core queue change. New worlds start directly in this mode; importing an
  arbitrary legacy mid-encounter World/history is deferred, not reconstructed.
- Action order: ActionResolved, any Died, then TurnStarted **or** EncounterEnded.
  Legal failed attacks emit ActionResolved too. EndTurn emits only actual turn/
  terminal transitions. Start emits participant spawns then initial TurnStarted.
  Despawn emits Despawned then any resulting turn/terminal transition.
- EncounterEnded means the first transition to terminal (`active = None`), not
  release. Explicit abort/release does not fabricate natural encounter completion.
  Last-participant removal that implicitly releases counts as terminal once.
  Releasing an already-terminal encounter adds no second end event.
- TurnStarted identifies a logical turn start, including round rollover when
  actor identity repeats; do not compare only old/new actor identity. Rejected
  actions and non-ending actions do not invent turns.
- One authoritative journal consumer: `crpg-server` host capture/projector.
  Host fans out to clients and persists its own delivery cursors; slow client
  progress never directly pins the sim journal. Ack only after host capture.
- Initial journal caps: 4096 envelopes AND 1 MiB canonical serialized envelope
  bytes; event string at most 256 UTF-8 bytes. Read pages at most 256 envelopes.
  Sequence starts 1, never wraps; reserved exhaustion must be a typed precommit
  failure. No eviction of unacknowledged history. Ack is monotonic/idempotent;
  stale/future reads and acks get precise typed errors in the exact contract.
- Capacity is checked on staged results before publication, not by rejecting a
  mutated live World. A prototype may stage a full clone; optimize only with
  equivalent transactional tests. Invalid existing gameplay input retains its
  existing error precedence over post-execution capacity errors.
- Persist and hash the **entire wrapper**, including World, pending journal,
  sequence/ack state and mode/version. Add a new full-authority hash operation;
  existing `state_hash(&World)` remains a World hash, not claimed complete for
  the wrapper. No field exclusion. New-mode determinism includes the same
  consumer-ack schedule as well as gameplay inputs.
- New wrapper goldens require their own explicit ADR coverage and independent
  native generation. Old World/B4/replay artifacts remain byte-identical.

The named consumer is accepted for the selected server package. Its later
implementation remains T022, not code allowed in T020. This architectural
selection must be checked against every public mutation route when writing the
exact API; do not ship a bypass or claim the ADR/API already exists.

## D09 — event protocol: explicit v2 alongside frozen v1

Select separate v2 modules/codecs; v1 exports and byte fixtures remain intact.
Explicit version selection only; no automatic tag guessing or silent downgrade.
Pin new event discriminants after old delta tags: 8 ActionResolved, 9 TurnStarted,
10 EncounterEnded in v2. These selections must be encoded into the protocol ADR
with exact field order and bounds before implementation.

Project actor/target as per-client NetId, ability/encounter as authored ULID
only when disclosable; outcome is the symbolic string, damage reported u32,
round u64. No roll/DC/source internals. Suppress an event unless every field
and identity is allowed, while independently replicating permitted state.
Order is the D08 journal order with private gaps removed and client sequence
assigned. Receipts remain epoch/lane/seq/tick/status only. v1 rejects both the
v2 version and, in a separate correctly formed v1 envelope, each unknown tag.

## D10 — host capture and restart: authoritative continuation, fresh sessions

- Owner is crpg-server. Capture returned ActionOutcome, history range and
  permitted post-state synchronously before executing another command. Correlate
  with `(epoch, lane, seq)` in the host, not peer identity in sim.
- Keep approved eight-peer envelope, 256-entry/2-MiB per-peer lane cache,
  128-frame/2-MiB per-peer egress and 16-MiB host egress. Additional shared
  captured-operation journal: 4096 records AND 8 MiB canonical bytes. Reserve
  bounded capacity before committing; no silent loss of an accepted result.
- Checkpoints carry full authoritative wrapper + captured outcomes/progress and
  version/content identity. Persist the relevant historical correlation records,
  but **do not restore live transport bindings, credentials, grace deadlines or
  command epochs as resumable sessions after process restart**.
- Restart issues new epochs and requires fresh authentication/full filtered
  snapshot. Old-epoch commands are refused, never reapplied under a reset ledger.
  Same-process graceful reconnect follows D12 and preserves the original ledger.
- Save/restart equivalence means identical authoritative continuation and
  retained history/outcomes for the same subsequent logical commands. It does
  not mean identical network session ids or successful old-epoch retries.
  Tests compare the explicit new-session path, rather than weakening World/
  history equality. Recovery from an older checkpoint can lose post-checkpoint
  progress; crash-durable exactly-once/WAL guarantees are not claimed.
- First capture slice exposes bounded in-memory checkpoint encode/decode and
  destroy/recreate acceptance. Actual filesystem/compression backend is a
  separate persist-owned task, followed by a server adapter; do not call byte
  round-trip proof a crash-atomic disk backend. Activation of save/load gate 10
  must match the real capability claimed.

## D11 — snapshot transfer: separate versioned control framing

Select a distinct versioned transfer subprotocol, with explicit demultiplexing
at transport/channel selection; it does not reuse v1/v2 combat tags. A snapshot
has a fixed epoch, transfer id, base tick and last included delivery cursor.

- 1 MiB total, 1024 visible entities, 32 chunks, one in flight per peer,
  5 s from accepted begin using injected monotonic time; no retry extension.
- Each chunk carries at most **eight byte segments of at most 4096 bytes**:
  32 KiB payload, comfortably below 64 KiB framed cap with fixed bounded header.
  32 such chunks can represent 1 MiB. Pin canonical segmentation and all counts
  in T024; do not raise any existing byte-field maximum.
- Reassembly payload budget: 1 MiB/peer, 8 MiB aggregate at eight peers.
  Publish by ownership transfer, not an unaccounted full-size staging clone;
  metadata and decoded replica allocations need separately bounded accounting.
- Identical duplicate chunks are idempotent; conflicts/mixed transfer metadata
  fail/discard, missing chunks expire at 5000 ms. Failed assembly never publishes.
- Host captures a permitted snapshot at a cutover cursor. Later deltas buffer
  under existing egress caps and apply only after atomic snapshot publication.
  Overflow aborts/resyncs or closes explicitly; no missing-delta success.
- Permission revocation cancels an in-flight snapshot that would newly disclose
  forbidden facts and retires its generation before another publication.

## D12 — reconnect: 30-second same-process grace

- Retain session epoch, canonical command cache and original receipts for
  **30,000 ms** from disconnect; `< 30,000` may resume, `>= 30,000` expires.
  Attempts do not extend the deadline; host-injected monotonic time, not ticks.
- No AI takeover. Player input is absent, but the ordinary authoritative world
  continues. No implicit EndTurn, resource refresh, despawn or combat reset.
  If existing turn rules wait on that actor, they still wait; this decision
  does not invent a timed-turn/forfeit rule. Expiry changes session retention,
  not gameplay. A later game policy can address abandoned turns separately.
- Resume requires server-issued session-bound proof over authenticated transport,
  fresh grant/revocation checks and fencing of the old connection. Reuse epoch/
  lane/seq/canonical fields unchanged, including observed_tick on retries.
- Full freshly filtered snapshot; old secrets are not replayed from caches.
  Rate/failure budgets are not replenished just by disconnecting/resuming.
- Process restart expires resumption; D10's new-session path applies. Therefore
  no cross-process monotonic-clock rebasing or persisted grace is needed.

## D13 — movement/prediction: ordered scope, unchanged dependency graph

Select authoritative movement/nav before movement wire/host adapter and bridge
prediction. Nav computes paths in crpg-nav; host supplies typed path results to
sim with revision/request identity; sim owns movement stepping and revalidation.
No sim→nav edge. Exact movement rules/path format and numeric limits are a
specification deliverable, not a choice to copy the spike unchecked.

Movement gets a separately versioned lane and processed-seq/server-tick acks;
lane-0 remains window 1. Local-player movement prediction is outside immutable
replica/sim, no combat prediction. Other actors interpolate permitted state.
Do not raise the lane-0 window in this milestone. Quantitative correction/perf
thresholds are measured under D06 before claims.

## D14 — interest: whole-area baseline, explicit private-field grants

Select whole-area membership as the initial entity-interest policy, **not LOS**
or within-area AOI. Sim owns persisted authoritative area membership and its
typed transitions/queries; the server owns viewer-to-area and actor-control
bindings and per-field grants. Net projects those supplied facts.

Presence is visible to an authorized viewer in the same area; health/turn/
action-result fields require separate explicit grants. Default unknown fields
to absent. No stealth/perception system is claimed; introducing stealth later
requires its own authoritative semantics and tests. Revoke actionable mapping
before next admission; hidden removal sends cause-neutral leave. Cross-area
transitions invalidate pending snapshots that would disclose obsolete facts.
Implement missing facts in T027a; do not derive area from NetId or health.

## D15 — legality: read-only bounded enumeration

Select enumeration plus shared pure validation with perform_action. Query takes
World and actor; returns CombatAction-shaped executable options in deterministic
ability-ULID then target-full-EntityId order, followed by EndTurn when legal.
Self-target/targetless conventions follow current combat semantics, not a new
optional-target wire. Entire query leaves World/RNG/events unchanged.

Cap output at 4096 options; excess is an explicit whole-query error, never
truncation. Invalid/absent/dead/out-of-turn actor gets the matching existing
validation error; valid active actor with no affordable ability can still get
EndTurn. No peer entitlement in sim. Host filters what may be disclosed and
execution revalidates current state. Exact return/error types require the sim ADR.

## D16 — declarations: immutable, exact-version compatibility

Select data-owned immutable store with 1024 actions, 32 parameters/action,
1–128 UTF-8 bytes per action/parameter id, no implicit normalization or overwrite.
Reuse current ValueType/DataValue category semantics without coercion; optional
omission is distinct from supplying an incorrectly typed value. Preserve parameter
declaration order; lookup/diagnostics deterministic by symbolic id.

Bundle format version 1 plus explicit bundle id/content revision. Execution
requires exact bundle identity/revision; no automatic semver-compatible handler
substitution. Additive actions require a new revision, changed parameter meaning
requires a new revision and explicit content update/migration. No existing
campaign/document schema changes merely to store trusted startup declarations.
Bound calls to 32 args, depth 32 (root value depth 1), 4096 total DataValue nodes,
4096 UTF-8 bytes per string and 64 KiB canonical serialized call bytes. Enforce
through bounded public construction/validation; input decoders must cap bytes
before parsing. Exact constructor/error/serde surface is specification work.

## D17 — bindings: synchronous first slice; deterministic later interpreter

- T029b is an immutable trusted Rust-handler binding store and synchronous
  dispatch slice. No Lua, Wait, native loading or resumed coroutine in this task.
  Unknown/mismatched ids/signatures fail before any mutation.
- Handler effects are approved typed simulation proposals applied transactionally;
  handlers do not receive unrestricted host/World mutation access. Pure validation
  and deterministic budget checks precede publication; rejected invocation commits
  no RNG/event/resource change. Trusted Rust handlers are not preemptively
  instruction-sandboxed; their bounded work is part of their reviewed contract.
- Later graph/interpreter tasks use world-owned serializable IR continuations in
  a separate earlier sim task. Never store live Lua stack/coroutine in a save.
- E010 policy selected for that later interpreter: 1024 node dispatches and
  100,000 execution units per resumed trigger slice; maximum nested call depth
  32. Charge a graph node once on entry. Each Lua instruction consumes one
  shared execution unit and its per-call Lua counter; a per-call ceiling of
  100,000 never grants extra shared budget. Nested graphs share counters.
- Abort at the first exceeded limit, propagate to the trigger, discard staged
  effects since the last committed yield/trigger boundary. Wait commits a valid
  slice and serializes continuation; resume receives a new slice budget. This
  bounds a slice, not aggregate triggers per server tick, which the interpreter
  task must separately schedule and cap.
- Lua per-state ceiling 2 MiB initially; instruction hooks check at each
  instruction for exact boundary tests. No wall-clock abort in rules. These
  are initial policy values, not measured throughput claims. Tune only in a
  separately recorded policy change with semantic/budget tests.
- Carry ADR-0005's restricted libraries, deterministic replacements and explicit
  stripping of load/loadfile/dofile/require/loadstring. No Lua/runtime dependency
  is selected or approved by this budget decision.

## D18 — remaining roadmap: selected ordering, explicit deferrals

Adopt POST-T018's remaining ledger owners/phases as specification order. No
multiplayer host migration in these milestones; retain Windows embedded hosting.
Headless editor command API precedes UI; optional CLI apply follows that API.
AI consumes T028, never a net legality copy. Keep E007 append-only ADR
supersession and E013 dependency-arrow clarification as documentation work.
Scaffolding/runner and measured workload tasks stay separately scoped tooling.

Defer PF2e/content expansion, native extensions and distribution/signing product
design to their recorded phases. Do not approve the independent D0 filesystem
proposal by association. These deferrals are deliberate scope decisions, not
an endlessly pending approval needed by T019–T029.

## Dispatch and remaining technical gates

See [POST-T018-HANDOFF.md](POST-T018-HANDOFF.md). The D01–D18 option calls are
settled as above. Remaining exact APIs/ADRs, source/dependency audit and measured
thresholds are engineering deliverables; their absence must not be disguised
as ready implementation. Only T019 is source-ready in this checkout today.

## Agent log

- 2026-09-28 (UTC) · opencode/gpt-6-astra + delegated post-T018 decisions · Exercised the user's explicit delegation to settle the recorded choices, using live quinn evidence and the existing graph/compatibility rules. Kept selections distinct from unfiled ADRs, unaudited dependencies and unimplemented interfaces so agents can advance specifications without inventing approval provenance.
