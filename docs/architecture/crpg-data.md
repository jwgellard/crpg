# crpg-data architecture

## Scope

T010 implements the headless campaign-format crate. The implementation contract is
[T010](../../tasks/T010.md); verification status belongs to the task record.
The only internal dependency is `crpg-core`.

## Module flow

- `types` holds typed values, fixed-point transforms, variable declarations,
  validated package coordinates, logical source paths, and digests.
- `ir` holds authored graphs and declarative action signatures.
- `document` holds entities, explicit aggregates, and the versioned document union.
- `canonical` supplies duplicate-rejecting integer JSON parsing and canonical bytes.
- `package` resolves a supplied flat semver catalog and owns byte-oriented lock APIs.
- `loader` accepts a logical path-to-bytes map and derives the complete object index.
- `schema` generates 17 self-contained draft-2020-12 schemas from Rust shapes.
- `error` exposes fail-fast structural errors, without collected diagnostics.
- `migrations` owns the single production registry, pure per-type chains and the
  public `SchemaVersion` view consumed by the byte reader and the coverage gate.

Document reading passes through canonical parsing, envelope checks, current-or-
migrated schema selection, typed decoding, then lock-local checks. Loading adds
layout, engine compatibility, object indexing, package coverage and assets-lock
digest checks. Serialization shares those checks except engine compatibility and
derives identity independently of the mutable index. Callers own filesystem I/O.

## Authorities and consumers

Object ULIDs, immutable package ids, and logical source paths are separate domains.
The index includes roots and identified nested entries; aggregates have explicit
owners rather than invented ids. Core primitive serde remains authoritative and
data-local schema adapters describe its wire forms.

[E016](../../tasks/E016-campaign-envelope.md) assigns source hashes/import settings
to assets.lock, package versions/checksums plus one assets-lock digest to
campaign.lock, and future packaged-byte authority to a separate manifest.
[ADR-0008](../adr/0008-event-ownership.md) and
[ADR-0011](../adr/0011-event-payload-fields.md) assign authored IR here. Waits are
relative integer ticks; no execution or live-event dispatch belongs here.

Future rules conversion, editor, script and CLI consumers inherit portable source
bytes and typed references, never persisted interned handles. T012a migrations
are implemented as described below; T013 wrappers and T014 conversion remain
separate work.

## Semantic validation (T011a)

`validation` adds the collected pass that T010 defers: `validate` rebuilds
occurrence and ownership tables from `documents` in lexical-path, authored
order — never trusting the caller-mutable index — checks every typed ULID
reference at its exact pointer (dangling, wrong kind, foreign owner),
aggregate owners against sibling/root ids, slug uniqueness within kind,
variable default types, graph edge ports plus duplicate ports and
start reachability, dialogue reachability, quest completion, locale coverage
per referring field and table, and assets-lock membership for ambience. Object
references inside tagged values are walked recursively, including asset import
settings, and positioned at their `/value` member with RFC 6901 escaping.
`validate_files` calls `load_campaign` once and unifies the two outcomes:
one structural diagnostic from `diagnostic_for_data_error`, or the semantic
vector. `campaign_document_path` is the data-owned classifier between a
caller's directory walk and the loader layout predicates, which share one
implementation. Diagnostics serialize as six-field snake_case objects with
explicit nulls; `Display` is single-pathed and `Warning` stays non-fatal.
The `broken_references` fixture with its 15-diagnostic canonical snapshot and
the data-owned `expected.json` gate manifest pin the contract; T011b consumes
both generically without a same-task CLI edit.

## Implementation notes

`Trigger`, `NodeBody`, `Port`, and `DialogueBody` keep their derived
`Serialize`/`JsonSchema` shapes but use hand-written `Deserialize` impls that
enforce exact key sets per `kind`: serde's derived `deny_unknown_fields`
silently ignores trailing fields on internally tagged unit variants such as
`{"kind":"end","extra":0}`, which the round-trip suite pins as rejected. The
17 checked-in schemas and the ten-file `one_area_one_creature` fixture are
byte-stable through the production writer; the fixture's assets-lock digest is
`957bc137f1abb3cde6cee277d10c099b1dcd2f8814a5fd0e29b1df3506ee44fb`.

## Schema authoring and gate 7

The explicit `generate_schemas` example writes the baseline. Ordinary compilation
and tests never write it. The read-only `schema_drift` integration test compares
the entire 17-file set and exact bytes in the existing Windows/Linux workspace
test jobs. Scoped attributes pin schema and fixture bytes to LF.

## Migrations (T012a)

`migrations` holds one sorted fifteen-family registry backing both the
dispatcher and `schema_versions`; only `crpg.item` is at version 2 behind a
tag-only `1 -> 2` edge, while the other fourteen families and both locks stay
at version 1 with no edges. `read_document` keeps syntax and envelope phases,
then migrates a complete historical chain in memory before strict typed decode
and lock-local checks, preserving syntax-over-unsupported-over-typed precedence
and per-file lexical order; `load_campaign` runs those migrated reads inside
its lexical-read phase before layout, engine, index, coverage and digest
phases, never erasing engine requirements. Locks are covered without edges and
never repaired: resolutions, checksums and the assets-lock digest stay with
their T010 authorities. The eleven-file `migration_v1/campaign` fixture pins
ten byte-identical `one_area_one_creature` files plus one `crpg.item/1` item,
its `migration_v1/expected.json` golden maps every path to current canonical
text with only the item tag advanced, `migrations.json` carries the single
edge, and the three-root gate manifest lets T011b validate the old campaign to
`[]` read-only. Immediate-edge oracles for superseded edges would live under
`migration_steps/<type>/<from>-to-<to>.json`; latest edges use their campaign
goldens. The unignored `migrations` and `migration_coverage` suites prove the
golden, registry, schema and serde agreement plus every prescribed failure on
in-memory copies without touching tracked files.

## Introspection (T013a)

`introspection` exposes the data-owned `explain_object` query for the future
CLI-only `crpgc explain`: one object plus its inbound and outbound typed
references as canonical JSON bytes with one final LF. The report carries `id`,
`kind`, `file`, `pointer`, the current serialized `object` subtree, and
lexically ordered `inbound`/`outbound` occurrence arrays; each occurrence
carries `file`, `pointer`, nullable `source`, `target`, and nullable
`target_location`. Kind spellings are the report words (`dialogue_node`,
`quest_state`), distinct from validation's message words. Top-level objects
retain their schema envelope; nested objects retain their actual nested shape.

The structural-versus-semantic boundary matches the writer: layout, duplicate
identities, local lock validity, package coverage, and assets-lock digest are
checked via the loader's shared structural check with writer precedence, and
structural failure wins over unknown id. Semantic findings never block
introspection. Locations are rebuilt from documents via the private shared
`inventory`, never trusting the caller-mutable index. The inventory is the
single authority for identity locations, typed traversal, and pointer
construction (campaign entry, world areas, neighbours, faction/inventory,
relations, prefab, dialogue/quest/graph sites, edge endpoints, recursive
`ObjectRef` in defaults/args/overrides/imports including branch cases and
locals, and aggregate owners); validation consumes it for generic checks and
specialized ownership while retaining its messages, precedence, reachability,
and sort policy. Sources name the nearest enclosing object (null outside any
identified object); outbound uses component-boundary subtree matching so
`/nodes/1` never absorbs `/nodes/10`. Ordering is `(file, pointer, target)`
lexically with ULID order. The CLI owns process/I/O treatment; data owns
object and reference semantics.

## Agent log

- 2026-09-10 (UTC) · opencode/gpt-6-astra + T010 crate opening · Established approved module and authority boundaries before source implementation. This opening record does not claim completed implementation or passing gates.
- 2026-09-10 (UTC) · opencode/muse-spark + T010 implementation · Reconciled the opening record with the verified implementation, documenting the hand-written kind-tag deserializers and the checked-in schema/fixture bytes.
- 2026-09-13 (UTC) · opencode/muse-spark + T011a implementation · Documented the collected semantic-validation flow, the diagnostic and classifier authorities shared with the loader, and the broken snapshot plus gate-manifest consumers owned for T011b.
- 2026-09-17 (UTC) · opencode/muse-spark + T012a implementation · Documented the single-registry migration flow, the item-only current tags, the historical golden and coverage gate, superseding the migrations-deferred line without copying ADR rationale.

## Migration gate hardening after review

The test-only `tests/support/migration_gate.rs` now supplies shared validation
for immutable fixture snapshots, strict manifest rows, registry/schema/serde
agreement, contiguous edge coverage, full campaign output maps and immediate
oracle inventory. Both positive gates and negative mutation tests call these
validators. This replaces the initial negative checks that only compared
changed values or exercised JSON parsing independently of the gate.

Private registry tests compare actual registered edges with manifest rows and
execute each edge directly against its checked-in source/destination oracle.
Historical destinations use the conventional step file; current destinations
use the named document in their campaign golden. Inventory comparison rejects
extra files as well as missing ones. A two-edge private test demonstrates that
whole-chain output cannot substitute for an earlier immediate destination.
Only tests include this support module; production loading remains pure and
the public API, Item-only version bump and lock authorities are unchanged.

- 2026-09-17 (UTC) · opencode/gpt-6-astra + T012a review fixes · Documented the shared hard gate and direct registered-edge oracle checks, superseding the initial claim that all prescribed negative checks were already proven. Wider manifest numbers now fail strict u32 decoding instead of truncating into valid edges.
- 2026-09-18 (UTC) · opencode/muse-spark + T013a implementation · Documented the data-owned introspection report, the writer-shared structural boundary, the single inventory authority with ownership/subtree/ordering rules, and the data/CLI consumer split for the future explain command.
