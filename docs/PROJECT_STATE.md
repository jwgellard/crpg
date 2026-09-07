# Project state

Updated: 2026-09-07

## Phase
Phase 1 — core skeleton and test harness.

## Branch state
All merged work is on local `master`. T009a's typed replay, T009c's
Windows-primary/Linux-supported correction, and T009b's thin `crpgc replay`
wrapper are merged as of 2026-09-07; the working tree is clean. The Done
history below is merged work only. T010 campaign data is next.

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
    `crpg-testkit` may no longer depend on `crpg-godot` (higher crates may
    eventually dev-depend on testkit, and that must not pull in the engine).
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
- T007 `crpg-sim` skeleton plus the ADR-0008 core substrate: `World` reuses
  `GenerationalArena<EntityMeta>` (spawn with explicit meta, total despawn
  with auto `Spawned`/`Despawned` events), `ComponentStore<T>`,
  `Timeline` container (`BTreeMap<(InitiativeKey, EntityId)>`,
  advance rules reserved for T008), `f32` `Transform`, and generic
  `EventEnvelope`/`EventQueue` in core. Seven sim tests including two
  10,000-case property tests (skeleton round-trip, replay determinism) and
  four substrate tests. Implementation findings now pinned in code and docs:
  stores/timeline serialize as pair lists (struct keys are not JSON keys),
  `EntityMeta` is braced (a unit serializes as `null`, i.e. a vacant slot).
- T008a measurement + loop in `crpg-sim`: BLAKE3 `state_hash` over canonical
  JSON (queue bytes in, exclusions none, NaN panics by design), fixed-step
  `tick` (counter then one-system list), `end_turn` pop primitive. Seven
  tick tests including the 10,000-tick hash-identity backbone plus seed
  sensitivity. New runtime deps `blake3` + promoted `serde_json`; `deny`
  clean.
- T008b harness in `crpg-testkit` (first testkit code): fixed
  script→tick→hash interleaving, line-hex goldens with scope headers,
  exact-tick `Mismatch` vs `Io` errors, hand-rolled hex (zero new deps).
  Four wiring tests including the 10,000-tick golden round trip. Sim
  untouched; direction followed the then-unratified E005 policy, now ratified.
- Review follow-up 3 (third whole-project review, 2026-09-06). Hardened the
  T007/T008a/T008b seams with no behavior change, in four commits:
  `crpg-sim` asserts `is_finite` up front in `state_hash` (`serde_json`
  emits `null` for NaN/infinity instead of failing), `Timeline::from`
  replaces while `Deserialize` rejects duplicate entities, and `World`
  loading rejects dangling ids; `crpg-core` `EventQueue` loading rejects
  duplicate `seq` and stale `next_seq`; `crpg-testkit` `Mismatch` is now a
  truthful enum (ADR-0010: only present sides, non-UTF-8 as content
  divergence); ADR-0011 clarifies the event payload bound as fields, not
  the enum, affirming `EventQueue<SimEvent>`. Sim now 24 tests (11 tick +
  13 world, incl. interleaved 10k ops), core event 7, testkit 6. All gates
  green each time.
- T009a replay in `crpg-testkit` (merged 2026-09-07): versioned
  `.replay` format (format 1: seed, campaign/engine identity, tick count,
  ordered inputs with opaque `serde_json::Value` payloads), validation
  before playback in fixed check order, input→tick→hash playback through
  public `World` APIs only, typed `ReplayError` reusing the ADR-0010
  `Mismatch` via a boxed `ReplayDivergence`, and `play_and_verify` as the
  end-to-end gate. New runtime deps `serde` + `serde_json` via the existing
  workspace dependencies (both already in the graph; `deny` clean, lockfile
  gains two edges, no new package). Original T009a fixture/golden pair:
  `fixtures/replay_basic.replay` (8 ticks, 5 inputs: a same-tick pair, a
  gap, trailing ticks) and
  `goldens/replay_basic_rust-1.98.0_linux-x86_64_debug.golden` (8 hashes,
  generated on genuine Ubuntu 24.04 / Rust 1.98.0 Linux by a deleted
  temporary example driving the production path). T009a verification had 19 tests
  (6 harness + 12 portable replay + 1 canonical-Linux gate doing the real
  comparison); the full suite passed on Windows and on that Linux
  environment under T009a's original policy. This is the historical T009a
  verification record, not verification of T009c's two target-scoped gates;
  ADR-0012 supersedes its Linux-only selection. Sim and CLI untouched.
- T009c platform correction in `crpg-testkit` (merged 2026-09-07):
  ADR-0012 makes `x86_64-pc-windows-msvc` the primary development, product,
  release-gate, and behavioural-baseline target while `x86_64-unknown-linux-gnu`
  stays fully supported for dedicated server, headless CLI/tooling,
  server-side extensibility, and tests. Each target compares against its own
  independently generated golden under pinned Rust 1.98.0 — exact-build
  determinism, never cross-platform lockstep. Goldens: Windows
  `replay_basic_rust-1.98.0_x86_64-pc-windows-msvc_test-default.golden`
  (570 B, sha `f2385639...21edb2`) and Linux
  `replay_basic_rust-1.98.0_x86_64-unknown-linux-gnu_test-default.golden`
  (568 B, sha `31998a38...68278d`); fixture `replay_basic.replay`
  (750 B, sha `2b65e7be...c8bd0`), all LF/no-BOM with `.gitattributes`
  pinning `*.replay`/`*.golden` to `eol=lf`. Verified on native
  Windows/MSVC and genuine WSL Ubuntu 24.04 Linux/GNU: 19 testkit tests
  (6 harness + 12 portable + 1 native golden), 135 workspace tests
  (134 unit/integration + 1 doctest), and 65 lint self-tests per target,
  plus the release-profile fallback test and withheld-golden failure proof
  on each. Final audit reran the gates and byte-audited LF/no-BOM. See the
  [T009c completion record](../tasks/T009c.md).
- T009b `crpgc replay` in `crpg-cli` (merged 2026-09-07): a thin
  `crpgc replay <path> [--golden <path>]` over `crpg_testkit::play_and_verify`
  with the clap-convention exit codes 0/1/2 (hand-rolled args for one
  subcommand; clap revisited at T013), a provisional reference-intents apply
  (`spawn_at`/`timeline`/`draw`/`despawn` over public `World` APIs,
  superseded when T014/T016 land), and no replay semantics in the crate. The
  task opened the crate: arch doc + crate `AGENTS.md` + architecture README
  row from "due with T013". 23 CLI tests (10 unit + 13 integration) pass on
  Windows/MSVC and genuine WSL Ubuntu 24.04 Linux/GNU, including real
  comparisons against each target's golden; workspace 158 and 65 lint
  self-tests pass per target. No new package in the lockfile; strictly
  verify-only (no rebless command).
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
- T010 campaign data in `crpg-data` is next — the replay harness, both native
  baselines, and the `crpgc replay` wrapper are all merged, so it is
  unblocked.

## Platform decision and verification
- [ADR-0012](adr/0012-windows-primary-platform.md) is Accepted, recording the
  maintainer's 2026-09-06 decision. `x86_64-pc-windows-msvc` is the primary
  development, product, release-gating, and behavioural-baseline target for
  client, editor, embedded single-player server, dedicated server, and CLI.
- `x86_64-unknown-linux-gnu` is fully supported for dedicated server, headless
  CLI/tooling, server-side extensibility, and CI/testing. A target-specific
  failure is a defect. Linux GUI client/editor builds are not promised;
  Linux headless support must not depend on Godot.
- One platform-neutral authoritative server implementation is hosted
  in-process for Windows single-player behind an in-memory transport and by
  dedicated processes on Windows and Linux. Clients never mutate authority
  directly. OS-specific concerns remain above core/rules/sim.
- Each target must reproduce its independently generated golden under pinned
  Rust 1.98.0, normal test profile, and default features. Windows owns the
  primary behavioural baseline, Linux the supported server regression
  baseline; neither is compared to the other. Exact-build determinism, not
  cross-platform lockstep, remains the promise.
- All required T009c gates passed on native Windows/MSVC and genuine
  Linux/GNU in WSL Ubuntu 24.04. Each native run passed 19 testkit tests
  (6 harness, 12 portable replay, 1 native golden), 135 workspace tests total
  (134 unit/integration + 1 doctest), and 65 lint self-tests. See the
  [T009c completion record](../tasks/T009c.md) for native commands and
  provenance. This is post-change verification, separate from the retained
  T009a Linux history above. The independent Linux re-verification run
  confirmed provenance by regenerating both build and goldens on genuine
  WSL Ubuntu 24.04 with zero tolerance.
- Final audit fixed code/documentation issues and reran the Windows/Linux
  gates with the totals above. The release-profile portable fallback test
  passed on each native target (1 test each). Byte audit confirmed LF and no
  BOM on replay/golden artifacts; a temporary `GIT_INDEX_FILE` audit
  confirmed `i/lf w/lf` without changing the user index. The scoped source
  diff was empty. See the
  [completion record](../tasks/T009c.md) for the pre-merge audit results;
  T009a and T009c are both merged on `master` as of 2026-09-07; T009b's
  `crpgc replay` wrapper is merged too (2026-09-07), and T010 is next.

## Future platform obligations
- [E012](../tasks/E012-binary-crate-naming.md) and
  [E022](../tasks/E022-server-editor-api-shapes.md) track assigning one reusable
  authoritative host to Windows embedded,
  Windows dedicated, and Linux dedicated adapters. They own whether it is a
  library target in `crpg-server` or code in another existing crate; no
  package/API choice is made here.
- The open, planning-only
  [E023](../tasks/E023-native-extension-loading-and-packaging.md) tracks T0 native
  extension loading/packaging on Windows/MSVC and Linux/GNU, reconciling
  target artifacts and ABI/loading with only `crpg-godot` permitting `unsafe`.
  No loader, dependency, unsafe exception, or stable ABI is authorized.
  T1 campaign data and sandboxed Lua/ruleset content remain portable.
- Future product CI must require Windows client/editor/server and Linux
  headless server artifacts, plus real Windows embedded-server and
  dedicated-server smoke tests and a Linux dedicated-server smoke test when
  those capabilities exist. E020 forbids unavailable placeholder jobs.
- E012/E022/E023 track these future host, extension, and capability-gated
  product obligations; no server hosting, extension loading, packaging, or
  product CI implementation is claimed by T009c verification.

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
- ADR-0009 (2026-09-06): determinism scope is replay-over-exact-build, not
  lockstep; goldens filename-scoped; hash exclusion list governed, starting
  empty. Only Decision 3's canonical-Linux-only selection is superseded by
  ADR-0012's independent Windows/MSVC and Linux/GNU comparisons.
- ADR-0010 (2026-09-06): testkit `Mismatch` is an enum carrying only present
  sides (no zeroed-hash sentinels); non-UTF-8 goldens are content
  divergence, not I/O failures.
- ADR-0011 (2026-09-06): supersedes ADR-0008 Decision 1's wording — the
  core-closed event payload bound applies to payload fields, not the
  vocabulary enum; `World.events: EventQueue<SimEvent>` affirmed as the
  sanctioned instance. Docs aligned, no code changes.
- E005 (2026-09-06): testkit is a one-way integration consumer. Core, data,
  rules, and sim do not depend on it even for tests; their cross-layer tests
  live in testkit. Higher crates may dev-depend only when all transitive edges
  remain legal and testkit does not depend back.
- E020 (2026-09-06): integration gates 7–13 activate only when their
  capabilities exist, never as skipped green placeholders. T009a's historical
  Linux comparison passed on genuine Rust 1.98.0 Linux, 2026-09-06. ADR-0012
now requires real target-scoped comparisons in both existing Windows and
  Linux workspace-test jobs; T009a and T009c are merged as of 2026-09-07.
  See the [completion record](../tasks/T009c.md).
- E016 (2026-09-06): campaign JSON distinguishes entity and named aggregate
  documents; object references use ULIDs while dependencies use immutable
  package ids; assets/campaign/package manifests have separate hash authority;
  event-IR waits count ticks. T010 owns data behavior, T013 the thin lock CLI.
- Godot pinned at 4.7.2
- Toolchain pinned at rustc 1.98.0

## Open questions
- Whether to carry a patch for quinn-proto's dedup window (quinn#2710) or
  wait/track upstream, once crpg-net design starts (see ADR-0004). Not on the
  critical path until after T018 — the in-memory transport comes first.

## Known problems
- Scaffolding from workflow plan §15 still missing, none of it blocking:
  `tools/preflight.ps1`, `docs/adr/0000-template.md`, per-crate `AGENTS.md`
  for every crate except `crpg-core` (T006a), `crpg-sim` (T007),
  `crpg-testkit` (T008b) and `crpg-cli` (T009b).
- Spec §14's `docs/contracts/` and `docs/guides/` still do not exist. Neither
  has a gate depending on it: contracts matter once `crpg-contracts` holds
  traits, guides once there is a campaign format to author against.
   (`docs/architecture/` was the one with a gate — §15.6 — and now exists.)

---

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark + hygiene + T007-unblock · Folded T006e into Done (it landed in 8f2e38b; "complete in working tree" was stale) and recorded E006-A/E009/E014/E015 so T007 can be specified.
- 2026-09-06 (UTC) · opencode/muse-spark + T007 merged · Recorded the skeleton above; next is T008 with the open E004 split question flagged.
- 2026-09-06 (UTC) · opencode/muse-spark + E004 decided (Option A) · T008 is now T008a (sim, specify first) + T008b (testkit); E011 acceptance criterion sits with T008a.
- 2026-09-06 (UTC) · opencode/muse-spark + E011 filed · ADR-0009 accepted; T008a's scope input is now ratified, leaving only the `blake3` choice for its task file.
- 2026-09-06 (UTC) · opencode/muse-spark + T008a merged · Recorded the loop and instrument above; next is T008b (specify first), then T009 with E005/E020.
- 2026-09-06 (UTC) · opencode/muse-spark + T008b merged · Recorded the harness above; next is T009 (specify after E005 + E020), then T010 (needs E016).
- 2026-09-06 (UTC) · opencode/muse-spark + review 3 follow-up · Recorded the sim/core/testkit hardening, ADR-0010/0011, and the AGENTS.md correction above; next is still T009 (blocked: E005 + E020).
- 2026-09-06 (UTC) · opencode/gpt-5.6-sol + E005/E016/E020 decisions · Recorded the testkit boundary, capability-gated CI sequence, and campaign-envelope policy; T009a is now the unblocked next task.
- 2026-09-06 (UTC) · opencode/muse-spark + T009a · Recorded the replay implementation (format, opaque payloads, typed divergence), the 19-test suite green on Windows and genuine Rust 1.98.0 Linux, and CI step 9 going live; next is T009b, then T010.
- 2026-09-06 (UTC) · opencode/gpt-5.6-sol + T009c platform correction plan · Made the Windows-primary/Linux-supported correction the next priority ahead of T009b; T009a remains unmerged until both target-scoped replay baselines and the superseding ADR are in place.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c documentation alignment · Recorded the accepted platform policy and future host, extension, and product-gate obligations while retaining T009a's genuine Linux verification history. T009a remains uncommitted and T009c in progress pending Windows and WSL Ubuntu verification; no completion or merge is claimed.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c verification status and audit · Recorded the reported passing native gates with 19 testkit tests, 144 workspace tests including one doctest, and 65 lint self-tests per environment, preserving T009a's historical provenance. Linked the completion record and tracked E012/E022/E023 obligations; T009c awaits review/merge with final audit running and T009a also uncommitted.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c count correction · Corrected the active workspace total from the revised native report to 135 (134 unit/integration + 1 doctest): core 91, sim 24, testkit 19, and core doctest 1. The earlier attribution's 144 is superseded, not rewritten; final audit remains running and review/merge is outstanding.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c final audit · Recorded the reported completed final audit, passing native reruns and release fallback tests, LF/no-BOM byte and temporary-index checks, and empty scoped source diff. Implementation/verification is complete in the working tree awaiting review/merge; T009a remains uncommitted and T009c retains priority before T009b.
- 2026-09-07 (UTC) · opencode/big-pickle + T009a/T009c merged · Recorded the merge of T009a's typed replay and T009c's Windows-primary/Linux-supported native golden policy onto `master` (commit `bb9a702`, pushed to `origin`). Next is T009b's thin `crpgc replay` wrapper, then T010.
- 2026-09-07 (UTC) · opencode/big-pickle + T009b merged · Recorded the `crpgc replay` wrapper (thin consumer of `play_and_verify`, exit codes 0/1/2, provisional reference apply, crate-opened docs) landed on `master` with both native gates green; next is T010 campaign data in `crpg-data`.
