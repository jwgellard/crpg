# crpg-script — agent contract

Read the root rules, [POST-T018-RULES](../../tasks/POST-T018-RULES.md), and
[T029b](../../tasks/T029b.md) — its "Specification revision — 2026-09-29"
section is the normative contract for everything in this crate today.
Architecture: [crpg-script](../../docs/architecture/crpg-script.md). This file
describes the T029b synchronous trusted-binding slice only.

## Public surface

Module `bindings`, re-exported at the root: `MAX_BINDING_PROPOSALS` (32),
`ActionHandler`, `InvocationContext` (`world`, `actor`), `ActionBinding`
(`signature`, `handler`), `ActionBindings` (`new`, `declarations`,
`contains`, `dispatch`), `HandlerError` (`Refused`, `UnsupportedWait`),
`BindingError` (12 variants). The exact signatures, check order and display
strings are in T029b; changing any of them needs a reviewed specification.

## Invariants

1. **Fail closed at startup.** `new` checks identity, then the
   `MAX_ACTION_SIGNATURES` bound (before sorting or building the map), then
   duplicate, unknown, missing, signature — every id chosen lexically.
   Never overwrite a duplicate, never skip a declaration.
2. **Immutable after construction.** No registration, replacement, removal,
   `Deserialize`, `From<World>` or other path by which authored input or a
   save installs or swaps a handler. `contains` is inert.
3. **Validation before the handler.** T029a `validate_call` (as `Call`),
   then `validate_action(EndTurn { actor })` (as `Actor`). Never re-implement
   T029a type/bound checks or T028 actor checks here, and never resolve on a
   clone to "check".
4. **Read-only context.** `InvocationContext` exposes `&World` and the actor
   only; keep its fields private and add no mutator, RNG or host handle. The
   `compile_fail` doctests on it must stay and must fail for their stated
   reason (E0596/E0451 — stable rustdoc does not check the code, so verify
   by hand when you touch them).
5. **Bound, then substitute, then stage.** `ProposalLimit` before
   `ForeignActor`; both before any clone. Apply in returned order with
   `perform_action` on one staged clone; publish once on full success; on the
   first `Apply` rejection drop the clone. Never prevalidate every proposal
   against the initial state instead.
6. **No fake results.** Outcomes are `perform_action`'s own `ActionOutcome`s;
   script never computes damage or receipts. `UnsupportedWait` is always an
   error.

## Allowed dependencies

Normal: `crpg-core`, `crpg-data`, `crpg-sim` (path, default features), per
D21. Dev: workspace `serde_json`. Anything else — including `crpg-rules`,
which `deps.py` would permit, any Lua crate, or a feature expansion — needs
the T018a-style record and explicit approval first. No `unsafe`, no build
script, no `HashMap`/`HashSet`, no floats.

## Definition of done for any change

```text
cargo fmt --all
cargo clippy -p crpg-script --all-targets --locked -- -D warnings
cargo test -p crpg-script --test action_bindings --locked
cargo test -p crpg-script --locked
```

plus the root/POST-T018 common gates on both native targets.

## Known traps

- `ActionBinding`/`ActionBindings` deliberately lack `PartialEq`, serde and a
  pointer-printing `Debug`; function-address equality is not semantic.
- `BindingError`'s `Call`/`Actor`/`Handler` delegate both `Display` and
  `source` to the wrapped error (so `source()` is the inner error's own
  source, `None` today); only `Apply` exposes its `CombatError` as source.
- Dispatch uses the legacy `World`. Do not route it through `HistoryWorld`
  or add an unjournaled mutation path to T020; that needs a separate sim
  batch boundary.
- Test handlers may count invocations through a thread-local; production
  handlers must be deterministic and side-effect free.

## Agent log

- 2026-10-04 (UTC) · claude-code + T029b · Wrote the crate's first working contract with its first code: the pinned startup/dispatch order, the read-only context guarantee and the D21 dependency ceiling, so later Lua/interpreter work extends rather than reinterprets the synchronous slice.
