# crpg-data — agent contract

Read the root rules and [T010](../../tasks/T010.md), the binding initial API and
acceptance contract. Architecture: [crpg-data](../../docs/architecture/crpg-data.md).
This is the how-to-work contract. Design, behaviour and history live in the
architecture doc; detail moved out by A0-8 is under
`docs/architecture/crpg-data.md` §"Moved from AGENTS.md (A0-8)".

## Public surface

Public modules and operations are enumerated in T010 and the architecture doc
§Module flow / §Public surface / §"Moved from AGENTS.md (A0-8)" §Public surface.
Do not extend the surface casually. Primitive newtypes expose validated parsing
plus their prescribed accessors.

## Invariants and traps

- One schema-tagged object per document; fourteen families stay at version 1
  while `crpg.item` is at version 2 behind the single `1 -> 2` tag-only edge.
  Placement and ActionSignature are embedded schema roots, not document kinds.
  (Full trap list: arch doc §"Moved from AGENTS.md (A0-8)" §"Wire and validation traps".)
- Reject unknown fields; only the listed optional-missing fields may be absent.
- Core ULID aliases and overflow rules govern input; output is uppercase. Fixed
  point is raw signed 32-bit integers. Use data-local schema surrogates for
  core/semver fields, including nested collections; never edit core.
- Tagged values and IR preserve ordered arrays. Do not persist interned handles.
- Canonical JSON sorts keys recursively, uses two spaces and exactly one LF, and
  rejects non-integer numeric forms and duplicate keys.
- Read/loader/writer/migration order and idempotency: arch doc §Module flow and
  §Migrations (T012a); `migrate_document` clones then publishes, validates
  current input without rewriting, and stays idempotent.
- Resolver checks requirement kinds, then all candidate conflicts (including
  unused entries), then resolves package-id order. Semver precedence ties use
  lexically greatest complete version text. Never silently sort lock arrays.
- Object ids, package coordinates, and logical paths are different authorities.
  Caller-mutable index entries must not influence output or bypass validation.
- No filesystem access in the library; generation example owns I/O. No builds or
  tests may rewrite schema/fixture baselines. Drift compares exact bytes.
- Internally `kind`-tagged enums (`Trigger`, `NodeBody`, `Port`,
  `DialogueBody`) use hand-written `Deserialize` impls enforcing exact key sets
  per variant; derived `deny_unknown_fields` ignores extra fields on unit
  variants. Keep the manual impls and the unknown-field tests. `Serialize`/
  `JsonSchema` stay derived so schemas keep `additionalProperties: false`.
- Semantic validation is collected and positioned, never fail-fast: rebuild
  occurrence/ownership tables from `documents` in lexical-path, authored order
  and ignore the caller-mutable index entirely.
- `campaign_document_path` shares the loader's `family`/`area_file`/
  `locale_shape` predicates; a second path-family list is a drift defect.
  Classifier rejections stay `invalid_path` through `diagnostic_for_data_error`;
  `io` belongs to filesystem-owning callers only.
- Introspection shares one private `inventory` for identity locations, typed
  traversal, and pointer construction; a second reference-field list is a drift
  defect. `explain_object` reuses the writer's structural check with writer
  precedence. No CLI semantics here.
- Fixture and snapshot bytes are read-only in tests; never bless, rewrite or
  invent cases (counts and no-bless rule: arch doc §"Moved from AGENTS.md
  (A0-8)").
- Combat/second-ruleset/kernel-range/action-signature/pointer-edit/migration-gate
  working rules: arch doc §"Moved from AGENTS.md (A0-8)" and the named sections.

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
cargo test -p crpg-data --test pointer_edit --locked
cargo test -p crpg-data --test campaign_index --locked
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

## Design pointers

- Combat vocabulary: arch doc §Combat vocabulary (T016a)
- Second-ruleset vocabulary: arch doc §Second-ruleset vocabulary (T017a)
- Kernel-range surface: arch doc §Kernel-range surface (T136b)
- Real-time fields: arch doc §Real-time fields (T136a)
- Action signatures: arch doc §Action-signature declarations (T029a)
- Pointer edits / index: arch doc §Pointer edits and in-memory index (T058a)
- Migrations: arch doc §Migrations (T012a), §Migration gate hardening after review
- Semantic validation: arch doc §Semantic validation (T011a)
- Introspection: arch doc §Introspection (T013a)
- Moved detail: arch doc §"Moved from AGENTS.md (A0-8)"

## Agent log

- 2026-09-10 (UTC) · opencode/gpt-6-astra + T010 crate opening · Established the API, validation, dependency and verification working rules before implementation. The task remains subject to all its acceptance gates.
- 2026-09-10 (UTC) · opencode/muse-spark + T010 implementation · Reconciled the contract with the verified tree and recorded the hand-written kind-tag deserializer trap.
- 2026-09-13 (UTC) · opencode/muse-spark + T011a implementation · Extended the surface with the validation API, recorded the collected-diagnostics and classifier-ownership traps, and added the validation focused command.
- 2026-09-17 (UTC) · opencode/muse-spark + T012a implementation · Recorded the single-registry item-only migration surface, the strict read precedence with clone-then-publish rollback, the three-root fixture and golden-authoring rule, and the migration coverage commands.
- 2026-09-18 (UTC) · opencode/muse-spark + T013a implementation · Extended the surface with the introspection report API, recorded the single-inventory ownership/subtree/ordering traps with no CLI semantics here, and added the introspection plus migrate regression commands.
- 2026-09-26 (UTC) · opencode/muse-spark + T016a crate opening · Extended the surface with the four generic combat families, the hand-rolled enum trap, read-time versus semantic rule split, checked Fixed-to-Int conversion, and the combat_content plus gate-representative obligations before source implementation.
- 2026-09-27 (UTC) · opencode/muse-spark + T017a crate opening · Extended the surface with the effect family, primary-pool cost convention with all-local /1→/2 defaults, defense selector, and unknown_pool walk before source implementation; no dependency, ADR, downstream, or re-baseline decision is taken here.
- 2026-09-27 (UTC) · opencode/muse-spark + T017a implementation · Aligned the surface with the as-built dual-layer empty-spend rule, the PoolTemplate inventory variant, the shared-root migration fixtures, and the 14-test srd_content suite; recorded the downstream sim workspace blocker in tasks/T017a.md with no silent workaround.
- 2026-10-04 (UTC) · claude-code + T058a implementation · Added the pointer-edit and in-memory index surface, its single-implementation, no-migration, no-engine and array-token traps, and the two focused commands, so later edits keep one decoder and one structural check.
- 2026-10-23 (UTC) · claude-code/sonnet-5.5 + T136a · Recorded the rule for joining a family at its current version and added the three optional real-time fields to the optional-missing exceptions; zero `round_ticks`/`recovery_ticks` are `Malformed` read-time checks, with no semantic rule.
- 2026-10-23 (UTC) · claude-code/sonnet-5.5 + T136b · Recorded the kernel-range wire vocabulary, the additive-variant rule at the current tag (D32c), the odd-node-count reading of the 4096 bound and the first warning diagnostic.
- 2026-10-25 (UTC) · opencode-go/beam-free + A0-8 · Trimmed AGENTS.md to how-to-work by moving design/detail blocks to docs/architecture/crpg-data.md §"Moved from AGENTS.md (A0-8)" and condensing duplicate invariants; no rule meaning changed.