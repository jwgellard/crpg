# D0 — input limits and authored-file safety (PROPOSED)

**Decision status: PROPOSED — awaiting human approval.** This is a source-grounded
decision draft for remediation plan §7, not an executable task contract, an ADR
acceptance, or permission to implement. Every strategy, numeric limit, new error,
and handoff below is **PROPOSED**, including imperative wording in the proposals.
Existing behavior and existing policy are identified separately.

## 1. Baseline, scope, and evidence status

- Source reviewed on clean `master` at `a302320`, following T016/T017 merge
  `4686a73`; confirmed with `git status --short --branch` and recent Git history.
- **Implemented / merged:** the source paths below exist in that baseline.
  Historical dual-native verification is recorded by the owning tasks and root
  `AGENTS.md`; it is not fresh verification by this draft.
- **Specified:** [remediation plan §7](REMEDIATION_PLAN_2026-09-27.md#7-workstream-d--protect-authored-content-and-bound-acquisition)
  assigns D0–D5. It does not supply approved numeric limits or a filesystem protocol.
- **Verified here:** source inspection and baseline state only. No runtime,
  crash, memory-use, Windows replacement, Linux durability, or full code gates
  were run. This new draft is neither approved, implemented, behaviorally verified,
  nor merged.
- Root and data/CLI/testkit `AGENTS.md`, architecture introductions, T010's opening
  contract, and `tools/lint/deps.py::ALLOWED` were read. There are no additional
  scoped `AGENTS.md` files under `docs/` in the inspected tree.

Scope is campaign documents and locks, the CLI's supplied package catalog,
testkit replay/golden input, and CLI authored writes. Asset media, archive
extraction, compressed saves, and network frames need their own owner budgets.
The spec's SnapshotBackend “temp file, fsync, rename” requirement (§8) is a
persistence specification, not evidence of an implemented CLI transaction.
[E017](../../tasks/E017-t018-interface-debt.md) retains intent/delta shapes,
sequence/tick policy, network rate/payload caps, and rejection codes. D0 neither
duplicates nor resolves E017; an opaque replay payload cap is not a network cap.

### Compact source evidence (paths relative to repository root)

| Evidence | Implemented behavior / gap observed |
|---|---|
| `crates/crpg-data/src/canonical.rs::StrictValue, parse, canonical_json` | Rejects duplicate keys and noninteger numbers; recursively constructs collections without explicit application count/string budgets. Canonicalization serializes, parses, then pretty-serializes, creating multiple representations. Serde recursion protection is not an approved application depth policy. |
| `crates/crpg-data/src/document.rs::read_document, write_document`; `src/loader.rs::load_campaign, structural_check, serialize_campaign` | Pure supplied-byte APIs; case collisions/required files precede lexical reads, then layout, engine, index, coverage/digest. No aggregate acquisition cap. Writer rebuilds checks rather than trusting the mutable index. |
| `crates/crpg-data/src/migrations/mod.rs::dispatch, migrate_document` | Full chain selected before steps; public Value API clones then validates before publishing. Current Value is validated without normalization. Today item/ruleset/ability have /1→/2 edges, effect is /1; old architecture paragraphs alone are not the current inventory. |
| `crates/crpg-data/src/package.rs::PackageCandidate, resolve_packages`; `src/error.rs`; `src/validation.rs` | Catalog entries are strict typed data; resolver checks requirement kinds, candidate conflicts, then selection. `DataError` has no resource-limit variant. Data owns diagnostic conversion and semantic positioning. |
| `crates/crpg-cli/src/main.rs::RealFs, collect_into, collect_file_entry` | `fs::read`; directory names fully accumulated before sorting; recursive sorted depth-first traversal. Records ignored regular files for alias checks but does not read their content. Ignored symlinks are ignored, not followed; recognized symlinks are rejected. |
| Same file: `run_lock_with, read_regular_input, write_lock_output` | Campaign read+decode, assets read+decode, catalog read+direct typed `Vec<PackageCandidate>` decode, resolve/write, input/output identity checks. Existing output is raw-byte compared, then reread/truncated; absent output uses `create_new`. |
| Same file: `run_migrate_with, plan_updates, reject_rewrite_aliases, execute_rewrites, run_fmt_with` | Whole-map preflight before writes; lexical differing files only; observed symlink/type and source-byte checks; `same-file` alias rejection; truncate/write/sync, stop at first error. Earlier updates can persist. No locking or secure live-tree snapshot. |
| `crates/crpg-testkit/src/replay.rs::read_replay, validate_replay, play_and_verify` | Whole-file read and ordinary derived decoding precede semantic validation. Existing maxima: 1,000,000 ticks and inputs. Version→metadata→counts→schedule; apply before golden acquisition. Payload is `serde_json::Value`, unknown envelope fields ignored. |
| `crates/crpg-testkit/src/harness.rs::verify_golden, write_golden` | Whole-file read and whole-file UTF-8 check before comparison; skip `#` lines, exact lowercase hex, truthful mismatch variants. Writers use `File::create`/`fs::write`, not staged authored-save machinery. |

## 2. PROPOSED resource policy for approval

Use fixed initial ceilings, not environment overrides or “unlimited” switches.
They are conservative product-policy candidates, **not measured capacity claims**.
D1–D3 specification must inventory current fixtures and representative creator
content against them before adoption. `KiB = 1024`, `MiB = 1024²`; all maxima are
inclusive and counters use checked arithmetic. Multiple limits intersect: a
maximum-size item need not fit a maximum-count collection.

| Input | Per-file bytes | Aggregate bytes / counts | Depth and subordinate bounds |
|---|---:|---|---|
| Campaign recognized documents, including both locks | 8 MiB raw; 8 MiB canonical output | 128 MiB raw supplied/collected map; independently 128 MiB output map; 4,096 documents including locks | JSON container depth 64; 65,536 entries per array/object; 262,144 value nodes/document; 2,000,000 nodes/campaign; decoded string 256 KiB, decoded key 1 KiB |
| CLI catalog | 8 MiB | One catalog, 16,384 candidates; `lock` input set (campaign+assets+catalog) 24 MiB, output separately ≤8 MiB | JSON depth 16; candidate root array exception 16,384, other containers ≤65,536; 262,144 nodes; decoded strings 4 KiB and keys 1 KiB |
| Replay | 128 MiB raw and independently pretty output | One replay; existing 1,000,000 inputs and ticks retained; payload source spans together ≤64 MiB; 4,000,000 JSON value nodes/file | Whole JSON depth 64; each payload depth ≤32; payload source span ≤64 KiB, ≤8,192 nodes; non-input arrays/objects ≤65,536 entries; decoded payload string ≤32 KiB/key ≤1 KiB; each identity string ≤4 KiB |
| Golden | 80 MiB | One file; ≤1,000,000 non-comment lines, ≤1,024 comment lines, ≤1 MiB comment bytes | No JSON nesting; physical line ≤4 KiB excluding CR/LF. Valid hash lines remain exactly 64 lowercase hex characters. Blank lines remain malformed, not comments. |

PROPOSED counting definitions: a scalar or container counts as one value node;
keys count toward key/string bytes and member count, not as value nodes. Root
container depth is 1; scalar root depth is 0; payload depth restarts at its root,
while also respecting whole-replay depth. Raw bytes include whitespace and LF.
Payload source spans include its value token and internal whitespace; aggregate
counts include every input payload, not just distinct payloads. Unknown replay
fields also consume file/node/depth/string budgets (32 KiB string default) while
remaining semantically ignored. D3's in-memory payload byte measure is compact
JSON encoded length; file input additionally obeys source-span bytes. Pretty
output is independently counted, so an accepted compact input can fail an
explicit save without any target mutation.

PROPOSED successful replay writes must pass every reader limit in their actual
emitted pretty representation, including per-payload and aggregate source-span
bytes, not only compact in-memory payload length and whole-output bytes. Check
before target mutation; preserve the current pretty format and opaque payload
meaning. A successful save must not create a replay rejected by its own reader.

PROPOSED campaign traversal budgets (CLI acquisition, including ignored entries):
32 directory levels below root (root depth 0), 32,768 total entries, 8,192
entries in any directory, 1,024 encoded bytes per relative logical path, and
8 MiB aggregate retained encoded path/name bytes. Count directories, ignored
files and links, and recognized files; do not prune ignored directories or
read ignored media to count its content bytes. The source-map API applies the
path-byte budget too, before duplicating paths. `lock`/replay direct paths do
not trigger directory walks; cap their ancestor-check work at 128 components
and 32 KiB encoded path bytes, without reclassifying non-Unicode as usage.

Existing narrower semantic limits (dice text, outcome bands, effect modifiers,
etc.) remain authoritative. These structural limits do not increase any of them.
PROPOSED no new blanket cap on `canonical_json` used for diagnostics/schemas:
document writers get a bounded serialization route, while generic diagnostic
output must not accidentally inherit a single-document limit. Public in-memory
document/Value entry points must count before cloning, indexing, migration,
canonicalization or expensive validation. Callers already allocated their input;
the API can bound additional work, not undo that allocation.

## 3. PROPOSED enforcement and error precedence

### Before allocation or expensive transformation

- Acquisition: read through a fixed-size buffer, limited by remaining per-file
  and aggregate budget plus one detection byte. Check before extending a Vec;
  never reserve from file metadata or an untrusted sequence hint. Metadata size
  may reject early but is insufficient: growth during read must hit the same
  cap. Checked sums must precede insertion/copying. Bound the recheck reads too.
- Directory enumeration: enforce count/name-byte budget as entries arrive,
  before accumulating the unbounded list; sort only an admitted directory.
  An over-budget listing fails at the directory, not whichever unsorted entry
  happened to overflow. Within-budget traversal retains existing sort order.
- Decode: use a bounded lexical preflight over admitted bytes, with budget-aware
  collection decoding where the owner has the existing serde edge; do not first
  build `Value`, `String`, or `Vec` and then
  check lengths. In particular a serde string visitor is too late to bound the
  decoder's escaped-string scratch buffer. Preflight validates escapes/counts
  decoded UTF-8 bytes without constructing the decoded string. It must recognize
  JSON strings correctly, not count brackets inside strings as nesting.
- Data keeps its duplicate-rejecting integer parser; catalog stays directly
  typed, without an intervening `Value` that would erase duplicate fields.
  Replay keeps serde JSON numeric/payload semantics, not data's integer-only
  parser. Stop before decoding element N+1 or pushing member N+1.
  CLI has serde_json but no direct serde dependency: its catalog preflight must
  establish all structural budgets before existing typed decoding, or report
  the need for separately approved dependency/API work. Share campaign node
  counters across lexical document reads before constructing the next node.
- Migration: pre-count a supplied Value iteratively before the first clone;
  budget each step's output and final typed/canonical output. Preserve
  clone-then-publish on failure. Writer counting/sink limits precede output
  buffering and file creation. Budget intermediate expansion, not only input.
- Use bounded iterative traversal where recursion could exhaust the stack;
  no disabling serde recursion limits. Allocation ceilings are not exact RSS
  guarantees: map overhead, typed clones, indexes, diagnostics and hash vectors
  remain costs to measure in the child specifications.

### Precedence and reporting

PROPOSED an explicit resource-admission phase before existing content semantics.
An oversized malformed file may therefore report a limit before a syntax error.
This is a deliberate new boundary, not a claim that old precedence never changes.

1. CLI usage remains first. Root/ancestor/type/list/read failures retain their
   acquisition order. Directory admission failure precedes visiting its children;
   admitted directories keep sorted depth-first first-failure behavior.
2. Pure campaign-map admission: count, path budget, then lexical file byte and
   cumulative-byte checks before `check_paths`. Next retain case collisions,
   required files, and lexical reads. Each read performs structural-budget
   preflight, then existing syntax/duplicates/numbers→envelope→migration→typed
   checks. Later phases remain layout→engine→index→coverage→digest. No campaign
   semantic findings are emitted for failed structural admission.
3. Within lexical preflight, first encountered syntax or resource violation
   wins; do not scan beyond invalid syntax to seek another error. At a single
   opening token, depth precedes node count; before an element, collection count
   precedes child decode; decoded string/key cap is checked during token scan.
   Duplicate/noninteger checks still precede schema selection in admitted data.
4. `lock` retains campaign acquire+decode→assets acquire+decode→catalog
   acquire+decode→resolution→serialization→alias/output checks. Catalog limit
   failure is distinguishable from the existing `invalid catalog` typed error.
5. Replay acquisition/predecode budgets precede decoding and then the existing
   version→metadata→ticks→input count→schedule checks. For caller-created Replay,
   retain these cheap existing checks, then bound payloads before playback or
   serialization. Same-tick ordering, descriptive identity, and apply errors
   remain unchanged. Golden acquisition still happens only after playback.
6. Golden bounded acquisition precedes whole-file UTF-8 validation, then
   line-budget preflight, then existing first-mismatch comparison. Retain
   `str::lines` treatment of CRLF/final unterminated lines. A resource error
   can beat an earlier mismatch; within bounds UTF-8 still beats comparison.

PROPOSED typed limit errors with resource name, limit, observed lower bound
(normally limit+1), and logical path/pointer when available; never include huge
tokens or native absolute paths in errors. Data would gain a limit error and
data-owned diagnostic conversion; CLI acquisition uses its portable `io` shape
with a proposed `limit_exceeded` kind. Replay can extend `TooLarge` resource
names; golden needs a distinguishable resource error, not fake `Io(NotFound)`
or fabricated mismatch hashes. Exact public variants, Display strings and ADR
approval belong in D1/D3 specifications; this draft does not declare them landed.
CLI keeps exit 1 for resource/write failures, 2 for usage, existing stream shapes.

## 4. PROPOSED single-file publication (D4)

PROPOSED safe Rust using existing `std` and CLI `same-file` only, with no assumed
new dependency, unsafe exception, or hidden platform API authorization:

1. Complete collection/load/serialization/key-set/alias preflight. Empty update
   plan returns without creating temp files, journals, directories or write
   handles. `fmt --check` remains read-only. `lock` retains its input/output
   alias checks even when desired output might compare equal.
2. For each differing target in lexical order, create a uniquely named sibling
   staging file via `create_new`. Use process-local counter/name attempts with
   a fixed retry ceiling (PROPOSED 128), not an unbounded collision loop; never
   truncate a collision. A sibling is on the target's filesystem; EXDEV is a
   hard error, never a copy/delete fallback. Existing parent directories only.
3. Write complete admitted bytes, apply approved permission policy, `sync_all`,
   and close before publication (important on Windows). Pre-publication failure
   leaves original bytes intact. Clean only artifacts this operation owns;
   cleanup failure must not hide the primary failure or delete unrelated files.
   PROPOSED define standalone-stage ownership and orphan reporting in D4;
   process termination can leave a sibling stage even though the target is intact.
   D5 must record/reserve its stage name and ownership in transaction evidence
   before creation, reconcile a surviving stage before creating another, and
   never reclaim an unrelated same-named file based on a name alone.
4. Immediately recheck root including its lexical ancestors, target ancestors,
   observed link/type status, filesystem identities and bounded original bytes.
   Retain `source_changed` on drift and existing nonregular errors. Comparing
   size/mtime alone is insufficient. Never intentionally follow an observed
   symlink. This is best-effort change detection, not compare-and-swap.
5. Existing regular target: use same-filesystem `std::fs::rename(stage,target)`
   as the candidate replacement primitive. Never delete/rename-away the old
   target first, truncate it, or fall back after a failed replace. Validate the
   native primitive/filesystem behavior before calling this an atomic guarantee.
6. Missing `campaign.lock`: ordinary rename would overwrite a concurrent creator
   on Unix. PROPOSED publish via `std::fs::hard_link(stage,target)` (no clobber),
   then unlink the owned stage name. AlreadyExists maps to `source_changed`;
   a filesystem without hard-link support fails closed. This intentionally
   changes the old direct `create_new` output implementation, not its no-clobber
   guarantee. Do not use a zero-byte destination reservation as publication.

### Platform/durability envelope — PROPOSED promises, not verified claims

| Situation | Proposed claim and limitation |
|---|---|
| Linux local filesystem, same-filesystem rename of regular files | Candidate old-or-new namespace visibility; an already open reader can retain old inode. Sync staged file before rename and sync parent directory after publication where supported. File sync alone does not persist the directory entry. |
| Windows/MSVC local filesystem | `std::fs::rename` supports replacing an existing file, but implementation/OS/filesystem and sharing modes matter. Open handles, antivirus and permissions may reject replacement; close own handles and report failure with old target retained under the tested primitive contract. Do not infer ReplaceFile-style metadata or power-loss semantics merely from using rename. |
| Portable durability | Safe std provides no uniform directory-sync guarantee across Windows/Linux filesystems. Unix directory open+sync can be attempted with safe std; Windows needs an explicitly reviewed supported route before equivalent durability is promised. Network/removable/custom filesystems are not assumed to share local guarantees. |
| Failure after successful publication, e.g. parent sync or stage-name cleanup | New complete target may already be visible. Report published-but-durability/cleanup-incomplete; do not claim original unchanged or attempt blind rollback. Distinguish this from replacement failure. |
| Power loss / machine crash | Not approved as loss-proof on both targets. D4 can promise tested prepublication preservation and process-interruption behavior only; stronger durability remains a blocker requiring a separate primitive/dependency decision if demanded. |

PROPOSED preserve ordinary permissions where safe std supports them, fail before
publication on preservation failure, and refuse replacement of read-only targets
rather than bypassing prior protections via directory permissions. Inode/file ID,
timestamps, hard-link topology, Windows ACLs/alternate streams and Unix extended
metadata are not automatically preserved by replacement. Human approval must
define whether unsupported metadata preservation blocks authoring writes.

Existing hard-link checks remain required even though replacement avoids writing
through an old inode: reject lock aliases of all three inputs and migrate/fmt
aliases of every observed regular campaign-tree path, including ignored content.
These checks do not discover arbitrary hard links outside the tree. PROPOSED
external aliases retain old bytes when a target is replaced; no claim of global
alias discovery. Path rechecks and `same-file` do not prevent hostile concurrent
ancestor substitution, reparse-point races, or edits after the last check.
PROPOSED support a cooperative, quiescent authoring tree; secure hostile-tree
operations require separately approved handle-relative/no-follow primitives.

## 5. PROPOSED multi-file migration recovery (D5)

Per-file replacement is not an atomic campaign transaction: readers can observe
a mixed prefix. PROPOSED a recoverable roll-forward protocol for `migrate` and
the same multi-file `fmt` writer, after D4. No automatic rollback or all-files
visibility promise. Recovery uses recorded bytes, never reruns migration logic
that could change with an engine upgrade.

- Reserve one root-local transaction directory (PROPOSED `.crpg-write-txn`)
  with exclusive creation after nonempty preflight. An existing directory is
  recovery-required, never evidence it is safe to delete. No PID-age guessing.
  Explicit recovery command syntax and ordinary-command refusal are D5 contract
  decisions; no flags become supported by this draft.
- Before touching targets, write immutable old-byte snapshots of all recognized
  documents and new-byte copies for updates, plus a strict versioned manifest:
  lexical logical targets, old/new lengths,
  artifact names, operation, and original recognized path inventory. Compare
  bytes exactly; CLI has no approved hashing dependency. Backups are copies,
  not hard links to originals. Manifest paths reject absolute/parent escapes,
  duplicates, case collisions, links and nonregular artifact types.
- PROPOSED manifest ≤8 MiB, ≤4,096 updates, depth ≤8; old/new artifacts obey
  document caps and each set totals ≤128 MiB. Complete retained transaction
   ≤264 MiB plus at most one 8 MiB sibling stage (≤272 MiB extra storage).
   This bound includes interrupted/recovered stages: recovery must reuse or
   safely remove its recorded surviving stage before allocating another. If
   ownership cannot be established, stop with a conflict rather than accumulating
   fresh stages. Stage reservation/publication/cleanup are journal state, not
   unrecorded transient work; their exact restart-safe form is D5 specification work.
  Refuse over-budget journal input before decode. Space checks are advisory;
  real write/sync errors still stop preparation without original mutation.
- Sync prepared files and manifest, then publish a complete READY marker only
  after preparation succeeds. Sync relevant directories where supported under
  §4. Without valid READY, targets must not have been touched by the protocol;
  incomplete preparation requires explicit checked discard, not silent cleanup.
- For each lexical target, compare current bounded bytes with recorded old and
  new. Old→stage recorded new and publish via D4; new→already applied; neither,
  missing, link, changed inventory or alias→stop with recovery conflict and
  preserve evidence. Recheck before each publication. Do not use a mutable
  “last completed index” as the sole truth: crash can occur between replacement
  and progress recording. Unchanged recognized files are rechecked against the
  recorded snapshot before resuming; ignored file identities are rescanned.
- After every target equals recorded new, publish a complete COMMITTED marker.
  Keep manifest/READY/COMMITTED until artifact cleanup finishes; remove final
  markers last under a specified restart-safe cleanup order. A committed
  transaction interrupted during cleanup requires no missing old backup to
  republish targets. Never discard the last recovery evidence before proving
  completion. D5 must specify/test every cleanup intermediate state.
- Repeated recovery converges to the same recorded new map or the same conflict;
  it never overwrites an author's third version. Recovery instructions tell
  authors to preserve the transaction and resolve a named conflict explicitly.
  Read-only CLI consumers refuse a live prepared transaction instead of loading
  a mixed campaign; they do not perform recovery as a side effect. External
  readers remain outside that cooperative exclusion boundary.

PROPOSED process-termination recovery on verified native local filesystems is
the initial guarantee. READY/COMMITTED/journal corruption or loss after power
failure must fail closed; recoverability is conditional on durable intact
artifacts. A journal is not a substitute for Windows directory durability.
Transaction directory naming, classifier interaction (today ignored directories
are traversed), complete cleanup protocol and explicit recovery UX are required
D5 specification work, not permission to silently change the collector now.

## 6. Existing guarantees and tests that constrain approval

Source-read tests, **not executed here**:

- Data `tests/loader.rs::{read_and_load_phase_precedence,
  migration_sits_inside_reads_with_fixed_precedence}` pins collision/required
  file order, lexical reads and migration positioning. `tests/migrations.rs`,
  `tests/migration_coverage.rs` and `tests/support/migration_gate.rs` own rollback,
  idempotence and independent migration oracles. Canonical duplicate rejection
  must survive bounded parsing; current schema/fixture bytes stay read-only.
- CLI `main.rs` collector tests include ignored-symlink behavior and ancestor
  rejection. Rewrite tests pin changed-source/no-open, observed symlinks,
  no-op/preflight zero writes, stable error operations and first-failure stops.
  `rewrite_first_lexical_failure_stops_and_keeps_prefix` explicitly asserts an
  earlier target remains written when the next write fails. The seam models
  truncate/write/sync, not atomic publication.
- CLI `tests/migrate.rs` compares private-copy results with the independently
  checked-in full migration map, preserves ignored files and silent second run.
  `tests/fmt.rs::fmt_refuses_a_document_hard_linked_to_ignored_content` and
  `tests/lock.rs::lock_refuses_hard_link_aliases_of_every_input` require identity
  checks and unchanged input bytes. `main.rs::lock_create_race_is_source_changed`
  preserves missing-output no-clobber behavior.
- Testkit `tests/replay.rs` pins byte-stable writes, same-tick order, trailing
  ticks, typed schedule/count errors, malformed UTF-8 and missing-file versus
  divergence. `tests/harness.rs` pins raw malformed line/real hash sides,
  header skipping, exact tick, short sequences and UTF-8 distinction.
  `tests/replay_golden.rs`, combat/srd replay suites and native goldens retain
  exact-build compile-time target selection. No rebaseline is part of D0–D5.

**Approval blocker:** D4/D5 supersede the explicitly documented partial-write
contract and may conflict with existing injected writer expectations and exact
messages. Root rules prohibit weakening/deleting tests to pass. First obtain a
separately scoped, explicit contract-change decision naming each affected test,
preserved intent and stronger replacement oracle. Do not reinterpret a green
legacy test as verification of atomicity or silently rewrite its expectation.

## 7. PROPOSED ordered agent handoffs and future tests

No implementation agent is authorized by this file. The independent D0 drafting
work was delegated; these are handoff boundaries for subsequent implementation
agents, not claimed independent runtime reviews. Use the sequence below for
delivery; D3's policy is independent of campaign limits, while D2 needs landed
D1 policy and D5 needs landed D4 primitives.

| Order | Sole implementation crate | Agent specification deliverable / boundary | Tests to specify later |
|---|---|---|---|
| D1 | `crpg-data` | Approved pure limits/error API and ADR where required; supplied-map admission, bounded strict decode, Value/migration and writer checks. Extend its existing architecture/AGENTS only as separately authorized. No filesystem or testkit import. | Every byte/count/depth/string boundary, escaped tokens, duplicates plus limits, multifault precedence, pre-clone refusal, intermediate expansion rollback, output expansion, positioned diagnostics and unchanged migration/schema oracles. |
| D2 | `crpg-cli` | Consume landed D1 policy; bounded campaign/catalog/recheck reads and enumeration. Catalog structural preflight is CLI-local, resolver semantics stay data-owned. No copied classifier or lower-crate fixes. | Growth/shrink during read, unknown lengths, many tiny/ignored files, bounded pre-sort listing, depth/path budget, Unicode/order/errors, catalog duplicates/limits, zero write on rejection across every collector consumer. |
| D3 | `crpg-testkit` | Approved replay/golden public error changes and predecode limits using existing serde/std. Keep payload opaque; no production data dependency for shared caps and no CLI changes. | Largest-valid/first-invalid on-disk and in-memory cases, oversized ignored fields, escaped string scratch bounds, input/payload aggregation, floats preserved, allocation/read seams, UTF-8 versus mismatch precedence, all native goldens unchanged. |
| D4 | `crpg-cli` | Approved contract supersession, staged per-file replace/no-clobber publish, source/alias rechecks and precise publication/durability errors. No transaction claim. | Stage creation collision/exhaustion, disk-full/short write/sync/replace failure, read-only/metadata failures, open Windows reader, real rename and hard-link publication on both natives, no-op zero artifact, old-or-new process kill points, source/link/alias races at seams. |
| D5 | `crpg-cli` | Specify versioned recovery format, namespace, UX, cooperative exclusion, cleanup state machine; consume D4, never reproduce migrations. | Kill/restart at every preparation/publication/marker/cleanup boundary; repeated recovery, mixed prefixes, edited/missing targets, corrupt/oversized journal, missing artifact, renamed root, symlink/alias substitution, disk/sync errors and deterministic conflict. |

Each child first gets an approved task file with allowed paths, public/error
shapes, literal focused commands and `cargo test -p <owner> --locked` stopping
command, plus all applicable root/crate/CI gates on native Windows/MSVC and
genuine Linux/GNU. Future suite names and tests above are not existing gates.
Boundary tests should assert refusal before allocation/insertion/publication,
not merely that an oversized already-allocated object eventually errors.
Native filesystem tests supplement fault injection; no runtime skips to green.

Dependency policy is already binding: data→core only; CLI's broad ALLOWED set
does not authorize dependencies; testkit's broad table entry does not override
its narrower approved manifests or one-way integration-consumer rule. No lower
layer may import testkit to share limits. CLI currently has `same-file`; data
has serde/serde_json/blake3/schemars/semver; testkit has serde/serde_json and sim
normally, with data/rules only dev edges. No new crate, edge, `unsafe`, contracts,
toolchain, lint or golden modification is approved here. If safe existing
primitives cannot meet the chosen guarantee, stop and request an explicit
dependency/platform decision; do not route unsafe through Godot for headless I/O.

## 8. Exact human approval questions / blockers

1. **Approve the numerical tables and counting definitions in §2 as the initial
   supported input envelope, subject to fixture/content inventory before each
   child contract?** If not, which resource ceiling and value should change?
2. **Approve resource admission before content semantics and the precise phase
   ordering in §3, including oversized+malformed and golden-limit+mismatch cases?**
   Approve separately the D1/D3 public error ADRs; no error shape is accepted yet.
3. **Approve safe-std same-filesystem rename for existing files and hard-link
   no-clobber publication for absent locks, failing closed on unsupported filesystems,
   with no delete-first fallback?** Native proof remains required, not assumed.
4. **Is tested process-interruption recovery sufficient initially, with power-loss
   durability explicitly unpromised on Windows, or must D4 block until an approved
   safe platform/dependency solution establishes stronger durability?**
5. **Approve the cooperative quiescent-tree threat model, byte+identity rechecks,
   external-hard-link old-byte behavior and metadata limitations in §4?** Must
   ACL/extended metadata preservation instead be a prerequisite to replacement?
6. **Authorize a separate contract-change task for legacy partial-write tests and
   new publication-state diagnostics before D4/D5 implementation?** Which exact
   affected assertions and documentation are approved to be superseded?
7. **Approve deterministic roll-forward with immutable old/new copies, a reserved
   transaction directory, explicit recovery, and refusal by cooperative readers
   during unfinished multi-file work?** D5 still needs an approved command/format
   and cleanup-state contract; this is not atomic whole-campaign visibility.
8. **Approve the ordered single-crate D1–D5 handoffs without new dependencies or
   E017/network scope?** Each later agent must stop on an owning-crate API gap
   rather than editing two crates or changing replay payload meaning.

Until these decisions and child contracts are approved, the only deliverable is
this draft. Code gaps remain open; no implementation, gate pass, approval or merge
can be inferred from its existence.

## Attribution

- 2026-09-28 (UTC) · opencode/gpt-6-astra + D0 proposed strategy · Inspected the merged data, CLI and testkit boundaries and existing test guarantees to propose bounded acquisition and recoverable authoring writes. Kept numerical policy, filesystem guarantees and single-crate handoffs explicitly subject to human approval so planning cannot be mistaken for implemented or verified contracts.
- 2026-09-28 (UTC) · opencode/gpt-6-astra + D0 handoff review · Clarified that D0 drafting was delegated independently of E017 and that the handoff table does not claim runtime review or implementation approval. Preserved the original attribution and proposed-only status.
- 2026-09-28 (UTC) · opencode/gpt-6-astra + D0 delegated consistency review · Required emitted replay bytes to satisfy the reader's payload-span limits and included surviving sibling stages in recovery ownership and storage accounting. These close planning inconsistencies without changing payload semantics or claiming filesystem behavior has been verified.
- 2026-09-28 (UTC) · opencode/gpt-6-astra + D0 durability review · Split authoring-write delivery into process-recovery now versus verified power-loss durability as an explicit release blocker, with versioned limits and deterministic roll-forward recovery, so provisional safety cannot become the permanent guarantee.
- 2026-09-28 (UTC) · opencode/gpt-6-astra + approval receipt · Recorded user approval of D0 §9 direction with recommended defaults; D1 and D3 are ready for independent specification agents, D2/D4/D5 serialize per §7/§9 with no implementation authorized here.

## 9. Durability review for robustness / longevity / flexibility (2026-09-28, PROPOSED)

**Status remains PROPOSED and unapproved. This section answers the
robustness-first challenge: process-interruption recovery alone is not a
durable authoring guarantee.** It keeps staged delivery but makes the full path
explicit so a provisional D4 is never mistaken for the finished guarantee.
No implementation, dependency, or contract change is authorized here.

### 9.1 Revised target: staged, with power-loss durability as an explicit release blocker

- **Stage D4a (provisional, shippable behind review):** staged same-filesystem
  replacement, no-clobber publication for absent outputs, pre-publication
  preservation, tested process-kill recovery at every staging/publication/
  cleanup boundary. Power-loss durability explicitly unpromised.
- **Stage D4b (required before authoring writes are called durable):**
  verified directory-durability and file-durability behavior on supported
  Windows/MSVC and Linux/GNU local filesystems, precise published-vs-durable
  error taxonomy, and a safe-primitive/dependency decision where std is
  insufficient. Network, removable, and custom filesystems remain out of the
  promised envelope unless separately verified.
- **Release rule:** user-facing claims must say process-recovery or
  power-loss-durable explicitly, never crash-safe ambiguously. D4a cannot be
  advertised as D4b.

This split is the only honest way to keep robustness-first without blocking
all bounded-input work on platform research. If the user requires power-loss
durability before any new writer ships, then D4a and D4b serialize and D2/D5
wait.

### 9.2 What D4b must establish (each a specified single-crate investigation or contract)

1. Native primitive behavior for `rename` replacement and hard-link no-clobber
   publication on both targets, including open-handle, antivirus, permission,
   and sharing-mode failures on Windows; inode/directory-entry semantics on
   Linux. Report old-target-retained versus published-but-not-durable versus
   durability-unknown distinctly.
2. Directory-sync route: Unix directory open+sync where supported; an
   explicitly reviewed supported Windows route before equivalent durability is
   promised. Where no safe route exists, stop and request a dependency/platform
   decision rather than inferring durability from file sync alone.
3. Journal/manifest/marker durability for D5: READY/COMMITTED recoverability is
   conditional on intact artifacts. Corrupt or missing journal after power loss
   fails closed; it never silently resumes or discards evidence.
4. Metadata policy decision: ordinary permission preservation plus refusal of
   read-only targets is the D4a default. Whether unsupported ACLs, alternate
   streams, extended attributes, timestamps, or hard-link topology block
   publication is a human decision recorded before D4b, with fail-closed
   behavior where preservation is required.
5. Threat-model decision: cooperative quiescent tree for D4a/D5, with byte and
   filesystem-identity rechecks. Hostile-tree protection via handle-relative /
   no-follow primitives is a separate explicitly approved task, not an implied
   property of rechecks.

### 9.3 Longevity and flexibility corrections to §§2–5

- Limits are a versioned policy envelope, not constants scattered through
  call sites. Raising any ceiling requires fixture/content inventory and
  boundary tests; lowering is safe. Byte limits never imply peak-memory
  guarantees; child specifications must measure typed clones, indexes,
  diagnostics, and hash vectors.
- Transaction and manifest formats are versioned. Recovery never reruns
  migration logic under a newer engine; it publishes recorded bytes. Repeated
  recovery converges or reports the same conflict; it never overwrites an
  author's third version.
- Replay writer/reader limits are the same envelope: emitted pretty bytes must
  satisfy reader payload-span and aggregate limits before target mutation.
  Payload meaning stays opaque.
- Storage accounting includes surviving sibling stages and transaction
  artifacts. Stage ownership is journal state; an unowned same-named file is
  never reclaimed by name alone.

### 9.4 Ordered evolution handoffs (single-crate, after approval)

| Order | Sole crate | Boundary |
|---|---|---|
| D1 | `crpg-data` | Versioned limit/error policy, bounded decode/migration/writer checks. |
| D3 | `crpg-testkit` | Bounded replay/golden acquisition/decoding on the same envelope. |
| D2 | `crpg-cli` | Bounded acquisition consuming landed D1 policy. |
| D4a | `crpg-cli` | Staged replacement + process-recovery, explicit non-durability wording. |
| D4b | `crpg-cli` | Verified durability envelope + primitive/dependency decision. Blocks durability claims. |
| D5 | `crpg-cli` | Versioned roll-forward recovery consuming D4a/D4b, explicit UX and cleanup state machine. |

D1/D3 can be specified in parallel; D2 needs D1; D5 needs D4a and the D4b
decision. Each child gets exact public/error shapes, literal focused commands,
`cargo test -p <owner> --locked` stopping command, and native Windows/MSVC plus
genuine Linux/GNU evidence. Legacy partial-write tests are superseded only by
the separately approved contract-change task in §6.

### 9.5 Revised D0 approval questions on durability

- [ ] Approve D4a now with D4b as an explicit release blocker, or serialize
  D4b before any new writer ships?
- [ ] Which metadata failures block publication versus warn-and-continue?
- [ ] Is the cooperative-tree model sufficient until hostile-tree work is
  separately approved, or must that work precede D4a?
