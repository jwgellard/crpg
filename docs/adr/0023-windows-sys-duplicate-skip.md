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

## Agent log

- 2026-09-30 (UTC) · claude-code + D22 · Recorded the user-approved scoped skip that unblocks the audited QUIC graph, with its removal condition, instead of widening the duplicate policy.
