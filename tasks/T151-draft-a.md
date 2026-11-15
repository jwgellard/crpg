# T151 — `crpg-godot` bridge move to protocol v4 (`crpg-godot`, `apps/client`)

**Status: draft (blind), 2026-11-15 (UTC).** Not approved. A Claude architect
writes the final contract; the user approves it. Bound by ADR-0033, ADR-0036,
the approved T026p contract (`tasks/T026p.md` §Out of scope `:1477–1479`) and
the approved T037 contract (`tasks/T037.md` §Out of scope `:1356–1358`, and the
Interface items T037b–T037d). Citation convention (T037's): a bare `:N` or
`:N–M` is a line of the file cited earlier in the same list item or sentence.
Every `path:line` was checked on `2aae6c9` (branch `task/T151-draft-a-d1`).

## Task

## Parts

## Readings

## Interface

## Behaviour

## Errors

## Tests

## What must not change

## Dependency edges

## Out of scope

## Open questions

## Agent log

- 2026-11-15 (UTC) · opencode-go/deepseek-v4-pro + T151 draft a · Blind draft of the `crpg-godot` bridge move to protocol v4 (`ReassemblyPoolV4`, `ReplicaWorldV4`, `MoveOutbox`), so the bridge opens a v4 session, sees positions and `Moved`, and sends lane-1 moves through the embedded server, leaving T049/T026/T050 room without doing them.
