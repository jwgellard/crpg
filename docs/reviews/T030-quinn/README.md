# T030 — quinn dedup-window patch and dependency-preparation dossier

Status: **investigation complete on Linux/GNU; T023 integration remains
blocked.** Dependency approval: **pending** (no approval has been received).
Date: 2026-09-30 (UTC). Owner: dependency-preparation evidence (no workspace
crate, manifest, lock, deny policy or workflow was changed).

## Summary

| Question | Answer |
|---|---|
| Release selected | `quinn` 0.11.12 + `quinn-proto` 0.11.19 (newest published pair; MSRV 1.85, pinned toolchain 1.98.0) |
| Source verified | Registry index checksum = independently computed SHA-256 for both archives; tags peel to the archives' `.cargo_vcs_info.json` commits; audited files equal the tagged git tree |
| Window defect (unit level) | **Reproduced** on unpatched source: 7 independent-model `target_` tests fail (ages 129–2048 wrongly treated as duplicates, missing-packet queries clamp at 128); all `control_` tests pass |
| Patch | 2048 history bits (`[u64; 32]`) plus the separately stored highest packet → **effective window 2049**; every width-dependent operation rewritten, not a typedef swap |
| Patched results | Harness 17/17; upstream `quinn-proto` suite 325/325 + 3 doctests; `quinn` crate suite identical to unpatched (19 pass, 3 IPv6-unavailable failures on both) |
| Memory | `Dedup` 32 → 264 bytes (layout); 3 packet spaces per connection → **+696 bytes per connection**, no heap allocation |
| End-to-end reproduction (ADR-0004 conditions) | **Did not reproduce.** 0 dedup discards in all 26 real QUIC runs (13 per build) of the 39-run matrix; QUIC reorder depth at the relay ≤ 29 packets. Observed QUIC datagram shortfall comes from sender-side loss detection / congestion response to reordering (#2711-class), which the patch does not change |
| Policy audit (unchanged `deny.toml`) | advisories ok (with rustls 0.23.45), licenses ok, sources ok; **bans FAIL**: `windows-sys` 0.52 (via `ring` 0.17.14) vs 0.61 (via tokio/mio/socket2/quinn-udp). Same with the aws-lc-rs provider. **Blocker** |
| Native Windows/MSVC | **Not run** — no Windows host in this environment. **Blocker** for the native gate |

## 1. Inputs and source selection

Read: ADR-0004, `tasks/POST-T018-DECISIONS.md` D04, `tasks/POST-T018-QUINN.md`.
The historical 0.11.11/0.11.15 versions and the 2026-09-27 upstream revision
were treated as evidence to recheck. Selection used the crates.io sparse
index (`https://index.crates.io/qu/in/{quinn,quinn-proto}`): newest
non-yanked `quinn` is 0.11.12 and it resolves `quinn-proto` 0.11.19, the
newest non-yanked `quinn-proto`. Both declare `rust-version = 1.85`.

Exact identities (full data in [`source-manifest.json`](source-manifest.json)):

| Package | Archive URL | SHA-256 (independent) = index `cksum` | Tag → peeled commit = `.cargo_vcs_info.json` |
|---|---|---|---|
| quinn-proto 0.11.19 | `https://static.crates.io/crates/quinn-proto/quinn-proto-0.11.19.crate` | `0e750cca55fe4f0439a15d0bb529da9651e79993e8e72c61a899a36d462befbe` | `quinn-proto-0.11.19` (`983a6096…`) → `8192ed399a26a6e8f0aba43d10ec8badaa5b905e` |
| quinn 0.11.12 | `https://static.crates.io/crates/quinn/quinn-0.11.12.crate` | `4051e23e9185c255a7e33ef59cdbca87a22d359052eecd22fc6b901fb37d9d11` | `quinn-0.11.12` (`54c50a08…`) → `eaec0db4bcb698f76df736d89938743c88a2ad5f` |

`git fetch --depth 1 https://github.com/quinn-rs/quinn 8192ed39…` confirmed
`quinn-proto/src/connection/spaces.rs` and `…/mod.rs` equal the archive
byte-for-byte. The repo's `quinn-proto/LICENSE-{MIT,APACHE}` are symlinks to
the root licence files, which equal the archive's `LICENSE-MIT`
(`4b2d0aca…`) and `LICENSE-APACHE` (`c71d239d…`). Licence: MIT OR Apache-2.0.

Commands (Linux, from an empty scratch directory outside the repo):

```sh
curl -sSf https://index.crates.io/qu/in/quinn-proto | grep '"vers":"0.11.19"'
curl -sSfL -o quinn-proto-0.11.19.crate https://static.crates.io/crates/quinn-proto/quinn-proto-0.11.19.crate
sha256sum quinn-proto-0.11.19.crate
git ls-remote https://github.com/quinn-rs/quinn 'refs/tags/quinn-proto-0.11.19*'
tar xzf quinn-proto-0.11.19.crate && cat quinn-proto-0.11.19/.cargo_vcs_info.json
```

## 2. Audit of the packet window (quinn-proto 0.11.19)

All width-dependent logic is in `Dedup`, `src/connection/spaces.rs`:

- `type Window = u128`, `WINDOW_SIZE = 1 + 128 = 129` (highest packet stored
  separately via `next`).
- `insert`: right-of-window `((window << 1) | 1).checked_shl(diff)`
  (shift-by-word-or-more → 0); within-window `1 << bit`; left-of-window →
  "duplicate". Also a "virtual packet −1" bit on the first insert, which the
  patch preserves.
- `smallest_missing_in_interval`: `BITFIELD_SIZE = 128`, a `u128::MAX` special
  case at full width, `((1u128 << len) - 1) << start` masks (high bits fall off
  the integer), and `128 - leading_zeros` for the oldest gap.
- `missing_in_interval` wraps it.

Consumers: `connection/mod.rs` packet receipt (`dedup.insert(n)` → "discarding
possible duplicate packet", line 2340) and the immediate-ACK / reordering
logic `PendingAcks::is_out_of_order` (RFC 9000 §13.2.1 and the ACK-frequency
draft), which calls both missing-packet queries. No congestion-control,
ACK-frequency or #2711 policy was changed.

## 3. The patch

[`dedup-window.patch`](dedup-window.patch) (`patch -p1` at the archive root;
touches only `src/connection/spaces.rs`, `a1b44aa9…` → `2274f18c…`):

- `Window = [u64; 32]` (2048 history bits, bit `i` = packet `highest − 1 − i`),
  `WINDOW_BITS = 2048`, `WINDOW_SIZE = 2049`.
- `insert` keeps the exact upstream semantics via a multi-word
  `shift_older(diff + 1)` plus setting history bit `diff` (the old highest),
  zeroing everything when the shift reaches the width.
- `smallest_missing_in_interval` keeps the upstream offset arithmetic and
  clamps the range at the window; the oldest gap is found word-by-word from
  the high end (`highest_gap`).
- Three upstream unit tests that asserted on the `u128` bit pattern (`sanity`,
  `jump`) or on 129-specific numbers (`dedup_smallest_missing`) were rewritten
  width-generically; the rewritten `dedup_smallest_missing` produces the
  original 170/172/300/500 → 372 values when `WINDOW_SIZE == 129`.
- Unused `cmp` import removed.

Replay (verified; hashes equal the tested trees):

```sh
tar xzf quinn-proto-0.11.19.crate && cd quinn-proto-0.11.19
sha256sum src/connection/spaces.rs   # a1b44aa9c1ff38a545741fff5546552b9650ae9381a6fdf5f9ef9ea62ef6a45c
patch -p1 --dry-run < dedup-window.patch && patch -p1 < dedup-window.patch
sha256sum src/connection/spaces.rs   # 2274f18c6f6d0f2855ba7fb9699427158e35561a69046b4bc532f732c79f9cc5
patch -p1 < harness.patch                            # optional: test harness
patch -p1 -R < harness.patch && patch -p1 -R < dedup-window.patch   # restores a1b44aa9…
```

## 4. Independent regression harness

[`harness/dedup_reference.rs`](harness/dedup_reference.rs), attached by
[`harness.patch`](harness.patch) (a 4-line `#[cfg(test)]` module hook plus the
file), is applied identically to both trees. Its oracle is a `BTreeSet` of
received packets with the documented window contract — no bit arithmetic.
Cases: empty/first packet, identical duplicate, in-window reordering, ages
127/128/129 and 2047/2048/2049, advancement by 0/1/63/64/65/127/128/129/
2047/2048/2049/2050/5000, packet numbers at `2^62 − 1`, wholly/partially
missing, full and disjoint ranges across words and past bit 128, duplicates
after eviction, and fixed-seed SplitMix64 schedules (seeds 1, 42, 0xC0FFEE,
2710; reordering up to ~3000) plus shallow-reorder controls (seeds 7, 99,
2711). No sleeps; deterministic.

```sh
RUSTUP_TOOLCHAIN=1.98.0 cargo test --locked --lib dedup -- --nocapture   # in each tree
```

| Tree | Result | Evidence |
|---|---|---|
| unpatched + harness | 10 passed, **7 failed** (every `target_` test) | [`results/linux-unpatched-harness.txt`](results/linux-unpatched-harness.txt) |
| patched + harness | **17 passed** | [`results/linux-patched-harness.txt`](results/linux-patched-harness.txt) |

Retained failing assertions on unpatched source, for example:
`insert(4871) with highest Some(5000): implementation says duplicate=true,
model says false` (age 129), `insert(7953) with highest Some(10000)` (age
2047), `insert(4611686018427387774) with highest Some(4611686018427387903)`
(age 129 at the packet-number limit), and `smallest_missing_in_interval(252,
600) with highest Some(2300)`: implementation `None`, model `Some(401)`.

## 5. Upstream suites (Linux/GNU, rustc 1.98.0)

| Suite | Unpatched | Patched |
|---|---|---|
| `quinn-proto` 0.11.19 `cargo test --locked` (lib + doctests), harness attached | 318 passed, 7 failed (only the new `target_` tests) | **325 passed + 3 doctests** |
| `quinn` 0.11.12 `cargo test --no-fail-fast` with `--config patch.crates-io.quinn-proto.path=…` | 19 passed, 3 failed, `many_connections` 0/1 ignored, doctest 1 passed | identical |

The three `quinn` failures (`echo_v6`, `echo_dualstack`, `local_addr`) are
`Os { code: 97, "Address family not supported by protocol" }`: this container
has no IPv6. They fail identically on both trees and are environmental, not a
patch effect. Logs: `results/linux-*-upstream-suite.txt`,
`results/linux-*-quinn-crate-suite.txt`.

## 6. Real UDP/QUIC loopback impairment measurements

Harness: [`probe/`](probe/) (isolated crate with its own `[workspace]`, exact
pins, [`probe/Cargo.lock`](probe/Cargo.lock) resolved from the registry only)
and [`probe/run-matrix.sh`](probe/run-matrix.sh). A tokio UDP relay sits
between a real `quinn` client and server on `127.0.0.1` and applies, per
direction, a SplitMix64-scheduled impairment: `clean` (none), `reorder`
(75 ms ± 15 ms one-way, i.e. 150 ms nominal RTT ± 15 ms jitter), `impaired`
(`reorder` + 3% scheduled loss). The raw-UDP control sends the same schedule
through the same relay without QUIC.

Parameters: payload 1000 bytes (one DATAGRAM per packet), rates 30/300/3000
datagrams/s, 20 s (10 s at 3000/s), seed 1; `impaired` repeated with seeds 2
and 3 at 30 and 300/s; single connection, one packet-number space in use
after the handshake; `datagram_receive_buffer_size` 64 MiB,
`datagram_send_buffer_size` 16 MiB; stats sampled 2 s after the last send.
Builds: `--release`; unpatched = registry `quinn-proto` with `--locked`;
patched = same lockfile with `--config patch.crates-io.quinn-proto.path=<fresh
archive + dedup-window.patch>`.

Attribution per run: *injected* (relay's scheduled drops); *socket/kernel*
(relay-forwarded client→server datagrams minus the server connection's
`udp_rx.datagrams`, which excludes the pre-connection handshake datagram(s) —
hence a constant 1–3); *window accounting* (count of `quinn_proto` "discarding
possible duplicate packet" events via an in-process tracing subscriber, both
endpoints); *application progress* (unique sequence numbers the server app
read).

Full table: [`results/linux-impairment-summary.md`](results/linux-impairment-summary.md)
(raw lines: [`results/linux-impairment-matrix.jsonl`](results/linux-impairment-matrix.jsonl)).

Findings (observations with provenance, not deterministic goldens):

1. **Dedup discards: 0 in every run, both builds**, including all `reorder`
   and `impaired` runs. The deepest reordering the relay produced for QUIC
   traffic was 29 packets, far below 129, so the 129-packet window was never
   reached; the unit-level defect is real but did not manifest end-to-end here.
2. At 30 datagrams/s (the ADR-0004 spike's rate) application loss never
   exceeded the relay's injected drops: QUIC received 584/600 (seed 1, 16
   injected), 588–589/600 (seed 2, 12) and 574/600 (seed 3, 28) on both
   builds, against 584, 588 and 572 for the raw-UDP control. Differences of
   one or two arise because the relay's drop schedule also hits QUIC's
   handshake and ACK-only packets, not only datagram-carrying ones.
3. At ≥ 300/s under `reorder`/`impaired`, QUIC transmitted only a fraction of
   the offered datagrams by the sampling point, while declaring dozens of
   packets lost with **zero** injected loss in `reorder`. The shortfall is on
   the sender (loss detection treating reordering as loss, then congestion
   response), the #2711-class interaction D04 said to keep separate. The patch
   does not change it (unpatched and patched are within run-to-run variation).
4. Raw UDP through the relay received exactly `sent − scheduled drops` in all
   13 control runs, with 0 socket-level loss (measured, reported per run).
5. The relay itself reorders at high rates even under `clean` (one task per
   datagram and timer granularity); this is a harness property and is why the
   `clean` rows show inversions at 3000/s.

Interpretation: ADR-0004's "~26–30% loss with zero packets dropped" is better
explained, on this evidence, by sender-side loss/congestion behaviour under
jitter than by the receive dedup window. The window patch removes a real
correctness limit (reordering deeper than 128 packet numbers, reachable at
higher packet rates or larger jitter), but it is **not** a demonstrated fix for
the spike's symptom. No latency or throughput promise is made.

## 7. Memory

Layout counts from `size_of`/`align_of` in the harness (no heap allocation in
either version): `Dedup` 32 bytes (align 16) → 264 bytes (align 8), +232 bytes
per packet space. A connection holds `spaces: [PacketSpace; 3]` (Initial,
Handshake, Data), so **+696 bytes per connection**; for `N` connections the
added resident state is `696 × N` bytes (≈ 0.68 MiB at 1024 connections). The
issue's reported byte figures were not reused.

## 8. Dependency dossier and policy audit

Proposed product edges (probe manifest; exact pins, default features off):

| Package | Version | Features | Role |
|---|---|---|---|
| quinn | =0.11.12 | runtime-tokio, rustls-ring | QUIC endpoint API |
| quinn-proto | =0.11.19 | rustls-ring | protocol state machine (patched, vendored) |
| tokio | =1.53.1 | rt-multi-thread, net, time, macros, sync | async runtime |
| rustls | =0.23.45 | ring, std | TLS 1.3; `ring` crypto provider |

TLS/certificate verification: no `platform-verifier` feature; the client
trusts an explicitly supplied server certificate/root through rustls/webpki
(credential issuance and storage belong to T022). Secure entropy: `ring`'s
`SystemRandom` and `rand`/`getrandom` as already pulled by `quinn-proto`; no
extra entropy crate. Probe-only (not proposed for product): `rcgen` (in-memory
test certificate; nothing written to disk or committed), `tracing`
(measurement subscriber), `bytes`.

Resolved graph of the proposed product edges (registry only): **42 packages
on `x86_64-unknown-linux-gnu`, 44 on `x86_64-pc-windows-msvc`**, all MIT /
Apache-2.0 / ISC / other permissive expressions inside the allow list; build
scripts in `getrandom`, `libc`, `proc-macro2`, `quinn`, `quinn-udp`, `quote`,
`ring` (C/assembly via `cc`), `rustls`, `thiserror`. Per-target table with
licences, enabled features and `rust-version`:
[`results/dependency-graph.md`](results/dependency-graph.md); lockfile:
[`results/proposed-product-edges.Cargo.lock`](results/proposed-product-edges.Cargo.lock).

`cargo deny check` (cargo-deny 0.20.2) with the repository's **unchanged**
`deny.toml`:

| Graph | advisories | bans | licenses | sources |
|---|---|---|---|---|
| proposed product edges | ok | **FAILED** | ok | ok |
| probe (product + rcgen/tracing/bytes) | ok | **FAILED** | ok | ok |
| aws-lc-rs provider variant | ok | **FAILED** | ok | ok |
| first attempt with rustls 0.23.40 (quinn's own lock) | **FAILED** (RUSTSEC-2026-0285, fixed ≥ 0.23.45) | FAILED | ok | ok |

The `bans` failure in every configuration is the duplicate `windows-sys`:
0.52.0 via `ring` 0.17.14 (newest `ring`, requires `^0.52`; pulled by
`quinn-proto` even when aws-lc-rs is selected) and 0.61.2 via `tokio`, `mio`,
`socket2`, `quinn-udp`. Logs: `results/linux-cargo-deny-*.txt`. Under the
T030 rules this is a **reported blocker, not a reason to widen the policy**.
Options for a separate human decision: (a) a scoped, documented
`skip = ["windows-sys@0.52.0"]` for a target-specific transitive duplicate,
reviewed like the existing ADR-0006 skips; (b) wait for a `ring` release on
`windows-sys` 0.61; (c) choose older tokio/mio/socket2 aligned on 0.52 (not
recommended: forgoes current fixes). Integrating into the workspace would
also merge with the existing graph (e.g. proptest's `getrandom` skips); that
combined check must be rerun at integration time.

Proposed integration mechanism, as a reviewed but unapplied diff:
[`proposed-integration.diff`](proposed-integration.diff) — workspace pins, a
`[patch.crates-io]` path entry and a checksum-pinned `third_party/quinn-proto`
vendor directory with its re-derivation command. No git source; `deny.toml`
unchanged.

## 9. Maintenance

- Role: **project dependency maintainer** owns the vendored patch.
- Upgrade procedure: select the new published release pair; re-verify archive
  checksums, tags and licences; re-apply or re-author the patch; re-run the
  harness (unpatched must still fail the `target_` tests, or the patch is
  retired), both upstream suites, the impairment matrix and `cargo deny check`
  on native Windows/MSVC and Linux/GNU; update `source-manifest.json`.
- Security advisories against quinn/quinn-proto/rustls/ring/tokio: treat as a
  patch-level upgrade with the same evidence set.
- Retirement: remove the vendor directory and `[patch]` entry once a pinned
  upstream release passes this same independent regression and control suite
  unpatched.

## 10. Blockers and open gates

1. **Native Windows/MSVC**: none of the suites, the harness or the impairment
   matrix have been run on Windows. Required before T023 integration.
2. **`cargo deny` bans** under the unchanged policy (the `windows-sys` split):
   needs a separate decision (§8).
3. **Dependency approval**: pending; nothing here is approved.
4. The end-to-end symptom of ADR-0004 was not reproduced; T023 should not
   assume the patch fixes it, and the sender-side reordering behaviour
   (#2711-class) needs its own evidence and decision.

T023 therefore remains blocked. This dossier completes the investigation, not
the integration gate.

## Environment and provenance

Linux container `x86_64-unknown-linux-gnu`, 4 CPUs, kernel 6.18,
`rustc 1.98.0 (88d9e12ae 2026-08-18)` / `cargo 1.98.0` selected with
`RUSTUP_TOOLCHAIN=1.98.0` (the scratch directory is outside the repository,
so `rust-toolchain.toml` does not apply there; an earlier pass accidentally
used the container default 1.94.1 and was **discarded and fully re-run** on
1.98.0 — every result here is from the 1.98.0 run). `cargo-deny 0.20.2`. Logs
have the scratch path replaced by `<t030>` and trailing whitespace stripped;
content is otherwise verbatim. See [`results/linux-environment.txt`](results/linux-environment.txt).

## Agent log

- 2026-09-30 (UTC) · claude-code + T030 dependency preparation · Recorded the selected release, verified provenance, the 2049-packet patch with an independent reference-model reproduction, Linux suite and impairment evidence, and the unchanged-policy audit, reporting the Windows gap and the windows-sys bans failure as blockers instead of widening policy or claiming the spike symptom fixed.
