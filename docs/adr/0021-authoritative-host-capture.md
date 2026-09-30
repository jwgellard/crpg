# ADR-0021: Authoritative host capture and fresh-session restart

Date: 2026-09-29 (UTC)
Status: Selected under recorded D02/D03/D10 delegation for specification;
source, dependency approval, independent review, and native acceptance remain
T022 work. No independent human review is claimed.

## Context

T020 pins the opt-in `HistoryWorld` journal, transaction, read/ack, and
full-hash contract ([ADR-0017](0017-opt-in-authoritative-history.md)). T021
pins explicit v1/v2 codecs and the host-fed per-field projection contract
([ADR-0019](0019-event-protocol-v2.md)). Both specifications are complete for
review; both implementations live in the working tree uncommitted, with
review/merge outstanding. Neither may be treated as approved or merged.

The host slice needs the same treatment before source: without a pinned
admission/capture/checkpoint contract, implementation would invent host policy
during coding — exactly what the T022 owner gate forbids. The open inputs are
the host lifecycle API, the bounded atomic-capture discipline, the checkpoint
representation with the mandatory pre-parse byte cap, and the trust boundary
between authenticated operator facts and untrusted client bytes.

## Decision

Adopt [T022's specification revision](../../tasks/T022.md#specification-revision--2026-09-29)
as the normative exact API/error/cap/checkpoint/acceptance contract. The
structure below records why each load-bearing choice was made; the task
appendix — not this ADR — is normative. Where the two disagree, the appendix
governs and this ADR is superseded by a new one, never edited.

1. **One platform-neutral host in `crpg-server`, in-memory first.** D02
   already selects the owner and the single-implementation rule (Windows
   embedded and Windows/Linux dedicated share one authority). The first slice
   is transport-agnostic: raw intent bytes enter through `ingest`, staged
   work executes through `pump`, and encoded frames leave through
   `take_delivery`. The QUIC adapter (T023b) later feeds the same boundary;
   no socket, runtime, or thread type enters this slice.
2. **Explicit version selection, frozen v1.** The host is constructed with one
   `ProtocolSelection` (`V1` or `V2`) for its lifetime. There is no sniffing,
   negotiation, or downgrade: the unselected version's bytes are refused as
   `UnsupportedVersion`, and new tags inside a v1 envelope are refused as
   `UnknownMessage`, reusing the T021 codecs unchanged. Per-peer negotiation
   is deferred, not guessed.
3. **Trust boundary, not test-driver promotion.** `bind_peer`/`set_grants`/
   `set_control` take already-authenticated operator/embedding facts as
   trusted inputs. Credential issuance/verification, TLS termination, and
   entropy backends stay outside T022 behind the T030-style dependency audit;
   no credential crypto and no sim RNG enter the host. The T018c/T021
   drivers keep their status as test-only doubles: the host re-derives their
   admission order and failure semantics as a normative contract, it does not
   import their code or promote fixture grants into production policy.
   Production interest facts (T027a area, T027c viewer context) will later
   *supply values* for the same `ControlGrant`/`DisclosureGrants` shapes,
   not change their types without a new ADR.
4. **Serial two-phase admission.** `ingest` performs transport-level checks
   (peer, rate, size, queue) without consuming lane sequence; `pump`
   executes the command pipeline (decode, epoch, seq/cache, exhaustion,
   freshness, mapping/ownership, reservation, sim execution, capture) in
   FIFO order. Grants are evaluated at pump time so revocation closes the
   ingest-to-pump window. This split is what makes backpressure retryable
   and shutdown completable without losing accepted results.
5. **Reserve before executing; nothing fallible after mutation.** Every new
   command's worst-case footprint (one capture record, retry-cache bytes,
   per-peer worst-case delivery frames, sim journal range) is checked before
   `HistoryWorld::perform_action` runs. Reservation failure is retryable
   `QueueFull` with no sequence consumed and no mutation. After mutation,
   only infallible appends remain; the bounds that make them infallible are
   proven by tests, not assumed. Sim capacity failures commit nothing (T020
   guarantee) and surface as the same retryable `QueueFull`.
6. **Capture synchronously, acknowledge after.** Each accepted command's
   `ActionOutcome` (symbolic fields only — roll/margin never stored),
   history range, and permitted post-state are captured before the next
   command executes. The sim journal is acknowledged only through captured
   sequences. Accepted `EndTurn` is `Applied` with an empty outcome and no
   fabricated `ActionResolved`.
7. **Fresh sessions on restart, in-memory checkpoint.** D10 selects
   authoritative continuation with new epochs after process restart. The
   checkpoint carries the full `HistoryWorld`, the capture journal, and
   session identity — never live transport bindings, credentials, lane
   caches, delivery logs, or grace state, which reset to fresh. Old-epoch
   commands after restart are refused as `SessionExpired`, never reapplied.
   Persisted-session continuation is explicitly not selected, so the D10
   lost-receipt-after-restore clause is vacuous here rather than silently
   satisfied. The slice uses a bounded in-memory byte representation with a
   mandatory pre-parse input cap; any filesystem/compression backend is a
   separate `crpg-persist` task with its own compressed/decompressed caps,
   not a second-crate addition to T022.

## Consequences

No new dependency is approved by this ADR: the implementation will need
human-approved normal edges on `crpg-core`/`crpg-sim`/`crpg-net` (path) plus
workspace `serde`/`serde_json`, each with a T018a-style audit record, and a
`[lib]` target beside the thin dedicated binary. Checkpoint size honesty is
enforced by a max-shape measurement test against the 16 MiB cap, not by
deriving the World bound analytically. True `u64::MAX` lane exhaustion is
unreachable by counting, so no production test hook is added for it; the
dispatch order is pinned and the reachable eviction/gap/stale/conflict
boundaries carry the acceptance weight, with the residual stated openly in
the appendix. `docs/architecture/crpg-server.md` and
`crates/crpg-server/AGENTS.md` remain due with the first source task, not
this specification: writing them now would describe code that does not exist.

## Agent log

- 2026-09-29 (UTC) · opencode/muse-spark + T022 readiness ADR · Filed the selected host-capture/restart rationale under D02/D03/D10 delegation, keeping the normative contract in tasks/T022.md and naming the unaudited-auth, persist-backend, and exhaustion-reachability residuals as open gates rather than silent choices.
