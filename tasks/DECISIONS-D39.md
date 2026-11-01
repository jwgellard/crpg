# Decision D39

Status: **Answered 2026-10-30 (the user, session 24)**. The four exact
contracts proposed on `mgr/session-24-batch` ([T139](T139.md),
[T024q](T024q.md), [T024h](T024h.md), [T025a](T025a.md)) are approved,
conditional on one Claude `reviewer` pass per contract. T139 and T024q are
first revised as below; the user is asked again only if a review raises a
point that needs their decision.

## D39a — T139 (persistent creature state; join and leave)

**Answered 2026-10-30: approved with changes.**

- **OQ1, who becomes `CreatureState`:** changed from the contract's
  registered-only (a). Survivors **persist by default**: a placed creature
  that leaves or outlives an encounter keeps its state, and the dead stay
  dead and never respawn. Starting an encounter enters a placement from its
  existing `CreatureState` instead of spawning it fresh. The ruleset can
  change the default, and a per-encounter `respawns` flag marks repeatable
  fights. The four goldens this moves (`history_v1.*`, `surface_v1.*`) are
  approved for re-baselining.
- **OQ3, TB round-0 joiner whose slot has passed:** waits for the rollover
  **by default**, and the ruleset can choose "acts right after the current
  head" (`pending_joins`).
- **OQ5, effects outside encounters:** suspended **by default**, and the
  ruleset can choose strip-always or strip-at-release.
- **Rule hooks:** add the optional ruleset fields (and the encounter
  `respawns` field) in `crpg-data` now and implement every variant in this
  batch. They are additive optional fields at the current versions (D32c), so
  there is no version bump.
- **OQ2** (a) `start_encounter_with_members`; **OQ4** (a) reject with
  `EncounterTerminal`; **OQ6** (a) `Fixed` only; **OQ7** (a) stay at history
  version 2; **OQ8** (a) no join/leave golden until T141; **OQ9** (a)
  T139a → T139b → T139c plus companions, with the new `crpg-data` package
  added.

## D39b — T024q (`crpg-lane0/2` channel selector)

**Answered 2026-10-30: approved, amended per D39d Q2.**

Q1 opt-in `bind_dual`; Q2 channel byte in the existing 4-byte header, with
a third channel `0x91` and a resume-first connection added per T025a Q2;
Q3 `ChannelSendError` and `FrameError::UnknownChannel` only; Q4 new
ADR-0030.

## D39c — T024h (host snapshot sender, seamless narrowing)

**Answered 2026-10-30: every question (a), the recommendation.**

Q1 T024h1 now, T024h2 after T024q; Q2 opt-in `bind_snapshot_peer`; Q3 the
additive S§5/S§14 surface, new ADR-0031 and a signed ADR-0026 addendum;
Q4 atomic, immediate staging, 8 segments per chunk, log cleared on
narrowing and `Resync`; Q5 snapshot only on narrowing and `Resync`, always
cancel first, no `Resync` dedupe; Q6 two match arms in
`tests/host_quic.rs`'s `replay`, no assertion changes; Q7 no new overflow
mechanism; Q8 `0x82` stays transfer version 2 (resume takes `0x91`, D39d).

## D39d — T025a (N-RECONNECT resync mechanisms)

**Answered 2026-10-30: every question (a), the recommendation.**

Q1 resume subprotocol `0x91`, lane 3; Q2 amend T024q before its approval to
carry `0x91` (no `crpg-lane0/3`); Q3 `ResumeProof` 16..=128 bytes, redacted
`Debug`, `ct_eq`, delivered by `Ticket`; Q4 epoch kept plus per-session
`ConnectionFence`, window `[confirmed, current]`, grants never re-issued;
Q5 advisory `receipted_through` in, authoritative `finalized_through` out,
per-lane lists; Q6 `Denied`/`Busy` only; Q7 additive
`ReassemblyPool::rebind_receiver`; Q8 T025a1 then T025a2, T025a1 before the
amended T024q; Q9 new ADR (likely 0032).

## Agent log
- 2026-10-30 (UTC) · claude-code/opus-5.5 + D39 answered · Recorded the user's answers to the 30 open questions in the T139, T024q, T024h and T025a contracts: all approved subject to review, with T139 changed to survivor persistence and ruleset-chosen effect and joiner policies, and T024q amended for the `0x91` resume channel.
