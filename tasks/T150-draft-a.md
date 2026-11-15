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

Each part is one crate and one implementer run. Order: **serial**, T150a →
T150b. T150b needs T150a's `Framing::V3`, `try_recv_lane1`,
`try_send_lane1` and `try_send_datagram`.

| Part | Crate | Scope | S-set | Review |
|---|---|---|---|---|
| T150a | `crpg-net-quic` | V3 framing + lane-1 stream + datagrams + dedup patch + probe | **yes** | Claude reviewer mutant run, one fix round |
| T150b | `crpg-server` | the `quic.rs` v4 route | no | same |

**S-set** (`docs/IMPLEMENTATION_PLAN.md` §1.4, read as T140 read it): T150a
fixes wire bytes (a new ALPN = a new stream mapping and DATAGRAM
negotiation) and vendors a dependency patch (the workspace `Cargo.toml`
`[patch.crates-io]` and `third_party/quinn-proto/` are S-set). T150b fixes
no new byte: every wire byte it moves is already fixed by T026p (lane 1),
T140 (v3) and the T037 host surface; it maps `DeliveryKind` and host calls
to the transport API, so it is not S-set.

### T150a owns
- `crates/crpg-net-quic/src/handshake.rs` (`ALPN_LANE0_V3`, `Framing::V3`),
  `crates/crpg-net-quic/src/tls.rs` (the V3 transport parameters and the
  datagram buffers), `crates/crpg-net-quic/src/queue.rs` (the lane-1
  reader/writer and the datagram path), `crates/crpg-net-quic/src/server.rs`,
  `crates/crpg-net-quic/src/client.rs` (the lane-1 and datagram API),
  `crates/crpg-net-quic/src/limits.rs`, `crates/crpg-net-quic/src/close.rs`
  (a datagram send error), `crates/crpg-net-quic/src/lib.rs` (re-exports)
- `crates/crpg-net-quic/tests/lane1.rs` (new), `crates/crpg-net-quic/tests/lane1_datagram.rs` (new)
- `third_party/quinn-proto/patches/0002-widen-dedup-window.patch` (new),
  `third_party/quinn-proto/crate/src/connection/spaces.rs` (re-derived by the
  patch, never hand-edited), `third_party/quinn-proto/VENDOR.md` (patch table
  + tree digest), the root `Cargo.toml` (only if the `[patch.crates-io]`
  comment changes; the table itself already points at `third_party/quinn-proto/crate`)
- `docs/architecture/crpg-net-quic.md`, `crates/crpg-net-quic/AGENTS.md`
- goldens: none new, none moved.

### T150b owns
- `crates/crpg-server/src/quic.rs` (the `Framing::V3` arm, `Invitation`'s
  two new fields, the lane-1 drain/deliver steps, the `HostCall` variants)
- `crates/crpg-server/tests/host_quic_move.rs` (new)
- `docs/architecture/crpg-server.md`, `crates/crpg-server/AGENTS.md`
- goldens: none new, none moved.

## Readings

Design decisions this contract makes. Each needs approval with the contract.

1. **A new ALPN `crpg-lane0/3` = `Framing::V3`.** A second stream plus the
   DATAGRAM extension is a stream-mapping and framing change, which ADR-0025
   item 2 requires a new ALPN for
   (`docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:31–34`), and the
   crate trap forbids enabling either without one
   (`crates/crpg-net-quic/AGENTS.md:125`). `Framing::V3`
   (`crates/crpg-net-quic/src/handshake.rs:71`) pins: the `/2` channel
   framing on stream 1 (Lane0, Transfer, Resume), the second client-initiated
   bidi stream for lane 1, and the DATAGRAM extension with a minimum of
   `MAX_MOVE_DOWN_BYTES`. *Rejected:* enabling datagrams under `/1` or `/2`
   (the trap); a new `crpg-lane1/1` prefix (breaks the `crpg-lane0/N`
   framing-version namespace, which already carries the snapshot and resume
   channels under one ALPN, `:100–128`).
2. **Lane 1 is a second client-initiated bidi stream, not a `/2` channel.**
   `Channel` (`crates/crpg-net-quic/src/handshake.rs:103–128`) multiplexes
   the lane-0 stream by a selector byte; lane 1 is its own sequence domain
   per epoch (ADR-0036 L1,
   `docs/adr/0036-movement-and-navigation.md:389–390`) and its frames start
   with `0xA1` (`crates/crpg-net/src/movement_v4.rs:32`), not a selector.
   *Rejected:* a `Channel::Movement = 0xA1` on stream 1 (would make the
   selector depend on payload bytes, against the "never choose a channel from
   payload bytes" trap, and would couple lane-1 order to lane-0's one queue).
3. **The lane-1 stream reuses the 4-byte length framing.**
   `frame_header`/`parse_frame_header`
   (`crates/crpg-net-quic/src/handshake.rs:43–67`). Upstream cap
   `MAX_INTENT_FRAME_BYTES` (4,096, the same cap `decode_intent` enforces,
   `crates/crpg-net/src/movement_v4.rs:229`); downstream cap
   `MAX_MOVE_DOWN_BYTES` (1,024, above the 60-byte largest receipt,
   `tasks/T037.md:459`). *Rejected:* a channel-framed lane-1 stream (on its
   own stream lane 1 needs no selector); a 65,536 downstream cap (a receipt
   is ≤ 60 bytes; 1,024 keeps the two downstream shapes on one bound).
4. **The datagram minimum is exactly `MAX_MOVE_DOWN_BYTES` (1,024).** The
   client advertises `max_datagram_frame_size = Some(1_024)`; the server
   refuses, before writing the welcome, a client whose advertised size is
   absent or `< 1_024`, with `CloseCode::ProtocolViolation` (2). The largest
   sample is 1,012 bytes (24 + 38 × 26,
   `tasks/T026p.md:607–609`), so 1,024 admits every legal sample.
   *Rejected:* a minimum below 1,024 (could refuse a legal sample); a new
   close code (none is approved; 2 already means "the peer violated the
   pinned framing", and the ALPN pins the minimum).
5. **The dedup patch is `0002-widen-dedup-window.patch`**, applied after
   0001 in file-name order by the vendoring mechanism
   (`third_party/quinn-proto/VENDOR.md:3–9`, `:69–70`). It widens
   `Dedup`'s window from 129 to 2,049 packets
   (`docs/reviews/T030-quinn/README.md:14–15`, `:77–92`) so reordered-but-new
   datagrams are not dropped (quinn#2710). *Rejected:* keeping the 129-bit
   window (silent datagram loss under reordering; D23 requires the patch with
   the first datagram lane, `tasks/DECISIONS-2026-09-30.md:83–88`).
6. **Samples flow downstream only, as datagrams only.** The server advertises
   `max_datagram_frame_size = None` (so a client `send_datagram` fails) and a
   bounded `datagram_send_buffer_size`; the client advertises
   `Some(MAX_MOVE_DOWN_BYTES)` and a bounded `datagram_receive_buffer_size`.
   *Rejected:* the server advertising a receive size (accepts unsolicited
   upstream datagrams lane 1 has no codec for, `movement_v4` has no upstream
   datagram shape).
7. **Datagram backpressure is drop-whole, repaired by refresh.** A
   `try_send_datagram` that is `Blocked` (buffer full) or `Closed` is not
   retried by the adapter; the host's 20-tick refresh
   (`POLICY_SAMPLE_REFRESH_TICKS`, `tasks/T037.md:456`) replaces the lost
   sample. Lane-1 stream backpressure reuses the lane-0 queue caps. The
   datagram buffers are limits (below), not new unbounded queues.
   *Rejected:* retrying a refused datagram (a stale sample is worse than
   none, and ADR-0036 L4 makes samples lossy).
8. **The lane-1 stream opens after the welcome.** The client opens stream 1
   (hello) and, once the handshake completes, stream 2 (lane 1); the server's
   accept loop expects a second `accept_bi` on `/3`. The server reads lane-1
   intents only from a bound connection. *Rejected:* opening stream 2 before
   the hello is accepted (the server cannot distinguish lane 1 from lane 0
   traffic until the framing is fixed).
9. **The `quic.rs` v4 route binds only wire version 4 on `/3`.** `on_hello`
   (`crates/crpg-server/src/quic.rs:596`) gains a `Framing::V3` arm that
   accepts wire version 4 and calls `bind_v4_peer`; `Invitation` gains
   `facts: FactGrants` and `rights: PauseRights`, because `bind_v4_peer`
   builds the first snapshot from them (`tasks/T037.md:437–441`) and
   `FactGrants` has no `Default` (`crates/crpg-server/src/host.rs:244`).
   v3-over-QUIC is not part of T150 (Out of scope). *Rejected:* empty
   defaults plus post-bind `set_fact_grants`/`set_pause_rights` (the first
   delivery is already staged at bind); a separate v4 invitation type (the
   same principal binds once, so the fields belong on `Invitation`).

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
