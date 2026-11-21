# crpg-server architecture

## Scope

`crpg-server` owns authoritative world state. As of T022 it is a library plus
a thin dedicated binary (D02): the library is the one platform-neutral host
that the Windows embedded single-player adapter and the Windows/Linux
dedicated adapters reuse through the T154a run loop, and the binary is the
OS adapter over it (T040b, below): a wiring shell that holds no host policy.
The exact contract —
API, error precedence, numeric caps, checkpoint shape and acceptance cases —
is the "Specification revision — 2026-09-29" appendix of
[T022](../../tasks/T022.md); the reasoning is
[ADR-0022](../adr/0022-host-session-capture-retirement.md). Verification
status belongs to the T022 completion record, not here.

What exists today is the in-memory host slice: one serial `Host` privately
owning one T020 `HistoryWorld`, authenticated-peer bindings with per-session
epochs, per-entity disclosure and control grants, byte-level ingest, a FIFO
admission pump, per-peer delivery logs in the T021 version-selected wire,
the bounded capture journal, in-memory checkpoint bytes with fresh
sessions on restart, and (T039) host save files: the checkpoint behind a
campaign/engine identity header, written and read through `crpg-persist`'s
compressed, capped envelope and atomic-replace file store, and (T023b,
[ADR-0026](../adr/0026-quic-host-adapter.md)) the real-transport adapter:
one `Host` driven over a `crpg-net-quic` server with invitation
credentials, backpressure, fencing and D02 shutdown. What is planned and
**not** here: credential generation; reconnect, resume and the 30 s grace
(T025a/T025b); production interest facts
(T027a–c supply values for these grant shapes); save slot/autosave policy and
the save directory; native OS service integration; listen-server mixed mode;
and the GM endpoint. The run loop and the in-memory local hosting exist since T154a, the
background runner and its clock since T154b, the QUIC hosting of the same
loop since T154d, and the dedicated binary that wires them to operator files
since T040b.

## Where it sits

```text
crpg-core     ─┐
crpg-sim      ─┤
crpg-net      ─┼─> crpg-server (lib) ─> crpg-server (bin, thin)
crpg-nav      ─┤     + serde/serde_json (checkpoint JSON)
crpg-ai       ─┤
crpg-net-quic ─┤
crpg-persist  ─┘
```

Edges are D20's: normal path dependencies on `crpg-core`, `crpg-sim` and
`crpg-net` plus workspace `serde`/`serde_json`, and T039's path edge to
`crpg-persist` (approved in [T039](../../tasks/T039.md) §8/Decisions; its
`zstd` tree was already locked through T038), T023b's path edge to
`crpg-net-quic` (ADR-0024, T023 R12), T037a's path edge to `crpg-nav`
(ADR-0036 H1–H5, approved at T037 Question 1 (a)) for the mesh, its bake,
queries and paths, and T153's path edge to `crpg-ai` (D65f) for the MVP
decision. That edge brings the transport's
quinn/rustls/tokio tree in transitively, but `crpg-server` names none of
those crates and has no direct edge to them. Nothing else. The host
consumes T020's public `HistoryWorld` (`perform_action`, `read_after`,
`acknowledge`, `history_hash`), T028's `validate_action`, and T021's
`codec`/`codec_v2`/`projection_v2` plus the T018b policy shapes
(`QueueCaps`, `RateCaps`, `TokenBucket`) — never T018c's test-only driver.
Sim and net stay OS-free and host-agnostic; all session, disclosure,
retention and checkpoint policy lives here.

## Module flow

- `host` — authority and lifecycle. `Host::new` (empty authority) and
  `Host::from_history` (embedding-choreographed authority with an empty sim
  journal) construct the host; `bind_peer`/`unbind_peer`/`set_grants`/
  `set_control` manage trusted binding facts; `ingest` does byte-level
  admission only (open → time → binding → rate → frame cap → queue room);
  `pump` drains the staged FIFO through decode → session epoch → per-peer
  seq/cache → freshness → mapping/control/disclosure → reservation →
  `perform_action` → commit. Outcomes are determined first (gameplay class
  through sim's own `validate_action`), worst-case room for that class is
  checked without mutating, and only then is anything committed; a failed
  reservation leaves the head staged. Accepted commands run execute →
  capture → project → cache → acknowledge to completion before the next
  command. `take_delivery`/`acknowledge_delivery` expose each peer's
  idempotent, gapless delivery log; `shutdown` is an idempotent fence.
- `capture` — the trusted evidence shapes and bounds. One `CapturedRecord`
  per accepted command: the returned outcome's symbolic fields, the exact
  `(history_start, history_end]` range with a clone of its envelopes taken
  before the sim acknowledgement, and the permitted post-state of every peer
  that received content (`CapturedView`). Records retire only through
  `acknowledge_captures`; reservation blocks new accepts when the journal is
  full instead of dropping a result. Events the host makes itself have their
  own `HostRecord` (T141s, below).
- `checkpoint` — `Host::save_checkpoint` and `load_checkpoint`/
  `load_checkpoint_from_reader`: compact JSON of the wrapper, the retained
  capture journal plus watermark, protocol and incarnation, in version 1
  (no host record retained, the original bytes) or version 2 (at least one
  `HostRecord`, with a `host_records` list); the loader reads both. Input
  capped before parsing; loading validates everything into a temporary host
  and restarts every session under a strictly greater incarnation.
- `save` — the persistence adapter (T039) and the crate's only filesystem
  I/O, all of it through `crpg_persist::save_file`/`load_file` under the
  payload kind `HOSTCKPT`. The payload is a fixed-order binary header —
  host save version (u32 LE, offsets 0..4 frozen), campaign id (16 bytes,
  big-endian ULID), campaign version text and engine version text (each a
  length byte plus 1–64 bytes of `0-9 A-Z a-z . + -`) — followed by
  `save_checkpoint` output verbatim, so `CHECKPOINT_VERSION` and `host`,
  `capture`, `checkpoint` are unchanged. Identity policy: campaign id and
  version are trusted caller facts (`SaveIdentity`; the crate has no
  `crpg-data` edge) and must match exactly on load; the engine version
  (`ENGINE_VERSION`, this crate's package version) is recorded and returned
  in `LoadedSave`, never enforced; compatibility is the three format
  versions (envelope, host save, checkpoint). Load precedence, first failure
  wins: caller identity, then the envelope (`crpg-persist`), then the header
  structure, then campaign id and version, and only then T022's
  `load_checkpoint` — a wrong-campaign save is refused without parsing its
  JSON. The header (at most 150 bytes) shares T038's unchanged 16 MiB
  payload cap, so a file save accepts a checkpoint of at most
  16 MiB minus the header length. The pure `encode_host_save`/
  `decode_host_save` expose the same format without a file. Exact contract:
  [T039](../../tasks/T039.md) §2–§4.
- `quic` — the real-transport adapter (T023b) and the crate's only network
  I/O, all of it through `crpg-net-quic`'s non-blocking API. `QuicHost`
  owns one `Host` and one `QuicServer`. `start` refuses a closed host, a
  host in use or a time regression and hands both back. Each
  `pump(now_ms)` runs fixed phases, visiting connections in ascending
  `ConnectionId` order: (0) poison and time checks; (1) poll every
  transport event, deciding hellos (constant-time credential match, then
  version, then supersession, then `bind_peer` and the welcome) and
  recording closes; (2) drain each bound connection, retrying its held
  frame first, then up to 128 fresh frames through `ingest`, holding one
  `QueueFull` frame and dropping terminal refusals; drain and discard
  retiring connections; (3) one `host.pump`; (4) fence bindings the host
  retired itself; (5) hand each binding's delivery to `try_send` and
  acknowledge what was accepted, or discard and acknowledge it for a
  closed transport, and fence a connection stalled for `STALL_TIMEOUT_MS`;
  (6) unbind closed peers once drained; (7) report. Every host call is
  recorded in `QuicPumpReport::calls`, so a replay against an in-memory
  host reproduces the host's transitions. Out-of-pump operator calls
  (`disconnect`, `revoke_invitation`, `set_invitation_grants`,
  `set_invitation_control`, `tick`, `acknowledge_captures`) are not in any
  trace. `shutdown` follows D02: flush committed delivery, count what was
  not sent, refuse staged work, optionally checkpoint, close the
  transport. Exact contract: [T023b](../../tasks/T023b.md) B§3–B§12 as
  amended by A§0–A§7.

Under v1, two host-wide caps cannot be reached by any input and are kept as
defence in depth, untested because no test can reach them; nobody should
count them as tested. The ingress host totals (1024 frames, 2 MiB) are
exactly 8 × the per-peer caps (128 frames, 256 KiB), and at most 8 peers
bind, so the per-peer cap always refuses first. The grant-narrowing
`QueueFull` ([T022](../../tasks/T022.md) §5) needs a resync state of more
than about 2 MiB, because the other seven peers hold at most 14 MiB of the
16 MiB egress host total.

## Host records and checkpoint version 2 (T141s)

Real-time ticks, host actions and the turn timer change the authority with
no originating peer, so no command record can hold their events. Each such
host operation that journals at least one event gets one `HostRecord`
(`capture_seq`, `op` of `tick`/`action`/`turn_timer`, the exact
`(history_start, history_end]` range, `processed_tick`, the cloned
envelopes). Before T141s a real-time tick's events were acknowledged
uncaptured, so the next checkpoint's evidence rule refused to load.

- **One journal of entries.** The capture journal is a sequence of
  `CaptureEntry` (`Command(CapturedRecord)` or `Host(HostRecord)`) numbered by
  the one `capture_seq`, retired by the one `acknowledge_captures`, and
  bounded by the one `MAX_CAPTURE_RECORDS` entry count and `MAX_CAPTURE_BYTES`
  total of compact `serde_json` lengths. `read_capture_entries` reads both
  kinds under `read_captures`' cursor rules; `read_captures` returns the
  `Command` entries of the same page, so a page may hold fewer than `limit`
  records.
- **Dual read, conditional write.** `save_checkpoint` writes
  `"version":1` and the original bytes when no `HostRecord` is retained, and
  `"version":2` with `"captured"` (command records) and `"host_records"`
  (each list ascending) otherwise. `load_checkpoint` reads both: version 1 is
  version 2 with no host record, so nothing is migrated; version 1 with a
  `host_records` key, or version 2 without one, is `Malformed`; an empty
  version-2 list loads and is written again as version 1. The loader merges
  the two lists by `capture_seq` (exactly `acknowledged + 1 ..= last`, no
  gap, no duplicate), checks each host record
  (`check_host_record`: non-empty range whose width is the event count, at
  most `MAX_HOST_RECORD_EVENTS`, contiguous envelopes, `processed_tick` not
  past the server tick, at most `MAX_HOST_RECORD_BYTES`), keeps the adjacency
  rule over the merged order (each entry starts where the previous ended, so
  every envelope after the wrap is evidenced) and applies the entry and byte
  totals (`check_journal_totals`).
- **Four events per peer command.** A killing command that ends the last turn
  of a round with a pending real-time switch journals `ActionResolved`,
  `Died`, `ModeChanged`, `RoundStarted`, so reservation, commit and the
  loader use `MAX_NEW_EVENTS_PER_COMMAND_RT` (4); `MAX_NEW_EVENTS_PER_COMMAND`
  keeps 3 and its turn-based meaning.
- **The host-operation tail.** After the sim commits: build the record (an
  over-bound record is `Invariant`), append it, then acknowledge the sim
  journal, which always runs; a record that could not be built is returned as
  `Invariant` only after the acknowledgement, with nothing appended.
  `commit_accepted` ends the same way. `Host::tick` is the first host
  operation: a real-time tick first reserves room for one host record
  (`CaptureFull` when the journal is full) and a turn-based, turn-based-only
  or no-encounter tick reserves nothing and journals nothing. Host operations
  never move `executions()` and send nothing to v1/v2 sessions.

## Host actions (T141a)

Three trusted `Host` calls act on the encounter without a peer. All three
return `Closed` after `shutdown`, and none moves `executions()`, consumes an
intent sequence, sets `last_combat_error` or sends a frame to a v1/v2
session.

- **`host_action(&CombatAction)`** (the AI's decision, later the turn timer's
  `EndTurn`): `Closed`, then the journal reservation of one host record
  (`CaptureFull`, always: an action may journal), then
  `crpg_sim::validate_action` (`Combat(error)`), then
  `HistoryWorld::perform_action` (`Combat(error)`; any other refusal is
  `Invariant("history refused a host operation")` and the sim committed
  nothing), then the host-operation tail with `HostOp::Action`, so the
  envelopes become one `HostRecord` before the sim acknowledgement. The sim's
  `Option<ActionOutcome>` is returned. A real-time order (`UseAbility`) and
  anything else that journals nothing makes no record. It is not stopped by
  pause (ADR-0029 P2; pause is the T141b section below).
- **`request_mode(EncounterMode)`** (the operator's mode request, ADR-0029
  T9): `Closed`, then `HistoryWorld::request_mode`; the sim's refusals
  (`NoEncounter`, `ModeUnsupported`) are `Combat(error)`. A request journals no
  event, so it reserves nothing and records nothing; the switch itself happens
  at the next round boundary and its `ModeChanged`/`RoundStarted` events are
  recorded by whichever operation crosses the boundary.
- **`awaiting_orders()`** reads the world only: nothing awaited with no
  encounter or a closed host; turn-based, the active head; real-time (T15),
  while at least two combatants are alive and the clock is below
  `RT_CLOCK_MAX`, every timeline entry with `key <= elapsed` whose combatant
  is alive and has no standing order, in ascending `(key, EntityId)` order. A
  combatant the sim removed from the timeline (the dead) or rescheduled past
  the clock (one that just acted) is not listed.

`HostError::Combat(CombatError)` is the new variant; its `Display` is
`Combat at host/admission` like the rest. Which combatants are AI-controlled
is the caller's fact: `awaiting_orders` lists everyone with no order, and the
caller decides whom to answer with `host_action`.

## Pause, settings and the tick driver (T141b)

Pause, the operator's settings and the turn timer are host state: never in
`World`, the journal, a capture or the checkpoint (ADR-0029 P1, P7). A host
from `new` or `from_history` starts running; a host from `load_checkpoint`
(so `load_host_file`) starts `Some(Restore)`-paused, and both start with
`HostSettings::multiplayer()`.

- **`HostSettings`**: `initial_mode` (stored only; T040's `start_encounter`
  reads it), `auto_pause` (the five ADR-0029 P6 toggles as
  `AutoPauseToggles`) and `turn_timer_ms` (`None` is off, `Some(0)` is
  `InvalidSettings`). `single_player()` is bits `0x19`
  (`SINGLE_PLAYER_AUTO_PAUSE_BITS`: `EncounterStarted`, `OrderEnded`,
  `PartyMemberDied`), `multiplayer()` and `Default` are bits `0x00`.
  `set_settings(settings, now_ms)`: `Closed`, `TimeRegression`,
  `InvalidSettings`, then record `now_ms` and store; a changed timer length
  disarms the timer and observes it again.
- **Pause**: `paused()` is `None` or the `PauseReason`. `pause(now_ms)` is
  `Closed`, `TimeRegression`, then `Operator` when running and a no-op when
  already paused (the reason stays). `unpause(now_ms)` clears any reason,
  whatever it was. Both record `now_ms`. Only `advance` stops while paused:
  `ingest`/`pump`, `host_action`, `request_mode` and the bare `tick()` run
  unchanged (P2). `tick()` ignoring pause is deliberate: it is the operator's
  primitive, and a `Restore`-paused host still ticks through it (gate 10).
- **`advance(now_ms)`** is the cadence: `Closed`, `TimeRegression`, record
  `now_ms`; if running, the timer step; if paused now, return
  `{ ticked: false, timer_ended_turn, paused }`; otherwise `tick()` (its error
  is returned as is, a timer `EndTurn` stays committed) and return
  `{ ticked: true, .. }`. One call ticks at most once and ends at most one
  turn.
- **Auto-pause** is step 1 of the post-commit tail (S-3, A-1), so it runs for
  every committed operation: peer command, `tick`, `host_action`, the timer's
  `EndTurn`. Events are walked in journal order; `RoundStarted`,
  `ModeChanged` and `OrderEnded` map to their trigger and `Died { entity }` to
  `PartyMemberDied` when `entity` is in the party (the union of every bound
  session's control list at that moment). The first trigger whose toggle is
  on sets `Auto(trigger)` and freezes the timer; later events are not
  inspected; an already-paused host is not touched. Zero-event operations
  never pause. Nothing in T141 fires `EncounterStarted` (T040's
  `start_encounter` will).
- **The turn timer** (P8) is observed right after auto-pause, on every
  committed operation and after a changed `turn_timer_ms`. The key is
  `(head, round)` of a turn-based encounter with an active head; with the
  timer off or no key it disarms, and when no timer is armed or the key
  differs it arms with the full length (deadline `last_now_ms + length` when
  running, a frozen remainder of the full length when paused). `advance`
  expires it when running and `now_ms >= deadline`: `host_action(EndTurn {
  head })` recorded as `HostOp::TurnTimer` (`timer_ended_turn = Some(head)`).
  `CaptureFull` leaves the timer armed (the next call retries; the tick still
  runs, a turn-based tick needs no room); any other error disarms it; neither
  is returned. A disarmed timer is armed again by the next observation, which
  the tick's own tail performs.
- **Freezing** (B-9): pause stores `remaining = deadline - now` (saturating),
  unpause sets `deadline = now + remaining`; `turn_timer_remaining_ms()` is
  the frozen remainder when paused, else `deadline - last accepted now_ms`
  (saturating), `None` when disarmed. A private timer epoch counts arm,
  disarm, freeze and resume of an armed timer only, so a pause with no timer
  changes nothing; T141d's host-facts image reads it.

`HostError::InvalidSettings` is the new variant; its `Display` is
`InvalidSettings at host/admission`.

## Protocol v3 sessions (T141c)

`Host::bind_v3_peer` binds a v3 session: always a snapshot session, speaking v3
whatever `HostConfig.protocol` says; `bind_peer` and `bind_snapshot_peer` keep
the host-wide v1/v2. The session holds a private `V3Session` (`host_v3.rs`): the
epoch's `InterestView` and `FactView`, the operator's `FactGrants`, the viewer's
`PauseRights` and a paced outbox. Nothing of `World` is stored: every staging
reads it afresh into the two views' contexts (`read_world`), and ids come from
the `InterestView` (key = `World::area()` or `HOST_UNBOUND_AREA`, plus `EntityId`).
The `order` fact follows the control list.

Staging (bind, narrowing `set_grants`, upstream `Resync`) is atomic: plan on
clones of the views (`Cancel` if a transfer is in flight, the `0x82` plan at
cursor `next_event_seq - 1`, one `[Clock, HostFacts]` delta that takes its
`event_seq` when built), check the ordinal room and the outbox bound (90 entries,
2,852,148 bytes), then commit. The plan goes to the outbox, which is not egress;
pacing moves entries into the delivery log, in order, while the per-peer (128
frames, 2 MiB) and host caps allow, at the end of staging, in `take_delivery*`
and after `acknowledge_delivery`. A bind therefore never returns `QueueFull`.
Until T141d, v3 sessions were skipped by `accepted_room` and received no v2 frames
and no pages; publication and the v3 intent pipeline are the next section. v3
sessions are still skipped by `accepted_room`.

## v3 publication (T141d)

**Intents.** A v3 session's staged bytes decode with `codec_v3::decode_intent` (the
session's codec, also for the `Blocked` head's `seq`) into one common `Frame`; a
v1/v2 verb maps into the v3 vocabulary, so one pipeline serves both. Steps 1 to 4
of the T022 pipeline are unchanged. Step 5 resolves the actor through the session's
`InterestView` (mapped, in the world's area, `present && action_actor`, controlled,
else `NotAuthorized`), then the body: `DeclareAction` as T022 (target `present &&
action_target`), `EndTurn`, `Stop`, `Pause` and `Unpause` (their right, else
`NoPauseRight`), `RequestMode` (the `pause` right, else `NotAuthorized`). A sim
action is judged by `validate_action`; `RequestMode` reserves the accepted class and
runs `HistoryWorld::request_mode`, whose refusal commits a rejection under that
reservation. `Pause` and `Unpause` change host state (reason `Player`, the timer
frozen or resumed) and journal nothing. Every `Applied` intent goes through
`commit_accepted`, so it makes a capture record and moves `executions()`.

**Codes.** `code(CombatError)` maps `ModeUnsupported` 21, `WrongMode` 22,
`StatEvaluation` 23, `EncounterTerminal` 24, `RulesetMismatch` 25, `PlacementInUse`
26, `NotCreature` 27, `AlreadyParticipant` 28, `NoEntrants` 29, every other error
`IllegalAction` 18; v1/v2 sessions keep 18 for all. The retry cache stores the exact
`protocol_v3::ReceiptStatus` for every session (a v1/v2 status converts by `From`);
a hit re-sends the exact code. `AdmissionResult.status` and `cached_status` keep the
frozen v1 type and report 21 to 29 as `IllegalAction` and 30 as `NotAuthorized`;
`cached_status_v3` returns the exact status (`UnknownPeer`, then `NotV3Peer`).

**Pages.** After every operation one page per v3 session, ascending handle: an
accepted peer command (its events, the receipt to the originator), every host
operation (also a zero-event one: an RT order, a pending mode, a tick's clock),
`pause`, `unpause`, `set_settings`, and to one session `set_fact_grants`,
`set_pause_rights`, a widening `set_grants` and `set_control`. The page is built on
clones of the session's two views after auto-pause and the timer, so its `Clock` and
`HostFacts` are current: `[receipt?] ++ enters ++ events ++ rest of the page (health,
turn, removals, then fact ops) ++ [Clock?] ++ [HostFacts?]`, cut only by
`codec_v3::frame_spans`, one `event_seq` per span. `Clock` and `HostFacts` are added
when they differ from what the session was last sent; the host-facts image holds the
rights, the toggles if granted and the timer epoch if granted, so the countdown
alone never re-sends it. Event candidates (`candidate_v3`) resolve ids against the
view after the page, else before it: the six v2 events by the v2 predicates,
`RoundStarted` and `ModeChanged` by `turn_round` and the mode grant, `OrderEnded`
only for a controlled, present actor, `Joined` and `Left` by `present && spawn` and
`present && despawn`, `Moved` withheld. Candidates are projected in chunks of 256.

**Room, overflow and failure.** A page that fits the session's egress room is logged
and its clones committed; otherwise the session is restaged (`Superseded`) and the
page dropped, so v3 never blocks admission and a rejected v3 intent never returns
`Blocked` (its receipt is logged if it fits and dropped if not, the client's retry
recovers it). Every fallible step of every session is decided before any ledger
moves: a page or restage that cannot be built fences that one session (as
`unbind_peer`) and never fails the operation, which has already committed. A fenced
originator is skipped by the cache and sequence update, and its record is appended
without its view. A v3 session that gets at least one frame for an accepted command
adds a capture view, named by its own ids.

## v3 timing (T141e)

**Heartbeat.** At every `tick` publication of a running host (`HostOp::Tick`, not
paused), a session whose `server_tick - last_clock_tick` is at least
`POLICY_CLOCK_HEARTBEAT_TICKS` (100) gets `Clock` at the end of that tick's page,
even when the page is otherwise empty. `last_clock_tick` is the server tick of the
last `Clock` the session was sent (a staging's delta, a changed `Clock`, a
heartbeat), so a pause or unpause page restarts the count. A paused host does not
tick through `advance`, and a direct `tick()` on a paused host sends no heartbeat.

**Throttle.** A staging installs a `Throttle` on the session (`host_v3.rs`) that
lasts until `Published` ends the transfer. While it is set a publication at server
tick `t` is logged only if `t >= last_page + POLICY_V3_THROTTLE_TICKS` (10;
`last_page` is the staging tick, then the tick of the last logged frames) and the
frames keep the delta frames and bytes logged since the staging (C-6's frame
included) within `POLICY_V3_IN_FLIGHT_FRAMES` (128) and `POLICY_V3_IN_FLIGHT_BYTES`
(2 MiB), the receiver's held-delta budget. A publication that is not allowed logs
nothing and leaves both views and the sent host facts as they were, so the next
allowed page carries every change; a page the budget refuses also ends the deltas
until the transfer ends. An allowed page carries no event ops and no receipt: a
download window loses notices (Question 4 (a)), never state. A rejected-intent
receipt and a cache-hit receipt are single delta frames under the same gate, so a
dropped one is recovered by the client's retry at an allowed tick. A new staging
(`Resync`, narrowing, overflow) restarts the throttle; other sessions are unaffected.

## Navigation and trusted movement (T037a)

The host now owns a navigation mesh. `Host::bind_nav_mesh` pairs a validated
`crpg_nav::NavMesh` with the revision sim validates routes against (H1);
`bind_area_bounds(min, max)` bakes that mesh from an area's two bounds and
`nav_revision()` reads the bound revision. Binding computes
`NavRevision::of_bytes(mesh.to_canonical_bytes())`, calls `history.bind_nav`
(H1, M4: a same-revision rebind changes nothing; a new revision clears every
route in ascending `EntityId`), then places every mover whose position
`locate`s to no polygon at its `nearest_on_mesh`, also ascending, and keeps
`HostNav { mesh, revision }` as host state. `NavBinding` reports the revision,
the cleared `(entity, tag)` pairs and the placed entities. The mesh is the
host's, not the checkpoint's: sim saves the revision (so routes reload
validated) but a restored host has `nav_revision() == None` until the
embedding binds the mesh again, and until then every route request is
refused.

- **Mesh input (Reading 2).** A mesh, or the area's two bounds;
  `area_mesh` is `bake_box(min[1], NavRect { min[0], min[2], max[0], max[2] },
  &[])`, so the floor is `min.y`. A failed bake is
  `HostError::NavBake(BakeError)` with nothing changed. There is no
  `crpg-data` edge.
- **One route computation (Reading 6).** `nav::plan_route` is the one
  function behind `move_to`, `halt`'s sibling requests, `host_move` and (later)
  `DeclareMove` and lane-1 `MoveTo`: a bound mesh (`NoNavMesh`), the actor's
  presence (`Move(AbsentEntity)`), its mover (`Move(NotMover)`), then
  `nearest_on_mesh(snap(measure, destination))` as the goal and
  `find_path(mover.position(), goal)`; the path's points are copied whole.
  `measure` is the encounter definition's `TurnMovement.measure` when it has
  one, else `Measure::Euclidean`; `snap` is the identity in v1. It reads only.
- **Off-mesh movers (Reading 5).** Only a binding places. A mover created
  later (spawn, join, area transfer) is the caller's to `place` (M6), and a
  route request from an off-mesh mover is refused (`StartOffMesh`), never
  silently teleported.
- **Route tags (Reading 7).** A trusted host route is `HOST_ROUTE_TAG =
  RouteTag(0)`; lane-0 and lane-1 intents start at 1, so a peer's
  `RouteTag(seq)` never collides with it. The tag is hashed with the route, so
  it is a constant, not a counter a restore would lose.
- **Trusted operations (Readings 8).** `move_to(actor, destination)` sets a
  route and returns the tag it replaced; `halt(actor)` clears one and returns
  its tag; `place(entity, position)` sets a position and clears the route; all
  three are zero-event operations with no record and no publication.
  `host_move(actor, destination)` is the turn-based move (W4, W7): it
  reserves one host record (`CaptureFull`), runs the `EndTurn { actor }` turn
  check (`Combat`), plans the route, runs `validate_move` (a `Combat` refusal
  is `Combat`, any other `Move`), performs it and ends in the T141 host-record
  tail with `HostOp::Action`, so its one `Moved` envelope is captured and
  published to v3 sessions. It never moves `executions()` and sends nothing to
  a v1/v2 session.
- **The revision is not on the wire yet (Reading 4).** `nav_revision()` is
  the trusted announcement until a `crpg-net` task adds one; no v4 op carries
  it.

Refusals are `HostError::NavBake(BakeError)`, `NoNavMesh`, `NoPath(NavError)`
and `Move(MoveError)`, appended after `NotV3Peer` in that order with the
usual `Display` (`<Variant> at host/admission`) and no `source()`. The two
internal texts are `Invariant("nav binding refused")` and
`Invariant("nav placement refused")`; both are unreachable for a live mover
and a validated mesh.

| Refusal | Result |
|---|---|
| bake of the area bounds fails | `Err(NavBake(e))`, nothing changes |
| route request with no mesh bound (trusted) | `Err(NoNavMesh)` |
| `find_path` fails (trusted) | `Err(NoPath(e))` |
| sim refuses `place`, `set_route`, `clear_route`, `validate_move` (trusted) | `Err(Move(e))` |
| `host_move` turn check | `Err(Combat(e))` |

## Protocol v4 sessions (T037b)

A v4 session is a v3 session plus a v4 state (Reading 1): `V3Session` carries
`v4: Option<Box<V4State>>` (`None` for a v3 session), and every v3 path —
binding, staging, pacing, throttle, pages, receipts, the retry cache,
`cached_status_v3` — serves it unchanged. Only the points that produce or read
bytes branch on `v4`: the intent codec, the receipt frames, the page and the
staging page. `bind_v4_peer` is `bind_v3_peer` with the same precedence and
errors, plus `V4State { positions: PositionView::new(SessionEpoch(epoch)),
withheld: [] }`; `session_wire` is `4`. Lane-0 intents decode with
`codec_v4::decode_intent` and are cached as `codec_v4::encode_intent` bytes; a
decoded `DeclareMove` resolves its actor like any other v4 intent and is a
move request (T037c). A v1/v2/v3 intent body maps
into the v4 vocabulary in one `Frame`, so the pipeline below `decode` has one
shape.

- **Positions (HM-6, Reading 16).** `V4State.positions` is the session's
  `PositionView`, and `position_rows(world, grants, withheld)` is one
  `PositionRow` per disclosure-grant entry, in stored order, whose entity is
  present, live and has a mover: its key, `granted` exactly when the operator
  has not withheld that entity from this viewer, its raw point and
  `moving = mover.route().is_some()`. The withheld list is per session, empty
  by default, so the default is HM-6's "presence minus nothing"; no authoring
  setting or perception rule is implied.
- **Pages (B-5, B-6).** A v4 page is the v3 layout built by
  `project_page_v4` on clones of the interest, fact and position views, with
  B-4's rows: `[V3(receipt)?] ++ enters ++ events ++ rest ++ [V3(Clock)?] ++
  [V3(HostFacts)?]`, the `Position` ops being the page's tail, cut by
  `codec_v4::frame_spans`. Committing the page installs all three clones; a
  held page changes none. Each v3 event candidate is wrapped as
  `EventCandidate::V4::V3`; a `HistoryEvent::Moved` becomes
  `EventCandidate::Moved` with the post-page id, its raw points, and
  `disclose` exactly when the same page's rows carry a granted row for that
  entity. `Moved` never reaches a v3 session's page.
- **Staging (HM-7, Reading 18).** A v4 staging projects the snapshot with
  `project_snapshot_v4` on the same clones, then builds the anchoring page:
  `project_page_v4` with B-4's rows followed by `[V3(Clock), V3(HostFacts)]`,
  cut by `codec_v4::frame_spans`, one `event_seq` per frame. The whole plan
  must fit `OUTBOX_MAX_ENTRIES_V4` (94) and `OUTBOX_MAX_BYTES_V4` (2,880,916),
  else `Invariant("v4 plan exceeds the outbox bound")`; the commit queues
  every frame after the transfer frames, moves `next_event_seq` past the last
  and restarts the throttle with all of them counted. A v3 staging is still
  the one `[Clock, HostFacts]` delta.
- **Withholding (B-8).** `set_position_withheld(peer, entities)` checks
  `Closed`, `UnknownPeer`, `NotV4Peer`, then `InvalidGrants` (more than
  `MAX_GRANT_IDENTITIES` entries or a duplicate), stores the list sorted and
  publishes to that session alone (`publish_v3(Some(peer), &[])`). Every
  immediate consequence of the change is a `Position` op — a withdrawal for a
  newly withheld entity, a fresh anchor for a re-granted one; an unchanged
  list publishes a page with no ops and therefore no frame. `Moved` follows
  the grant in the same page, so revoking after disclosure stops further
  positions at once.
- **Older wires unchanged (B-9).** A v1, v2 or v3 session has no `V4State`,
  receives no `Position` or `Moved` op and keeps byte-identical entries and
  capture views whether or not a v4 session shares the host.

## The turn-based move and samples (T037c)

`DeclareMove` is the turn-based player's move on lane 0 (H3, HM-9), and every
committed tick publishes each v4 session's movement samples (H4, HM-5).
Neither changes a v1–v3 session's bytes.

- **Resolution and class (Readings 19).** A v4 `DeclareMove { destination }`
  resolves its actor exactly as T141 D-2 (`InterestView`, `present &&
  action_actor`, the control list; else `Rejected(NotAuthorized)`), then is a
  move request for that actor with the raw `Fx16_16` destination. The class
  checks run in order: `validate_action(EndTurn { actor })` (a refusal is
  `code(e)` and sets `last_combat_error`), `plan_route(.., RouteTag(seq))`
  (any refusal is `IllegalAction`, `last_combat_error` unchanged), then
  `validate_move` (`Combat(e)` as the turn error, any other `MoveError`
  `IllegalAction`). The turn check precedes navigation on purpose: an
  out-of-turn player gets the turn error, not a spent query. Every terminal
  refusal goes through the ordinary `reject` path, so the sequence is
  consumed and one receipt is logged; lane 0's own rate window still applies
  and the lane-1 per-window query cap does not.
- **Execution (C-3).** An accepted move reserves the accepted class, runs
  `history.perform_move` and ends in `commit_accepted` with evidence
  `CombatAction::Stop { actor }` and no returned outcome: the
  `CapturedRecord` carries `outcome: None` and the one `Moved` envelope,
  `executions()` moves by one, the retry cache takes the lane-0 entry, and
  every v3/v4 session gets its page (v1/v2 sessions get the permitted legacy
  state ops only; `Moved` has no v2 form). The retry and dedup rules are lane
  0's: identical bytes never move twice.
- **Samples (C-5).** After every committed tick — `tick` step 4 and
  `advance`'s tick, whether the record/publication tail returned `Ok` or an
  `Invariant` — every v4 session, ascending by handle, computes
  `position_rows` and `project_samples(view, positions, rows, refresh)` with
  `refresh = server_tick % POLICY_SAMPLE_REFRESH_TICKS == 0` (20 ticks, one
  second at 20 Hz), then `movement_v4::sample_datagrams(server_tick,
  processed_seq, &entries)` (T037d's `MoveLane.processed_seq`; 0 before any
  lane-1 receipt). The queue **replaces**
  the previous tick's datagrams: the old ones are discarded and their egress
  freed first, and the new ones are queued only if all of them fit the
  session's egress room; otherwise none is (datagram loss, repaired by the
  next change or refresh). Only positions the viewer was anchored are
  sampled, and a withheld or ungranted row is skipped, so revocation is
  immediate. Samples never enter the delivery log and consume no
  `event_seq`; a refused tick, a paused `advance` and every other operation
  produce none.
- **Reading them (C-6).** `take_samples(peer)` checks `UnknownPeer` then
  `NotV4Peer` and returns the queue, emptying it and freeing its egress; a
  closed host has no sessions, so it is `UnknownPeer`.
- **Lane-1 egress (C-7, Reading 23).** `V4State::lane1_egress` counts the
  session's lane-1 frames and bytes — its sample datagrams, and from T037d its
  released untaken receipts and one reservation per unreleased slot — and the
  host's `egress_bytes` includes those bytes. `room_for` and `pace_session`
  test `log.len() + lane1_frames` and `log_bytes + lane1_bytes`; a v1–v3
  session counts zero, so its checks are unchanged. A fence, `unbind_peer` and
  `shutdown` release lane-1 egress with the session. Lane 1 never fails on
  those caps after admission, and taking a sample frees its room.

## Lane 1 (T037d)

Lane 1 is the real-time and exploration movement lane (HM-2, HM-3; Readings 9,
10, 15). `Host::ingest_move(peer, bytes, now_ms)` admits at most the session's
next intent, synchronously, and `advance` decides every admitted intent once per
call. Lane 1 keeps its own sequence, cache, move bucket and receipt queue per
session (HM-10, Reading 22); only the shared intent and failure buckets are
shared with lane 0.

- **Admission before HM-2 (D-1).** `Closed`; `TimeRegression` (then `now_ms` is
  recorded); an unbound handle is `Refused { None, Unauthenticated }`; both
  shared intent buckets consume (`RateLimited`); the 4,096-byte frame cap
  (`FrameTooLarge`); then per-peer and host ingress room (`QueueFull`). Nothing
  is staged: a lane-1 frame spends the session's intent buckets per attempt but
  never occupies the lane-0 FIFO (Reading 9).
- **HM-2 (D-2).** After D-1: the session must be v4 (`UnsupportedVersion`),
  the frame decodes with `movement_v4::decode_intent` (its code is
  `move_codec_code`), its epoch must match (`SessionExpired`), and a `seq` that
  is a slot or a cache entry is a duplicate (D-4) or, with other canonical
  bytes, `SeqConflict` **and a fence**. Then the sequence range
  (`SeqExhausted`, `SeqGap`, `StaleSeq`), the session's move bucket
  (`TokenBucket::new(40, 20)`, `RateLimited`), the in-flight window
  (`unreceipted + 1 > POLICY_MOVE_IN_FLIGHT`, 8) and the session's egress room
  for one `MAX_MOVE_RECEIPT_BYTES` (60) reservation (`QueueFull`). Steps 1–8
  consume no sequence and queue no receipt; the buckets keep what they spent.
  Step 9 resolves the actor with T141 D-2's rule (the `InterestView`,
  `present && action_actor`, the control list): without one the sequence is
  consumed and a slot decided at once as `Rejected(NotAuthorized)` is appended;
  otherwise `next_seq = seq + 1` and a pending slot takes the next host-wide
  admission order.
- **The decision window (D-5, D-6, D-7; Readings 12–14).** `advance` runs the
  window after recording `now_ms` and before the timer, paused or running, at
  `server_tick()`; `tick` never decides. Pending slots are taken in host-wide
  admission order and, for each actor, only the last admitted one is decided —
  every earlier one is `Superseded`, whichever session sent it. A winner that
  is a combatant of a turn-based encounter is `Rejected(WrongMode)`; `MoveTo`
  plans its route with `plan_route(.., RouteTag(seq))` (any refusal
  `IllegalAction`) and sets it (`TurnBased` is `WrongMode`, any other refusal
  `IllegalAction`); `Halt` clears it (any refusal `IllegalAction`); else the
  outcome is `Accepted { start_tick: t + 1 }`, so the route starts in the stage
  4 of the next tick in the same call when running, or on the first tick after
  unpause. Decisions journal nothing, make no record and publish no lane-0
  frame (Reading 21).
- **Release and the cache (D-3).** Whenever a slot is decided, the session
  releases from the front while the front is decided: the reservation becomes
  the receipt's own length, the encoded
  `MoveDown::Receipt(MoveReceipt { epoch, seq, actor, processed_tick, outcome })`
  enters the receipt queue, `processed_seq` advances, and the slot moves into
  the lane-1 cache (256 entries or 2 MiB of canonical plus receipt, oldest
  evicted first). Receipts leave in `seq` order, one per consumed sequence. A
  `Rejected` receipt spends the shared failure buckets and is never dropped by
  them (Reading 15), unlike a lane-0 rejection.
- **Duplicates (D-4).** A pending or held slot, or a cached entry whose receipt
  is still queued, is `Duplicate { resent: false }`. A cached receipt is queued
  again only if the session has egress room for it (`resent: true`), spending
  the failure buckets when it is a rejection; without room it is
  `resent: false`.
- **Receipts and lifecycle (D-8 to D-10).** `take_move_receipts(peer)` checks
  `UnknownPeer` then `NotV4Peer` and returns the queue in release order,
  emptying it and freeing its egress. The queue is bounded by the egress frame
  cap (128) through the same `room_for` checks. `unbind_peer`, a fence and
  `shutdown` discard the session's lane-1 state; its pending slots are never
  decided and get no receipt.
- **Egress (C-7, Reading 23).** A session's lane-1 egress is its sample
  datagrams, its released untaken receipts at their length and one frame plus
  `MAX_MOVE_RECEIPT_BYTES` per unreleased slot; `room_for` and `pace_session`
  count them beside the delivery log and the host's `egress_bytes` includes
  them. Lane 1 never fails on the caps after admission: a slot's room is
  reserved when it is admitted, so its receipt is never dropped.
- **Samples (D-12).** A sample's `processed_seq` is the session's
  `MoveLane.processed_seq`, the highest lane-1 `seq` receipted to it.

## Connections and bindings (T023b)

A QUIC connection has no gameplay identity until the adapter binds it. The
adapter keeps two private `BTreeMap`s. The first maps each registered
invitation (32-byte credential, normalized control and disclosure grants,
its live connection, and — on a resumable adapter — the handle of a session
held through its grace, `graced: Option<PeerHandle>`) by a never-reused
`InvitationId`. The second maps each mapped `ConnectionId` to either a
bound entry (`PeerHandle`, epoch, acknowledged delivery watermark, frame
ordinal, at most one held frame, stall start, transport-close state, and a
`fence: ConnectionFence` — `FIRST` for a hello binding, the granted fence
for a resume binding) or a retiring entry still being drained so the
transport can forget it. A connection is mapped only after `accept` or
`accept_resume` succeeds. One invitation has at most one bound connection,
and a newer one supersedes it; a resumable adapter holds a `resumable`
flag (`start_resumable` sets it) that turns on `poll_notice`, the ticket
after a `/2` welcome, the fence tags and the grace. `ConnectionId`s never
enter host state: the host sees only `PeerHandle`s minted in bind order, so
the same calls replayed in memory give the same handles and epochs.

## QUIC v4 sessions (T150c)

The adapter serves `QuicServer::bind_movement` endpoints — `crpg-lane0/3`,
wire version 4, lane 1 plus downstream datagram samples (T150b) — beside the
`/1` and `/2` routes it already served, without changing either. A movement
connection's lane stream is `Framing::V2`, so every lane-0 path keeps its
current shape; `QuicServer::movement(conn)` — not the framing — says whether
lane 1 exists. `QuicHost::start` accepts such a server as it accepts `bind`
and `bind_dual`.

- **Invitations.** `register_v4_invitation(V4Invitation)` stores the fact
  grants and pause rights `Host::bind_v4_peer` needs beside the control and
  disclosure grants; the record's private `v4` slot is `None` from
  `register_invitation`. Its precedence is `Poisoned` → `InvitationLimit` →
  `InvalidCredential` (length ≠ 32) → `Host(InvalidControl)` →
  `Host(InvalidGrants)` (disclosure grants by `normalize_grants`, then fact
  grants by `normalize_fact_grants`) → `DuplicateCredential`; both kinds
  share the cap, the id sequence and the duplicate check, and revocation,
  grant/control updates and `invitation_binding` work on both.
- **Hello precedence (C-2).** Credential match (`AuthRefused`) → the
  connection's movement flag (`Lost { EndpointFailed }` on an error) →
  version (a movement connection needs `MOVEMENT_WIRE_VERSION` 4; others
  keep `host.protocol()`) → a movement connection whose matched record has
  `v4: None` is `AuthRefused`, with no supersession and no bind →
  supersession as today → bind: a movement connection calls `bind_v4_peer`
  and traces `Lane1Call::BindV4Peer` in `lane1`, never in `calls`, while
  other connections trace `HostCall::BindPeer`/`BindSnapshotPeer` as before
  → the welcome carries version 4 on a movement connection → `accept` and
  the `Bound` entry record `movement`.
- **Lane-1 drain (C-3, phase 2).** After the unchanged `drain_bound`, a
  still-bound movement connection retries one held frame and then takes at
  most `MAX_DRAIN_PER_CONNECTION` frames with `try_recv_lane1`, each traced
  as `Lane1Call::IngestMove { conn, peer, frame, len, result }` with
  `frame` the 1-based stream ordinal (a held retry repeats it). `Admitted`,
  `Rejected` and `Duplicate` continue; `Refused { QueueFull }` holds the
  frame and stops that connection's lane-1 drain; `Refused {
  Unauthenticated }` counts `lane1_refused` and fences `EpochClosed`; any
  other refusal counts and continues. `Ok(None)` stops; `Closed` and
  `UnknownConnection` record the transport close (first wins) and mark
  `lane1_drained`; `NotAccepted`/`NotPending`/`InvalidWelcome` fence. The
  two lanes never gate each other, and a host error returns from the pump
  and poisons.
- **Lane-1 delivery (C-4/C-5, phase 5).** After the unchanged `deliver`, a
  movement binding whose transport is open takes receipts only when
  `lane1_stats(conn).outbound_frames == 0` (one take then fits the 128-frame
  transport cap), traces `TakeMoveReceipts`, and sends each in order with
  `try_send_lane1` (`lane1_sent`); a closed/unknown send records the close
  (first wins) and this and later receipts count `lane1_discarded`. Samples
  are taken every pump, traced as `TakeSamples` and sent once with
  `try_send_datagram` (`datagrams_sent`) — never held, never retried. A
  departed peer's receipts and samples are taken (both traced), discarded
  and counted, so they never hold host egress.
- **Retire and fence (C-6/C-7).** A movement binding retires only when its
  transport is closed, lane 0 is drained, `lane1_drained` holds and neither
  hold is set; `drain_retiring` also drains and discards lane-1 frames
  (`lane1_discarded`), and the record leaves once both lanes report an error
  and its `Closed` was polled. A fence discards a held lane-1 frame too.
  On `/1` and `/2` `try_recv_lane1` reports `Ok(None)` then the lane-0
  error, so retirement is exactly today's.
- **`shutdown` (C-8).** After the lane-0 flush, each open movement binding's
  released receipts are taken once and sent until the first error;
  `undelivered_receipts` counts the rest. No sample is taken.
- **The side trace (Reading 13).** `QuicPumpReport.lane1` holds
  `Lane1Trace { at, call }`; `at` is `calls.len()` when the call was made,
  so a replay interleaves both lists exactly (equal `at`s keep list order),
  and the existing `calls` replay stays exhaustive. The report adds
  `lane1_sent`, `datagrams_sent`, `lane1_refused`, `lane1_discarded` and
  `lane1_held` (bound connections holding a refused lane-1 frame at the end
  of the pump).
- **`advance` (C-9).** `QuicHost::advance(now_ms)` is an untraced
  pass-through to `Host::advance`: `Poisoned` → `TimeRegression` (below the
  adapter's last accepted `now_ms`; nothing changes) → record the time →
  the host call, whose error goes through `host_error` (an `Invariant`
  poisons). Lane-1 intents are decided only there.

## Identity and delivery shape

A session epoch is `incarnation` (LE) followed by the never-reused handle
(LE), so correlation `(epoch, lane, seq)` is globally unique across rebinds
and restarts. Per peer, the host keeps a live `EntityId → NetId` map minted
in ascending `EntityId` order (revoked ids retire; re-granted entities get
fresh ids), a lane-0 retry cache of canonical intent bytes plus fixed-size
outcome metadata, failure-response budgets, and a delivery log. Receipts go
to the originator only; permitted events (V2) and state ops fan out per
peer. V1 hosts deliver receipts plus legacy state ops only; V2 hosts add the
ordered event ops. Narrowing a `bind_peer` session restarts its delivery at
1 with a fresh filtered state (`RebindRequired`); narrowing a snapshot
session stages a snapshot transfer above the current cursor
(`SnapshotStaged`) and keeps `event_seq` and ordinals continuous. Widening
continues incrementally in both kinds.

## Host read API (T047a)

`Host::legal_intents(peer)` and `Host::replica_id(peer, entity)` are the two
read-only trusted methods ADR-0033 §7 fixes so a player-only option list and a
replica-id table need no second legality source. Both are `&self`, add no host
state, and cannot move the authority, the clock, a ledger or delivery.

1. **Prelude.** Both return `Closed` first, then `UnknownPeer`. `shutdown`
   clears sessions before setting `closed`, so after it every handle — bound
   or not — is `Closed`.
2. **Active head.** `legal_intents` reads
   `world().combat().and_then(|c| c.active)`; no encounter, terminal state and
   real time all give `Ok(vec![])`.
3. **Eligibility.** The head must be in the session's sorted control list,
   mapped in its live `EntityId → NetId` map, and granted
   `present && action_actor`; otherwise `Ok(vec![])`. Its mapped id is
   `actor_net`.
4. **One enumeration.** `crpg_sim::legal_actions(world, active)` is called
   once and its order kept — no sort, dedup or reorder. `Actor(_)` is
   `Ok(vec![])`; `TooManyOptions` is `Invariant("legal options exceed their
   bound")`; a partial list is never returned.
5. **Filter.** Each sim action goes through `legal_body`, the same
   mapping/ownership/disclosure pair `resolve_action` applies at admission:

| `CombatAction` | Kept when | `IntentBody` |
|---|---|---|
| `UseAbility { ability, target, .. }` | `map[target]` exists and `grants[target]` is `present && action_target` | `DeclareAction { ability, target: map[target] }` |
| `EndTurn` | always (item 3 covered the actor) | `EndTurn` |
| `Stop` | never (no wire verb exists) | dropped |

6. **`replica_id`.** After the prelude, `Ok(map.get(&entity).copied())`. The
   map is the snapshot the last installation left: an entity outside those
   grants, with `present: false`, or not live then is `None`. A dead
   combatant stays `Some`; an entity despawned since installation keeps its id
   until the next bind/`set_grants`. No liveness check is added.

Options are advisory: they confer no authorization, are never cached, and
every submitted intent is revalidated before execution.

## Snapshot sessions (T024h)

`bind_snapshot_peer` opts a session into the `0x81` transfer subprotocol
(ADR-0028) beside lane-0 frames, in one total order. The kind is fixed for
the session's life and both kinds share every other rule. Exact contract:
[T024h](../../tasks/T024h.md) S§5–S§11; reasoning:
[ADR-0031](../adr/0031-host-snapshot-sender-and-snapshot-sessions.md).

| | lane-0 session (`bind_peer`) | snapshot session (`bind_snapshot_peer`) |
|---|---|---|
| First delivery | `full_state_ops` as deltas from seq 1 | the same |
| Narrowing | T022: reset to seq 1, `RebindRequired` | snapshot transfer above the cursor, `SnapshotStaged`, no reset |
| Widening | incremental deltas | incremental deltas |
| `ingest_transfer` | `Refused(NotSnapshotPeer)` | S§9 |
| `transfer_state` | `Ok(None)` | `Ok(Some(..))` |

Every delivery entry carries `ordinal` (from 1, never reused in the
session), `kind` (`Delta` or `Transfer`) and the encoded bytes.
`acknowledge_delivery(through)` takes an ordinal; clearing the log counts
discarded entries as retired (`delivered = next_ordinal − 1`). In a lane-0
session the ordinal equals the entry's `event_seq`, so its path is
unchanged.

Staging is one private routine used by narrowing and `Resync`, and is
atomic and immediate:

1. `next_transfer == u64::MAX` is `CounterExhausted`.
2. If a transfer is in flight, plan `Cancel` (`Aborted` for narrowing,
   `Superseded` for `Resync`).
3. Build the body from `PeerView` (mapped entities, their health, and the
   turn when granted) — only facts granted at capture.
4. `plan_transfer(epoch, next_transfer, server_tick(), cursor, 8, body)`
   gives `Begin` and one `Chunk` per v1 body.
5. Check entries, bytes and host egress room against the cleared log,
   plus the ordinal counter, without mutating.
6. Commit: install the grants and map, clear the log, push
   `Cancel`/`Begin`/`Chunk` as `Transfer`, set `in_flight` and advance
   `next_transfer`.

There is no pending stage. At v1 caps the room check cannot fail
(S§1 item 7) and is kept as defence in depth. `ingest_transfer` decodes
the T024 S§5.2 upstream set (`Published`, `Resync`), spends the shared
intent frame and byte buckets before decoding, and returns a
`TransferDisposition` rather than an error for refusals.

The QUIC adapter routes by kind, never by reading bytes. A hello on a
`crpg-lane0/2` connection calls `bind_snapshot_peer` (`BindSnapshotPeer`);
a `/1` connection calls `bind_peer` as before. Inbound reads use
`try_recv_channel`: `Channel::Lane0` frames go through `ingest`,
`Channel::Transfer` frames through `ingest_transfer`, and the unreachable
`Channel::Resume` arm counts a refusal and fences. Delivery sends each
entry's `DeliveryKind` on `Lane0` or `Transfer` with `try_send_channel`
and acknowledges the last accepted ordinal; a departed peer and the
shutdown flush do the same. Retiring connections drain every channel, so
a transfer frame cannot leak the transport record. A narrowing
`set_invitation_grants` on a live `/2` binding stores the grants and
returns `GrantChange::SnapshotStaged`; on `/1` it still fences
(`Narrowed`). `QuicHost::start` takes a server from `QuicServer::bind` or
`bind_dual`, never `bind_resumable`; `QuicHost::start_resumable` accepts a
server from any `bind*` (`bind_resumable` included), polls `poll_notice`,
follows each `/2` welcome with a resume ticket, fence-tags every inbound
frame, and starts a grace on a silent connection loss (see "Reconnect
grace"). `/1` connections carry only lane-0 frames and keep T023b's
behaviour and trace.

## Reconnect grace (T025b)

The host side of the 30-second reconnect grace: a disconnected snapshot
session is held, without its binding, so the same session (same epoch) can
resume on a new connection. Policy, never wire: `POLICY_RECONNECT_GRACE_MS`
is 30_000 injected milliseconds. Exact contract:
[T025b](../../tasks/T025b.md) (part i, the host side below; part q, the
`quic.rs` adapter, follows).

- **State.** Each `Session` carries a `fence: SessionFence` (from
  `SessionFence::new(epoch)`) and a `proof: Option<ResumeProof>`; the host
  carries `graces: BTreeMap<PeerHandle, Grace>` with
  `Grace { session, deadline_ms }`. A grace holds at most `MAX_PEERS`
  sessions counted together with live bindings, and injected-time expiry
  in `sweep_graces` is its retirement path; proofs and fences never enter
  a checkpoint.
- **Ticket (`issue_resume_ticket`).** Stores the proof on a live resumable
  session and returns its ticket bytes. Precedence: `Closed`, then
  `UnknownPeer` for no live session (a graced handle included), then
  `NotResumable` for a `bind_peer` session or a v4 session. Only
  snapshot sessions (`bind_snapshot_peer`, `bind_v3_peer`) with no v4
  state are ever resumable.
- **Grace (`begin_grace`, `grace_deadline`).** Moves a live resumable
  session with a stored proof into the grace map with
  `deadline_ms = now_ms.saturating_add(POLICY_RECONNECT_GRACE_MS)` and
  returns the deadline. Precedence: `Closed`, `TimeRegression`, then the
  sweep, `UnknownPeer`, `NotResumable` (non-snapshot, v4, or no proof).
  Commit purges the peer's staged commands, discards its delivery log as
  `stage_snapshot`'s commit does, and moves nothing else.
  `grace_deadline` is `Some(deadline)` while held, `None` otherwise
  (`None` on a closed host).
- **Resume (`resume_peer`).** Decides one `ResumeRequest` by epoch, live
  or graced. `Closed` and `TimeRegression` are errors; every later
  refusal is `Ok(Rejected(..))` with `request.epoch` and no state change
  but the recorded time and the sweep. In order: unknown epoch is
  `Denied`; missing proof or a constant-time proof mismatch is `Denied`;
  a failing fence check is `Denied`; planning the resume snapshot
  read-only is `Busy` on `QueueFull`, `Denied` on `CounterExhausted`,
  `Err` on `Invariant`. Commit moves a graced session back to live,
  stages the resume snapshot (`SnapshotCause::Resume`, cancel reason
  `Superseded`) and returns `Accepted { peer, accepted }` with the epoch,
  the granted fence and lane-0 progress (`finalized_through` is the last
  new sequence). A refusal moves neither fence nor deadline.
- **Fence (`check_frame`).** Confirms one frame's connection tag against
  the session's current fence: `Closed`, then `UnknownPeer` for no live
  session (a graced handle included), then `SessionFenced` on mismatch.
- **A grace is not a binding.** A graced handle answers every per-peer
  call as an unbound handle and gets no publication, receipt or capture
  post-state; only `unbind_peer` (drops it), `grace_deadline` and
  `resume_peer` see it. `unbind_peer` also drops a grace, every `bind_*`
  counts both maps against `MAX_PEERS`, `pump` sweeps, `shutdown` clears
  graces, and `session_count` counts both. A swept session is gone as by
  `unbind_peer`: its handle and epoch retire for ever.

The QUIC route (`start_resumable`) turns the host mechanism into the `0x91`
wire, with `start` left byte-for-byte unchanged:

- **Start.** `start_resumable(host, server, now)` has `start`'s precedence
  (`HostClosed` → `HostInUse` → `TimeRegression`), `HostInUse` counting
  graces; `is_resumable()` reports the flag. A resumable adapter polls
  `poll_notice` and handles `ServerNotice::Resume` in phase 1.
- **Ticket (N-3).** After a `/2` hello's `accept` succeeds and the binding
  is recorded, the adapter calls `issue_resume_ticket(peer,
  resume_proof(credential, epoch)?)` and sends the bytes on
  `Channel::Resume`: the first frame after the welcome. A `/1` binding gets
  no ticket, and a `start` adapter sends none.
- **Fence tags (N-4).** A hello binding holds `ConnectionFence::FIRST`, a
  resume binding its `granted`. In phase 2 each fresh frame (not a held
  retry) is `check_frame`d before routing: `SessionFenced` retires the
  connection with cause `Resumed`, `UnknownPeer` fences it `EpochClosed`,
  and any other error poisons.
- **Resume (N-5).** One `ServerNotice::Resume` is decoded, decided by
  `resume_peer`, and answered: a `Rejected` closes at once with code 5
  (`AuthRefused`, for `Denied`) or 7 (`ServerBusy`); an `Accepted` retires
  a still-bound old connection with `retire_resumed` (no unbind), encodes
  the answer, `accept_resume`s and binds the connection with `fence =
  granted` and `Framing::V2`. The staged snapshot is delivered in phase 5,
  after the answer.
- **Grace on a silent loss (N-7).** Phase 6 starts a grace only for a
  closed, drained, unheld `/2` binding whose reason is `IdleTimeout`,
  `Transport` or `Reset`: `begin_grace` (traced `BeginGrace`), `Ok` →
  `record.graced = Some(peer)`; `NotResumable` → unbind as today. Every
  other binding unbinds exactly as today. Phase 4 also clears any
  `record.graced` whose `grace_deadline` is `None` (the host swept it).
- **Ending a grace (N-8).** A hello matching an invitation whose
  `graced == Some(g)` unbinds `g` first (before supersession), and
  `revoke_invitation`, `set_invitation_grants` and `set_invitation_control`
  do the same, untraced.
- **Proof (N-10).** `resume_proof(credential, epoch)` is the 32-byte
  `blake3::keyed_hash(&credential, b"crpg-resume-proof/1" ++ epoch)` (the
  credential must be exactly 32 bytes); the `blake3` edge is approved by
  D68 Question 1 (b). No proof bytes reach a `HostCall`, report, `Debug` or
  error.

## The run loop (T154)

T154a adds `run`, the module that actually runs the world. Nothing ran it
before: `Host::advance` was called only by tests and the dedicated binary
still refuses to start (T040). `HostLoop` owns the one cycle order and the
20 Hz grid; `LocalHost` is the in-memory hosting that carries a local
player's lane-0 and transfer frames to the one `Host`. The thread and clock
that drive the loop are T154b's, lane 1 on the local link is T154c's, and
the QUIC hosting (`QuicHosting`) is T154d's. Exact contract:
[T154](../../tasks/T154.md).

- **One order, one function.** `HostLoop::run_cycle` calls `intake`,
  `drive_ai`, `advance`, `deliver` and `retire_captures`, once each, and
  returns at the first error, so no later stage runs. `intake` is every
  waiting frame and then one `Host::pump`; `drive_ai` is the hosting's
  `AiHook`, if set; `advance` is the pause-aware driver; `deliver` hands each
  peer its delivery and acknowledges what was accepted; `retire_captures` is
  `acknowledge_captures(last_capture())`. The order is the spec's
  submit/AI/advance/publish, with the AI driver after `pump` and before
  `advance` (T153).
- **The grid.** `TICK_PERIOD_MS` is 50 (20 Hz); `HostLoop::new(h, s)` puts
  the first slot at `s` and `with_period` changes the period. `step(now)`
  reads no clock and never blocks: a `now` below the last accepted one is
  `LoopError::TimeRegression` and changes nothing; a `now` before the next
  slot runs no cycle; otherwise it runs exactly one cycle, counts
  `k = (now − next_due_ms) / period` skipped slots in `missed` and moves
  `next_due_ms` to the first slot after `now`, so game time slows under
  overload and never bursts. `cycles` counts the cycles attempted, a failed
  one included; the schedule and counters have already moved when its error
  returns.
- **The hosting seam.** `Hosting` is the five stages with one error type.
  `LocalHost` owns the `Host`, a `BTreeMap<PeerHandle, Link>` (ascending
  handle order in every pass), the retired-link records and an optional
  `AiHook`; the loop names no transport and no AI type. A later task adds
  `QuicHosting` behind the same trait.
- **Local links (B-6, B-7, B-9).** `attach` refuses an unbound handle
  (`UnknownPeer`) before an existing link (`AlreadyAttached`) and makes two
  bounded `sync_channel` queues of whole frames; `LocalClient` is the other
  end (`Send`, not `Clone` or `Sync`). Intake retries a held frame first,
  then takes at most `MAX_DRAIN_PER_CONNECTION` (128) fresh frames per link:
  a `QueueFull` holds the frame and stops that link's drain, a transfer
  frame (`0x81`/`0x82`) routes to `Host::ingest_transfer`, any other frame
  to `Host::ingest`, `Unauthenticated`/`Unbound` retire the link, other
  refusals are counted and dropped, and one `Host::pump` follows every link.
  Deliver offers the host's entries in order until the down queue is full
  and acknowledges the last accepted ordinal, so unaccepted entries stay
  and are offered first next cycle. A link ends only by retirement — a
  departed client, an unbound peer, or `UnknownPeer` at deliver — which
  unbinds the peer, drops both server ends and records the link's final
  counters in a bounded list (`MAX_RETIRED_RECORDS` 64, oldest dropped
  first and counted in `retired_dropped`); `take_retired` drains it.
  `LinkStats` is `#[non_exhaustive]` (`received`, `refused`, `held`, `sent`;
  T154c adds `held_moves`, the lane-1 hold count) so readers keep compiling
  across the addition.
- **Lane 1 on the local link (T154c, B-11–B-13).** Intake routes a first
  byte of `0xA1` to `Host::ingest_move`: `Admitted`/`Rejected`/`Duplicate`
  continue, `NotV4Peer` is counted and dropped, `QueueFull`/`RateLimited`
  wait at the back of the per-link hold queue, `Unauthenticated` retires the
  link, and any other refusal is counted and dropped. Held lane-1 frames
  retry front first each cycle; a fresh `0xA1` frame behind a hold joins the
  back with no admission attempt, lane-0 frames still route, and the drain
  stops only when the hold reaches the link's capacity. Deliver takes
  receipts into a per-link backlog that is filled when empty and never
  dropped while the link lives, then takes samples that are dropped when the
  down queue is full; the first `NotV4Peer` marks a v1–v3 link as lane-0
  only, so B-12/B-13 never run for it again. `LinkStats::held_moves` is the
  hold queue's length.
- **Pause and state (Readings 4, 14).** Neither the loop nor `LocalHost`
  reads `paused()`: a paused host still takes commands, runs the AI hook and
  delivers; only `advance` declines to tick. The schedule, the links and the
  hook are hosting state, not checkpoint state; `save_checkpoint` reads only
  the host.
- **Fences (Reading 9).** `run.rs` reads no clock, starts no thread, sleeps
  nowhere and uses no blocking call: the client queues are tried with
  `try_send`/`try_recv` and the loop is stepped by an injected `now_ms`.
  T154b's `runner.rs` owns the one thread and the clock.
- **The background runner (T154b).** `runner.rs` is the one module that reads
  a clock, owns a thread or blocks. `Clock` is the injected millisecond
  source (never decreases); `SystemClock` is the wall-clock one,
  `starting_at(origin_ms)` so the clock starts at the loop's `start_ms`
  (Reading 13). `spawn` runs the loop on the `crpg-server-loop` thread —
  step, run the pending closure, wait for the next slot — and `RunnerHandle`
  is the rendezvous: `submit`/`call` run a closure after the next step at
  `now >= t`, `stop` ends the runner with no further step, `is_finished`
  reports the thread, and `join` returns the loop with its links and schedule
  (so the later switch respawns it, Reading 10) while dropping an unjoined
  handle stops and joins the same way. A step error ends the runner and drops
  the pending closure (`RunnerGone`); a thread panic reports
  `RunnerPanicked`. Exact contract:
  [T154](../../tasks/T154.md).
- **The dedicated hosting (T154d, B-14–B-17).** `QuicHosting` is the same
  five stages over one `QuicHost`: `intake` is `pump` (report discarded),
  `drive_ai` the optional `QuicAiHook`, `advance` the adapter's own,
  `deliver` is `flush`, and `retire_captures` acknowledges through
  `host().last_capture()`. `QuicHost::flush(now)` is `pump`'s phases 4–7
  alone — `Poisoned`, then `TimeRegression` (below the adapter's last
  `now_ms`; nothing changes), then record `now_ms`, then reconcile, deliver,
  retire closed peers and report — and `pump` runs its phases 1–3 and then
  the same body, so every existing `host_quic*` byte and call is unchanged.
  `drive_ai`, `pause` and `unpause` are untraced trusted pass-throughs:
  `Poisoned` first, then the host call, an error through `host_error` (an
  `Invariant` poisons), which is how a loaded host takes its `Restore`
  unpause. A single-player save therefore runs on the dedicated loop with no
  conversion (Reading 11): the load is `Restore`-paused with no mesh and no
  hook, the first cycles deliver and report `ticked: false`, and the
  operator's `unpause` resumes the same authority the embedded loop had.
  Reading 13's wiring is `runner::spawn(HostLoop::new(QuicHosting::new(quic),
  now), SystemClock::starting_at(now))`, with `now` the host's load or start
  time; a clock below it ends the runner with `TimeRegression` on its first
  step.

## The MVP AI driver (T153)

`Host::drive_ai(sides, actors, budget)` is the trusted host operation the
embedding's AI hook calls between ticks (Reading 1): it runs
`crpg_ai::decide` for the caller's AI-controlled combatants and turns each
`Decision` into the host's own step — a turn-based move by `host_move`, a
real-time move by `move_to`, an action by `host_action`, or `halt` — within an
`AiBudget`, reporting what it did in an `AiReport` of `AiStep`s. The caller of
the hook is T154's cycle: intake and `pump`, the AI hook, `advance`, deliver
(`run.rs` already names `drive_ai` as its hook stage).

- **One step per call in turn-based play (Readings 2, 5).** A turn-based call
  decides for the active head only, at most once, so the embedding paces the
  steps (the server never does). The turn cap (`AiBudget::decisions_per_turn`,
  16 in `MVP_AI_BUDGET`) counts decisions across calls in a private, never-saved
  `ai_turn { round, head, decisions }`; once the count reaches the budget the
  call runs `host_action(EndTurn { head })` instead of deciding (an `EndTurn`
  the head may always submit). A move keeps the turn and costs a pool unit; a
  struck ability may keep or end it.
- **Real time decides every live controlled combatant (Readings 6, 7).** The
  candidates are the controlled set (ascending, deduplicated) filtered to live
  combatants under `awaiting_orders`' own fight gate (a clock, two or more
  alive, `elapsed < RT_CLOCK_MAX`), not only the "due with no order" ones.
  At most `AiBudget::actors_per_drive` (8) are decided per call; more rotate by
  `server_tick() % n` so every candidate is reached within a few ticks with no
  stored cursor.
- **The controlled set and `Sides` cross per call (Reading 3); the mesh is the
  host's (Reading 4).** The host stores neither the controller list nor
  `Sides`: the caller passes both, and the host holds no AI policy (T141a's
  rule). `decide` gets `Some(&mesh)` exactly when the host holds a `HostNav`,
  else `None`, so a restored host (no mesh until rebound) gets the no-move
  fallbacks.
- **Refusals and `decide` errors are steps; `Closed` is the only `Err`
  (Readings 9, 10).** `Act(a)` → `host_action(&a)`; `Move` → `host_move`
  (turn-based) or `move_to` (real time); `Halt` → `halt`; `Hold` → nothing. An
  operation's `Ok` gives `Applied`, its `Err` a `Refused` (unchanged, `CaptureFull`
  the reachable one), a `decide` `Err` an `Undecided` (unchanged); no retry, no
  forced `EndTurn` after a refusal. Every effect goes through those four
  operations, so it is recorded and published exactly as they are.
- **Pause does not stop the driver (Reading 13; D65g); route parity is a unit
  test and a compile-time tag check (Reading 12).** `crpg_ai::host_route`
  equals `nav::plan_route` (`AI_ROUTE_TAG == HOST_ROUTE_TAG == RouteTag(0)`),
  pinned at compile time and by the `host_route_equals_plan_route` unit test.
- **State and determinism (D-11).** The only new state is `ai_turn`, `None`
  from `new`, `from_history` and `load_checkpoint` and never saved, so a
  restore mid-turn restarts the count. Two hosts with equal authority, mesh and
  call history give equal reports and hashes; the driver reads no clock or RNG
  beyond the operations it calls.

The D-items are: D-1 `Closed` first; D-2 no fight → empty; D-3 the controlled
set and mesh; D-4 the turn-based gate (no head or an uncontrolled head →
empty); D-5 the turn-based step; D-6 the real-time step; D-7 the mapping; D-8
errors are steps, never retries; D-9 pause is ignored; D-10 authority (every
step's actor is in the controlled set and every applied `Act` is in
`legal_actions`); D-11 state and determinism; D-12 route parity; D-13 a host
that never calls `drive_ai` is byte-identical to today.



Spec §15.4 step 10 is active as **file-backed host save/load continuation
equivalence** (`tests/host_save.rs`, run by the existing
`cargo test --workspace --locked` job on Windows/MSVC and Linux/GNU). A host
driven through `ingest → pump` is saved with `save_host_file`, dropped, and
reloaded with `load_host_file` under a strictly greater incarnation; after
the same operator rebinding, the same later commands give the same
authority (`authority_hash`, the full-wrapper `history_hash`), capture
journal and watermarks, and decoded delivery, apart from session identity,
as an uninterrupted run in the same process.

Not claimed: crash durability or exactly-once (D10); Windows
directory-entry durability (T038); cross-target byte equality or loading a
save made on the other target (ADR-0012); and the spec §8 breadth "every
fixture campaign", which needs a host built from campaign data and comes
with T040/T041.

## Rejections and the intake observer (T040a)

`QuicPumpReport::rejections()` is the one place a rejected client message is
defined (`tasks/T040.md` Reading 5, B-1); a consumer such as the dedicated
binary's log never classifies for itself. It reads the report only, in this
order: every `hellos` entry refused (`Rejection::Hello`), every `resumes`
entry refused (`Rejection::Resume`), then `calls` in trace order, each giving
at most one of: an `Ingest` refused with a code other than `QueueFull`
(`Rejection::Ingest`; a `QueueFull` frame is held and retried, not rejected),
an `IngestTransfer` refused (`Transfer`), a `CheckFrame` that failed
(`Frame`), or, for a `Pump` summary, every result with status `Rejected` that
is not the retained head (`Admission`). Lane-1 traces are not read: the
resumable endpoint carries no lane 1.

Two reachable paths are easy to confuse. `Host::ingest` decides byte-level
admission only (rate, size, queue room) and never decodes, so an undecodable
frame such as `[0xFF]` is `Staged` and surfaces later as an `Admission`
rejection (`UnsupportedVersion`); an `Ingest` rejection over a real connection
is a `RateLimited` frame (the client transport refuses an oversized one before
`ingest`, and the server closes the connection). `Display` is B-2's one line
per rejection, without a trailing newline.

`QuicHosting` holds an optional `IntakeObserver`
(`Box<dyn FnMut(u64, &QuicPumpReport) + Send>`, set by
`set_intake_observer`). `Hosting::intake` pumps once and, when an observer is
set, calls it exactly once with the intake's `now_ms` and that report, before
the AI stage; a failed `pump` calls nothing, and the observer changes no host
or adapter state. Tests: `tests/run_intake_observer.rs` (T1, T3a, T3b, T4,
T5) and `quic.rs` `mod tests` (T2).

## The dedicated binary (T040b)

`src/main.rs` is the dedicated server program for Windows/MSVC and a
Godot-free Linux/GNU build (`tasks/T040.md`, Readings 1 to 16). It is the OS
adapter ADR-0012 puts above the simulation crates: it may use `std::fs`,
`std::io`, `std::sync`, `std::process::exit` and the `ctrlc` handler, and it
holds no host policy. It hosts T154's loop, it does not reimplement it:
`runner::spawn(HostLoop::new(QuicHosting, START_MS), SystemClock::starting_at(START_MS))`,
reached only through `RunnerHandle`. The binary reads no clock (it keeps
`run_source_rules.rs`'s fence): load, start and loop all use `now_ms = 0`, so
the clock origin equals the loop start. It creates no thread: the main thread
reads the console, the runner thread is T154's, and the signal handler runs
on the one thread `ctrlc` creates.

**Command line.** `crpg-server --config <path>`, `--help`, `--version`; no
other flag. `--help` wins over `--version`, which wins over a run. A usage
failure is one line `crpg-server: usage: ...` on standard error and exit 2.
Standard output carries only help and version.

**Configuration** is one JSON file (`deny_unknown_fields` on every object).
Required: `listen` (socket address), `certificate`, `private_key`,
`invitations`, `save`, `campaign_id`, `campaign_version`. Optional: `area`
(`{ "min": [x, y, z], "max": [x, y, z] }` in whole world units,
`-32768..=32767`, bound with `Host::bind_area_bounds` before the world is
unpaused), `ai` (a file of `sides` and `controlled`, driven once per loop
cycle through `QuicHost::drive_ai`), `log_file` (appended, created if absent,
its parent directory is not created). A relative path resolves against the
configuration file's own directory. The certificate and key files are PEM
(a `CERTIFICATE` and a `PRIVATE KEY` section, PKCS#8 only) or DER, told apart
by a leading `-----BEGIN` after any ASCII whitespace; PKCS#1 and SEC1 PEM keys
are refused ("no items found"). The invitations file is a JSON array of 1 to
`MAX_INVITATIONS` entries, each with `credential` (64 lowercase hex digits),
`control` and `grants`, every key required, mapping field for field onto
`Invitation`. Credentials are parsed and registered, never printed.

**The incarnation** is automatic. The loader needs one above every one ever
used for the authority, so the binary keeps `<save>.incarnation` (decimal and a
newline), uses `last + 1` (absent: `1`) and writes it **before** loading. A
crash after clients bound can therefore never reuse it, and a refused load
still spends one. A sidecar deleted while the save holds a higher incarnation
is refused by the loader; restore the sidecar rather than deleting it.

**Start-up order** (a failure is one line `crpg-server: error {stage}: {detail}`
and exit 1; nothing after it runs): `config`, `log`, `certificate`,
`invitations`, `ai`, `incarnation`, `load`, `area`, `unpause`, `bind`, `start`,
`register`, `spawn`, `signals`. Every file is read and validated before the
incarnation is spent; the mesh is bound before the unpause (a restored host
is unbound and `Restore`-paused); the endpoint is `bind_resumable` with
`start_resumable`. The last line of start-up is
`crpg-server: start listen=... incarnation=N pin=<64 hex> nav=bound|none ai=on|off invitations=K`,
written after the signal handler is set. `pin` is the certificate pin clients
hold.

**Logs** go to standard error and, when configured, the log file, one
unbuffered line per write. One line per rejected client message is
`crpg-server: reject t={now_ms} {rejection}`, from the intake observer over
`QuicPumpReport::rejections()` (T040a): the binary defines no rejection of its
own, and `now_ms` is the loop's time in milliseconds from the start, not a
multiple of the 50 ms period. A rejected message is never an exit.

**Stopping.** The console command `quit`, Ctrl-C and stop signals all run one
`finish` routine: stop the loop, close the adapter, save the world to its save
file through `save_host_file` (an atomic replace), write
`crpg-server: stop reason=quit|signal saved=<path>` and exit 0. `finish`
locks one shared slot, takes the runner handle and exits **while still
holding the lock**, so whichever of the console and the signal arrives first
finishes and the other blocks until the process ends. End of standard input
does not stop the server: the main thread then calls the loop once per cycle
and checks the stop flag between calls. A loop that ends by itself (a stage
error or a panic) is `crpg-server: error loop: ...`, exit 1, and **no save**.
A failed save is `crpg-server: error save: ...`, exit 1.

What the operator can rely on, and what not: on Unix `SIGINT`, `SIGTERM`
(systemd's stop) and `SIGHUP` save; on Windows every console control event
reaches the handler (Ctrl-C, Ctrl-Break, and a service wrapper that sends
Ctrl-C such as NSSM's default stop). Windows ends a process soon after its
handler returns for a console close, log-off or shutdown event, and the
handler returns at once, so **closing the console window does not guarantee a
save** (the save file stays whole: the replace is atomic). A native Windows
service (the Service Control Manager) sends no console event, so **a service
needs a wrapper that sends Ctrl-C**. A second signal during a stop does
nothing, since the handler's thread is busy in the first. A forced kill
(`SIGKILL`, `taskkill /F`) never saves.

**Exit codes:** 0 clean stop, 2 command-line usage, 1 everything else.

**Edges.** Two crates.io dependencies, approved by the user on 2026-11-19:
`ctrlc` 3.5.2 with `termination` (exact pin; the stop signals) and
`rustls-pki-types` (PEM decoding through `pem::PemObject`; already locked
through the QUIC stack). No workspace edge is added.

**Tests.** `tests/server_binary.rs` drives the built binary black box
(`CARGO_BIN_EXE_crpg-server`): help and version, usage errors, configuration
and invitation refusals that spend nothing, the sidecar, the served, logged and
saved-on-`quit` run, end of input, the area and AI files, a registration
refusal, PEM and DER, and (Unix only, compile-time gate) the signals.
Windows has no automated Ctrl-C case; sending a console control event to a
child needs an FFI call the test crate may not make, so the manual Windows
check is recorded in the pull request.

## Gate-13 dedicated smoke (T041d)

`tests/dedicated_smoke.rs` is gate 13's dedicated leg (`tasks/T041.md`, B-26 to
B-32): one test, G1, over the built binary and two real QUIC clients. It lives
in this crate because Cargo sets `CARGO_BIN_EXE_crpg-server` only for the
package's own binaries; the embedded leg is `crpg-testkit`'s `desync` D5. It
runs in the ordinary workspace test job on Windows/MSVC and Linux/GNU; no CI
job is added. It includes the shared QUIC rig (`tests/support/quic_host_rig.rs`)
for the fixture host, grants, wire encoders, the v2 `Replica` and its
`expected_state` oracle.

**What it does.** `setup` writes a fresh directory under `CARGO_TARGET_TMPDIR`:
the rig's certificate and key as DER, a save of the three-combatant fixture
host, the sidecar `7`, an invitations file of two entries (credential `0x11`
controlling `a` and `c`, credential `0x22` controlling `b`, both with the
rig's full grants) and a configuration with no optional key. `Server::spawn`
starts the binary with standard output discarded and standard error read by
one thread into a channel, and checks the start line exactly (incarnation 8,
the rig's pin, `nav=none ai=off invitations=2`). Two `QuicClient`s connect on
`/1` with wire version 2, so they speak v2 deltas with no transfers and no
movement lane; the binary serves no `/3` endpoint, so movement over the
dedicated server is not smoked here (D71a, T041e). The epoch's first half is
8 and its second half is the peer handle (1, 2).

**What it proves.** (1) Player 2 ends the turn of `a`, which it does not
control: the binary logs `reject t=... admission peer=2 seq=1 code=17
cached=false` and player 2 gets receipt `Rejected(NotAuthorized)`. (2) The
players fight: each acts once per new turn of one of its own actors with the
strike that always succeeds for 3 and ends the turn, so the six strikes are
a to b, b to a, c to b, a to b, b to a, c to b, read back from both replicas'
`ActionResolved` events. (3) Both replicas end with health `{1: (4, 10,
false), 2: (0, 10, true), 3: (3, 3, false)}`, equal to each other and to
`expected_state` of an independent mirror (`HistoryWorld` given the same six
actions); the control is that the mirror after five strikes differs. (4)
`quit` ends with exit 0 and the last line `stop reason=quit saved=<path>`, the
save loads at incarnation 9, every line is the start line, a reject line or
the stop line, and no line holds either credential's hex.

**Readings.** The reject line's `t` is the loop's time in milliseconds
(T040-E2), so the test checks a decimal number below 30,000, not a multiple of
50. The certificate and key are DER; PEM is `server_binary`'s. An intent the
server answers `StaleTick` (a stalled machine) is re-sent with the next seq and
the receipt's `processed_tick`, at most three times a turn, and each re-send
adds exactly one `code=15` reject line that the log check expects; any other
refusal panics `refused`. The binary paces by the wall clock, so every wait is
a guard that panics `deadline: ...` after 30 s and never an oracle. A guard
(`Drop` on `Server`) kills and reaps the child on every failure path.

## Authorities and consumers

- Contract: [T022 appendix](../../tasks/T022.md) and
  [ADR-0022](../adr/0022-host-session-capture-retirement.md) (superseding
  ADR-0021); restart policy D10 and host ownership D02 in
  [POST-T018-DECISIONS](../../tasks/POST-T018-DECISIONS.md); dependency
  approval D20 and ADR acceptance D19 in
  [DECISIONS-2026-09-30](../../tasks/DECISIONS-2026-09-30.md). The
  `save` adapter: [T039](../../tasks/T039.md). The `quic` adapter:
  [T023b](../../tasks/T023b.md) and
  [ADR-0026](../adr/0026-quic-host-adapter.md).
- Inputs: ADR-0017 (history), ADR-0018 (single legality source), ADR-0019
  (v2 wire and projection), T018a policy constants, and for `quic`
  ADR-0024/ADR-0025 (`crpg-net-quic` placement, wire and pinned trust).
- Consumers: T023b's `quic` adapter consumes the authenticated byte
  boundary (`ingest`, `take_delivery`/`acknowledge_delivery`,
  `QueueFull`/fencing semantics) and fences by closing the connection
  ([ADR-0026](../adr/0026-quic-host-adapter.md)). T040 drives `QuicHost`
  from the dedicated binary (pump cadence, clock conversion, credentials,
  certificate). T025b builds resume on its newest-wins supersession and
  fresh-epoch rule. T024h's `/2` snapshot sessions make narrowing seamless
  instead of fencing.
  T025b inherits the fresh-session restart rule — a world save does not
  deduplicate commands across restarts. T027b/c supply production grant
  values for the existing shapes.
- T040 (dedicated server) and the client single-player embedding get
  `save_host_file`/`load_host_file`, `SaveIdentity`, `LoadedSave` and the
  pinned error precedence; they own slot/autosave policy, the save
  directory, a never-decreasing incarnation source and gate 10's
  campaign-wide breadth. T041 drives restarts through the same two
  functions.

## Agent log

- 2026-10-04 (UTC) · claude-code + T022 · Opened the doc with the as-built host slice (modules, identity and delivery shape, edges, consumers) so later host, transport and persistence tasks extend one description rather than restating the T022 contract.
- 2026-10-05 (UTC) · claude-code + T039 · Added the `save` module, the `crpg-persist` edge, the payload/identity/precedence summary and the gate-10 claim with its non-claims, so the doc describes the persistence the crate now has rather than calling it planned.
- 2026-10-06 (UTC) · claude-code + T023b · Added the `quic` adapter (pump phases, connection-to-binding mapping), the `crpg-net-quic` edge in the dependency diagram, its consumers, and a "planned and not here" list without QUIC endpoints, so the doc describes the transport adapter the crate now has.
- 2026-10-21 (UTC) · opencode/deepseek-v4.1-flash-free + T132 (D29g/h/i) · Recorded that the two unreachable v1 host-wide caps are defence in depth, so nobody counts them as tested.
- 2026-10-30 (UTC) · opencode/deepseek-v4.1-flash-free + T024h1 · Added the snapshot-session section (session-kind table, ordinal ledger, staging steps, adapter routing), replaced the blanket narrowing reset with the per-kind rule, and removed snapshot transfer from the planned list, so the doc describes the sender T024h1 landed.
- 2026-11-01 (UTC) · opencode-go/deepseek-v4.1-flash + T024h2 · Updated the adapter routing to the as-built `/2` path (framing bind, channel reads and writes, ordinal acks, retiring drain, shutdown flush, `bind_dual` precondition), so the doc describes the adapter rather than calling it a future task.
- 2026-11-02 (UTC) · opencode-go/deepseek-v4.1-flash + T047a · Added the host read API section (prelude, active head, eligibility, one enumeration, the sim-action mapping table, replica-id snapshot semantics), so the bridge can rely on one described legality path with non-authorizing options.
- 2026-11-14 (UTC) · claude-code/claude-sonnet-5-5 + T141s · Added the host-records and checkpoint-version-2 section (entry journal, dual read and conditional write, four-event bound, tail order) and named both versions in the `checkpoint` and `capture` module paragraphs, so the doc describes the journal and save the crate now has.
- 2026-11-14 (UTC) · claude-code/claude-sonnet-5-5 + T141a · Added the host-actions section (`host_action`, `request_mode`, `awaiting_orders`, the `Combat` error and their reservation and precedence order), so the doc describes the trusted calls the host now has.
- 2026-11-14 (UTC) · claude-code/claude-sonnet-5-5 + T141b · Added the pause, settings and tick-driver section (`HostSettings`, pause state and its restore default, `advance`, auto-pause as tail step 1, the turn timer and its freezing), so the doc describes the host calls the crate now has.
- 2026-11-14 (UTC) · claude-code/claude-sonnet-5-5 + T141c · Added the protocol v3 sessions section (per-session wire, views read from `World` each call, atomic `0x82` staging, the paced outbox and its interim guards), so the doc matches `bind_v3_peer` and `host_v3.rs`.
- 2026-11-14 (UTC) · claude-code/claude-sonnet-5-5 + T141d · Added the v3 publication section (intent pipeline and codes, the exact-status cache and its v1 projection, the page layout, candidates, room, overflow and per-session fencing) and replaced the T141c interim paragraph, so the doc matches the v3 intents and pages the host now has.
- 2026-11-14 (UTC) · claude-code/claude-sonnet-5-5 + T141e · Added the v3 timing section (the running-host `Clock` heartbeat and the in-flight throttle with its frame and byte budget, receipts and notices dropped during a download), so the doc matches the throttle and heartbeat the host now has.
- 2026-11-15 (UTC) · opencode-go/deepseek-v4.1-flash + T037a · Added the navigation and trusted movement section (binding steps, the one `plan_route`, `HOST_ROUTE_TAG`, the four trusted ops and their error order) and the `crpg-nav` edge to the diagram and edge paragraph, so the doc describes the mesh the host now keeps instead of a host with no navigation.
- 2026-11-15 (UTC) · opencode-go/deepseek-v4.1-flash + T037b · Added the protocol v4 session section (v4 as a v3 session plus `V4State`, v4 lane-0 codec and receipts, `position_rows` and the withheld list, v4 pages and `Moved`, the anchoring staging page with its 94-entry/2,880,916-byte outbox bound, and the untouched v1–v3 wires), so the doc describes the v4 sessions the host now binds.
- 2026-11-15 (UTC) · opencode-go/deepseek-v4.1-flash + T037c · Added "The turn-based move and samples (T037c)" (the resolution and class order, the `commit_accepted` execution, the per-tick samples with their 20-tick refresh and replacement queue, `take_samples`, and lane-1 egress in `room_for`/pacing) and amended the T037b note that `DeclareMove` was a placeholder rejection, so the doc describes the move and the samples the host now produces.
- 2026-11-16 (UTC) · opencode-go/deepseek-v4.1-flash + T037d · Added "Lane 1 (T037d)" (synchronous admission before HM-2, HM-2's checks and actor resolution, the `advance` decision window with supersede and HM-4's outcomes, release and the 256-entry cache, duplicates, receipts, lane-1 egress and samples' `processed_seq`) and amended the T037c sample and egress notes that named a placeholder `processed_seq` and the removed `V4State.egress` field, so the doc describes the lane the host now admits and decides.
- 2026-11-17 (UTC) · opencode-go/muse-spark-1.3-contributor + T037b · Noted fix round 3b's six `host.rs` unit tests for the surviving mutants (s11, b07, w09, p12, p22, p24) and the b03/p06 equivalence reports, so the doc records how the suite pins the multi-frame staging count, the handle sentinel, per-session withheld publication, 256-op chunking, empty-page capture and the page `event_seq` sentinel.
- 2026-11-17 (UTC) · opencode/deepseek-v4.1-flash-free + T154a · Added the run-loop section (one cycle order, the 20 Hz grid and skip-not-burst, the `Hosting` seam, local intake/deliver and bounded retirement, pause and the no-clock/no-thread fence) and updated Scope, so the doc describes the loop the crate now has instead of a host nothing drives.
- 2026-11-17 (UTC) · opencode-go/muse-spark-1.3-contributor + T025b-i · Added the reconnect-grace section (ticket, grace, resume, fence, expiry, and the grace-is-not-a-binding rule), so the doc describes the 30-second host-side grace the crate now holds.
- 2026-11-18 (UTC) · opencode-go/deepseek-v4-pro + T025b-q · Extended the reconnect-grace section with the resumable-adapter route (`start_resumable`, the ticket after the `/2` welcome, fence tags, the resume answer, grace on a silent loss, what ends a grace, and the keyed-BLAKE3 proof), and updated the connection-to-binding map and the `bind_resumable` sentence to the as-built `graced`/`fence` fields and `poll_notice` polling, so the doc describes the QUIC side the crate now has.
- 2026-11-19 (UTC) · opencode-go/muse-spark-1.3-contributor + T154b · Extended the run-loop section with the background runner (the injected `Clock`, `SystemClock::starting_at`, the `crpg-server-loop` step/closure/wait thread, the `RunnerHandle` rendezvous and stop, the step-error and panic exits, and the join-and-respawn), so the doc describes the thread and clock the crate now owns.
- 2026-11-19 (UTC) · opencode-go/muse-spark-1.3-contributor + T154c · Extended the run-loop section with lane 1 on the local link (the `0xA1` intake arms and the hold queue, the receipt backlog that never drops, the samples that do, and the v1–v3 no-lane-1 mark), so the doc describes the receipts and samples the loop now delivers.
- 2026-11-19 (UTC) · opencode-go/deepseek-v4-pro + T153 · Added "The MVP AI driver (T153)" (the `drive_ai` host operation, one turn-based step per call with a cross-call 16-decision cap, real-time decisions for every live controlled combatant rotated by the world tick, caller-supplied `Sides` and controlled set, the host's own mesh or `None`, refusals as steps with `Closed` the only `Err`, pause ignored, route parity pinned by a compile-time tag check and a unit test, and the never-saved `ai_turn` that a restore restarts) and the `crpg-ai` edge to the diagram and edge paragraph, so the doc describes the AI driver the host now runs.
- 2026-11-19 (UTC) · opencode-go/deepseek-v4.1-flash + T150c · Added "QUIC v4 sessions (T150c)" (the `/3` route's v4 invitations, hello precedence, lane-1 drain and delivery, retire and fence, shutdown flush, the side trace and its `at`, and the untraced `advance`), so the doc describes the movement route the adapter now serves beside `/1` and `/2`.
- 2026-11-20 (UTC) · opencode-go/deepseek-v4.1-flash + T154d · Extended the run-loop section with the dedicated hosting (`QuicHosting`'s five stages, `flush` as `pump`'s phases 4–7 with `pump` running 1–3 then the same body, the three untraced pass-throughs, and a loaded save running through the `Restore` unpause) and updated Scope, so the doc describes the QUIC hosting the loop now has.
- 2026-11-21 (UTC) · claude-code/claude-sonnet-5-5 + T040a · Added "Rejections and the intake observer (T040a)" (`QuicPumpReport::rejections` as the single definition of a rejected client message, its B-1 order, why an undecodable frame is an admission rejection and an `Ingest` rejection is a rate-limited frame, and the `QuicHosting` intake observer), so the dedicated binary's log has one library-defined source.
- 2026-11-21 (UTC) · claude-code/claude-sonnet-5-5 + T040b · Added "The dedicated binary (T040b)" (the OS-adapter shell, the configuration, invitations, AI and PEM/DER files, the automatic incarnation sidecar, the start-up order, the rejection log, the one `finish` stop path with its Windows and signal limits, and the exit codes) and dropped the binary wiring, certificate loading, invitation format, operator configuration and incarnation source from the "not here" list, so the doc describes the program the crate now ships.
- 2026-11-21 (UTC) · claude-code/sonnet-5.5 (implementer) + T041d · Added "Gate-13 dedicated smoke (T041d)" (what G1 does over the real binary and two QUIC clients, what it proves, and the readings: DER files, `t` as loop time, the StaleTick re-send and its code-15 lines), so the doc describes the smoke the crate now has.
