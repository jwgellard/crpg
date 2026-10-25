# T025a — reconnect/resync mechanisms (`crpg-net`): blind contract draft (beam)

Status: doc-only design draft. Sole crate `crpg-net`. No code, no dependency,
no manifest/lock/deny/tools change. Follows the S§ shape of
`tasks/T024.md` (S§0–S§16). D12 selected (30 s same-process grace, host-owned).

## S§0. Inputs read

- `tasks/T025a.md`, `tasks/T025a-facts.md`, `tasks/T025b.md`.
- `tasks/POST-T018-DECISIONS.md` §D12; `tasks/POST-T018-RULES.md`.
- `tasks/T024.md` (S§1–S§16), `docs/adr/0019-event-protocol-v2.md`,
  `docs/adr/0028-snapshot-transfer-subprotocol.md`,
  `docs/adr/0022-host-session-capture-retirement.md`.
- `docs/architecture/crpg-net.md`, `crates/crpg-net/AGENTS.md`.
- `crates/crpg-net/src/protocol.rs`, `protocol_v2.rs`, `protocol_transfer.rs`,
  `codec.rs`, `codec_v2.rs`, `codec_transfer.rs`, `reassembly.rs` (read-only).

Code facts are cited as `path:line` per `tasks/T025a-facts.md`. Where the fact
sheet is silent this draft states the gap rather than inferring.

## S§1. Findings (current code)

- No resume/reconnect type exists in `crpg-net` (facts §2, §6).
- `SessionEpoch` is the only session-generation identity; documented as replay
  isolation, not authentication (`protocol.rs:202-207`).
- No connection-generation/fence token exists beyond epoch match + closed set
  (`tests/support/driver.rs:227-228,423-432`); `InMemoryTransport.add_peer`
  ignores `_epoch` (`sim.rs:430-436`).
- No credential/token/opaque-blob type and no `Debug` redaction
  (`protocol.rs:206`, `sim.rs:235`, `protocol_transfer.rs:234`).
- Transfer subprotocol already supplies atomic snapshot install + cutover
  (`reassembly.rs:739-851`, `884-998`); receiver is bound to one epoch for
  life and a new epoch is a new receiver (`reassembly.rs:696-708`).
- Command identity is epoch/lane/seq plus canonical bytes in `PeerState.cache`
  (`protocol.rs:244-258`, `sim.rs:244-246`,
  `tests/support/driver.rs:435-439`); retries return the original
  `ReceiptStatus` without re-execution (`tests/support/driver.rs:447`).
- `event_seq`/`next_event_seq` is per receiver and never resets within an
  epoch by sender rule (T024 S§11.2); same epoch resume must preserve it.
- Budgets (failure/egress) are host policy and must not be replenished by
  resume (D12; `protocol.rs:145-149`, `sim.rs:83-147`).

## S§2. Decisions this contract makes

1. **Separate reconnect control subprotocol, not a main version bump.**
   First byte `RESUME_VERSION = 0x82` (high bit marks subprotocol, so main
   versions stay below 0x80 and transfer stays 0x81). Lane byte `0` (control
   channel; demultiplexed at channel selection, per T024 S§4). v1/v2/transfer
   bytes and tags are untouched. Rationale: reconnect handshake is control, not
   combat and not snapshot bytes.
2. **Reuse T024 transfer for the snapshot.** Resume acceptance does not define
   new snapshot bytes; the host starts a normal `BeginTransfer` (id higher than
   retired) on lane 2 with the current per-lane cursor. This keeps one snapshot
   mechanism.
3. **Host-approved decisions only.** Net exposes a `HostResumeDecision` input
   (`Accepted`/`Rejected`) and applies it. Net never inspects the credential,
   never runs a timer, never grants authority. Grace expiry is host policy
   (T025b).
4. **Fencing by generation, not by epoch.** A monotonic `ConnectionGeneration`
   is issued on every accepted resume. The old connection is fenced by
   generation mismatch; the epoch is unchanged. Identity is never "claimed
   epoch alone".
5. **No seq/dedup reset.** `next_event_seq`, `retired_through`, `PeerState`
   seq/cache and budgets survive resume. Only the connection binding changes.
6. **No new dependency.** Types are plain Rust with `postcard` primitives; no
   crypto, no serde derive beyond existing, no new crate.

## S§3. Files (proposed, implementation-time)

New:

- `crates/crpg-net/src/protocol_resume.rs` — constants, identity, message types.
- `crates/crpg-net/src/codec_resume.rs` — encode/decode.
- `crates/crpg-net/tests/reconnect_wire.rs` — the cases in S§12.

Changed (implementation-time only, not in this draft):

- `crates/crpg-net/src/lib.rs` — two module lines + crate-doc sentence.
- `crates/crpg-net/AGENTS.md`, `docs/architecture/crpg-net.md` — entries.
- `docs/adr/0029?` or a reconnect ADR — filed at implementation.

Not changed: `Cargo.toml`, `Cargo.lock`, `deny.toml`, `tools/`, existing
`src/` modules, existing tests.

## S§4. Version, lane, direction and tags

| Item | Value | Rule |
|---|---|---|
| First byte | `0x82` | `decode_request`/`decode_result` refuse any other with `UnsupportedVersion`; v1/v2/transfer codecs refuse 0x82 the same way |
| Lane | `0` | anything else is `WrongDirection` |
| Upstream (client → host), ≤ 4,096 B | `0x20` ResumeRequest | `decode_result` reads it as `WrongDirection` |
| Downstream (host → client), ≤ 4,096 B | `0x10` ResumeResult | `decode_request` reads it as `WrongDirection` |
| Any other tag | — | `UnknownMessage` |

Demultiplexing at channel selection (T024 S§4). No decoder guesses.

## S§5. Wire layout (exact field order)

Encoding: `postcard` primitives (`u8`/`bool` raw, varints, `[u8;16]` raw,
length-prefixed byte vectors). Each frame: `0x82`, `0`, tag.

### S§5.1 Upstream — ResumeRequest

| Tag | Fields after tag, in order |
|---|---|
| `0x20` | `epoch [u8;16]`, `generation u64`, `credential_len u8` (1..=256) is not a separate field — credential is `len` varint (1..=256) then `len` bytes, `progress_count u8` (0..=8), then `progress_count` × (`lane u8`, `last_event_seq u64` (≤ `SEQ_LAST`)) |

### S§5.2 Downstream — ResumeResult

| Tag | Fields after tag, in order |
|---|---|
| `0x10` | `epoch [u8;16]`, `generation u64`, `status u8` (1 `Accepted`, 2..=N rejected codes) |

Accepted carries the new `generation`. Rejected carries the generation the
request supplied (unchanged) so the client can correlate; `generation` 0 on a
rejected result means "no session".

### S§5.3 Credential

- Opaque bytes, length 1..=256 (`MAX_RESUME_CREDENTIAL_BYTES = 256`).
- No prescribed format, no version byte inside, no crypto dependency.
- Identity is never "claimed epoch alone": the host must validate the
  credential plus its own session record; net only transports it.
- Redaction: `ResumeRequest` and `ResumeResult` derive `Debug` with the
  credential field replaced by `[REDACTED len=N]`. `Display` of any error
  never includes credential bytes. Logs in tests must not print them.

### S§5.4 Progress

- `lane` must be a known lane (0, 1, or 2) and unique within the list;
  duplicate lane is `Malformed`.
- `last_event_seq` 0 means "none received"; otherwise ≤ `SEQ_LAST`.
- Empty list is valid (fresh client).
- Cap `MAX_RESUME_LANES = 8` (room for future lanes without a wire change).

## S§6. Codec rules and error precedence

Errors are the existing six `CodecError` variants (re-exported), plus a new
closed `ResumeError` (S§8) for mechanism-level outcomes.

Frame decode precedence (first failure wins):

1. Whole-frame cap: request input > 4,096 or result input > 4,096 →
   `FrameTooLarge`, before any read.
2. First byte: not `0x82` → `UnsupportedVersion`; empty → `Malformed`.
3. Lane: not `0` → `WrongDirection`.
4. Tag: the other direction's tag → `WrongDirection`; any other →
   `UnknownMessage`.
5. Fields left to right:
   - `epoch` is raw 16 bytes (no check).
   - `generation` varint (u64, tenth-byte overflow → `Malformed`).
   - credential length 0 or > 256 → `LimitExceeded` if > 256, `Malformed`
     if 0; bytes then copied with `extend` from already-bounded length.
   - `progress_count` > 8 → `LimitExceeded`; 0 valid.
   - per progress: `lane` not in {0,1,2} → `Malformed`; duplicate lane →
     `Malformed`; `last_event_seq` > `SEQ_LAST` → `Malformed`.
   - `status` byte: 1 `Accepted`, 2 `RejectedFenced`, 3 `RejectedCrossEpoch`,
     4 `RejectedUnknownVersion`, 5 `RejectedBadCredential`,
     6 `RejectedHostDenied`, 7 `RejectedExpired`. Any other → `UnknownMessage`.
6. Trailing bytes → `Malformed`.

Encode checks the same rules in the same order, then the frame cap, and never
returns partial bytes. `ResumeRequest::validate` / `ResumeResult::validate`
are shared by encode and (future) host admission.

## S§7. Hard caps and policy

| Bound | Value | Kind | Source |
|---|---|---|---|
| Resume credential | 256 B | wire | this contract |
| Resume lanes | 8 | wire | this contract |
| Frame (both directions) | 4,096 B | wire | `MAX_INTENT_FRAME_BYTES` |
| `ConnectionGeneration` | u64, saturating at `u64::MAX` | wire | this contract |
| Lane progress seq | ≤ `SEQ_LAST` | wire | `protocol.rs:48` |

No wire hard maximum is raised; no `MAX_*` in `protocol.rs` changes. Policy
limits (grace 30,000 ms, budgets) stay host-supplied and never wire.

## S§8. Interface (exact public Rust)

```rust
pub const RESUME_VERSION: u8 = 0x82;
pub const RESUME_LANE: u8 = 0;
pub const RESUME_TAG_REQUEST: u8 = 0x20;
pub const RESUME_TAG_RESULT: u8 = 0x10;
pub const MAX_RESUME_CREDENTIAL_BYTES: usize = 256;
pub const MAX_RESUME_LANES: usize = 8;

pub struct ConnectionGeneration(u64);
impl ConnectionGeneration {
    pub fn initial() -> Self;          // 0
    pub fn get(self) -> u64;
    pub fn next(self) -> Self;         // saturating_add(1)
}
// Copy, Ord, Debug(redacted-free: generation is not secret).

pub struct LaneProgress {
    pub lane: u8,
    pub last_event_seq: u64,
}

pub struct ResumeRequest {
    pub epoch: [u8; 16],
    pub generation: ConnectionGeneration,
    pub credential: Vec<u8>,
    pub progress: Vec<LaneProgress>,
}
impl ResumeRequest {
    pub fn validate(&self) -> Result<(), CodecError>;
}

pub struct ResumeResult {
    pub epoch: [u8; 16],
    pub generation: ConnectionGeneration,
    pub status: ResumeStatus,
}
impl ResumeResult {
    pub fn validate(&self) -> Result<(), CodecError>;
}

pub enum ResumeStatus {
    Accepted,
    Rejected(ResumeRejectCode),
}

#[repr(u8)]
pub enum ResumeRejectCode {
    Fenced = 2,
    CrossEpoch = 3,
    UnknownVersion = 4,
    BadCredential = 5,
    HostDenied = 6,
    Expired = 7,
}
// as_u8 / from_u8; Copy.

pub enum ResumeError {
    FencedConnection,
    CrossEpochSnapshot,
    UnknownVersion,
    ReplayFromFenced,
    ObsoleteTransfer,
    HostDenied,
}

pub enum HostResumeDecision { Accepted, Rejected }

pub fn encode_request(msg: &ResumeRequest) -> Result<Vec<u8>, CodecError>;
pub fn decode_request(bytes: &[u8]) -> Result<ResumeRequest, CodecError>;
pub fn encode_result(msg: &ResumeResult) -> Result<Vec<u8>, CodecError>;
pub fn decode_result(bytes: &[u8]) -> Result<ResumeResult, CodecError>;
```

`ResumeRequest`/`ResumeResult` derive `Debug, Clone, PartialEq, Eq` with a
manual `Debug` that redacts `credential` (shows `[REDACTED len=N]`). All other
numbers come from `crate::protocol` by qualified reference.

## S§9. Behaviour

1. **Resume is a host decision.** Net receives `HostResumeDecision` from the
   host stand-in/host. On `Accepted`, net issues `generation.next()`, binds
   the new connection to the session epoch, fences the old connection, and
   returns `ResumeResult { Accepted, generation }`. On `Rejected`, net returns
   `ResumeResult { Rejected(code), generation: request.generation }` and
   changes no session state.
2. **Epoch unchanged on resume.** The session epoch stays the same for the
   whole grace window (D12). A new epoch is a new session (T025b restart).
3. **Fencing.** The old connection's generation is now stale. Any frame
   (resume request, intent, delta ack) presented by the old connection with a
   generation below the current one is rejected as `ReplayFromFenced`. Net
   supplies the generation type and the comparison; enforcement lives at the
   host binding (T025b) for real transports and in the test stand-in.
4. **No seq/dedup reset.** `next_event_seq`, `retired_through`, command
   cache and budgets are untouched by resume. A resumed client continues with
   its original seq space; retries keep original epoch/lane/seq/canonical bytes
   including `observed_tick`.
5. **Atomic snapshot before post-cutover deltas.** After accepted resume the
   host starts a transfer with `cursor` = current per-lane event seq. The
   client installs the full filtered snapshot atomically (T024 S§10.6) before
   any delta at or above `cursor + 1` is applied. Deltas at or below the
   cursor are `Superseded`.
6. **In-flight commands keep identity.** Commands submitted before disconnect
   that have not received a receipt retain their original epoch/lane/seq/
   canonical bytes. On retry after resume they are admitted through the
   existing cache (`tests/support/driver.rs:435-439`) and return the original
   `ReceiptStatus` without re-execution.
7. **Cross-epoch snapshot rejection.** A transfer whose `epoch` differs from
   the receiver's epoch is `WrongEpoch` (T024 S§10.2.4). A snapshot with
   `cursor + 1 < next_event_seq` or `base_tick` below published is
   `BehindReplica` (T024 S§10.3.4). These are not bypassed by resume.
8. **Unknown version rejection.** A frame whose first byte is not `0x82` on
   this subprotocol (including 1, 2, 0x81) is `UnsupportedVersion`. v1/v2/
   transfer codecs reject `0x82` the same way (T024 S§9).
9. **Replay from fenced connection.** A resume request carrying a generation
   below the current one is rejected `Fenced` and changes no state.
10. **Obsolete partial transfer cannot overwrite a later resync.** Transfer ids
    are retired high-water marks (T024 S§10.1). A `Begin` at or below
    `retired_through` is `StaleTransfer`; `BehindReplica` guards base
    rollback. A failed/expired/cancelled transfer frees its reservation and
    retires its id; a later `Begin` with a higher id publishes normally.
11. **Budgets not replenished.** Failure/egress budgets are untouched by
    resume; D12 states they are not replenished by disconnect/resume.

## S§10. Error enums (closed)

- Frame shape: existing `CodecError` (six variants), re-exported.
- Mechanism: `ResumeError` (six variants, S§8) — closed, `Debug + Display +
  Error`, no payload bytes.
- Resume rejection codes: `ResumeRejectCode` (seven variants, S§8) —
  `#[repr(u8)]`, frozen 2..=7; new codes need a version bump.
- Host policy failures (bad/expired/revoked proof, grace expiry, wrong client)
  are reported as `ResumeRejectCode::HostDenied` or `Expired` plus host logs;
  net does not expand the enum per host reason (T025b owns detail).

## S§11. Reassembler

No new reassembler. Resume reuses T024:

- `ReassemblyPool::add_receiver` (same epoch, new receiver only if needed;
  normally the existing receiver survives because the epoch is retained).
- `ReassemblyPool::offer_transfer` / `offer_delta` / `pop_update` for the
  post-resume snapshot cutover.
- `TransferAssembly` for byte-layer proof of the snapshot.

Net does not reset `next_event_seq` or `retired_through` on resume. The host
binding (T025b) maps a resumed connection to the existing receiver.

## S§12. Tests (`reconnect_wire`, public API only)

Every refusal has a positive control and a whole-state `assert_eq!` where
applicable. The host stand-in supplies an explicit `HostResumeDecision`
input; tests never claim to prove authentication. The replica oracle is
independent and hand-authored; it never uses a production projection or a
full World hash.

Numbered cases (mapped to `tasks/T025a.md` §Test):

1. `wire_identifiers_and_caps_are_pinned` — constants (S§4/S§7), `0x82` ∉
   {1,2,0x81}; tags disjoint from intent {0,1}, delta 0..=10, transfer
   0x10..=0x21; credential 256, lanes 8; v1/v2/transfer constants unchanged.
2. `literal_byte_oracles` — minimal request/result encode to exact bytes and
   decode back (all-zero epoch, generation 0, empty credential impossible so
   length 1, empty progress).
3. `frame_decode_precedence_request` — 4,097 bytes `FrameTooLarge` even with
   bad version; versions 1,2,0x81 `UnsupportedVersion`; lanes 1,2,3
   `WrongDirection`; tag 0x10 `WrongDirection`; 0x00,0xFF `UnknownMessage`;
   credential 0 and 257 `Malformed`/`LimitExceeded`; progress count 9
   `LimitExceeded`; duplicate lane `Malformed`; `last_event_seq` `u64::MAX`
   `Malformed`; trailing byte `Malformed`; tenth-byte overflow `Malformed`.
4. `frame_decode_precedence_result` — size/lane/tag/status precedence; status
   byte 0,8,255 `UnknownMessage`.
5. `credential_bounds_and_redaction` — 1 byte accepted, 256 accepted, 257
   `LimitExceeded`; empty `Malformed`; `Debug` output contains
   `[REDACTED len=N]` and not the credential bytes.
6. `progress_bounds` — 0..=8 lanes; duplicate lane rejected; lane 3 rejected;
   `last_event_seq` 0 and `SEQ_LAST` accepted, `u64::MAX` rejected.
7. `generation_fencing` — request with generation below current is rejected
   `Fenced` and changes no state; equal/current accepted given host
   `Accepted`; above current rejected `Fenced`.
8. `cross_epoch_snapshot_refused` — transfer with differing epoch is
   `WrongEpoch` with pool unchanged; resume does not bypass it.
9. `unknown_version_refused` — 0x81/0x01/0x02 frame on this subprotocol is
   `UnsupportedVersion`; v1/v2/transfer codecs reject 0x82 the same way.
10. `replay_from_fenced_refused` — old connection's request after a successful
    resume returns `Fenced`; state unchanged.
11. `obsolete_partial_transfer_cannot_overwrite` — expired/failed/cancelled
    transfer frees reservation and retires id; a later higher id publishes; a
    `Begin` at or below `retired_through` is `StaleTransfer`; `BehindReplica`
    guards base rollback.
12. `atomic_snapshot_before_deltas` — after accepted resume, transfer publishes
    with queue `Snapshot` then deltas ≥ cursor+1; deltas ≤ cursor are
    `Superseded`; oracle matches. Negative control: applying a ≤-cursor delta
    after the snapshot makes the oracle fail.
13. `command_retry_identity` — in-flight command submitted before disconnect
    retains epoch/lane/seq/canonical bytes; retry after resume is cache hit
    returning original `ReceiptStatus` without re-execution; `observed_tick`
    unchanged.
14. `no_seq_or_dedup_reset` — `next_event_seq`, `retired_through`, command
    cache depth and budgets are byte-identical before/after resume.
15. `duplicate_late_old_transfer_delivery` — duplicate `Begin`, duplicate chunk,
    late chunk after expiry, old transfer id are `Duplicate`/`StaleTransfer`/
    `Failed(Expired)` with unchanged state except time-driven sweep.
16. `budgets_not_replenished` — failure and egress budgets are byte-identical
    across resume.
17. `host_decision_input_controls` — host stand-in `Accepted` yields Accepted
    result and new generation; `Rejected` yields Rejected with supplied code
    and no state change. Neither test claims authentication.
18. `replica_oracle_matches_after_resync` — independent filtered replica
    oracle equals expected view after resume + snapshot + deltas.
19. `oracle_corruption_negative_control` — one health byte corrupted in the
    snapshot makes the oracle report mismatch while the transfer still
    publishes; unmodified transfer matches.
20. `fabric_resume_with_faults` (optional) — through `InMemoryTransport` with
    dup/reorder faults, resume + resync publishes once and oracle matches.

Commands (implementation-time):

```text
cargo test -p crpg-net --test reconnect_wire --locked
cargo test -p crpg-net --test snapshot_transfer --locked
cargo test -p crpg-net --test codec --locked
cargo clippy -p crpg-net --all-targets --locked -- -D warnings
```

## S§13. Boundary to other tasks

Left to T025b (host, `crpg-server`):

- Grace timer (30,000 ms), expiry, monotonic clock injection.
- Credential issuance/verification implementation, revocation, session record.
- Authentication, TLS, grant checks, fencing enforcement on real transports.
- Rate/failure budget accounting on real host (net only supplies types).
- Process restart → new epoch, no persisted grace (D10/D12).

Left to T024q/T024h and other T024 follow-ons:

- Real transport channel wiring for resume frames.
- Host capture integration (`try_resync`, `set_grants`, `full_state_ops`
  migration to snapshot transfer).
- Movement lane (D13) resume progress beyond lane 0/2.
- Interest/visibility revocation reusing transfer cancel (D14).

Left to T024 (already owned):

- Snapshot bytes, transfer codec, reassembly, cutover gate.

## S§14. S-set, dependency and invariant impact

- S-set (`crpg-contracts`, `World`/tick order, schema versions, GDExtension
  boundary, workspace `Cargo.toml`, `deny.toml`, workflows): **none**.
- Dependency: **none**. No new crate, feature, or lock change.
- Invariants touched: none of the v1/v2/transfer bytes; seq/dedup state is
  preserved, not reset; budgets preserved. New subprotocol is additive with
  its own version byte.

## S§15. Open questions for the user

1. **Subprotocol byte 0x82 vs main protocol v3.** Recommend 0x82 (separate
   control subprotocol, reusing T024 demux). Main v3 would force every codec
   to carry a version parameter and blur the combat lane.
2. **Fencing generation scope.** Recommend handshake-only issuance plus host
   binding; do not add a generation field to frozen v1/v2/transfer frames.
   Alternative (per-frame generation) breaks frozen bytes.
3. **Credential cap 256 vs 64.** Recommend 256 to fit operator-provisioned
   proofs (D03) with headroom; 64 is enough for a 32-byte token but leaves no
   room for structured proofs. Cap is wire-versioned.
4. **Resume result rejects with code vs boolean.** Recommend closed
   `ResumeRejectCode` (seven variants) so tests can assert exact rejection;
   a single boolean would hide host policy detail that T025b needs.
5. **Should net track grace expiry?** Recommend no — host owns the clock and
   the 30,000 ms deadline (D12). Net only carries `Expired` as a host-reported
   result code.
6. **Progress as Vec vs fixed array.** Recommend bounded `Vec<LaneProgress>`
   (cap 8, unique lanes) for forward compatibility with movement/interest
   lanes; a fixed 3-tuple would need a wire change later.

## Agent log

- 2026-10-25 (UTC) · opencode-go/beam-free + T025a draft · Blind contract draft for reconnect/resync mechanisms: 0x82 control subprotocol, opaque redacted credential, fencing generation, preserved seq/dedup/budgets, reuse of T024 snapshot cutover, closed error enums, 20 reconnect_wire cases, boundary list.