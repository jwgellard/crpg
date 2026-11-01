# ADR-0031: Host snapshot sender and snapshot sessions

Date: 2026-10-30 (UTC)
Status: **Accepted** — filed by T024h1 under D39c. Covers both T024h
packages (T024h1 host sender, T024h2 QUIC `/2` adapter).

## Context

ADR-0028 gives `crpg-net` a versioned snapshot-transfer subprotocol
(first byte `0x81`) and states the T024 S§11 sender rules, but the
`crpg-server` host has no sender for it. Today narrowing grants reset a
peer's delivery: `Host::set_grants` clears the log, bumps the generation,
restarts `event_seq` at 1 and returns `GrantUpdate::RebindRequired`
(T022). That satisfies neither "`event_seq` never resets" nor "the
snapshot's cursor is the last issued sequence", so the QUIC adapter
fences a narrowing connection instead of calling `set_grants`
(ADR-0026 item 7).

The transport forces the split. A `crpg-lane0/1` connection cannot carry
transfer frames, so its session must keep the T022 reset. The opt-in
kind, chosen at bind, is what keeps the two existing narrowing tests
untouched.

## Decision

1. **Snapshot sessions are opt-in, chosen at bind.**
   `Host::bind_snapshot_peer` binds exactly as `bind_peer` (same
   precedence, errors, first delivery as delta frames from seq 1) but the
   session's delivery channel also carries the `0x81` transfer
   subprotocol in one total order with lane-0 frames. `bind_peer` is
   unchanged. The kind is fixed for the session's life; both kinds share
   every other rule (admission, receipts, caches, caps, unbind,
   shutdown).
2. **Delivery entries are tagged and ordinal-numbered.**
   `take_delivery_entries` returns `DeliveryEntry { ordinal, kind,
   bytes }`, with `DeliveryKind::{Delta, Transfer}`, so an adapter picks
   the channel without reading frame bytes. `take_delivery` keeps its
   signature and returns the same bytes in order.
   `acknowledge_delivery`'s `through` is a delivery **ordinal**, not an
   `event_seq`. In a lane-0 session every ordinal equals its entry's
   `event_seq`, so that path is unchanged. Clearing the log counts
   discarded entries as retired (`delivered = next_ordinal − 1`), so an
   acknowledgement taken before a clear never removes a later entry.
3. **Staging is atomic and immediate.** Narrowing a snapshot session and
   an honoured `Resync` plan `Cancel` (if a transfer is in flight) then
   `Begin`/`Chunk`, check room against the cleared log, and commit in the
   same call, or change nothing. There is no pending stage in `pump` or
   `take_delivery`. Segments per chunk are a private 8, so every valid v1
   body is one chunk. At v1 caps the room check cannot fail; it is kept
   as the determine–reserve–commit invariant.
4. **The unacknowledged log is cleared on narrowing and on `Resync`.**
   Every cleared entry is at or below the new cursor, and a receiver
   drops held deltas at or below the cursor at publication, so clearing
   is invisible to the replica. A cleared receipt is recoverable by the
   client's retry.
5. **A transfer in flight is always cancelled first:** `Aborted` for
   narrowing (T024 S§11 item 5), `Superseded` for `Resync`. Transfer ids
   are strictly increasing per session from 1 and never reused.
   `next_event_seq`, `next_transfer` and the epoch never go back.
6. **Trigger set:** narrowing a snapshot session, and a `Resync` naming
   an issued transfer id or none. Widening stays incremental deltas.
   Binding stages delta state from seq 1. No `Resync` dedupe.
7. **Refusals are a disposition, not an error.** `ingest_transfer`
   returns `TransferDisposition::{Published, Resynced, Stale,
   Refused(TransferRefusal)}` where the refusal is `Unbound`,
   `NotSnapshotPeer`, `RateLimited`, `Malformed`, `WrongEpoch` or
   `UnknownTransfer`. Precedence is `Closed` → `TimeRegression` →
   `Unbound` → `NotSnapshotPeer` → rate spend → decode → epoch →
   message. Transfer frames spend the shared intent buckets, both frame
   and bytes, before decoding. `transfer_state` is `Ok(None)` for a
   lane-0 session and `Ok(Some(..))` for a snapshot session.
8. **No new mechanism for overflow.** Admission backpressure, no skipped
   delta, receiver overflow healed by `Resync`, and the adapter's
   slow-consumer fence as the terminal case. No host-side cancel
   threshold or transfer deadline.
9. **The QUIC adapter fences until T024h2.** T024h1 changes no adapter
   behaviour; `/2` bindings narrow by snapshot only in T024h2, and `/1`
   connections keep fencing after it.

The exact API, ledger, staging steps and test cases are
`tasks/T024h.md` S§5–S§11 (T024h1) and S§14–S§16 (T024h2).

## Authority and affected surface

- Owning crate: `crates/crpg-server`, module `host` (T024h1) and module
  `quic` (T024h2).
- New public items in `crpg_server::host`: `DeliveryKind`,
  `DeliveryEntry`, `GrantUpdate::SnapshotStaged`, `TransferDisposition`,
  `TransferRefusal`, `TransferState`, `Host::bind_snapshot_peer`,
  `Host::take_delivery_entries`, `Host::ingest_transfer`,
  `Host::transfer_state`.
- Changed meaning, same signatures:
  `acknowledge_delivery(peer, through)` acknowledges by ordinal,
  `take_delivery` also returns transfer frames in a snapshot session, and
  `set_grants` on a snapshot session narrows by transfer.
- Contract: `tasks/T024.md` S§11, [ADR-0028](0028-snapshot-transfer-subprotocol.md),
  and `tasks/T024h.md`.

## Compatibility, hash and dependency impact

- S-set: none. No `crpg-contracts`, `World`/tick order, data schema,
  GDExtension boundary or workflow change.
- Goldens and `.replay` files: none touched. Sessions are not
  checkpointed, so no `CHECKPOINT_VERSION` bump. `authority_hash` and
  `history_hash` do not change.
- Wire: lane-0 v1/v2 bytes are unchanged; transfer bytes are
  `plan_transfer`'s. `bind_peer` sessions and `/1` connections produce
  today's bytes and traces.
- Dependencies: none added. No manifest, `Cargo.lock` or `deny.toml`
  change.
- Existing tests: T024h1 edits none. T024h2 adds two `replay` arms to
  `tests/host_quic.rs` and changes no assertion.

## Consequences

- The T024 S§11 sender rules are implemented in memory with a
  `crpg-net` receiver pool as the client stand-in (T024h1), then on real
  QUIC `crpg-lane0/2` (T024h2).
- `bind_peer` and `/1` behaviour, including the T022 narrowing reset and
  fence, stay as they are.
- T025b later adds a resume trigger by reusing the staging routine and
  retains the ledger across a reconnect; T024h adds no grace, resume
  entry point or persistence.

## Agent log

- 2026-10-30 (UTC) · opencode/deepseek-v4.1-flash-free + T024h1 · Filed ADR-0031 Accepted under D39c: opt-in snapshot sessions, tagged ordinal delivery, atomic staging and shared-budget refusals; covers both T024h packages.
