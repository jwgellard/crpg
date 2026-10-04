# crpg-edit — agent contract

Read the root rules, [POST-T018-RULES](../../tasks/POST-T018-RULES.md) and
[T058](../../tasks/T058.md). T058 holds the exact public API, the §5.3
algorithm and precedence, the §6 history rules, the §7 save rules and the
user's Decisions, which are normative. Also read [T058a](../../tasks/T058a.md)
for the `crpg-data` primitives this crate is built on. Architecture:
[crpg-edit](../../docs/architecture/crpg-edit.md). This document describes
the T058 headless document, command, undo and validation API.

## Public surface

Modules `command`, `document` and `error`, plus the private `history`. Every
public item is re-exported at the root by name (no glob re-export). T058 §3
is exhaustive. Adding a public item, trait, subscription or callback API,
filesystem function, ID generator, serde derive, command wire format or
merge/coalesce API needs a new approved contract.

- `command`: `EditTarget` (`Document(path)`, `Object(id)`) and the closed
  nine-variant `EditCommand`.
- `document`: `MAX_BATCH_COMMANDS` (1,024), `MAX_POINTER_BYTES` (4,096),
  `MAX_VALUE_BYTES` (1 MiB), `MAX_DOCUMENT_BYTES` (4 MiB), `MAX_UNDO_ENTRIES`
  (256), `MAX_HISTORY_BYTES` (32 MiB); `CampaignDocument` (`open`, `campaign`,
  `canonical_files`, `revision`, `apply`, `apply_batch`, `undo`, `redo`,
  `undo_depth`, `redo_depth`, `history_bytes`, `validate`, `save_plan`,
  `mark_saved`); `ChangeKind`, `PathChange`, `CommandReceipt`, `SavePlan`.
- `error`: `EditLimit` and the 16-variant `EditError`, with pinned lowercase
  `Display` text (T058 §3.3) and no error source.

## Invariants

1. **Atomic.** Every `Err` from `apply`, `apply_batch`, `undo`, `redo` and
   `mark_saved` leaves `canonical_files()`, `campaign()`, `revision()`, both
   stacks, `history_bytes()` and the saved baseline exactly as they were.
   All work happens on a working copy that is installed only at commit.
2. **Precedence is contract.** Per command: input caps, resolution,
   protection, mutation, document cap, structure (`campaign_index`),
   identity. First failure wins, and a batch reports the failing command's
   index. Do not reorder the steps.
3. **Exact undo bytes.** An entry stores the canonical bytes before and after
   every changed path. After `undo`, `canonical_files()` equals the map
   before the undone apply byte for byte. After `redo`, it equals the map
   after it.
4. **Redo invalidation.** Only a committed (non-no-op) apply clears redo. A
   failed or no-op apply, `undo`, `redo` and `mark_saved` never clear either
   stack. The revision counts commits, undos and redos only.
5. **Bounds.** After each commit, evict the oldest undo entry while
   `undo_depth() > MAX_UNDO_ENTRIES` or `history_bytes() > MAX_HISTORY_BYTES`.
   An entry larger than the history cap on its own is refused
   (`HistoryTooLarge`) before commit, so the newest entry always survives
   and every successful apply can be undone once.
6. **One implementation of everything structural.** JSON parsing, pointer
   decoding, typed decoding, the index, the writer's structural checks and
   semantic validation are `crpg-data`'s (`edit_document`, `pointer_tokens`,
   `campaign_index`, `write_document`, `read_document`, `validate`). This
   crate has no parser, pointer decoder, index builder or validator.
7. **Structural failures block; semantic findings report.** Receipts carry
   `crpg_data::validate` and nothing else. A dangling reference never
   refuses a command.
8. **Caller-supplied ids.** Every new object id comes from the caller. An
   existing object's id never changes (`IdentityChanged`).
9. **No migration.** No call to a migration API. `read_document` is used only
   on bytes this crate wrote itself (undo/redo). Every document written
   carries its current tag.
10. **No I/O, no clock, no entropy.** Ordered tree collections, `Vec` and
    `VecDeque` only. No unordered hash collections, clocks, threads,
    environment reads, randomness, floating-point types or OS branches.
    `CampaignDocument` is plain owned data, `Send + Sync` (tested).

## Dependencies

Exactly the approved T058 manifest (T058 §8 with Decisions answer 3):
normal path dependencies `crpg-core`, `crpg-data` and `crpg-rules`, and the
workspace `proptest` as the only dev-dependency. No external normal
dependency, and no JSON, version-range or error-derive crate. `ALLOWED` in
`tools/lint/deps.py` is a ceiling, not an approval. Any new edge, including a
dev edge or a feature, needs its own approved record.

## Known traps

- **Never trust the opened index.** `open` drops `LoadedCampaign::index` and
  derives its own with `campaign_index`. The index is re-derived after every
  command, never patched by hand.
- **Never add a typed command that bypasses `edit_document`.** Typed
  conveniences (`PlaceInstance`, `MovePlacement`) build a pointer edit and go
  through the same call. A new typed command needs an editor that shows the
  generic form is not enough (T058 Decisions, answer 12).
- **Never let semantic findings block** apply, undo, redo or save.
- **Never generate ids** or read a clock, even for convenience in tests.
- An object-relative pointer is decoded with `pointer_tokens` before the
  index pointer is prefixed, so `slug` is `InvalidPointer` and is never read
  as part of the prefix. A pointer the decoder refuses is "not protected".
  The same refusal then surfaces from the mutation step.
- Lock paths are required files, so resolution reports `PathExists` for a
  create or rename onto them before protection is reached (see the T058
  completion record).
- The no-op test compares canonical maps, not typed documents. A no-op
  returns `Ok` with empty `changes` and the unchanged revision.
- The property suites use a fixed proptest seed, so each run is reproducible.
  Retain any regression seed file a failure produces.
- Fixture bytes are embedded read-only with `include_bytes!` by explicit
  path. No test writes a file.
- The DoD `rg` in T058 §12 covers this whole directory, this file included.
  Keep the listed words out of prose here too.

## Verification

Pinned toolchain, default features and normal test profile, on Windows/MSVC
and genuine Linux/GNU:

```text
cargo test -p crpg-edit --test commands --locked
cargo test -p crpg-edit --test history --locked
cargo test -p crpg-edit --test save --locked
cargo test -p crpg-edit --test command_sequences --locked
cargo test -p crpg-edit --test mvp_authoring --locked
cargo clippy -p crpg-edit --all-targets --locked -- -D warnings
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
cargo deny check
git diff --check
cargo test -p crpg-edit --locked
```

Never weaken an existing test to make a build pass.

## Agent log

- 2026-10-04 (UTC) · claude-code + T058 · Wrote the crate's first working contract with its first code: the exact surface, the atomicity, precedence, history and no-migration invariants, the approved dependency set and the traps, so T059 and the editor views extend the document rather than route around it.
