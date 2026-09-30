# E019 — measurable workloads and crpgc bench task shape

**D06 selected: versioned workloads and controlled per-target measurement first.**
See [decision record](POST-T018-DECISIONS.md#d06--performance-measure-before-enforcing-ceilings).
No unmeasured CI threshold or dependency is approved; alternatives below are history.
E019 records undefined active loads, absent AI budgets, bandwidth without
interest tier, and conflicting absolute/relative performance gates. The
1000-entity / 60 fps / no-LOD combination is contradicted by ADR-0003's
43.7 fps result on the measured RTX 4060 Laptop; do not certify it on paper.

## Recommendation / genuine choices

Recommend reproducible, versioned workloads first: seed/content identity,
entity and active-system counts, admitted input schedule, interest tier,
warmup/sample method, pinned profile/features/toolchain and hardware/OS.
Use per-target controlled-runner baselines; shared CI checks result schema and
workload correctness, not a flaky absolute millisecond assertion. Choose the
8 ms target and/or 20% regression rule explicitly after measurement.

Alternative: keep targets aspirational until a dedicated runner exists.
For rendering choose either a measured lower supported ceiling or a separate
bulk-scene-sync/animation-LOD implementation task; no unconditional 60 fps
promise from the existing polling bridge.

## Ordered single-crate shape after the call

1. Owning measured subsystem implements a callable headless workload/report
   API (or testkit harness if its legal downward edges suffice), one crate.
   Report must distinguish elapsed measurements from deterministic result
   checksums and carry target/workload provenance. Exact report/errors/caps
   and baseline storage are part of the approval, not supplied here.
2. `crpg-cli` gets a separate thin `crpgc bench` task consuming that API:
   workload selection, machine-readable report, invalid-workload failure and
   no hidden baseline regeneration. Approve CLI flags/exit codes first.
3. Runner/baseline policy is a tooling-only task; bridge LOD is a separate
   `crpg-godot` task. Activate gate 11 only with real measurements.

**Call:** workload matrix, fixture owner, report API home, hardware/native
baseline policy, threshold/confidence policy and renderer ceiling vs LOD.
ALLOWED does not authorize CLI→testkit or any other new edge automatically.

## Agent log

- 2026-09-28 (UTC) · opencode/gpt-6-astra + E019 decision brief · Split workload measurement from the CLI wrapper and runner policy, preserving single-crate execution. Kept unmeasured thresholds and the falsified renderer target out of acceptance claims.
- 2026-09-28 (UTC) · opencode/gpt-6-astra + delegated performance resolution · Selected subsystem workload APIs before the CLI wrapper, target-scoped controlled measurements and separately tasked bulk sync/LOD. Deferred numeric performance enforcement until measured baselines exist.
