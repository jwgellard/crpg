# ADR-0026: QUIC host adapter: invitation credentials, fencing and backpressure

Date: 2026-10-06 (UTC)
Status: **Accepted**. The user approved T023b's exact contract on 2026-10-05,
including B§16 Q1 ("ADR-0026 … is written by T023b and Accepted with this
approval"). The approval is recorded in [tasks/T023b.md, "Decisions —
2026-10-05 (UTC)"](../../tasks/T023b.md#decisions--2026-10-05-utc). The
stall-case amendment and the implementer's readings were approved on
2026-10-06 ("Amendment decisions" and "Reading decisions" in the same file).
None of them changes the decisions below.

## Context

T022 built `crpg-server`'s host as an in-memory authority. It takes
already-authenticated peers through `bind_peer` and lane-0 bytes through
`ingest` → `pump` → `take_delivery`/`acknowledge_delivery`. T023
([ADR-0024](0024-separate-crpg-net-quic-crate.md),
[ADR-0025](0025-lane0-quic-mapping-and-pinned-trust.md)) built the real
transport, `crpg-net-quic`. It checks an invitation credential's length
only and leaves judging it to the host adapter. Something has to connect
the two: decide which QUIC connection may become which host binding, move
bytes both ways under bounded queues, and end bindings without losing
committed receipts. These choices bind the dedicated server (T040) and
reconnect (T025b), so they are recorded here rather than only in the task
file.

## Decision

1. **One adapter owns the whole host.** `crpg_server::quic::QuicHost` takes
   ownership of a `Host` with no binding and no staged ingress, plus a
   bound `QuicServer`, and gives the host back on start failure and on
   shutdown. Nothing else can unbind, regrant or acknowledge behind it.
   Mixing in-memory and QUIC peers on one host (a listen server) is not
   supported.
2. **The invitation is the principal (D03).** An operator-registered
   invitation holds exactly 32 opaque credential bytes, the control grant
   and the disclosure grants. Grant and control changes are made on the
   invitation, so a reconnect cannot bring back revoked disclosure.
   `crpg-server` generates no credential (T040 owns generation and the
   invitation format). Credentials stay in memory only and are never
   printed, logged, checkpointed or put in a close reason. They are not
   zeroized.
3. **Constant-time matching, credential before version.** A hello's
   credential is compared with `Credential::ct_eq` against every registered
   invitation, folding the match flags with no early exit. Unknown, revoked
   and wrong-length credentials all get `AuthRefused` (5), and that check
   comes before the wire-version check (`VersionRefused`, 6). A full host
   gets `ServerBusy` (7) without consuming a handle.
4. **Newest connection wins.** A hello presenting an already-bound
   invitation fences the old binding as superseded (`SessionFenced`, 9) and
   then binds the new one.
5. **Delivery is acknowledged on hand-off.** A host delivery frame is
   acknowledged once `QuicServer::try_send` accepts it. QUIC delivers in
   order, exactly once, while the connection lives. A lost connection ends
   the session until T025a/T025b. Lost replies are recovered by retrying
   the same seq, which returns a cached receipt.
6. **Backpressure holds one frame.** When `ingest` refuses a frame as
   `QueueFull`, the adapter holds that one frame and reads nothing further
   from its connection until it stages. The transport's bounded queue and
   QUIC flow control then push back on the client. Frames refused as
   `RateLimited`, `FrameTooLarge` or `Unauthenticated` are dropped. A held
   frame's retries, and any backlog behind it, still spend rate tokens.
7. **Fencing closes the connection.** Unbind, revocation, narrowing grants,
   supersession, a host-side `SeqConflict` fence and a slow consumer all
   close with `SessionFenced` (9) in `Discard` mode. Narrowing does not
   call `Host::set_grants`, so no resync generation ever exists; the
   principal reconnects to a fresh epoch and state.
8. **Slow-consumer fence.** A connection whose transport keeps refusing
   delivery for `STALL_TIMEOUT_MS` = 10,000 ms of injected time is fenced.
   Until then a stalled peer can hold up host admission (T022's
   head-of-line rule). The value is a constant until T040 has operator
   configuration.
9. **Drain then unbind.** Frames fully received before a peer's close are
   still offered to the host, in order. Only then is the binding unbound.
   The departed peer's delivery is discarded and acknowledged, so it never
   blocks admission.
10. **D02 shutdown order.** Stop admissions, flush delivery already
    committed, count what could not be sent, refuse staged work
    (`SessionExpired`), discard held frames, optionally checkpoint the
    fenced host, then close the transport with `Shutdown` (1).
11. **Synchronous and injected.** The adapter reads no clock, spawns no
    thread, opens no socket and never blocks inside `pump`. `now_ms` is the
    caller's, and the socket, runtime and thread are `crpg-net-quic`'s.
    Every host call a pump makes is recorded in order, so the host's
    transitions can be replayed against an in-memory host.

## Authority and affected surface

- Decided by: the user, approving T023b's contract (B§16 Q1–Q13) on
  2026-10-05 and its stall-case amendment (A§7 Q1–Q8) and readings on
  2026-10-06.
- Authoritative contract: [tasks/T023b.md](../../tasks/T023b.md), "Exact
  contract — 2026-10-05" (B§1–B§18) as amended by "Contract amendment —
  2026-10-06" (A§0–A§7). The public items are those of B§3.
- Owning crate(s)/files: `crates/crpg-server/src/quic.rs` (public module
  `crpg_server::quic`), plus `pub(crate)` visibility changes in
  `crates/crpg-server/src/host.rs`.

## Compatibility, hash and dependency impact

- Persisted/serialized shapes: unchanged. Checkpoints encode the `Host`,
  which never sees a credential (D10). `CHECKPOINT_VERSION`, the save
  header and the lane-0 wire are untouched.
- Replay, `state_hash`, goldens: unchanged. No golden is added. The
  equivalence evidence is exact-build and in-process (ADR-0012).
- Dependency graph: one new path edge, `crpg-server → crpg-net-quic`, which
  `ALLOWED` already permits and ADR-0024/T023 R12 assign to this task. No
  direct quinn, quinn-proto, rustls or tokio edge, and no new external
  crate: `Cargo.lock` gains one line. `crpg-server`'s normal graph grows
  from 40 to 70 packages on Linux/GNU and from 40 to 72 on Windows/MSVC.
- Platform scope (ADR-0012): platform-neutral source. It is gated on
  native Windows/MSVC and Linux/GNU through the existing workspace jobs.

## Alternatives considered

- **Refuse a second connection for a bound invitation (`ServerBusy`).**
  Rejected: until T025a, a client whose path died would be locked out for
  up to the 60 s idle timeout. Someone holding the credential can already
  act as the principal, and revocation is the remedy.
- **Acknowledge on an application-level ack.** Rejected: lane 0 has none,
  and QUIC already delivers reliably within a connection.
- **Unbind on first detection of a peer close.** Rejected: which commands
  ran would then depend on timing. Draining first makes a `Flush` close
  deterministic.
- **Seamless narrowing through `set_grants` and resync.** Deferred to
  T024/T025a. Bytes already handed to QUIC cannot be recalled (ADR-0025).
- **A new close code for slow consumers.** Rejected: ADR-0025 fixes codes
  0–9, and a new code needs a new ALPN.
- **Credential generation in `crpg-server`.** Rejected here: it needs a
  CSPRNG dependency and its own record. It belongs with T040's invitation
  format.
- **Completing staged work at shutdown.** Rejected: that would run a pump
  that can block behind a slow peer during shutdown.

## Consequences

- T040 must pump more often than the transport's `decision_timeout_ms`
  (5 s), or hellos close with `AuthTimeout`. It must convert its monotonic
  clock to `now_ms` at or above the host's last accepted time, generate
  and distribute credentials with the certificate pin, and give the
  operator a pump cadence of at least 25 ms while frames are held. Below
  25 ms, held-frame retries drain the rate bucket.
- A slow consumer can block host admission for up to `STALL_TIMEOUT_MS`.
  Fenced connections may already have received a prefix of their queued
  delivery.
- T025b inherits newest-wins supersession and the fresh-epoch rule, and
  adds resume on top of them.
- Credentials are not zeroized (no `zeroize` dependency).

## Acceptance and evidence

Implemented by T023b. The evidence is `crates/crpg-server/tests/host_quic.rs`
(25 tests: 8 cases × 2 wire versions plus 9 single cases) and the
unchanged `host_capture` (31) and `host_save` (20) suites. Commands, counts
and environment are in the T023b completion record in
[tasks/T023b.md](../../tasks/T023b.md). The Linux/GNU run is recorded
there; the native Windows/MSVC run is CI's and was pending at filing.

## Supersession and corrections

## Agent log

- 2026-10-06 (UTC) · claude-code + T023b · Filed ADR-0026 Accepted, as the approved contract (B§16 Q1) requires, recording the adapter choices T025b and T040 inherit: the invitation principal, constant-time matching, newest-wins supersession, acknowledge on hand-off, the one-frame hold, `SessionFenced` fencing, the slow-consumer fence, drain-then-unbind and the D02 shutdown order.
