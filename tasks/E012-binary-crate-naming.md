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

## T009c Platform Obligations

Status remains open. ADR-0012/T009c fixes Windows/MSVC as primary, supporting
client, editor, embedded single-player server, dedicated server, and CLI.
Linux/GNU dedicated server, headless CLI/tooling, server-side extensibility,
and testing are fully supported, not best effort. Linux GUI client/editor
builds are not promised; headless support must not depend on Godot.

With E022, assign one reusable platform-neutral authoritative host
implementation to Windows embedded, Windows dedicated, and Linux dedicated
adapters. Decide whether that reusable code is a library target in
`crpg-server` or belongs in another existing crate; this appendix chooses
neither package placement nor API. In-process transport must preserve the
client/server boundary: the client never mutates authoritative state directly.
OS-specific process, filesystem, service, and presentation concerns stay above
core/rules/sim.

The eventual target/package mapping must cover real Windows client/editor/
server and Linux headless server artifacts. Owning implementation tasks must
activate Windows embedded-server and dedicated-server smoke tests plus Linux
dedicated-server smoke tests as those capabilities exist (E020: no placeholder
jobs). Coordinate target-specific native-extension artifacts with open E023;
do not choose a loader, ABI, dependency, or unsafe exception here.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark · Filed as part of the spec-gap triage: three names for the bridge, two incompatible binary counts.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c · Added the shared authoritative host packaging obligation and supported product targets while leaving package/API placement open with E022. Recorded capability-gated product checks and the E023 extension decision boundary without selecting an implementation.
