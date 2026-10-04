# ADR-0024: Real QUIC transport in a separate `crpg-net-quic` crate

Date: 2026-10-04 (UTC)
Status: **Accepted** — the user chose Q12 Option B (a separate crate) on
2026-10-04, recorded in [tasks/T023.md "Decisions — 2026-10-04
(UTC)"](../../tasks/T023.md#decisions--2026-10-04-utc), and approved this
ADR's filing as Accepted through revised question R13 and the
[T023s approval](../../tasks/T023s.md#approval--2026-10-04-utc).

## Context

T023's exact contract (2026-10-04) put the real lane-0 QUIC transport in a
`crpg_net::quic` module, under a scoped exception to `crpg-net`'s rule that
it has no I/O, threads or clocks. A lint can police where that code sits
inside the crate. It cannot change what the crate links: every dependant of
`crpg-net` would still compile and link tokio, quinn and rustls, even a
crate that needs only the byte protocol, the codec or the in-memory
`Transport` fabric.

`tools/lint/deps.py` fails closed on a crate missing from `ALLOWED`, so a new
crate and its row must land together. The row is the enforcement the split
relies on.

## Decision

1. The real QUIC transport lives in a new crate, `crpg-net-quic`, whose
   `ALLOWED` row is `{"crpg-net"}`. It sees only `crpg-net`'s public byte
   protocol, `Transport` trait and queue caps.
2. `crpg-net` keeps its no-I/O, no-thread, no-clock rule with **no
   exception**. Its row stays `{"crpg-core", "crpg-data", "crpg-sim"}`, so it
   can never depend on `crpg-net-quic`.
3. Only crates that depend on `crpg-net-quic` link the QUIC graph. Today the
   only rows that admit it are the `None` rows above it: `crpg-server`,
   `crpg-cli`, `crpg-testkit` and `crpg-godot`.
4. `crpg-core`, `crpg-data`, `crpg-rules`, `crpg-sim` and `crpg-net` may not
   name tokio, quinn, quinn-proto, quinn-udp, rustls, ring, mio or socket2 in
   any manifest section (target tables included, renamed entries resolved).
   `deps.py` enforces this as the `io-free` check (revised question R14).
5. The D23 pins are approved for `crpg-net-quic`, not `crpg-net` (revised
   question R12): `crpg-net` gets none, and `crpg-server` takes no direct QUIC
   edge.

## Authority and affected surface

- Decided by: the user (Q12, R11–R14), recorded in `tasks/T023.md`
  "Decisions — 2026-10-04 (UTC)" and "Decisions on the revised questions —
  2026-10-04 (UTC)".
- Authoritative contract: [tasks/T023s.md](../../tasks/T023s.md) for the stub,
  the `ALLOWED` row and the ban; the
  [T023 re-contract](../../tasks/T023.md#re-contract--2026-10-04-utc-the-crpg-net-quic-crate)
  for the transport's API.
- Owning crate(s)/files: `crates/crpg-net-quic` (stub until T023),
  `tools/lint/deps.py` and `tools/lint/test_deps.py`.

## Compatibility, hash and dependency impact

- Persisted/serialized shapes: unchanged. The wire format stays owned by
  `crpg-net`; ALPN, framing, pinning and close codes are T023's ADR-0025.
- Replay, `state_hash`, goldens: unchanged.
- Dependency graph: one new `ALLOWED` row, `"crpg-net-quic": ({"crpg-net"},
  set())`, and the `io-free` manifest ban. No existing row changes. The stub
  has no dependencies; T023 adds the `crpg-net` edge and the D23 pins under
  its own audit.
- Platform scope (ADR-0012): the crate is platform-neutral and is gated on
  both native targets like every other crate.

## Alternatives considered

- **A `crpg_net::quic` module plus a lint rule** (the earlier exact
  contract). Rejected by the user: the lint keeps the code in one module but
  every `crpg-net` dependant still links tokio, quinn and rustls.
- **A `quic` cargo feature on `crpg-net`.** Either the feature is off by
  default, and ADR-0012's default-features gates never test the QUIC path,
  or it is on, and the graph is the same as the module option.

## Consequences

- `crpg-net` dependants other than `crpg-server` and its consumers stay
  I/O-free, and the dependency graph, not review, is what keeps them so.
- T023b takes a path edge on `crpg-net-quic`, not the QUIC crates.
- Each later lane splits into a `crpg-net` wire task and a `crpg-net-quic`
  transport task.
- Adding `crpg-core` (or anything else) to the `crpg-net-quic` row is a
  reviewed change: the row is a ceiling, kept to what the design needs.

## Acceptance and evidence

not yet run. T023s lands the stub, the row and the ban with lint self-tests
(its completion record); the transport itself is accepted by T023's
completion record.

## Supersession and corrections

## Agent log

- 2026-10-04 (UTC) · claude-code + T023s · Filed the crate-split decision as Accepted on the user's recorded Q12 and R13 answers, so the new `ALLOWED` row and the I/O-crate ban land with their reason.
