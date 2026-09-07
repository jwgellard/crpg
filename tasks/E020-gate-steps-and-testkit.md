## Task
Close the integration-gate gap and give the testkit boundary an owner.
**Decided 2026-09-06: capability-gated sequencing.**

## Original problem

- The §15.4 gate lists 13 steps (`docs/CRPG_ENGINE_SPEC.md:1090-1105`); CI
  implements 6 (fmt, clippy, test, deny, two lints, self-tests —
  `.github/workflows/ci.yml:32-97`). Steps 7–13 (schema drift, `crpgc
  validate`, golden replay, save/load equivalence, perf gate, client+editor+
  server build, smoke test) have no jobs, and original T09's done-when required
  step 9 before its activation path was assigned. The workflow plan's two-layer CI, merge
  queue, and self-hosted runner (`docs/AGENTIC_WORKFLOW_PLAN.md:264-272`)
  are likewise absent.
- E005's proposed sim→testkit→sim combined dependency cycle lacked an ownership
  rule. Its original Cargo-rejection premise was incorrect; E005 now records
  the architectural reason for rejecting the upward dev edge. Related:
  T08/T09/T16/T18 touch 2–3 crates
  each under a one-task-one-crate rule (see E004), and `deny.toml:79` still
  says `yanked="warn"` against S001's `deny`.

## Decision
- Do not create green placeholder jobs. Each gate becomes mandatory when its
  owning capability exists.
- T009 splits per E004: T009a is testkit-only replay semantics and a checked-in
  golden comparison; T009b is the later CLI wrapper.
- T009a's canonical-Linux comparison runs under the existing Linux workspace
  test job and makes step 9 live. Portable replay tests still run on Windows;
  runtime platform selection and tolerant comparison remain forbidden.
- Steps 7 and 8 activate with T010 and T011's eventual per-crate split. Step 10
  activates with persistence, step 11 with E019, step 12 as product binaries
  become real, and step 13 after an integrated server/client fixture exists.
- E005 owns the one-way testkit rule. E004 owns per-crate splits. S001 retains
  the separate `yanked = "deny"` security decision; none blocks T009a.

## Consequences
- A green gate always means a real check ran; unavailable capabilities remain
  visible in the backlog rather than as skipped CI jobs.
- No workflow edit is needed before T009a: the Linux workspace-test job is the
  enforcement point once the Linux-only golden test exists.
- T009a is now unblocked for Stage 2 specification.

## Constraints
- Planning-only; no workflow or source changes in this decision.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark · Filed as part of the spec-gap triage: a 13-step gate with 6 steps built, plus an ownerless dependency cycle.
- 2026-09-06 (UTC) · opencode/gpt-5.6-sol + maintainer decision · Chose capability-gated activation, assigned the real Linux golden comparison to T009a in testkit, and kept unavailable gates out of CI until they can enforce behavior.

## T009c Supersession

The historical Linux-only step 9 assignment above is superseded by
ADR-0012/T009c. Windows/MSVC is primary; Linux/GNU remains fully supported for
headless server, tooling, extensibility, and testing. Step 9 requires two
independently generated target-scoped baselines under Rust 1.98.0, normal test
profile, and default features. Real comparisons are selected at compile time:
`cfg(all(target_os = "windows", target_arch = "x86_64", target_env = "msvc", debug_assertions))`
and
`cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu", debug_assertions))`.
Other targets/profiles run portable parse/play repeatability and shape checks
without either golden. No runtime selection, tolerance, or cross-platform
equality; each missing supported baseline fails with `Io(NotFound)`.
ADR-0009's exact-build promise and hash-exclusion governance remain intact;
only Decision 3's Linux-only selection is superseded.

The existing Windows/Ubuntu workspace-test matrix is the enforcement point;
no placeholder workflow is needed. Required native Windows/MSVC and genuine
WSL Ubuntu Linux/GNU gates passed; see the [T009c completion record](T009c.md).
T009a and T009c are merged on `master` as of 2026-09-07, and T009b is next.
The
genuine original T009a Linux record remains separate historical provenance.

Capability-gated activation remains in force. Step 12 must eventually require
real Windows client/editor/server and Linux headless server artifacts. Step
13 must activate real Windows embedded-server and dedicated-server smoke
tests plus a Linux dedicated-server smoke test when those capabilities exist.
E012/E022 assign one reusable authoritative implementation across all three
hosts without choosing its package/API placement here. Linux headless support
must not depend on Godot; Linux GUI client/editor builds are not promised.
E023 owns the future native-extension loading/packaging decision, not a loader
in T009c.

## Agent log (T009c append)

- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c · Appended a supersession of the Linux-only step 9 assignment without rewriting the historical decision or logs. Retained the no-placeholder rule and recorded future platform-specific product gates and pending native verification.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c verification alignment · Updated only the active supersession status to reflect reported native gate passes and link the completion record. Preserved the original decision and all prior attributions; review/merge remains outstanding and T009b waits for landing.
