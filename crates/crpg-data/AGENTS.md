# crpg-data — agent contract

Read the root rules and [T010](../../tasks/T010.md), the binding initial API and
acceptance contract. Architecture: [crpg-data](../../docs/architecture/crpg-data.md).
This document describes the verified T010 implementation; results live in T010.

## Public surface

Public modules `types`, `ir`, `document`, `canonical`, `package`, `loader`,
`schema`, `error` re-export their public items at the root. T010 lists every
required model, field and variant. Operations are `read_document`,
`write_document`, `canonical_json`, `generated_schemas`, `resolve_packages`,
`assets_lock_digest`, `make_campaign_lock`, both lock read/write pairs,
`load_campaign`, and `serialize_campaign`. Primitive newtypes expose validated
parsing plus their prescribed accessors. Do not extend the surface casually.

## Wire and validation traps

- One schema-tagged object per document; all 15 tags are explicitly version 1.
  Placement and ActionSignature are embedded schema roots, not document kinds.
- Reject unknown fields. Only `_note` may be missing; other nullable fields
  need a required-nullable serde helper and matching schema required annotation.
- Core ULID aliases and overflow rules govern input; output is uppercase.
  Fixed point is raw signed 32-bit integers. Use data-local schema surrogates
  for core/semver fields, including nested collections; never edit core.
- Tagged values and IR preserve ordered arrays. Do not persist interned handles.
- Canonical JSON sorts keys recursively regardless of feature unification,
  uses two spaces and exactly one LF, and rejects non-integer numeric forms.
  Reject duplicate keys before constructing an ordinary JSON value.
- Read order: syntax/duplicates/numbers; envelope; supported tag; typed fields;
  lock-local invariants. Loader whole-input order: case collisions/required
  files; read; layout; engine; duplicate-free index; package coverage; digest.
  Writer shares layout/local/index/lock checks but has no engine check.
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
and genuine Linux/GNU. All focused commands and gates in T010 are mandatory:

```text
cargo run -p crpg-data --example generate_schemas -- schemas
cargo test -p crpg-data --test round_trip --locked
cargo test -p crpg-data --test canonical --locked
cargo test -p crpg-data --test loader --locked
cargo test -p crpg-data --test resolver --locked
cargo test -p crpg-data --test locks --locked
cargo test -p crpg-data --test schema_drift --locked
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
