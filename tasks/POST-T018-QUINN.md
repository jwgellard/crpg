# quinn dedup-window — decision before N-QUIC specification

**D04 selected: carry a project-maintained patch.** See
[current evidence and preparation gate](POST-T018-DECISIONS.md#d04--quinn-carry-a-patch-with-an-auditable-preparation-gate).
The below before-choice hold is historical and superseded. N-QUIC specification
may proceed; source/patch/dependency audit still blocks integration.

ADR-0004 records the 129-packet window issue against quinn 0.11.11 /
quinn-proto 0.11.15 and [quinn-rs/quinn#2710](https://github.com/quinn-rs/quinn/issues/2710).
It reports a 2048-bit private-fork fix and a jitter reproduction with a raw
UDP control; it does not prove the spike's packet-number arithmetic or today's
upstream status. No upstream fix was verified in this planning session.

## Options and recommendation

1. **Recommend evaluating carry-a-patch.** Human selects an auditable exact
   source revision, patch diff, update/security maintenance owner, dedup memory
   cost and reproducible reorder test. Reproduce before/after and include the
   zero-actual-loss control. A wider window is a candidate, not an approved
   magic number. Assess native builds, license, source/ban/advisory policy and
   transitive build requirements under the unchanged deny policy.
2. **Wait on upstream.** No fork burden; task stays blocked until an exact
   released revision contains a verified fix and passes the same regression.
   “Issue closed” is not evidence of a merged remedy.

Reliable-unordered streams/lower snapshot rate were ADR evaluation ideas;
neither is an automatic substitute for this explicit carry-versus-wait call.
Changing channel delivery classes would need separate approval.

## Blocking effect / required output

**This decision blocks writing executable N-QUIC, not merely running it.**
[T023](T023.md) is therefore a specification hold. After the call, write exact
endpoint/channel API, wire stream framing, MTU/backpressure/close semantics,
runtime integration and error precedence; obtain a T018a-style dependency
approval for quinn and every necessary runtime/TLS edge. Do not widen deny.toml.
Use lane-0 reliable ordered combat; never apply movement highest-seq-wins to
combat. Versioned movement/snapshot semantics remain separately reviewed.

Acceptance planning must include ADR-0004's 150 ms RTT, ±15 ms jitter, 3%
loss and zero-loss reorder control, bounded queues, duplicate detection,
peer close and genuine Windows/Linux loopback plus real-host integration.
No source or patch is selected here.

## Agent log

- 2026-09-28 (UTC) · opencode/gpt-6-astra + quinn decision brief · Preserved the carried before-specification blocker and distinguished the recorded spike evidence from an unverified current upstream state. Assigned explicit patch provenance and maintenance decisions before any QUIC dependency is approved.
- 2026-09-28 (UTC) · opencode/gpt-6-astra + delegated quinn resolution · Checked issue #2710 and current upstream main, which still retains the u128 window, then selected an audited project patch strategy. Kept the no-git-source/no-deny-widening policy and exact dependency/patch evidence as technical prerequisites rather than claiming a patch already passed.
