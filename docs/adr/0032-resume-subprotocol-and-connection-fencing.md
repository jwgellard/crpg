# ADR-0032: Resume subprotocol and connection fencing

Date: 2026-10-30 (UTC)
Status: **Accepted** — filed by T025a1 (D39d, S§26 item 9).

## Context

`crpg-net` needs a way for a retained session to resume across a connection
loss inside the host's 30-second same-process grace (D12), without rotating
the epoch (which would break every cached retry, because the epoch is inside
every command's canonical bytes). Resume arrives as a separate subprotocol
beside frozen v1/v2 combat and the `0x81` transfer wire, because lane 0's v1
wire stays byte-identical and its `SessionEpoch` doc still says reattachment
is unsupported. Authoritative contract: `tasks/T025a.md`.

## Decision

1. **A separate resume subprotocol, first byte `0x91`, lane 3.** The high
   nibble names the family and the low nibble its version: `0x8_` is
   transfer (`0x81` now, `0x82` reserved for transfer v2), `0x9_` is resume.
   Downstream tags `0x30` Ticket, `0x31` Accepted, `0x32` Rejected; upstream
   `0x40` Request. Tags are disjoint from every existing tag.
2. **Over QUIC, resume frames take a third `/2` channel value, `0x91`** (one
   value, one codec). A resume-first connection is opt-in on
   `bind_resumable`; the request replaces the hello, the result replaces the
   welcome. The amended T024q carries this (S§20).
3. **Epoch retained; a separate per-session `ConnectionFence`.** The fence
   starts at `FIRST` (1) and each accepted resume grants `current + 1`; a
   grant is never re-issued, so at most one connection of a session passes
   at any time. The fence never appears on a lane-0 or transfer frame.
4. **Explicit decision input; net never authorises.** `SessionFence::resolve`
   takes a `ResumeDecision` (`Accept { progress }` or `Reject(reason)`) that
   the host builds after authenticating. Net checks only epoch, fence order
   and shape. No trusted flag, no test hook, no `Accepted` without `Accept`.
5. **An opaque proof, `ResumeProof`**, 16..=128 bytes, no
   `PartialEq`/`Eq`/`Ord`/`Hash`/`Display`, `Debug` exactly
   `ResumeProof(<redacted>)`, and a constant-time `ct_eq` (XOR fold, no early
   exit, `black_box`). A downstream `Ticket` delivers the proof.
6. **Advisory claims in, authoritative progress out.** The request carries
   per-lane `LaneClaim { lane, receipted_through }`; the accepted result
   carries `LaneProgress { lane, finalized_through }` from the host's
   decision. Lane 0 is the only assigned lane in resume v1; a claim never
   changes state or a result.
7. **Two cause-neutral rejections: `Denied` (1) and `Busy` (2).** No
   `Expired` (it would tell an unauthenticated caller a session existed) and
   no per-cause codes. Any net refusal is answered by the host as `Denied`.
8. **`rebind_receiver` keeps the floors and fails closed.** It re-anchors a
   retained receiver without resetting `next_event_seq`, `retired_through`,
   `published` or queued updates; it discards and retires the in-flight
   transfer, drops held deltas, sets `AwaitSnapshot` and clears `resync`.
9. **No new `TransferError` or `CodecError` variant.** Codec errors are the
   six existing `CodecError` variants; mechanism errors are a new closed
   `ResumeError`.
10. **v1, v2 and `0x81` stay byte-identical.** The only edit to an existing
    source module is the additive `rebind_receiver` in `reassembly.rs`.

The wire (constants, identity, proof, messages, codec) is T025a1; the fence,
decision and rebind mechanisms are T025a2.

## Host requirements (T025a S§21)

Authoritative contract: `tasks/T025a.md` S§21.

- **H1.** Create one `SessionFence` per session at bind, with the session's
  epoch. Retain it through the D12 grace, drop it only at expiry or unbind,
  and never re-create it for a live session. Over QUIC, bind with
  `QuicServer::bind_resumable` and drive it with `poll_notice` only.
- **H2.** Tag the first connection `FIRST` and a resumed connection with
  exactly the `granted` fence. Call `check_frame(tag)` on every inbound frame
  of a bound connection before any other processing. On `Fenced` or
  `UnknownFence`, discard the frame and close that connection
  (`SessionFenced`).
- **H3.** Decode with `decode_request`, then build the `ResumeDecision` only
  after its own checks: the proof authenticated with `ct_eq` against the
  proof it issued, grace not expired, the invitation still registered, and
  its own use of the advisory claims. It never builds `Accept` without
  authenticating.
- **H4.** Call `resolve` once per request and send its result as the
  connection's first frame. On `Err(_)` from `resolve`, send `Rejected {
  epoch, reason: Denied }` (cause-neutral). After any `Rejected`, close the
  connection.
- **H5.** On `Accepted`, fence the previous connection if still open, send
  the result, then stage the resume snapshot before any later delta: cancel
  any in-flight transfer, then a `Begin` with an id above every id issued in
  the epoch, `cursor` = the last `event_seq`, and a body built from current
  grants.
- **H6.** Never reset the epoch, `PeerState`, the retry cache,
  `next_new_seq`, `event_seq`, ordinals, transfer ids or rate budgets at
  resume. Report `Accept { progress }` from the retained ledger.
- **H7.** Issue a `Ticket` after each welcome, optionally rotating it after
  each `Accepted`. Proofs are never logged, printed, checkpointed or
  persisted. A restart issues fresh epochs.
- **H8.** A `Busy` or `Denied` never moves the fence, and a resume attempt
  never extends the grace deadline.

Mechanism lives here; authenticated policy lives in `crpg-server` (T025b).

## Authority and affected surface

- Owning crate: `crates/crpg-net`. T025a1: `protocol_resume`, `codec_resume`;
  T025a2: `resume`, an additive `rebind_receiver` in `reassembly`.
- Contract: `tasks/T025a.md` S§0–S§27.

## Compatibility, hash and dependency impact

- No new dependency. v1/v2/`0x81` bytes untouched; no `MAX_*`/`POLICY_*`,
  tag or seq-meaning change. No goldens, `.replay` files or `state_hash`.

## Consequences

- Follow-on tasks: T024q (channel `0x91`), T025a2 (fence/decision/rebind),
  then T025b (host grace, proof issuance/verification, QUIC resume entry).

## Agent log

- 2026-10-30 (UTC) · opencode-go/deepseek-v4-pro + T025a1 · Filed ADR-0032 Accepted per D39d S§26 item 9, recording S§2 and H1–H8 with the contract as authoritative.
