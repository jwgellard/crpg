# crpg-data — agent contract

Read the root rules and [T010](../../tasks/T010.md), the binding initial API and
acceptance contract. Architecture: [crpg-data](../../docs/architecture/crpg-data.md).
This document describes the verified T010 implementation plus the T011a
semantic-validation layer and the T012a migration framework; results live in
T010, [T011a](../../tasks/T011a.md) and [T012](../../tasks/T012.md).

## Public surface

Public modules `types`, `ir`, `document`, `canonical`, `package`, `loader`,
`schema`, `error`, `validation`, `migrations`, `introspection`,
`action_signatures` (T029a) re-export their
public items at the root. T010 lists every required model, field and variant.
Operations are `read_document`, `write_document`, `canonical_json`,
`generated_schemas`, `resolve_packages`, `assets_lock_digest`,
`make_campaign_lock`, both lock read/write pairs, `load_campaign`,
`serialize_campaign`, plus T011a's `validate`, `validate_files`,
`diagnostic_for_data_error`, and `campaign_document_path` with the `Diagnostic`,
`DiagnosticCode`, and `Severity` model, plus T012a's `SchemaVersion` with
`schema_versions` and `migrate_document`, plus T013a's `explain_object` returning
canonical report bytes. Primitive newtypes expose validated parsing plus their
prescribed accessors. Do not extend the surface casually.

## Wire and validation traps

- One schema-tagged object per document; fourteen families stay at version 1
  while `crpg.item` is at version 2 behind the single `1 -> 2` tag-only edge.
  Placement and ActionSignature are embedded schema roots, not document kinds.
  Per-type steps are private `fn v1_to_v2` mutations; one sorted registry backs
  both dispatch and `schema_versions`, with no caller registration.
- Reject unknown fields. Only `_note` may be missing; other nullable fields
  need a required-nullable serde helper and matching schema required annotation.
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

## Scope and dependencies

One task, this crate only, with T010's explicit schema/lockfile/docs exceptions.
Only internal dependency: core. Approved external dependencies: workspace serde,
serde_json, blake3; schemars 1 with default features off and derive/std; semver 1
with serde; dev-only workspace proptest. Policy remains unchanged.
Keep unsafe forbidden and missing docs warned; document every public item.
Use ordered tree collections and integer/core values throughout source, examples,
tests and doctests. No floating types/literals/conversions, unordered hash
collections, OS branches, clocks, registry queries, or runtime id generation.

## Verification

Use pinned Rust 1.98.0, default features and normal test profile on Windows/MSVC
and genuine Linux/GNU. All focused commands and gates in T010 plus the T012a
migration gate are mandatory:

```text
cargo run -p crpg-data --example generate_schemas -- schemas
cargo test -p crpg-data --lib migrations --locked
cargo test -p crpg-data --test migrations --locked
cargo test -p crpg-data --test migration_coverage --locked
cargo test -p crpg-data --test round_trip --locked
cargo test -p crpg-data --test canonical --locked
cargo test -p crpg-data --test loader --locked
cargo test -p crpg-data --test resolver --locked
cargo test -p crpg-data --test locks --locked
cargo test -p crpg-data --test validation --locked
cargo test -p crpg-data --test introspection --locked
cargo test -p crpg-data --test combat_content --locked
cargo test -p crpg-data --test schema_drift --locked
cargo test -p crpg-cli --test validate --locked
cargo test -p crpg-cli --test migrate --locked
cargo test -p crpg-data --locked
cargo fmt --all
cargo clippy -p crpg-data --all-targets -- -D warnings
cargo test -p crpg-data
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
cargo deny check
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
git diff --check
```

Generation is an authoring step before read-only verification, not a gate that
blesses drift. Retain property regression seeds; never weaken existing tests.

## Combat vocabulary (T016a)

Public module `combat` (re-exported at the root) holds the four generic
authored combat families: `Ruleset`, `Ability`, `OutcomeTable`, and
`Encounter`, plus `StatDecl`/`StatKindWire`, `ActionPoolTemplate`,
`RefreshWire`, `OutcomeWire`, `DamageEntry`, `OutcomeBandWire`,
`NaturalEffectWire`, `NaturalRuleWire`, and `EncounterParticipant`.
`StatKindWire` is currently `Int` only. `OutcomeWire`/`RefreshWire`/
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
  a whole fixed value with positive health. Fixed→Int conversion is
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
`/2` with exactly one `/1 → /2` edge each (all-local defaults: `pools:
[action_pool]`; `extra_costs: []`, `ends_turn: true`, `effect: None`,
`defense: ActorAttribute`, `natural_die: None`). `crpg.effect` enters at
version 1 with no edges; registry 19 → 20, schemas 21 → 22 via the existing
generator only.

- The five new adjacent `type`/`value` enums keep hand-written `Deserialize`
  impls enforcing exact key sets per variant (the T016a unit-variant trap);
  `Serialize`/`JsonSchema` stay derived. `effect`/`natural_die` are
  optional-missing; other nullable fields stay required-nullable.
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
  non-health declared stat — ward qualifies); missing/fractional creature
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

## Agent log

- 2026-09-10 (UTC) · opencode/gpt-6-astra + T010 crate opening · Established the API, validation, dependency and verification working rules before implementation. The task remains subject to all its acceptance gates.
- 2026-09-10 (UTC) · opencode/muse-spark + T010 implementation · Reconciled the contract with the verified tree and recorded the hand-written kind-tag deserializer trap.
- 2026-09-13 (UTC) · opencode/muse-spark + T011a implementation · Extended the surface with the validation API, recorded the collected-diagnostics and classifier-ownership traps, and added the validation focused command.
- 2026-09-17 (UTC) · opencode/muse-spark + T012a implementation · Recorded the single-registry item-only migration surface, the strict read precedence with clone-then-publish rollback, the three-root fixture and golden-authoring rule, and the migration coverage commands.
- 2026-09-18 (UTC) · opencode/muse-spark + T013a implementation · Extended the surface with the introspection report API, recorded the single-inventory ownership/subtree/ordering traps with no CLI semantics here, and added the introspection plus migrate regression commands.
- 2026-09-26 (UTC) · opencode/muse-spark + T016a crate opening · Extended the surface with the four generic combat families, the hand-rolled enum trap, read-time versus semantic rule split, checked Fixed-to-Int conversion, and the combat_content plus gate-representative obligations before source implementation.

- 2026-09-27 (UTC) · opencode/muse-spark + T017a crate opening · Extended the surface with the effect family, primary-pool cost convention with all-local /1→/2 defaults, defense selector, and unknown_pool walk before source implementation; no dependency, ADR, downstream, or re-baseline decision is taken here.
- 2026-09-27 (UTC) · opencode/muse-spark + T017a implementation · Aligned the surface with the as-built dual-layer empty-spend rule, the PoolTemplate inventory variant, the shared-root migration fixtures, and the 14-test srd_content suite; recorded the downstream sim workspace blocker in tasks/T017a.md with no silent workaround.

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

- 2026-09-17 (UTC) · opencode/gpt-6-astra + T012a review fixes · Added working rules for checked version parsing, shared negative-test enforcement and actual single-edge oracle coverage. This supersedes the initial gate's weaker checks without changing the migration API or fixture baselines.

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

- 2026-09-30 (UTC) · claude-code + T029a implementation · Added the declaration-only surface and its working rules (byte-length identifiers, pinned failure orders, exact content-derived identity, bounded traversal) so later edits cannot quietly add execution authority or loosen compatibility.
