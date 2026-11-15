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

Carry lane 1 (movement) between a v4 host and its clients, as T026p left to
a new `crpg-net-quic` task and T037 froze out of `quic.rs`
(`tasks/T026p.md:1471–1476`, `tasks/T037.md:1351–1355`): lane 1's reliable
stream pair (intents upstream, receipts downstream), the QUIC DATAGRAM
extension for samples with a maximum datagram size of at least
`MAX_MOVE_DOWN_BYTES` refused at connect otherwise, D23's vendored dedup
patch, and the native Windows impairment probe re-run — all in
`crpg-net-quic` — and then the `crpg-server` `quic.rs` route that binds v4
sessions and moves `ingest_move`, `take_move_receipts` and `take_samples`
bytes. Today lane 0 is the only lane: one client-initiated bidi stream
carries hello, welcome and every lane-0 frame, and the DATAGRAM extension is
off (`crates/crpg-net-quic/src/tls.rs:86–91`, `:129–134`;
`docs/architecture/crpg-net-quic.md:141–147`). It is split into two
serialized parts, **T150a → T150b**, one crate each; T150a is S-set (a new
ALPN is a wire framing, and the dedup patch is a vendored dependency patch
plus the workspace `Cargo.toml`), T150b is not.

- **T150a — `crpg-net-quic` (S-set).** `Framing::V3` and ALPN
  `crpg-lane0/3`; the second client-initiated bidi stream for lane-1 intents
  and receipts; the DATAGRAM extension with the connect-time minimum and
  refusal; D23's `0002-widen-dedup-window` vendored patch; the native
  Windows impairment probe re-run.
- **T150b — `crpg-server`.** The `quic.rs` route: accept wire version 4 on
  `/3`, bind `bind_v4_peer`, and move lane-1 intents into `ingest_move`,
  receipts out of `take_move_receipts` and samples out of `take_samples`,
  traced in `HostCall`.

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
