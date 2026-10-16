# Decision D28 — 2026-10-16 (UTC)

Status: **Decided by the user, 2026-10-16.**

## Authority

`docs/AGENT_TEAM_PLAN.md` §13 put eight questions to the user (D28 a–h), and
the `cargo deny` failure found the same day added a ninth (D28i). The user
answered by choosing options the agent offered, and asked for (c) directly;
their choices are quoted below. These extend D01–D27 and do not waive any
task's exact contract, tests, native gates or completion record. D27a stands:
agents open PRs and never merge.

## D28a — the agent-team plan is the plan of record

User: "Adopt, start A0."

`docs/AGENT_TEAM_PLAN.md` becomes the plan of record for delegation, with
`docs/agents/MODELS.md` as its routing table. Where it disagrees with
`docs/AGENTIC_WORKFLOW_PLAN.md` §§3, 4 and 9 or `docs/IMPLEMENTATION_PLAN.md`
§§1.3–1.4, it wins. Setup wave A0 runs next, after PR #29 merges.

## D28b — the Claude side runs on Pro

User: "Pro."

Claude subagent workers use Sonnet 5.5 (implementer, reviewer) and Haiku 5.5
(bookkeeper). Opus 5.5 is used as a subagent only for A and C packages
(`architect`); there is no Opus implementer rung, so an escalation past
Sonnet 5.5 goes to the manager to split or rewrite the package, or to the
user. OpenCode takes all other implementation volume.

## D28c — disk

User, before the question was put: "Yeah clean up the merged worktree, clean
up everything in this project that doesn't need to be there."

Done 2026-10-16: the merged `crpg-t014` worktree and every merged local
branch, the empty `crpg-t005` folder, the spikes' build folders and the
workspace build cache (`cargo clean`, 10.3 GiB) were removed; the nine merged
remote branches were deleted by the user. 43 GB free; two build slots.

## D28d — Zen balance stays at $0

User: "Zen balance stays $0."

Workers use OpenCode Go and the free models only. Paid Zen models stay
unavailable (402).

## D28e — training-use models may see repository content

User: "Allow training-use models."

Free and contributor models that may retain or train on prompts may see this
public repository's content, never secrets, local paths outside the slot, or
the user's messages (`docs/AGENT_TEAM_PLAN.md` §4.2 rule 4).

## D28f — move the root `AGENTS.md` status note

User: "Move AGENTS.md note."

A0-8's first step is approved: the root `AGENTS.md` "Note" status log moves to
`docs/PROJECT_STATE.md`, leaving a one-line pointer. Trimming the crate
`AGENTS.md` files stays crate by crate, each reviewed.

## D28g — no advisor trial yet

User: "Defer advisor."

Revisit after A0 lands and one milestone has baseline usage numbers.

## D28h — `.github/workflows/opencode.yml` is parked

User: "Park opencode.yml."

The untracked workflow is not committed as is. Reviving it is a reviewed
workflow task that moves it to a free or Go model.

## D28i — `rustls` exact pin moves to 0.23.46

User: "Approve" (bump the exact `rustls` pin in `crpg-net-quic` from =0.23.45
to =0.23.46).

RUSTSEC-2026-0346, -0347, -0348 and -0349 against `rustls` 0.23.45 fail
`cargo deny check` on every PR; 0.23.46 fixes all four. This supersedes the
`rustls` line of D23 only; the other D23 pins are unchanged. The change is
one crate (`crpg-net-quic`) plus `Cargo.lock`, with the QUIC suites re-run on
Windows/MSVC and Linux/GNU, recorded as S002.

## Agent log

- 2026-10-16 (UTC) · claude-code + D28 · Recorded the user's answers to the agent-team plan's questions and the same-day rustls advisory decision, with what each answer changes, so the manager's first session starts from a settled record.
