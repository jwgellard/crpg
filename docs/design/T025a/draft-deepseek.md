# T025a — resume/resync mechanism for `crpg-net`: blind contract draft

Status: **draft, not approved.** For an architect to compare with an independent
draft. It proposes the versioned resume request/result shapes, the opaque
credential, the session/fence generation, the cutover handoff to T024, the
closed errors, the API and the `reconnect_wire` suite. It authorises no source
change. The D12 grace policy itself is **not** here (`tasks/T025b.md`).

Below, S§N cites this contract, T§ cites `tasks/T024.md` S§N, D12 cites
`tasks/POST-T018-DECISIONS.md` §D12, B3/B5 cite `tasks/POST-T018-RULES.md`.
"Refused" = returns an error and changes no state; "rejected" = the gate
produced a `ResumeResult::Rejected` frame.

## S§0. Inputs read

- Root `AGENTS.md`; `tasks/T025a.md`, `T025a-facts.md`, `T025b.md`, `T024.md`
  (S§1–S§16, especially S§10/S§11/S§16), `POST-T018-DECISIONS.md`
  D10/D11/D12/D14, `POST-T018-RULES.md`, `T024q-facts.md`, `T024h-facts.md`.
- ADR-0019, ADR-0022, ADR-0026, ADR-0028; `docs/architecture/crpg-net.md`;
  `crates/crpg-net/AGENTS.md`; `docs/IMPLEMENTATION_PLAN.md` §1.4 and §3.
- Code: `crates/crpg-net/src/{lib,protocol,protocol_v2,protocol_transfer,
  codec,codec_transfer,reassembly,sim}.rs`, `tests/support/driver.rs`,
  `Cargo.toml`. `crpg-net-quic`/`crpg-server` only via the two fact sheets and
  ADR-0026 (read-only boundary evidence).

## S§1. Findings (measured on this branch)

### S§1.1 Versions, lanes, tags, identity

| Fact | Value | Cite |
|---|---|---|
| v1 version / lane | `1` / `0` | `protocol.rs:25,34` |
| v1 seq | `1..=u64::MAX-1`, no wrap | `protocol.rs:41,48` |
| v2 version | `2` | `protocol_v2.rs:34` |
| transfer subprotocol | `0x81`, lane `2`, down `0x10..=0x12`, up `0x20..=0x21` | `protocol_transfer.rs:19,22,25-33` |
| v1/v2 delta tags | `0..=7` / `0..=10` | `protocol.rs:172-196`, `protocol_v2.rs:37-43` |
| `SessionEpoch` | `[u8;16]`; "replay isolation, not authentication credentials"; "session reattachment is not supported in v1" | `protocol.rs:202-207` |
| lane 1 | reserved for D13 movement | `protocol.rs:30-31`; T§2 item 1 |
| frame caps | intent 4,096 up / delta 65,536 down | `protocol.rs:58,64`; `protocol_transfer.rs:46,48` |

No resume version, lane, tag, message, credential, fence or redaction exists:
the public surfaces of `lib.rs:28-37` and the three protocol modules have none;
the fact sheet records the same absence (`T025a-facts.md:115-116,205-214`).

### S§1.2 Command identity and dedup live in host state, not net

- `PeerState { epoch, lane, next_new_seq, cache: Vec<(seq, canonical bytes,
  status)>, fail_budget, byte_budget }` (`sim.rs:235-251`); `new` binds a fresh
  epoch with `next_new_seq = 1` and full budgets (`sim.rs:253-271`).
- The driver checks the cache first: identical bytes are `Cached(original
  status)`; the same seq with different bytes is `SeqConflict` and closes the
  epoch (`driver.rs:433-456`). Finalize caches the exact wire bytes and advances
  `next_new_seq` (`driver.rs:553-570`).
- Receipts keep the original `processed_tick` (`driver.rs:586-604`); the codec
  passes `observed_tick` through (`codec.rs:163,205`).
- **`NetDriver::bind_peer` replaces `PeerState`** (`driver.rs:277-287`). A
  resume that called it would reset the dedup ledger. Net cannot today tell
  "bind a new session" from "reattach".

### S§1.3 Replica state is per receiver

- `add_receiver(epoch, wire, mode)` binds one epoch; a differing frame epoch is
  `WrongEpoch` (`reassembly.rs:491-523,600-602`).
- One transfer in flight (`reassembly.rs:646-648`); `retired_through` is the id
  high-water mark (`reassembly.rs:563-580,641-644,733-736`).
- `publish` is the atomic cutover: decode body, free staging, retire, clear a
  queued snapshot, drop held deltas `<= cursor`, set
  `next_event_seq = cursor + 1`, `Live`, push the snapshot, release contiguous
  held deltas (`reassembly.rs:739-851`).
- `offer_delta` supersedes `eseq < next_event_seq`, holds above it, fails closed
  on overflow (`reassembly.rs:884-998`).
- There is **no rebind**; `remove_receiver` only frees accounting
  (`reassembly.rs:524-533`).

Already reusable: atomic install and cursor gate (`reassembly.rs:739-851,
884-998`), epoch refusal (`:600-602`), retired/behind refusal (`:641-660`),
canonical retry (`driver.rs:435-439,553-570`), the filtered body vocabulary
(`protocol_transfer.rs:277-312`). The new mechanism only carries *who may
reattach, at which connection generation, with which progress*, then hands the
view to T024.

## S§2. Decisions this draft makes

1. **Separate subprotocol, not a main version.** `RESUME_VERSION = 0x82`
   (high bit = subprotocol, as `0x81`), `LANE_SESSION = 3`, up `0x30`, down
   `0x31`.
2. **Two messages only.** `ResumeRequest` up, `ResumeResult` down. The
   replacement view is a normal T024 `0x81` transfer on lane 2
   (`Begin`/`Chunk`/`Published`), unchanged: no new body, no second cutover.
3. **Opaque credential.** 1..=64 raw bytes, redacted `Debug`, no equality or
   verification API. No format, crypto dependency, "trusted" flag or test hook.
4. **Per-session monotonic fence.** `ResumeFence` is host-granted, never 0, from
   1. Request presents the last grant; result returns the new one; the gate
   compares; the adapter tags connections (T025b).
5. **Per-lane progress split by authority.** Request: advisory `LaneClaim`.
   Result: host-authoritative `LaneProgress` (next new seq + lowest retained
   seq). A claim never advances/resets/rewrites host state.
6. **`ResumeGate` is a pure state machine.** Open sessions (epoch → fence), a
   request check and an explicit `HostResumeDecision`. No clock, grace, budget,
   transport or World.
7. **The client re-anchors.** `ReassemblyPool::rebind_receiver` discards
   staging and clears held/queued deltas but keeps epoch, published base and
   `next_event_seq`, so an obsolete snapshot cannot publish behind an applied
   cursor. A client that lost its replica uses `add_receiver(...,
   AwaitSnapshot)`.
8. **Fail closed.** Shape/version/lane/tag/field/session/fence errors return
   `ResumeError` and no frame. Only a host decision produces a result.
   Identity rejections are cause-neutral.
9. **No S-set, no dependency.** T025a is not an **S** package
   (`docs/IMPLEMENTATION_PLAN.md:191`; serialized set §1.4 lines 88-98). No
   dependency (`Cargo.toml:7-23` unchanged), golden, `crpg-contracts`/
   toolchain change, `unsafe`, `HashMap`/`HashSet`, `f32`/`f64`,
   `SystemTime`/threads/I/O.
10. **v1, v2 and `0x81` byte-identical.** No existing codec, tag, seq meaning
    or cap changes.

### S§2.1 Reused unchanged

| Need | Reused shape | Cite |
|---|---|---|
| Replacement view/facts | `SnapshotBody`, `SnapshotEntity`, `HealthFact`, `TurnFact` | `protocol_transfer.rs:277-312` |
| Snapshot messages/tags | `TransferDown`/`TransferUp`, `0x81`, lane 2 | `protocol_transfer.rs:19-33,233-270` |
| Atomic install/cutover | `publish` + delivery-cursor gate | `reassembly.rs:739-851,884-998`; T§10.6–10.7 |
| Obsolete/epoch refusal | `retired_through`, `StaleTransfer`, `BehindReplica`, `WrongEpoch` | `reassembly.rs:600-662,733-736` |
| Shape errors | `CodecError` (six variants) | `codec.rs:51-66` |
| Delta gate types | `DeltaWire`, `GateMode`, `GatedDelta`, `ReplicaUpdate` | `reassembly.rs:249-288` |

The only new wire shapes are S§5's two frames; the only new reassembly method is
`rebind_receiver` (S§11.3).

## S§3. Files (implementation later)

New: `src/protocol_resume.rs` (S§8.1), `src/codec_resume.rs` (S§8.2),
`src/resume.rs` (S§8.3), `tests/reconnect_wire.rs` (S§14),
`docs/adr/0030-resume-subprotocol.md` (next free number after 0029; architect
confirms).

Changed: `src/lib.rs` (three `pub mod` lines in the alphabetical block
`lib.rs:28-37`, no glob, one crate-doc sentence); `src/reassembly.rs`
(`rebind_receiver` only); `crates/crpg-net/AGENTS.md`,
`docs/architecture/crpg-net.md` (S§15).

Not changed: `Cargo.toml`/`Cargo.lock`, `deny.toml`, `tools/`, every existing
`src/` module except `lib.rs`/`reassembly.rs`, every existing test fixture,
every other crate.

## S§4. Version, lane, direction, tags

| Item | Value | Rule |
|---|---|---|
| First byte | `0x82` | any other is `UnsupportedVersion`; v1/v2/`0x81` already refuse `0x82` |
| Lane | `3` | anything else is `WrongDirection` |
| Up (client → host, ≤4,096 B) | `0x30` Request | `0x31` here is `WrongDirection` |
| Down (host → client, ≤4,096 B) | `0x31` Result | `0x30` here is `WrongDirection` |
| Any other tag | — | `UnknownMessage` |

Demultiplexing is at channel selection (T§4, T§16 Q3): the caller picks the
codec for the lane a frame arrived on, here lane 3. No decoder guesses or falls
back (ADR-0019); the first byte is a check inside the chosen codec, not a
discriminator. Disjointness: `0x82 ∉ {1,2,0x81}`, lane `3 ∉ {0,1,2}`, tags
`{0x30,0x31}` disjoint from `0..=10` and the transfer tags. Compile-time
`const _: () = assert!(...)` pins all four facts.

## S§5. Wire layout (exact field order)

`postcard` primitives exactly as `codec_transfer`: raw `u8`/`bool`/enum tag,
LEB128 varint `u64`, raw `[u8;16]`, `Option<T>` as a 0/1 byte then `T` (the
`Resync.after` precedent, `protocol_transfer.rs:261-269`). Each frame is
`0x82`, `3`, tag.

### S§5.1 Upstream `0x30` `ResumeRequest`

| Field | Type | Bound |
|---|---|---|
| `epoch` | `[u8;16]` | claimed epoch; not identity by itself |
| `presented` | `Option<ResumeFence>` | `None` = never resumed; `Some(f)`, `f >= 1` |
| `credential_len` | varint `u64` | `1..=64` |
| `credential` | raw bytes | opaque; no UTF-8/structure/normalization |
| `lane_count` | varint `u64` | `1..=4` |
| per lane, ascending: `lane` | `u8` | `0..=3`; lane 0 required in this version |
| `next_new_seq` | varint `u64` | `1..=u64::MAX`; `u64::MAX` = exhausted |

`next_new_seq` is the next seq the client will send; the claim is advisory
(S§12).

### S§5.2 Downstream `0x31` `ResumeResult`

| Field | Type | Bound |
|---|---|---|
| `epoch` | `[u8;16]` | echo |
| `outcome` | `u8` | 1 `Accepted`, 2 `Rejected` |
| Accepted: `granted` | varint `u64` | `1..=u64::MAX` |
| `lane_count` | varint `u64` | `1..=4` |
| per lane, ascending: `lane` | `u8` | `0..=3`; lane 0 required |
| `next_new_seq` | varint `u64` | `1..=u64::MAX` |
| `retained_from` | varint `u64` | `1..=next_new_seq`; lowest cached seq, `== next_new_seq` when empty |
| Rejected: `reason` | `u8` | 1 `Expired`, 2 `Denied`, 3 `Busy` |

`retained_from` tells "still cached" from "finalized but evicted" (never
re-execute; D12). Sizes (T024's measured varints, `T024.md:145-149`): request
max 140 / min 25; accepted max 115 / min 25; rejected 21. All far below 4,096.

### S§5.3 Literal examples (all-zero epoch)

- Minimal request (credential `AA`, lane 0, next 1):
  `82 03 30` + 16 × `00` + `00 01 AA 01 00 01` (25).
- After one grant, fence 1, next 900 (`84 07`):
  `82 03 30` + 16 × `00` + `01 01` + `01 AA` + `01 00 84 07` (27).
- Accepted, grant 1, lane 0, next 1, retained 1:
  `82 03 31` + 16 × `00` + `01 01 01 00 01 01` (25).
- Rejected `Expired`: `82 03 31` + 16 × `00` + `02 01` (21).

Encode reproduces these exactly; a valid value re-encodes byte-identically.

## S§6. Codec rules and error precedence

Errors are the existing six `CodecError` variants (`codec.rs:51-66`), as
`codec_transfer` re-exports them (`codec_transfer.rs:9`).

1. Whole-frame cap first: over 4,096 either way is `FrameTooLarge`; empty is
   `Malformed`.
2. First byte not `0x82`: `UnsupportedVersion`.
3. Lane not `3`: `WrongDirection`.
4. Tag: the other direction's tag is `WrongDirection`; any other is
   `UnknownMessage`.
5. Fields left to right, each checked when read; first failure wins:
   - a varint overflowing `u64` in its tenth byte is `Malformed`;
   - `credential_len` 0 `Malformed`, over 64 `LimitExceeded`, missing body
     `Malformed`; length checked before any copy;
   - `presented` `Some(0)` `Malformed`;
   - `lane_count` 0 `Malformed`, over 4 `LimitExceeded`, a lane over 3
     `UnknownMessage` (unassigned-`reason` precedent), repeated/descending lane
     or lane 0 absent `Malformed`;
   - `next_new_seq` 0 `Malformed`;
   - result `outcome` not 1/2 `UnknownMessage`, `granted` 0 `Malformed`,
     `retained_from` 0 or `> next_new_seq` `Malformed`, unassigned `reason`
     `UnknownMessage`.
6. Trailing bytes: `Malformed`.

Encode checks the same rules in the same order, then the cap, and never returns
partial bytes. `validate` is those routines, shared by encode/decode/gate.
Counts are compared before allocation; the lane vector grows by push (≤4).

## S§7. Caps (B3/B5)

Wire hard maxima are versioned; raising one is a new subprotocol version and a
review. Policy is tightening-only and never on the wire (B5;
`protocol.rs:97-165`, `reassembly.rs:150-223`).

| Bound | Value | Kind |
|---|---|---|
| Resume frame either way | 4,096 | wire, `= MAX_INTENT_FRAME_BYTES` (`protocol.rs:58`); no cap raised |
| Credential bytes | 1..=64 | wire; a cap, not a format |
| Lanes per message | 1..=4 | wire; ascending, lane 0 required |
| Sessions per gate | 8 (`v1()`) | policy, `POLICY_PROOF_PEERS` (`protocol.rs:152`) |
| Fence values per session | `u64::MAX` (`v1()`) | policy `max_fence`; tightening makes exhaustion testable |
| Time / attempts / budgets | none here | T025b owns grace, retry rate and budgets |

`ResumeLimits` has exactly `{ max_sessions: usize, max_fence: u64 }`, validated
in order: 0 → `InvalidLimits { field }`; `max_sessions > 8` → `LoosensPolicy`;
`max_fence` has no looser value above `u64::MAX`. `v1()` is `{ 8, u64::MAX }`.
There is no resume rate/failure budget: D12 says budgets are not replenished,
and net must not touch the host's `TokenBucket`s (`sim.rs:247-250,297-301`).

## S§8. Interface (exact public Rust)

Every new struct/enum derives `Debug, Clone, PartialEq, Eq` except `Credential`
and `ResumeRequest` (S§9). Errors implement `Display` as
`<VariantName> at resume/<area>` with no payload bytes and `Error` with no
source. Numbers come from existing constants by qualified reference.

### S§8.1 `protocol_resume`

```rust
pub const RESUME_VERSION: u8 = 0x82;
pub const LANE_SESSION: u8 = 3;
pub const RESUME_TAG_REQUEST: u8 = 0x30;
pub const RESUME_TAG_RESULT: u8 = 0x31;
pub const MAX_RESUME_FRAME_BYTES: usize = crate::protocol::MAX_INTENT_FRAME_BYTES;
pub const MAX_RESUME_CREDENTIAL_BYTES: usize = 64;
pub const MAX_RESUME_LANES: usize = 4;
pub const POLICY_RESUME_SESSIONS: usize = crate::protocol::POLICY_PROOF_PEERS;

/// Host-granted connection generation, per session, never 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResumeFence(u64);
impl ResumeFence {
    pub fn new(raw: u64) -> Option<Self>; // None for 0
    pub fn get(self) -> u64;
}

/// Opaque, host-verified resume proof. Not a token format.
#[derive(Clone)] // Debug is written manually and redacted (S§9)
pub struct Credential(Vec<u8>);
impl Credential {
    pub fn new(bytes: &[u8]) -> Option<Self>; // None outside 1..=64
    pub fn as_bytes(&self) -> &[u8];
}
// Deliberately no PartialEq/Eq/Ord/Hash/Display (S§9).

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneClaim { pub lane: u8, pub next_new_seq: u64 }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneProgress { pub lane: u8, pub next_new_seq: u64, pub retained_from: u64 }

#[derive(Debug, Clone)] // not PartialEq: holds a Credential
pub struct ResumeRequest {
    pub epoch: [u8; 16],
    pub presented: Option<ResumeFence>,
    pub credential: Credential,
    pub lanes: Vec<LaneClaim>,
}
impl ResumeRequest { pub fn validate(&self) -> Result<(), crate::codec::CodecError>; }

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeRejection { Expired = 1, Denied = 2, Busy = 3 }
impl ResumeRejection {
    pub fn as_u8(self) -> u8;
    pub fn from_u8(raw: u8) -> Option<Self>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeResult {
    Accepted { epoch: [u8; 16], granted: ResumeFence, lanes: Vec<LaneProgress> },
    Rejected { epoch: [u8; 16], reason: ResumeRejection },
}
impl ResumeResult { pub fn validate(&self) -> Result<(), crate::codec::CodecError>; }
```

### S§8.2 `codec_resume`

```rust
pub use crate::codec::CodecError;
pub fn encode_request(msg: &ResumeRequest) -> Result<Vec<u8>, CodecError>;
pub fn decode_request(bytes: &[u8]) -> Result<ResumeRequest, CodecError>;
pub fn encode_result(msg: &ResumeResult) -> Result<Vec<u8>, CodecError>;
pub fn decode_result(bytes: &[u8]) -> Result<ResumeResult, CodecError>;
```

The bounded-varint reader and `extend` are private duplicates for this module,
as `codec_v2`/`codec_transfer` did; `codec.rs` is not touched.

### S§8.3 `resume`

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeLimits { pub max_sessions: usize, pub max_fence: u64 }
impl ResumeLimits {
    pub fn v1() -> Self; // { POLICY_RESUME_SESSIONS, u64::MAX }
    pub fn validate(&self) -> Result<(), ResumeLimitsError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeLimitsError {
    InvalidLimits { field: &'static str },
    LoosensPolicy { field: &'static str },
}

/// The host's authenticated verdict. Net checks shape/fence/session only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostResumeDecision {
    Accept { lanes: Vec<LaneProgress> },
    Reject(ResumeRejection),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeGate { /* limits, BTreeMap<[u8;16], Session> */ }
impl ResumeGate {
    pub fn new(limits: ResumeLimits) -> Result<Self, ResumeLimitsError>;
    pub fn limits(&self) -> ResumeLimits;
    pub fn open_session(&mut self, epoch: [u8; 16]) -> Result<(), ResumeError>;
    pub fn close_session(&mut self, epoch: [u8; 16]) -> bool;
    pub fn current_fence(&self, epoch: [u8; 16])
        -> Result<Option<ResumeFence>, ResumeError>;
    pub fn check_connection(&self, epoch: [u8; 16], fence: Option<ResumeFence>)
        -> Result<(), ResumeError>;
    /// Returns the exact wire result the caller should encode and send.
    pub fn handle_request(&mut self, bytes: &[u8], decision: &HostResumeDecision)
        -> Result<ResumeResult, ResumeError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeError {
    Codec(crate::codec::CodecError),
    UnknownSession, SessionExists, SessionsFull,
    StaleFence, UnknownFence, Fenced, FenceExhausted, InvalidProgress,
}
```

`ResumeGate` derives `Debug, Clone, PartialEq, Eq`, so a no-change refusal can
be proved with `assert_eq!` against a clone. `Display` is
`<Name> at resume/gate` (including `Codec`); limits add `: <field>`.

## S§9. The opaque resume credential

| Rule | Decision |
|---|---|
| Representation | `Credential(Vec<u8>)`, private; `new` returns `None` outside 1..=64; `as_bytes` for the host verifier |
| Format | None: no UTF-8, charset, base64, prefix, version, embedded epoch, parsing or normalization. `00 FF 80` is as valid as ASCII |
| Identity | Never by a claimed epoch alone: a zero-length credential is unrepresentable (`Malformed`), and `handle_request` cannot return `Accepted` without `HostResumeDecision::Accept`. The host decides with `(session, proof, fence)` |
| Redaction | `Debug` prints exactly `Credential(<redacted>)`; `ResumeRequest` derives `Debug`, so it is redacted transitively. No `Display`; errors carry no bytes; net logs nothing |
| Comparison | No `PartialEq`/`Eq`/`Ord`/`Hash` and no `ct_eq`: net is not the verifier, and an unreviewed timing claim is worse than none |
| Zeroization | Not performed (no `zeroize`; `unsafe` forbidden at `lib.rs:1`); never claim it. The host must not log, checkpoint or persist it (ADR-0026 item 2's rule; T025b for the resume proof) |
| Hooks | None. The only injectable trust is `HostResumeDecision`; no `trusted`, `skip_auth` or test bypass |

Options: a fixed 32-byte array (matching ADR-0026's invitation, uniform
`ct_eq`) versus 1..=64 raw bytes. The fixed size makes net imply a length
contract it does not own (T025b/T040 issue the proof); the range keeps issuance
out. Deriving `PartialEq` was rejected so host code cannot compare proofs
through a non-verifier. Consequence: tests compare `as_bytes()`;
`ResumeRequest` is not `PartialEq`.

## S§10. Session gate and connection fencing

### S§10.1 Lifecycle

- `open_session(epoch)` inserts `current = None`. A live epoch is
  `SessionExists` and is **not touched**: a fence never resets, because a reset
  would let an old connection pass (the `StaleTransfer` precedent,
  `reassembly.rs:641-644`).
- At `max_sessions` a new epoch is `SessionsFull`; the map is unchanged.
- `close_session` removes and returns whether it existed. The host calls it at
  D12 expiry (T025b); after that a resume is `UnknownSession`. Net has no timer.
- `current_fence` separates unknown session (`Err`) from never resumed
  (`Ok(None)`).

### S§10.2 Fence progression

The session's fence is `None` until the first accepted resume, then a
`ResumeFence` from 1 upward. `handle_request` compares the presented value:

| Current | Presented | Result |
|---|---|---|
| `None` | `None` | new: grant `Some(1)` |
| `Some(n)`, `n < max_fence` | `Some(n)` | new: grant `Some(n+1)` |
| `Some(n)`, `n == max_fence` | `Some(n)` | new: `FenceExhausted` |
| `Some(1)` | `None` | **replay** of the first grant: `Some(1)` |
| `Some(n)`, `n >= 2` | `Some(n-1)` | **replay** of the last grant: `Some(n)` |
| `Some(n)`, `n >= 2` | `None` | `StaleFence` |
| any | lower than the replay row | `StaleFence` |
| any | higher, or `Some` when current is `None` | `UnknownFence` |
| any | `Some(0)` | impossible from the codec (`Malformed`) |

The replay row makes a lost result recoverable with no client nonce: re-send a
request presenting the same value and receive the same grant (the progress echo
is freshly authoritative). A retry never extends grace (T025b's clock; net has
none) and never grants twice. An older value is not a replay and is refused.

`check_connection(epoch, tag)` is the per-frame gate the adapter calls after
tagging a connection: unknown epoch → `UnknownSession`; `tag == current` →
`Ok`; `tag < current` (including `None < Some(_)`) → `Fenced`; `tag > current` →
`UnknownFence`. A resume request on a fenced connection is therefore refused
before the gate is asked; net supplies the comparison, the adapter the identity
(T025b). The fence is not a credential and not secret: it orders generations,
it does not prove who may reattach.

### S§10.3 `handle_request` precedence (first failure wins)

1. `decode_request`: `Codec(e)`, no frame.
2. Epoch not open: `UnknownSession`, no frame.
3. Presented fence per S§10.2: `StaleFence`/`UnknownFence`/`FenceExhausted`,
   no frame.
4. The supplied decision:
   - `Reject(reason)` → `Ok(ResumeResult::Rejected { epoch, reason })`; fence
     unchanged; the caller encodes it.
   - `Accept { lanes }` → validate `lanes` as S§5.2/§6 (`InvalidProgress`,
     nothing changed); on a new attempt set `current = granted`, on a replay
     keep it → `Ok(ResumeResult::Accepted { epoch, granted })`.

The decision is last so a shape/fence failure never receives a policy verdict,
and a rejection never moves the fence. A duplicate request replays the grant; a
duplicate result is inert bytes and decoding it twice is idempotent.

## S§11. Resync and cutover

### S§11.1 Host side

On `Accepted` the host may, and only then, treat the new connection as the
session's current generation and send the replacement view. It must:

1. cancel or let expire any transfer in flight for the old connection (T§11
   item 4; `Cancel(Aborted)` is `reassembly.rs:852-883`);
2. start a normal T024 transfer on the same ordered connection with the
   **retained epoch**, a strictly higher `TransferId`, and `cursor` = the last
   `event_seq` issued before capture (T§11 items 1-3);
3. hold deltas above that cursor in egress under the existing caps (T§11
   item 7).

The result carries no snapshot, body or cursor: `Begin` carries them
(`protocol_transfer.rs:88-107`), so there is one cutover implementation. No
client cursor field is needed: the host's last issued `event_seq` is never
below the client's applied cursor, so capture covers everything.

### S§11.2 Client side

1. Decode `Accepted`; check `epoch` is the retained one (the pool would also
   say `WrongEpoch`).
2. Retained replica: `rebind_receiver(id)`. Lost replica:
   `add_receiver(epoch, wire, AwaitSnapshot)`.
3. Process no post-result frame before step 2: the result precedes `Begin`, and
   `Begin` precedes every delta above its cursor in the one total order
   (T§10.7, T§11 item 3). If that order breaks, the pool holds or refuses
   (`BehindReplica`), never corrupts (`reassembly.rs:649-660`).

### S§11.3 `ReassemblyPool::rebind_receiver` (only `reassembly.rs` change)

```rust
impl ReassemblyPool {
    /// Re-anchor a retained receiver for a resumed connection.
    pub fn rebind_receiver(&mut self, id: ReceiverId) -> Result<(), TransferError>;
}
```

Exact, in one call: (1) unknown id → `UnknownReceiver`, nothing changes; (2) if
a transfer is in flight, discard it, free staging and raise `retired_through`
to its id (its chunks become `StaleTransfer`, `reassembly.rs:733-736`);
(3) drop every held **and queued** delta and subtract the bytes from receiver
and pool (they apply to the replaced base; the snapshot is the whole state);
(4) mode `AwaitSnapshot`, clear `resync`; (5) keep `epoch`, `wire`, `published`
and `next_event_seq`. Step 5 is the floor: a `Begin` with
`cursor + 1 < next_event_seq` or a `base_tick` below the published one is
`BehindReplica`, so an obsolete snapshot cannot roll back an applied cursor and
old deltas stay `Superseded`. Return `Ok(())`.

`remove_receiver` + `add_receiver` was rejected: it resets the floor to
`next_event_seq = 1`, `published = None`, so an old snapshot could publish
behind an applied cursor and an old transfer id could be reused.

### S§11.4 Atomic install and obsolete transfers

- Publication is unchanged T024 (`reassembly.rs:739-851`): body decoded and
  validated, reservation freed, id retired, a queued snapshot (and its deltas)
  cleared, held `<= cursor` dropped, `next_event_seq = cursor + 1`, `Live`,
  snapshot pushed, contiguous held released. Nothing failed reaches the queue.
- Cross-epoch: `offer_transfer` returns `WrongEpoch`
  (`reassembly.rs:600-602`). A new epoch is a new receiver; a resume never
  changes the epoch.
- Obsolete partial: after rebind its id is retired, so a late chunk is
  `StaleTransfer`; `Begin` at/below it is `StaleTransfer`; `Begin` behind the
  replica is `BehindReplica`; another while in flight is `InFlight`. No path
  overwrites a later resync.
- Revocation during the gap: rebind drops queued/held deltas and retires the old
  transfer; the host captures under current grants. Net checks shape, not
  entitlement (T§10.9, T§11 item 6).

## S§12. Command retry identity and "no reset"

| Net must not | Why | Cite |
|---|---|---|
| call `bind_peer`/replace `PeerState`/reset `TokenBucket`s | D12 retains seq/cache/receipts/budgets; replacement resets dedup | `driver.rs:277-287`; `sim.rs:235-301`; D12 |
| rotate the epoch | new lane numbering loses the ledger; rotation is T025b's expiry path | `protocol.rs:202-205`; `driver.rs:426-432` |
| rewrite seq or `observed_tick` | a retry is the original canonical bytes; the codec passes the tick through | `codec.rs:163,205`; `driver.rs:435-439` |
| mutate the dedup cache/receipt | a retry returns the original status and `processed_tick` | `driver.rs:447,586-604` |
| treat `LaneClaim` as authority | a client cannot advance/erase/reset host progress; only `LaneProgress` is echoed | S§5.1, S§10.3 |

After `Accepted` the correct retry is the **byte-identical** `IntentFrame`
(epoch, lane, seq, observed_tick, actor, body): the host cache returns
`Cached(original status)` with no execution; a changed byte is `SeqConflict`.
The gate holds only `(epoch, Option<ResumeFence>)`, so it has no lane state to
reset.

## S§13. Errors

`ResumeError` (S§8.3): `Codec(CodecError)`, `UnknownSession`, `SessionExists`,
`SessionsFull`, `StaleFence`, `UnknownFence`, `Fenced`, `FenceExhausted`,
`InvalidProgress`. `ResumeLimitsError`: `InvalidLimits { field }`,
`LoosensPolicy { field }`. Wire `ResumeRejection` (frozen 1..=3): `Expired`,
`Denied`, `Busy`. `CodecError`/`TransferError` are reused unchanged;
`rebind_receiver` adds no variant.

| Outcome | State change | Frame |
|---|---|---|
| `Err(Codec)` or any gate error (`UnknownSession`, `SessionExists`, `SessionsFull`, `StaleFence`, `UnknownFence`, `Fenced`, `FenceExhausted`, `InvalidProgress`) | none | none |
| `Ok(ResumeResult::Rejected { reason })` | none; fence unchanged | Rejected |
| `Ok(ResumeResult::Accepted { granted })` | fence advanced only on a new attempt | Accepted |

`Display` names only the variant (and field for limits), never bytes;
`std::error::Error` has no source. `CodecError` is unit-only, so no credential
byte can travel through an error.

## S§14. Tests — public `reconnect_wire`

`crates/crpg-net/tests/reconnect_wire.rs`, public API only. Reuses
`tests/support/driver.rs` and `fixture.rs` read-only via
`#[path = "support/..."] mod ...` (as `conformance.rs:13-16`) and adds a
test-local `ResumeHost`: `ResumeGate`, an explicit `HostResumeDecision` per
call, a retained `NetDriver` (same `PeerId`, never `bind_peer`), a client
`ReassemblyPool` and a fixture-table body builder. The oracle is independent: a
`BTreeMap` replica applying popped `ReplicaUpdate`s, compared per fact with
hand-authored expected facts; no production projection, no World hash. No test
claims authentication: the decision is an input, and the suite says so.

| T025a §Test (`T025a.md:38-42`) | Cases |
|---|---|
| exact messages round-trip | 1, 2, 3, 4 |
| unknown/oversized/truncated/cross-epoch/stale-fencing, each with control | 5, 9, 14, 17 |
| cutover and command retry identity | 11, 12 |
| duplicate/late/old-transfer delivery | 13, 14, 18 |
| independent oracle + corruption control | 15 |
| explicit decision input, never auth | 7 (all cases) |

1. `wire_identifiers_and_caps_are_pinned` — `0x82 ∉ {1,2,0x81}`; lane
   `3 ∉ {0,1,2}`; tags `{0x30,0x31}` disjoint from `0..=10`, `0x10..=0x12`,
   `0x20..=0x21`; caps 4,096/64/4; `ResumeLimits::v1()` = `{8, u64::MAX}`;
   v1/v2/transfer constants unchanged. Control: the pinned v1/`0x81` values
   equal their sources.
2. `literal_byte_oracles` — every S§5.3 example encodes to its bytes and
   decodes back; re-encode is identical; the fence-1/next-900 tail is `84 07`.
3. `request_round_trip_bounds` — maximal request (epoch `FF`, `Some(u64::MAX)`,
   64-byte credential, lanes 0..=3 at `u64::MAX`) is 140 bytes and round-trips;
   minimal 25. With the valid value as control: credential 0 (`Malformed`), 65
   (`LimitExceeded`), missing byte; lane count 0 (`Malformed`), 5
   (`LimitExceeded`); lane 4 (`UnknownMessage`); repeated/descending lane and
   absent lane 0 (`Malformed`); next 0 (`Malformed`); `Some(0)`
   (`Malformed`); tenth-byte overflow (`Malformed`).
4. `result_round_trip_bounds` — accepted maxima are 115 bytes and round-trip;
   each reason round-trips. Controls: outcome 0/3 (`UnknownMessage`), granted 0
   (`Malformed`), reason 0/4 (`UnknownMessage`), retained 0 and `> next`
   (`Malformed`), lane count 0/5, trailing byte.
5. `decode_precedence_and_direction` — 4,097 bytes is `FrameTooLarge` even with
   a bad first byte; versions 1, 2, `0x80`, `0x81`, `0x83` are
   `UnsupportedVersion`; lanes 0, 1, 2, 4 are `WrongDirection`; `0x31` in a
   request and `0x30` in a result are `WrongDirection`; tags `0x00`, `0x22`,
   `0x32`, `0xFF` are `UnknownMessage`; empty and every truncation are
   `Malformed`; combinations report the earliest stage. Control: each valid
   frame decodes.
6. `credential_opaque_redacted_and_unrepresentable` — `00 FF 80` and a 64-byte
   credential round-trip byte-identically; `format!("{:?}", credential)` is
   `Credential(<redacted>)` and `format!("{:?}", request)` contains no
   credential bytes; every `ResumeError` `Display` is byte-free; a 0-length
   credential is unbuildable and decodes `Malformed` (epoch-only resume is not
   expressible).
7. `host_decision_is_explicit_and_required` — same valid bytes with
   `Reject(Denied)` give `Rejected{Denied}` and no fence change; with `Accept`,
   `Accepted` (control); with `Accept` and a bad `LaneProgress` list,
   `InvalidProgress` and no change; no call accepts without `Accept`. The
   comment states it proves the mechanism reflects a verdict, not auth.
8. `session_lifecycle_never_resets_a_fence` — open, fence `None`; duplicate open
   is `SessionExists` with the gate `assert_eq!` to its clone; after one accept
   `Some(1)`; close then resume is `UnknownSession`; eight fit, the ninth is
   `SessionsFull`; close of an unknown epoch is `false`.
9. `fence_progression_and_connection_fencing` — `None → Some(1)`; old tag
   `None` is `Fenced`; `None` again replays `Some(1)`; second accept
   `Some(1) → Some(2)`; `Some(1)` now replays `Some(2)`; `None` is
   `StaleFence`; `Some(3)` is `UnknownFence`; `Some(0)` is unconstructible;
   with `max_fence: 2`, new `Some(2)` is `FenceExhausted` while replay
   `Some(1)` returns `Some(2)`; every refusal leaves the gate equal to its
   clone.
10. `claim_never_advances_host_state` — a request claiming next 900 with
    `Accept { lanes: [(0, 42, 7)] }` echoes `(0, 42, 7)` and the session equals
    the clone apart from the fence; a second accept keeps echoing the host's
    values; a replay returns the same grant.
11. `snapshot_before_post_cutover_deltas` — V2 receiver at `next_event_seq = 6`;
    resume accepted; rebind; `Begin(cursor 9)` + 2 shuffled chunks, then deltas
    10, 11 and late 5. Nothing pops before the snapshot; order is Snapshot,
    delta 10, delta 11; delta 5 is `Superseded`; the oracle matches. Negative
    control: applying delta 10 first makes the oracle fail.
12. `command_retry_identity_across_resume` — admit one intent at `(epoch, lane
    0, seq n, observed_tick t)`, capture receipt and `state_hash`, disconnect,
    accept a resume (no re-bind), re-send identical bytes: `Cached(Applied)`,
    identical receipt, `executions()` and `state_hash` unchanged, and `n + 1`
    is still admitted as new (not `SeqGap`). Negative control: `observed_tick +
    1` at the same seq is `SeqConflict` and closes the epoch; hash unchanged.
13. `duplicate_and_late_result_delivery` — accepted bytes decode twice to the
    same value; the identical request replays the grant without a second
    advance; after `Some(2)`, `None` is `StaleFence` while `Some(1)` replays
    `Some(2)`; a result after a newer grant is inert bytes (the client takes
    its fence only from its own attempt). Control: the first request accepts.
14. `obsolete_transfer_cannot_overwrite_later_resync` — receiver at
    `next_event_seq = 10`, transfer 5 in flight (3/6 chunks); rebind frees
    staging and retires 5; late chunk 4 is `StaleTransfer`; `Begin(6, cursor
    9)` publishes; the oracle never shows 5's body. Cross-epoch: `Begin`/chunk/
    `Cancel` with E2 are `WrongEpoch`, E1 accepted (control); `Begin(5)` after
    rebind is `StaleTransfer`; `Begin(6, cursor 8)` is `BehindReplica`.
15. `filtered_replica_oracle_and_corruption` — a 2-entity body (health for A,
    not B; turn active A) after a resume; the oracle matches permitted facts
    (B's health absent, not zero). Corruption control: one health byte corrupted
    consistently (duplicates identical) still decodes and publishes, and the
    oracle reports the mismatch; the unmodified transfer matches. Revocation
    control: A's health revoked during the gap, the new body omits it, the
    oracle finds it absent; the pre-resume delta would have shown it.
16. `replay_from_a_fenced_connection_is_refused` — after the accept,
    `check_connection` with the old tag is `Fenced`; a byte-identical intent
    admitted before the fence is not re-admitted (`executions()` unchanged);
    the same bytes on the current connection return the cached receipt
    (control); a higher tag is `UnknownFence`.
17. `v1_v2_and_transfer_refuse_each_other` — v1 `decode_intent`/`decode_delta`,
    v2 and transfer `decode_down`/`decode_up` each refuse every valid resume
    frame with `UnsupportedVersion`; resume refuses valid v1/v2/`0x81` frames
    the same way; a valid v1 delta with op tag `0x30` is `UnknownMessage` in v1
    and v2. Control: each codec decodes its own frame.
18. `fabric_resume_with_lost_result_and_snapshot` — request/result/`Begin`/
    chunks/deltas through `InMemoryTransport` (`v1_egress()`, `generous_rates()`
    as T024 S§1.5). With dup/reorder the snapshot publishes once. With the
    result lost, the identical request replays the grant and the snapshot
    publishes; after a later second accept, `None` is `StaleFence`. Control:
    the clean run publishes with no duplicate update.

Commands (`T025a.md:44-51`) then the common native gates:

```text
cargo test -p crpg-net --test reconnect_wire --locked
cargo test -p crpg-net --test snapshot_transfer --locked
cargo test -p crpg-net --test codec --locked
cargo clippy -p crpg-net --all-targets --locked -- -D warnings
cargo test -p crpg-net --locked
```

Expected: `reconnect_wire` 18 passed; `snapshot_transfer` 31 and `codec` 28
unchanged; existing suites unchanged; both native targets, pinned toolchain,
normal test profile, default features (ADR-0012); no golden.

## S§15. Docs and ADR-0030

- `docs/adr/0030-resume-subprotocol.md`, Accepted only with the approved
  contract: S§2 items 1-8, S§4, S§10, S§11 in ADR-0028's shape; links this
  contract, no copied layout.
- `crates/crpg-net/AGENTS.md`: the three modules; traps — never `bind_peer` on
  resume, never reset the fence, never derive `PartialEq` on `Credential`,
  `rebind_receiver` keeps the floor; signed entry.
- `docs/architecture/crpg-net.md`: "Resume/resync (T025a)" with module flow,
  fence table, T024 handoff and host requirements; signed entry.

## S§16. Out of scope and boundaries

T025a does not do, and its API must not grow to do: grace timing, credential
issuance/verification, authentication, session lookup policy, AI takeover,
restart persistence, a new snapshot body/version, a new transport, prediction,
or gameplay.

**T025b (`crpg-server`)** (`tasks/T025b.md:9-31`): the 30,000 ms grace clock and
`< 30,000`/`>= 30,000` boundary, calling `close_session` at expiry; issuing,
storing, verifying (constant-time or otherwise), lifetime and revocation of the
proof, mapping ADR-0026's invitation principal; producing
`HostResumeDecision` and choosing `Expired` only from its clock; binding the new
connection to `granted`, fencing the old one with `SessionFenced` (9) (ADR-0026
items 4/7), and calling `check_connection` on every inbound frame; retaining the
exact `PeerState` and never calling `bind_peer`; capturing the full filtered
snapshot under current grants, allocating the transfer id, sending `Begin`
first, never resetting `event_seq` (T§11); D10/D12's no-AI/no-restart rules; the
`host_reconnect` suite.

**T024q (`crpg-net-quic`)** (`T024q-facts.md` §3, §7, §8): carry lane-3 `0x82`
frames on the same ordered stream as lanes 0/2 under an explicit channel
selector, no payload sniffing (T§16 Q3); extend ALPN/channel selection for lane
3 (`crpg-lane0/2` has no constant yet; `T024q-facts.md:288-296`); any
framing/stream/close-code change still needs a new ALPN (ADR-0025); resume
frames fit 4,096/65,536; reuse `SessionFenced` (9); per-connection ordering only.

**T024h (`crpg-server`)** (`T024h-facts.md` §8): the T024 S§11 sender (transfer
ids, snapshot capture into `SnapshotBody`, `cursor`/`base_tick` alongside
`event_seq`, post-cutover holding); seamless narrowing (keep `event_seq`,
capture under fresh grants, `Cancel` an affected transfer instead of fencing;
`host.rs:699-712`, `quic.rs:1110-1132`); there is no sender today, so T025b's
snapshot step depends on it.

Order: T024q/T024h before T025b (T§16 Q5, approved); T025a is the net mechanism
they call.

**S-set/dependency:** none. T025a is not an S package
(`docs/IMPLEMENTATION_PLAN.md:190-192`); `crpg-net` keeps `crpg-core` + `serde`
+ `postcard` (`Cargo.toml:7-23`); the dev-only `crpg-data`/`crpg-sim`/
`serde_json` edges are already used. No golden, hash, `crpg-contracts`,
toolchain, `deny.toml` or `tools/` change.

## S§17. Open questions for the user (each with a recommendation)

1. **Version form.** (a) Separate subprotocol `0x82` (recommended). (b) Transfer
   v2 (`0x82`) on lane 2 with resume tags. (c) Main protocol v3. (b) makes every
   `0x81` check learn a second tag space and puts session control in the
   snapshot version; (c) repeats D11's rejected shape. *Recommendation: (a).*
   The disjoint first byte fails every mix-up at the first byte, both ways.
2. **Lane.** (a) New `LANE_SESSION = 3` (recommended). (b) Share lane 2 with
   the transfer, dispatching on the first byte. (b) makes the selector peek
   payload bytes and lane 2 mean two things; (a) costs unstarted T024q one
   selector entry. *Recommendation: (a).*
3. **Credential shape/equality.** (a) Opaque 1..=64 bytes, redacted `Debug`, no
   equality/verification API (recommended). (b) Fixed 32 bytes matching
   ADR-0026, with `ct_eq`. (c) Derive `PartialEq`. (b) implies a length/format
   contract net does not own and invites net to verify; (c) lets host code
   compare proofs through a non-verifier. *Recommendation: (a).* No zeroization
   is claimed either way.
4. **Lost-result recovery.** (a) Replay of `current - 1` returns the same grant,
   no nonce (recommended). (b) A client nonce plus a cached result. (c) No
   retry, so a lost result costs the session. (b) adds a field and client state
   for a case (a) covers; (c) is user-hostile on ordinary loss.
   *Recommendation: (a).* A retry never extends grace.
5. **Snapshot handoff.** (a) Add `rebind_receiver`, keeping
   `published`/`next_event_seq` (recommended). (b) `remove_receiver` +
   `add_receiver`, losing the floor. (b) resets `next_event_seq` to 1 and
   `published` to `None`, so an obsolete snapshot can publish behind an applied
   cursor; (a) costs one reviewed method. *Recommendation: (a).*
6. **Rejection causes.** (a) `Expired`, cause-neutral `Denied`, `Busy`
   (recommended). (b) Distinct `BadCredential`/`UnknownSession`/`Revoked`/
   `Expired`. (b) tells an unauthenticated caller whether an epoch exists and
   whether a grant was revoked, contradicting ADR-0026 item 3's single
   `AuthRefused`; the client needs only "try again", "start fresh", "later".
   *Recommendation: (a).*

## Agent log

- 2026-10-30 (UTC) · opencode-go/deepseek-v4.1-flash + T025a draft · Drafted the independent 0x82/lane-3 resume contract with an opaque credential, a monotonic per-session fence, bounded per-lane progress, a T024 handoff with a receiver rebind, closed errors, an 18-case `reconnect_wire` plan and explicit T025b/T024q/T024h boundaries.
