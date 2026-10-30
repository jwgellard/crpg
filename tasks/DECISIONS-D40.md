# Decision D40

Status: **Answered 2026-10-30 (the user, session 25)**: D40a (a), D40b (a), D40c **(b)**, D40d (a). Also approved: a lockfile-only `cargo update -p zstd-sys` (to >=2.1.2) for the new advisory that fails `cargo deny`. The T139 contract, revised to
D39a and through one Claude `reviewer` pass (seven findings, all resolved;
`tasks/T139.md` "Review 1 resolutions"), raises three questions only the
user can answer. Its new `crpg-data` package T139d (S-set) and the review's
one design call also want explicit approval. T139 implementation waits on
these; T024h1 and T025a1 are building meanwhile.

## D40a — three existing tests under the survivor default

Three tests replay an authored fight twice and expect fresh entities, which
the new default (survivors persist) breaks: `history_schedule.rs:387`
(step 11), `combat_persistence.rs:481`
(`encounter_release_full_lifecycle`), `src/history/tests.rs:674` (cap case
c). AGENTS.md forbids weakening a test without the user.

- **(a) Recommended:** add a one-line `respawns = Some(true)` marker to each
  fixture; no assertion changes.
- (b) Rewrite the three tests to expect persisted survivors.

## D40b — despawn and the record

- **(a) Recommended:** `despawn` removes the placement's record, so a
  despawned placement spawns fresh at its next start.
- (b) A separate "defeated" list on `World` that survives despawn (about
  +120 lines).

## D40c — tags outside an encounter

- **(a) Recommended:** tags are read from the prefab, not stored on the
  record.
- (b) Store tags on the record; this moves the six `crpg-testkit` goldens and
  the CLI replay fixtures.

## D40d — T139d and `PlacementInUse`

- **(a) Recommended:** approve T139d (`crpg-data`, S-set): optional
  `Ruleset.creature_persistence`, `late_join`, `effects_on_exit` and
  `Encounter.respawns` at the current versions (D32c), two regenerated
  schemas, four `crpg-data` test literals gaining the new fields and the
  `schema_drift.rs` optional-keys list, following T136a. Also approve the
  review's rule that a transfer into an area already holding a record or
  combatant for that placement is refused with a new
  `AreaError::PlacementInUse`.
- (b) Hold T139d for changes.

## Consequences of D40c (b)

T139 is revised before implementation: every combatant carries its tags, the record stores them, and the six `crpg-testkit` goldens and the `crpg-cli` replay fixtures built from `combat_basic`/`combat_srd` are approved for re-baselining alongside D39a's four. The revised contract needs no second user gate unless the revision raises a new question.

## D40e — zstd advisories: upgrade the pinned tree

Asked and answered 2026-10-30 (the user, session 26): **upgrade**. The approved
`zstd-sys` bump cleared only RUSTSEC-2026-0410, which is wasm32-only. Three
more advisories from the same upstream fix (zstd-rs PR #384) landed in the
same database update: RUSTSEC-2026-0391 (`zstd`, `with_context` constructors),
0398 (`zstd-safe`, `seekable` feature) and 0403 (`zstd-safe`, legacy frames
through `write::Decoder`). None is reachable from `crpg-persist`, which calls
only `bulk::Compressor::new` and `stream::read::Decoder::with_buffer` with
default features off. `cargo deny` matches on version alone, so every gate
failed anyway. The pin in `crates/crpg-persist/Cargo.toml` moves from
`=0.14.0` to `=0.14.3`, and the lockfile from `zstd-safe` 8.0.0 to 8.2.0 and
from `zstd-sys` 2.1.0 to 2.1.2. No new crates, identical dependency tables,
and the same bundled zstd 1.5.7 C sources (byte-identical `zstd/lib`). The
rejected alternative was ignoring the four IDs in `deny.toml`.

## Follow-up approvals (2026-10-30, session 27)
- **D40c test edits approved:** the five `crates/crpg-sim/tests/surface_tags.rs` / `surface_draws_contract.rs:492` assertions that pinned T137's no-grant-no-tags rule now expect the prefab's tags, with four doc comments (T139a2 owns them; recorded in `tasks/T139.md`).
- **T139d pinned-key edit approved:** `crates/crpg-data/tests/rt_fields_contract.rs:389` gains `creature_persistence`, `effects_on_exit` and `late_join` in the pinned ruleset key list; the T139 contract's L1 had missed this file.

## Agent log
- 2026-10-30 (UTC) · claude-code/opus-5.5 (manager) + T139 review closed · Drafted the four user questions the D39a revision and its review raised, each with the architect's recommendation, so T139 implementation can start on one answer.
- 2026-10-30 (UTC) · claude-code/opus-5.5 (manager) + D40 answered · Recorded the user's answers: test markers, despawn removing the record and T139d approved as recommended, tags stored on the record against the recommendation (with the extra goldens approved), and the zstd-sys lockfile bump approved.
- 2026-10-30 (UTC) · claude-code/opus-5.5 (manager) + D40e · Recorded the user's choice to upgrade the pinned zstd tree after the zstd-sys bump proved to clear only a wasm32-only advisory, with the reachability findings that informed it.
- 2026-10-30 (UTC) · claude-code/opus-5.5 (manager) + session 27 approvals · Recorded the user's two follow-up test-edit approvals (D40c assertions, T139d pinned schema keys) so the reviewed exceptions to the no-test-edit rule are on file.
