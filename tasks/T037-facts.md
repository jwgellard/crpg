# T037 facts — `crpg-server` host movement adapter

Fact sheet for the T037 contract. Route: doc-only. Branch
`task/T037-facts-f1`. Every citation is `path:line` on this branch; no
recommendations, no proposed types, no opinions.

## Requirements

- **ADR-0036 §H heading** (`docs/adr/0036-movement-and-navigation.md:419`):
  "H. Host adapter (T037, `crpg-server`)".
- **H1 area load** (`docs/adr/0036-movement-and-navigation.md:421`–`424`):
  bake from area data (v1 `bake_box` from `Area.bounds`), hash
  `NavRevision::of_bytes(canonical)`, `bind_nav`, `place` an off-mesh mover at
  `nearest_on_mesh`, announce the revision with the area.
- **H2 `MoveTo`** (`docs/adr/0036-movement-and-navigation.md:426`–`432`):
  control check, at most one navigation query per actor per tick, earlier
  intents `Superseded`, `find_path(mover.position,
  nearest_on_mesh(snap(measure, destination)))`, `set_route`; success
  `Accepted { start_tick: t + 1 }`, failure `Rejected`.
- **H3 `DeclareMove`** (`docs/adr/0036-movement-and-navigation.md:434`–`435`):
  same path computation, then `perform_move`.
- **H4 samples** (`docs/adr/0036-movement-and-navigation.md:436`–`437`):
  each tick, send granted entities that moved plus every granted position at a
  refresh interval T037 pins.
- **H5 shared constants** (`docs/adr/0036-movement-and-navigation.md:439`–`442`):
  `COORD_LIMIT_RAW`, the point limit and the length limit asserted equal in nav
  and sim; the `crpg-server`→`crpg-nav` path edge needs approval at contract
  review.
- **§S3 destination snap** (`docs/adr/0036-movement-and-navigation.md:327`–`333`):
  host (H2) and client preview pass the clicked point through
  `snap(measure, point)` before `nearest_on_mesh`; `snap` is a pure `crpg-sim`
  export, identity in v1.
- **Follow-on table row T037** (`docs/adr/0036-movement-and-navigation.md:541`):
  "T037 | `crpg-server` | H1-H5, S3 | T026p, T035".
- **HM-1 session version** (`tasks/T026p.md:968`): one protocol version per
  session; a v4 session uses lane-0 v4, lane 1, `0x82` snapshots,
  `ReassemblyPoolV4`, `ReplicaWorldV4`, `MoveOutbox`, with
  `codec_v4::frame_spans` and `project_page_v4` replacing the v3 names.
- **HM-2 lane-1 admission** (`tasks/T026p.md:973`–`984`): per peer, mirrors
  lane 0's order at `crates/crpg-server/src/host.rs:1122–1170`, nine steps, and
  its own retry cache and sequence domain; lane 0 stays window 1.
- **HM-3 aggregates** (`tasks/T026p.md:985`–`987`): lane-1 frames also count
  toward `POLICY_INTENT_*`, the ingress caps and the failure budgets.
- **HM-4 receipts** (`tasks/T026p.md:988`–`996`): exactly one per admitted
  intent in `seq` order on the lane-1 stream, `processed_tick` at the decision,
  `Accepted { start_tick: t + 1 }`, earlier intents `Superseded`, and the
  decision refusal order `Superseded`, `WrongMode`, `IllegalAction`, else
  `Accepted`.
- **HM-5 samples** (`tasks/T026p.md:997`–`1001`): datagrams only, never on a
  stream or into lane-0 `event_seq`; `project_samples` with the refresh interval,
  `sample_datagrams`, `processed_seq` the highest lane-1 `seq` receipted.
- **HM-6 rows** (`tasks/T026p.md:1002`–`1008`): one `PositionRow` per visible
  entity with a mover; `moving` true exactly with a route; `granted` true
  exactly when the viewer is granted presence.
- **HM-7 pages** (`tasks/T026p.md:1009`–`1012`): every v4 page built by
  `project_page_v4`; a page in the same tick as `project_snapshot_v4`, with
  `Clock` and `HostFacts`.
- **HM-8 `Moved`** (`tasks/T026p.md:1013`–`1021`): each
  `HistoryEvent::Moved` becomes `EventCandidate::Moved` at the history slot,
  entity from the post-page `InterestView`, `disclose` from the viewer's grant;
  this replaces the `Moved` → `None` arm at
  `crates/crpg-server/src/host.rs:2489`.
- **HM-9 `DeclareMove`** (`tasks/T026p.md:1022`–`1024`): control check, the H2
  path computation, `perform_move`, retry and dedup as every lane-0 verb.
- **HM-10 no combat effect** (`tasks/T026p.md:1025`–`1026`): lane-1 traffic
  simultaneous with a lane-0 retry never spends a resource twice or reorders a
  lane-0 receipt.
- **Out of scope T037** (`tasks/T026p.md:1465`–`1470`): ADR H1–H5 and HM-1..HM-10
  including choosing v4 per session, lane-1 admission, the refresh interval,
  the position-grant defaults and mapping `HistoryEvent::Moved`; it waits for
  T141 as well as T026p.
- **T026p "Who builds what"** (`tasks/T026p.md:88`–`91`): `crpg-server` (T037)
  admits lane-1 intents, runs navigation, calls `set_route`/`clear_route`/
  `perform_move`, builds `PositionRow`, grants, receipts and `Moved`
  candidates, and sends frames and datagrams.
- **T026p Reading 1** (`tasks/T026p.md:172`): "There is no negotiation; T037
  picks the versions per session."
- **T026p Reading 15** (`tasks/T026p.md:315`): the earlier host-level tests
  (independent sequence domains, aggregate caps, no double spend) "are proven
  in T037 (HM-2..HM-4, HM-10)".
- **T026p manager edits** (`tasks/T026p.md:1566`): the T037 row's dependency on
  T141.
- **T026p Question 1** (`tasks/T026p.md:1574`): "the host task (T037) sets the
  default".
- **T035 intro** (`tasks/T035.md:24`): "the host (T037) copies points into sim's
  `MoveRoute` … and hashes the canonical bytes into a `NavRevision`".
- **T035 shared limits** (`tasks/T035.md:127`): nav keeps sim's values under
  nav's names "so T037 can assert equality at compile time (H5)".
- **T035 Out of scope** (`tasks/T035.md:667`–`668`): T037 owns baking from
  `Area.bounds`, `NavRevision` hashing, `bind_nav`, `place`, `snap` and H5's
  compile-time asserts.
- **T035 docs** (`tasks/T035.md:693`): `docs/architecture/crpg-nav.md` lists
  consumers T037, T026, T050.
- **T035 ordering** (`tasks/T035.md:715`): "T037 needs T035c and T036."
- **T036 `snap` export** (`tasks/T036.md:190`–`191`): `snap` (S3) is exported
  as the identity "because only `crpg-sim` can hold it and T037 calls it".
- **T036 Out of scope** (`tasks/T036.md:1065`–`1069`): T037 owns baking and
  binding at area load, `place`, `snap` + `nearest_on_mesh` + `find_path` +
  `set_route` or `perform_move`, receipts, sample refresh and H5's asserts.
- **T036 ordering** (`tasks/T036.md:1132`–`1133`): "T037 after T036c (it calls
  `perform_move`)".
- **crpg-nav data flow** (`docs/architecture/crpg-nav.md:21`–`26`): the host
  (T037) bakes/loads a `NavMesh`, hashes canonical bytes into `NavRevision`,
  asks `find_path`, copies points into sim's `MoveRoute`; the shared limits
  keep sim's values.
- **crpg-nav planned** (`docs/architecture/crpg-nav.md:103`–`104`): baking from
  `Area.bounds`, `NavRevision` and `MoveRoute` are out of scope for `crpg-nav`
  (T037, T036).
- **crpg-nav consumers** (`docs/architecture/crpg-nav.md:109`): "Consumers:
  T037 (host integration), T026, T050."

## Host today

All in `crates/crpg-server/src/`.

- **`Host`** (`host.rs:779`–`803`): `config`, `history`, `closed`,
  `last_now_ms`, `next_handle`, `sessions: BTreeMap<PeerHandle, Session>`,
  `ingress: VecDeque<StagedCommand>`, `ingress_bytes`, `egress_bytes`,
  `executions`, `last_combat_error`, the capture journal (`captures`,
  `capture_lens`, `capture_bytes`, `capture_acknowledged`, `last_capture`),
  `ingress_caps`/`egress_caps: QueueCaps`, `rates: RateCaps`, and T141b's
  `settings`, `paused`, `timer`, `timer_epoch`.
- **`PeerHandle(u64)`** (`host.rs:144`), opaque and never reused; `get()`
  (`host.rs:148`). Bound peers are `MAX_PEERS = POLICY_PROOF_PEERS` (8)
  (`host.rs:94`).
- **`Session`** (`host.rs:605`–`637`): `epoch`, `control`, `grants`, `map` and
  `reverse` (`BTreeMap`), `next_net`, `next_new_seq`, `cache: VecDeque<CacheEntry>`,
  `cache_bytes`, four `TokenBucket`s (`intent_frames`, `intent_bytes`,
  `fail_frames`, `fail_bytes`), `ingress_frames`/`ingress_bytes`, `log:
  VecDeque<LoggedFrame>`, `log_bytes`, `next_event_seq`, `delivered`,
  `generation`, `resync_pending`, `snapshot: bool`, `next_ordinal`,
  `next_transfer`, `in_flight`, and `v3: Option<Box<V3Session>>`.
- **`StagedCommand`** (`host.rs:553`): `peer` plus copied `bytes`. **`CacheEntry`**
  (`host.rs:560`): `seq`, canonical bytes, `ReceiptV3` status, `processed_tick`,
  optional `(generation, ordinal)` receipt, ledger `bytes`. **`LoggedFrame`**
  (`host.rs:578`): `ordinal`, `event_seq: Option<u64>`, `bytes`.
- **Protocol version per session.** `HostConfig.protocol` is `V1 | V2`
  (`host.rs:106`, `:134`–`139`); `Host::protocol` returns it
  (`host.rs:877`). A binding does not negotiate: `bind_peer`/`bind_snapshot_peer`
  call `bind` (`host.rs:1048`) and speak `HostConfig.protocol`; `bind_v3_peer`
  (`host.rs:1153`) always makes a v3 snapshot session whatever the config says
  (`host.rs:1141`–`1143`). `session_wire` reports `1`, `2` or `3`
  (`host.rs:1195`–`1201`). `decode` picks the session's codec: `codec_v3` for a
  v3 session, else `codec`/`codec_v2` by `HostConfig.protocol`
  (`host.rs:1518`–`1549`).
- **Lane-0 admission order.** `Host::pump` (`host.rs:1456`) drains the FIFO
  and stops at the first `Step::Blocked` (`host.rs:1463`–`1494`); `admit`
  (`host.rs:1588`) is the pipeline, first failure wins: (1) decode with the
  session's codec (`host.rs:1600`–`1610`); (2) "session epoch" — no session or a
  different epoch is `SessionExpired` (`host.rs:1612`–`1626`); (3) cache first,
  identical bytes re-send the retained receipt, different bytes
  `SeqConflict` and fence (`host.rs:1628`–`1640`); `next_new_seq > SEQ_LAST`
  is `SeqExhausted`, a higher `seq` `SeqGap`, a lower `StaleSeq`
  (`host.rs:1641`–`1661`); (4) freshness `FutureTick`/`StaleTick` against
  `POLICY_OBSERVED_TICK_WINDOW` (`host.rs:1662`–`1669`); (5) mapping/control/
  disclosure resolution (`host.rs:1670`–`1681`); then `validate_action`
  (`host.rs:1682`–`1691`); (6) the reservation and execution/commit split
  (`host.rs:1692`–`1728`).
- **Note on the T026p citation.** `tasks/T026p.md:974` cites
  `crates/crpg-server/src/host.rs:1122–1170` for this order; on this branch
  those lines are inside `blank_session`/`bind_v3_peer`
  (`host.rs:1122`–`1126` are `fail_bytes` bucket setup, `:1153` is
  `bind_v3_peer`), and the pipeline is `admit` at `host.rs:1588`.
- **Retry cache.** `CacheEntry` per acknowledged intent; `cache_hit`
  re-sends the retained receipt without re-execution
  (`host.rs:1821`–`1879`); `cache_insert` evicts oldest until
  `POLICY_RETRY_CACHE_PER_PEER_LANE` (256) entries and
  `POLICY_RETRY_CACHE_BYTES_PER_PEER_LANE` (2 MiB) hold
  (`host.rs:3963`–`3974`); the constants are `crpg-net`'s
  (`crates/crpg-net/src/protocol.rs:147`, `:149`) and are imported at
  `host.rs:47`–`48`.
- **Rate buckets.** Per session, `intent_frames`/`intent_bytes` and
  `fail_frames`/`fail_bytes`, built from `RateCaps` in `blank_session`
  (`host.rs:1109`–`1124`) from `rates: RateCaps::v1()` (`host.rs:846`).
  `ingest` spends both intent buckets per attempt, retries included
  (`host.rs:1425`–`1431`), then checks `MAX_INTENT_FRAME_BYTES`
  (`host.rs:1432`). The failure buckets are spent in `try_consume_failure`
  (`host.rs:3955`–`3959`), called by `cache_hit` (`host.rs:1849`) and
  `commit_rejection` (`host.rs:1955`). The values are
  `POLICY_INTENT_FRAMES_PER_SEC` 40, `_FRAME_BURST` 80,
  `_BYTES_PER_SEC` 64 KiB, `_BYTE_BURST` 128 KiB
  (`crates/crpg-net/src/protocol.rs:108`–`114`).
- **Ingress caps.** `Host::ingress_caps = QueueCaps::v1()` (per peer 128
  frames / 256 KiB, host 1024 frames / 2 MiB; `host.rs:844`, values at
  `crates/crpg-net/src/protocol.rs:130`–`136`, mapped in
  `crates/crpg-net/src/sim.rs:88`–`91`). `ingest` checks per-peer then host
  frames and bytes and refuses `QueueFull` (`host.rs:1435`–`1442`); released
  on pump pop in `release_ingress` (`host.rs:1498`) and re-added by
  `restage_front` (`host.rs:1506`). Egress caps are `QueueCaps::v1_egress()`
  (`host.rs:845`, values `crates/crpg-net/src/protocol.rs:139`–`143`), checked
  in `pace_session` (`host.rs:3909`–`3934`) and `room` (`host.rs:2236`).
- **Failure budgets.** A refused or cached rejection can drop its receipt when
  `try_consume_failure` fails; `reject` returns `Step::Blocked` for v1/v2 when
  a receipt does not fit and no resync is pending (`host.rs:1797`–`1798`), and
  `AdmissionResult` carries the retained status either way
  (`host.rs:1805`–`1816`).
- **Per-tick step function.** `Host::tick` (`host.rs:3206`–`3218`) reserves
  one host record when real time (`reserve_host_record`), calls
  `HistoryWorld::tick`, then `complete_host_op(HostOp::Tick, before)`
  (`host.rs:3349`): step 1 `after_commit` (auto-pause then timer), then either
  `publish_v3_with` for a zero-event op (`host.rs:3357`) or
  `read_host_record` + `finish_host_op` (`host.rs:3360`–`3361`). The real-time
  movement stage is inside sim's `tick` (`tick_with_movement`, sim §"Movement
  stage 4"); the host has no movement step today.
- **Projection and delivery.** Accepted command: v1/v2 sessions get a receipt
  and, for v2, `project_events` over `PeerView::candidate` (`host.rs:2044`–`2060`);
  health and turn ops follow (`host.rs:2061`–`2068`); frames are encoded with
  `encode_frame` and pushed by `push_frames` (`host.rs:2077`, `:2146`–`2149`).
  v3 sessions get a page from `decide_v3_pages`/`commit_v3`
  (`host.rs:2102`–`2158`); the page builder is `plan_v3_page`, with
  `project_events_v3` (`host.rs:2599`) and `frame_spans` handling spans.
  Snapshot staging is `stage_snapshot` (`host.rs:2280`, using
  `codec_transfer::plan_transfer` at `:2320`) and `plan_v3_stage`
  (`host.rs:2395`, `project_snapshot_v3` at `:2431`,
  `transfer_v3::plan_transfer` at `:2439`). Delivery leaves through
  `take_delivery` (`host.rs:2859`) / `take_delivery_entries` (`host.rs:2873`).
- **History events to clients.** v2: `project_events(&candidates)` where
  candidates come from `PeerView::candidate` (`host.rs:2053`–`2059`, the
  grant-filtered match ending at `:4462`). v3: `candidate_v3`
  (`host.rs:4470`) maps `RoundStarted`, `ModeChanged`, `OrderEnded`, `Joined`,
  `Left` and the six v2 events, and returns `None` for `Moved`
  (`host.rs:4498`). Host operations publish through `publish_v3_with`
  (`host.rs:2833`), gated by `heartbeat_for` (`host.rs:2847`).
- **Area load: none found.** No `Area` load, bake, `bind_nav` or bounds path
  exists in `crates/crpg-server/src/`. The only `area` uses are
  `world.area().unwrap_or(HOST_UNBOUND_AREA)` in v3 keys
  (`host_v3.rs:255`, `:365`; `host.rs:4128`) and the constant
  `HOST_UNBOUND_AREA` (`host.rs:206`). Nothing reads `Area.bounds`, holds a
  `NavRevision`, or calls `crpg-nav`.
- **T141 pause/mode/heartbeat entry points.** `host_action` (`host.rs:3231`),
  `request_mode` (`host.rs:3265`), `awaiting_orders` (`host.rs:3286`),
  `settings` (`host.rs:3443`), `set_settings` (`host.rs:3454`), `paused`
  (`host.rs:3474`), `pause` (`host.rs:3480`), `unpause` (`host.rs:3495`),
  `advance` (`host.rs:3515`), `turn_timer_remaining_ms` (`host.rs:3543`);
  heartbeat publication in `publish_v3_with`/`heartbeat_for`
  (`host.rs:2833`, `:2847`) and the `POLICY_CLOCK_HEARTBEAT_TICKS` read at
  `host.rs:2611`.
- **T141 v3 session entry points.** `bind_v3_peer` (`host.rs:1153`),
  `session_wire` (`host.rs:1195`), `set_fact_grants` (`host.rs:1209`),
  `set_pause_rights` (`host.rs:1227`), `pause_rights` (`host.rs:1244`).

## Protocol v4 surface to call

All in `crates/crpg-net/src/`.

- **`codec_v4` module** (`lib.rs:47`): version 4 lane-0 codec. Public
  functions: `encode_intent(&IntentFrame) -> Result<Vec<u8>, CodecError>`
  (`codec_v4.rs:181`), `decode_intent(&[u8]) -> Result<IntentFrame,
  CodecError>` (`codec_v4.rs:223`), `frame_spans(ops: &[DeltaOp]) ->
  Result<Vec<Range<usize>>, CodecError>` (`codec_v4.rs:375`),
  `encode_delta(&DeltaFrame) -> Result<Vec<u8>, CodecError>`
  (`codec_v4.rs:402`), `decode_delta(&[u8]) -> Result<DeltaFrame,
  CodecError>` (`codec_v4.rs:428`).
- **`protocol_v4`** (`protocol_v4.rs`): `PROTOCOL_VERSION = 4` (`:27`),
  `INTENT_TAG_DECLARE_MOVE = 6` (`:30`), `DELTA_TAG_MOVED = 21` (`:32`),
  `DELTA_TAG_POSITION = 22` (`:34`), `COORD_LIMIT_RAW: i32 = 1 << 29` (`:38`),
  `MAX_MOVED_POINTS = 64` (`:41`). `Point([i32; 3])` (`:45`) with
  `Point::new(raw: [i32; 3]) -> Option<Self>` (`:50`) and `raw() -> [i32; 3]`
  (`:60`); `PositionFact { point, moving }` (`:67`). `IntentBody` (`:95`) adds
  `DeclareMove { destination: Point }` (tag 6) at `:118`. `DeltaOp` (`:155`) is
  `V3(v3::DeltaOp)` (tags 0..=20), `Moved { entity: NetId, points: Vec<Point> }`
  (`:160`) and `Position { entity: NetId, position: Option<PositionFact> }`
  (`:168`); `check_points(&[Point])` at `:187`.
- **Lane-1 module `movement_v4`** (`movement_v4.rs`):
  `MOVEMENT_VERSION = 0xA1` (`:32`), `LANE_MOVEMENT = 1` (`:34`).
  `MoveBody::MoveTo { destination: Point }` (`:78`) and `Halt` (`:83`);
  `MoveIntent { epoch: [u8; 16], seq: u64, actor: NetId, body: MoveBody }`
  (`:89`); `MoveOutcome::{Accepted { start_tick: u64 }, Superseded,
  Rejected(RejectionCode)}` (`:102`, arms at `:104`, `:109`, `:111`);
  `MoveReceipt { epoch, seq, actor, processed_tick, outcome }` (`:116`);
  `SampleEntry { entity: NetId, point: Point, moving: bool }` (`:131`);
  `MoveSample { server_tick, processed_seq, entries }` (`:143`);
  `MoveDown::{Receipt, Sample}` (`:154`); `encode_intent` (`:207`),
  `decode_intent` (`:228`), `encode_down` (`:273`), `decode_down` (`:318`),
  `check_sample` (`:432`), and
  `sample_datagrams(server_tick: u64, processed_seq: u64, entries:
  &[SampleEntry]) -> Result<Vec<Vec<u8>>, CodecError>` (`:447`).
- **Lane-1 wire bounds** (`movement_v4.rs`): `MAX_MOVE_DOWN_BYTES = 1_024`
  (`:50`), `MAX_SAMPLE_HEADER_BYTES = 24` (`:53`), `MAX_SAMPLE_ENTRY_BYTES =
  26` (`:55`), `MAX_SAMPLE_ENTRIES = 38` (`:57`).
- **Lane-1 host policies** (`movement_v4.rs`): `POLICY_MOVE_INTENTS_PER_SEC =
  20` (`:67`), `POLICY_MOVE_INTENT_BURST = 40` (`:69`), `POLICY_MOVE_IN_FLIGHT
  = 8` (`:71`), with the const assertion against
  `protocol::POLICY_RETRY_CACHE_PER_PEER_LANE` at `:72`. The doc comments say
  "Policy, never wire" (`:65`, `:68`, `:70`); the const assertions at
  `:58`–`64` pin the sample byte bounds.
- **`positions_v4`** (`positions_v4.rs`): `MAX_POSITION_PAGE_OPS`
  (`:27`), `MAX_PAGE_OPS_V4` (`:32`), `PositionRow { key: EntityKey, granted:
  bool, point: Point, moving: bool }` (`:36`), `PositionView` (`:54`, `new` at
  `:61`), `PageV4 { ops, enters, cursor }` (`:76`), and
  `project_page_v4(view: &mut InterestView, facts: &mut FactView, positions:
  &mut PositionView, state: &ViewerContext<'_>, context: &FactContext<'_>,
  rows: &[PositionRow]) -> Result<PageV4, FactError>` (`:116`).
  `project_samples(view: &InterestView, positions: &mut PositionView, rows:
  &[PositionRow], refresh: bool) -> Result<Vec<SampleEntry>, FactError>`
  (`:199`); `project_snapshot_v4(view, facts, positions, state, context) ->
  Result<SnapshotProjectionV3, FactError>` (`:252`).
- **`projection_v4`** (`projection_v4.rs`): `EventCandidate::{V3(v3 candidate),
  Moved { entity: Option<NetId>, points: Vec<Point>, disclose: bool }}`
  (`:18`, variant at `:23`) and
  `project_events(candidates: &[EventCandidate]) -> Result<Vec<DeltaOp>,
  ProjectionError>` (`:41`); a `Moved` with no entity, `disclose == false` or
  points failing `check_points` is omitted, not refused (`:38`–`40`).
- **`SEQ_LAST`** (`protocol.rs:48`): `u64::MAX - 1`, the last per-epoch,
  per-lane sequence; `SEQ_FIRST` is 1 (`protocol.rs:41`).
- **`RejectionCode`** (`protocol_v3.rs:525`–`586`, re-exported by
  `protocol_v4.rs:24`): the closed codes 1..=30 used by lane-1
  `MoveOutcome::Rejected`; lane-1-relevant members include `RateLimited` 8,
  `SessionExpired` 10, `SeqConflict` 11, `SeqGap` 12, `StaleSeq` 13,
  `SeqExhausted` 14, `NotAuthorized` 17, `IllegalAction` 18, `WrongMode` 22.

## Movement in sim to call

## Navigation to call

## Dependency edges

## Tests and goldens that could move

## Agent log
