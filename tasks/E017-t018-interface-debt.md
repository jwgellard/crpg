## Task
Specify the shapes T018 must encode before the protocol task is written.
Human-decision task (spec edit + task scoping).

## Why this is deferred

T018 owns `ClientIntent`/`DeltaOp` enums, the `postcard` codec with version
byte, and conformance/desync tests (`docs/CRPG_ENGINE_SPEC.md:1705-1711`,
`tasks/BACKLOG.md:86`), but every shape it encodes is a sketch or missing:

- `SimEvent` variants: one example (`Damage {target,amount,type,outcome,
  source}`, spec `:750`) and a `SimEvent(SimEvent)` wrapper (`:669`) — no
  variant list, versioning, or interaction with per-client filtering (`:695`).
- `ClientIntent` fields (`:680-686`): position/tick/ability/target/net-id
  types missing; spec uses `tick` while the ADR-0004 spike used per-intent
  `seq` (`docs/adr/0004-quic-movement-spike.md:19-24`); no validation
  taxonomy or idempotency rule (`tasks/T002.md:61` already flags the match).
- Action registry: IR `Action(action_id,args)` lives in `crpg-data` per
  ADR-0008, which cannot depend on rules/sim (`tools/lint/deps.py:42-44`) —
  the indirection from IR reference to Rust implementation is undescribed,
  and no owner holds the signature store (`:532,903,925`, spec `:161`).
- Intent rate limits + payload caps (`:689`) have no values, scope, or
  rejection codes, though the malicious-client suite (`:1176`) must test them.
- `legal_actions -> Vec<ActionOption>` (`:620`), `visible_to`
  (`:695`), `InterestSet` (`:694`), `ScriptContinuation` (`:527`) are named
  with undefined types; interest/filtering/prediction/reconnect beyond the
  in-memory transport have no follow-on task (T018 covers codec/in-memory/
  desync only).

## Decision to make
- Fix the `seq`-vs-`tick` discrepancy; type intents, deltas, and the action
  indirection; set caps and rejection codes; scope T018 vs named follow-ons
  (interest impl, prediction, grace-window behavior).

## Deliverable
- Edits to `docs/CRPG_ENGINE_SPEC.md` (§§5.2, 6.2, 7.3–7.6) with dated notes
  + BACKLOG T018 scope + follow-on rows. No source changes.

## Constraints
- Doc-only. Human sign-off; T018 is unwritable against sketches.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark · Filed as part of the spec-gap triage: everything T018 encodes is currently a sketch.

## Appendix A — PROPOSED decision brief (2026-09-28, non-binding)

**E017 remains OPEN; human approval is required.** All recommendations, candidate
shapes, limits, codes and scope choices below are PROPOSED. This appendix is not
the T018 executable contract, does not approve E003, and authorizes no source,
dependency, ADR, spec or backlog change. Part 2 waits for explicit instruction.
The independent [D0 draft](../docs/reviews/D0_LIMITS_FILESYSTEM_PROPOSED.md) can
be reviewed separately; neither decision depends on the other.

### A1. Reviewed baseline and evidence

Initial tree: clean `master` at `a302320` (T016/T017 bookkeeping merge), containing
combat merge `4686a73`. `git ls-remote origin refs/heads/master` also returned
`a3023202d758123a9203c2843d348a1511d8d4e1`; this is not a stale checkout.
**Merged/implemented** below means inspected source at that revision, not a new
test result. **Specified** means an existing requirement, sometimes still a
sketch. **Verified in this planning session** means source/history inspection
only; no networking behavior or new runtime gate has been verified.

| Source (repository-relative) | Finding that constrains this decision |
|---|---|
| `crates/crpg-net/src/lib.rs`, `Cargo.toml` | Documentation-only scaffold; empty dependency section. Protocol, codec, transport and filtering are not implemented. Even spec-named postcard still needs explicit dependency approval. |
| `crates/crpg-sim/src/event.rs` | Exactly `Spawned { entity: EntityId }`, `Despawned { entity: EntityId }`, `Died { entity: EntityId }`. No action, damage, turn or effect event. |
| `crates/crpg-sim/src/world.rs:541–719` | Public spawn/despawn, liveness/id queries, transforms/timeline/events access, read-only combatants/combat/interners. Combat mutation access is crate-private. No `visible_to`, `legal_actions`, area/ownership/session registry or filtered replica API. |
| `crates/crpg-sim/src/combat.rs::CombatAction, ActionOutcome, perform_action` | `UseAbility { actor: EntityId, ability: Ulid, target: EntityId }` and `EndTurn { actor: EntityId }`; returns `Result<Option<ActionOutcome>, CombatError>`. Ability returns actor/target/ability, roll, margin, outcome, damage and death flag; EndTurn returns `None`. These results are not persisted events. |
| `crates/crpg-core/src/event.rs`; `crates/crpg-sim/src/hash.rs`, `tick.rs` | Envelope has `tick: Tick`, `seq: u64`, payload. Queue drains in `(tick, seq)` order, has no read iterator; clone/drain is available. Queue bytes participate in state hash. Current tick only advances the counter and runs the no-effect timeline traversal; comments about draining do not implement a drain. |
| `crates/crpg-data/src/ir.rs:172–202` | `ActionCall { action_id: String, args: BTreeMap<String, DataValue> }`, `ActionSignature { action_id: String, parameters: Vec<ActionParameter> }`, parameters `{ name: String, value_type: ValueType, required: bool }` exist. The call explicitly has no function pointers or registry. |
| `crates/crpg-testkit/src/replay.rs` and crate `AGENTS.md` | Opaque `serde_json::Value` payloads; caller-owned apply function, inputs in file order → tick → hash. B4 adapters live under tests, not shared production protocol vocabulary. |
| `tools/lint/deps.py::ALLOWED` | Net may reach core/data/sim, not rules/script/contracts. Data may reach core only. Script may reach core/data/rules/sim. Allowed edges are ceilings, not permission to add dependencies. |

Read together: spec §§5.2/6.2/7.3–7.6/10/15.4, ADRs 0004/0008/0011,
T002:61, E003/E004/E022, BACKLOG's T018/blocker rows, remediation plan §§6–7.
ADR-0011 governs payload **fields**, not the vocabulary enum. Spec §7.3's
“same stream” wording does not describe today's replay format: replay stores
opaque inputs, not a serialized SimEvent stream. Spec §10's nine stages are
requirements for future integration, not nine implemented sim systems.

### A2. Gap 1 — minimal events, filtering, versioning and desync proof

**Recommend:** T018 v1 carries projections of the three existing SimEvent
variants only. This is sufficient to test codec/transport convergence,
structural replication, combat death, and rejection-without-mutation. The
malicious-client suite does not require new simulation events: rejection is a
request outcome, never a world event. Use accepted ability/EndTurn operations
for the positive control, rather than a test-only invented damage operation.

| Existing authoritative variant and all fields | Proposed per-client behavior |
|---|---|
| `Spawned { entity: EntityId }` | Send a visible spawn notice only if this client is entitled to see the entity at projection time. Introduce its permitted initial state before any reference to its net id. Hidden spawn emits nothing. Becoming visible later produces EntityEnter, not a fabricated historical SimEvent. |
| `Despawned { entity: EntityId }` | Send a typed despawn notice only if the entity is still known and the removal is disclosable under the authoritative visibility context captured before removal. Otherwise send only a cause-neutral EntityLeave if still known; send nothing if already forgotten. A retained cleanup mapping grants no disclosure permission. World no longer has the components, so never query the removed entity for a payload. |
| `Died { entity: EntityId }` | Send only when death itself is disclosable and the entity is known/visible. No attacker, secret stat, roll, DC or ability is inferred. Died never implies despawn; retain the permitted terminal entity until a separate leave. |

Candidate wire event fields: `server_tick: u64`, `event_seq: u64` (per-client
delivery order), and a closed `Spawned/Despawned/Died { entity: NetEntityId }`
payload, where `NetEntityId` is a nonzero `u64` scoped to the session/replica
epoch, never reused in that epoch. Internally keep the whole core EntityId,
including generation, in the mapping. Do not serialize raw arena ids or reuse
sim queue sequence gaps as a covert count of undisclosed events. Preserve
relative `(Tick, sim_seq)` order when assigning client sequences; intent seq,
sim seq and delivery seq are three distinct domains.

Wire version: explicit `u8` protocol version (initial value 1), reject an
unknown version/discriminant and trailing bytes. Freeze the approved v1 tags,
field order and meaning; an incompatible change or new closed variant needs
a reviewed protocol version and conformance fixtures. Never derive a stable
wire contract by serializing the evolving sim enum directly. This version is
independent of replay format 1, authored schema versions and World saves.

**Mandatory T018 boundary:** explicit allow-listed projection, including
per-field permissions, before encoding; an unrestricted World/save serializer
is not a snapshot. T018 can use a supplied visibility/ownership table in its
net-local test driver to prove non-disclosure and mapping revocation. That
driver is not an implemented perception system or reusable host. Area/LOS
policy stays a named follow-on, but filtering itself cannot be deferred.

Candidate v1 replica facts are entity presence, permitted transform, permitted
health/dead values, and permitted active-actor/round state. Omit undisclosed
fields rather than sending zero or hidden entity identities. Exact component
wire shapes/masks, snapshot chunking and replica queries are Part 2 decisions;
there is no open-ended component byte bag in this recommendation. A removed or
newly hidden entity loses its actionable mapping before new requests execute.

For desync, compare the reconstructed replica with an independently projected
per-client expected view after delivery converges; corrupt/drop a required
update to prove the assertion detects divergence. A filtered replica cannot
equal the full World hash. Separately compare authoritative runs receiving the
same admitted command schedule, including complete RNG/event state for rejected
inputs. Do not claim that arrival reordering preserves gameplay ordering for
different clients or that two native targets share hashes.

Retain the Phase 4 roadmap's 5,000-tick loss/jitter exercise when specifying the
T018 conformance child. The combat-only proof is an earlier gate, not completion
of Phase 4's real-network move/fight/reconnect acceptance; those capabilities
remain with the named follow-ons and host integration.

**Genuine option — richer presentation before v1:** action/turn/damage results
can first be captured by the host from `ActionOutcome` and post-operation World
queries and converted to filtered receipts/deltas. This needs no new SimEvent,
but does not provide a durable, world-owned history of every intermediate
transition. Recommended for the narrow T018 proof; C4 must actually capture the
return value, rather than discard it like a hash-only adapter.

If C1 must instead establish authoritative event history before T018, candidate
new sim vocabulary for separate review is:

- `ActionResolved { actor: EntityId, target: EntityId, ability: Ulid,
  outcome: String, damage: u32 }`: symbolic outcome, no rules enum/DiceRoll nested
  in an event. Consumer: filtered combat log/result projection plus sim ordering
  assertions. Disclosure requires every referenced identity/fact to be allowed;
  otherwise suppress this event, still replicate independently permitted target
  state. Damage is the controller's reported damage, not an assumed health delta.
- `TurnStarted { actor: EntityId, round: u64 }` and
  `EncounterEnded { encounter: Ulid, round: u64 }`: consumers would be timeline
  presentation and terminal lifecycle assertions. Suppress hidden actor/encounter
  identities; a client's turn display uses only its permitted state projection.
  Terminal combat and explicit release are distinct; the producing transition
  and ordering must be decided in C1, not guessed here.

These are **not** the recommended v1 minimum and are not approved additions.
Each violates the current closed sim API unless separately authorized by an ADR,
producer/real-consumer contract and sim-only task. Net cannot add them. Adding
emissions or draining the authoritative queue changes hashed state: do not do
either under T018, change hash exclusions, or rebaseline B4/legacy goldens.
For the T018 fixture driver, observe a detached queue clone with a cursor;
production bounded retention/drain and outcome correlation remain C1/C4 work.
Remediation C2 currently says “C1 contracts”; choosing the narrow alternative
requires an explicit approved scoping update, not silently declaring C1 done.

### A3. Gap 2 — typed intents, admission and retry semantics

**Recommend seq AND an advisory observed tick, with different jobs.** Per-intent
seq identifies/retries commands; a tick is not unique when several actions occur
in one tick. ADR-0004 demonstrates this distinction, while T002:61 explicitly
says its spike shape need not match final protocol. The client never schedules
or rewinds authority by supplying a tick.

Candidate envelope (field types are recommendations, not an exported API):

| Field | Proposed type / meaning |
|---|---|
| `version` | `u8`, 1, in the frame header, not repeated inside the intent |
| `session_epoch` | `[u8; 16]`, host-issued connection/session generation; replay isolation, not authentication credentials |
| `seq` | `u64`, starts at 1, checked monotonic increment; never wraps |
| `observed_tick` | `u64`, last authoritative tick acknowledged by this client |
| `actor` | `NetEntityId` (nonzero `u64` above), resolved through authenticated peer binding |
| `body` | Closed two-variant intent below |
| `DeclareAction.ability` | `crpg_core::Ulid`, authored ability identity (canonical ULID representation pinned later) |
| `DeclareAction.target` | One `NetEntityId`; matches implemented single-target CombatAction |
| `EndTurn` | No body fields; actor is in the envelope |

No client-supplied peer identity, ownership, damage, resource cost or RNG state.
**Needs user call:** approve the narrow combat-only executable vocabulary?
Movement, positional/multi-target abilities, dialogue, items, interaction,
save/admin and chat are specified future verbs but have no corresponding
implemented sim/host authority here. Do not pretend they work by mutating
transforms or adding placeholder success handlers. Their tags are not v1
variants; arbitrary tags are `UnknownMessage`. A future movement proposal can
use finite `[f32; 3]` spatial positions with area bounds, but coordinates/range
semantics belong to the movement task, not a current accepted intent.

Proposed validation taxonomy and precedence (first failure wins):

| Stage / owner | Check | Candidate outcome code / effect |
|---|---|---|
| Transport/codec (`crpg-net`) | Frame byte limit before allocation, bounded ingress queue | `FrameTooLarge`, `QueueFull`; no decode/World access |
| Transport/codec | Header/version, direction/channel, tags, lengths, complete decode, no trailing bytes | `UnsupportedVersion`, `WrongDirection`, `UnknownMessage`, `Malformed`, `LimitExceeded`; terminate malformed protocol session, no unauthenticated detailed reply |
| Transport admission (`crpg-net`, host-supplied monotonic time) | Per-peer receive budget, including retries and invalid attempts | `RateLimited`; discard without semantic execution; budget response traffic too |
| Host authority (placement/API owned by E012/E022) | Authenticated connection, matching epoch | `Unauthenticated`, `SessionExpired`; close/refuse, no sim call |
| Host session admission | Sequence/dedup rules below | `SeqConflict`, `SeqGap`, `StaleSeq`, `SeqExhausted`, or cached result |
| Host session admission | New command's observed tick within `[server_tick.saturating_sub(200), server_tick]` | `StaleTick`, `FutureTick`; no client clock trust, no rollback |
| Host authority | Resolve actor/target in that client's current mapping, actor controllable, target facts addressable | Public `NotAuthorized` for missing/hidden/foreign ids alike; private diagnostics may distinguish reasons without leaking them |
| Host capability policy | Ordinary player operation allowed; privileged operations denied until E018 | `NotAuthorized`; no GM bypass via in-process connection |
| Host invokes sim | Revalidate current gameplay legality immediately before execution, through `perform_action` | Public `IllegalAction`; retain typed CombatError internally. Existing sim precedence stays authoritative, not copied into net |
| Host result projection | Capture accepted return plus permitted post-state before next operation overwrites observations | `Applied` even on a legal failed attack; accepted EndTurn is Applied with no attack result |

Structural checking alone never authenticates an entity. §6.2's shared
`legal_actions -> Vec<ActionOption>` is unimplemented; don't manufacture it in
net or call it a tested defense. Current `perform_action` is the source of
gameplay rejection, and a later sim task owns the shared legality query. Host
admission can occur at spec §10 stage 1; gameplay must be checked again at stage
3 against current state. Visibility for deltas is evaluated after stage 7.
The T018 net-local driver directly sequences public sim calls for conformance;
it must not claim to implement all nine host stages or change `tick.rs`.

Proposed duplicate/retry rule:

- Reliable ordered logical command lane. One sequence stream per epoch across
  both variants; next new seq must equal last finalized seq + 1. `SeqGap` does
  not advance it; retry the missing command. Do not lift the spike's
  highest-seq-wins movement rule to resource-spending actions.
- Retain the last 256 finalized requests' canonical intent bytes and bounded
  public outcomes per peer. Same epoch/seq/same decoded canonical fields returns
  the cached outcome without revalidating gameplay, re-emitting events, spending
  or drawing. Retry must retain observed_tick too. Authenticate and apply ingress
  caps before cache lookup; do not replay formerly permitted secret payloads.
- Same retained seq with different canonical fields is `SeqConflict` and closes
  the epoch. Older than the cache is `StaleSeq`, never reapplied. New epoch resets
  numbering; session reattachment is not yet supported. Sequence exhaustion
  requires a new epoch, never saturation or wrap.
- A well-formed next-seq request reaching temporal/authority/gameplay validation
  finalizes once, whether Applied or rejected. Such rejections consume seq but
  never World/RNG/events. Early frame/rate/queue failures and SeqGap do not consume
  seq; retry the same bytes. Advancing to a different new command requires receipt
  of the previous terminal result (or retry to recover it).
- Proposed receipt fields: `session_epoch: [u8;16]`, `seq: u64`,
  `processed_tick: u64`, `status: Applied | Rejected(code)`; no hidden stats,
  arbitrary strings or echoed offending payload. Duplicate returns the original
  terminal receipt, not a second action. Retryable transport failure is distinct
  from a terminal rejected receipt. Wire error numeric assignments await Part 2.

T018 conformance must distinguish its net-owned frame/admission checks from
host-policy examples. Net-local malicious tests cover forged ids, cross-client
bindings, revoked visibility, wrong epochs, repeated commands, gaps, exhausted
seq, stale/future ticks, invalid ability/turn/resource attempts, malformed and
oversized lengths. Every rejection needs a valid positive control and unchanged
complete authoritative state assertion. Real authenticated host integration is
blocked on E012/E018/E022; a test double passing is not that gate passing.

### A4. Gap 3 — action signatures versus executable handlers; E003

**Recommend E003 Option B (definitions only), with consumer-owned local
interfaces/implementations where the current graph requires them.** Specifically
Transport belongs in net for this milestone; no contracts trait or cross-crate
implementation is invented. Option C literally implementing contracts traits
inside net still needs net→contracts, absent from ALLOWED today. Option A's
dev-dependency relaxation does not supply production implementations and adds
graph risk. Both are genuine alternatives only with explicit graph/dependency
and human-owned contracts decisions. Record the chosen interpretation in a
later ADR/spec update; this appendix does not change §15.3 by itself.

Proposed action indirection has two distinct stores:

1. **Data-owned declaration store:** an immutable
   `BTreeMap<String, ActionSignature>` keyed by exactly the signature's
   `action_id`, built from caller-supplied trusted declarations. Reuse the
   existing `ActionParameter` and `ValueType`/`DataValue` shapes, preserving
   parameter declaration order. Reject duplicate action ids/parameter names,
   missing required arguments, extra arguments and wrong value categories.
   Candidate policy: 1,024 actions, 32 parameters/action, 128 UTF-8 bytes per
   action/parameter name; no empty names or implicit overwrite. No Rust function
   pointer, rules/sim import, filesystem discovery or dynamic native loading in
   data. The store/validator API is a separate `crpg-data` task, not already
   implemented by the schema-able struct.
2. **Executable binding store:** `crpg-script` owns an immutable ordered table
   from symbolic action id to a locally defined Rust handler plus the identical
   data declaration. It is the proposed IR interpreter owner and can reach sim
   under ALLOWED. Trusted startup assembles bindings; fail before graph execution
   on missing implementation, duplicate registration or signature mismatch.
   Editor/schema consumers receive declarations only. Exact handler context,
   return/errors, budgets and World-owned continuation semantics wait for the
   script task/E010, not a fictitious function taking an unrestricted host.

Authored IR references resolve against declarations for validation and against
the frozen trusted binding table for execution. Campaigns cannot install
function pointers by declaring a name. Native/plugin registration is E023 work.
Version the declaration bundle with engine/content compatibility; changing an
existing action's parameter meaning requires an explicit content compatibility
decision, not silently replacing the handler for old graphs.

Client `DeclareAction.ability: Ulid` is **not** `ActionCall.action_id: String`:
the former chooses an authored combat ability already executed by sim, the latter
names IR vocabulary. T018 needs neither a general action registry nor a script
dependency to call today's CombatAction. Approve this scoping distinction so a
future script feature does not unnecessarily block a combat transport proof.

### A5. Gap 4 — caps, scope and named follow-ons

All limits inclusive; KiB = 1024 bytes, MiB = 1024². **Proposed policy values,
not measured throughput promises.** Enforce limits before reserving/allocating
from hostile lengths, with checked arithmetic and bounded encode as well as
decode. A frame cap alone does not bound an encoded Vec length. No compression,
arbitrary recursive Value, or bulk package transfer in v1.

| Resource / scope | Proposed initial ceiling | Rejection / backpressure |
|---|---|---|
| Client intent frame, header included | 4 KiB per frame | `FrameTooLarge` before decode |
| Server delta frame | 64 KiB, ≤256 operations | `FrameTooLarge` / `LimitExceeded`; sender chunks only at operation boundaries |
| Any wire string / byte field | 256 UTF-8 bytes / 4 KiB; total frame cap also applies | `LimitExceeded`; no opaque unbounded component payload |
| Any wire collection | ≤256 entries; intent has one target, no variable target list | `LimitExceeded` before allocation; no nested collection language |
| Intent arrival per authenticated peer | Token bucket 40 frames/second, burst 80; separate 64 KiB/second byte bucket, burst 128 KiB | `RateLimited`; retries count; monotonic host time or simulated injected time, never World tick (pause cannot stop replenishment) |
| Pre-auth connection | At most 4 KiB opening frame, no gameplay queue until authenticated; T018 supplies a bound test peer, not auth protocol | Refuse `Unauthenticated`; real handshake/auth limits await E022 |
| Ingress queues | Per-peer 128 frames AND 256 KiB; host total 1,024 frames AND 2 MiB | `QueueFull`, no growth past either ceiling |
| Outgoing queues | Per-peer 128 frames AND 2 MiB; host total 16 MiB | Backpressure; terminate slow session with `QueueFull`, never silently lose reliable events |
| Cache / peer population for T018 proof | 256 receipts+canonical intents/peer, ≤2 MiB cache/peer; 8 peers | `StaleSeq` after eviction; refuse excess peer with `ServerBusy` |
| Snapshot/reassembly / replica | 1 MiB snapshot, 1,024 visible entities, one in-flight snapshot/peer, ≤32 chunks, 5 seconds reassembly deadline | `LimitExceeded` / `SnapshotExpired`; rebuild/resync, never partial publication |

Rate accounting is ingress policy, outside core/rules/sim. Budget compact failure
responses with separate per-peer token buckets: 10 responses/second, burst 10,
and 4 KiB/second, burst 4 KiB. These buckets cover all failure responses (including
cached rejection receipts), not normal outbound deltas; all share the outgoing
queue caps. Drop excess failure responses; a sender can retry under the retained
sequence policy.
Transport fault schedules use injected time and a bounded queue, not sleeps.
For simulated loss, reliable logical delivery must retry/deduplicate in the
transport model; an intentionally unreliable lane cannot silently be used for
combat commands. Exact scheduling and error assignments remain Part 2 work.

Named follow-ons (proposed labels, not newly allocated task numbers):

| Name | Owning crate(s), each a separate task | Phase / prerequisite |
|---|---|---|
| C1 outcome/retention decision | `crpg-sim` for any new vocabulary or queue API; host consumer separately after E012/E022 | Phase 4 before richer event-history claims; ADR and replay compatibility decision first |
| N-INTEREST: area membership + perceived component projection | `crpg-net` for filtering/interest machinery; host for authoritative viewer context; any missing perception facts in a separate sim task | Phase 4 follow-on; mandatory simple explicit filtering is already in T018 scope, actual area/perception implementation waits for facts |
| N-LEGALITY: shared action options | `crpg-sim`; AI consumption separately in `crpg-ai` | Before Phase 9 AI; earlier if player action enumeration needs it, §6.2 query contract, no duplicated net legality engine |
| N-QUIC: real channels/loss/reorder | `crpg-net`, then dedicated host adapter | Phase 4 after in-memory conformance; ADR-0004 dedup-window decision before datagram dependence, port forwarding/NAT scope retained |
| N-RECONNECT: grace/session resync | Host selected by E012/E022; net resync mechanisms separately | Phase 4 after auth/capabilities and filtered snapshot contract; propose 30-second grace, full filtered snapshot, retained command epoch/cache during grace, no combat escape. AI-versus-frozen control and cross-process persistence require a user decision before this task |
| N-PREDICT: local movement prediction/interpolation | `crpg-godot` bridge/client; any reusable protocol support separately in net | Phase 5 Client, after authoritative movement/nav and ack contract; reconcile by seq with server tick as time, no combat prediction |
| IR-SIGNATURES / IR-BINDINGS | `crpg-data` declarations, then `crpg-script` handlers | Phase 8 Event graphs/dialogue/quests; earlier for any MVP script consumer, E010 budgets and continuation owner first; no T018 dependency |

Phase numbers refer to the roadmap; these rows are planning proposals, not
claims of existing task files. Grace-window behavior must specify what happens
to resources/turns while disconnected; reconnect cannot reset the dedup ledger
and reapply old attacks. T018 only tests closed/expired epoch refusal and full
view replacement in a fixture, not persistence-backed session recovery.

### A6. Approval checklist and bounded agent handoff

Each item is unapproved. Labels distinguish a recommendation from a genuine
alternative and a decision that cannot be made by this agent.

- [ ] **recommend:** Approve E003 B with Transport local to net and implementations
  consumer-owned under existing allowed edges? This requires later ADR/spec
  reconciliation, not a contracts edit in T018.
- [ ] **needs-user-call:** Approve the three-existing-events, two-combat-intents
  T018 scope and explicitly defer C1's richer history? Or require a separately
  approved sim prerequisite first? The latter must resolve hashed event emissions
  while keeping existing B4/legacy goldens untouched; T018 cannot solve it by
  silently rebaselining.
- [ ] **recommend:** Approve filtered net-id projections, per-client delivery seq,
  strict v1 versioning, and replica-to-permitted-view desync comparison?
- [ ] **recommend:** Approve seq plus advisory observed_tick, strict ordered new
  commands, 256-entry retry cache and terminal rejection semantics in A3?
- [ ] **genuine option:** Keep the proposed 200-tick age bound, or accept any past
  observed tick and rely only on present-state legality? A bound rejects delayed
  stale UI commands; omitting it tolerates pauses/network delays but admits older
  intentions. Neither option grants rollback or client-authoritative scheduling.
- [ ] **recommend:** Approve data-owned signature declarations and script-owned
  executable bindings, with combat ability ids kept distinct from IR action ids?
- [ ] **recommend:** Approve A5's byte/count/queue/rate/snapshot limits and A3's
  error categories as the initial product envelope? Exact wire codes and public
  API names remain for the approved executable specification.
- [ ] **needs-user-call:** Approve the named follow-on ownership/phase queue?
  E012/E022 still choose host placement/API, E018 still chooses capabilities;
  grace freeze-versus-AI and persisted dedup policy remain expressly undecided.

**After explicit approval and instruction only:** a specification agent reads
this appendix + the approval record + E004 and writes ordered single-crate
children for protocol/codec, simulated transport, and conformance. Prefer all
three in `crpg-net`, including net-local tests, under the proposed local Transport
ownership; do not pull net/host runtime dependencies into testkit. If a different
approved conformance home requires a second crate, it gets its own later child
and dependency review. The agent must supply exact public shapes, code/tag
assignments, limits, acceptance tests and literal stopping commands then, and
open net with its architecture doc/AGENTS. Nothing in this appendix is that
contract or a claim that a future test command already exists.

Independent specification agents can take approved D0→D1 and D0→D3; serialize
D2/D4/D5 edits within CLI and honor their prerequisites. Host decision work can
proceed separately through E012/E018/E022 without inventing host signatures in
T018. Keep replay payload decoding caller-owned; no changes to replay version,
input→tick→hash order, B4 adapters, existing fixtures/goldens, hash exclusions or
native target selection. Gates 9+ remain capability-gated per spec §15.4:
previous replay passes do not verify protocol, host, persistence or product smoke.

### Appendix attribution

- 2026-09-28 (UTC) · opencode/gpt-6-astra + E017 proposed decision brief · Inspected the merged event, World, combat, IR and replay boundaries before proposing a minimal protocol scope and concrete admission policies. Kept approval, host API design, sim event expansion and executable T018 contracts explicitly gated so subsequent agents can work from bounded decisions without changing replay compatibility.
- 2026-09-28 (UTC) · opencode/gpt-6-astra + E017 delegated review · Tightened despawn disclosure to avoid exposing a hidden removal, made failure-response rate budgets explicit, and aligned follow-on phase names with the actual roadmap. These are corrections to the unapproved proposal, not approval of its scope or policies.
- 2026-09-28 (UTC) · opencode/gpt-6-astra + Phase 4 gate scope clarification · Retained the roadmap's 5,000-tick fault exercise while distinguishing a combat-only T018 proof from the broader movement, real-network and reconnection milestone. This prevents the proposed scope reduction from being mistaken for full Phase 4 acceptance.

## Appendix B — Durability review for robustness / longevity / flexibility (2026-09-28, non-binding)

**E017 remains OPEN. This appendix revises Appendix A under an explicit
robustness-first priority. Nothing here is approved and nothing is a T018
contract.** Appendix A over-weighted speed-to-proof: several choices would have
worked for a first gate and then required breaking changes. The revision below
keeps a small first proof only where the wire shape can carry it without
breaking, and attaches a full ordered evolution plan to every provisional
element so a later agent can implement the change without redesigning the
protocol.

Durable means the choice is intended to survive into the stable product without
a wire break, a rebaseline, or a dependency-graph change. Provisional means the
choice is bounded test scaffolding with a named successor, an owning crate, a
compatibility rule, and acceptance evidence. A provisional choice without all
four is rejected by this appendix.

### B1. Classification of Appendix A choices

| Appendix A choice | Revised verdict | Why |
|---|---|---|
| Three existing events + host-captured `ActionOutcome` for v1 | **Provisional scope on a durable wire** — keep only if the wire reserves extension and C1 is approved alongside | Avoids new hashed emissions now, but presentation needs durable history later |
| Detached queue-clone observation in the T018 driver | **Provisional test scaffolding only** — never a production retention policy | Clone/cursor proves projection without touching hashed state; production needs bounded retention/drain |
| Strict stop-and-wait seq + 256-entry cache | **Durable wire sequence, provisional window policy** — keep seq/cache wire-compatible, specify window as policy | Seq/dedup semantics survive; window size can grow without a wire break if lanes are separated now |
| No lane/channel field, one stream for both intents | **Change before stable v1** — add an explicit lane field now, implement only lane 0 | Without lanes, adding movement later reuses combat delivery semantics and breaks them |
| 200-tick observed-tick bound | **Durable as host policy, not wire** — keep value provisional, keep placement durable | A server-side constant can be tuned without a wire break; a wire-embedded constant cannot |
| Data declarations + script bindings split | **Durable** — no revision | Respects the enforced graph and separates tooling from execution |
| Fixed network caps as a single table | **Split before stable v1** — wire hard maxima versus operational policy | Hard maxima need versioning; operational rates need safe downward configurability |
| E003 Option B, Transport local to net | **Durable for this milestone** — no graph change | Net owns its protocol; a future shared abstraction can be added as an adapter without moving v1 |

### B2. Gap 1 revised — narrow scope only on an extensible wire, C1 approved in parallel

Keep the three-event, two-intent executable scope for the first conformance
gate, but do not call that gate stable v1 unless these durability conditions
are approved with it:

1. Wire header carries `version: u8`, `lane: u8`, and a closed discriminant
   space with reserved values. Lane 0 is reliable-ordered commands and
   reliable-ordered deltas. Lanes 1+ are reserved; v1 implements lane 0 only
   and rejects all other lanes as `UnknownMessage`. This prevents future
   movement from inheriting combat ordering.
2. v1 freezes only the tags it implements. Any additive event/intent variant,
   field addition, or meaning change requires a reviewed protocol version bump
   plus new conformance fixtures. v1 implementations reject unknown
   versions/discriminants and trailing bytes; they never ignore-and-continue.
3. Never serialize the evolving sim enum directly. The wire event is a
   versioned projection with its own tags, field order, and per-field
   disclosure rule. Sim can grow without silently moving the wire.
4. The T018 driver may observe a detached queue clone with an explicit cursor
   for projection tests only. Production retention, drain, correlation of
   outcomes to events, and per-client delivery cursors are not solved by that
   clone. They belong to the evolution plan below.

**Evolution plan for authoritative history (each row is a separate single-crate
task, ordered):**

| Order | Sole crate | Deliverable, compatibility, acceptance |
|---|---|---|
| C1-sim | `crpg-sim` | ADR + new vocabulary/queue API only if approved. Producer and real consumer named. New emissions covered by new semantic oracles and new goldens; existing B4/legacy goldens byte-identical. Hash-exclusion changes forbidden without their own ADR. |
| N-EVENTS-v2 | `crpg-net` | Protocol version bump carrying the newly approved events as new discriminants. v1 fixtures still pass against a v1 peer; v2 conformance proves rejection of unknown variants by v1 and correct projection/filtering of new variants by v2. No silent reinterpretation. |
| C4-consume | Host crate selected by E012/E022 | Capture accepted `ActionOutcome` plus permitted post-state before the next operation, correlate to wire receipts/deltas, enforce bounded retention. Acceptance: one valid and one rejected action through transport, permitted replica order asserted, hidden fields absent, save/restart continuation identical. |

If the user wants longevity over speed, the alternative is to require C1-sim
before any protocol freeze. That delays transport proof but removes the
provisional scope entirely. This appendix recommends provisional scope **with**
the above plan approved at the same time, rather than provisional scope with
the plan deferred.

### B3. Gap 2 revised — durable identity, provisional window, explicit lanes

Retain seq as command identity and observed tick as advisory freshness, with
these durability corrections:

- One sequence stream per epoch **per lane**, not one global stream. Lane 0
  starts at 1, increments by 1, never wraps; exhaustion requires a new epoch.
  Lane separation is wire; window size is policy.
- v1 policy for lane 0 is window 1 (stop-and-wait): advance to a new command
  only after the previous terminal receipt, or retry to recover it. The wire
  already supports a larger window (gap detection, cached receipts, conflict
  close), so growing the window later is a policy + conformance change, not a
  wire break. Do not specify window >1 in v1.
- Retain 256 finalized canonical intents + bounded public outcomes per peer per
  lane as the v1 cache. Same epoch/lane/seq with identical canonical bytes
  returns the cached receipt without re-execution; different bytes is
  `SeqConflict` and closes the epoch; older than cache is `StaleSeq`. Cache
  lookup happens after authentication and ingress caps; cached rejections never
  replay formerly permitted secret payloads.
- Observed-tick bound stays a host policy constant (proposed 200), validated
  against current `server_tick`, never trusted as scheduling. Changing the
  constant later must not change the wire. Document the pause semantic:
  ticks, not wall time, age observations.
- Receipts carry epoch, lane, seq, processed tick, and Applied/Rejected(code)
  only. No hidden stats, no echoed offending payload, no arbitrary strings.

**Evolution plan for delivery semantics:**

| Order | Sole crate | Change without breaking v1 combat |
|---|---|---|
| N-LANES-policy | `crpg-net` | Raise lane-0 window or add lane-1 movement semantics as a versioned policy + conformance update. Combat lane behavior for v1 peers unchanged. Highest-seq-wins is confined to a future movement lane, never applied to lane 0. |
| N-RECONNECT | Host + net mechanisms separately | Grace, resync snapshot, retained epoch/cache policy. Reconnect never resets the dedup ledger and never reapplies old attacks. |

### B4. Gap 3 confirmed durable — add versioning to the declaration bundle

Appendix A split stands. Add for longevity: the declaration bundle is
versioned with engine/content compatibility; changing a parameter meaning
requires a content-compatibility decision; campaigns cannot install handlers
by declaration. Executable bindings assemble at trusted startup and fail
closed on missing/duplicate/mismatched registrations. Combat ability ULIDs and
IR action-id strings remain distinct namespaces. No script dependency enters
T018.

### B5. Gap 4 revised — two-tier limits, explicit evolution

Split Appendix A caps into:

- **Wire hard maxima (versioned):** frame sizes, collection entry counts,
  string/byte field ceilings, operation counts, snapshot/chunk ceilings. Raising
  any hard maximum requires a protocol version review because it changes what a
  peer must accept.
- **Operational policy (downward-configurable):** token-bucket rates, bursts,
  queue depths, peer counts, cache sizes, reassembly deadlines, grace periods.
  Operators may tighten them; loosening beyond the approved envelope requires
  the same review as a hard-maximum change. All enforcement happens before
  allocation from hostile lengths, on encode as well as decode.

Failure-response budgets are separate per-peer buckets from normal outbound
traffic, sharing only the outgoing queue caps. Dropped failure responses are
recoverable through the retained sequence policy. Transport fault injection
uses injected time and bounded queues.

### B6. E003 confirmed — local Transport is not provisional debt

Transport in net does not need a later move. If a second transport or a shared
harness needs a common shape, add a narrow adapter trait in the consuming
higher crate without moving v1. No ALLOWED change is proposed here.

### B7. What agents implement after approval, in order

1. C0 decisions (E003/E012/E017/E018/E022) recorded; C1-sim scoped if richer
   history is required before stability.
2. T018 lane-0 protocol/codec child in `crpg-net` on the extensible wire.
3. T018 simulated-transport child in `crpg-net` with bounded fault schedules.
4. T018 conformance child in `crpg-net` (net-local malicious suite + desync
   oracle against permitted views + 5,000-tick loss/jitter exercise).
5. C1-sim, N-EVENTS-v2, host slice, N-QUIC, N-RECONNECT, N-PREDICT, IR tasks
   per §§B2–B5, each with exact shapes, limits, tests, and stopping commands
   specified at that time.

Conformance lives in net under the proposed ownership. Do not add net/host
runtime dependencies to testkit. Replay stays opaque and caller-owned;
existing fixtures, goldens, hash exclusions, and native target selection stay
untouched. Gates 9+ remain capability-gated: prior green suites prove nothing
about new protocol/host claims.

### B8. Revised approval checklist (supersedes A6 for decision purposes; A6 retained as history)

- [ ] **recommend:** Approve lane-0 provisional scope **with** the C1/N-EVENTS-v2
  evolution plan approved in principle at the same time, on the extensible
  wire in §B2? Or require C1-sim before any protocol work?
- [ ] **recommend:** Approve per-lane seq, window-1 policy, 256-entry cache,
  advisory observed tick, and terminal-rejection semantics in §B3?
- [ ] **genuine option:** Keep the 200-tick host-policy bound or accept any past
  tick subject to current legality? Placement as host policy is durable either way.
- [ ] **recommend:** Approve two-tier limits (versioned hard maxima, tightening-only
  operational policy) and separate failure-response buckets in §B5?
- [ ] **recommend:** Approve E003-B local Transport, data/script action split with
  versioned declaration bundle, filtered net-id projections, and strict versioning?
- [ ] **needs-user-call:** Approve the ordered evolution queue in §B7 while leaving
  host placement/lifecycle (E012/E022), capabilities (E018), grace freeze-vs-AI,
  and persisted-dedup policy to their owners?

### Appendix B attribution

- 2026-09-28 (UTC) · opencode/gpt-6-astra + E017 durability review · Reclassified Appendix A choices into durable versus provisional-with-evolution-plan and added lane separation, two-tier limits, and the ordered C1/v2/host evolution queue so a narrow first proof cannot silently become permanent debt.
- 2026-09-28 (UTC) · opencode/gpt-6-astra + approval receipt and Part 2 contracts · Recorded user approval of Appendix B with recommended defaults (200-tick host-policy bound, D4a now with D4b as release blocker, cooperative tree) and wrote ordered single-crate T018/T018a/b/c contracts without approving E017 itself or authorizing implementation.

## Resolution — 2026-09-30 (T033, D01, T018/T021)

The option calls were settled in POST-T018 D01 and by the as-built T018/T021
wire. T033 reconciled spec §5.2 (action registry → T029a declarations plus
T029b trusted bindings; continuations → world-owned IR state, D17), §6.2
(`legal_actions` as built by T028), §7.2–7.3 (lane byte, per-lane sequence as
command identity with advisory tick, wire caps versus policy values, versioned
projections instead of raw `SimEvent`), §7.4 (whole-area presence plus explicit
per-field grants, D14), §7.5 (movement after its spec, D13/D24) and §7.6
(30-second same-process grace and snapshot resync, D11/D12). Residual work is
the named evolution queue (T019–T029b), not this task.

- 2026-09-30 (UTC) · claude-code + T033 · Recorded how the already-selected decision closes this item and where the spec now says it, per D25.
