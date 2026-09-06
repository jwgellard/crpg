## Task
Settle what `crpg-client`, `crpg-editor`, and the bridge are as build targets.
Human-decision task (spec + README edit).

## Why this is deferred

Three names, no mapping. Spec §§0/2/23 list `crpg-client` and `crpg-editor`
as shipped binaries (`docs/CRPG_ENGINE_SPEC.md:21-28,128-133,1546-1550`);
spec §14 lists only `crpg-server [bin]` + `crpg-cli [bin]` + `crpg-godot
[cdylib]`, with client/editor as `apps/client`, `apps/editor` Godot projects
(`:998-1002`) — neither of which exists. Spec §2.2 names a
`crpg-client-bridge / crpg-edit` layer (`:149-151`); §14 names only
`crpg-godot`; no `crpg-client-bridge` crate exists. The README binary table
(`README.md:28,40-48`) repeats the binary naming. An agent told to scaffold
"the client" will create a `crpg-client` crate, violating one-task-one-crate
and the dependency lint, while §24's post-T18 "client bridge" track (`:1715`)
cannot be tasked until the crate boundary is named.

## Decision to make
- Choose one: (a) client/editor are Godot projects over `crpg-godot`, never
  crates — fix §§0/2/23 + README to §14's shape; or (b) a real bridge crate
  exists — name it and place it in the layer diagram and `ALLOWED` table.
  Either way, state where `ReplicaWorld` query APIs live.

## Deliverable
- Edits to `docs/CRPG_ENGINE_SPEC.md` (§§0, 2.1, 2.2, 14, 23), `README.md`
  binary table, each with a dated inline note. Possibly a `deps.py` table
  edit (with lint self-tests) if (b). No other source changes.

## Constraints
- Doc-only except the possible lint-table row. Human sign-off; renames
  propagate to every future task file.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark · Filed as part of the spec-gap triage: three names for the bridge, two incompatible binary counts.
