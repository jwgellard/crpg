# T047-draft-b — Bridge skeleton contract draft (b) (`crpg-godot`)

## Task / Crate(s) / Purpose

Sole crate: `crpg-godot`, plan row `docs/IMPLEMENTATION_PLAN.md:249`
(C → I, S). Opens the crate: the gdext extension, `apps/client/`, the
embedded authority wrapped once (ADR-0033, `docs/adr/0033-godot-bridge.md`
accepted, D41d 2026-11-02), `LocalLink`, proxy spawn from a bulk snapshot
(spec §19.1 #22, `docs/CRPG_ENGINE_SPEC.md:1666`), and the Windows client
build (gate 12, `docs/CRPG_ENGINE_SPEC.md:1325`). Inputs landed: T047a
(`Host::legal_intents`, `Host::replica_id`,
`crates/crpg-server/src/host.rs:671,715`, contract `tasks/T047a.md`) and
T046 (`ReplicaWorld`, `crates/crpg-net/src/replica.rs:121`, contract
`tasks/T046.md`). T047c (client CI job, plan row `:251`) is a separate
S-set task in this batch; this draft names only what T047 hands it.

Status: **Proposed blind draft (b)** — not approved, changes no ADR-0033
decision. Where the ADR leaves a choice, each item below gives at least
two options with consequences and a recommendation.

## Exact contract

### Decision

#### 1. Pins: `godot` version/features, engine, API level, MPL-2.0, protected files

| Choice | Options | Consequences | Recommendation |
|---|---|---|---|
| `godot` version | (a) `=0.5.5` exact; (b) caret `0.5` | (a) D27d-compliant (`tasks/DECISIONS-2026-10-04.md:69`), reproducible; (b) drifts on next `cargo update` | **(a)** |
| Features | (a) `["api-4-7"]` only; (b) + `double-precision`; (c) + `experimental-threads` | (a) matches engine 4.7.2 with the prebuilt `gdextension-api-0.5.1/src/4.7/extension_api.json` (verified in registry cache); (b) changes float ABI, sim stays `f32`-spatial-only; (c) enables threads the slice forbids (ADR §4) | **(a)** |
| `engine` feature | (a) optional, off by default; (b) unconditional | (a) `cargo test --workspace` stays Godot-free on Linux per ADR-0012 (`docs/adr/0012-windows-primary-platform.md:23`); (b) breaks Linux headless | **(a)** |
| Engine | (a) 4.7.2.stable.official.ed1daf0bf (`Godot4 --version` on PATH, 2026-11-01, `tasks/T045-facts.md:209-210`); (b) newer 4.x | (a) pinned per ADR-0001/D27d; (b) unreviewed API drift | **(a)** |
| `gdextension-api` | (a) transitive via `godot-bindings` (`version = "0.5.0"` requirement, resolved 0.5.1 in cache); (b) direct pin | (a) no new direct edge; (b) second pin to maintain for no gain | **(a)** |
| MPL-2.0 (U1) | (a) per-crate exceptions for exactly the 8 family packages; (b) widen `allow`; (c) refuse + C++ bridge | (a) minimal, matches ADR U1 rec; (b) policy drift; (c) months of work (spec risk 17, `docs/CRPG_ENGINE_SPEC.md:1772`) | **(a)**, user decides (Q1) |
| `default-features` / `lazy-function-tables` | (a) keep defaults now, measure the build; (b) pre-emptively trim | (a) `default = ["__codegen-full"]` is what generates the bindings (registry `godot-0.5.5/Cargo.toml.orig`); trimming first risks a broken codegen nobody measured; (b) speculative | **(a)**, record the measured build, revisit only if too heavy |

All eight family licences verified MPL-2.0 in the registry cache:
`godot`, `godot-core`, `godot-bindings`, `godot-cell`, `godot-codegen`,
`godot-ffi`, `godot-macros` (all `=0.5.5`), `gdextension-api` (`0.5.1`).

Protected files: **`crates/crpg-godot/Cargo.toml`** (adds the optional
`godot` edge, `[lib] crate-type`, `engine` feature — required because the
`[dependencies]` table is empty today, `crates/crpg-godot/Cargo.toml:7`);
**`Cargo.lock`** (records the `godot` tree — required by `--locked` gates);
**`deny.toml`** (per-crate exceptions only if U1(a) is accepted — required
because `MPL-2.0` is deliberately not in `allow`,
`deny.toml:20-33`). The workspace `Cargo.toml` is **not** touched: no
workspace-level pin is needed for one optional edge. `tools/lint/deps.py`
needs no `ALLOWED` change (`"crpg-godot": (None, set())` already permits
every edge, `tools/lint/deps.py:71`).

#### 2. Module layout (ADR §1)

| Choice | Options | Consequences | Recommendation |
|---|---|---|---|
| Halves split | (a) `core/authority.rs` + `core/client.rs` + `core/link.rs`; (b) one `core.rs` | (a) `client.rs` naming neither `crpg_server` nor `crpg_sim` is the compile-visible authority boundary (source-scan test); (b) boundary by discipline only | **(a)** |
| Error table | (a) `core/errors.rs` closed code table; (b) `Display` strings across the FFI | (a) GDScript switches on stable ints, text via `last_error()`; (b) string parsing in GDScript | **(a)** |
| Shell | (a) one `shell.rs` (`#[cfg(feature = "engine")]`), conversion + guard only; (b) logic in `#[func]` bodies | (a) Godot-free `core` testable on both targets; (b) untestable without the engine | **(a)** |
| `unsafe impl` home | (a) `shell.rs`; (b) `lib.rs` | The single `unsafe impl ExtensionLibrary` (`pub unsafe trait`, `godot-core-0.5.5/src/init/mod.rs:557`) must live with the other engine code; (a) keeps the root to declarations | **(a)** |

Files and ownership:

```text
crates/crpg-godot/src/
  lib.rs                  crate root; keeps #![allow(unsafe_code)] (macro expansion,
                          tasks/T045-facts.md:106-108); declares core, shell; entry point
  core/mod.rs             #![forbid(unsafe_code)]; BridgeCore; always compiled
  core/authority.rs       trusted half: owns Host, campaign walk, NPC driver.
                          May use crpg_server, crpg_sim. No godot types
  core/client.rs          untrusted half: ReplicaWorld, intent encoding, CrpgView.
                          Bytes only; names neither crpg_server nor crpg_sim
  core/link.rs            LocalLink: bounded FIFO Transport pair (item 4)
  core/errors.rs          BridgeCode table (item 7); Display for last_error()
  shell.rs                #[cfg(feature = "engine")]; CrpgSession + CrpgView
                          gdext classes; conversion and panic guard only
```

`pub`: `BridgeCore::{open,poll,submit_option,view,close,last_error}`,
`AuthorityHalf`/`ClientHalf`/`LocalLink`/`CrpgView` types and `BridgeCode`
(integer constants + `Display`). The single `unsafe` token is the
`unsafe impl ExtensionLibrary` in `shell.rs` with a `// SAFETY:` comment.

#### 3. Construction (ADR §2): campaign → authority path

| Choice | Options | Consequences | Recommendation |
|---|---|---|---|
| Encounter assembly | (a) private copy of `crates/crpg-cli/src/combat_apply.rs:211` assembly in `authority.rs`; (b) hoist into a library now | (a) no `crpg-sim`/`crpg-data` touch, no CLI golden move; three small copies until the follow-on; (b) touches two crates + golden review inside an S-set task | **(a)**, follow-on hoists later |
| Name resolution | (a) private copy of `crates/crpg-cli/src/demo.rs:217` locale rule; (b) depend on CLI | (a) same as above (`crpg-cli` has no library target, `tasks/T045-facts.md:153`); (b) impossible without restructuring the CLI | **(a)** |
| Start-event handling | (a) acknowledge before wrap (`Host::from_history` refuses non-empty journal, `crates/crpg-server/src/host.rs:571-573`); (b) deliver start events to the client | (a) first turn reaches the client as `Turn` state; (b) needs event-op history the client never asked for | **(a)** |
| Session kinds | (a) player `bind_snapshot_peer` (`host.rs:776`), NPC `bind_peer` (`host.rs:691` per ADR §2 step 7); (b) both `bind_peer` | (a) exercises the `/2` remote shape the product will use; (b) tests a shape the product will not ship | **(a)** |

`BridgeCore::open(campaign_dir, encounter_id, player_side, seed)` runs on
the calling thread, in order: read campaign dir (regular files only, no
symlink following, classified by
`crates/crpg-data/src/validation.rs:261`) → `load_campaign` + `validate`
(any finding stops) → assemble `EncounterSpec` → `HistoryWorld::new(seed)`
+ `start_encounter` (turn-based only) → record `(EntityId, placement)` →
`acknowledge(last_sequence)` → `Host::from_history(history,
HostConfig { protocol: V2, incarnation: 1 }, 0)` → bind player snapshot
peer (controlling chosen side) + NPC peer (all other combatants) →
build `PresentationTable`, hand to client half by value. `V2` is required
(the combat log needs event ops); `incarnation` is `1` (every `open` is a
new authority, saves cut). GDScript chooses neither. Empty `encounter_id`
means first in document order; empty `player_side` means the first
combatant's side (the `crpgc play` rule).

#### 4. Transport (ADR §3): `LocalLink`

| Choice | Options | Consequences | Recommendation |
|---|---|---|---|
| Shape | (a) `LocalLink`: two bounded in-process FIFO byte queues implementing `Transport` (`crates/crpg-net/src/transport.rs:14`), 128 frames per direction (T022 per-peer log cap); (b) reuse `crpg_net::sim::InMemoryTransport` | (a) no clock, no faults in the product path; (b) drags `SimClock` fault machinery into the shipped bridge (`tasks/T045-facts.md:148`) | **(a)** |
| Lane/channel | (a) lane-0 only, `Transfer` is fatal fence `54`; (b) channel tag now | (a) grants never narrow in the slice (ADR §8); (b) speculative generality for T048+ | **(a)** |
| Ack rule | (a) hand off, then acknowledge by last accepted ordinal, never what the link refused (QUIC rule, `crates/crpg-server/AGENTS.md:170-171`); (b) acknowledge after apply | (a) matches the adapter BR-7 already proves; (b) a refused frame would stall the log | **(a)** |

Client half holds one end, never a `Host` or `PeerHandle`; authority half
holds the other and feeds `Host::ingest`. The client never acknowledges.

#### 5. Threading and time (ADR §§4–5)

| Choice | Options | Consequences | Recommendation |
|---|---|---|---|
| Threads | (a) one thread (Godot main), no `Send` at the FFI, `experimental-threads` off; (b) worker thread owning the host | (a) no double buffering/second clock; (b) cross-thread handoff for a turn-based slice | **(a)** |
| Clock | (a) `Instant` taken at `open` in `shell.rs`, elapsed ms into `poll(now_ms)`; (b) engine clock or FFI timestamp | (a) no FFI timestamp (GDScript cannot cause `TimeRegression`, `host.rs:679`), tests script the clock; (b) lets GDScript reach host time | **(a)** |
| Pump cadence / tick | (a) pump once per `poll()`, no throttle, never call `Host::tick`; (b) 50 ms throttle / tick per frame | (a) the 50 ms note concerns QUIC hellos (`crates/crpg-server/AGENTS.md:226`), `tick` journals nothing (`host.rs:1950`); (b) misreads both | **(a)** |

#### 6. Per-frame flow (ADR §6)

GDScript calls `session.poll()` once per `_process`; then reads
`session.view()`. `poll` does, in order:

```text
shell: now_ms <- Instant; enter panic guard (item 7)
core.authority.step(now_ms):
  1. recv from link -> Host::ingest(player, bytes, now_ms) (host.rs:976)
       Refused => report INTENT_REFUSED (no reply exists; clears pending)
  2. NPC step (item 8): at most one Host::ingest(npc, ...) per poll
  3. Host::pump(now_ms) (host.rs:1026)
       backpressured => not an error; retried next poll
       npc result Rejected => fatal HOST_FAILED
       dropped_replies > 0 => report INTENT_REFUSED
  4. per peer: Host::take_delivery_entries (host.rs:1767)
       player: Delta -> link.send; Transfer -> fatal UNEXPECTED_TRANSFER
       Host::acknowledge_delivery(peer, last ordinal handed off) (host.rs:1793)
       npc: acknowledge last ordinal, unread
  5. Host::acknowledge_captures(Host::last_capture()) (host.rs:1986,2015)
       required: captures cap admission at 4,096 (crpg-server AGENTS.md trap)
  6. Host::legal_intents(player) (host.rs:671) -> report.options
core.client.step(report):
  7. recv from link -> codec_v2::decode_delta -> apply to replica (T046)
       collect event ops and receipts in frame order
  8. if anything changed: revision += 1; rebuild CrpgView
```

Step 5 has no consumer in the slice; the watermark call is the whole
policy. Step 6's `report.options` is one-way by value and confers nothing
(execution revalidates). `submit_option` only writes to the link; it
reaches the host on the next `poll` — one queue, one ordering.

Options considered: (a) the flow above; (b) `poll` returning the view
directly — rejected: two calls give one explicit order (`poll` then `view`)
and no signal is needed (item 7).

#### 7. FFI surface (ADR §10) and errors (ADR §11)

| Choice | Options | Consequences | Recommendation |
|---|---|---|---|
| Class set | (a) `CrpgSession` + `CrpgView` only; (b) + `Document`/`Assets`/`Preview`/`Diagnostics` stubs | (a) spec §11.3's five types arrive with their owning tasks; (b) dead surface needing H-unsafe review | **(a)** |
| Signals | (a) none; (b) state-changed signal | (a) `poll`-then-`view` is one explicit order; (b) a mid-`poll` signal races the view it describes | **(a)** |
| Submission | (a) `submit_option(revision, index)`; (b) generic `submit(bytes)`; (c) typed `submit_declare_action` | (a) no ability/target/seq/epoch in GDScript; (b) puts encoding in GDScript; (c) invites client-side legality | **(a)** |
| Errors | (a) closed int table + `last_error()` text; (b) strings as control flow | (a) stable codes, text never controls; (b) GDScript parses text | **(a)** |

Every member runs on the Godot main thread; not callable from a worker.

### Interface

Manifest (`crates/crpg-godot/Cargo.toml`):

```toml
[lib]
crate-type = ["cdylib", "rlib"]

[features]
engine = ["dep:godot"]

[dependencies]
crpg-core   = { path = "../crpg-core" }
crpg-data   = { path = "../crpg-data" }
crpg-net    = { path = "../crpg-net" }
crpg-server = { path = "../crpg-server" }
crpg-sim    = { path = "../crpg-sim" }
godot = { version = "=0.5.5", optional = true, features = ["api-4-7"] }
```

Rust surface (`core`, always compiled):

```rust
pub struct BridgeCore; // owns AuthorityHalf + ClientHalf + Instant
impl BridgeCore {
    pub fn open(campaign_dir: &str, encounter_id: &str, player_side: &str, seed: u64)
        -> Result<Self, BridgeCode>;
    pub fn poll(&mut self) -> BridgeCode; // BridgeCode::Ok on success
    pub fn view(&self) -> &CrpgView;
    pub fn submit_option(&mut self, revision: u64, index: usize) -> BridgeCode;
    pub fn close(&mut self);
    pub fn last_error(&self) -> &str;
}
pub struct CrpgView { /* packed arrays + revision; see Behaviour item 10 */ }
```

GDScript surface (`shell.rs`, `#[cfg(feature = "engine")]`):

| Class | Member | Meaning |
|---|---|---|
| `CrpgSession` | `open(campaign_dir: String, encounter_id: String, player_side: String, seed: int) -> int` | Item 3. Returns a code |
| | `poll() -> int` | Item 6. `0` or a code; a fatal code repeats until `close` |
| | `view() -> CrpgView` | View from the last changing `poll`; empty, revision 0 before that |
| | `submit_option(revision: int, index: int) -> int` | Sends option `index` of the view with that `revision` |
| | `close() -> void` | `Host::shutdown`, drop the core. Idempotent; also runs on drop. `open` may follow |
| | `last_error() -> String` | Human text for the last non-zero code, else empty. Never control flow |
| `CrpgView` | `revision() -> int` | Increases when state, options, events or receipts change |
| | `net_ids() -> PackedInt64Array` | Visible entities, ascending; every per-entity array below is indexed the same way |
| | `names() -> PackedStringArray` | Display name, from the presentation table |
| | `sides() -> PackedStringArray` | Display side |
| | `placements() -> PackedStringArray` | Placement ULID text, the later key for art |
| | `health() -> PackedInt64Array` | Current health; `0` when `health_known` is false |
| | `max_health() -> PackedInt64Array` | Maximum health; same rule |
| | `health_known() -> PackedByteArray` | `1` where health is disclosed. Undisclosed is never zero |
| | `dead() -> PackedByteArray` | Terminal flag, `0` where unknown |
| | `active() -> int` | Active `NetId`, or `0` for none |
| | `round() -> int` | The wire round, unadjusted. The HUD adds 1, as `crpgc play` does |
| | `ended() -> bool` | An `EncounterEnded` event was applied |
| | `option_labels() -> PackedStringArray` | `"<ability> -> <target>"` or `"End turn"`, the `crpgc play` wording |
| | `option_targets() -> PackedInt64Array` | Target `NetId` per option, `0` for End turn |
| | `events() -> Array` of dictionaries | Events that produced this revision, oldest first |
| | `receipts() -> Array` of dictionaries | Receipts that produced this revision, oldest first |

`events()` entries always carry `kind`: `action_resolved` (`actor`,
`target`, `ability` display name, `ability_id`, `outcome`, `damage`),
`turn_started` (`actor`, `round`), `died` (`entity`), `encounter_ended`
(`encounter`, `round`). `outcome` is the wire's symbolic text. `receipts()`
entries are `seq`, `applied` (bool), `code` (frozen `RejectionCode` number,
or `0`). Health arrays are 64-bit because the wire's `u32` does not fit a
32-bit Godot integer. A negative or out-of-range GDScript `int` is
`BAD_ARGUMENT` with no core call.

`submit_option` checks, in order: open → `revision` matches the current
view → `index` in range → no intent pending. The client half then builds
the `IntentFrame` with the epoch given at `open`, its own seq, and
`observed_tick` from the last frame's `server_tick`, and encodes with
`codec_v2::encode_intent`. One intent may be outstanding; the receipt or an
`INTENT_REFUSED` report clears it. A gameplay rejection is a receipt, not
an error; `poll` still returns `0`.

Error mapping (closed table, `0` = `OK`, in `core/errors.rs`):

| Code | Name | Source | Session | GDScript action |
|---|---|---|---|---|
| 1 | `NOT_OPEN` | call before `open` | unchanged | caller bug |
| 2 | `ALREADY_OPEN` | second `open` | unchanged | caller bug |
| 3 | `BAD_ARGUMENT` | bad `int` / ULID text | unchanged | caller bug |
| 4 | `STALE_OPTIONS` | `revision` mismatch | unchanged | re-read the view |
| 5 | `INTENT_PENDING` | intent outstanding | unchanged | wait |
| 6 | `NO_SUCH_OPTION` | `index` out of range | unchanged | caller bug |
| 20 | `CAMPAIGN_IO` | directory read failed | not opened | show text |
| 21 | `CAMPAIGN_INVALID` | `load_campaign` error / validation finding | not opened | show text |
| 22 | `NO_ENCOUNTER` | campaign has none | not opened | show text |
| 23 | `UNKNOWN_ENCOUNTER` | `encounter_id` unknown | not opened | show text |
| 24 | `UNKNOWN_SIDE` | `player_side` unknown | not opened | show text |
| 25 | `START_FAILED` | `start_encounter` / `from_history` / bind failed | not opened | show text |
| 30 | `INTENT_REFUSED` | `IngestDisposition::Refused`, full link, dropped reply | open | another choice |
| 50 | `HOST_CLOSED` | `HostError::Closed` | fatal | stop; show text |
| 51 | `HOST_FAILED` | any other `HostError`, rejected NPC intent | fatal | stop; show text |
| 52 | `FRAME_UNDECODABLE` | `CodecError` on a delivered frame | fatal | stop; show text |
| 53 | `REPLICA_REJECTED` | T046 refused a frame | fatal | stop; show text |
| 54 | `UNEXPECTED_TRANSFER` | `DeliveryKind::Transfer` in the slice | fatal | stop; show text |
| 55 | `PANICKED` | panic caught by the guard | fatal | stop; show text |

Fatal: the core calls `Host::shutdown` (`host.rs:2041`), drops the host,
and every later `poll` returns the same code until `close`. A bad frame is
never skipped (the replica would silently diverge). `RejectionCode`
numbers (`crates/crpg-net/src/protocol.rs:393`) to player text is T051's job.

Panic/`unsafe` policy: exactly one `unsafe` token in hand-written source
(the `unsafe impl ExtensionLibrary`, `pub unsafe trait` at
`godot-core-0.5.5/src/init/mod.rs:557`); `core/mod.rs` carries
`#![forbid(unsafe_code)]`; the root stays `#![allow(unsafe_code)]` for macro
expansion. Every `#[func]` is conversion plus one guarded `core` call; no
indexing/`unwrap`/`expect`/arithmetic on GDScript values in the shell. The
pinned crate catches unwinds at the boundary (`handle_fallible_varcall`,
`handle_fallible_ptrcall`, `godot-core-0.5.5/src/private.rs:465,484`),
which would leave the session live but possibly half-updated — so every call
also goes through one `guard` using `catch_unwind`: on panic it drops the
core, records `PANICKED`, returns `55`. No authority survives a panic. No
`panic = "abort"`. `crpg-godot` stays outside `determinism.py`
(`CRATES` at `tools/lint/determinism.py:57` excludes it).

### Behaviour

1. **Open order.** `open` follows item 3 exactly; any failure returns codes
   20–25 with no host built, and a later valid `open` succeeds. A second
   `open` without `close` is `ALREADY_OPEN` (2).
2. **First delivery shape.** The bind stages the first permitted state
   (`host.rs:691-776`); the first applied `poll` yields every participant
   once, ascending `NetId`, non-zero `active`, as `Turn` state (start events
   were acknowledged pre-wrap).
3. **Authority equivalence.** Same seed + same action script through the
   bridge and through an oracle `HistoryWorld` given the same actions and
   acknowledgements give equal `Host::authority_hash`. The bridge changes no
   outcome (`crpgc play` calls `perform_action` directly,
   `crates/crpg-cli/src/play.rs:128`; the client must not).
4. **NPC driver.** Lives in `core/authority.rs`, trusted side, own peer,
   same `ingest` path. If `legal_intents(npc)` is non-empty and no intent is
   due, due time becomes `now_ms + NPC_ACTION_DELAY_MS` (`600`); at
   `now_ms >= due` it submits one intent: the first `DeclareAction` whose
   target is on another side, else the first option (the `crpgc play` rule,
   `crates/crpg-cli/src/play.rs:210`; sides from the presentation table).
   Submitting clears due time. `observed_tick` is `Host::server_tick()`;
   `seq` is the driver's own counter. At most one NPC ingest per `poll`.
5. **Idle stability.** An idle `poll` changes `authority_hash`,
   `executions` and the view `revision` not at all.
6. **Link behaviour.** A full link stops hand-off; the acknowledged ordinal
   equals the last accepted; the remainder arrives later, in order, once.
7. **Transfer fence.** A `Transfer` entry is fatal `54`, never silently
   dropped (grants never narrow in the slice).
8. **Capture watermark.** After each `poll`,
   `capture_acknowledged == last_capture`.
9. **Client-half purity.** `core/client.rs` names neither `crpg_server` nor
   `crpg_sim` (source scan). `EntityId` never crosses into the client half
   or the FFI; every id GDScript sees is a `NetId`
   (`crates/crpg-net/src/protocol.rs:217`).
10. **View coherence.** All per-entity arrays are equal length in `net_ids`
    order; under partial grants undisclosed health is `health_known = 0`
    with no zero presented; `u32::MAX` health survives in the 64-bit array;
    each `net_ids` row names the placement `replica_id` maps to it
    (`host.rs:715`); `turn().active` naming a non-live id is no highlighted
    actor, not an error (T046 item 8, `tasks/T046.md:239-243`).
11. **Submission precedence.** `submit_option` returns 1, 4, 6, 5 in that
    order and sends nothing on any of them. A crafted intent naming an
    unknown `NetId` yields a receipt with code 17, `poll` returning 0, the
    session live.
12. **Fault fencing.** A corrupt frame on the link gives fatal `52`; the host
    is closed; every later `poll` repeats it. Any T046 refusal gives fatal
    `53`. An injected core panic returns `55`, later calls repeat it, the
    core is gone. `shell.rs` has exactly one `unsafe` token (source scan).
13. **Proxy spawn (spec §19.1 #22).** From one bulk `view()`: one proxy per
    `net_ids` row, holding `NetId`, name, side, placement text and the
    disclosed health/dead snapshot. Placement is from the presentation table
    (`BTreeMap<NetId, { placement: Ulid, name, side }>` built at `open`
    from `(EntityId, placement)` + `replica_id(player, entity)` + locale/
    creature/faction documents); the NPC driver keeps its own table keyed by
    its peer's ids. Proxy lifecycle (diff/free/`dead()`) is T048's; scene
    sync beyond spawn is T048's; interpolation is T049's.

### Tests

Godot-free `core` tests run under plain `cargo test -p crpg-godot`
(no `engine` feature, both targets). Engine tests run
`Godot4 --headless` on Windows; what needs CI plumbing is T047c's.

- **T1 (B1).** `open` on `campaigns/fixtures/combat_basic` yields every
  participant once, ascending `NetId`, non-zero `active`. *Control:* each
  code 20–25 from a bad input (missing dir, invalid campaign, no
  encounter, unknown encounter, unknown side, bind failure seed); no host
  built; a later valid `open` succeeds.
- **T2 (B3).** Same seed + scripted option choices through the bridge equal
  the oracle `HistoryWorld`'s `authority_hash`. *Control:* a different
  choice changes the hash, so the comparison is not vacuous.
- **T3 (B4).** NPC chooser against fixed option lists: first attack on
  another side, else first option. *Control:* all-targets-same-side list
  picks the first option.
- **T4 (B4).** Scripted clock: no NPC intent before 600 ms, one at 600 ms,
  at most one per `poll`. *Control:* player intent path is unpaced.
- **T5 (B5).** Idle `poll` × N: hash, executions, revision unchanged.
  *Control:* one played option changes all three.
- **T6 (link).** Fill the link to 128: hand-off stops at the cap, ack equals
  last accepted, drain arrives in order once. *Control:* an unfilled link
  hands everything off in one `poll`.
- **T7 (B7).** Inject a `Transfer` entry: fatal `54`. *Control:* a
  Delta-only run never produces `54`.
- **T8 (B8).** After each `poll`, `capture_acknowledged == last_capture`.
  *Control:* a run of 4,097+ admissions would stall without the watermark;
  assert admission still succeeds (spot-check, not a full stall run).
- **T9 (B9).** Source scan: `core/client.rs` contains no `crpg_server` or
  `crpg_sim`. *Control:* `core/authority.rs` contains both.
- **T10 (B11).** `submit_option` before `open` → 1; wrong revision → 4;
  bad index → 6; with intent outstanding → 5; sends nothing in each case
  (link empty). *Control:* the valid call sends exactly one frame.
- **T11 (B11).** Crafted unknown-`NetId` intent: receipt code 17, `poll`
  returns 0, session live. *Control:* a kept option returns `Applied`.
- **T12 (B12).** Corrupt frame on the link: fatal `52`, host closed, every
  later `poll` repeats `52`. *Control:* the same bytes undecoded never reach
  the replica (codec fault, not replica fault).
- **T13 (B12).** T046 refusal on a delivered frame: fatal `53`.
  *Control:* the oracle replica accepts the uncorrupted stream.
- **T14 (B10).** Array lengths equal in `net_ids` order under full and
  partial grants; undisclosed health is `health_known = 0`, never 0 shown.
  *Control:* full grants show `health_known = 1` with the same values.
- **T15 (B10).** `u32::MAX` health in the 64-bit array. *Control:*
  neighbouring rows keep their own values (no spill).
- **T16 (events).** Events + receipts appear in frame order, once, in the
  revision that applied them. *Control:* a `poll` with no new frames adds
  none and bumps no revision.
- **T17 (B10).** Each `net_ids` row names the placement `replica_id` maps
  to it. *Control:* re-query after narrowing-free play keeps the mapping.
- **T18 (args).** Negative / oversized `int` args give 3 with no core call
  (link empty, revision unchanged). *Control:* boundary-valid ints call
  through.
- **T19 (B12).** Injected core panic → `55`; later calls repeat `55`; core
  gone (next `open` required). *Control:* a non-panicking call returns its
  own code.
- **T20 (B12).** Source scan: exactly one `unsafe` token in `shell.rs`;
  `#![forbid(unsafe_code)]` in `core/mod.rs`. *Control:* `lib.rs` keeps
  `#![allow(unsafe_code)]` for macro expansion.
- **T21 (graph).** Default-feature graph on both targets has no `godot`
  package (left to T047c's CI job; run locally with `cargo tree` if
  available). *Control:* `--features engine` graph has exactly `godot =0.5.5`.
- **T22 (engine).** `Godot4 --headless --path apps/client -s tests/smoke.gd`
  spawns one proxy per participant from one bulk `view()`, plays
  `combat_basic` to `ended()` choosing option 0, exits 0 (BR-22). Runs on
  Windows only. *Control:* choosing an out-of-range option exits non-zero
  via `NO_SUCH_OPTION`, proving the smoke path submits for real.
- **T23 (L1–L6 oracle).** T047a's `host_legal.rs` plus T046's `replica.rs`
  pass unmodified. *Control:* they are untouched by this task's diff.

Engine-only: T22 (needs the built extension + Godot binary). T21's CI
proof and the negative `deny` control (remove the exception, watch `deny`
fail) are T047c's.

### Definition of done

On both native targets (default features):

```text
cargo fmt --all
cargo clippy -p crpg-godot --all-targets --locked -- -D warnings
cargo test -p crpg-godot --locked
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
cargo deny check
git diff --check
```

Plus, on Windows with the engine: `cargo build -p crpg-godot --features
engine --locked -j 2`, `Godot4 --headless --path apps/client --import`,
and T22. Also:

- `git diff --exit-code <base> -- crates/crpg-core crates/crpg-rules
  crates/crpg-sim crates/crpg-net crates/crpg-server Cargo.toml Cargo.lock`
  exits 0 except `Cargo.lock`'s additive `godot` tree entries: no wire,
  golden, fixture, sim, core, net or server change.
- `rg -n "HashMap|HashSet" crates/crpg-godot/src` finds nothing outside
  generated code; no `f32`/`f64` in game-logic paths.
- `docs/architecture/crpg-godot.md` (new: what the crate owns, the halves,
  the flow) and `crates/crpg-godot/AGENTS.md` (new: traps — never call
  `Host` from `client.rs`, never skip a bad frame, never acknowledge what
  the link refused, watermark every `poll`, one `unsafe` only in
  `shell.rs`), each signed. No new ADR: ADR-0033 §7 already covers T047a;
  this contract cites ADR-0033 throughout.

### Out of scope

- T048 proxy lifecycle/scene sync (diff `net_ids`, free on leave,
  `dead()`), T049 interpolation (`positions()`/`previous_positions()` come
  later; adding to `CrpgView` is why it is an object), cut-down T051 HUD/
  log/end screen (GDScript over `view()`, no FFI change, no T104/T108
  prerequisite), T050 camera/click-to-move, T026 prediction, T060+ editor.
- Movement, transforms, click-to-move, camera rig (one fixed camera); art,
  audio, VFX (primitives + event hooks with nothing attached); remote
  connect/resume; save/load; `Document`/`Assets`/`Preview`/`Diagnostics`;
  exported `.exe` (demo runs through installed Godot; export templates
  unneeded); real AI (driver replaced behind the host, FFI unchanged);
  perception, grant narrowing, snapshot transfer; real-time encounters; more
  than one human peer.
- Any wire/protocol/golden/sim/core/net/server change; any new dependency
  beyond the pinned `godot` edge; any `deny.toml` change beyond U1(a)
  exceptions; any CI workflow change (T047c's).

### What must not change

Every existing public item, signature, cap and `Display` string in
`crpg-core`, `crpg-data`, `crpg-rules`, `crpg-sim`, `crpg-net`,
`crpg-server`; every test, fixture and golden; the T047a/T046 surface
lists; `tools/lint/deps.py`'s `ALLOWED` table; `deny.toml` policy except
U1(a) exceptions. `Host` stays non-`Clone`; no new `Host` method, ledger,
map, cache or clock. Crate roots keep `#![forbid(unsafe_code)]` everywhere
except `crpg-godot`. A needed change outside `crates/crpg-godot`,
`apps/client`, `docs/architecture/crpg-godot.md` and
`crates/crpg-godot/AGENTS.md` is a stop, not a fix.

### Routing and size budget

`crpg-godot` + `apps/client`, plan §1.4 S-set (`docs/IMPLEMENTATION_PLAN.md:249`
is C → I, S). H-unsafe applies to the one `unsafe impl`. One session if
possible; if more than one, split one crate each in landing order: (1)
Godot-free `core` + tests, (2) `shell.rs` + `apps/client` + smoke test.
Left to: T048 proxy lifecycle, T049 interpolation, cut-down T051 HUD/log,
T050 camera/input, T047c CI job, follow-on CLI-glue hoist. Sizes:
`core/` ~400–550 lines with docs; `shell.rs` ~250–350; tests ~400–600;
`apps/client` ~5 files + smoke script; docs ~40 lines; this contract 0 code.

### Open questions for the user

- **Q1. Accept MPL-2.0 exception U1(a)?** Eight named packages only, general
  rule unchanged. **Recommendation: accept** — the family needs it and (a)
  grants nothing more. Without it T047 cannot land.
- **Q2. Accept H-unsafe on the one `unsafe impl`?** Verified `pub unsafe
  trait` in the pinned source; the impl carries a `// SAFETY:` comment.
  **Recommendation: approve on that basis**, no broader `unsafe` permitted.
- **Q3. Add `client` to the master ruleset's required checks now (C7)?**
  T047c lands in this batch; the gate is red until it does.
  **Recommendation: yes**, so the batch cannot merge half-green.
- **Q4. Keep the CLI-glue copies until the follow-on?** Hoisting now would
  touch `crpg-sim`/`crpg-data` and risk a CLI golden inside an S-set task.
  **Recommendation: keep the copies**, hoist in the unnumbered follow-on.
- **Q5. If the engine build is too heavy, trim with `lazy-function-tables`
  or `default-features = false`?** Both exist in the pinned manifest; the
  effect is unmeasured. **Recommendation: measure first**, record the build
  in the completion record, trim only on evidence.

**Unverified** (could not be checked in the repo or `~/.cargo/registry`;
verify at implementation, shapes above do not depend on the answers):
exact `#[gdextension]`/`GodotClass`/`#[godot_api]`/`#[func]` syntax and
`RefCounted` init form (macros exist in `godot-macros-0.5.5`); dictionary/
typed-`Array` conversion method names; `.gdextension` keys, entry symbol,
`compatibility_minimum`, `reloadable`, and `res://../../target/...`
library-path loading (incl. `CARGO_TARGET_DIR`); off-main-thread `#[func]`
behaviour; `multiple-versions = "deny"` / `unmaintained = "all"` against
the `godot` tree (T047c C5 negative control); first load of `api-4-7` on
4.7.2 (proved by T22).

## Agent log

- 2026-11-02 (UTC) · opencode-go/muse-spark-1.3-contributor + T047 draft · Wrote blind contract draft (b) from ADR-0033, T045 facts and the as-built T047a/T046 surfaces with per-item options and recommendations, so the architect comparison has an independent pin to judge.
