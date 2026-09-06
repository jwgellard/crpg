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
| `docs/adr/NNNN-*.md` | **Why** this and not the alternative | Immutable. Superseded by a new ADR, never edited |
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
| `crpg-data` | due with T010 | spec §4 |
| `crpg-rules` | due with T014 | spec §3, §15.1 |
| `crpg-sim` | [crpg-sim.md](crpg-sim.md) | spec §2.4 |
| `crpg-nav` | due with its first task | spec §6.3 |
| `crpg-script` | due with its first task | spec §5, ADR-0005 |
| `crpg-ai` | due with its first task | spec §6 |
| `crpg-net` | due with T018 | spec §7, ADR-0004 |
| `crpg-persist` | due with its first task | spec §8 |
| `crpg-edit` | due with its first task | spec §11 |
| `crpg-contracts` | due with its first task | spec §15.1 (human-owned) |
| `crpg-testkit` | [crpg-testkit.md](crpg-testkit.md) | spec §15.3, §16 |
| `crpg-server` | due with its first task | spec §10 |
| `crpg-cli` | due with T013 | spec §24 |
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
