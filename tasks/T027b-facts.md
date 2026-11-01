# T027b/T046 fact sheet — interest projection and replica inputs

Facts only: no recommendations, no proposed types, no opinions.
Every `path:line` below was verified to exist on this branch.

## Requirements

- `D14 whole-area baseline` (`tasks/POST-T018-DECISIONS.md:324`): "Select whole-area membership as the initial entity-interest policy, **not LOS** or within-area AOI."
- `D14 sim ownership` (`tasks/POST-T018-DECISIONS.md:327`): "Sim owns persisted authoritative area membership and its typed transitions/queries; the server owns viewer-to-area and actor-control bindings and per-field grants."
- `D14 presence rule` (`tasks/POST-T018-DECISIONS.md:331`): "Presence is visible to an authorized viewer in the same area; health/turn/action-result fields require separate explicit grants."
- `D14 deny default` (`tasks/POST-T018-DECISIONS.md:332`): "Default unknown fields to absent."
- `D14 revocation` (`tasks/POST-T018-DECISIONS.md:334`): "Revoke actionable mapping before next admission; hidden removal sends cause-neutral leave."
- `D14 no-derivation` (`tasks/POST-T018-DECISIONS.md:337`): "Implement missing facts in T027a; do not derive area from NetId or health."
- `T027b sole crate` (`tasks/T027b.md:5`): "Sole crate: `crpg-net`, Phase 4. **D14 selected; T027a's approved facts or sufficiency proof, T021/T024 and exact projection API remain.**"
- `T027b interface gate` (`tasks/T027b.md:11`): "Append approved public `InterestSet`/projection-context input, field-mask types, mapping lifecycle, per-peer view/cursor and delta/snapshot projection functions with exact errors/counts/order."
- `T027b no oracle` (`tasks/T027b.md:13`): "Context is supplied by the authoritative host; no net ownership/LOS oracle is fabricated."
- `T027b no raw World` (`tasks/T027b.md:14`): "Raw complete World serialization is never a projection interface."
- `T027b enter/leave` (`tasks/T027b.md:20`): "Generate enter plus permitted initial state before any reference, leave when interest/permission revokes, retire ids for the epoch and revoke actionable mapping before a subsequent request."
- `T027b hidden spawn` (`tasks/T027b.md:23`): "Hidden spawns consume no visible id or sequence; disclosable despawns use approved notice, otherwise cause-neutral leave."
- `T027b death/despawn` (`tasks/T027b.md:25`): "Death remains distinct from despawn."
- `T027b omission` (`tasks/T027b.md:25`): "Omit undisclosed fields, never zero-fill them or leak them through cached receipts/counts/sequences."
- `T027b bounds` (`tasks/T027b.md:28`): "Store only bounded view/mapping/retired-id metadata; define exhaustion→resync/new epoch in D14 without id reuse."
- `T027b B5/policy` (`tasks/T027b.md:29`): "Apply B5 hard maxima and tightening-only policy."
- `T027b no AOI` (`tasks/T027b.md:30`): "Phase 4 whole-area interest does not authorize within-area AOI indexing."
- `T027b constraints` (`tasks/T027b.md:36`): "No gameplay legality, host credentials, new sim facts, net→rules/script dependencies or implicit testkit integration."
- `T027b fixtures` (`tasks/T027b.md:37`): "T018 tables remain test fixtures, not runtime perception."
- `T027b test gate` (`tasks/T027b.md:41`): "Public `interest` suite: two viewers/areas, field-by-field entitlements, enter/update/leave/reenter without id reuse, hidden spawn/death/despawn, same-tick revoke-and-request mapping, snapshot/delta cutover and bounded state."
- `T027a gate` (`tasks/T027a.md:5`): "**D14 selected**" — T027b's missing-facts prerequisite, "not a license for net to invent perception from its fixture table" (`tasks/T027a.md:8`).
- `T027a no NetId in sim` (`tasks/T027a.md:15`): "Client-scoped NetId/peer/capability types do not enter sim."
- `T027a identity` (`tasks/T027a.md:141`): "Runtime identity across multiple Worlds is `(area ULID, full EntityId)`; two areas can mint equal arena ids."
- `T027a no v1 change` (`tasks/T027a.md:143`): "This does not change EntityId's core representation or any v1 wire shape."
- `T027a host handoff` (`tasks/T027a.md:182`): "the returned pair of area identities and entity ids identifies that handoff without net ids in sim."
- `T027a consumer` (`tasks/T027a.md:196`): "Host T027c must validate authored area identity against its loaded campaign and assign viewer-area/control bindings and field grants."
- `T027c blocked` (`tasks/T027c.md:5`): "Sole crate: `crpg-server` … **Blocked** on T022, T027b and exact API".
- `T027c revocation order` (`tasks/T027c.md:12`): "Pin pre-removal capture, post-operation projection phase and atomic mapping revocation before admission".
- `T027c entitlement` (`tasks/T027c.md:17`): "Compute entitlement from authoritative facts and server grants, not requested client interest."
- `T027c denial` (`tasks/T027c.md:19`): "Missing/hidden/foreign ids yield the same public denial."
- `T027c no cache restore` (`tasks/T027c.md:23`): "save/restart must not restore stale authorization from a serialized view cache."
- `Standing rules E004` (`tasks/POST-T018-RULES.md:32`): "one implementation crate per child."
- `Standing rules oracle` (`tasks/POST-T018-RULES.md:54`): "Replica tests use independent permitted-view oracles with drop/corrupt negative controls, not full World hash equality or production projection as oracle."
- `Standing rules v1` (`tasks/POST-T018-RULES.md:59`): "Preserve v1 fixtures/semantics; additive closed variants require reviewed versions and fixtures."
- `E012 Resolution replica split` (`tasks/E012-binary-crate-naming.md:72`): "`ReplicaWorld`: `crpg-net` owns filtered replica storage, delta application and engine-neutral read queries; `crpg-godot` owns the engine-facing query/presentation objects."
- `E012 option (a)` (`tasks/E012-binary-crate-naming.md:65`): "Client and editor are Godot projects (`apps/client`, `apps/editor`) over the `crpg-godot` GDExtension crate … they are never Rust packages."
- `E012 open item` (`tasks/E012-binary-crate-naming.md:81`): "**E022** owns the exact host/editor/FFI interfaces … Nothing here is implemented yet."
- `Spec Tier 1` (`docs/CRPG_ENGINE_SPEC.md:815`): "interest set = the area the player's character is in."
- `Spec Tier 2` (`docs/CRPG_ENGINE_SPEC.md:816`): "grid-cell AOI within an area" is "designed for, not built".
- `Spec filtering mandate` (`docs/CRPG_ENGINE_SPEC.md:817`): "Per-client component filtering is mandatory, not optional." (D14 realization note appended on the same line.)
- `Spec interest size` (`docs/CRPG_ENGINE_SPEC.md:740`): "Entity counts per client's interest set are in the hundreds."
- `Spec ReplicaWorld` (`docs/CRPG_ENGINE_SPEC.md:175`): "the client's copy of the world is behind a `ReplicaWorld` type with no mutating methods except `apply_delta`."
- `Spec replica owners` (`docs/CRPG_ENGINE_SPEC.md:874`): "Replica storage, delta application and engine-neutral read queries belong to `crpg-net`; `crpg-godot` owns the engine-facing query objects."
- `Spec client queries` (`docs/CRPG_ENGINE_SPEC.md:881`): "**UI reads the replica world through a stable query API**, not through Godot nodes."
- `Spec 500×500` (`docs/CRPG_ENGINE_SPEC.md:815`): Tier 1 is "sufficient for 8 players in a 500×500 m area".
- `Spec net tests` (`docs/CRPG_ENGINE_SPEC.md:1480`): "requesting entities outside the interest set" must "produce a clean rejection and a log line, never a panic".
- `Spec desync` (`docs/CRPG_ENGINE_SPEC.md:1480`): "run server and client replicas over the simulated network for 5,000 ticks and assert the replica's visible state matches the server's filtered projection."
- `Spec Phase 4` (`docs/CRPG_ENGINE_SPEC.md:1578`): "Deliverables: `crpg-net` (protocol, codec, quinn, sessions, interest sets, per-client filtering)".
- `Spec no AOI` (`docs/CRPG_ENGINE_SPEC.md:1585`): "**Do not build:** prediction polish, AOI within areas, a master server."
- `PLAN T027b` (`docs/IMPLEMENTATION_PLAN.md:199`): "D14 whole-area baseline plus private-field grants; leak test. H-filter".
- `PLAN T027c` (`docs/IMPLEMENTATION_PLAN.md:200`): "Authenticated viewer context".
- `PLAN T046` (`docs/IMPLEMENTATION_PLAN.md:248`): "Replica storage, delta application, engine-neutral read queries".
- `PLAN demo-first` (`docs/IMPLEMENTATION_PLAN.md:410`): "T027b, T046, T045, T047, T048, cut-down T051" ahead of movement and nav.
- `ADR-0019 scope` (`docs/adr/0019-event-protocol-v2.md:32`): v2 is a "projection input, not a replacement for T027's production interest policy".
- `ADR-0021 handoff` (`docs/adr/0021-authoritative-host-capture.md:53`): "Production interest facts (T027a area, T027c viewer context) will later" feed capture shapes.
- `ADR-0022 grants` (`docs/adr/0022-host-session-capture-retirement.md:71`): "production interest facts (T027a/c supply values for these shapes, not type" changes.
- `ADR-0031 narrowing` (`docs/adr/0031-host-snapshot-sender-and-snapshot-sessions.md:11`): "Today narrowing grants reset a peer's delivery" via `GrantUpdate::RebindRequired`.
- `ADR-0031 opt-in` (`docs/adr/0031-host-snapshot-sender-and-snapshot-sessions.md:26`): "Snapshot sessions are opt-in, chosen at bind."

## Sim facts available

- `World::new_in_area` (`crates/crpg-sim/src/world.rs:878`): binds an immutable authored area ULID at construction; no setter.
- `World::area` (`crates/crpg-sim/src/world.rs:886`): returns the world's bound area, if any.
- `World::area_of` (`crates/crpg-sim/src/world.rs:896`): returns the world's area iff the full generation-bearing id is currently live; dead-but-retained entities keep presence.
- `World::contains` (`crates/crpg-sim/src/world.rs:1007`): liveness check used by transfer preflight.
- `World::ids` (`crates/crpg-sim/src/world.rs:1022`): ascending-id iteration; T027a's stated iteration path with `area_of`.
- `World::transforms` (`crates/crpg-sim/src/world.rs:1046`): read-only transform store a host reads for spatial facts.
- `HistoryWorld::new_in_area` (`crates/crpg-sim/src/history.rs:437`): directly creates the bound inner world; no mutable escape.
- `AreaError` (`crates/crpg-sim/src/area.rs:30`): ten transfer-failure variants checked in declaration order.
- `transfer_entity` (`crates/crpg-sim/src/area.rs:158`): moves one noncombat, unscheduled entity between two bound worlds with a fresh destination id.
- `transfer_history_entity` (`crates/crpg-sim/src/area.rs:193`): same move through the wrappers' typed despawn/spawn paths on staged clones.
- `validate_action` (`crates/crpg-sim/src/combat.rs:2949`): the only pre-mutation admission path; `perform_action` calls it first; changes nothing.
- `legal_actions` (`crates/crpg-sim/src/combat.rs:3177`): bounded enumeration in ascending ability-ULID × full-`EntityId` order plus `EndTurn`; host filters disclosure.
- `CombatError::AreaMismatch` (`tasks/T027a.md:119`): returned on configured/resolved area disagreement after placement checks.
- `No NetId in sim`: none found — no `NetId`, `InterestSet`, `ReplicaWorld`, `ComponentMask` or `visible_to` exists under `crates/crpg-sim/src`.
- `No per-viewer store`: none found — sim holds no viewer, peer, grant or entitlement state; iteration uses existing `ids` plus `area_of` (`tasks/T027a.md:139`).

## Wire today

- `PROTOCOL_VERSION v1` (`crates/crpg-net/src/protocol.rs:25`): lane-0 combat wire version byte 1.
- `LANE_COMBAT` (`crates/crpg-net/src/protocol.rs:34`): lane byte 0 for v1 intents and deltas.
- `SEQ_FIRST/SEQ_LAST` (`crates/crpg-net/src/protocol.rs:41`): per-lane command sequence window `1..=u64::MAX-1`.
- `MAX_INTENT_FRAME_BYTES` (`crates/crpg-net/src/protocol.rs:58`): 4096-byte v1 intent frame cap (B5).
- `MAX_DELTA_FRAME_BYTES` (`crates/crpg-net/src/protocol.rs:64`): 65536-byte v1 delta frame cap (B5).
- `MAX_DELTA_OPS` (`crates/crpg-net/src/protocol.rs:70`): 256 ops per delta frame (B5).
- `MAX_WIRE_COLLECTION` (`crates/crpg-net/src/protocol.rs:73`): 256-entry wire collection cap (B5).
- `MAX_WIRE_STRING_BYTES` (`crates/crpg-net/src/protocol.rs:80`): 256-byte wire string cap (B5).
- `MAX_WIRE_BYTES_FIELD` (`crates/crpg-net/src/protocol.rs:83`): 4096-byte wire bytes-field cap (B5).
- `MAX_SNAPSHOT_BYTES` (`crates/crpg-net/src/protocol.rs:89`): 1 MiB snapshot cap (B5).
- `MAX_VISIBLE_ENTITIES` (`crates/crpg-net/src/protocol.rs:92`): 1024 visible entities (B5).
- `MAX_SNAPSHOT_CHUNKS` (`crates/crpg-net/src/protocol.rs:95`): 32 snapshot chunks (B5).
- `T018a B5 list` (`tasks/T018a.md:39`): the nine hard maxima above, frozen as wire constants.
- `Intent tags` (`crates/crpg-net/src/protocol.rs:172`): `DeclareAction` tag 0, `EndTurn` tag 1.
- `Delta tags` (`crates/crpg-net/src/protocol.rs:177`): eight filtered-replica tags 0–7 (`EntityEnter` … `Receipt`).
- `NetId` (`crates/crpg-net/src/protocol.rs:217`): client-scoped replica id; `new` returns `None` on 0 (`crates/crpg-net/src/protocol.rs:221`).
- `IntentFrame` (`crates/crpg-net/src/protocol.rs:245`): epoch, lane, seq, observed tick plus `IntentBody`.
- `DeltaFrame v1` (`crates/crpg-net/src/protocol.rs:292`): lane, `server_tick`, per-client gapless `event_seq`, filtered ops.
- `DeltaOp v1` (`crates/crpg-net/src/protocol.rs:309`): `EntityEnter/EntityLeave/Spawned/Despawned/Died/Health/Turn/Receipt`; every entity field is a `NetId`, never a raw arena id.
- `RejectionCode` (`crates/crpg-net/src/protocol.rs:393`): frozen `1..=20` codes with `as_u8`/`from_u8` (`crates/crpg-net/src/protocol.rs:439`).
- `CodecError` (`crates/crpg-net/src/codec.rs:52`): six variants with first-failure-wins staged precedence.
- `encode/decode_intent` (`crates/crpg-net/src/codec.rs:154`): canonical v1 intent codec entries.
- `encode/decode_delta` (`crates/crpg-net/src/codec.rs:391`): canonical v1 delta codec entries.
- `protocol_v2 version` (`crates/crpg-net/src/protocol_v2.rs:34`): version byte 2 beside frozen v1.
- `v2 event tags` (`crates/crpg-net/src/protocol_v2.rs:37`): tags 8/9/10 (`ActionResolved`/`TurnStarted`/`EncounterEnded`).
- `v2 DeltaOp` (`crates/crpg-net/src/protocol_v2.rs:88`): `Legacy` wrapper (no extra tag) plus the three event variants with actor/target/ability/outcome/damage fields.
- `codec_v2` (`crates/crpg-net/src/codec_v2.rs:242`): version-selected v2 intent/delta codecs; delta `event_seq` accepts full `u64`.
- `EventCandidate` (`crates/crpg-net/src/projection_v2.rs:53`): all-`Option` per-field host-authorized disclosure candidates consumed by projection.
- `project_events` (`crates/crpg-net/src/projection_v2.rs:141`): at most 256 inputs, ordered subsequence, no partial output on error.
- `TRANSFER_VERSION` (`crates/crpg-net/src/protocol_transfer.rs:19`): snapshot-transfer first byte `0x81`.
- `LANE_SNAPSHOT` (`crates/crpg-net/src/protocol_transfer.rs:22`): lane 2 for transfer frames.
- `Transfer tags` (`crates/crpg-net/src/protocol_transfer.rs:25`): `Begin`/`Chunk`/`Cancel` down tags and `Published`/`Resync` up tags.
- `TransferId` (`crates/crpg-net/src/protocol_transfer.rs:67`): nonzero transfer identity; `new` returns `None` on 0 (`crates/crpg-net/src/protocol_transfer.rs:71`).
- `BeginTransfer` (`crates/crpg-net/src/protocol_transfer.rs:90`): fixed epoch, transfer id, base tick and last included delivery cursor.
- `SnapshotBody` (`crates/crpg-net/src/protocol_transfer.rs:278`): versioned snapshot payload: entities plus optional turn fact.
- `SnapshotEntity` (`crates/crpg-net/src/protocol_transfer.rs:287`): wire entity carrying a `NetId` plus optional health fact.
- `codec_transfer` (`crates/crpg-net/src/codec_transfer.rs:160`): `encode_down`/`decode_down` and matching up/body/plan functions (`crates/crpg-net/src/codec_transfer.rs:488`).
- `TransferAssembly` (`crates/crpg-net/src/reassembly.rs:52`): byte-layer assembly proving the 1 MiB / 32-chunk envelope.
- `ReassemblyPool` (`crates/crpg-net/src/reassembly.rs:476`): bounded pool with deadline, budgets, atomic publication and the delivery-cursor cutover gate.
- `rebind_receiver` (`crates/crpg-net/src/reassembly.rs:1077`): additive resume rebind keeping epoch, wire, published and queued updates.
- `RESUME_VERSION` (`crates/crpg-net/src/protocol_resume.rs:18`): resume first byte `0x91`, lane 3 (`crates/crpg-net/src/protocol_resume.rs:21`).
- `SessionFence` (`crates/crpg-net/src/resume.rs:74`): per-session epoch/fence window; orders connection generations, authorises nothing.
- `EntityId→wire mapping`: none found in `crates/crpg-net/src` — the only mapping lives in the test-only driver (`entity_of`/`net_of`).

## Filtering today

- `Visibility` (`crates/crpg-net/tests/support/driver.rs:35`): test-only per-pair disclosure flags (`full`/`presence_only`/`none`).
- `VisibilityTable` (`crates/crpg-net/tests/support/driver.rs:73`): allow-listed `(PeerId, EntityId)` table; unlisted pairs are `none`.
- `OwnershipTable` (`crates/crpg-net/tests/support/driver.rs:99`): test-only per-peer actor control table.
- `NetDriver` (`crates/crpg-net/tests/support/driver.rs:217`): test-only double running decode → epoch → seq/cache → freshness → mapping/visibility/ownership → `perform_action` legality.
- `ExpectedView` (`crates/crpg-net/tests/support/driver.rs:176`): independently projected permitted view (events as multisets with separately gapless numberings).
- `NetDriver::project` (`crates/crpg-net/tests/support/driver.rs:875`): independent oracle path straight from authority, not from delivery bytes.
- `NetDriver::deltas` (`crates/crpg-net/tests/support/driver.rs:723`): filtered per-client delta pump with redeliverable frames.
- `NetDriver::admit` (`crates/crpg-net/tests/support/driver.rs:397`): wire admission path spending failure budgets on rejections.
- `entity_of/net_of` (`crates/crpg-net/tests/support/driver.rs:336`): test-only replica↔authority mapping with graves; revoked ids never reused.
- `Replica` (`crates/crpg-net/tests/conformance.rs:285`): client-half oracle buffering deliveries and comparing per-fact via `matches` (`crates/crpg-net/tests/conformance.rs:354`).
- `filtered_replica_converges_per_peer` (`crates/crpg-net/tests/conformance.rs:1039`): filtered convergence proof with negative controls.
- `desync_oracle_fails_loudly` (`crates/crpg-net/tests/conformance.rs:1095`): drop/corruption negative control on the oracle.
- `hidden_lifecycle_never_leaks` (`crates/crpg-net/tests/conformance.rs:1152`): hidden spawn/death/despawn non-disclosure proof.
- `five_thousand_tick_exercise` (`crates/crpg-net/tests/conformance.rs:1366`): 5,000-tick loss/jitter convergence exercise.
- `v1_and_v2_are_explicit` (`crates/crpg-net/tests/events_v2.rs:170`): explicit version selection, both directions.
- `field_by_field_suppression` (`crates/crpg-net/tests/events_v2.rs:1055`): two-peer disclosure views over the same journal.
- `real_history_order_survives` (`crates/crpg-net/tests/events_v2.rs:1739`): ordered projection of real T020 journals.
- `redelivery_is_byte_identical` (`crates/crpg-net/tests/events_v2.rs:2005`): byte-identical redelivery from a driver cache.
- `Suite files` (`crates/crpg-net/tests/conformance.rs:1`): conformance, `events_v2`, `snapshot_transfer`, `snapshot_transfer_contract`, `reconnect_wire`, `codec`, `transport`, `token_bucket_regression`, `audit_gaps` suites live beside `tests/support/`.
- `Production filter`: none found — T018 tables are fixtures; the conformance driver states supplied tables are "not perception" (`crates/crpg-net/AGENTS.md:86`).

## Host today

- `Host` (`crates/crpg-server/src/host.rs:524`): the only mutator of its private `HistoryWorld`; no accessor returns it.
- `Host::new` (`crates/crpg-server/src/host.rs:549`): in-memory host from seed, config and injected `now_ms`.
- `bind_peer` (`crates/crpg-server/src/host.rs:691`): binds an authenticated viewer to control plus disclosure grants; first delivery is delta frames from seq 1.
- `bind_snapshot_peer` (`crates/crpg-server/src/host.rs:704`): opt-in snapshot session whose delivery also carries `0x81` transfer frames in one total order.
- `ControlGrant` (`crates/crpg-server/src/host.rs:133`): which whole entities a peer may act as, at most 1024.
- `EntityDisclosure` (`crates/crpg-server/src/host.rs:140`): per-entity, per-field disclosure flags; unknown fields denied.
- `DisclosureGrants` (`crates/crpg-server/src/host.rs:167`): everything one peer may learn; `Default` denies all.
- `set_grants` (`crates/crpg-server/src/host.rs:819`): grant update; narrowing a lane-0 session clears the log, bumps generation, restarts `event_seq` at 1.
- `ingest` (`crates/crpg-server/src/host.rs:904`): staged admission correlating `(epoch, lane, seq)` in the host, not peer identity in sim.
- `pump` (`crates/crpg-server/src/host.rs:954`): determine–reserve–commit execution; gameplay validated through sim, never a host legality copy.
- `take_delivery` (`crates/crpg-server/src/host.rs:1682`): ordered per-peer byte log drained by the adapter.
- `take_delivery_entries` (`crates/crpg-server/src/host.rs:1695`): tagged `DeliveryEntry` (`DeliveryKind::Delta/Transfer`, `crates/crpg-server/src/host.rs:185`) with delivery ordinals.
- `acknowledge_delivery` (`crates/crpg-server/src/host.rs:1721`): ordinal-based acknowledgement of handed-off entries.
- `transfer_state/ingest_transfer` (`crates/crpg-server/src/host.rs:1752`): in-flight transfer inspection and `Resync` planning surface.
- `CapturedView` (`crates/crpg-server/src/capture.rs:105`): per-peer permitted post-state (epoch, ≤3 health facts, optional turn).
- `CapturedRecord` (`crates/crpg-server/src/capture.rs:118`): accepted command evidence with exact history range plus ≤8 views.
- `MAX_PEERS` (`crates/crpg-server/src/host.rs:72`): 8 peer bindings.
- `MAX_MAPPED_PER_PEER` (`crates/crpg-server/src/host.rs:65`): 1024 replica ids per session.
- `Snapshot cutover` (`docs/architecture/crpg-net.md:403`): H1–H8 host requirements pattern for resume/snapshot policy living in `crpg-server`.
- `QUIC adapter` (`crates/crpg-server/src/quic.rs:430`): `QuicHost` binding invitations to `PeerHandle`s with fencing on narrowing `/1` sessions.
- `Projection call site`: none found — no host call into `project_events` or any per-peer view/cursor projection function exists yet.

## Client side today

- `Replica client-half`: the conformance `Replica` (`crates/crpg-net/tests/conformance.rs:285`) buffers deliveries, applies per-`event_seq` (`crates/crpg-net/tests/conformance.rs:299`) and checks contiguity (`crates/crpg-net/tests/conformance.rs:338`); test-only, not shipped code.
- `crpgc demo` (`crates/crpg-cli/src/demo.rs:1`): terminal combat viewer reading replays through testkit; a viewer with no replica store.
- `crpgc play` (`crates/crpg-cli/src/combat_apply.rs:1`): interactive legal-action playback asking `legal_actions` for options; no networked state.
- `QuicClient` (`crates/crpg-net-quic/src/client.rs:102`): real QUIC peer with `try_send`/`try_recv` and per-channel variants (`crates/crpg-net-quic/src/client.rs:329`).
- `QuicServer` (`crates/crpg-net-quic/src/server.rs:199`): real transport endpoint with `try_send_channel`/`try_recv_channel`; no replica storage.
- `Channel` (`crates/crpg-net-quic/src/handshake.rs:103`): typed channel ids carried beside frame bytes.
- `ReplicaWorld type`: none found — no `ReplicaWorld` exists in `crates/`; only the spec and E012 name it as `crpg-net`'s future filtered-replica owner.
- `Read query over received state`: none found — no engine-neutral replica read API exists outside the test oracle's `matches` check.

## Bounds and errors

- `CodecError` (`crates/crpg-net/src/codec.rs:52`): `FrameTooLarge/UnsupportedVersion/WrongDirection/UnknownMessage/Malformed/LimitExceeded`, shared by v1/v2/transfer/resume codecs.
- `ProjectionError` (`crates/crpg-net/src/projection_v2.rs:107`): `TooManyEvents/InvalidOutcome/StringLimit` for host-fed projection.
- `AreaError` (`crates/crpg-sim/src/area.rs:30`): ten transfer failures displaying `<Variant> at area/transfer` (`crates/crpg-sim/src/area.rs:73`).
- `HostError` (`crates/crpg-server/src/host.rs:323`): closed/time/peer/busy/queue/grant/cursor/counter failures for host calls.
- `CheckpointError` (`crates/crpg-server/src/checkpoint.rs:52`): checkpoint encode/decode failures with file load entry (`crates/crpg-server/src/save.rs:287`).
- `SaveError` (`crates/crpg-server/src/save.rs:107`): file-save identity and size failures.
- `QuicHostError` (`crates/crpg-server/src/quic.rs:322`): QUIC adapter failures.
- `ResumeError` (`crates/crpg-net/src/resume.rs:32`): fence/epoch/progress failures for the resume window.
- `AssemblyError` (`crates/crpg-net/src/reassembly.rs:20`): chunk accept/publish failures.
- `LimitsError` (`crates/crpg-net/src/reassembly.rs:226`): reassembly pool limit validation failures.
- `TransferError` (`crates/crpg-net/src/reassembly.rs:365`): receiver/cutover failures with `FailReason` (`crates/crpg-net/src/reassembly.rs:334`).
- `TransferLimits::v1` (`crates/crpg-net/src/reassembly.rs:139`): 1-in-flight / 32-chunk / 5-second pool bounds.
- `POLICY_*` (`crates/crpg-net/src/protocol.rs:108`): tightening-only operational defaults (rates, bursts, ingress/egress depths, retry cache, 8-peer proof population, snapshot window).
- `Host caps` (`crates/crpg-server/src/host.rs:65`): 1024 mapped/controlled/grant identities, 8 peers, capture and delivery ledger caps (`crates/crpg-server/src/host.rs:54`).
- `MAX_LEGAL_ACTIONS` (`crates/crpg-sim/src/combat.rs:3177`): 4096-option whole-query enumeration bound at the legality surface.

## Agent log

- 2026-11-01 (UTC) · opencode-go/muse-spark-1.3-contributor + T027b facts · Gathered the cited requirement, sim, wire, filtering, host, client and bounds facts for the T027b/T046 contract drafts without designing anything.
