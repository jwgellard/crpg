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
signature, cap or `Display` text needs an ADR. Inventory: T022 §1, T039 §2
and T023b B§3. `quic` re-exports nothing from `crpg_net_quic` or
`crpg_net`; callers name `crpg_net_quic::{QuicServer, Credential, ...}`
themselves. Beyond
those lists the as-built crate adds only `Copy` on `ProtocolSelection`
(required by `as_u8(self)`). The binary (`src/main.rs`) is a wiring shell: no host
policy lives there.

## Invariants

1. **The host is the only mutator.** `HistoryWorld` stays private: no
   accessor returns `&HistoryWorld`, `&World` or `&mut` anything. Gameplay
   after the wrap enters only through `ingest` → `pump`; `tick()` is the one
   trusted extra op and journals nothing.
2. **Determine, reserve, then commit.** Every pump step decides the outcome
   class (gameplay through `crpg_sim::validate_action`, never a host copy of
   legality), checks worst-case room without mutating, and only then
   commits. A failed reservation leaves the head staged and changes nothing
   (no sequence, cache, budget, delivery or capture). Do not add a step that
   mutates before the reservation.
3. **Capture before anything else moves.** An accepted command's record
   (returned outcome, exact history range plus cloned envelopes, per-peer
   views) is built before the sim journal is acknowledged and before the
   next staged command runs. Never re-infer an outcome from later state;
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
- **Narrowing fences; it never calls `set_grants`.** A narrowing
  `set_invitation_grants` closes the connection with `SessionFenced`, so no
  resync generation ever reaches `take_delivery`. Seamless narrowing is
  T024/T025a.
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
