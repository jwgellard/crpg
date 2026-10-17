# ADR draft — T108: kernel range, time model, ruleset-described player UI

Date: 2026-10-17 (UTC)
Status: Proposed (draft by opencode-go/longcat-2.5-preview-free)

Route: blind design draft for T108 (`docs/IMPLEMENTATION_PLAN.md:206`) under
decision D28l (`tasks/DECISIONS-2026-10-16.md:125-144`). A second lab is
drafting the same ADR independently; a Claude architect compares both and
writes the final ADR. This file is a draft only — nothing here is approved.

## Context

T108 answers three questions (plan row `docs/IMPLEMENTATION_PLAN.md:206`):

1. **Kernel range.** Test the seven rules primitives of `crpg-rules` against the
   CRPG system families d20, dice pool, percentile, action points, and
   classless skill-based; per family state what is supported now, what is a
   gap (and the smallest kernel change that closes it), and what is out of
   scope.
2. **Time model.** Real-time-with-pause (RTwP) and turn-based (TB) in one
   game: the real-time `Timeline` policy, pause, switching modes per
   encounter or area, split parties, per-session pause authority and
   auto-pause in multiplayer, and what snapshots and the protocol must carry.
   Prior art: `docs/research/T099/R14-deep.md`.
3. **Ruleset-described player UI.** How a ruleset describes its player UI
   (character sheet, action bar, spellbook, inventory, level-up, combat HUD)
   so any ruleset displays correctly. Prior art: `docs/research/T099/R13-deep.md`
   and `R15-deep.md`. T104 (`tasks/DECISIONS-2026-10-16.md:119-120`) refines
   this later; this draft stays at the level of what data the ruleset must
   provide.

Engine facts this draft relies on (all read, not inferred):

- The kernel exposes seven public modules (`crates/crpg-rules/src/lib.rs:13-20`):
  `stats`, `derived`, `modifier`, `dice`, `resolution`, `resource`, `hooks`.
  Spec §3.1 states the discipline: the kernel defines how values are computed
  and outcomes decided, never what the values mean (`docs/CRPG_ENGINE_SPEC.md:329-339`).
- Time is one `Timeline` — a `BTreeMap<(InitiativeKey(i32), EntityId)>`
  container (`crates/crpg-sim/src/timeline.rs:41-44`). `InitiativeKey` is a
  bare `i32` precisely so rulesets can use "D&D-style descending counts,
  tick-indexed real-time keys, negative surprise penalties"
  (`timeline.rs:17-25`). The container holds no advance policy; policy belongs
  to the tick loop (`timeline.rs:1-9`). TB combat already runs on it: one entry
  per live participant, an accepted action pops the actor, and an empty
  timeline with two or more alive rolls the round over (`docs/adr/0013-combat-simulation-state.md`,
  Decision 5). The real-time side of the policy is not implemented: the
  per-tick queue step iterates the standing order and its body is empty,
  awaiting the first per-entity system (`crates/crpg-sim/src/tick.rs:48-60`).
- The server ticks at a fixed 20 Hz; all durations are in rounds, turns, or
  ticks, never seconds; RTwP and TB are one mechanism with two policies; the
  client interpolates with a ~100 ms delay buffer (`docs/CRPG_ENGINE_SPEC.md:312-317`).
  Tick order is fixed and handwritten (`docs/CRPG_ENGINE_SPEC.md:936-948`).
- Nothing in the sim is wall-clock: ticks, not wall time, age observations —
  "a pause ages nothing by itself" (`crates/crpg-net/src/protocol.rs:159-165`).
- v1 lane-0 has no snapshot transfer wire; snapshot bounds are frozen so the
  future snapshot lane inherits a reviewed ceiling (`protocol.rs:85-95`). The
  v2 event protocol carries `server_tick`/`event_seq` per delta frame and the
  tags 8–10 `ActionResolved`/`TurnStarted`/`EncounterEnded` (`docs/adr/0019-event-protocol-v2.md`).
- Host sessions own epochs, grants are per-entity/per-field, and captures
  retire through a trusted consumer (`docs/adr/0022-host-session-capture-retirement.md`).

Marked `[check: …]` claims in the research reports were not confirmed by the
citation check; no decision below rests on one alone.

## Decision

### 1. Kernel range

The seven primitives are tested as spec §3.1's seven concepts, realized in
code as follows (the crate has seven public modules; `error` is infrastructure,
and dice sits inside Stat/Resolution as their value/roll type):

| Concept | Realized as |
|---|---|
| Stat | `StatValue` Int/Fixed/Bool/Enum/Tags/Dice (`crates/crpg-rules/src/stats.rs:51-65`) plus the derived-`Expr` tree (`stats.rs:96-113`) |
| Modifier | `Modifier`, `ModOp`, tag-only `ConditionExpr`, three data-selected stacking policies (`crates/crpg-rules/src/modifier.rs:73-139`) |
| Effect | Authored `crpg.effect/1` documents (modifiers, policy, `duration_rounds`; `rulesets/srd-lite/focusing.json`) — the kernel keeps no effect store or dispatcher (`docs/architecture/crpg-rules.md:11-13`) |
| Resolution | `resolve` with `RollSpec`, `Against` (constant/stat/opposed), margin bands, one natural-face rule (`crates/crpg-rules/src/resolution.rs:272-395`) |
| Resource | `ResourcePool` with five refresh triggers, caller-clocked (`crates/crpg-rules/src/resource.rs:36-109`) |
| Tag | Interned-string `TagSet` on entities (`stats.rs:22-23`); stat/tag strings are the persisted authority (`docs/architecture/crpg-rules.md:68-74`) |
| Event/Hook | Nine-variant `KernelHook`, `HookMutation` proposals, fn-pointer handler, no registry (`crates/crpg-rules/src/hooks.rs:26-142`) |

Both existing rulesets were read as the d20-family evidence: `minimal-d6`
(2d6 roll-under, one action per turn: `rulesets/minimal-d6/ruleset.json:5-11`,
`roll_under.json`, `strike.json`) and `srd-lite` (two pools, natural-face
shift, per-outcome damage, effects: `rulesets/srd-lite/ruleset.json:15-30`,
`heavy.json`, `heavy_table.json`, `focus.json`).

#### d20 family (5e/SRD-like)

| Primitive | Supported now | Gap + smallest kernel change | Out of scope |
|---|---|---|---|
| Stat | Yes — Int stats, derived `Expr` (`stats.rs:96-113`); srd-lite's six attributes and health (`ruleset.json:33-53`) | — | Fixed/Bool/Enum/Tags unused here but available |
| Modifier | Yes — `HighestPriorityPerName` covers same-name no-stacking; `HighestBonusWorstPenalty` covers PF2e (`modifier.rs:131-139`) | — | — |
| Effect | Yes — authored documents with modifiers and `duration_rounds` (`focusing.json`); sim attaches effects by ULID without interpreting them (`crates/crpg-sim/AGENTS.md`, T017d) | — | Effect interpretation is host territory (`docs/architecture/crpg-rules.md:11-13`) |
| Resolution | Yes — d20 group, constant or stat DC (`resolution.rs:301-324`), natural-face crit rules (`resolution.rs:87-92`), degree-of-success bands (`resolution.rs:197-222`); advantage/disadvantage is 2d20 + Keep/Lowest(1) (`dice.rs:30-43`) | — | — |
| Resource | Yes — action/bonus/reaction/movement as pools with `OnTurnStart`/`OnRest` refresh (`resource.rs:36-47`; srd-lite's two pools) | — | — |
| Tag | Yes — conditions as tags gating modifiers (`modifier.rs:97-106`) | — | — |
| Event/Hook | Yes — nine variants; damage hooks carry caller-computed nonnegative amounts (`hooks.rs:78-107`) | — | — |

**Gap (one).** Crit doubling of damage dice: damage is a static per-outcome
integer (`heavy.json:5-24`; the sim's `CombatDefinition` damage is
`Vec<(Outcome, u32)>` per `crates/crpg-sim/AGENTS.md`), and the kernel
computes no damage (`hooks.rs:78-92`). No `crpg-rules` change closes this —
the smallest change is at the sim damage model (the combat generalization
track), i.e. outside T108's kernel scope. A ruleset can author the doubled
amount as a second outcome today.

#### Dice-pool family (Shadowrun/World of Darkness)

| Primitive | Supported now | Gap + smallest kernel change | Out of scope |
|---|---|---|---|
| Stat | Yes | — | — |
| Modifier | Yes — roll-target bonuses (`modifier.rs:55-69`) | — | — |
| Effect | Yes (authored data) | — | — |
| Resolution | Partial — `resolve`'s total is the kept sum plus one offset, clamped once (`dice.rs:159-169`); the margin is actor total − defence (`resolution.rs:703`). A success **count** cannot enter the margin, so pool-vs-pool opposed rolls cannot run through `resolve` | Smallest change: add an optional success threshold to `RollSpec::Dice` so the total becomes the count of kept dice ≥ threshold (one field; count clamps to `i32`). Zero-kernel alternative: the host evaluates both sides via `DiceExpr::evaluate` and counts faces in `DiceRoll.dice` (`dice.rs:165-169`) — works today for display, but loses `resolve`'s transactional staged-RNG commit (`resolution.rs:476-479`) | — |
| Resource | Yes | — | — |
| Tag | Yes | — | — |
| Event/Hook | Yes | — | — |

**Gaps (three).** (a) Success counting, above. (b) Exploding dice (reroll
max faces and add): the grammar is one fixed-count group (`dice.rs:3-4`); the
host can chain `evaluate` calls on one named stream, but `resolve` cannot
express it — same smallest-change family as (a). (c) Reroll-lowest is not
expressible at all, and World-of-Darkness 1s-subtract is not expressible via
the single offset (`dice.rs:124-128`); both are host-side trace-counting
exercises today. **Out of scope:** mixed side counts in one group and
per-die target numbers — one homogeneous group (`dice.rs:45-57`).

#### Percentile family (d100, Call of Cthulhu, Traveller, RuneQuest)

| Primitive | Supported now | Gap + smallest kernel change | Out of scope |
|---|---|---|---|
| Stat | Yes | — | — |
| Modifier | Yes | — | — |
| Effect | Yes (authored data) | — | — |
| Resolution | Yes — sides up to 1,000,000 (`lib.rs:68`); roll-under is a margin table (`rulesets/minimal-d6/roll_under.json`); degree-of-success bands (`resolution.rs:197-222`); opposed percentile via `Against::Opposed` (`resolution.rs:322-324`); skill/5 divides via `Expr::Divide` (`stats.rs:108`) | No capability gap. Natural rules match one face each, at most one rule per roll (`resolution.rs:80-92`): a 1–5 critical range enumerates as five rules — expressible; a range rule is convenience, recommended against (data suffices) | — |
| Resource | Yes — pools with `OnRest` refresh cover luck/FP-like pools (`resource.rs:41-42`) | — | — |
| Tag | Yes | — | — |
| Event/Hook | Yes | — | — |

**Note for module authors.** The literal-exclusion gate bans the d20 notation
inside `crpg-rules` only (`tasks/T015.md:557`). Percentile notation in
ruleset JSON data is unaffected; the two scopes should not be confused.
**Out of scope:** luck spend, push rolls — host/ruleset design.

#### Action-points family (Pathfinder 2e, AP hybrids, real-time cooldowns)

| Primitive | Supported now | Gap + smallest kernel change | Out of scope |
|---|---|---|---|
| Stat | Yes | — | — |
| Modifier | Yes | — | — |
| Effect | Yes (authored data) | — | — |
| Resolution | Yes — one-sided rolls and flat checks (`RollSpec::None`, `resolution.rs:272-277`) | — | — |
| Resource | Yes — pools with per-ability costs and `OnTurnStart` refresh cover PF2e's 3-action turn; real-time cooldowns use `OnTick(n)` (`resource.rs:36-47`); affordability sums same-pool entries atomically (T019, `crates/crpg-sim/AGENTS.md` invariant 10); multi-action turns via `ends_turn: false` (`rulesets/srd-lite/focus.json:25`) | (a) Stat-derived pool max (e.g. movement = 30 + 5×Dex): `ResourcePool` has **no maximum-adjustment API** — max is fixed at construction and refresh refills to it (`resource.rs:1-9`). Smallest kernel change: a validated `set_max` setter (clamp-vs-fail semantics is an open question). (b) Once-per-encounter pools: no encounter refresh trigger exists (`resource.rs:36-47`). Smallest kernel change: an additive `OnEncounterStart` trigger plus `RefreshEvent` variant, mirroring `KernelHook::OnEncounterStart` (`hooks.rs:48-51`); zero-kernel alternative is host tag bookkeeping | Interrupt/reaction windows — host systems |
| Tag | Yes | — | — |
| Event/Hook | Yes — `OnTurnStart`/`OnRoundStart`/`OnRest` are also the refresh vocabulary (`resource.rs:49-65`, `hooks.rs:32-51`) | — | — |

#### Classless skill-based family (GURPS, Traveller, CoC, RuneQuest)

| Primitive | Supported now | Gap + smallest kernel change | Out of scope |
|---|---|---|---|
| Stat | Yes — skills as stats; specializations as `Enum` (`stats.rs:77-82`) or tags | — | — |
| Modifier | Partial — conditions read **tags only** (`modifier.rs:93-106`): "if skill ≥ 12, +1" is inexpressible | Smallest kernel change: a stat-threshold `ConditionExpr`, but it needs a pinned evaluation point — conditions run inside the fold, where the target stat may itself be modified (phase order Set→Add→Multiply→Clamp, `docs/architecture/crpg-rules.md:90-96`). Zero-kernel alternative: the host projects thresholds to tags at authoring time (e.g. `skill_12_plus`) — works today, duplicates state, can drift | — |
| Effect | Partial — `Modifier` has **no lifetime field** (`modifier.rs:110-127`; spec §3.2's lifetime is not implemented). Temporary boosts expire via host filter-by-`SourceRef` removal plus re-query (`crates/crpg-rules/AGENTS.md`, "Mutation and query rules") | A kernel lifetime field is the real fix but a larger change touching validation, stacking, and persistence; recommend against for now — host removal already implements the semantics | — |
| Resolution | Yes — skill-vs-target-number tables, opposed skill checks (`Against::Opposed`, `resolution.rs:322-324`), degree-of-success bands | — | — |
| Resource | Yes | — | — |
| Tag | Yes — the vocabulary conditions read | — | — |
| Event/Hook | Yes | — | — |

**Out of scope (by design):** class/level systems, progression tables,
character-generation wizards, encumbrance — spec §3.4 (`docs/CRPG_ENGINE_SPEC.md:401-407`).

#### Range summary

The kernel covers d20, percentile, and action-points families as-is (with two
small, additive resource-pool changes available). Dice-pool success counting
and exploding dice are real gaps with a one-field kernel fix each; classless
families are blocked by two deeper shape decisions (stat-conditions,
modifier lifetimes) that this draft deliberately leaves open rather than
guessing (Open questions).

### 2. Time model

#### D28l-1 — Pause is a host loop control, and pause authority is a host-granted permission

**Decision.** Pause freezes the whole simulation for everyone: the host
simply stops delivering `tick`/`end_turn` to the sim. No pause state enters
`World` and nothing enters `state_hash` — pause is not sim-observable, and
"a pause ages nothing by itself" (`crates/crpg-net/src/protocol.rs:159-165`)
keeps determinism exact-build. Pause authority is a host-granted, revocable
permission with split pause/unpause rights: anyone holding it pauses
everyone (R14 rec 1: host-granted pause permission, all freeze; `docs/research/T099/R14-deep.md:8`).

**Options.**

- (a) Host-granted permission token, split rights — recommended. Matches the
  Baldur's Gate host model (R14 rec 1) and ADR-0022's grant discipline
  (`docs/adr/0022-host-session-capture-retirement.md`: control is granted,
  per-entity, and revoked by the operator).
- (b) Any authenticated player may pause. Rejected: one player freezes
  everyone — the exact abuse BG multiplayer exists to prevent.
- (c) Per-player pause. Rejected: a shared authoritative world cannot freeze
  for one viewer; the sim has no per-viewer time.

**Consequences.** No sim, net, or protocol change is required for pause
itself; the host loop stops advancing. Clients learn the paused state from the
host (below, D28l-6). Revocation is an operator action on the session, reusing
the existing grant machinery.

#### D28l-2 — Auto-pause is a host-side policy, default-off in multiplayer

**Decision.** Auto-pause (on enemy sighted, on round start, …) is evaluated
by host rules above the sim; when a trigger fires the host stops ticking.
Per-trigger toggles are host-controlled; the multiplayer default is off
(R14 rec 4: auto-pause is the top pause-spam vector on a shared timeline;
`R14-deep.md:11`).

**Options.**

- (a) Host-side trigger table with per-trigger toggles, MP default off —
  recommended. Zero sim surface; toggles are operator data.
- (b) Sim-emitted "please pause" events. Rejected: new `SimEvent` variants are
  vocabulary decisions with a producer and a reader
  (`crates/crpg-sim/AGENTS.md`, known traps), and a non-behavioural fact has
  no business in the hashed world.

**Consequences.** Trivial dialogue and stores never auto-pause; important
dialogue uses a party vote with host tiebreak (R14 rec 7) — a host policy, not
a sim rule. Turn timers, delay/skip, and party-AI disable in TB (R14 rec 5)
are likewise host policies above the sim; recommend adopting later, off by
default.

#### D28l-3 — Mode switches per encounter, on the fly, on the one Timeline

**Decision.** A mode (RTwP or TB) is stored **per encounter** inside the
world-owned combat state (ADR-0013 Decision 1: combat state lives in `World`,
behavior-hashed). Switching is a policy flip, never a per-campaign lock:
RTwP re-keys timeline entries by tick-indexed `InitiativeKey`s
(`timeline.rs:17-25` names tick-indexed real-time keys) and rotates the
standing queue every tick; TB keeps one entry per participant, pops on
`EndTurn`/accepted action, and rolls the round over when the timeline empties
(ADR-0013 Decision 5). A switch preserves entries, round, active head, and
pool balances. WotR ships on-the-fly per-encounter switching; Deadfire shipped
a campaign lock and reversed it (R14 rec 2, `R14-deep.md:9`) — the user's
requirement quotes both (`tasks/DECISIONS-2026-10-16.md:130-133`).

**Options.**

- (a) Per-encounter mode in the world-owned combat state, policy flips with
  it — recommended. The mode is behaviour-visible (it changes advance
  policy), so it must be hashed; that is exactly ADR-0013 Decision 1's
  discipline.
- (b) Per-area mode. Rejected: DOS:EE and BG3 run both modes in one area at
  once when the party is split (R14 rec 6, `R14-deep.md:13`).
- (c) Per-campaign lock. Rejected: Deadfire reversed exactly this
  (`R14-deep.md:9`); the user asked for both modes in one game.

**Consequences.** **This touches the S-set**: the mode is a new field on the
world-owned combat state, so `World`'s serialized shape changes and new
goldens are required (never a silent rebaseline; `docs/IMPLEMENTATION_PLAN.md:86-98`
serializes the `World` struct and tick order one at a time, human-reviewed).
The alternative — host-side mode storage — is rejected because a
behaviour-visible fact outside the hash would break exact-build replay
(ADR-0009). No invariant is broken; the S-set cost is stated, not hidden.

#### D28l-4 — Split parties: only the engaged group holds TB timeline entries

**Decision.** The single `Timeline` is scoped by participation: TB
participants hold entries (one per live combatant, ADR-0013 Decision 5);
unengaged entities in the same area hold none and stay real-time, joining the
encounter's timeline on approach or attack (R14 rec 6). Joining follows the
late-join rule: a join during round 1 keeps a full turn slotted behind the
acted combatants; a join after round 1 skips the turn (R14 rec 3,
`R14-deep.md:10`). Tied characters of one player act in ascending
`EntityId` order — simultaneity is a client input-collection concern, and
ties already order deterministically (`timeline.rs:17-25`).

**Options.**

- (a) One timeline, entries scoped by engagement — recommended. The
  container is already per-world; encounter state is world-owned
  (ADR-0013); no second ordering structure is introduced.
- (b) Two `Timeline` containers (RTwP queue + TB queue). Rejected: the tick
  loop walks one ordering; two structures double the load-coherence checks
  (`crates/crpg-sim/AGENTS.md`, load-coherence invariant) for no gain.
- (c) Per-entity mode tags on timeline entries. Rejected: a `BTreeMap` key
  cannot carry a mode without a second index, and ascending iteration would
  interleave two policies.

**Consequences.** The round-robin re-key for RTwP and the join-slot rule are
policy in the tick loop / encounter controller (T008/T016 territory,
`timeline.rs:1-9`), not container changes. Despawn/release keep working
unchanged (ADR-0014).

#### D28l-5 — What snapshots and the protocol carry

**Decision.** Sim-owned facts already hash and fan out: timeline order, round,
active head, pool balances, turn facts (v2 tags 8–10 carry
`ActionResolved`/`TurnStarted`/`EncounterEnded` with round;
`docs/adr/0019-event-protocol-v2.md`). Add to the **host-owned** session
facts (not hashed): current pause state (who holds it), per-encounter mode
echo, and the auto-pause trigger set. The full snapshot lane remains its own
protocol task — v1 freezes snapshot bounds with no transfer wire
(`crates/crpg-net/src/protocol.rs:85-89`), and the host capture/checkpoint
boundary already exists (ADR-0022 Decisions 3–4).

**Options.**

- (a) Host-level session facts envelope plus existing v2 event ops —
  recommended. Keeps non-sim facts out of the hash (ADR-0009 discipline) and
  reuses the reviewed capture path.
- (b) New sim fields for pause/mode. Rejected for pause (non-behavioural);
  mode belongs in the sim only per D28l-3.
- (c) Carry mode/pause inside v2 delta ops. Rejected: deltas are sim/event
  facts (ADR-0019); host session state is not an event.

**Consequences.** Clients render pause and mode from host facts; gameplay
reads the hashed world through the stable query API (spec §9.2,
`docs/CRPG_ENGINE_SPEC.md:877-883`).

### 3. Ruleset-described player UI

#### D28l-6 — A ruleset carries a screen manifest; the client generates layout from it

**Decision.** A ruleset ships a new data family — a `crpg.screens/1`
manifest — describing, per screen, which ruleset data it shows and how: the
character sheet (stat list, grouping, order, breakdown visibility — the
modifier breakdown already powers tooltips, spec §3.2,
`docs/CRPG_ENGINE_SPEC.md:363`), the action bar (abilities with costs,
targeting, `ends_turn`), the spellbook (abilities filtered by an authored
tag), the inventory (items are stats and tags — spec §3.4,
`docs/CRPG_ENGINE_SPEC.md:406`), level-up (references the progression table;
the table itself stays ruleset data), and the combat HUD (active head, round,
timeline order, pool balances, pause state). Layout is generated from these
schemas (spec §11.4 commitment 3: "Property forms are generated from schemas";
R13 rec 1: generate the UI from schema, never hand-written sheets —
`docs/research/T099/R13-deep.md:5`). The engine ships no per-ruleset
hand-written UI, honoring the kernel-never-defines-meaning rule (spec §3.1).

**Options.**

- (a) Ruleset-authored screen manifest — recommended. Matches the data-first
  authoring discipline (R13 recs 1–3: typed inheritable data models,
  clone-and-modify starters, layering + patch overrides — `R13-deep.md:5-9`;
  the two shipped rulesets are exactly clone-and-modify starters).
- (b) Engine-built screens keyed off stat/ability kinds. Rejected: hand-written
  sheets rot (spec §11.4), and screen meaning is ruleset semantics, which the
  engine must not hard-code.
- (c) Per-ruleset UI scripts in Lua. Rejected: client-side scripting is
  UI-only and cannot touch the replica world (spec §9.2 rule 4,
  `docs/CRPG_ENGINE_SPEC.md:882`); Lua hooks are the kernel's behaviour
  channel, not a layout language.

**Consequences.** **This touches the S-set**: `crpg.data` schema versions are
serialized S-set territory (`docs/IMPLEMENTATION_PLAN.md:86-98`); a new
`crpg.screens/1` family is added through the normal crpg-data schema +
migration path (the `crpg.ruleset/1→2` and `crpg.ability/1→2` migrations are
the precedent, `crates/crpg-data/src/migrations/`). T104
(`tasks/DECISIONS-2026-10-16.md:119-120`) owns research and a combat-HUD
prototype and may change the screen set; this draft fixes only the data
contract level. T050/T051 wait on both (D28m,
`tasks/DECISIONS-2026-10-16.md:150-151`).

## S-set and invariant impact

Explicitly, per the brief:

- **S-set (`docs/IMPLEMENTATION_PLAN.md:86-98`):** two recommendations touch
  it. (1) D28l-3 stores the per-encounter mode in the world-owned combat
  state — `World`'s serialized shape changes; new goldens are required and
  the change is serialized and human-reviewed. (2) D28l-6 adds a
  `crpg.screens/1` schema family in `crpg-data`. Both are stated here, not
  hidden; neither is implemented by this draft.
- **Platform invariants (root `AGENTS.md`):** no OS-specific branches are
  introduced; pause and mode are platform-neutral host/sim facts. The
  recommendation builds on ADR-0012's one-platform-neutral server.
- **Determinism invariants:** no floats are added anywhere (mode is an enum,
  pause is a loop control; the `f32`-spatial / `Fx16_16`-rules split and the
  no-`f64` ban are untouched); no `HashMap`/`HashSet` (any new maps are
  `BTreeMap`/`IndexMap`); exact-build replay is preserved — pause ages nothing
  (`protocol.rs:159-165`), and the one behaviour-visible new fact (mode) is
  hashed, never excluded (ADR-0009). No recommendation breaks an invariant.

## Consequences

- **Easier:** module authors get a per-family coverage map with the smallest
  kernel changes named; pause, mode switching, and split parties become
  implementable as host policy plus one hashed field, not a second engine;
  player UI becomes authorable data, so total conversions (spec §12.2,
  `docs/CRPG_ENGINE_SPEC.md:1116`) ship a UI without engine changes.
- **Harder:** the dice-pool and classless gaps need kernel decisions this
  draft deliberately defers (success counting, stat-conditions, lifetimes);
  the two S-set changes queue behind other serialized work.
- **Deliberately left open:** effect interpretation (host territory,
  `docs/architecture/crpg-rules.md:11-13`); the snapshot wire (frozen bounds,
  `protocol.rs:85-89`); pacing features (turn timers, R14 rec 5); late-join
  edge cases beyond the round-1 rule.

## Open questions for the user

1. **Stat-derived pool max.** Add a validated `ResourcePool::set_max` (clamp
   current vs fail on shrink), or model variable pools as multiple fixed
   pools? *Recommendation:* add `set_max` with clamp semantics — small,
   additive, no new persisted shape.
2. **Once-per-encounter pools.** Add the `OnEncounterStart` refresh trigger
   (additive) or use host tag bookkeeping? *Recommendation:* add the trigger;
   it mirrors `KernelHook::OnEncounterStart` and keeps refresh vocabulary in
   one place.
3. **Stat-threshold conditions.** Extend `ConditionExpr` with a stat
   comparison (needs an evaluation-point decision), or author-time tag
   projection? *Recommendation:* tag projection for v1; revisit after T109.
4. **Dice-pool success counting.** Extend `RollSpec::Dice` with a
   success-threshold total, or count faces host-side from `DiceRoll.dice`?
   *Recommendation:* extend `RollSpec` — it keeps pool combat inside
   `resolve`'s transactional staged-RNG path.
5. **Exploding dice.** Kernel support now, or host-side stream chaining?
   *Recommendation:* host-side for v1; kernel only if T109 needs it inside
   `resolve`.
6. **Mode storage.** Confirm the per-encounter mode lives in the world-owned
   combat state (S-set `World`-shape change, new goldens) rather than
   host-side? *Recommendation:* yes — behaviour-visible facts are world-owned
   and hashed (ADR-0013 Decision 1).
7. **Screen-manifest scope.** Confirm the six screens (sheet, action bar,
   spellbook, inventory, level-up, HUD) as the v1 set, with T104 free to add
   or remove? *Recommendation:* confirm the set is open for T104; the
   manifest schema accommodates additions without version bumps.
8. **Pacing features.** Adopt turn timers / skip / shared TB order now, or
   later as host policy? *Recommendation:* later, off by default (R14 rec 5);
   they are host policy above the sim.

## Agent log

- 2026-10-17 (UTC) · opencode-go/longcat-2.5-preview-free + T108 draft · Wrote this blind draft from the engine code, the two shipped rulesets, the T099 research record and the existing ADRs, answering T108's three questions with per-family kernel coverage, a host-owned pause/mode time model, and a ruleset screen manifest, and naming the two S-set touches explicitly.
