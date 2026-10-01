# Decisions D19–D26 — 2026-09-30 (UTC)

Status: **Approved by the user, 2026-09-30.**

## Authority

After PR #20 merged, the user asked the agent to "use your best judgement on
all the decisions". The agent drafted D19–D26 below with its reasoning and
presented them for explicit approval; the user replied **"Approved."** These
are therefore the user's decisions on the agent's recommendations. They extend,
and in D23 partly supersede, [POST-T018-DECISIONS.md](POST-T018-DECISIONS.md)
D01–D18. They do not waive a task's own exact contract, tests, native gates or
completion record.

Evidence reviewed for these calls: the ADR texts, the T020 golden files and
schedule, CI check runs of PRs #18 and #20 (all nine checks green, including
`check (windows-latest)` running `cargo test --workspace --locked`), and the
T030 dossier (`docs/reviews/T030-quinn/`).

## D19 — ADR acceptance and the T020/T021 review items

- **Accept ADR-0017, 0018, 0019, 0020 and 0022.** 0017/0019 are merged (PR #18)
  and 0018/0020 are merged (PR #20), each with its suites green in CI on
  Windows/MSVC and Linux/GNU. 0022 was reviewed as a contract: it fixes eight
  concrete gaps in 0021 (per-entity disclosure, revocation resync, per-session
  epochs, capture retirement, reserve-before-commit for rejections, receipt
  reproduction, explicit V1/V2 projection). Acceptance of 0022 approves the
  T022 contract; T022's implementation still has to pass its own gates.
- **ADR-0021 is superseded by ADR-0022** (dated note appended; text unchanged).
- **T020 golden review: passed.** `history_v1` goldens for both targets were
  generated independently and are byte-identical, consistent with
  integer-only determinism; the schedule's oracle runs before any hash
  comparison; the equal hashes at steps 14→15 and 16→17 are the two inert
  `read_after` steps, exactly as designed; CI re-verified both targets.
- **T021 gets a retrospective completion record** built from git and CI
  evidence (appended to `tasks/T021.md`).

Why: every one of these designs is now merged with dual-native evidence or
has been reviewed line by line; leaving them "Selected" only keeps the next
tasks blocked on a formality.

## D20 — T022 dependency edges approved

`crpg-server` may take normal path dependencies on `crpg-core`, `crpg-sim` and
`crpg-net`, and workspace `serde` (derive) and `serde_json`, plus a `[lib]`
target beside the thin binary. No crates.io crate is added (all are already in
`Cargo.lock`); `tools/lint/deps.py` already allows these edges. T022's
completion record must carry the T018a-style audit (exact edges, lock impact,
clean `cargo deny check`).

## D21 — T029b dependency edges approved

`crpg-script` may take normal path dependencies on `crpg-core`, `crpg-data`
and `crpg-sim`, and dev-only workspace `serde_json`. No third-party runtime,
no Lua. Same audit obligation in T029b's completion record.

## D22 — the windows-sys duplicate: one scoped skip, applied by T023

Select option (a) from the T030 dossier: when T023 adds quinn, `deny.toml`
gains exactly `skip = [..., "windows-sys@0.52.0"]` with a comment naming its
cause (`ring` 0.17.14 requires `windows-sys ^0.52`; tokio/mio/socket2/quinn-udp
use 0.61). Recorded as [ADR-0023](../docs/adr/0023-windows-sys-duplicate-skip.md).
Conditions: that exact version only; no other policy change; remove the skip
as soon as a `ring` release moves to 0.61 (cargo-deny's unmatched-skip warning
will flag it). The skip is not added before T023 needs it.

Why: `windows-sys` is Microsoft's raw API-bindings crate (MIT OR Apache-2.0);
the duplicate costs Windows compile time and binary size, not correctness or
trust. Waiting has no date and blocks the critical path; downgrading
tokio/mio would forgo current fixes. `deny.toml` treats skip growth as a
reviewed action, which the ADR is.

## D23 — T023 scope and the dedup patch's timing (supersedes D04's timing)

- T023 implements lane-0 **reliable ordered** combat over QUIC streams only.
  No unreliable datagram lane in T023.
- T023 uses the exact pins audited by T030 **without vendoring the dedup
  patch**: `quinn` =0.11.12 (runtime-tokio, rustls-ring), `quinn-proto`
  =0.11.19 (rustls-ring), `tokio` =1.53.1, `rustls` =0.23.45 (ring, std), all
  default features off. These third-party edges are approved for `crpg-net`
  (T023) and `crpg-server` (T023b) at those pins, subject to the T018a-style
  record and a clean `cargo deny check` with only the D22 skip.
- The prepared patch (`docs/reviews/T030-quinn/dedup-window.patch`) stays
  ready. It is applied, by the vendoring mechanism in the dossier, with the
  first unreliable-datagram lane (movement, T026p), or earlier if measurement
  shows dedup discards on a reliable lane. The T030 impairment probe must be
  re-run on native Windows at that point.
- Carried forward from D04: no git source, no fork URL, no `deny.toml`
  widening beyond D22.

Why: T030 found 0 dedup discards in 26 real QUIC runs at ADR-0004's
conditions (maximum reorder depth 29 < 129), and reliable streams retransmit
anything a too-small window drops, so the window affects throughput, not
correctness, for lane 0. The spike's loss came from sender-side
reorder-as-loss behaviour, which the patch does not change and which matters
most for unreliable datagrams. Vendoring now would add maintenance with no
measured benefit.

Remaining T023 gate (engineering, not a decision): its exact endpoint/stream
framing/backpressure/close contract, and the D03 pinned-certificate plus
invitation-credential handshake, written against the T022 host boundary.

## D24 — movement is scheduled after T024

Authoritative movement/nav specification (D13's path format, movement rules,
numeric limits) is scheduled after T024 snapshot transfer. T026p/T026 stay
blocked on it. It is not on the current critical path.

## D25 — close the remaining E-task wording debt (task T033)

The option calls behind E010, E017, E018, E019 and E022 were already made in
D01–D18 (D17, D01+T018, D03, D06, D02 respectively). A documentation-only task,
T033, reconciles the spec wording and closes them, plus E013's README diagram
and E007's `docs/architecture/README.md` pointer. E023 is marked
**deferred** (D07), not done.

## D26 — run the preflight tests in CI on both operating systems

Add `python -m unittest discover -s tools/tests -p "test_preflight.py" -v` to
the existing `lint-selftest` matrix job (ubuntu-latest and windows-latest,
both of which provide `pwsh`). This supplies T031's missing native Windows
evidence and keeps both launchers from regressing. No new action, job or
runner.

## Agent log

- 2026-09-30 (UTC) · claude-code + D19–D26 · Recorded the eight decisions the agent recommended and the user approved ("Approved."), with the evidence each rests on, so later tasks cite a dated authority instead of re-asking.
