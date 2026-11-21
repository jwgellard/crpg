# Decision D76 — T155: reconnect for movement sessions and the dedicated movement endpoint

Status: **answered by the user, 2026-11-21: all four as recommended.** Contract `tasks/T155.md` (Opus architect, `fe23faa`).

## Answers
- **D76a (Q1) — (a).** The reconnect answer carries no movement (lane-1) progress. The client resends its unanswered movement orders; the host answers each once (from its 256-entry answer cache for orders already answered, as new otherwise), and every tick's sample already reports `processed_seq`. No `crpg-net` change, no resume version `0x92`. The user asked which is best for the long term; the manager's answer: (a), because lane 1 recovers by its own design and a second source of the same number is what drifts.
- **D76b (Q2) — (a).** A movement connection reopens with the existing resume opening on the same `/3` label, served by a new `bind_movement_resumable` endpoint; `bind_movement` keeps refusing resumes.
- **D76c (Q3) — (a), with a follow-up.** The dedicated server offers movement behind an operator switch, `"movement": true` in its config; without it the binary is exactly T040b's. The user asked why movement is not always on: today's combat-only clients (terminal play, the scripted test clients, T041d's fight smoke) speak only `/1`/`/2`, and one world serves one kind. Follow-up row: make movement the default once those clients speak `/3`.
- **D76d (Q4) — approve.** T155a may change the one assertion at `crates/crpg-server/tests/host_reconnect.rs:1405–1408` (and its comment at `:1399`) from "a v4 session's ticket is `NotResumable`" to "a v4 session takes a ticket"; every other line of that test stays.

## Agent log
- 2026-11-21 (UTC) · claude-code/opus-5.5 (manager) + session 94 · Recorded the user's four T155 answers and the explanations given for Q1 and Q3.
