# ADR-0025: Lane-0 QUIC mapping and pinned trust

Date: 2026-10-04 (UTC)
Status: **Accepted** — filed by T023 as the ADR that its re-contract names
(R9). The user approved the re-contract, including "ADR-0025 (lane-0 QUIC
mapping and pinned trust) with T023" (revised question R13), on 2026-10-04.
That approval is recorded in [tasks/T023.md "Decisions on the revised
questions — 2026-10-04
(UTC)"](../../tasks/T023.md#decisions-on-the-revised-questions--2026-10-04-utc).

## Context

`crpg-net` defines the lane-0 combat wire (v1 and v2 intent/delta codecs)
and a net-local `Transport` byte-pipe trait. Until T023, its only transport
was the T018b in-memory fabric. A real host needs those bytes over a network
with authenticated host binding (D03), bounded ingress and egress, and
reliable ordered delivery. Datagrams and movement are not part of this
(D22/D23 scope lane 0 to reliable streams). ADR-0024 places this transport
in its own crate, `crpg-net-quic`, so that `crpg-net` keeps no I/O. This ADR
records the durable wire and trust decisions that crate implements. Any
peer, test client or later implementation must match them byte for byte.

## Decision

1. **One bidirectional stream per connection.** The client opens exactly one
   bidirectional QUIC stream. It carries the hello, the welcome, and then
   every lane-0 frame in both directions. The server permits one
   client-initiated bidi stream and zero uni streams. The DATAGRAM extension
   is off (`max_datagram_frame_size` is not advertised), so a peer's
   `send_datagram` fails with "unsupported by peer".
2. **ALPN `crpg-lane0/1` is the framing version.** It pins the framing, the
   hello/welcome layout, the stream mapping and the close codes. Any other
   ALPN fails TLS. A change to any of these needs a new ALPN, for example
   `crpg-lane0/2`. The hello names the lane-0 wire version (1 or 2). The
   server accepts that exact version or refuses it; nothing is negotiated
   or downgraded (ADR-0019).
3. **Length-prefixed framing with caps checked before allocation.** Each
   frame is a 4-byte big-endian `u32` length followed by that many payload
   bytes. A zero length is a violation. The caps are:
   - client → server: 4,096 bytes (`MAX_INTENT_FRAME_BYTES`);
   - server → client: 65,536 bytes (`MAX_DELTA_FRAME_BYTES`);
   - hello: at most 130 bytes;
   - welcome: exactly 17 bytes.

   A length over the cap closes the connection with `FrameTooLarge` (3), and
   no body is read. The transport never decodes intent or delta payloads.
4. **Hello and welcome.**
   - The hello is `[wire_version][cred_len][credential]`, with `cred_len`
     in 16..=128.
   - The welcome is `[wire_version][epoch: 16]`, and its version must equal
     the hello's.
5. **SHA-256 DER pinning (D03).**
   - The client trusts exactly one server certificate: the one whose DER
     hashes (SHA-256) to the pin it received out of band with its
     invitation.
   - A certificate chain is refused.
   - No webpki roots, no platform verifier and no TOFU are used, and there
     is no hostname, validity-period or revocation check. Rotating the
     certificate means issuing a new invitation.
   - The TLS 1.3 handshake signature is still verified, so the server must
     hold the pinned key.
   - TLS 1.3 only, with the ring provider. There is no 0-RTT, no session
     resumption and no client certificate.
   - The SNI is `crpg.invalid` and is ignored.
6. **The credential is checked for format only.** It is opaque, 16..=128
   bytes, and travels only inside the TLS-protected stream after the pin
   has been verified. The transport enforces its length. The host adapter
   judges validity and binding (T023b), using the constant-time `ct_eq`. A
   credential is never printed, logged or put in a close reason.
7. **Application close codes 0–9.** Each code is sent with its name as the
   QUIC reason phrase. A peer code outside 0..=9 is kept raw.

   | Code | Name |
   |---|---|
   | 0 | `Normal` |
   | 1 | `Shutdown` |
   | 2 | `ProtocolViolation` |
   | 3 | `FrameTooLarge` |
   | 4 | `HelloTimeout` |
   | 5 | `AuthRefused` |
   | 6 | `VersionRefused` |
   | 7 | `ServerBusy` |
   | 8 | `AuthTimeout` |
   | 9 | `SessionFenced` |

8. **Privileged control fails closed.** Because of item 1's stream limits, a
   GM/admin control stream cannot be opened. Adding one needs a new ALPN and
   its own D03-reviewed design.

## Authority and affected surface

- Decided by: the user, through the approval of T023's re-contract (Q1, Q2,
  Q4, Q12 and revised questions R1–R14, 2026-10-04).
- Authoritative contract: [tasks/T023.md](../../tasks/T023.md), the
  re-contract (R2–R9), carrying E§3–E§8 over with its renames.
- Owning crate(s)/files: `crates/crpg-net-quic` (placement: ADR-0024).

## Compatibility, hash and dependency impact

- Persisted/serialized shapes: none. The lane-0 v1/v2 payload bytes are
  `crpg-net`'s and unchanged; this ADR adds only the transport envelope
  around them.
- Replay, `state_hash`, goldens: unchanged. The transport touches no
  `World`, RNG, event queue or simulation clock.
- Dependency graph: `crpg-net-quic`'s `ALLOWED` row is `{"crpg-net"}`
  (ADR-0024). The external edges are the D23 pins (quinn =0.11.12,
  quinn-proto =0.11.19, rustls =0.23.45 with ring, tokio =1.53.1 with
  `rt`/`net`/`time`/`sync`). The `deny.toml` skips are recorded in the
  ADR-0023 addendum.
- Platform scope (ADR-0012): the crate is platform-neutral (no
  `cfg(target_os)`). It is gated on native Windows/MSVC and Linux/GNU.

## Alternatives considered

- **Datagrams for lane 0.** Rejected by D22/D23. Lane 0 needs reliable
  ordered delivery, and datagrams wait for the movement lane (T026p) and the
  dedup patch.
- **One stream per command, or one per direction.** Rejected. It loses the
  single total order per direction that the host's sequence and receipt
  logic relies on, and it would need extra stream limits and reassembly.
- **WebPKI roots, a platform verifier or TOFU.** Rejected by D03. A
  self-hosted game server has no public CA identity, and trust-on-first-use
  accepts a first-contact attacker.
- **Skipping over-cap frames instead of closing.** Rejected (R8). On a byte
  stream, skipping an attacker-chosen length means reading attacker-sized
  data.
- **Version negotiation in the hello.** Rejected (ADR-0019). The host
  selects one version and refuses others.

## Consequences

- Any change to the stream mapping, framing, hello/welcome layout or close
  codes needs a new ALPN and an ADR. T024 (snapshots), T025a (resume) and
  T026p (datagrams) each add their own.
- Certificate rotation is an invitation reissue.
- Bytes already handed to QUIC cannot be recalled, so fencing a binding
  closes the connection (`SessionFenced`) until a resync or reconnect design
  exists (T024/T025a; revised question R4).
- Credential policy (content, storage, comparison, revocation) stays with
  T023b. The transport guarantees only its length and confidentiality in
  transit.

## Acceptance and evidence

Implemented by T023. The evidence is its public endpoint suite
`crates/crpg-net-quic/tests/quic.rs` (cases 1–23), recorded with commands,
counts and environment in the T023 completion record in
[tasks/T023.md](../../tasks/T023.md). The Linux/GNU run is recorded there;
the native Windows/MSVC run is CI's and is pending at filing.

## Supersession and corrections

## Addendum — 2026-10-05 (UTC): flow-control windows are tightening-only limits (T023c)

Accepted with the approval of [T023c](../../tasks/T023c.md).

1. The QUIC stream receive window, connection receive window and send
   window of each endpoint are no longer fixed in `tls.rs`. They are the
   `ServerLimits` and `ClientLimits` fields `stream_window_bytes`,
   `connection_window_bytes` and `send_window_bytes`, under the same
   tightening-only rule as every other limit. Their v1 values are the
   values T023 fixed: server 65,536 / 131,072 / 1,048,576, client
   1,048,576 / 2,097,152 / 262,144. An endpoint built from `v1()`
   advertises and enforces exactly what it did before.
2. Each window holds at least one maximal frame of the direction it
   carries, plus the 4-byte header: 4,100 bytes for intents (client
   send, server receive) and 65,540 for deltas (server send, client
   receive). A connection receive window is at least its stream receive
   window.
3. This is not a wire change. Transport parameters are exchanged in the
   QUIC handshake, and every QUIC peer must honour whatever the other side
   advertises. The stream mapping, framing, hello and welcome, close codes
   and ALPN `crpg-lane0/1` are unchanged, so no new ALPN is needed.
4. Every other E§8.3 parameter stays fixed: one client-initiated bidi
   stream, no uni streams, no DATAGRAM extension, no migration,
   `max_incoming = max_connections`, no 0-RTT.
5. Why: a peer that stops reading is bounded by its **own** receive
   windows, not by the sender's send window, because quinn releases
   acknowledged data and a stalled reader still acknowledges. Tests and
   hosts that need a stalled client to push back within a few frames must
   be able to tighten the client's receive windows. Operators can tighten
   the per-connection buffer memory quinn may hold (up to
   64 × (65,536 + 1,048,576) bytes on a v1 server).
6. Recorded transport behaviour, not changed here: quinn-proto 0.11.19
   congestion-controls the `CONNECTION_CLOSE` packet of a locally closed
   connection that still holds unsent stream data, and it processes no
   ACKs once closed. If `bytes_in_flight + one datagram ≥ cwnd` when the
   close happens, the close is never sent. The peer then learns of it
   only from a stateless reset after the 3 × PTO drain
   (`CloseReason::Reset`), or from its own idle timeout. Smaller windows
   lower the bytes in flight and so the exposure, but do not remove it.
   Evidence: T023c C§1.4.

## Addendum — 2026-10-05 (UTC): quinn-proto's close gating fixed by vendoring (T023v)

Accepted with the approval of [T023v](../../tasks/T023v.md).

1. The workspace builds quinn-proto 0.11.19 from `third_party/quinn-proto/`
   with upstream commit `e556fde` applied (quinn-rs/quinn#2787, which fixes
   #2785). A close packet is no longer held by congestion control or
   pacing. The anti-amplification limit still applies.
2. A locally closed connection's close code therefore reaches the peer even
   with a full congestion window. Case 28
   (`discard_close_reaches_peer_when_congestion_blocked`) shows it end to
   end. Case 17 runs on v1 limits again, because T023c C§8.5 is withdrawn.
3. No wire, ALPN, transport-parameter or limit change. Item 6 of the T023c
   addendum stays as the record of unpatched quinn-proto. It applies again
   only if the vendoring is retired without an upstream fix.

## Agent log

- 2026-10-04 (UTC) · claude-code + T023 · Filed the lane-0 QUIC wire and trust decisions (single bidi stream, ALPN `crpg-lane0/1`, capped length-prefix framing, SHA-256 DER pinning, format-only credential, close codes 0–9, fail-closed control) as Accepted on the user's recorded approval of T023's re-contract (R13), so the durable wire choices have their own record apart from ADR-0024's placement.
- 2026-10-05 (UTC) · claude-code + T023c · Appended the approved T023c addendum making the three QUIC flow-control windows per endpoint tightening-only limits with v1 equal to the T023 values, and recording quinn-proto's congestion-gated `CONNECTION_CLOSE`, so the wire record says which transport parameters are still fixed and why a `Reset` can replace a close code.
- 2026-10-05 (UTC) · claude-code + T023v-b · Appended the approved T023v addendum: quinn-proto is vendored with upstream's close fix, case 28 proves the close code reaches a congestion-blocked peer and case 17 is back on v1 limits. The wire record now says the `Reset` exposure of the T023c addendum is closed while the vendoring stands.
