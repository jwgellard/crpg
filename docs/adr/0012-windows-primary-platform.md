# ADR-0012: Windows-primary platform and independent replay baselines

Date: 2026-09-07 (maintainer decision: 2026-09-06)
Status: **Accepted**

## Context

Windows is the development and product host, including the authoritative
runtime embedded for single-player. Treating dedicated servers as Linux-only
and giving Windows only portable replay shape/repeatability coverage leaves
the primary runtime without a real behavioural regression gate. Authority
and hosting platform are separate concerns: Linux remains a fully supported
headless target, not a best-effort alternative.

## Decision

1. **Windows/MSVC is primary** for development, product, release gating, and
   behavioural baselines. Its product surface is client, editor, embedded
   single-player server, dedicated server, and CLI.
2. **Linux/GNU is fully supported** for dedicated server, headless CLI/tooling,
   server-side extensibility, and CI/testing. Platform-specific failures are
   defects, not best effort. Linux is not the primary desktop presentation
   target; Linux GUI client/editor builds are not promised, and Linux
   headless support must not depend on Godot.
3. **One platform-neutral authoritative server implementation** is hosted
   in-process for Windows single-player and by dedicated processes on Windows
   and Linux. Platform-specific process, filesystem, service, and presentation
   concerns stay above `crpg-core`, `crpg-rules`, and `crpg-sim`; no OS-specific
   branches enter those crates. This grants no dependency permission.
4. **The client/server authority boundary is unchanged in-process.** Transport
   may be in-memory, but clients still cannot mutate authoritative state
   directly. Embedded hosting is not a separate single-player simulation.
5. **Replay determinism remains exact-build, not cross-platform lockstep.**
   Windows and Linux each compare an independently generated target-scoped
   golden under the pinned toolchain, normal test profile, and default
   feature set. Cross-platform hash equality is not required.
6. **Windows owns the primary behavioural baseline; Linux owns a supported
   server regression baseline.** Neither baseline is compared to the other.
   A divergence from a target's own baseline fails that target's gate.
7. **Current gate targets are `x86_64-pc-windows-msvc` and
   `x86_64-unknown-linux-gnu`.** Adding another target requires an explicit
   support decision and its own build/test scope.
8. **T1 campaign data and sandboxed Lua/ruleset content remain portable.**
   T0 native extensions require target-specific artifacts. Their loading and
   packaging mechanism remains a separately decided implementation concern,
   not a stable ABI promise.

## Supersession

This ADR supersedes **only ADR-0009 Decision 3's canonical-Linux-only
selection**. [ADR-0009](0009-determinism-scope.md) remains immutable. Its
exact-build promise, cross-platform non-promise, filename scoping, hash
exclusion governance, and server-authoritative rationale remain in force.

## Replay acceptance

T009c must use the portable `crates/crpg-testkit/fixtures/replay_basic.replay`
with independently generated goldens:

```
crates/crpg-testkit/goldens/replay_basic_rust-1.98.0_x86_64-pc-windows-msvc_test-default.golden
crates/crpg-testkit/goldens/replay_basic_rust-1.98.0_x86_64-unknown-linux-gnu_test-default.golden
```

The filenames encode pinned Rust 1.98.0, complete target triple, normal test
profile, and default features. Changing any scope element is a reviewed
re-baseline event. Real `play_and_verify` comparisons must be selected at
compile time with
`cfg(all(target_os = "windows", target_arch = "x86_64", target_env = "msvc", debug_assertions))`
and
`cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu", debug_assertions))`.
Other targets/profiles retain portable parse/play repeatability and shape
checks without reading either scoped golden. No runtime OS selection, fuzzy
comparison, tolerance, or missing-baseline skip is permitted; an absent
supported-target baseline fails with `Io(NotFound)`.

Generate on each genuine native target through the production
`read_replay` -> `play_replay` -> `write_golden` path. Never copy Linux hashes
to bless Windows, even if the sequences happen to match. Preserve T009a's
genuine Ubuntu 24.04/Rust 1.98.0 provenance; its matching Linux golden may be
renamed, but must be verified again after the changes, not described as
regenerated unless it was. Record native commands, `rustc -vV` target,
results, test counts, and golden SHA-256 for both targets. Replay/golden
artifacts must use canonical LF endings in checkout, index, and worktree,
with no BOM; see the [T009c completion record](../../tasks/T009c.md).

## Future obligations

- E012/E022 must assign one reusable authoritative host to Windows embedded,
  Windows dedicated, and Linux dedicated adapters. Those tasks decide whether
  reusable code is a library target in `crpg-server` or belongs in another
  existing crate; this ADR does not choose its API/package placement.
- The open, planning-only
  [E023](../../tasks/E023-native-extension-loading-and-packaging.md) tracks
  T0 native loading and packaging on
  Windows/MSVC and Linux/GNU. It must reconcile target artifacts and ABI/loading
  with the rule that only `crpg-godot` may use `unsafe`. No loader, dependency,
  unsafe exception, or stable ABI is authorized here.
- Future product gates must require real Windows client/editor/server and
  Linux headless server artifacts, with Windows embedded-server and dedicated-
  server smoke tests and a Linux dedicated-server smoke test when available.
  E020's no-placeholder rule remains in force; do not implement absent product
  build jobs in T009c. Linux server performance is a future supported-target
  concern; Windows remains primary, with no new performance policy here.

## Alternatives rejected

- Linux-only goldens: leave the primary Windows authoritative runtime ungated.
- Shared cross-platform hashes: promise lockstep that the architecture does
  not require and platform floating-point/libm behaviour does not warrant.
- Separate single-player simulation or best-effort Linux support: weaken the
  shared authority boundary or the explicit supported-server commitment.
- Cross-compilation as provenance: does not replace generation and execution
  in the genuine target environment.

## Implementation status

Acceptance records policy, not merge approval. All required T009c gates
passed on native Windows/MSVC and genuine Linux/GNU in WSL Ubuntu 24.04;
see the [completion record](../../tasks/T009c.md). T009a's replay and
T009c's native golden gates are merged on `master` as of 2026-09-07;
T009a's replay, T009c's native golden gates, and T009b's thin `crpgc replay`
wrapper are merged on `master` as of 2026-09-07; T010 campaign data is next. Future shared-host,
extension-loading/packaging, and product-gate obligations remain tracked by
E012/E022/E023, not implemented by replay-gate verification.

## Agent log

- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c documentation alignment · Recorded all eight accepted platform decisions, superseding only ADR-0009 Decision 3's canonical-Linux-only selection. Defined replay acceptance and future host/extension/product-gate obligations without claiming implementation, native verification, or merge.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c verification status · Updated implementation status from the reported passing native Windows and genuine WSL Ubuntu gates, linking the completion record and open planning-only E023 task without changing the accepted decisions. T009c awaits review/merge before T009b, T009a is also uncommitted, and final audit remains running.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c final audit · Recorded the reported final audit completion and completed working-tree implementation/verification, keeping review/merge outstanding and T009c ahead of T009b with T009a also uncommitted. Replaced documentation-pass scope wording with the timeless LF/no-BOM artifact requirement and completion-record link without changing the accepted decisions.
- 2026-09-07 (UTC) · opencode/big-pickle + T009a/T009c merged · Updated implementation status to the merged state (commit `bb9a702`) without changing the accepted decisions; T009b is next.
- 2026-09-07 (UTC) · opencode/big-pickle + T009b merged · Updated implementation status to the merged `crpgc replay` wrapper; the accepted platform decisions and host/extension/product obligations are unchanged.
