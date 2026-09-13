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

Document reading passes through canonical parsing, schema selection, typed decoding,
then lock-local checks. Loading adds layout, engine compatibility, object indexing,
package coverage and assets-lock digest checks. Serialization shares those checks
except engine compatibility and derives identity independently of the mutable index.
Callers own filesystem I/O.

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
bytes and typed references, never persisted interned handles. T011 semantic
diagnostics, T012 migrations, T013 wrappers and T014 conversion remain separate work.

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

## Agent log

- 2026-09-10 (UTC) · opencode/gpt-6-astra + T010 crate opening · Established approved module and authority boundaries before source implementation. This opening record does not claim completed implementation or passing gates.
- 2026-09-10 (UTC) · opencode/muse-spark + T010 implementation · Reconciled the opening record with the verified implementation, documenting the hand-written kind-tag deserializers and the checked-in schema/fixture bytes.
