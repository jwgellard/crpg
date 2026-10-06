# crpg-server architecture

## Scope

`crpg-server` owns authoritative world state. As of T022 it is a library plus
a thin dedicated binary (D02): the library is the one platform-neutral host
that the Windows embedded single-player adapter and the Windows/Linux
dedicated adapters will reuse, and the binary stays a wiring shell that
still refuses to run until the dedicated product (T040) wires it. The exact contract —
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
**not** here: the dedicated binary's wiring, certificate and key loading,
credential generation, the invitation file format and operator
configuration (T040); reconnect, resume and the 30 s grace (T025a/T025b);
snapshot transfer and seamless narrowing (T024); production interest facts
(T027a–c supply values for these grant shapes); save slot/autosave policy,
the save directory and the incarnation source (T040); OS service
integration; listen-server mixed mode; and the GM endpoint.

## Where it sits

```text
crpg-core     ─┐
crpg-sim      ─┤
crpg-net      ─┼─> crpg-server (lib) ─> crpg-server (bin, thin)
crpg-net-quic ─┤     + serde/serde_json (checkpoint JSON)
crpg-persist  ─┘
```

Edges are D20's: normal path dependencies on `crpg-core`, `crpg-sim` and
`crpg-net` plus workspace `serde`/`serde_json`, and T039's path edge to
`crpg-persist` (approved in [T039](../../tasks/T039.md) §8/Decisions; its
`zstd` tree was already locked through T038), and T023b's path edge to
`crpg-net-quic` (ADR-0024, T023 R12). That edge brings the transport's
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

## Connections and bindings (T023b)

A QUIC connection has no gameplay identity until the adapter binds it. The
adapter keeps two private `BTreeMap`s. The first maps each registered
invitation (32-byte credential, normalized control and disclosure grants,
and its live connection) by a never-reused `InvitationId`. The second maps
each mapped `ConnectionId` to either a bound entry (`PeerHandle`, epoch,
acknowledged delivery watermark, frame ordinal, at most one held frame,
stall start, transport-close state) or a retiring entry still being
drained so the transport can forget it. A connection is mapped only after
`accept` succeeds. One invitation has at most one bound connection, and a
newer one supersedes it. `ConnectionId`s never enter host state: the host
sees only `PeerHandle`s minted in bind order, so the same calls replayed in
memory give the same handles and epochs.

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
  fresh-epoch rule. T024 adds snapshot resync, which can make narrowing
  seamless.
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
