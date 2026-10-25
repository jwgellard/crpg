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
- `pointer_edit` applies one RFC 6901 edit to a typed document at its current tag.
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

## Combat vocabulary (T016a)

`combat` holds four generic authored families added for the T016
headless-combat proof: `Ruleset` (stat declarations, health designation,
attribute list, one action-pool template, ability references, package
self-identity), `Ability` (dice string, checked attribute name,
outcome-table reference, per-outcome flat damage, action cost, targeting
flags), `OutcomeTable` (margin bands plus natural-face rules), and
`Encounter` (ruleset reference plus placement/initiative participants).
Schemas, layout families, migration registry entries (version 1, no edges),
index kinds, inventory occurrences, slug/locale participation, and
introspection spellings all extend the existing authorities; nothing is
duplicated. Document-local invariants fail at read time; cross-document
stat membership, pool affordability, and participant creature coverage fail
as collected `unknown_stat`/`invalid_cost`/`missing_stat`/
`invalid_stat_value` diagnostics. `Creature.stats` keeps its fixed-point
wire meaning: only an encounter participant's declared stats convert, and
only when whole, so existing campaigns are unaffected and the adapter can
never truncate silently. The canonical `rulesets/minimal-d6/` source and
its byte-identical `combat_basic` campaign copies keep one truth with a
test-enforced parity check; the campaign stays out of the gate-8 fixture
inventory by explicit decision.

## Second-ruleset vocabulary (T017a)

`combat` gains the effect family and the plural-pool/cost/defense/turn
vocabulary that lets a second ruleset vary behavior through content alone:
`Effect` (self/target aim, one shared `mod_type` with one effect-scoped
`PolicyWire`, ordered `EffectModifierWire` entries with `i32` values and
`i16` priorities, `duration_rounds >= 1`), `Ruleset.pools` (ordered
templates replacing the single `action_pool`), and `Ability` with retained
primary-pool `cost` plus `extra_costs`, `ends_turn`, optional `effect`,
`DefenseWire`, and optional `natural_die` index. `crpg.effect/1` is the
22nd schema; `crpg.ruleset/2` and `crpg.ability/2` carry the new shapes
with clone-then-publish `/1 → /2` edges supplying all-local defaults.
Schemas stay count-agnostic; the srd-lite content pins two pools, two
abilities, two bands-plus-natural versus two bands, one effect, and
duration 2. The canonical `rulesets/srd-lite/` source and its
byte-identical `combat_srd` campaign copies keep one truth with a
test-enforced parity check; the campaign stays out of the gate-8 fixture
inventory by explicit decision. Pool balances are values inside the
ruleset, never indexed objects, so the extra-pool walk resolves against
the owning ruleset's templates.

As-built: `MAX_EFFECT_MODIFIERS` is 4096 (mirroring T014); pools stay
unbounded beyond nonempty/unique. Empty spend fails at both layers
(`Malformed` on file load preserving the T016a zero-cost test,
`invalid_cost` on in-memory validation pinning the code). Migration oracles
for the two new edges share the existing `migration_v1/campaign` root with
three new files/keys; existing keys are byte-identical. The breaking
`pools` replacement breaks downstream `crpg-sim` compilation until B3; the
blocker with options is recorded in `tasks/T017a.md`.

### Real-time fields (T136a)

`Ruleset.round_ticks` (`u32`) and `Ability.recovery_ticks` (`u32`) and
`repeats_in_real_time` (`bool`) are optional-missing at the current
`crpg.ruleset/2` and `crpg.ability/2` tags, with no migration edge. Absence is
turn-based-only, today's only mode, so every existing document decodes and
encodes unchanged. Read time rejects a zero `round_ticks` or `recovery_ticks`
as `Malformed`, as the last check of its arm. `validate` only warns when they
appear under a ruleset with no `round_ticks` (T136b); defaulting
`recovery_ticks` to `round_ticks` is the sim's (T138). The fields are unchanged
by T136b, which adds no version bump (D32c).

- 2026-10-23 (UTC) · claude-code/sonnet-5.5 + T136a · Documented the three optional real-time fields added at the current tags, their read-time zero checks, and that semantics stay with the sim.

### Kernel-range surface (T136b)

D32c: no version bump, no migration and no fixture change. Every addition is
optional-missing or an additive variant at `crpg.ruleset/2`, `crpg.ability/2`
and `crpg.effect/1`, so each existing document decodes to today's value and
re-encodes byte-for-byte. The sim cannot execute any of it yet; T136c makes it
reject each feature with `UnsupportedAuthored` until T137 maps it.

- Stats: `StatKindWire` gains `Fixed`, `Bool`, `Enum{enum_id, variants}`,
  `Tags`, `Dice` (externally tagged, hand-written `Deserialize`);
  `StatDecl.derived` holds an `ExprWire` (adjacent `type`/`value`, six binary
  operators plus `Literal`/`Stat`) over `StatValueWire` literals.
- Resolution: `Ability.bonus_stats`, `DefenseWire::{Constant, ActorStat,
  Opposed}`, `DamageEntry.dice`, `Ruleset.initiative` (`InitiativeWire`).
- Effects: `EffectTargetWire::Stat{stat}` and `Effect.grants`.
- Read time (`Malformed`): enum domains, `health_stat` a non-derived `Int`,
  `attributes` `Int`, expression node/depth/total bounds, literals, references,
  cycles, chain length and kinds are all judged inside one ruleset, in the
  order the contract lists; `crpg-data` never depends on `crpg-rules`.
  Bounds are `MAX_EXPR_NODES` and its siblings in `combat.rs`, mirroring the kernel.
- Semantic: `unknown_stat` also covers `bonus_stats`, `ActorStat`, `Opposed`
  and effect `Stat` targets (valid means a declared, non-health `Int` stat);
  `missing_stat` applies to non-derived `Int`/`Fixed` stats and the whole-number
  check to `Int` only. `real_time_fields_without_round_ticks` is the crate's
  first `Warning`: an ability with `recovery_ticks` or `repeats_in_real_time ==
  Some(true)` under a ruleset with no `round_ticks`.

- 2026-10-23 (UTC) · claude-code/sonnet-5.5 + T136b · Documented the kernel-range surface added at the current tags, its read-time and semantic rules, and the first warning; the T136a line that `validate` has no rule for the real-time fields is superseded by the new warning.

## Moved from AGENTS.md (A0-8)

Detail moved out of `crates/crpg-data/AGENTS.md` by A0-8 (2026-10-25) to keep that file at or below 10,000 bytes. Wording preserved; merge into the sections above as appropriate.

### Public surface

## Public surface

Public modules `types`, `ir`, `document`, `canonical`, `package`, `loader`,
`schema`, `error`, `validation`, `migrations`, `introspection`,
`action_signatures` (T029a), `pointer_edit` (T058a) re-export their
public items at the root. T010 lists every required model, field and variant.
Operations are `read_document`, `write_document`, `canonical_json`,
`generated_schemas`, `resolve_packages`, `assets_lock_digest`,
`make_campaign_lock`, both lock read/write pairs, `load_campaign`,
`serialize_campaign`, plus T011a's `validate`, `validate_files`,
`diagnostic_for_data_error`, and `campaign_document_path` with the `Diagnostic`,
`DiagnosticCode`, and `Severity` model, plus T012a's `SchemaVersion` with
`schema_versions` and `migrate_document`, plus T013a's `explain_object` returning
canonical report bytes, plus T058a's `PointerEdit`, `PointerEditError`,
`MAX_EDIT_POINTER_BYTES`, `pointer_tokens`, `edit_document` and
`campaign_index`. Primitive newtypes expose validated parsing plus their
prescribed accessors. Do not extend the surface casually.

### Wire and validation traps

## Wire and validation traps

- One schema-tagged object per document; fourteen families stay at version 1
  while `crpg.item` is at version 2 behind the single `1 -> 2` tag-only edge.
  Placement and ActionSignature are embedded schema roots, not document kinds.
  Per-type steps are private `fn v1_to_v2` mutations; one sorted registry backs
  both dispatch and `schema_versions`, with no caller registration.
- Reject unknown fields. Only `_note` (and the optional-missing `effect`,
  `natural_die`, `round_ticks`, `recovery_ticks`, `repeats_in_real_time`,
  `derived`, `initiative`, `bonus_stats`, damage `dice`, `grants`) may
  be missing; other nullable fields
  need a required-nullable serde helper and matching schema required annotation.
- A field may join a family at its current version, with no tag change and no
  migration edge, when (i) it is optional-missing, (ii) absence means exactly
  the behaviour before the field existed, and (iii) every existing document
  decodes to the same value with an unchanged canonical encoding (T136a). An
  additive enum variant is the same rule: no existing document can hold it, so
  it joins at the current tag (T136b, D32c).
- Core ULID aliases and overflow rules govern input; output is uppercase.
  Fixed point is raw signed 32-bit integers. Use data-local schema surrogates
  for core/semver fields, including nested collections; never edit core.
- Tagged values and IR preserve ordered arrays. Do not persist interned handles.
- Canonical JSON sorts keys recursively regardless of feature unification,
  uses two spaces and exactly one LF, and rejects non-integer numeric forms.
  Reject duplicate keys before constructing an ordinary JSON value.
- Read order: syntax/duplicates/numbers; envelope; current tag or complete
  registered chain with per-step tag postconditions; typed fields; lock-local
  invariants. Loader whole-input order: case collisions/required files;
  lexical reads including migrations; layout; engine; duplicate-free index;
  package coverage; digest. Writer shares layout/local/index/lock checks but
  has no engine check. Migration never erases engine requirements and never
  repairs locks; `migrate_document` clones then publishes, validates current
  input without rewriting, and stays idempotent.
- Resolver checks requirement kinds, then all candidate conflicts (including
  unused entries), then resolves package-id order. Semver precedence ties use
  lexically greatest complete version text. Never silently sort lock arrays.
- Object ids, package coordinates, and logical paths are different authorities.
  Caller-mutable index entries must not influence output or bypass validation.
- No filesystem access in the library; generation example owns I/O. No builds
  or tests may rewrite schema/fixture baselines. Drift compares exact bytes.
- Internally `kind`-tagged enums (`Trigger`, `NodeBody`, `Port`, `DialogueBody`)
  use hand-written `Deserialize` impls enforcing exact key sets per variant.
  Derived `deny_unknown_fields` ignores extra fields on unit variants such as
  `{"kind":"end","extra":0}`; keep the manual impls and the unknown-field tests
  that pin them. `Serialize`/`JsonSchema` stay derived so schemas keep
  `additionalProperties: false`.
- Semantic validation is collected and positioned, never fail-fast: rebuild
  occurrence/ownership tables from `documents` in lexical-path, authored order
  and ignore the caller-mutable index entirely. Sort findings by file, pointer,
  code, severity, message, fix; emit at most one diagnostic per reference and
  never judge an edge port or reachability from an unresolved endpoint.
  Aggregate owners get one specialized check, never a generic reference too.
- `campaign_document_path` shares the loader's `family`/`area_file`/
  `locale_shape` predicates; a second path-family list is a drift defect.
  Classifier rejections stay `invalid_path` through `diagnostic_for_data_error`;
  `io` belongs to filesystem-owning callers only.
- Introspection shares one private `inventory` for identity locations, typed
  traversal, and pointer construction with validation and the loader; a second
  reference-field list is a drift defect. `explain_object` reuses the writer's
  structural check with writer precedence, rebuilds locations without trusting
  the index, extracts the current canonical subtree (root keeps its envelope,
  nested keeps its shape), and orders edges by `(file, pointer, target)`
  lexically. Sources are nearest-enclosing ids (null outside identified
  objects); only absent identities have null target locations. Null optionals
  contribute no edge; never scan strings for ULIDs. No CLI semantics here:
  id-text parsing and `None` exit treatment belong to T013.
- Fixture and snapshot bytes are read-only in tests: `one_area_one_creature`
  validates to zero diagnostics, `broken_references` to exactly its 15
  checked-in findings, and `expected.json` lists exactly three roots including
  clean `migration_v1/campaign` for T011b's generic gate. Author
  `migration_v1/expected.json` and `migrations.json` through production writers
  and review the item tag-only diff; verify goldens by exact byte comparison,
  never by computing from actuals. Superseded edges would keep immediate
  oracles under `migration_steps/<type>/<from>-to-<to>.json`, never as new
  campaign roots. Never bless, rewrite, or invent a normal case for the
  snapshot comparison. Introspection expected reports/edges are independently
  authored with the same no-bless rule; missing fixtures fail hard.


### Combat vocabulary (T016a)

## Combat vocabulary (T016a)

Public module `combat` (re-exported at the root) holds the four generic
authored combat families: `Ruleset`, `Ability`, `OutcomeTable`, and
`Encounter`, plus `StatDecl`/`StatKindWire`, `ActionPoolTemplate`,
`RefreshWire`, `OutcomeWire`, `DamageEntry`, `OutcomeBandWire`,
`NaturalEffectWire`, `NaturalRuleWire`, and `EncounterParticipant`.
`StatKindWire` grew past `Int` in T136b (below). `OutcomeWire`/`RefreshWire`/
`NaturalEffectWire` mirror the T015 adjacent `type`/`value` snake-case
shapes with hand-written `Deserialize` impls enforcing exact key sets per
variant, for the same unit-variant reason as `Trigger`/`NodeBody`/`Port`/
`DialogueBody`; `Serialize`/`JsonSchema` stay derived. Never depend on
`crpg-rules` for these shapes: the adjacency is a copied convention, and
the sim adapter maps values explicitly.

- Nineteen families: the T012a fifteen plus `crpg.ruleset`, `crpg.ability`,
  `crpg.outcome-table`, `crpg.encounter`, all at version 1 with no edges.
  Twenty-one generated schemas. Layout families are `rulesets/`,
  `abilities/`, `outcome_tables/`, `encounters/` via the shared `family`
  helper; `campaign_document_path` shares the same predicates.
- Read-time (`validate_local`, `Malformed`) covers every single-document
  invariant: nonempty names, unique stat names, `health_stat` naming a
  declared stat, `attributes` nonempty/unique/declared and never holding
  health, `abilities`/`participants` nonempty with unique entries, dice
  byte length `1..=128`, `cost >= 1`, bands nonempty/`<=256`/first-`MIN`/
  strictly increasing, naturals `<=256` with faces `1..=1_000_000` unique,
  and nonzero tick periods. The kernel-mirrored bounds (128 dice bytes,
  256 bands/naturals, six-sided-compatible face ceiling) keep authored data
  inside the T015 constructors; lists with no kernel bound stay unbounded
  in data.
- Semantic codes added: `unknown_stat`, `missing_stat`,
  `invalid_stat_value`, `invalid_cost` (all `Error`). Rules: each listed
  ability's `attribute` must be one of its owning ruleset's `attributes`
  and its `cost` within the pool `max` (orphan abilities skip both);
  encounter prefabs must be creatures carrying every ruleset stat name as
  a whole fixed value with positive health. Fixedâ†’Int conversion is
  checked whole-number only (`raw % 65536 == 0`, integer part always fits
  `i32`); fractional combat stats fail here so the adapter never
  truncates. Existing campaigns are never converted.
- `tests/support/migration_gate.rs::representatives` carries one synthetic
  document per new family so the registry/schema/serde agreement gate
  cannot pass vacuously. The reserved `combat_content` suite pins the
  canonical `rulesets/minimal-d6/` source, its byte-identical campaign
  copies, and every boundary above. `combat_basic` lives at repo-root
  `campaigns/fixtures/combat_basic/` and does not join gate 8's
  `tests/fixtures/expected.json` inventory (three roots unchanged).


### Second-ruleset vocabulary (T017a, specified before source)

## Second-ruleset vocabulary (T017a, specified before source)

Public module `combat` gains the data-owned half of the abstraction proof:
`DefenseWire` (`ActorAttribute` / `TargetStat{stat}`), `AbilityCost`,
`EffectAimWire` (`Slf`/`Target`), `EffectTargetWire` (`Roll`/`Dc`),
`EffectOpWire` (`Add`/`Set`), `PolicyWire` (`StackAll` /
`HighestBonusWorstPenalty` / `HighestPriorityPerName`), `EffectModifierWire`,
and `Effect`, plus `Ruleset.pools` (replacing `action_pool`), `Ability`
`extra_costs`/`ends_turn`/`effect`/`defense`/`natural_die` with the retained
primary-pool `cost` convention (`cost` spends from `pools[0]`, `extra_costs`
covers all other pools). New `ObjectKind::Effect` (after `Encounter`).
`Document` gains `Effect` (`crpg.effect/1`); `Ruleset`/`Ability` tags move to
`/2` with exactly one `/1 â†’ /2` edge each (all-local defaults: `pools:
[action_pool]`; `extra_costs: []`, `ends_turn: true`, `effect: None`,
`defense: ActorAttribute`, `natural_die: None`). `crpg.effect` enters at
version 1 with no edges; registry 19 â†’ 20, schemas 21 â†’ 22 via the existing
generator only.

- The five new adjacent `type`/`value` enums keep hand-written `Deserialize`
  impls enforcing exact key sets per variant (the T016a unit-variant trap);
  `Serialize`/`JsonSchema` stay derived. `effect`/`natural_die` and T136a's
  `round_ticks`/`recovery_ticks`/`repeats_in_real_time` are optional-missing;
  other nullable fields stay required-nullable.
- Read-time (`Malformed`) covers the single-document rules: pools nonempty
  with unique ids, every extra amount `>= 1`, `TargetStat` stat nonempty,
  `mod_type` nonempty, modifiers nonempty (`<= MAX_EFFECT_MODIFIERS`, 4096,
  mirroring T014) with unique ids, `duration_rounds >= 1`, `natural_die`
  well-formed `u32` only. Empty total spend (`cost == 0` with empty
  `extra_costs`) and over-maximum costs are semantic `invalid_cost`, not
  `Malformed`; dice-count validity against `natural_die` is B3's check.
- Semantic codes add only `unknown_pool` (`UnknownPool`); unknown policy
  strings are `Malformed` at read time (closed enum, no `UnknownPolicy`
  code). Over-maximum costs reuse `invalid_cost`; undeclared or
  health-as-defense stats reuse `unknown_stat` (target stats may be any
  non-health declared stat â€” ward qualifies); missing/fractional creature
  stats reuse `missing_stat`/`invalid_stat_value`. Pools are values, not
  indexed objects: the `unknown_pool` walk resolves each extra entry
  against its owning ruleset's templates (naming ability and pool), never
  the object index. Orphan effects keep only generic table/pool checks;
  minimal content has no orphans (tested).
- `tests/support/migration_gate.rs::representatives` gains one synthetic
  document per new/changed shape; `migrations.json` gains the two dictated
  edge rows sharing the existing `migration_v1/campaign` root (no new
  fixture root, no gate-8 growth); the reserved `srd_content` suite pins
  the canonical `rulesets/srd-lite/` source, its byte-identical
  `combat_srd` copies, and every boundary above. `combat_srd` lives at
  repo-root `campaigns/fixtures/combat_srd/` and does not join gate 8.

### As-built clarifications (T017a implementation)

- Empty total spend is enforced in both layers to preserve intent:
  `read_document` rejects `cost == 0` with empty `extra_costs` as
  `Malformed` (keeping the T016a zero-cost structural test green for file
  loads), while `validate` reports the same in-memory state as
  `invalid_cost` at `/cost` (pinning the positioned code for mutations).
  Zero `extra_costs[*].amount` stays `Malformed` at read time; over-maximum
  primary/extra amounts and unknown extra pools stay semantic
  (`invalid_cost` at `/cost` or `/extra_costs/<i>/amount`, `unknown_pool`
  at `/extra_costs/<i>/pool` naming ability and pool).
- Pool-template references use a dedicated `RefPolicy::PoolTemplate`
  inventory variant: enumerated for introspection outbound edges, skipped by
  the generic `check_ref` loop, and resolved in `walk_combat` against the
  owning ruleset's templates. `Effect` has no references of its own; orphan
  effects are clean, orphan abilities keep only generic table/effect refs.
- Migration fixtures share the existing `migration_v1/campaign` root with
  three new files (`rulesets/migrated.json`, `abilities/migrated.json`,
  `outcome_tables/migrated.json` at v1, ids 60/61/62) and three new golden
  keys (ruleset/ability at /2, table identical); existing keys are
  byte-identical. `srd_content` is 14 tests; `cargo test -p crpg-data
  --locked` is 117 passed on Windows/MSVC. The breaking `pools` replacement
  breaks downstream `crpg-sim` compilation until B3; see `tasks/T017a.md`
  for the recorded blocker with options and recommendation.


### T012a review fixes

## T012a review fixes

`tests/support/migration_gate.rs` is a test-only shared checker, included by
the integration coverage suite and private migration unit tests. It snapshots
fixture bytes read-only; mutation tests change only those in-memory snapshots.
Manifest rows deserialize strictly with `u32` versions and unknown-field
rejection. Never cast wider JSON integers into version numbers.

Keep positive gates and negative mutations on the same checker paths. Schema
and serde tag sets must each equal the registry, with one typed read/write
representative per family. Private tests check the actual registry edges,
then invoke each registered single step against its checked-in Value oracle.
The integration suite verifies full campaign byte maps and the exact immediate
oracle inventory: older destinations require their conventional
`migration_steps/<type>/<from>-to-<to>.json`, latest destinations use the
campaign golden. Extra, missing, malformed or wrong-tag oracles fail.
The private two-edge inventory test is synthetic in-memory data only, not a
new production version or permission to regenerate historical fixtures.

- 2026-09-17 (UTC) Â· opencode/gpt-6-astra + T012a review fixes Â· Added working rules for checked version parsing, shared negative-test enforcement and actual single-edge oracle coverage. This supersedes the initial gate's weaker checks without changing the migration API or fixture baselines.

### Action-signature declarations (T029a)

## Action-signature declarations (T029a)

Public module `action_signatures` adds `ActionSignatureStore` (`new`,
`identity`, `len`, `is_empty`, `get`, `iter`, `validate_call`, `read_call`,
`write_call`), `ActionBundleIdentity`, `read_action_signatures`,
`write_action_signatures`, `SignatureError` / `SignatureErrorCode` /
`SignatureLimit`, and the `ACTION_BUNDLE_VERSION` / `MAX_ACTION_*`
constants. The existing `ActionCall`, `ActionSignature`, `ActionParameter`,
`ValueType` and `DataValue` shapes and their schemas are unchanged.

- Declarations only: no handler, function pointer, library loading or
  registration API may be added here; trusted execution binds to the
  identity in `crpg-script` (T029b). Action ids are symbolic strings, never
  combat ability ULIDs or paths.
- Identifiers are 1..=128 UTF-8 **bytes**, preserved exactly (no trimming,
  case folding or normalization). Check `len()`, never `chars().count()`.
- First-failure orders are part of the contract (constructor: action count,
  invalid ids lexically, first duplicate id, then per action lexically:
  parameter count, invalid names, duplicate names; calls: identity, action
  id, argument names, argument count, value budgets, canonical size, unknown
  action, extra arguments, then parameters in declaration order). Pointers
  are logical RFC 6901 locations, not source offsets.
- The revision is content-derived and exact. Reordering action input must
  not change it; any change to an id, parameter order/name/type/required
  flag, or bundle id must. Never add a compatibility fallback here.
- Value budgets run iteratively with lazy pointer construction before any
  serde traversal; canonical call and bundle sizes go through a capped
  writer that counts (or hashes) without retaining oversized output. The
  private canonical views serialize keys lexically; `bundle_revision_is_canonical`
  pins equality with `canonical_json` against a hand-written literal oracle.
- Rejections are read-only for the store and the caller's call.

- 2026-09-30 (UTC) Â· claude-code + T029a implementation Â· Added the declaration-only surface and its working rules (byte-length identifiers, pinned failure orders, exact content-derived identity, bounded traversal) so later edits cannot quietly add execution authority or loosen compatibility.

### Pointer edits and in-memory index (T058a)

## Pointer edits and in-memory index (T058a)

`pointer_edit` holds `PointerEdit` (`Set`, `Insert`, `Remove`),
`PointerEditError` (seven closed variants with pinned lowercase `Display`),
`MAX_EDIT_POINTER_BYTES` (8,192), `pointer_tokens` and `edit_document`.
`loader` adds `campaign_index`. The contract, with its precedence table and
the named tests, is [T058a](../../tasks/T058a.md).

- One implementation each. `pointer_tokens` is the only RFC 6901 decoder;
  `structural_check` is the only structural check and `build_index` the only
  index builder (`campaign_index` just returns what `structural_check`
  already builds). Never add a second decoder, check or index builder, and
  never reorder `structural_check`: `serialize_campaign`, `explain_object`
  and `campaign_index` share its precedence.
- Never migrate or retag in an edit. `edit_document` decodes with
  `decode_current` at the tag the value already carries and refuses a first
  token of `schema`; it must never call `migrate_to_current` or
  `read_document` on the edited value.
- `campaign_index` never takes an engine version and never runs semantic
  validation. Dangling references and other `validate` findings must not
  make it fail. Its precedence is the writer's (layout before local
  invariants), not the loader's; `load_campaign` stays as it is.
- Array tokens are checked only when the traversal reaches an array,
  because `01` is a valid object member name. On an array, `0` or
  `[1-9][0-9]*` is an index; `-` (except as `Insert`'s final token) and
  indices that overflow `usize` are `NotFound`; any other spelling is
  `InvalidPointer`. `Insert` into an object is `NotFound`; `Set` adds
  members.
- Every `Ok` must pass the canonical writer: the last step runs
  `write_document` because a value the strict parser accepts can still
  exceed the 128-level limit once placed. Keep that step.
- Pure and panic-free: no I/O, clock, environment, thread or float; index
  arithmetic is checked and nothing on a caller-controlled path is
  unwrapped. Error messages for `Value` and `Document` are the underlying
  `DataError` text and may quote input; callers truncate when logging.

- 2026-10-04 (UTC) Â· claude-code + T058a implementation Â· Added the pointer-edit and in-memory index surface, its single-implementation, no-migration, no-engine and array-token traps, and the two focused commands, so later edits keep one decoder and one structural check.



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
- 2026-09-26 (UTC) · opencode/muse-spark + T016a crate opening · Documented the four generic combat families, the read-time versus semantic validation split, the checked whole-number conversion preserving existing fixed-point meaning, and the canonical-source versus campaign-copy content ownership before source implementation.
- 2026-09-27 (UTC) · opencode/muse-spark + T017a crate opening · Documented the effect family with effect-scoped policy, the plural-pool primary-cost convention with all-local migration defaults, and the canonical-source parity for the second ruleset before source implementation.
- 2026-09-27 (UTC) · opencode/muse-spark + T017a implementation · Aligned the document with the as-built modifier bound, dual-layer empty-spend rule, shared-root migration oracles, and the downstream sim blocker recorded in tasks/T017a.md.

## Action-signature declarations (T029a)

Public module `action_signatures` (root re-export) is a declaration-only
store for IR action vocabulary under D16. `ActionSignatureStore::new` takes
caller-supplied, trusted `ActionSignature` values keyed by exact action id
(BTreeMap, lexical iteration), preserves parameter declaration order, and
derives an `ActionBundleIdentity`: the caller's bundle ULID plus a BLAKE3
revision over the canonical JSON of `{version, bundle, actions}`. Identity is
exact; there is no semver or additive compatibility. The bundle reader and
writer (`read_action_signatures` / `write_action_signatures`) use the crate's
strict integer JSON with duplicate-key rejection and a 32 MiB byte ceiling;
the reader recomputes the revision and refuses disagreement.
`validate_call` / `read_call` / `write_call` check an `ActionCall` against the
store in one pinned first-failure order: identity, identifiers, argument
count, depth/node/string budgets (iterative, before any recursive serde
walk), counted 64 KiB canonical size, unknown action, extra arguments, then
parameters in declaration order. Errors carry a stable code and logical JSON
pointer. The store holds no runtime implementation, registers no handler and
grants no authority to install one; T029b binds trusted handlers to this
identity. No schema, document family, migration or fixture changed.

- 2026-09-30 (UTC) · claude-code + T029a implementation · Documented the declaration-only store, content-derived bundle identity, strict bundle/call boundaries and first-failure order, stating that declarations carry no execution authority.

## Pointer edits and in-memory index (T058a)

Public module `pointer_edit` (root re-export) and one `loader` function give
the future `crpg-edit` its only JSON-level primitives, so the editor needs no
JSON pointer code, second parser, index builder or `serde_json`/`semver` edge
of its own. The contract is [T058a](../../tasks/T058a.md).

`edit_document(&Document, &PointerEdit)` is pure: it borrows the input, never
changes it, reads no clock, environment or randomness, and returns the same
result for the same inputs. Its flow, first failure wins:

1. `pointer_tokens` decodes the pointer: the 8,192-byte bound
   (`MAX_EDIT_POINTER_BYTES`), then RFC 6901 syntax with one left-to-right
   `~0`/`~1` pass.
2. The empty pointer (`RootPointer`) and a first token of `schema`
   (`SchemaTag`) are refused, so no edit can replace a whole document or
   touch its tag.
3. The new value text, if any, goes through the crate's strict parser
   (`Value`).
4. The document is serialized with its existing serde shape and walked on
   the owned value. Array tokens are checked only when an array is reached:
   `0` or `[1-9][0-9]*` is an index; `-` and indices too large for `usize`
   name nothing (`NotFound`); other spellings are `InvalidPointer`. `Set`
   replaces a member or element, or adds an absent member. `Insert` needs an
   array parent and an index `0..=len` or `-`. `Remove` needs an existing
   target.
5. The edited value is decoded at the tag it already carries, with local
   invariants, and then written once with the canonical writer (`Document`).

The edit is current-tag only: no migration runs and the variant cannot
change, so every `Ok` carries the current tag of its family. Every `Ok` is
also canonical: `write_document` succeeds on it and a strict re-read gives
it back. Normalisation is the typed round trip that loading and writing
files already apply (`Set /_note null` drops the note). Identity is not
policed here; the caller decides which ids may change.

`campaign_index(&BTreeMap<SourcePath, Document>)` is the writer's existing
private structural check, now returning the index it already built: case
collisions and required files, layout, local invariants, `build_index`, then
lock coverage and the assets-lock digest. `serialize_campaign` and
`explain_object` call the same function and discard the index, so all three
share one acceptance and one precedence. On success the index equals the one
`load_campaign` derives for the same documents, which the test suite checks
over every campaign fixture. It never sees an engine version, never migrates
and never runs semantic validation. Its precedence is the writer's, which
reports layout before local invariants where the loader's read phase does
the reverse; `load_campaign` is unchanged.

- 2026-10-04 (UTC) · claude-code + T058a implementation · Documented the pointer-edit flow and precedence, the current-tag-only, canonical and purity guarantees, and `campaign_index` as the writer's structural check returning its index, so the editor's primitives have one owner here.
- 2026-10-25 (UTC) · opencode-go/beam-free + A0-8 · Moved design/detail blocks from crates/crpg-data/AGENTS.md into this doc (new section "Moved from AGENTS.md (A0-8)") so AGENTS.md stays at or below 10,000 bytes; no rule meaning changed.
