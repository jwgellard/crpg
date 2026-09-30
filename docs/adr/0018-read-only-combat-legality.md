# ADR-0018: Shared read-only combat legality

Date: 2026-09-29 (UTC)
Status: Selected under the recorded D15 delegation for specification; implementation
acceptance remains T028's native gates. No independent human review is claimed.

## Context

`perform_action` currently owns pure admission checks and mutation in one
function. UI and AI need options without resolving dice or copying that policy.
T019's same-pool repair must remain the single affordability rule. Sim's public
API contract requires an ADR for this addition.

## Decision

Adopt [T028's specification revision](../../tasks/T028.md#specification-revision--2026-09-29)
as the normative signature/error/ordering contract: additive `validate_action`,
`legal_actions`, `LegalActionsError`, and `MAX_LEGAL_ACTIONS = 4096` in sim.
Execution and queries share the pure pre-mutation validator. Execution always
revalidates; returned actions confer no authorization or prepared-state lease.

Enumerate the existing mandatory-target CombatAction vocabulary exactly, sorted
by full ability ULID then full target EntityId, EndTurn last. `requires_target`
does not silently gain new semantics. Over-limit output fails wholly; read-only
validation changes no RNG, interner, queue, turn or resource state.

## Consequences and alternatives

No persisted fields, dependencies, replay format or hash changes. Legacy
behavior/error precedence is a compatibility gate. A net-side legality engine
and clone-and-execute enumeration are rejected because they duplicate policy or
perform resolution to answer a pure question. This bounded output is not a
real-time complexity guarantee; CPU budgeting for very large authored inputs
would be a separate contract. Host entitlement filtering remains outside sim.

## Agent log

- 2026-09-29 (UTC) · opencode/gpt-6-astra + T028/D15 specification · Recorded the additive public API and shared validation boundary without changing historical ADRs or claiming implementation verification. The task appendix owns exact details to avoid competing copies.

## Status update — 2026-09-30 (UTC)

**Accepted** — approved by the user on 2026-09-30 ("Approved.", decision D19). Implemented by T028 and merged in PR #20 with its suite green on Windows/MSVC and Linux/GNU CI. See [tasks/DECISIONS-2026-09-30.md](../../tasks/DECISIONS-2026-09-30.md).

- 2026-09-30 (UTC) · claude-code + D19 status update · Appended the user-approved status change as a dated section instead of editing the original status line, per the E007 append-only policy.
