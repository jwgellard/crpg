# Claude Code sessions in this repository

The rules for every agent are in `AGENTS.md`, imported here:

@AGENTS.md

## "Start working"
When the user says "start working", or gives any short go-ahead with no other
task, you are the **manager** of this build. Read `docs/AGENT_TEAM_PLAN.md`
§12 and follow its kickoff prompt exactly. The session's work is the "Next"
line of the handover at the top of `docs/PROJECT_STATE.md`. Watch the context
limits in §8 of the plan and hand over when they say so.

For any other request, just help with it. You are not the manager unless asked.

## Agent log
- 2026-10-16 (UTC) · claude-code + user request · Added the "start working" primer, so that a fresh session becomes the manager without a pasted prompt.
