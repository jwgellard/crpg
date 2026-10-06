# Architecture docs

One doc per crate, named after the crate. Spec §14 lists this directory; spec
§15.6 states the rule it exists to enforce:

> `docs/architecture/` mirrors the crate list one-to-one. If a crate has no
> architecture doc, it is not ready for agent work.

That is a **readiness gate, not a documentation quota**. A crate needs its doc
before an agent is turned loose on it, which is why the index below marks most
crates "due with T0NN" rather than carrying fifteen documents written ahead of
the decisions they would describe. Writing a design doc for a crate whose design
has not been decided produces fiction that the first real task then has to
contradict — the opposite of what the gate is for.

`crpg-core` was worked in T006a without one. That was the actual violation, and
`crpg-core.md` closes it.

## What goes in one, and what does not

Three kinds of document describe a crate, and they answer different questions.
Keeping them apart is what stops them drifting into three copies of each other:

| | Question | Lifetime |
|---|---|---|
| `docs/adr/NNNN-*.md` | **Why** this and not the alternative | Decision text never rewritten: superseded by a new ADR, or corrected by a dated appended note (E007 policy; statuses and template in `docs/adr/0000-template.md`) |
| `docs/architecture/<crate>.md` | **What** the crate is, and how its pieces fit together | Living. Updated when the design changes |
| `crates/<crate>/AGENTS.md` | **How to work on it** without breaking it | Living. The contract an agent reads before editing |

So: an architecture doc explains the shape and cites the ADR for the reasoning.
It does not restate the ADR's argument, and it does not repeat `AGENTS.md`'s
rules. If you find yourself copying either, link instead — a duplicated
invariant is one that will eventually disagree with itself.

An architecture doc should cover: what the crate is for and where it sits, what
exists today versus what is planned, how the modules relate, the decisions that
govern it (as links), and what consumers inherit from it.

## Index

| Crate | Doc | Governed by |
|---|---|---|
| `crpg-core` | [crpg-core.md](crpg-core.md) | ADR-0006, ADR-0007 |
| `crpg-data` | [crpg-data.md](crpg-data.md) | spec §4, E016, ADR-0008/0011 |
| `crpg-rules` | [crpg-rules.md](crpg-rules.md) | spec §3, §15.1 |
| `crpg-sim` | [crpg-sim.md](crpg-sim.md) | spec §2.4 |
| `crpg-nav` | due with its first task | spec §6.3 |
| `crpg-script` | [crpg-script.md](crpg-script.md) | spec §5, D17, ADR-0005 |
| `crpg-ai` | due with its first task | spec §6 |
| `crpg-net` | [crpg-net.md](crpg-net.md) | spec §7, ADR-0004 |
| `crpg-net-quic` | [crpg-net-quic.md](crpg-net-quic.md) | spec §7.2, ADR-0024, D22/D23, ADR-0023 |
| `crpg-persist` | [crpg-persist.md](crpg-persist.md) | spec §8, D10, D27b, T038 |
| `crpg-edit` | [crpg-edit.md](crpg-edit.md) | spec §11, D18, T058 |
| `crpg-contracts` | due with its first task | spec §15.1 (human-owned) |
| `crpg-testkit` | [crpg-testkit.md](crpg-testkit.md) | spec §15.3, §16 |
| `crpg-server` | [crpg-server.md](crpg-server.md) | spec §10, ADR-0022, T039, ADR-0026/T023b |
| `crpg-cli` | [crpg-cli.md](crpg-cli.md) | spec §24 |
| `crpg-godot` | due with its first task | spec §9, ADR-0001, ADR-0003 |

Writing the doc is part of the **first task that puts real code in a crate**,
listed in that task's definition of done alongside the crate's `AGENTS.md`.
Later tasks in the same crate extend it rather than starting a new one.

Agent attribution follows the root `AGENTS.md` rule: every agent edit signs
itself with `YYYY-MM-DD (UTC) · <harness/model> + <task> · 1–2 sentence why`.
Attribution footer/appendix lines are separate from the content they annotate;
they do not change what an ADR's Decision section says. Prior log entries are
superseded by appending, never rewritten.

## Agent log

- 2026-09-05 · opencode/big-pickle + agent-attribution rule · Restated the root attribution rule here so agents working on architecture docs see it without leaving the directory; no change to the readiness gate.
- 2026-09-06 (UTC) · opencode/muse-spark + T007 · Linked the new crpg-sim doc; the §15.6 readiness gate is now satisfied for the second crate.
- 2026-09-06 (UTC) · opencode/muse-spark + T008b · Linked the new crpg-testkit doc; third crate through the gate.
- 2026-09-07 (UTC) · opencode/big-pickle + T009b · Linked the new crpg-cli doc; the replay subcommand is the crate's first real code, so the §15.6 gate is satisfied and the "due with T013" place-holder is retired.
- 2026-09-10 (UTC) · opencode/gpt-6-astra + T010 crate opening · Linked the data architecture document before source implementation. The opening documents describe the approved contract; completion remains gated by T010 verification.
- 2026-09-18 (UTC) · opencode/muse-spark + T010–T012a landing catch-up · Noted the data doc opened at T010 and extended through T011a/T011b validation and the T012a migration framework; the index already links all five live crate docs.
- 2026-09-26 (UTC) · opencode/muse-spark + T014 crate opening · Linked the new crpg-rules doc; the stat/modifier kernel is the crate's first real code, so the §15.6 gate is satisfied and the "due with T014" place-holder is retired.
- 2026-09-28 (UTC) · opencode/muse-spark + T018a crate opening · Linked the new crpg-net doc; the lane-0 protocol v1 plus bounded codec plus local Transport is the crate's first real code, so the §15.6 gate is satisfied and the "due with T018" place-holder is retired.
- 2026-09-30 (UTC) · claude-code + T033 E007 follow-up · Pointed the ADR row at the E007 append-only supersession policy and the ADR template, replacing the stricter "never edited" wording that dated status/evidence appendices already contradicted.
- 2026-10-04 (UTC) · claude-code + T029b · Linked the new crpg-script doc and added D17 to its governing column; the trusted-binding slice is the crate's first real code, so the §15.6 gate is satisfied and the "due with its first task" place-holder is retired.
- 2026-10-04 (UTC) · claude-code + T038 · Linked the new crpg-persist doc and added D10/D27b/T038 to its governing column; the save envelope and file store are the crate's first real code, so the §15.6 gate is satisfied and the "due with its first task" place-holder is retired.
- 2026-10-04 (UTC) · claude-code + T022 · Linked the new crpg-server doc; the T022 host slice is the crate's first real code, so the §15.6 gate is satisfied and the "due with its first task" place-holder is retired.
- 2026-10-04 (UTC) · claude-code + T058 · Linked the new crpg-edit doc and added D18/T058 to its governing column; the headless document, command and undo API is the crate's first real code, so the §15.6 gate is satisfied and the "due with its first task" place-holder is retired.
- 2026-10-04 (UTC) · claude-code + T023s · Added the `crpg-net-quic` index row as due with T023; the T023s stub carries no real code, so the crate's doc and `AGENTS.md` arrive with the first transport source.
- 2026-10-04 (UTC) · claude-code + T023 · Linked the new crpg-net-quic doc; the lane-0 QUIC transport is the crate's first real code, so the §15.6 gate is satisfied and the "due with T023" place-holder is retired.
- 2026-10-05 (UTC) · claude-code + T039 · Added T039 to the `crpg-server` row's governing column, since its contract now governs the crate's save adapter and gate-10 claim.
- 2026-10-06 (UTC) · claude-code + T023b · Added ADR-0026 and T023b to the `crpg-server` row's governing column, since they now govern the crate's QUIC adapter.
