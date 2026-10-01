## Task
Design the capability model for the privileged editor channel and connect the
trust table to its enforcement. Human-decision task (security design).

## Why this is deferred

The highest-severity forward-looking hole in the review:

- The editor promises live session mutation (move NPC, give item, fire
  trigger) over a GM-privileged session (`docs/CRPG_ENGINE_SPEC.md:819-820`)
  via a presumed admin/RPC surface (`:790`) — with no credential, capability,
  scoping, or LAN-only constraint named. Nothing stops a player crafting
  privileged commands at the protocol level.
- The trust table asserts enforcement it never constructs: T0 "signed and
  listed in server config" (`:917`) names no verification flow, trust root,
  or load-failure semantics; T1 covers content with a sandbox that validates
  nothing about data; T2 "never executes code" (`:921`) coexists with shipped
  GDScript views and untrusted-asset parsers, and never cites the real
  anti-cheat (server-side data minimization `:479`, per-client filtering
  `:695`).
- Bulk package transfer (`:658`) carries Lua (`:478`) to clients forbidden
  from running it (`:921`) — the server-side strip filter has no spec, no
  verifier, and no leak test (the `:1369` suite covers components, not
  package contents).

## Decision to make
- Define the privilege tiers (player/GM/admin), the credential mechanism,
  and the channel separation (intent flag vs separate protocol); map each
  trust row to its actual enforcement (verification flow, filter, sandbox,
  harness); specify the package strip filter and its test.

## Deliverable
- Edits to `docs/CRPG_ENGINE_SPEC.md` (§§7.2, 10, 11.1, 12) with dated notes.
  No source changes.

## Constraints
- Doc-only. Human sign-off; must be designed before either endpoint
  (editor session, server admin surface) is built.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark · Filed as part of the spec-gap triage: a promised privileged channel with no authorization story.

## Resolution — 2026-09-30 (T033, D03)

The capability model was selected in POST-T018 D03: operator-provisioned,
revocable invitation credentials; a server certificate pinned out of band; no
trust-on-first-use, anonymous authority or LAN bypass; ordinary, scoped-GM and
admin grants as explicit operation/resource sets checked immediately before
execution; a separate typed control protocol for privileged traffic,
fail-closed until its own task; no privilege from being in-process; and an
allowlisted client presentation manifest instead of a stripped server package.
T033 wrote this into spec §7.2, §10, §11.1 and §12.1. Building the endpoints,
the manifest export and its leak test belongs to the tasks in the BACKLOG
API-shape ledger.

- 2026-09-30 (UTC) · claude-code + T033 · Recorded how the already-selected decision closes this item and where the spec now says it, per D25.
