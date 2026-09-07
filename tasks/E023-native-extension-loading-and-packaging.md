## Task
Decide T0 native-extension loading and packaging across Windows/MSVC and
Linux/GNU. **Status: open.** Human-decision task; no implementation selected.

## Context

ADR-0012/T009c makes `x86_64-pc-windows-msvc` the primary product target and
`x86_64-unknown-linux-gnu` a fully supported headless server, tooling,
server-side extensibility, and test target. T1 campaign data and sandboxed
Lua/ruleset content remain portable. T0 native extensions require distinct
target-specific artifacts; this is not a cross-platform binary or stable ABI
promise. Linux headless support must not depend on Godot, and Linux GUI
client/editor support is not promised.

## Decisions Required

- Define artifact identity, target/toolchain compatibility, version negotiation,
  packaging/discovery, and rejection diagnostics for incompatible extensions
  on Windows/MSVC and Linux/GNU. Do not assume a stable Rust or native ABI.
- Decide ABI and loading boundaries, lifecycle/unloading, ownership/threading,
  error handling, trust and distribution responsibilities before implementation.
- Reconcile the proposed boundary with the existing rule that only
  `crpg-godot` may use `unsafe`, while preserving Godot-free Linux headless
  support. Identify where loading/FFI obligations would live and require an
  explicit governance decision for any necessary policy change; this task
  does not grant an unsafe exception or choose a mechanism.
- Coordinate with E012/E022's shared authoritative host package/API decision
  for Windows embedded, Windows dedicated, and Linux dedicated adapters. An
  extension must not create a second authoritative implementation or bypass
  the client/server authority boundary.
- Assign future per-crate implementation tasks and native compatibility,
  loading/failure, and packaging acceptance tests on both supported targets.
  Activate real checks with capabilities under E020, never placeholder jobs.

## Deliverable

A human-approved decision record and aligned platform/package/API obligations
in the owning tasks, including explicit resolution of unsafe governance before
any loader implementation. This decision blocks native-extension loading and
packaging implementation, not T009c's documentation or replay-golden work.

## Constraints

Planning only: no loader, dependency, unsafe exception, stable ABI claim,
packaging implementation, or product CI job is introduced here or by T009c.
Do not choose the shared host's crate placement; E012/E022 own it. Preserve
portable core/rules/sim boundaries and independent exact-build replay scopes;
no tolerance, runtime golden selection, or cross-platform equality promise.

## Agent log

- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c · Filed the open native-extension decision for target artifacts, ABI/loading, and packaging across Windows/MSVC and Linux/GNU. Made unsafe governance and Godot-free headless support explicit prerequisites without choosing or implementing a loader.
