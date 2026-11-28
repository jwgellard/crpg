#!/usr/bin/env python3
"""Mutant driver for T157c (crates/crpg-sim/src/combat.rs, movement.rs).

Applies one mutant at a time by exact text replacement, runs the crate's tests
with CARGO_INCREMENTAL=0, records one JSON line per mutant in results.jsonl,
and restores the original bytes in a `finally`.

usage: run_mutants.py [--check] [--baseline] [--full] [ids...]
       --check    applies every anchor without cargo (no results written).
       --baseline runs the unmutated tree once (no results written).
       --full     use the full `cargo test -p crpg-sim` command (default: fast).
                  Full results go to results_full.jsonl.
       no ids: all rows, in table order.
"""
import json
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "crates/crpg-sim/src"
FULL = "--full" in sys.argv
OUT = ROOT / ("reviews/T157c/results_full.jsonl" if FULL else "reviews/T157c/results.jsonl")
TIMEOUT = 900
DUMP = os.environ.get("MUTANT_DUMP")

COMBAT = "combat.rs"
MOVE = "movement.rs"

M = {}
_n = [0]


# Rows considered and left out to stay within the brief's 22-30 rows.
SKIPPED = (
    "start check: `all` -> `any`",
    "Repr: engage loses",
    "is_valid: `> 0` -> `> 1`",
    "Serialize: key renamed",
    "engagement_due: `<= limit` -> `<= limit + 1`",
    "engagement_due: failed try_from",
    "Deserialize: refusal checks only the first rule",
)


def m(desc, item, edits):
    if desc.startswith(SKIPPED):
        return
    _n[0] += 1
    M["M%02d" % _n[0]] = (desc, item, edits)


GATE = "    if !definition.engage.iter().all(EngageRule::is_valid) {\n        return Err(CombatError::InvalidEngage);"
SETROUTE_IF = ("        if self.combatants().contains(entity)\n"
               "            && self.combat().is_some_and(|state| {\n"
               "                state.mode == EncounterMode::TurnBased && state.active.is_some()\n"
               "            })\n"
               "        {\n"
               "            return Err(MoveError::TurnBased { entity });\n"
               "        }\n")
DEAD_IF = ("        if is_dead(self, entity) {\n"
           "            return Err(MoveError::DeadCombatant { entity });\n"
           "        }\n")
VALIDATE = "        validate_route(&route, self.nav(), mover.position)?;\n"
S4 = ("    // S4.\n"
      "    if members.is_empty() && sources.iter().all(|source| matches!(source, Source::Skip)) {\n"
      "        return Err(CombatError::NoEntrants);\n"
      "    }\n")
S_C1 = ("    // T157c S-C1: the engagement distances, after every existing start check.\n"
        "    if !definition.engage.iter().all(EngageRule::is_valid) {\n"
        "        return Err(CombatError::InvalidEngage);\n"
        "    }\n")

# ---- S-C1: resolution ----------------------------------------------------------
m("resolution: distance replaced by the constant 131072", "S-C1",
  [(COMBAT, "            crpg_data::EngageTriggerWire::Proximity { distance } => EngageRule::Proximity {\n                distance: *distance,\n            },",
    "            crpg_data::EngageTriggerWire::Proximity { distance } => EngageRule::Proximity {\n                distance: Fx16_16::from_raw(131072 + 0 * distance.to_raw()),\n            },")])
m("definition.engage always empty (resolved list dropped)", "S-C1",
  [(COMBAT, "        movement: turn_movement,\n        engage,\n    };",
    "        movement: turn_movement,\n        engage: Vec::new(),\n    };")])
m("resolution: authored order reversed", "S-C1",
  [(COMBAT, "        .unwrap_or_default()\n        .iter()\n        .map(|trigger| match trigger {",
    "        .unwrap_or_default()\n        .iter()\n        .rev()\n        .map(|trigger| match trigger {")])
m("resolution: absent engage gets a default 1 m rule", "S-C1",
  [(COMBAT, "        .as_deref()\n        .unwrap_or_default()\n        .iter()\n        .map(|trigger| match trigger {",
    "        .as_deref()\n        .unwrap_or(&[crpg_data::EngageTriggerWire::Proximity {\n            distance: Fx16_16::from_raw(65536),\n        }])\n        .iter()\n        .map(|trigger| match trigger {")])
m("InvalidEngage check removed (if false)", "S-C1",
  [(COMBAT, GATE, "    if false {\n        return Err(CombatError::InvalidEngage);")])
m("is_valid: `> 0` -> `>= 0` (zero accepted)", "S-C1",
  [(COMBAT, "Self::Proximity { distance } => distance.to_raw() > 0,",
    "Self::Proximity { distance } => distance.to_raw() >= 0,")])
m("is_valid: `> 0` -> `> 1` (raw 1 refused)", "S-C1",
  [(COMBAT, "Self::Proximity { distance } => distance.to_raw() > 0,",
    "Self::Proximity { distance } => distance.to_raw() > 1,")])
m("InvalidEngage check placed before the NoEntrants check", "S-C1 (place)",
  [(COMBAT, S4 + S_C1,
    "    if !definition.engage.iter().all(EngageRule::is_valid) {\n        return Err(CombatError::InvalidEngage);\n    }\n" + S4)])
m("InvalidEngage check moved after the initiative rolls (RNG drawn before refusal)", "S-C1 (place)",
  [(COMBAT, S_C1, ""),
   (COMBAT, "    for ((member, source), mover) in pending.into_iter().zip(sources).zip(movers) {\n",
    "    if !definition.engage.iter().all(EngageRule::is_valid) {\n        return Err(CombatError::InvalidEngage);\n    }\n    for ((member, source), mover) in pending.into_iter().zip(sources).zip(movers) {\n")])
m("InvalidEngage Display text changed", "S-C1",
  [(COMBAT, 'write!(f, "InvalidEngage at combat/engage")', 'write!(f, "InvalidEngage at combat/engagement")')])
m("start check looks only at the first rule (`take(1)`)", "S-C1",
  [(COMBAT, GATE,
    "    if !definition.engage.iter().take(1).all(EngageRule::is_valid) {\n        return Err(CombatError::InvalidEngage);")])
m("start check: `all` -> `any` (one valid rule hides a bad one)", "S-C1",
  [(COMBAT, GATE,
    "    if !definition.engage.iter().any(EngageRule::is_valid) && !definition.engage.is_empty() {\n        return Err(CombatError::InvalidEngage);")])

# ---- S-C1a: legacy shape, Serialize, Deserialize --------------------------------
m("definition_is_legacy: engage conjunct dropped", "S-C1a",
  [(COMBAT, "        && definition.movement.is_none()\n        && definition.engage.is_empty()\n",
    "        && definition.movement.is_none()\n")])
m("Serialize len: engage term dropped", "S-C1a",
  [(COMBAT, "                + usize::from(self.movement.is_some())\n                + usize::from(!self.engage.is_empty())\n",
    "                + usize::from(self.movement.is_some())\n")])
m("Serialize: engage field never written", "S-C1a",
  [(COMBAT, "            if !self.engage.is_empty() {\n                shape.serialize_field(\"engage\", &self.engage)?;",
    "            if false {\n                shape.serialize_field(\"engage\", &self.engage)?;")])
m("Serialize: engage written even when empty", "S-C1a",
  [(COMBAT, "            if !self.engage.is_empty() {\n                shape.serialize_field(\"engage\", &self.engage)?;",
    "            if true {\n                shape.serialize_field(\"engage\", &self.engage)?;")])
m("Serialize: key renamed \"engages\"", "S-C1a",
  [(COMBAT, 'shape.serialize_field("engage", &self.engage)?;', 'shape.serialize_field("engages", &self.engage)?;')])
m("Deserialize legacy arm: engage dropped", "S-C1a",
  [(COMBAT, "                movement: repr.movement,\n                engage: repr.engage,\n            })\n        } else {",
    "                movement: repr.movement,\n                engage: Vec::new(),\n            })\n        } else {")])
m("Deserialize full arm: engage dropped", "S-C1a",
  [(COMBAT, "                movement: repr.movement,\n                engage: repr.engage,\n            })\n        }\n    }\n}",
    "                movement: repr.movement,\n                engage: Vec::new(),\n            })\n        }\n    }\n}")])
m("Deserialize: non-positive-distance refusal removed", "S-C1a",
  [(COMBAT, "        if !repr.engage.iter().all(EngageRule::is_valid) {\n            return Err(",
    "        if false {\n            return Err(")])
m("Deserialize: refusal message text changed", "S-C1a",
  [(COMBAT, '"engage proximity distance must be positive",', '"engage proximity distance must be non-negative",')])
m("Deserialize: refusal checks only the first rule", "S-C1a",
  [(COMBAT, "        if !repr.engage.iter().all(EngageRule::is_valid) {\n            return Err(",
    "        if !repr.engage.iter().take(1).all(EngageRule::is_valid) {\n            return Err(")])
m("Repr: engage loses #[serde(default)] (missing key refused)", "S-C1a",
  [(COMBAT, "    #[serde(default)]\n    engage: Vec<EngageRule>,", "    engage: Vec<EngageRule>,")])

# ---- S-C2: engagement_due -------------------------------------------------------
m("engagement_due: RealTime test flipped (`!=` -> `==`)", "S-C2",
  [(COMBAT, "    if state.mode != EncounterMode::RealTime\n        || state.pending_mode.is_some()",
    "    if state.mode == EncounterMode::RealTime\n        || state.pending_mode.is_some()")])
m("engagement_due: pending_mode conjunct dropped", "S-C2",
  [(COMBAT, "    if state.mode != EncounterMode::RealTime\n        || state.pending_mode.is_some()\n        || !fight_is_live(world)",
    "    if state.mode != EncounterMode::RealTime\n        || !fight_is_live(world)")])
m("engagement_due: fight_is_live conjunct dropped", "S-C2",
  [(COMBAT, "        || state.pending_mode.is_some()\n        || !fight_is_live(world)\n",
    "        || state.pending_mode.is_some()\n")])
m("engagement_due: dead filter dropped", "S-C2",
  [(COMBAT, "                .filter(|(_, combatant)| !combatant.dead)\n                .filter_map(|(id, _)| world.movers()",
    "                .filter_map(|(id, _)| world.movers()")])
m("engagement_due: mover filter dropped (moverless at the origin)", "S-C2",
  [(COMBAT, ".filter_map(|(id, _)| world.movers().get(id).map(|mover| (id, mover.position())))",
    ".map(|(id, _)| (id, world.movers().get(id).map_or([Fx16_16::ZERO; 3], |mover| mover.position())))")])
m("engagement_due: opposed conjunct dropped", "S-C2",
  [(COMBAT, ".any(|(b, at_b)| opposed(world, *a, *b) && seg_len(*at_a, *at_b) <= limit)",
    ".any(|(_b, at_b)| seg_len(*at_a, *at_b) <= limit)")])
m("engagement_due: `<= limit` -> `< limit`", "S-C2",
  [(COMBAT, "seg_len(*at_a, *at_b) <= limit)", "seg_len(*at_a, *at_b) < limit)")])
m("engagement_due: `<= limit` -> `<= limit + 1`", "S-C2",
  [(COMBAT, "seg_len(*at_a, *at_b) <= limit)", "seg_len(*at_a, *at_b) <= limit + 1)")])
m("engagement_due: some rule -> every rule (`any` -> `all`)", "S-C2",
  [(COMBAT, "    state.definition.engage.iter().any(|rule| match rule {",
    "    state.definition.engage.iter().all(|rule| match rule {")])
m("engagement_due: failed try_from holds the rule (`return false` -> `return true`)", "S-C2",
  [(COMBAT, "            let Ok(limit) = u64::try_from(distance.to_raw()) else {\n                return false;",
    "            let Ok(limit) = u64::try_from(distance.to_raw()) else {\n                return true;")])

# ---- S-C3: set_route head predicate ---------------------------------------------
m("set_route: head conjunct dropped (old rule)", "S-C3",
  [(MOVE, "                state.mode == EncounterMode::TurnBased && state.active.is_some()\n",
    "                state.mode == EncounterMode::TurnBased\n")])
m("set_route: head predicate inverted (`is_some` -> `is_none`)", "S-C3",
  [(MOVE, "state.mode == EncounterMode::TurnBased && state.active.is_some()",
    "state.mode == EncounterMode::TurnBased && state.active.is_none()")])
m("set_route: TurnBased refusal moved after validate_route", "S-C3 (order)",
  [(MOVE, SETROUTE_IF + VALIDATE, VALIDATE + SETROUTE_IF)])
m("set_route: TurnBased refusal moved before the dead check", "S-C3 (order)",
  [(MOVE, DEAD_IF + SETROUTE_IF, SETROUTE_IF + DEAD_IF)])

FILES = [COMBAT, MOVE]

FAST = ["cargo", "test", "-p", "crpg-sim", "--locked", "-j", "2", "--no-fail-fast", "--lib",
        "--test", "engage", "--test", "movement_core", "--test", "movement_core_contract"]
FULLCMD = ["cargo", "test", "-p", "crpg-sim", "--locked", "-j", "2", "--no-fail-fast"]


def read_originals():
    return {name: (SRC / name).read_bytes() for name in FILES}


def apply(originals, edits):
    out = {}
    for name, old, new in edits:
        src = out.get(name, originals[name])
        crlf = b"\r\n" in src
        text = src.decode("utf-8")
        if crlf:
            old = old.replace("\n", "\r\n")
            new = new.replace("\n", "\r\n")
        count = text.count(old)
        if count != 1:
            raise ValueError(f"{name}: anchor matched {count} times: {old[:70]!r}")
        out[name] = text.replace(old, new, 1).encode("utf-8")
    return out


def run_cargo():
    env = dict(os.environ, CARGO_INCREMENTAL="0")
    cmd = FULLCMD if FULL else FAST
    p = subprocess.Popen(cmd, cwd=ROOT, env=env, stdout=subprocess.PIPE,
                         stderr=subprocess.STDOUT, text=True, encoding="utf-8",
                         errors="replace")
    try:
        out, _ = p.communicate(timeout=TIMEOUT)
        return p.returncode, out, False
    except subprocess.TimeoutExpired:
        p.kill()
        try:
            out, _ = p.communicate(timeout=30)
        except Exception:
            out = ""
        return -1, out, True


def classify(rc, out, timed_out):
    if timed_out:
        return "killed", ["<timeout>"]
    if "could not compile" in out or any(ln.startswith("error[") for ln in out.splitlines()):
        return "compile-error", []
    if "test result:" not in out:
        return "compile-error", []
    failed = [ln.split()[1] for ln in out.splitlines()
              if ln.startswith("test ") and ln.rstrip().endswith("FAILED")]
    if rc == 0 and not failed:
        return "survived", []
    return "killed", failed


def main():
    args = sys.argv[1:]
    check = "--check" in args
    baseline = "--baseline" in args
    ids = [a for a in args if not a.startswith("--")] or list(M)
    originals = read_originals()
    if baseline:
        rc, out, to = run_cargo()
        status, failed = classify(rc, out, to)
        tail = [ln for ln in out.splitlines() if "test result" in ln]
        print("baseline", "full" if FULL else "fast", "pass" if status == "survived" else status,
              "rc", rc, failed, len(tail), "result lines;", sorted(set(tail)), flush=True)
        return
    lines = {}
    if not check:
        OUT.parent.mkdir(parents=True, exist_ok=True)
        if OUT.exists():
            for line in OUT.read_text(encoding="utf-8").splitlines():
                if line.strip():
                    lines[json.loads(line)["id"]] = line
        for mid in ids:
            lines.pop(mid, None)

    def flush():
        OUT.write_text("".join(lines[mid] + "\n" for mid in M if mid in lines),
                       encoding="utf-8")

    try:
        for mid in ids:
            desc, item, edits = M[mid]
            rec = {"id": mid, "desc": desc, "contract": item,
                   "edits": [{"file": f, "old": o, "new": n} for f, o, n in edits]}
            t0 = time.time()
            try:
                mutated = apply(originals, edits)
            except ValueError as e:
                rec.update(status="anchor-error", detail=str(e))
                print(mid, "anchor-error", e, flush=True)
                if not check:
                    lines[mid] = json.dumps(rec)
                    flush()
                continue
            if check:
                print(mid, "anchor-ok", desc, flush=True)
                continue
            rc, out, to = (0, "", False)
            try:
                for name, data in mutated.items():
                    (SRC / name).write_bytes(data)
                rc, out, to = run_cargo()
            finally:
                for name in mutated:
                    (SRC / name).write_bytes(originals[name])
            if DUMP:
                Path(DUMP).mkdir(parents=True, exist_ok=True)
                (Path(DUMP) / f"{mid}.cargo.log").write_text(out, encoding="utf-8")
            status, failed = classify(rc, out, to)
            rec.update(status=status, failed_tests=failed,
                       seconds=round(time.time() - t0, 1))
            if status == "compile-error":
                rec["detail"] = [ln for ln in out.splitlines()
                                 if ln.startswith("error")][:3]
            if status == "survived":
                rec["class"] = "UNCLASSIFIED"
            lines[mid] = json.dumps(rec)
            flush()
            print(mid, status, failed, flush=True)
    finally:
        for name, data in originals.items():
            (SRC / name).write_bytes(data)


if __name__ == "__main__":
    main()
