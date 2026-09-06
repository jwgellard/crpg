# ADR-0010: testkit mismatch shape — truthful sides, not zeroed hashes

Date: 2026-09-06
Status: **Accepted**

## Context

`crpg-testkit::Mismatch` was `{ tick, expected: [u8; 32], actual: [u8; 32] }`.
Three `verify_golden` paths had only one side (or none): the run outliving
the golden, the golden outliving the run, and a golden line that is not 64
lowercase hex chars. All three reported `[0; 32]` for the missing side, so a
file-longer-than-run failure printed "expected 000…000, got 000…000" and
discarded the approved hash available on the next line. Readable-but-non-UTF-8
goldens were additionally misclassified as `HarnessError::Io`, directing
consumers toward regeneration rather than content investigation.

## Decision

1. **`Mismatch` is an enum with only present sides.** `Diverged` carries both
   hashes; `GoldenShort` carries only `actual`; `RunShort` carries only
   `expected`; `Malformed` carries the raw line plus `actual` when the run
   reaches that tick (`None` for a bad trailing line beyond the run);
   `InvalidUtf8` carries no tick. No variant fabricates a hash.
2. **Non-UTF-8 goldens are content divergence**, `Mismatch::InvalidUtf8`, not
   `Io`. `Io` stays for filesystem failures (missing/unreadable files).
3. **Length mismatches report at the shorter length**, unchanged; only the
   payload becomes truthful.
4. **Display stays single-pathed per variant** so existing call sites keep one
   match arm on `HarnessError` and format without string matching.

## Consequences

- Golden failures name the exact tick with the exact available evidence;
   file-longer-than-run reports the approved hash instead of zeros.
- The `{ tick, expected, actual }` struct shape is a breaking API change;
   `Mismatch::tick()` returns `Option<usize>` (`None` for `InvalidUtf8`).
- Malformed-line coverage becomes testable without sentinels.

## Rejected

- **Keep the struct with zeroed sentinels:** preserves source compatibility
   but keeps the meaningless zero-versus-zero report and makes
   `expected != actual` unreliable for a mismatch.
- **Separate `Corrupt` error variant for malformed/UTF-8:** splits content
   divergence across two variants and doubles call-site handling for what is
   one outcome ("the approved baseline cannot be compared").

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark + testkit mismatch hardening · Filed per maintainer approval of the typed-enum option: truthful missing/malformed sides, UTF-8 as content divergence, length rule unchanged.
