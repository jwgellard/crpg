# crpg-net-quic — agent contract

Read the root rules, [POST-T018-RULES](../../tasks/POST-T018-RULES.md), and
the [T023 re-contract](../../tasks/T023.md#re-contract--2026-10-04-utc-the-crpg-net-quic-crate)
before editing. That section is the exact API and behaviour, with E§3–E§8
carried over and amended by "Decisions on the revised questions".
Architecture: [crpg-net-quic](../../docs/architecture/crpg-net-quic.md).
Reasoning: [ADR-0024](../../docs/adr/0024-separate-crpg-net-quic-crate.md)
(placement) and
[ADR-0025](../../docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md) (wire
and trust).

## Public surface

The crate root re-exports, explicitly with no glob, exactly the R4 / E§3
items:
- `QuicServer`, `ServerLane`, `ServerEvent`, `ConnectionId`, `ServerError`,
  `BindError`, `QueueTotals`, `ShutdownReport`;
- `QuicClient`, `ConnectError`, `CloseReport`;
- the handshake constants and functions, `Credential`, `Hello`, `Welcome`,
  and their errors;
- `CertificatePin`, `ServerIdentity`;
- the limits, the configs and `ConfigError`. Since
  [T023c](../../tasks/T023c.md), `ServerLimits` and `ClientLimits` each
  carry the three QUIC window fields `stream_window_bytes`,
  `connection_window_bytes` and `send_window_bytes` (`u32`, v1 = T023's
  fixed values, floors 4,100 / 65,540 by direction, connection ≥ stream);
- `CloseCode`, `CloseMode`, `CloseReason`, `SendError`, `ConnectionStats`.

The submodules are private. Changing a public item, a wire byte, a cap, a
close code, an ALPN or a `Display` text (`<VariantName> at quic/<area>`)
needs an ADR. A wire change also needs a new ALPN.

## Invariants

1. **I/O is allowed here, crate-wide.** Sockets, threads, `std::time`,
   `std::sync`, tokio, quinn and rustls belong in this crate and nowhere
   below it. `crpg-net` stays I/O-free.
2. **The host side stays synchronous.** Every public call is non-blocking,
   except `wait`, `connect`, `close`, `shutdown` and `Drop`, and reaches the
   I/O thread only through the shared state and its wakeups. No tokio,
   quinn or rustls type appears in a public signature.
3. **Never touch simulation state.** No `World`, RNG, event queue or sim
   clock. quinn's and ring's randomness never reaches the host.
4. **Bytes are opaque.** Lane payloads are never decoded here. The caps are
   checked on the length prefix before any allocation.
5. **Every queue is bounded** in frames and bytes, per connection and
   host-wide. `QueueFull` is retryable and changes nothing. A full inbound
   queue parks one frame and stops reading, and nothing is dropped.
6. **Ordering.** Within a connection and a direction, frames are delivered
   exactly once, in `try_send` order. Across connections there is no order.
7. **Closes are recorded first-wins, then published.** A close becomes
   visible through the `Closed` event and `try_recv`'s error only after the
   reader has stopped adding frames. Frames QUIC already received stay
   drainable after any close.
8. **One endpoint, one thread, one current-thread runtime.** Nothing is
   global, and no process-wide crypto provider is installed.

## Allowed dependencies

- `crpg-net`, a path edge and the only workspace edge (`ALLOWED` row
  `{"crpg-net"}`, ADR-0024).
- The four D23 pins, with default features off:
  - `quinn` =0.11.12 (`runtime-tokio`, `rustls-ring`);
  - `quinn-proto` =0.11.19 (`rustls-ring`) (vendored with patches,
    `third_party/quinn-proto/VENDOR.md`);
  - `rustls` =0.23.45 (`ring`, `std`);
  - `tokio` =1.53.1 (`rt`, `net`, `time`, `sync`).

There are **no dev-dependencies**. The tests use std, the normal edges above
and `crpg_net`'s public codecs. Any new edge or feature needs a T018a-style
record and approval. `deny.toml` changes follow ADR-0023 and its addendum.

## Definition of done for any change

```text
cargo fmt --all -- --check
cargo clippy -p crpg-net-quic --all-targets --locked -- -D warnings
cargo test -p crpg-net-quic --test quic --locked   # 30 cases (T023: 1–23, T023c: 24–27, T023v: 28, T023d: 29–30)
cargo test -p crpg-net --locked
cargo tree -p crpg-net -e normal,build --target all   # must name no tokio/quinn/rustls/ring/mio/socket2
cargo test --workspace --locked
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
cargo deny check
git diff --check
```

The scope grep must print only the `#![forbid(unsafe_code)]` line:

```text
rg -n "HashMap|HashSet|f32|f64|unsafe|cfg\(target_os|cfg\(windows|println|eprintln|tracing|log::|tokio::main" crates/crpg-net-quic/src
```

Run on native Windows/MSVC (CI) and genuine Linux/GNU.

## Known traps

- Never hold the state mutex across an `.await`. The guard is `!Send`, so
  `tokio::spawn` refuses the future; do not work around that.
- Never use an unbounded channel or an unbounded queue.
- Never decode lane payloads in this crate.
- Never log, `Display` or put a credential in a close reason. Its `Debug` is
  `Credential(<redacted>)`, and `crpg-net-quic` emits no logs.
- Never enable DATAGRAM frames or more streams without a new ALPN.
- Never add a platform verifier, webpki roots or `rustls-native-certs`.
- Never use `#[tokio::main]` or any tokio macro. `race` in `queue.rs` is the
  hand-rolled select.
- Keep the DER fixture keys test-only (`tests/fixtures/`, marked binary in
  `.gitattributes`). They are not operator identities.
- Never add a `crpg-sim` or `crpg-data` edge, even dev-only (ADR-0024).
- Never re-export `crpg-net` items. Consumers name `crpg_net::sim::QueueCaps`
  and `crpg_net::transport::Transport` themselves.
- A parked reader must keep waiting after a close. Exiting drops frames QUIC
  already acknowledged, and a peer's `Flush` close would then lose data.
- Record a local close (`record_close`) before calling quinn's `close`.
  Otherwise the monitor sees `LocallyClosed` with no code.
- Never remove a record directly once the API has named it; call
  `State::release`, because the control task may still need the record to
  send a close (T023d).
- Test waits are 30 s failure guards, never oracles. The relay paces with
  read timeouts, not sleeps.
- An exact stall frame count needs the stalled side to send no window
  update: `21 + 2F < W / 8` toward a client (22 toward a server), with `F`
  the framed size and `W` the smaller of its stream and connection windows
  (T023c C§1.3). Otherwise the count depends on read chunking; assert only
  `≥` and the stall.
- A server cannot bound what a stalled client absorbs. Only the client's
  own receive windows do; the server's `send_window_bytes` bounds its own
  memory and bytes in flight, not the peer.
- quinn-proto is vendored with upstream's close fix (T023v). A `Reset` in
  case 17 or 28 now means the vendoring regressed: check
  `[patch.crates-io]`, `Cargo.lock` and VENDOR.md. Never loosen the
  assertion.

## Agent log

- 2026-10-04 (UTC) · claude-code + T023 · Opened the crate contract with the transport's surface, the crate-wide I/O permission against `crpg-net`'s ban, the four D23 pins and no dev-deps, and the E§14 traps plus the no-sim-edge and no-re-export rules, so later lane tasks work inside the same bounds.
- 2026-10-05 (UTC) · claude-code + T023c · Listed the six window fields in the public surface, the 27-case suite count and three traps (the exact-count no-update condition, only a client bounds what it absorbs, quinn's close gating), so later work tightens windows knowingly and never answers a `Reset` by loosening an assertion.
- 2026-10-05 (UTC) · claude-code + T023v-b · Marked the quinn-proto pin as vendored, raised the suite count to 28 and replaced the close-gating trap with the regression check, so a future `Reset` in case 17 or 28 is traced to the vendoring instead of answered by loosening an assertion.
- 2026-10-06 (UTC) · claude-code + T023d · Added the release-not-remove trap and raised the suite count to 30, so later work never deletes a record its control task still needs to send a close.
