# crpg-data — agent contract

Read the root rules and [T010](../../tasks/T010.md), the binding initial API and
acceptance contract. Architecture: [crpg-data](../../docs/architecture/crpg-data.md).
This document describes the verified T010 implementation plus the T011a
semantic-validation layer and the T012a migration framework; results live in
T010, [T011a](../../tasks/T011a.md) and [T012](../../tasks/T012.md).

## Public surface

Public modules `types`, `ir`, `document`, `canonical`, `package`, `loader`,
`schema`, `error`, `validation`, `migrations`, `introspection` re-export their
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

## Agent log

- 2026-09-10 (UTC) · opencode/gpt-6-astra + T010 crate opening · Established the API, validation, dependency and verification working rules before implementation. The task remains subject to all its acceptance gates.
- 2026-09-10 (UTC) · opencode/muse-spark + T010 implementation · Reconciled the contract with the verified tree and recorded the hand-written kind-tag deserializer trap.
- 2026-09-13 (UTC) · opencode/muse-spark + T011a implementation · Extended the surface with the validation API, recorded the collected-diagnostics and classifier-ownership traps, and added the validation focused command.
- 2026-09-17 (UTC) · opencode/muse-spark + T012a implementation · Recorded the single-registry item-only migration surface, the strict read precedence with clone-then-publish rollback, the three-root fixture and golden-authoring rule, and the migration coverage commands.
- 2026-09-18 (UTC) · opencode/muse-spark + T013a implementation · Extended the surface with the introspection report API, recorded the single-inventory ownership/subtree/ordering traps with no CLI semantics here, and added the introspection plus migrate regression commands.

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
