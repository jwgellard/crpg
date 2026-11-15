# T150-draft-a — lane-1 movement transport (`crpg-net-quic`, then `crpg-server`)

**Status: blind draft (nothing approved).** Written by the architect from the
T150 fact sheet (`tasks/T150-facts.md`), the approved T026p lane-1 items and
Out-of-scope (`tasks/T026p.md:967–1026`, `:1464–1476`), the approved T037
contract's Interface and Out-of-scope (`tasks/T037.md:1351–1355`), ADR-0036,
ADR-0024, ADR-0025, ADR-0026 and `tasks/DECISIONS-2026-09-30.md:83–88`. A
Claude architect writes the final contract; the user approves it. Nothing
here is approved.

Citation convention (T140's): a bare `:N` or `:N–M` is a line of the file
cited earlier in the same list item or sentence. Every citation was checked
against the branch `task/T150-draft-a-d1` (base `c4d32b9`, the fact sheet).

**`<base>`** (used in "Definition of done") is the commit a part's branch is
cut from: a descendant of `master` that holds T037d's code and the fact
sheet, plus the previous part's commits.

## Task

<!-- TODO -->

## Parts

<!-- TODO -->

## Readings

<!-- TODO -->

## Interface

<!-- TODO -->

## Behaviour

<!-- TODO -->

## Errors

<!-- TODO -->

## Tests

<!-- TODO -->

## What must not change

<!-- TODO -->

## Dependency edges

<!-- TODO -->

## Out of scope

<!-- TODO -->

## Open questions

<!-- TODO -->

## Agent log

- 2026-11-15 (UTC) · opencode-go/deepseek-v4-pro + T150 draft a · Drafted the lane-1 movement transport contract (the `crpg-net-quic` stream pair, DATAGRAM extension, D23 dedup patch and impairment probe, then the `crpg-server` `quic.rs` v4 route) from the fact sheet and the T026p/T037 contracts, so the architect can review and finalize it.
