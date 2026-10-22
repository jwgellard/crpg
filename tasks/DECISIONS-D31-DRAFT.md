# Decision D31 — draft

Status: **Draft; not yet put to the user.** Non-blocking questions collected
by manager sessions under `docs/AGENT_TEAM_PLAN.md` §6, each with a
recommendation. Nothing here is decided until the user answers.

## D31a — approve the T136 contract (T136a exact, T136b frame)?

From `tasks/T136.md` ("Status", "T136a exact contract", "T136b frame"):
the contract proposes T136a as the exact RT-fields addition
(`round_ticks` on ruleset, `recovery_ticks` and `repeats_in_real_time` on
ability; optional-missing at the current versions, no bump, no migration, no
golden change; unblocks T138) and binds T136b to one version bump per
document (ruleset /2→/3, ability /2→/3, effect /1→/2, encounter /1→/2).
Status: contract proposed — awaits the user's approval.

- **Recommend:** (a) approve the T136 contract as written (T136a exact, T136b frame only).
- Alternative: (b) do not approve; revise the contract.

## D31b — approve the T135 contract?

From `tasks/T135.md` ("Status", "Decision"): the contract proposes
`refill: Option<NonZeroU32>`, the `with_refill` constructor, and the
infallible clamping `set_max(&mut self, u32) -> bool`; a pool without refill
encodes byte-identically, so no golden or hash changes. S-set, serialized,
human-reviewed. Status: contract proposed — awaits the user's approval.

- **Recommend:** (a) approve the T135 contract as written.
- Alternative: (b) do not approve; revise the contract.

## D31c — T136 Q1: no version bump for additive optional fields (D2)?

From `tasks/T136.md` ("Open questions", Q1): "No version bump for additive
optional fields (D2)?" The file recommends yes: it is the only split that
gives T138 an early start and still bumps each document once.

- **Recommend:** (a) yes, add the three RT fields as optional-missing at the current versions with no tag change and no migration edge.
- Alternative: (b) bump `/2 → /3` now, which means `/4` later or an additive-only T136b.

## D31d — T136 Q2: is T136a S-set?

From `tasks/T136.md` ("Open questions", Q2): "Is T136a S-set? It changes the
shape of two versioned schemas, but not a version or a migration." The file
recommends yes (Claude implementer): the rule exists for this kind of change,
and the package is small.

- **Recommend:** (a) yes, T136a is S-set (Claude `implementer`).
- Alternative: (b) rule T136a outside the S-set and route it to an OpenCode O1 implementer instead.

## D31e — T136 Q3: RT ability fields under a TB-only ruleset?

From `tasks/T136.md` ("Open questions", Q3): "RT ability fields under a
TB-only ruleset." The file recommends no diagnostic in T136a; T136b's
semantic pass should add a `Warning`, and a `ruleset.round_ticks` that no
ability needs stays clean.

- **Recommend:** (a) no diagnostic in T136a for RT ability fields under a ruleset without `round_ticks`.
- Alternative: (b) emit a diagnostic for that mix already in T136a.

## D31f — T136 Q4: explicit `null` reads as absent?

From `tasks/T136.md` ("Open questions", Q4): "Explicit `null` reads as
absent (the `natural_die` precedent, not the crate's required-nullable
rule)." The file recommends keeping the precedent, so all optional-missing
fields behave alike.

- **Recommend:** (a) keep the precedent: explicit `null` decodes as absent for the three RT keys.
- Alternative: (b) follow the crate's required-nullable rule for these keys instead.

## D31g — T136 Q5: a `/1` source carrying an RT key is accepted?

From `tasks/T136.md` ("Open questions", Q5): "A `/1` source carrying an RT
key is accepted. The `1 → 2` steps do not strip later keys, and the same is
true for `natural_die` today." The file recommends accepting it now; T136b's
steps may reject keys their source version lacks.

- **Recommend:** (a) accept a `/1` source carrying an RT key now.
- Alternative: (b) strip or reject source-version-unknown RT keys already in T136a.

## D31h — T136 Q6: T134/T135 exposure?

From `tasks/T136.md` ("Open questions", Q6): "T134/T135 exposure. ADR row 3
does not list success counting, pool `refill` or `set_max`." The file
recommends leaving them out of T136 and adding each with its first consumer
(T109 or a later surface row), under the D2 rule when additive.

- **Recommend:** (a) leave success counting, pool `refill` and `set_max` out of T136.
- Alternative: (b) expose T134/T135 fields in T136.

## D31i — T136 Q7: `repeats_in_real_time: Option<bool>`?

From `tasks/T136.md` ("Open questions", Q7):
"`repeats_in_real_time: Option<bool>` rather than a defaulted `bool`." The
file recommends `Option`: it reuses the existing optional-missing pattern and
needs no `skip_serializing_if = is_false` helper. Absent and `false` mean the
same, and the sim maps both to `false`.

- **Recommend:** (a) `Option<bool>` with the optional-missing pattern.
- Alternative: (b) a defaulted `bool` with a `skip_serializing_if = is_false` helper.

## D31j — T135 Q1: is "validated maximum setter" infallible?

From `tasks/T135.md` ("Open questions", Q1): 'Is "validated maximum setter"
infallible? The ADR's word can read as `Result`.' The file recommends
infallible `-> bool` (D2): every `u32` is a legal maximum, so an `Err` path
would be dead API.

- **Recommend:** (a) infallible `set_max(&mut self, u32) -> bool`.
- Alternative: (b) a fallible `set_max` returning `Result` with an unreachable `Err`.

## D31k — T135 Q2: raising `max` leaves `current` alone?

From `tasks/T135.md` ("Open questions", Q2): "Raising `max` leaves `current`
alone?" The file recommends yes: "grant the difference" is a gain operation;
add it with its first consumer.

- **Recommend:** (a) yes, raising the maximum never raises `current`.
- Alternative: (b) raise `current` together with `max` (grant the difference).

## D31l — T135 Q3: allow `refill >= max`, and `refill` on a `Never` pool?

From `tasks/T135.md` ("Open questions", Q3): "Allow `refill >= max`, and
`refill` on a `Never` pool?" The file recommends allowing both: `set_max`
can later drop `max` below a refill, so a construction-time ban would be
inconsistent. Such pools behave like full-refill or never-refill pools and
differ only in hashed bytes.

- **Recommend:** (a) allow both.
- Alternative: (b) ban `refill >= max` or `refill` on a `Never` pool at construction.

## D31m — T135 Q4: representation?

From `tasks/T135.md` ("Open questions", Q4): "Representation." The file
recommends `Option<NonZeroU32>`, absent meaning full refill: H1 falls out of
it, and zero cannot be a second spelling of anything.

- **Recommend:** (a) `Option<NonZeroU32>` with absent meaning full refill.
- Alternative: (b) a `RefreshTrigger` payload for the amount, which changes every trigger's wire and the T015 tests.

## D31n — T135 Q5: wire key name?

From `tasks/T135.md` ("Open questions", Q5): "Wire key name." The file
recommends `refill`, which pairs with `refresh`.

- **Recommend:** (a) `refill`.
- Alternative: (b) `refill_amount`.

## D31o — T135 Q6: `set_max` return value?

From `tasks/T135.md` ("Open questions", Q6): "`set_max` return value." The
file recommends `bool` ("max changed"), matching `refresh`: a caller that
needs the clamped amount reads `current()` before and after.

- **Recommend:** (a) `bool` ("max changed").
- Alternative: (b) return the clamped `current` amount (or unit) instead.

## Agent log

- 2026-10-22 (UTC) · opencode-go/muse-spark-1.3-contributor + S13 bookkeeping · Started the D31 draft with the T136/T135 contract approvals and their thirteen open questions, so the user can answer them in one batch.
