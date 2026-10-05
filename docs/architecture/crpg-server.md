# crpg-server architecture

## Scope

`crpg-server` owns authoritative world state. As of T022 it is a library plus
a thin dedicated binary (D02): the library is the one platform-neutral host
that the Windows embedded single-player adapter and the Windows/Linux
dedicated adapters will reuse, and the binary stays a wiring shell that
still refuses to run until a transport adapter exists. The exact contract —
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
compressed, capped envelope and atomic-replace file store. What is planned
and **not** here: QUIC endpoints and the invitation/pinned-certificate
handshake (T023/T023b, with T030 evidence), production interest facts
(T027a–c supply values for these grant shapes), save slot/autosave policy,
the save directory and the incarnation source (product tasks, T040), OS
service integration, reconnect grace, and the GM endpoint.

## Where it sits

```text
crpg-core    ─┐
crpg-sim     ─┤
crpg-net     ─┼─> crpg-server (lib) ─> crpg-server (bin, thin)
crpg-persist ─┘     + serde/serde_json (checkpoint JSON)
```

Edges are D20's: normal path dependencies on `crpg-core`, `crpg-sim` and
`crpg-net` plus workspace `serde`/`serde_json`, and T039's path edge to
`crpg-persist` (approved in [T039](../../tasks/T039.md) §8/Decisions; its
`zstd` tree was already locked through T038). Nothing else. The host
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
  full instead of dropping a result.
- `checkpoint` — `Host::save_checkpoint` and `load_checkpoint`/
  `load_checkpoint_from_reader`: compact JSON of the wrapper, the retained
  capture journal plus watermark, protocol and incarnation; input capped
  before parsing; loading validates everything into a temporary host and
  restarts every session under a strictly greater incarnation.
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

## Identity and delivery shape

A session epoch is `incarnation` (LE) followed by the never-reused handle
(LE), so correlation `(epoch, lane, seq)` is globally unique across rebinds
and restarts. Per peer, the host keeps a live `EntityId → NetId` map minted
in ascending `EntityId` order (revoked ids retire; re-granted entities get
fresh ids), a lane-0 retry cache of canonical intent bytes plus fixed-size
outcome metadata, failure-response budgets, and a delivery log. Receipts go
to the originator only; permitted events (V2) and state ops fan out per
peer. V1 hosts deliver receipts plus legacy state ops only; V2 hosts add the
ordered event ops. Narrowing grants restart a peer's delivery at 1 with a
fresh filtered state; widening continues it.

## Gate 10 (save/load equivalence)

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

## Authorities and consumers

- Contract: [T022 appendix](../../tasks/T022.md) and
  [ADR-0022](../adr/0022-host-session-capture-retirement.md) (superseding
  ADR-0021); restart policy D10 and host ownership D02 in
  [POST-T018-DECISIONS](../../tasks/POST-T018-DECISIONS.md); dependency
  approval D20 and ADR acceptance D19 in
  [DECISIONS-2026-09-30](../../tasks/DECISIONS-2026-09-30.md).
- Inputs: ADR-0017 (history), ADR-0018 (single legality source), ADR-0019
  (v2 wire and projection), T018a policy constants.
- Consumers: T023/T023b inherit the authenticated byte boundary (`ingest`,
  `take_delivery`/`acknowledge_delivery`, `QueueFull`/fencing semantics) and
  must fence bytes already taken before a grant change, unbind or shutdown.
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
