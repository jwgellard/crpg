# crpg-server — agent contract

Read the root rules, [POST-T018-RULES](../../tasks/POST-T018-RULES.md), and
the normative "Specification revision — 2026-09-29" appendix of
[T022](../../tasks/T022.md) (exact API, §3 admission precedence, §2 ledger
caps, §7 checkpoint) before editing. Reasoning:
[ADR-0022](../../docs/adr/0022-host-session-capture-retirement.md).
Architecture: [crpg-server](../../docs/architecture/crpg-server.md). This
document describes the T022 in-memory host slice, the T039 save adapter
and the T023b QUIC adapter; for `save`, [T039](../../tasks/T039.md) §2–§4
is the exact contract, and for `quic`, [T023b](../../tasks/T023b.md) B§1–B§18
as amended by A§0–A§7, with the reasoning in
[ADR-0026](../../docs/adr/0026-quic-host-adapter.md).

## Public surface

The library exposes five modules, `host`, `capture`, `checkpoint`, `save`,
`quic`, with no glob re-export; changing any public type, field, variant,
signature, cap or `Display` text needs an ADR. Inventory: T022 §1, T039 §2,
T023b B§3 and T024h S§5. `quic` re-exports nothing from `crpg_net_quic` or
`crpg_net`; callers name `crpg_net_quic::{QuicServer, Credential, ...}`
themselves. Beyond
those lists the as-built crate adds only `Copy` on `ProtocolSelection`
(required by `as_u8(self)`). The binary (`src/main.rs`) is a wiring shell: no host
policy lives there.

T024h's additive `host` items (ADR-0031): `DeliveryKind`,
`DeliveryEntry`, `GrantUpdate::SnapshotStaged`, `TransferDisposition`,
`TransferRefusal`, `TransferState`, `Host::bind_snapshot_peer`,
`Host::take_delivery_entries`, `Host::ingest_transfer` and
`Host::transfer_state`. The same ADR gives `acknowledge_delivery` an
ordinal meaning and makes `take_delivery` carry transfer frames in a
snapshot session; their signatures do not change.

T024h2's `quic` items (the ADR-0026 addendum): `HostCall::BindSnapshotPeer`
and `HostCall::IngestTransfer`, and `GrantChange::SnapshotStaged`.
`HostCall::Ingest`'s `frame` counts every channel, `TakeDelivery`'s result
is an entry count, and `AcknowledgeDelivery`'s `through` is a delivery
ordinal. On `/1` every traced and reported value is T023b's.

T047a's `host` read API (ADR-0033 §7): `LegalIntent`, `Host::legal_intents`
and `Host::replica_id`. They read the session map and grants only, and their
options confer no authorization.

T141b's additive `host` items: `HostSettings`, `AdvanceReport`,
`SINGLE_PLAYER_AUTO_PAUSE_BITS`, `HostError::InvalidSettings`, and
`Host::{settings, set_settings, paused, pause, unpause, advance,
turn_timer_remaining_ms}`. Signatures of existing items do not change.

## Invariants

1. **The host is the only mutator.** `HistoryWorld` stays private: no
   accessor returns `&HistoryWorld`, `&World` or `&mut` anything. Gameplay
   after the wrap enters only through `ingest` → `pump`; `tick()`,
   `host_action()` and `request_mode()` are the trusted extra ops (T141a), and
   `advance()` is the pause-aware driver over `tick()` (T141b).
   Host operations journal and are recorded as
   `HostRecord`s (T141s); they are never peer commands.
2. **Determine, reserve, then commit.** Every pump step decides the outcome
   class (gameplay through `crpg_sim::validate_action`, never a host copy of
   legality), checks worst-case room without mutating, and only then
   commits. A failed reservation leaves the head staged and changes nothing
   (no sequence, cache, budget, delivery or capture). Do not add a step that
   mutates before the reservation.
3. **Capture before anything else moves.** An accepted command's record
   (returned outcome, exact history range plus cloned envelopes, per-peer
   views) is built before the sim journal is acknowledged and before the
   next staged command runs, and so is every host operation's events, before
   the sim acknowledgement. Never re-infer an outcome from later state;
   never store `roll`, `margin` or `target_died`.
4. **Disclosure is per entity, per field, deny by default.** Unknown
   entities and flags are denied; any missing field omits the whole event
   (net's `project_events` does the omission from host-built candidates).
   Event suppression never suppresses an independently permitted state op.
   Retry caches hold canonical intent bytes plus fixed-size metadata only.
5. **Identity never repeats.** Handles start at 1 per host and never reuse;
   epochs embed the strictly increasing incarnation; `NetId`s are minted
   monotonically per session and retire on revocation.
6. **Every ledger is bounded in entries and bytes** (T022 §2). New state
   needs a cap, an accounting unit and a retirement path, or it does not go
   in.
7. **Time is injected; I/O is fenced.** `now_ms` only; no clock, thread,
   sleep or socket in `crpg-server` source. Filesystem I/O exists **only**
   in `save.rs`, only through `crpg_persist::save_file`/`load_file`; network
   I/O exists **only** in `quic.rs`, only through `crpg-net-quic`'s API,
   which owns the socket, runtime and thread. `host`, `capture` and
   `checkpoint` stay I/O-free apart from T022's generic checkpoint reader.
   No `HashMap`/`HashSet`.
8. `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]` on both crate
   roots (`src/lib.rs`, `src/main.rs`).

## Allowed dependencies

Normal: `crpg-core`, `crpg-sim`, `crpg-net` (path) and workspace `serde`,
`serde_json` — approved by D20 with the audit in T022's completion record —
plus `crpg-persist` (path, no features), approved for T039 (H-dep, T039 §8
and its Decisions); its `zstd` tree was already locked through T038.
No dev-dependencies. In particular there is no `crpg-data` edge, so tests
build authorities from the checked-in `HistoryWorld` fixture (below), and no
`crpg-rules` edge, so the captured outcome text is taken from T020's own
`ActionResolved` envelope for the same operation, cross-checked field by
field against the returned `ActionOutcome`. T023b adds the
`crpg-net-quic` path edge (ADR-0024, T023 R12) and nothing else: there is no
direct quinn, quinn-proto, rustls or tokio edge, because `crpg-net-quic`
exposes none of their types. Anything else needs its own approval; never
widen `deny.toml`.

## Definition of done for any change

```
cargo fmt --all
cargo clippy -p crpg-server --all-targets --locked -- -D warnings
cargo test -p crpg-server --test host_capture --locked
cargo test -p crpg-server --test host_save --locked
cargo test -p crpg-server --test host_quic --locked
cargo test -p crpg-server --locked
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
cargo deny check
git diff --check
```

## Known traps

- **The fixture is generated, not hand-written.**
  `tests/fixtures/three_combatants.json` is a `HistoryWorld` serialized after
  `start_encounter` of a three-participant encounter (seed 7; A/B/C health
  10/10/3; strike/smite/whiff abilities with single-band tables) with its
  four start envelopes unacknowledged. It loads through sim's validated
  `Deserialize`, so a sim persistence change surfaces as a load failure, not
  silent drift. To regenerate, author the same documents in a throwaway test
  inside a crate that may depend on `crpg-data` (never commit that test
  outside its own task), and review the diff like a golden.
- **`views` are not a replica.** They prove the first command's permitted
  post-state survived later mutations; replica convergence is proved through
  delivery bytes against the independent oracle.
- **Cache hits reserve nothing.** A retry of an acknowledged receipt stages
  a fresh copy only if egress has room (and, for rejections, failure budget);
  otherwise the next identical retry recovers it. Do not make cache hits
  block the queue.
- **Retry-cache eviction is the retirement path**, so the cache never
  blocks admission; an evicted retry is `StaleSeq`, never re-executed.
- **Narrowing grants return `QueueFull` when the fresh state has no room**,
  with the new grants installed and the reset pending; the next
  `take_delivery` stages it. Treat that error as `RebindRequired` in
  adapters.
- **Commands staged by a binding that is later fenced** (`unbind_peer`,
  `SeqConflict`) stay queued and are refused as `SessionExpired` by the next
  pump; `shutdown` refuses all staged work immediately.
- **`executions()` counts this host lifetime.** It is not checkpointed;
  after `load_checkpoint` it restarts at 0 while captures and authority
  continue.
- **Counter exhaustion is unreachable by counting.** `u64::MAX` sentinels for
  handles, capture sequences, replica ids and delivery sequences are refused
  without test hooks; the pinned dispatch order is the reviewed evidence.
- **No replay-format, hash-exclusion or golden change.** `authority_hash` is
  `history_hash`; the host's acknowledgement schedule is part of the
  deterministic input.
- **Never write version 2 when no host record is retained.** The writer
  emits `"version":1` and today's exact bytes unless at least one
  `HostRecord` is in the journal; the loader reads both. A turn-based host,
  and any host whose host records are all acknowledged, must keep writing
  version 1 (T141s).
- **Never route a host operation through `commit_accepted`.** A tick (and
  later a host action or the turn timer) has no originating peer, intent
  sequence or receipt, never moves `executions()`, and ends in
  `finish_host_op`: record, append, then the sim acknowledgement, which
  always runs. A record that cannot be built is `Invariant` after the
  acknowledgement, with nothing appended (the next checkpoint then refuses
  to load, visibly).
- **`MAX_NEW_EVENTS_PER_COMMAND` is the turn-based worst case (3); bound
  checks use `MAX_NEW_EVENTS_PER_COMMAND_RT` (4).** A killing command that
  ends the last turn of a round with a pending switch to real time journals
  `ActionResolved`, `Died`, `ModeChanged`, `RoundStarted`.
- **Which combatants are AI-controlled is the caller's fact (T141a).**
  `awaiting_orders()` lists every combatant the AI must decide for (turn-based
  head; real-time due, alive, unordered); the host holds no AI policy and no
  controller list, and `host_action` executes whatever the caller decides
  after `validate_action`. A host action is never a peer command: no
  sequence, receipt, `executions()` move or `last_combat_error`, and a
  refused one (`Combat(_)`, `CaptureFull`) changes nothing. `request_mode`
  journals nothing and records nothing.
- **`tick` ignores pause; `advance` is the cadence (T141b).** `tick()` is the
  operator's primitive and runs on a paused host, so a `Restore`-paused host
  still ticks through it (gate 10). A driver that honours pause, the turn timer
  and auto-pause calls `advance(now_ms)`; it ticks at most once and ends at
  most one turn per call.
- **Pause and settings are never persisted (T141b).** A restored host is
  `Restore`-paused with `HostSettings::multiplayer()`; the operator calls
  `unpause` and `set_settings` again. Pause, settings and the timer are never
  in `World`, the journal, a capture or the checkpoint, and `save_checkpoint`
  bytes do not change with them.
- **The timer runs on the caller's `now_ms` only (T141b).** No clock exists in
  the crate: the deadline is the last accepted `now_ms` plus the length, so a
  caller that never passes time never expires a turn. Auto-pause and timer
  observation are step 1 of every post-commit tail (`commit_accepted` and
  `complete_host_op`): never move them behind a fallible step.
- **v3 sessions read `World` each call and stage through an outbox (T141c).**
  `bind_v3_peer` sessions keep no map: ids come from the `InterestView`, facts
  from the `FactView`, both rebuilt from `World` per staging. Staging plans on
  clones and commits only whole; its frames wait in a per-session outbox (not
  egress) that `pace_session` drains in order under the egress caps, so a bind
  never returns `QueueFull`. A delta takes its `event_seq` when built, not when
  logged. v3 sessions are skipped by `accepted_room`.
- **v3 publication (T141d): split a page only with `frame_spans`.** One page per
  v3 session per operation is `[receipt?] ++ enters ++ events ++ rest ++ [Clock?]
  ++ [HostFacts?]`, built on clones of the views after auto-pause and the timer,
  and cut by `codec_v3::frame_spans`, never `ops.chunks`. v3 sessions never block
  admission: an overflowing page restages a snapshot, a rejected v3 intent never
  returns `Blocked` (its receipt is logged if it fits, else dropped and recovered
  by the retry), and a page or restage that cannot be built fences that one
  session, never the operation. Decide every session's plan (read-only) before any
  ledger moves; a fenced originator skips the cache and sequence update.
- **`AdmissionResult` carries v1 codes; read exact v3 codes with
  `cached_status_v3`.** The retry cache stores the v3 status for every session;
  `AdmissionResult` and `cached_status` report 21..=29 as `IllegalAction` and 30
  as `NotAuthorized`. v1/v2 sessions never get a code above 20.
- **Save identity never goes into the checkpoint JSON** without a
  `CHECKPOINT_VERSION` bump (which reopens T022). It lives in the `save`
  header in front of the verbatim checkpoint.
- **The header narrows the file-save checkpoint ceiling.** The header (22
  bytes plus both version texts, at most 150) shares `crpg-persist`'s
  16 MiB payload cap, so a file save accepts at most
  `MAX_PAYLOAD_BYTES − header_len` checkpoint bytes and refuses more as
  `TooLarge` before touching the filesystem.
- **Never compare save bytes, digests or hashes across targets.** Saves are
  deterministic per build and per target (ADR-0012); gate 10 compares runs
  inside one process and adds no golden.
- **`Persist(Io { op: SyncDir, .. })` means the save landed.** The rename
  already replaced the file; only the directory entry's durability is
  unconfirmed (Unix only). Treat it as written-but-unconfirmed, not as a
  failed save.
- **The incarnation on load is a caller fact.** The header records none; a
  product must pass one greater than every incarnation ever used for that
  authority, not only the saved one, or an older save would reuse epochs.

- **QUIC adapter (T023b): acknowledge on hand-off.** A delivery frame is
  acknowledged to the host once `try_send` accepts it, and `acked` must
  stay equal to the host's watermark. Never acknowledge frames the
  transport refused.
- **Narrowing fences `/1` bindings; `/2` bindings are snapshot sessions and
  narrow by transfer (T024h2).** A narrowing `set_invitation_grants` on a
  `/1` connection closes it with `SessionFenced`, so no resync generation
  ever reaches `take_delivery`. `bind_peer` sessions keep the T022 reset.
- **Never reset `event_seq` or ordinals in a snapshot session.** Narrowing
  and `Resync` clear the unacknowledged log, cancel in-flight, and stage
  `Begin`/`Chunk` above the current cursor; `next_event_seq`,
  `next_transfer` and the epoch never go back.
- **Acknowledge by ordinal, never by count.** `acknowledge_delivery`'s
  `through` is a delivery ordinal; a cleared entry counts as retired, so
  `acked + n` falls short. In a lane-0 session the ordinal equals the
  entry's `event_seq`. Live delivery, a departed peer and the shutdown
  flush alike acknowledge the last entry taken or accepted, never a count
  (T024h2).
- **Never pick a channel from frame bytes.** Use `DeliveryKind` from
  `take_delivery_entries`; reading the first byte is the adapter's job to
  avoid, not to do.
- **Never call the legacy `try_send`/`try_recv` on a connection that may be
  `/2`.** Every adapter read and write uses `try_send_channel`/
  `try_recv_channel`: the legacy send tags a transfer entry `Lane0`, and a
  `Transfer` frame at the head of a retired connection reads as `Ok(None)`
  for ever, leaking the record (T024h2).
- **`QuicHost::start` takes a server from `bind` or `bind_dual`, never
  `bind_resumable`, until T025b.** The adapter polls only `poll_event`,
  which stops for good at a resume notice, and no public call tells a
  resumable server apart, so this is a documented precondition on `start`
  (T024h2).
- **Staging is atomic, never pending.** Narrowing and an honoured `Resync`
  determine, check room against the cleared log, then commit in the same
  call, or change nothing. No stage point exists in `pump` or
  `take_delivery`.
- **Never read a held connection further.** After an `ingest` `QueueFull`,
  the one held frame is retried first and nothing behind it is taken, or
  per-connection order breaks.
- **Held frames delay `Closed`.** An undrained connection's reader stays
  parked, so its `Closed` event is held back. A closed peer is detected
  through `try_send` → `Closed` instead, and its delivery is discarded and
  acknowledged so it cannot block admission for the rest.
- **"Nothing is lost" under backpressure is not true in general.** A held
  frame's retry spends a rate token on every pump: at a cadence under
  25 ms while a frame is held, the bucket drains and the held frame is
  dropped as `RateLimited`. A backlog behind a hold that is larger than the
  remaining 80-frame burst is partly dropped on resume. Recovery is a
  client retry by seq. Revisit with T040 if its cadence makes this likely
  (T023b Amendment decisions, answer 6).
- **A 1-frame transport outbound queue fences peers that are reading.**
  Phase 5 clears `stalled_since` only when a whole log was accepted. Keep
  `ServerLimits.outbound` at the T022 per-peer log cap (128 frames) unless
  a slow-consumer fence is wanted.
- **Captures cap admission at 4,096 unacknowledged records.** Long runs
  need the trusted consumer to call `acknowledge_captures`, or admission
  stops with `backpressured: 1`.
- **Out-of-pump calls are not in the trace.** `disconnect`,
  `revoke_invitation`, `set_invitation_grants`, `set_invitation_control`,
  `tick` and `acknowledge_captures` make host calls that no
  `QuicPumpReport.calls` records. A replay must mirror them itself.
- **The pump cadence must beat `decision_timeout_ms` (5 s).** Hellos are
  decided only inside `pump`. A slower cadence closes them with
  `AuthTimeout`. Pump at most every 50 ms, or after `wait` returns.
- **Never print a credential.** `Invitation`'s `Debug` relies on
  `Credential`'s redaction; `QuicHost`'s `Debug` is hand-written. No error
  variant or report carries credential bytes.

## Agent log

- 2026-10-04 (UTC) · claude-code + T022 · Opened the crate contract with the host invariants, the D20 dependency boundary, the gate list and the traps found while implementing T022, so later transport and persistence tasks start from the as-built rules.
- 2026-10-05 (UTC) · claude-code + T039 · Added the `save` module to the surface, fenced filesystem I/O to `save.rs` through `crpg-persist` (invariant 7 amended, not weakened), recorded the approved `crpg-persist` edge and the `host_save` gate, and listed the save-adapter traps.
- 2026-10-06 (UTC) · claude-code + T023b · Added the `quic` module to the surface, amended invariant 7 to fence network I/O to `quic.rs` through `crpg-net-quic`, replaced the QUIC-crates sentence with the path edge, added the `host_quic` gate, and listed the adapter traps, including the approved correction that a hold can lose frames to the rate budget.
- 2026-10-30 (UTC) · opencode/deepseek-v4.1-flash-free + T024h1 · Added the snapshot-session public surface and host traps (opt-in kind, ordinal acks, `DeliveryKind`, atomic staging) and replaced the old "narrowing fences" trap with the per-kind rule, so the crate contract matches ADR-0031.
- 2026-11-01 (UTC) · opencode-go/deepseek-v4.1-flash + T024h2 · Added the adapter surface and traps for the `/2` path (ordinal acks live/departed/shutdown, no legacy `try_send`/`try_recv`, `bind`/`bind_dual` only), so the crate contract matches the as-built QUIC adapter and the ADR-0026 addendum.
- 2026-11-02 (UTC) · opencode-go/deepseek-v4.1-flash + T047a · Recorded `LegalIntent`, `Host::legal_intents` and `Host::replica_id` in the surface, so the bridge read API is known to be trusted, read-only and non-authorizing rather than a client capability.
- 2026-11-14 (UTC) · claude-code/claude-sonnet-5-5 + T141s · Amended invariants 1 and 3 (host operations journal as `HostRecord`s and are captured before the sim acknowledgement) and added the version-1-unless-a-host-record, no-`commit_accepted`-for-host-operations and four-event-bound traps, so the contract matches checkpoint version 2.
- 2026-11-14 (UTC) · claude-code/claude-sonnet-5-5 + T141a · Added `host_action`, `request_mode` and `awaiting_orders` to invariant 1's trusted ops and a trap that AI control is the caller's fact and a host action is never a peer command.
- 2026-11-14 (UTC) · claude-code/claude-sonnet-5-5 + T141b · Added the traps for `tick` versus `advance`, unpersisted pause and settings, and the caller-time timer, and amended invariant 1 to name the T141b pause calls, so the contract matches the pause and timer code.
- 2026-11-14 (UTC) · claude-code/claude-sonnet-5-5 + T141c · Added the v3-session trap (no stored map, atomic staging into a paced outbox, interim v3 guards) and named the `bind_v3_peer` family in the surface, so the contract matches the v3 session code.
- 2026-11-14 (UTC) · claude-code/claude-sonnet-5-5 + T141d · Replaced the interim v3 trap with the publication traps (frame_spans only, never-blocking v3 sessions with per-session fences, exact v3 codes behind `cached_status_v3`), so the contract matches the v3 pipeline.
