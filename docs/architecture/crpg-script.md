# crpg-script architecture

## Scope

`crpg-script` is where authored behaviour meets the authoritative simulation.
Spec §5 gives it three eventual jobs: the Lua 5.4 sandbox (ADR-0005), the
event IR interpreter (graph nodes/edges compiled from visual graphs and Lua),
and serializable `Wait` continuations that survive save/load.

What exists today is only the first, synchronous slice selected by D17
([decision record](../../tasks/POST-T018-DECISIONS.md#d17--bindings-synchronous-first-slice-deterministic-later-interpreter))
and specified by [T029b](../../tasks/T029b.md): an immutable table of trusted
Rust handlers bound to T029a action declarations, and a transactional dispatch
that turns one validated call into bounded `CombatAction` proposals applied
through the public `crpg-sim` controller. There is no Lua runtime, no graph
interpreter, no wait/yield or continuation, no native extension loading and no
host or network integration. Each of those is a later, separately specified
task; nothing here is a placeholder for them.

## Where it sits

Normal dependencies are `crpg-core` (`EntityId`), `crpg-data` (T029a
`ActionSignatureStore`, `ActionBundleIdentity`, `ActionCall`,
`SignatureError`) and `crpg-sim` (the legacy `World`, `CombatAction`,
`ActionOutcome`, `CombatError`, `validate_action`, `perform_action`), approved
by D21 ([decisions](../../tasks/DECISIONS-2026-09-30.md)). `serde_json` is a
dev-only edge for complete-World byte assertions and fixture documents.
`tools/lint/deps.py`'s `ALLOWED` row also permits `crpg-rules`; that is a
ceiling, not a current edge. `crpg-server` is the eventual consumer; no host
consumes the table yet.

## Module flow

One module, `bindings`, re-exported at the root.

- **Startup.** A trusted caller hands `ActionBindings::new` the declaration
  store, the identity it expects, and one `ActionBinding` (declaration plus
  `fn` pointer) per action. Validation runs in a pinned order — identity,
  count bound, duplicate, unknown, missing, exact signature equality — with
  every id diagnostic chosen lexically, so registration order cannot change
  the result. Only then is the private `BTreeMap` built; the table never
  changes afterwards.
- **Dispatch.** `dispatch(world, actor, identity, call)` is a fixed pipeline:
  T029a `validate_call` → T028 `validate_action(EndTurn { actor })` → one
  handler call with a read-only `InvocationContext` → proposal count and
  actor-substitution checks → sequential `perform_action` on a staged
  `World` clone → publish the clone once on complete success. Every earlier
  stop leaves the caller's world byte-identical; a late `Apply` rejection
  discards the clone, so staged RNG draws, events, deaths, resource spends
  and turn advances never escape.
- **Context.** `InvocationContext` carries `&World` and the validated actor,
  constructible only by dispatch. It cannot reach `&mut World`, RNG, stores
  or host capability (pinned by `compile_fail` rustdoc examples).

Dispatch targets the legacy `World`, not `HistoryWorld`: a history-aware
consumer needs its own sim batch boundary (T029b "Scope and dependencies"),
and T020's journal is neither mutated nor bypassed here.

## Governing decisions

- D17 — synchronous trusted handlers first; E010 budgets and world-owned
  continuations belong to the later interpreter and a separate sim task.
- D21 — the dependency edges above.
- ADR-0018 — `validate_action` as the single admission path the actor check
  reuses, and `perform_action` revalidating every proposal.
- ADR-0005 / E010 — carried forward for the future Lua work; not
  implemented here.
- ADR-0012 — exact-build determinism: the declaration digest does not attest
  to handler code, so a rebuilt binary is a new determinism domain.

## What consumers inherit

A frozen trusted binding table, a validated-call dispatch with pinned error
precedence, a 32-proposal bound on applied simulation operations, and an
honest synchronous scope: `HandlerError::UnsupportedWait` is an error, never
a saved continuation. Saving means saving the `World` between completed
invocations and rebuilding identical bindings at startup; no table or
partial call is persisted. Host actor authorization and target disclosure
remain the host's job. Trusted handlers are not preemptively metered and a
panicking handler is a bug, not a budget error.

## Agent log

- 2026-10-04 (UTC) · claude-code + T029b · Opened the crate's architecture doc with its first real code, describing only the synchronous trusted-binding slice and naming the Lua, interpreter, continuation and history-batch work as later owners so the doc does not read ahead of decisions.
