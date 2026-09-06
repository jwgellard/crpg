# ADR-0009: determinism scope — replay, not lockstep

Date: 2026-09-06
Status: **Accepted**

## Context

Spec §2.4 requires replay determinism — same binary, same inputs, same
result — and explicitly does not require cross-platform lockstep, "because
the server is authoritative," adding that "[t]his … should be stated in an
ADR." No ADR stated it. Meanwhile `state_hash` goldens (spec §16.1), the
T008a acceptance tests, and the T009 replay harness all need to know what a
hash divergence *means*: without a recorded scope, a cross-platform mismatch
reads as a bug rather than out-of-scope, and the hash exclusion set
("excluding presentation-only and non-deterministic fields," spec §16.1) has
no authority behind it.

## Decision

1. **The promise is replay determinism over an exact build.** Same binary +
   same inputs ⇒ same hash sequence, tick for tick. "Same binary" means the
   pinned toolchain (`rust-toolchain.toml`), the same cargo profile, the
   same feature set, and the same target platform. Debug-vs-release,
   cross-toolchain, and cross-platform reproduction are not owed. A toolchain
   bump re-blesses goldens deliberately, as a reviewed event.
2. **Cross-platform and cross-build sameness are explicitly not promised.**
   Integer/rules math is bit-stable everywhere by construction (lints enforce
   it); `f32` basic ops and `serde_json` float formatting are in practice
   stable across CI's platforms today — but platform `libm`
   transcendentals (movement, LOS, AI, all future work) are allowed their
   last-ulp differences, and the day a system calls one, cross-platform hash
   equality ends. That ending is this ADR working, not failing.
3. **Golden files carry their scope in the filename** (toolchain, platform,
   profile) and CI compares goldens on canonical Linux (`ubuntu-latest`)
   only; every platform still runs the full unit and property suites.
   Shared-everywhere golden comparison is rejected: it holds until the
   first `libm` call and then becomes a flaky-gate crisis.
4. **The hash exclusion list is governed, starting empty.** T008a ships it
   empty — tick and queue bytes provably hashed. An exclusion requires a
   test proving the excluded field cannot affect behaviour. Category (a)
   presentation-only caches pass on test + review; category (b) anything
   non-deterministic admitted to `World`, or any change to this rule, is an
   ADR-level change.
5. **Lockstep is unnecessary because the server is authoritative.** Only the
   server simulates; clients replicate read-only and predict own movement
   (E015). The cross-machine check is the desync test — replica visible
   state matches the server's filtered projection — a semantic comparison,
   never hash equality.
6. **Tightening later is allowed, not owed.** Cross-platform goldens (e.g.
   player-shared cross-OS replays) would need their own ADR plus the float
   audit that earns it: every transcendental call site, every serialization
   path. Deferral is not prohibition — but neither is it permission to "just
   try it."

## Consequences

- A hash divergence across platforms, profiles, or toolchains is
  out-of-scope by citation, not a bug to chase.
- T008a's seed-sensitivity and hash-identity tests are read against the
  exact-build scope; T009's replay goldens inherit the scoping convention.
- The first `f32::sin` (or sibling) in sim code does not trigger a
  determinism review — this ADR already settled what it means.
- Toolchain upgrades come with a golden re-bless step. That step is expected
  and reviewable, not a regression.

## Rejected

- **Same-source-any-build** (any profile/toolchain reproduces goldens):
  fragile by construction — Rust integer overflow panics in debug and wraps
  in release, so it would force overflow-auditing the sim to earn a promise
  nobody asked for.
- **Shared-everywhere goldens**: simpler until the first transcendental,
  then a crisis. The filename convention costs one line now.
- **Promising cross-platform equality "for now"** while the world is
  float-light: a promise that expires silently the day movement lands is
  worse than no promise.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark + E011 · Filed per maintainer sign-off on the recommended scope: exact-build replay promise, explicit cross-platform non-promise, filename-scoped goldens compared on canonical Linux, governed exclusion list, server-authoritative justification.
