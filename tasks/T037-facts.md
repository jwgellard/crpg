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

## Protocol v4 surface to call

## Movement in sim to call

## Navigation to call

## Dependency edges

## Tests and goldens that could move

## Agent log
