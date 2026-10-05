# Upstream issue draft: quinn-proto withholds `CONNECTION_CLOSE` under a full congestion window

**Status: draft for the user to file** at
<https://github.com/quinn-rs/quinn/issues>. Nothing has been filed. The
evidence is [T023c](../../tasks/T023c.md) C§1.4 (measured in a scratch copy
of this repository at `86dbaff`, Linux/GNU, rustc 1.98.0, debug `test`
profile). The user decided on 2026-10-05 (T023c Q6) to report it upstream.
Edit freely before filing. The proposed issue is everything between the
horizontal rule and "Agent log"; the agent log stays in this repository.

Before filing, check:
- that no open or closed issue already covers it (search for
  "CONNECTION_CLOSE congestion", "close not sent", "stateless reset after
  close");
- that `main` still has the same `poll_transmit` logic (the line numbers
  below are from the 0.11.19 crate).

---

## Title

`CONNECTION_CLOSE` is congestion controlled when stream data is still
queued, and is never sent if the window is full at close time

## Versions

- quinn 0.11.12, quinn-proto 0.11.19 (the newest release when measured,
  2026-10-05), rustls 0.23.45 with ring, tokio 1.53.1.
- Linux x86_64 (reproduced), Windows x86_64 MSVC (observed intermittently in
  CI; not instrumented there).

## Summary

When a connection is closed locally with `Connection::close` while it still
holds unsent stream data in its send buffer, quinn-proto decides whether
the close packet is congestion controlled from the stream state, not from
what the packet will carry. If `bytes_in_flight + one datagram ≥ cwnd` at
that moment, the `CONNECTION_CLOSE` is never transmitted:

- not at first, because the packet is treated as ack-eliciting and is
  blocked by congestion control;
- and not later, because in the closed state no ACKs are processed, so
  `bytes_in_flight` never falls, and every re-armed attempt (after each
  peer packet) is blocked the same way.

The connection then sits in the closed state for 3 × PTO and drains. The
peer's next packet is answered with a stateless reset, so the peer sees
`ConnectionError::Reset` instead of the application's error code (or, with
no further packets, its own idle timeout).

As we read RFC 9002 §2, a packet whose only frame is `CONNECTION_CLOSE`
(plus padding) is not ack-eliciting, so it should not be held back by the
congestion controller.

## Where

`quinn-proto/src/connection/mod.rs`, `Connection::poll_transmit`
(0.11.19):

```rust
let mut ack_eliciting = !self.spaces[space_id].pending.is_empty(&self.streams)
    || self.spaces[space_id].ping_pending
    || self.spaces[space_id].immediate_ack_pending;
if space_id == SpaceId::Data {
    ack_eliciting |= self.can_send_1rtt(frame_space_1rtt);   // streams.can_send_stream_data()
}
// ...
if ack_eliciting && self.spaces[space_id].loss_probes == 0 {
    // ...
    if self.path.in_flight.bytes + bytes_to_send >= self.path.congestion.window() {
        // blocked by congestion control
        continue;
    }
```

In the closed state the packet built here carries only `CONNECTION_CLOSE`,
but `ack_eliciting` is still `true` when stream data remains queued, so the
congestion check applies.

In `process_decrypted_packet`, the `State::Closed(_)` arm only looks for a
peer `CONNECTION_CLOSE`; ACK frames are ignored, so in-flight bytes are
never released after the close. `close_inner` arms `Timer::Close` at
`3 × PTO`, after which the connection drains and the endpoint answers the
peer with a stateless reset.

## Reproduction

Our reproduction is inside an application test suite, so this is the
shape rather than a standalone program:

1. Server and client over loopback, one client-opened bidirectional
   stream. Default congestion controller (Cubic), default MTU and pacing.
   The server has a 1 MiB send window; the client has a 1 MiB stream
   receive window and a 2 MiB connection receive window and reads the
   stream continuously.
2. The server hands 64 × 65,536 bytes (4 MiB) to a writer task that writes
   them into the stream as fast as flow control allows. The data is
   generated before connecting, so the writer outruns the network.
3. Server → client datagrams pass through a relay that models a small
   receive buffer: B bytes, drained one datagram per 30 µs, overflow
   dropped. This stands in for a receiver with a small `SO_RCVBUF` (quinn
   does not set socket buffer sizes).
4. Right after the last chunk is handed over, while quinn's send buffer
   still holds unsent data and data is in flight, the server calls
   `connection.close(VarInt::from_u32(9), b"")`.
5. Record what the client's `connection.closed().await` returns.

Expected: `ApplicationClosed` with code 9, every time. Observed: mostly
`Reset`.

## Measurements

Each cell is `ApplicationClosed {9}` / `Reset` over the runs shown.

| Server send window / client stream, connection windows | B = 64 KiB | B = 32 KiB | Bytes in flight at close (B = 64 KiB) |
|---|---|---|---|
| 1,048,576 / 1,048,576, 2,097,152 | **1 / 33** (34 runs) | 0 / 10 | 132–289 KB |
| 65,540 / 1,048,576, 2,097,152 | 32 / 2 (34 runs) | 3 / 7 | 24–67 KB |
| 1,048,576 / 65,540, 65,540 | 34 / 0 (34 runs) | 6 / 4 | 13–47 KB |
| both of the above | 20 / 0 | 4 / 6 | — |
| 1,048,576 / 262,144, 262,144 | 4 / 16 | 0 / 10 | — |
| 1,048,576 / 1,048,576, 2,097,152, lossless relay (B = 16 MiB) | 9 / 1 | — | 180–938 KB |

- In every `Reset` run the client received **no** `CONNECTION_CLOSE`,
  although the server received 12–37 datagrams from the client after
  calling `close`.
- The `Reset` arrived 87–93 ms after `close` (219 ms in the lossless run,
  10 ms RTT), which matches the 3 × PTO close timer.
- In every `ApplicationClosed {9}` run, exactly one `CONNECTION_CLOSE`
  arrived, 0.8–2.5 ms after `close`.
- Smaller windows reduce the exposure only because they keep
  `bytes_in_flight` below `cwnd`; once losses shrink `cwnd` (the 32 KiB
  model) the failure returns.

## One-line patch (evidence)

Applied to a local copy of quinn-proto 0.11.19, in `poll_transmit`,
directly after `ack_eliciting` is computed:

```diff
             if space_id == SpaceId::Data {
                 ack_eliciting |= self.can_send_1rtt(frame_space_1rtt);
             }
+            if close {
+                ack_eliciting = false;
+            }
```

Results with the patch:

| Configuration (as above) | B = 64 KiB, unpatched | B = 64 KiB, patched |
|---|---|---|
| 1,048,576 / 1,048,576, 2,097,152 | 1 / 33 | **10 / 0** |
| 65,540 / 1,048,576, 2,097,152 | 32 / 2 | 10 / 0 |
| 1,048,576 / 65,540, 65,540 | 34 / 0 | 10 / 0 |

We have not checked the patch for side effects beyond our suite (for
example on pacing, on padding of the close packet, or on close packets
coalesced with handshake-space packets), so it is offered as a pointer to
the cause rather than as the fix. An alternative would be to compute
`ack_eliciting` from the frames actually written into the closing packet.

## Impact

An application that closes a busy connection with an error code (for
example, to fence a slow or superseded client) cannot rely on the peer
learning that code: under a full congestion window the peer sees a
stateless reset instead. The window for this is exactly the case where
closing is most likely: a sender with a lot of queued data.

## Agent log

- 2026-10-05 (UTC) · claude-code + T023c · Drafted the upstream quinn issue from the T023c C§1.4 evidence (reproduction shape, measurements, the code path and the one-line evidence patch) for the user to file, as the user decided in T023c Q6; nothing was filed.
