# Project state

Updated: 2026-09-06

## Phase
Phase 1 — core skeleton and test harness.

## Branch state
All merged work is on local `master`. The working tree is clean. The Done
history below is merged work only.

## Done
- T004 workspace, 15 stub crates, CI green on Linux and Windows
- T005 dependency-direction lint
- T005b determinism lint (bans HashMap/HashSet iteration, wall-clock,
  threads, and external RNG in `crpg-rules`/`crpg-sim`, plus floats in
  `crpg-rules`; `// determinism-ok: <reason>` escape hatch). Wired into CI
  as `lint-determinism`.
- T005c `deny.toml` (licences, advisories, bans, sources) plus a `cargo deny`
  job in CI (pinned `EmbarkStudios/cargo-deny-action@v2.1.1`, cargo-deny
  0.20.2), and CI narrowed to push-on-master + pull-request so a PR branch no
  longer runs every job twice.
- T006a `crpg-core`: `CoreError`, `EntityId`, `GenerationalArena<T>`, plus the
  crate's `Cargo.toml`, module layout and `AGENTS.md`. Arena semantics are
  ADR-0006 Decision 1: generations start at 1, lowest-index slot reuse,
  ascending-index iteration as a documented invariant, and a slot whose
  generation would overflow is retired rather than wrapped (the exact
  boundary was corrected by review 2 below). The free list is
  serialized so a loaded arena allocates the ids the saved one would have, and
  deserialization rejects an arena whose slots and free list disagree. 23
  tests pass: 4 property tests (id-reuse safety, arena invariants,
  iteration order, serde round trip including next-allocation), 18 unit tests
  and a doctest. (Now 26: the boundary fixes below added three.)
- Review follow-up 1 (whole-project review, 2026-09-04). Closed the
  gaps it found, none of which any gate was catching:
  - The arena's deserialization guard accepted a *retired* slot (generation
    `u32::MAX`) that was on the free list, and would then issue an id at
    `u32::MAX` from it. Now `CoreError::CorruptArena(RETIRED_BUT_FREE)`.
  - `deps.py` read only `[dependencies]`. It now checks `[dev-dependencies]`
    and `[build-dependencies]` too (a dev-dep is still an import, and ADR-0006
    has just made dev-deps a live concept), fails closed on a crate missing
    from the allowed-edges table, and enforces the unsafe rule directly by
    requiring `#![forbid(unsafe_code)]` on every crate root but `crpg-godot`'s.
    That check immediately found `crpg-cli/src/main.rs` missing it.
  - `determinism.py` now covers `crpg-core` as well. `crpg-sim` stays exempt
    from the float ban on purpose — spec §2.4 puts positions in `f32` — and
    README/AGENTS.md now state what is actually enforced rather than the
    vaguer "rules paths".
  - The two lint self-test files used incompatible conventions, so
    `unittest discover` silently ran 5 of 11 tests and reported OK, and CI ran
    neither. Both are `unittest` now, 22 tests, with a `lint-selftest` CI job.
  - `deny.toml`: dropped the unused `MPL-2.0` allowance (weak copyleft
    pre-approved for nothing) and silenced the unused-allowance warnings.
  - Stripped the UTF-8 BOM from 14 tracked files; both lints now read
    `utf-8-sig` so a reintroduced BOM cannot mask a line-1 violation.
- Review follow-up 2 (second whole-project review, 2026-09-04).
  Same shape as the first — every finding was a **partial enumeration**, a
  check that knew about some of its cases and not the rest:
  - **The arena's exhaustion boundary was off by one, and the runtime and the
    loader disagreed about it.** `remove` bumped a slot at `u32::MAX - 1` to
    `u32::MAX` *and* returned it to the free list, so a live arena could reach
    the exact state its own `TryFrom` rejects as `RETIRED_BUT_FREE` — it
    serialized to a save it could not load — and would then issue an id at
    `u32::MAX`. `u32::MAX` is now a reserved tombstone that is never issued: a
    slot retires on *reaching* it. `remove` and `clear` share one
    `retire_or_free` so they cannot drift, and the guard gained a fourth defect
    (`OCCUPIED_AT_RETIRED`) now that an occupied slot at the tombstone is also
    impossible. The old test forced a slot to `u32::MAX` and removed, which
    tests a state no arena reaches; the boundary case one below it was the
    missing test, and is now two. Within ADR-0006 Decision 1 ("retired
    permanently rather than wrapped"), which this pins the exact edge of rather
    than reverses.
  - **`deps.py` read three dependency tables and trusted table keys as crate
    names.** A `[target.'cfg(windows)'.dependencies]` block was invisible, and
    so was any renamed dependency (`x = { package = "godot" }`). Both together
    put a godot dependency and an upward `crpg-core -> crpg-sim` edge in one
    manifest with the lint green. It now walks every `[target.*]` block and
    resolves `package` over the key, and names the offending table in the
    violation. Also: `src/bin/*.rs` are crate roots for the unsafe check, and
    `crpg-testkit` may no longer depend on `crpg-godot` (every crate dev-depends
    on testkit).
  - **`determinism.py` treated `///` as prose.** Doctests are compiled and run,
    and `crpg-core/AGENTS.md` says the bans hold "anywhere, including tests" —
    so a doctest using `HashMap` or `f64` passed. Fence bodies are scanned now
    (`text`/`ignore` fences excepted), `/* */` block comments no longer produce
    false positives, and an unterminated one is reported rather than silently
    swallowing the rest of a file. That change immediately exposed a second
    hole: `\bf(?:32|64)\b` never matched a suffixed literal like `1.5f64`,
    because there is no word boundary after a digit.
  - `EntityId` and `Slot` are now closed shapes (`deny_unknown_fields`), and an
    `EntityId` deserializes only at a generation an arena issues —
    `CoreError::InvalidEntityId`. That is a well-formedness check, not an
    authority check; authority belongs to whoever knows the sender.
  - CI: `--locked` on clippy and test (the committed lockfile was not the
    tested one), a weekly schedule so a new advisory does not wait for a push,
    `permissions`, `concurrency`, `timeout-minutes`, and a pinned Python 3.11
    for the lint jobs (`tomllib`).
  - `LICENSE-MIT` and `LICENSE-APACHE` added — the workspace has declared
    `MIT OR Apache-2.0` since T004 with neither text shipped. `ADR-0001.md`
    renamed to `0001-godot-pinned-not-forked.md`, matching every other ADR.
  - Root `AGENTS.md` was weaker than CI: it asked for `clippy -p <crate>`
    without `--all-targets`, and never mentioned the lints or their self-tests.
  - Lint self-tests: 22 -> 49.
- T006b `crpg-core`: `Fx16_16`, a 16-fractional-bit integer fixed point.
  Saturating (never panicking or wrapping) `+ - * /` plus checked and
  saturating variants, floor division for either divisor sign, `floor`/`ceil`/
  `round` (round halves away from zero) and `abs` with `MIN.abs() == MAX`.
  `Display` prints the shortest exact decimal; `FromStr` rejects inexact or
  out-of-range input rather than rounding; serde is the raw `i32`. 19 new tests
  (10 property tests) prove arithmetic against i64 oracles, so no floats
  anywhere; 45 core tests pass in debug and release. Review found only doc
  gaps (rounding-mode wording, Display/FromStr impl docs), fixed before
  landing.
- T006c `crpg-core`: `DeterministicRng` owns lazily-created named PCG32-XSH-RR
  streams in canonical `BTreeMap` order. Stream parameters derive only from the
  master seed and length-separated name bytes through SplitMix64, so first-use
  order and draws from other streams cannot shift a sequence. Range generation
  is rejection-sampled, inclusive signed ranges cover the full `i32` domain,
  and serde resumes every stream exactly. Twelve new tests pin the 16-value
  golden vector, independence/order properties, range bounds and distribution,
  and serde continuation. All required local gates pass: 57 core tests
  including the doctest, both lints, and all 49 lint self-tests.
- T006d `crpg-core`: `Tick(u64)` and `RoundCount(u32)` provide explicit
  saturating/checked simulation-time arithmetic with transparent integer serde
  and no seconds conversion. `Ulid(u128)` masks caller-supplied 48-bit timestamp
  and 80-bit randomness fields, uses canonical uppercase Crockford base32 for
  display and string serde, accepts lowercase plus `I`/`L`/`O` aliases, and
  reports length, character and overflow parse failures separately. Seventeen
  tests, including six property tests, brought core to 74 tests including its
  doctest. All required local gates passed; no proptest regression file was
  produced.
- T006e `crpg-core`: `Interner` assigns dense `u32` handles in first-intern
  order and serializes as an ordered string list. `Interners` owns distinct stat
  and tag namespaces whose private-field `StatId`/`TagId` handles are
  runtime-only and deliberately do not implement serde or `Display`. A full
  review made equality order-sensitive and made deserialization reject duplicate
  strings, then hardened RNG deserialization against invalid stream parameters
  and added a direct rejection-sampling regression. Nine interner tests and two
  RNG regressions bring core to 85 tests including its doctest.
- T001 GDExtension rendering spike — go (ADR-0003), 200 chars @ 231.7 fps,
  FFI cost 87.4 µs/frame, on the RTX 4060 laptop. Spike lives in
  `C:\CRPG\Dev\spike-gdext`, not this workspace.
- T003 Lua sandbox spike — go (ADR-0005). All 10 escape-attempt fixtures
  blocked, instruction budget aborts an infinite loop, memory ceiling
  blocks unbounded allocation, `pairs`/`math.random` substitutions are
  reproducible under a seed and change with a different one. Found that
  `mlua`'s `StdLib` bitset does not gate the base library — `load`,
  `loadfile`, `dofile` are loaded regardless and must be stripped from
  globals by hand; recorded so `crpg-script` doesn't rediscover it the
  hard way. Spike lives in `C:\CRPG\Dev\spike-lua-sandbox`, not this
  workspace.
- T002/T002b QUIC movement spike — go (ADR-0004, updated 2026-09-04).
  Prediction/reconciliation validated (small, non-compounding corrections
  even under ~30% effective loss). NAT leg closed: a human-run two-machine
  test (server behind a normal home router, no port forward/DMZ; client on
  a separate network) failed to connect, as spec §7.7 anticipated. Local
  Windows Firewall and router UPnP capability were both confirmed present
  and ruled out as the cause; the failure is the NAT layer having no port
  mapping, which is exactly what §7.7 already deferred handling for.
  Known, scoped future fix: give the real server a UPnP/IGD client (`igd`
  crate) or document manual port-forwarding for operators. Still flagged
  as a risk, unresolved: a real quinn 0.11.11 defect
  (quinn-rs/quinn#2710: 129-packet dedup window silently discards
  reordered-but-delivered datagrams) that inflates effective loss well
  past the shim's configured rate under realistic jitter — needs a
  decision before crpg-net's snapshot channel depends on raw datagrams.
  Spike lives in `C:\CRPG\Dev\spike-quic`, not this workspace.

## Next
- T007 crpg-sim: World and ComponentStore. Its entity arena is
  `GenerationalArena<EntityMeta>` from T006a, already property-tested — not a
  second implementation.

## Task backlog
`tasks/BACKLOG.md` is the index of every numbered task with its status, plus
the carried blockers and the throughput log.

## Decisions
- ADR-0001 Godot consumed as a pinned dependency, not forked
- ADR-0002 Rust below the presentation layer
- ADR-0003 T1 spike go/no-go: go
- ADR-0004 T2 spike go/no-go: go — local reconciliation validated, NAT
  leg closed (fails without manual port forward/UPnP request, as
  expected per §7.7; known future fix is a UPnP/IGD client or documented
  manual forwarding), quinn dedup-window defect still flagged as a risk
  to resolve before crpg-net depends on raw QUIC datagrams
- ADR-0005 T3 spike go/no-go: go — mlua sandbox holds against all 10
  scripted escape attempts; base-library `load`/`loadfile`/`dofile` are
  not gated by `StdLib` and must be stripped explicitly, carry that
  forward into crpg-script
- ADR-0006 crpg-core primitive semantics — **Accepted** on 2026-09-04:
  generational arena in core, `Fx16_16` saturating/floor, PCG32 sub-streams in
  a `BTreeMap`, interned ids runtime-only (persist the string). Authorises
  `proptest` + `serde_json` as workspace dev-dependencies.
- ADR-0007 reserves `u32::MAX` as the arena's never-issued retirement
  tombstone, superseding ADR-0006 Decision 1's original overflow boundary.
- E006-A (2026-09-06): `f64` banned in `crpg-sim`; the determinism lint
  enforces it as `no-f64`, `f32`-spatial-only per spec §2.4.
- E009/E014/E015 (2026-09-06): spec carries the ADR-0008 residue fixes, the
  skeleton-only `World` serde caveat (conversion pair owned by T014), and the
  replica/`Timeline` ownership (prediction buffer outside sim, container in
  T007, advance rules in T008). T007 scope is locked.
- Godot pinned at 4.7.2
- Toolchain pinned at rustc 1.98.0

## Open questions
- Whether to carry a patch for quinn-proto's dedup window (quinn#2710) or
  wait/track upstream, once crpg-net design starts (see ADR-0004). Not on the
  critical path until after T018 — the in-memory transport comes first.

## Known problems
- Scaffolding from workflow plan §15 still missing, none of it blocking:
  `tools/preflight.ps1`, `docs/adr/0000-template.md`, per-crate `AGENTS.md`
  for every crate except `crpg-core` (written in T006a).
- Spec §14's `docs/contracts/` and `docs/guides/` still do not exist. Neither
  has a gate depending on it: contracts matter once `crpg-contracts` holds
  traits, guides once there is a campaign format to author against.
   (`docs/architecture/` was the one with a gate — §15.6 — and now exists.)

---

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark + hygiene + T007-unblock · Folded T006e into Done (it landed in 8f2e38b; "complete in working tree" was stale) and recorded E006-A/E009/E014/E015 so T007 can be specified.
