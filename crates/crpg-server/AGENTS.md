# crpg-server — agent contract

Read the root rules, [POST-T018-RULES](../../tasks/POST-T018-RULES.md), and
the normative "Specification revision — 2026-09-29" appendix of
[T022](../../tasks/T022.md) (exact API, §3 admission precedence, §2 ledger
caps, §7 checkpoint) before editing. Reasoning:
[ADR-0022](../../docs/adr/0022-host-session-capture-retirement.md).
Architecture: [crpg-server](../../docs/architecture/crpg-server.md). This
document describes the T022 in-memory host slice and the T039 save adapter;
for `save`, [T039](../../tasks/T039.md) §2–§4 is the exact contract.

## Public surface

The library exposes four modules, `host`, `capture`, `checkpoint`, `save`,
with no glob re-export; changing any public type, field, variant, signature,
cap or `Display` text needs an ADR. Inventory: T022 §1 and T039 §2. Beyond
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
7. **Time is injected; I/O is fenced.** `now_ms` only; no `SystemTime`,
   threads or sleeps. Filesystem I/O exists **only** in `save.rs`, and only
   through `crpg_persist::save_file`/`load_file`; `host`, `capture` and
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
field against the returned `ActionOutcome`. Anything else (including QUIC
crates, which D23 approves only for T023b with its own record) needs its own
approval; never widen `deny.toml`.

## Definition of done for any change

```
cargo fmt --all
cargo clippy -p crpg-server --all-targets --locked -- -D warnings
cargo test -p crpg-server --test host_capture --locked
cargo test -p crpg-server --test host_save --locked
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

## Agent log

- 2026-10-04 (UTC) · claude-code + T022 · Opened the crate contract with the host invariants, the D20 dependency boundary, the gate list and the traps found while implementing T022, so later transport and persistence tasks start from the as-built rules.
- 2026-10-05 (UTC) · claude-code + T039 · Added the `save` module to the surface, fenced filesystem I/O to `save.rs` through `crpg-persist` (invariant 7 amended, not weakened), recorded the approved `crpg-persist` edge and the `host_save` gate, and listed the save-adapter traps.
