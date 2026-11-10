# T140 — next `crpg-net` protocol version: ADR-0029 N1–N4 facts, events, intents, snapshot allowlist, clock heartbeat (draft, blind)

## Task

One new main protocol version after v2 that carries the ADR-0029 §4 N1 facts,
the events `RoundStarted`/`ModeChanged`/`OrderEnded`, the intents
`DeclareAction`/`EndTurn`/`Stop`/`Pause`/`Unpause`/`RequestMode`, the rejection
codes other contracts left to T140, the snapshot-allowlist extension of
ADR-0028, per-field grant filtering for every new fact (T027b H-filter, N2),
and the `Clock` heartbeat message (N4). v1, v2, transfer (`0x81`) and resume
(`0x91`) stay frozen byte for byte.

## Crate(s)

Sole crate: `crpg-net`.

## Split (T140a, T140b, T140c)

T140a owns constants and wire types; T140b owns codecs and projection; T140c
owns interest/snapshot extension, heartbeat and tests. (Filled below.)

## Readings of the ADR

1. (placeholder)
2. (placeholder)
3. (placeholder)
4. (placeholder)
5. (placeholder)

## Interface

(placeholder)

## Behaviour

1. (placeholder)

## Errors

(placeholder)

## Tests

1. (placeholder)

## Out of scope

(placeholder)

## Definition of done

(placeholder)

## Questions for the user

1. (placeholder)

## Agent log

- (placeholder)
