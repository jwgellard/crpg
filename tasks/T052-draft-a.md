# T052 — persisted continuation state for dialogue, quests and waits (`crpg-sim`)

**Status: blind draft (draft A), 2026-11-19 (UTC).** Written by
opencode-go/deepseek-v4-pro from the fact sheet (`tasks/T052-facts.md`) and the
root `AGENTS.md`. A Claude architect writes the final contract from this and
the other blind draft; nothing here is approved.

Citation convention: every citation is `path:line`, checked on this branch
(`task/T052-draft-a-d1`, base `task/T052-facts-f1` at `d1a7c04`). A bare `:N`
or `:N–M` is a line of the file cited earlier in the same sentence.

Notation: `G`, `W`, `N` are non-nil `Ulid`s; `e0` is an `EntityId` index 0,
generation 1. `Tick::get()` (or `world.tick()`) is the world's current tick.

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

- 2026-11-19 (UTC) · opencode-go/deepseek-v4-pro + T052 draft a · Wrote the blind draft of the T052 contract: world-owned continuation, dialogue and quest state with an absence-skip serialized shape, a stage-2 wait countdown, structural load validation, and sim controllers for `crpg-script` to write, keeping the continuation as data in `World` with execution upward.
