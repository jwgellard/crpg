# ADR-0023: Scoped cargo-deny skip for the windows-sys 0.52 duplicate

Date: 2026-09-30 (UTC)
Status: **Accepted** — approved by the user on 2026-09-30 ("Approved.") as
decision D22 in [tasks/DECISIONS-2026-09-30.md](../../tasks/DECISIONS-2026-09-30.md).
Applied by T023 when quinn is added, not before.

## Context

T030 audited the QUIC dependency set proposed for T023 (`quinn` 0.11.12,
`quinn-proto` 0.11.19, `tokio` 1.53.1, `rustls` 0.23.45) under the unchanged
`deny.toml`. advisories, licenses and sources pass; `bans` fails because
`multiple-versions = "deny"` sees two `windows-sys` versions: 0.52.0 through
`ring` 0.17.14 (the newest `ring`, which requires `^0.52` and is pulled by
`quinn-proto` with either crypto provider) and 0.61.2 through `tokio`, `mio`,
`socket2` and `quinn-udp`. Evidence: `docs/reviews/T030-quinn/results/`.

## Decision

When T023 adds the QUIC dependencies, add exactly one entry to
`[bans] skip` in `deny.toml`: `"windows-sys@0.52.0"`, with a comment naming
the `ring` cause and this ADR. No other policy field changes. T023's
`cargo deny check` must pass with only this addition. Remove the entry when a
`ring` release depends on `windows-sys` 0.61 (cargo-deny's unmatched-skip
warning will show when it stops matching).

## Alternatives

- Wait for `ring` to move to 0.61: no date; blocks the critical path.
- Pin older `tokio`/`mio`/`socket2` that still use 0.52: forgoes current fixes
  in the runtime the server will depend on.
- Switch to the aws-lc-rs provider: does not help; `quinn-proto` 0.11.19 still
  pulls `ring`.

## Consequences

Windows builds compile two versions of Microsoft's raw API-bindings crate
(MIT OR Apache-2.0), costing build time and some binary size. Unlike the
ADR-0006 skips, this one is in a shipped runtime graph, which is why it is an
ADR and not a one-line edit. Linux is unaffected (target-specific dependency).

## Addendum — 2026-10-04 (UTC): applied by T023

Dated facts recorded when T023 applied this decision. The Decision section
above is not edited (E007).

1. **Placement.** T023 applies the skip for the QUIC dependencies of the new
   `crpg-net-quic` crate ([ADR-0024](0024-separate-crpg-net-quic-crate.md)),
   not `crpg-net`. The skip entry is exactly `"windows-sys@0.52.0"`, with a
   comment naming `ring` 0.17.14 and this ADR.
2. **`rand` / `rand_core` (T023 Q1, approved by the user 2026-10-04).**
   `deny.toml` gains two more skips, `"rand@0.9.5"` and `"rand_core@0.9.5"`.
   - Cause: quinn-proto ≥ 0.11.16 requires `rand ^0.10.1` (not optional),
     so `rand` 0.10.3 / `rand_core` 0.10.1 are in the runtime graph. The
     newest proptest (1.11.0, dev-only under ADR-0006) still requires
     `rand ^0.9`.
   - The skips pin the dev-only proptest side, so the shipped graph keeps
     exactly one `rand`.
   - The T030 audit missed this because it checked the product edges
     without the workspace's dev graph. T023's measured audit
     (`tasks/T023.md` E§12, R6) found it.
3. **`getrandom` (T023 Q2, approved by the user 2026-10-04).**
   - `ring` 0.17.14 uses `getrandom` 0.2.17, and `rand` 0.10.3 (through
     quinn-proto) uses `getrandom` 0.4.3. Both are compiled into
     `crpg-net-quic` on both x86_64 targets. No aligned versions exist, so
     this runtime duplicate is accepted.
   - It passes through the existing ADR-0006 skip `"getrandom@0.4.3"`. That
     skip's comment said dev-only; T023 corrected the comment to name the
     runtime path and this addendum (comment only; no policy change).
4. **Windows cost (T023 revised question R1, confirmed by the user
   2026-10-04).**
   - `windows-sys` 0.52 is reached only through `ring`'s
     `cfg(all(target_arch = "aarch64", target_os = "windows"))` dependency,
     so it is not compiled for `x86_64-pc-windows-msvc`. The Consequences
     section's build-time and binary-size cost therefore applies to aarch64
     Windows only.
   - The skip is still required because cargo-deny evaluates every target.
     No `targets` filter is added to `deny.toml`.

## Agent log

- 2026-09-30 (UTC) · claude-code + D22 · Recorded the user-approved scoped skip that unblocks the audited QUIC graph, with its removal condition, instead of widening the duplicate policy.
- 2026-10-04 (UTC) · claude-code + T023 · Appended the dated addendum recording where the skip landed (`crpg-net-quic`), the approved `rand`/`rand_core` dev-side skips, the accepted runtime `getrandom` duplicate with its comment fix, and the aarch64-only Windows cost, without editing the decision text.
