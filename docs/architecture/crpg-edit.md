# crpg-edit architecture

## Scope

`crpg-edit` is the headless editor document model. Spec §11.2 puts the
document, undo and validation in Rust so that the GUI editor, `crpgc apply`
and AI agents all go through one implementation and cannot bypass the
invariants it enforces. D18 orders it before any editor UI. The E022 ledger
row "`EditCommand`, receipts, diagnostics, undo (headless first)" names this
crate as owner.

What exists today is the first slice, specified by [T058](../../tasks/T058.md)
(contract, decisions and completion record):

- **`CampaignDocument`**: an open campaign that is always structurally valid,
  with its canonical bytes per path, a revision counter and a saved baseline.
- **`EditCommand`**: a closed set of nine mutations. Six are generic
  (`CreateDocument`, `DeleteDocument`, `RenameDocument`, `SetValue`,
  `InsertValue`, `RemoveValue`). Three are identity-level or typed
  conveniences (`DeleteObject`, `PlaceInstance`, `MovePlacement`).
  `EditTarget` addresses a pointer edit either by document path or by object
  ULID.
- **Atomic `apply` / `apply_batch`** returning a `CommandReceipt` (the new
  revision, the changed paths, and `crpg_data::validate` over the result).
  A batch is one undo entry.
- **Bounded undo and redo** that restore canonical bytes exactly.
- **A pure `SavePlan`** with `mark_saved`.

There is no command wire format, ID generator, subscription API, filesystem
function or live-session editing. Each is planned work (below), not a
placeholder.

## Where it sits

Above `crpg-data`, below the CLI and the Godot bridge. Normal dependencies are
`crpg-core` (the `Ulid` in the public API), `crpg-data` (the document model,
writer, structural index and validation) and `crpg-rules`. The `crpg-rules`
edge was approved and taken with T058 (T058 Decisions, answer 3), ahead of
need. No T058 code uses it yet. The only dev-dependency is workspace
`proptest`. There are no external normal dependencies.
`tools/lint/deps.py`'s `ALLOWED` row is exactly those three crates, and the
`crpg-server` row excludes `crpg-edit`, so the authoritative server never
links the editor.

Consumers: T059 (`crpgc apply`, which owns the command wire format, ID
generation, the open preflight and file writes) and T060 onward (editor views
through the bridge, which add `campaign()` for `explain_object` and the
read-only explorer).

## Module flow

Three public modules and one private one. Every public item is re-exported at
the crate root by name.

- **`command`**: `EditTarget` and `EditCommand`, plain data with no behaviour.
- **`error`**: `EditLimit` and `EditError`, with pinned lowercase `Display`
  text and no error source.
- **`document`**: the six bounds, `CampaignDocument`, `ChangeKind`,
  `PathChange`, `CommandReceipt` and `SavePlan`. This module holds the T058
  §5.3 algorithm. It clones the state into a working copy. For each command
  in order, it then checks input caps, resolves paths and ids against the
  working copy, checks protection, and mutates. It re-writes every touched
  document against the document cap, re-derives the index with
  `crpg_data::campaign_index` and checks identity. After the last command it
  diffs canonical bytes, builds the history entry, and commits or reports a
  no-op.
- **`history`** (private): undo entries oldest first and redo entries
  newest-undone last. Each entry is a list of `(path, before, after)`
  canonical bytes, and its size is the byte sum (§6.3).

All JSON work is `crpg-data`'s. Pointer edits go through
`crpg_data::edit_document`. The protection check reads the first token
through `crpg_data::pointer_tokens`. The index and the writer's structural
checks come from `crpg_data::campaign_index`, canonical bytes from
`write_document`, and undo re-reads stored bytes with `read_document`. The
crate has no parser, pointer decoder, index builder or validator of its own
([T058a](../../tasks/T058a.md) added the public primitives for exactly this).

## Structural failures block; semantic findings are reported

A command whose result fails the writer's structural acceptance is refused as
`Rejected`. That covers layout, required files, duplicate ids, lock coverage
and the assets digest, through `campaign_index` with `crpg-data`'s
precedence. Typed shape and local invariants are refused as `Pointer`, through
`edit_document`. Semantic findings never block apply, undo, redo or save.
Dangling references, missing locale keys, unreachable nodes and duplicate
slugs appear in every receipt's `diagnostics` (T058 Decisions, answers 5 and
10; spec §11.4 Problems panel). So every reachable state is loadable by
`crpg_data::load_campaign`, and every intermediate authoring state can be
represented.

Identity is the editor's own rule on top of that. Object ids are
caller-supplied and never change after creation (spec §4.3, T058 Decisions,
answers 2 and 11). Lock documents and the manifest's `package`, `engine` and
`requires` members are read-only to commands (answer 9).

## No migration

`crpg-edit` never migrates. `CreateDocument` takes a typed, current-version
`Document`, and `edit_document` re-decodes at the current tag only. A
document that the caller's `load_campaign` migrated in memory is therefore
written at its current tag only when a command changes it. The saved baseline
is the canonical form at `open`, so an untouched migrated file never appears
in a save plan. Callers refuse to open a campaign that needs `crpgc fmt` or a
migration first (T058 Decisions, answer 8). `crpg-edit` does not enforce
that.

## Spec §11.2 deviation: a pure save plan

Spec §11.2 sketches `save(&self) -> Result<Vec<PathBuf>>`. T058 replaces it
with `save_plan()` (paths to write with exact bytes, and paths to remove,
relative to the saved baseline) and `mark_saved(&plan)`. That keeps the
library I/O-free like `crpg-data`. Writing, ordering and atomic replacement
belong to T059 and T060 (T058 Decisions, answer 7). The spec text is
unchanged.

## Planned

- **T059** `crpgc apply`: the JSON form of `EditCommand` (T058 Decisions,
  answer 4), ID generation from host entropy, the open preflight, and file
  writes.
- **T060–T068** editor views through the bridge. Typed dialogue, quest and
  graph commands are added only when their editors (T066, T067, T074) show
  the generic form is not enough (answer 12).
- **D03 / T085** privileged live edits of a running session. These are out of
  scope here; `crpg-edit` edits authored data only.
- Measured optimisation of the per-command clone and re-index, with a
  D06-style workload (answer 14), never with a second validator.

## Agent log

- 2026-10-04 (UTC) · claude-code + T058 · Wrote the crate's architecture doc with its first code: its position above `crpg-data`, what exists versus what is planned, the structural-blocks/semantic-reports split, the no-migration rule and the spec §11.2 save deviation, so T059 and T060 build on the document rather than reinterpret it.
