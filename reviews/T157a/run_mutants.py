#!/usr/bin/env python3
"""Mutant driver for T157a (crpg-sim: combat.rs, creature.rs, world.rs).

Applies one mutant at a time by exact text replacement (the old text must occur
exactly once, else `anchor-error`), runs the crate's tests with
CARGO_INCREMENTAL=0, records one JSON line per mutant in results.jsonl, and
restores the original bytes in a `finally`. CRLF files are handled by
converting the edit's `\n` to `\r\n`.

usage: run_mutants.py [--check] [--baseline] [--full] [ids...]
       --check     applies every anchor without cargo (no results written).
       --baseline  runs the unmutated tree once (no results written).
       --full      runs the whole crate (`cargo test -p crpg-sim`) instead of
                   the fast command (`--lib --test sides`); results go to
                   results_full.jsonl.
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
HERE = ROOT / "reviews/T157a"
TIMEOUT = 900

COMBAT = "combat.rs"
CREATURE = "creature.rs"
WORLD = "world.rs"

OLD_COUNT = "world.combatants().iter().filter(|(_, state)| !state.dead).count()"

M = {}
_n = [0]


def m(desc, item, edits):
    _n[0] += 1
    M["M%02d" % _n[0]] = (desc, item, edits)


# ---- S-A1: the fields ---------------------------------------------------------
m("Combatant.faction: skip_serializing_if dropped (null written when absent)", "S-A1",
  [(COMBAT, '    #[serde(default, skip_serializing_if = "Option::is_none")]\n    faction: Option<Ulid>,\n}',
    '    #[serde(default)]\n    faction: Option<Ulid>,\n}')])
m("CreatureState.faction: skip_serializing_if dropped (null written when absent)", "S-A1",
  [(CREATURE, '    #[serde(default, skip_serializing_if = "Option::is_none")]\n    pub(crate) faction: Option<Ulid>,',
    '    #[serde(default)]\n    pub(crate) faction: Option<Ulid>,')])
m("Combatant.faction: never serialized (skip_serializing)", "S-A1",
  [(COMBAT, '    #[serde(default, skip_serializing_if = "Option::is_none")]\n    faction: Option<Ulid>,\n}',
    '    #[serde(default, skip_serializing)]\n    faction: Option<Ulid>,\n}')])
m("CreatureState.faction: never serialized (skip_serializing)", "S-A1",
  [(CREATURE, '    #[serde(default, skip_serializing_if = "Option::is_none")]\n    pub(crate) faction: Option<Ulid>,',
    '    #[serde(default, skip_serializing)]\n    pub(crate) faction: Option<Ulid>,')])

# ---- S-A2: the copy points ----------------------------------------------------
m("prefab spawn: participant combatant takes no faction (Pending.faction = None)", "S-A2",
  [(COMBAT, "            faction: creature.faction,\n", "            faction: None,\n")])
m("register_creature: record takes no faction", "S-A2",
  [(CREATURE, "            faction: spec.creature.faction,", "            faction: None,")])
m("start: Source::Enter record->combatant drops the faction", "S-A2",
  [(COMBAT, "                let entered = enter_combatant(&definition, record, 0, member.initiative);",
    "                let mut entered = enter_combatant(&definition, record, 0, member.initiative);\n                entered.faction = None;")])
m("start: member seat record->combatant drops the faction", "S-A2",
  [(COMBAT, "        let entered = enter_combatant(&definition, record, 0, seat.initiative);",
    "        let mut entered = enter_combatant(&definition, record, 0, seat.initiative);\n        entered.faction = None;")])
m("join: joiner record->combatant drops the faction", "S-A2",
  [(COMBAT, "    let entered = enter_combatant(&definition, record, round, key);",
    "    let mut entered = enter_combatant(&definition, record, round, key);\n    entered.faction = None;")])
m("exit_record: record takes no faction back (end and leave)", "S-A2",
  [(COMBAT, "        faction: combatant.faction,\n    })", "        faction: None,\n    })")])

# ---- S-A3: side_of / opposed --------------------------------------------------
m("side_of: a combatant is always Alone (faction ignored)", "S-A3",
  [(COMBAT, "        return Some(side_for(combatant.faction, entity));",
    "        return Some(Side::Alone(entity));")])
m("side_of: a creature record is always Alone (record faction ignored)", "S-A3",
  [(COMBAT, "        .map(|record| side_for(record.faction, entity))",
    "        .map(|_| Side::Alone(entity))")])
m("opposed: `side_a != side_b` -> `==`", "S-A3",
  [(COMBAT, "(Some(side_a), Some(side_b)) => side_a != side_b,",
    "(Some(side_a), Some(side_b)) => side_a == side_b,")])
m("opposed: an entity with no side is opposed (`_ => false` -> `true`)", "S-A3",
  [(COMBAT, "        _ => false,\n    }\n}\n\n/// An encounter exists",
    "        _ => true,\n    }\n}\n\n/// An encounter exists")])

# ---- S-A4: fight_is_live / live_sides -----------------------------------------
m("fight_is_live: `combat().is_some()` check removed", "S-A4",
  [(COMBAT, "    world.combat().is_some() && live_sides(world) >= 2\n",
    "    live_sides(world) >= 2\n")])
m("fight_is_live: `>= 2` -> `>= 1`", "S-A4",
  [(COMBAT, "    world.combat().is_some() && live_sides(world) >= 2\n",
    "    world.combat().is_some() && live_sides(world) >= 1\n")])
m("live_sides: dead filter removed (dead combatants count)", "S-A4",
  [(COMBAT, "        .filter(|(_, state)| !state.dead)\n        .map(|(id, state)| side_for(state.faction, id))",
    "        .filter(|(_, _)| true)\n        .map(|(id, state)| side_for(state.faction, id))")])
m("live_sides: every faction-less combatant shares one side (zero faction)", "S-A4",
  [(COMBAT, "        .map(|(id, state)| side_for(state.faction, id))",
    "        .map(|(_, state)| Side::Faction(state.faction.unwrap_or(Ulid::from_u128(0))))")])
m("live_sides: factions ignored (every combatant its own side = old living count)", "S-A4",
  [(COMBAT, "        .map(|(id, state)| side_for(state.faction, id))",
    "        .map(|(id, _)| Side::Alone(id))")])

# ---- S-A5: one row per site, live_sides replaced by the old living count -------
m("S-A5 join_encounter J2 (RT terminal) counts the living", "S-A5 join",
  [(COMBAT, "        EncounterMode::RealTime => live_sides(world) < 2,",
    "        EncounterMode::RealTime => " + OLD_COUNT + " < 2,")])
m("S-A5 admit Stop (RT terminal gate) counts the living", "S-A5 admit-stop",
  [(COMBAT, "                EncounterMode::RealTime => {\n                    if live_sides(world) < 2 {",
    "                EncounterMode::RealTime => {\n                    if " + OLD_COUNT + " < 2 {")])
m("S-A5 admit UseAbility (RT/Plan terminal gate) counts the living", "S-A5 admit-use",
  [(COMBAT, "(Admission::Execute, EncounterMode::RealTime) | (Admission::Plan, _) => {\n                    if live_sides(world) < 2 {",
    "(Admission::Execute, EncounterMode::RealTime) | (Admission::Plan, _) => {\n                    if " + OLD_COUNT + " < 2 {")])
m("S-A5 perform_action action end counts the living", "S-A5 action-end",
  [(COMBAT, "            let sides = live_sides(world);", "            let sides = " + OLD_COUNT + ";")])
m("S-A5 advance_tracked empty-timeline gate counts the living", "S-A5 advance-empty",
  [(COMBAT, "    if world.timeline().is_empty() {\n        if live_sides(world) >= 2 {",
    "    if world.timeline().is_empty() {\n        if " + OLD_COUNT + " >= 2 {")])
m("S-A5 advance_tracked head gate counts the living", "S-A5 advance-head",
  [(COMBAT, "    } else {\n        if live_sides(world) >= 2 {",
    "    } else {\n        if " + OLD_COUNT + " >= 2 {")])
m("S-A5 rt_running counts the living", "S-A5 rt_running",
  [(COMBAT, "        && live_sides(world) >= 2\n", "        && " + OLD_COUNT + " >= 2\n")])
m("S-A5 stage-3 per-actor break counts the living", "S-A5 stage3-break",
  [(COMBAT, "    for actor in due {\n        if live_sides(world) < 2 {",
    "    for actor in due {\n        if " + OLD_COUNT + " < 2 {")])
m("S-A5 stage-3 end report counts the living", "S-A5 stage3-end",
  [(COMBAT, "    if live_sides(world) < 2 {\n        // The pass made",
    "    if " + OLD_COUNT + " < 2 {\n        // The pass made")])
m("S-A5 remove_combatant_tracked 'before' count is the living count", "S-A5 leave-before",
  [(WORLD, "(state.mode == EncounterMode::RealTime).then(|| crate::combat::live_sides(self))",
    "(state.mode == EncounterMode::RealTime).then(|| self.combatants.iter().filter(|(_, combatant)| !combatant.dead()).count())")])
m("S-A5 remove_combatant_tracked 'after' count is the living count", "S-A5 leave-after",
  [(WORLD, "            let after = crate::combat::live_sides(self);",
    "            let after = self.combatants.iter().filter(|(_, combatant)| !combatant.dead()).count();")])

FILES = [COMBAT, CREATURE, WORLD]


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


def run_cargo(full):
    env = dict(os.environ, CARGO_INCREMENTAL="0")
    cmd = ["cargo", "test", "-p", "crpg-sim", "--locked", "-j", "2", "--no-fail-fast"]
    if not full:
        cmd += ["--lib", "--test", "sides"]
    p = subprocess.Popen(cmd, cwd=ROOT, env=env, stdout=subprocess.PIPE,
                         stderr=subprocess.STDOUT, text=True, encoding="utf-8",
                         errors="replace")
    try:
        out, _ = p.communicate(timeout=TIMEOUT)
        return p.returncode, out, False
    except subprocess.TimeoutExpired:
        subprocess.run(["taskkill", "/F", "/T", "/PID", str(p.pid)], capture_output=True)
        try:
            out, _ = p.communicate(timeout=30)
        except Exception:
            out = ""
        return -1, out, True


def classify(rc, out, timed_out):
    if timed_out:
        return "killed", ["<timeout>"]
    if "could not compile" in out or "test result:" not in out:
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
    full = "--full" in args
    ids = [a for a in args if not a.startswith("--")] or list(M)
    out_path = HERE / ("results_full.jsonl" if full else "results.jsonl")
    originals = read_originals()
    if baseline:
        rc, out, to = run_cargo(full)
        status, failed = classify(rc, out, to)
        tail = [ln for ln in out.splitlines() if "test result" in ln]
        print("baseline", "pass" if status == "survived" else status, "rc", rc,
              failed, tail, flush=True)
        return
    lines = {}
    if not check:
        if out_path.exists():
            for line in out_path.read_text(encoding="utf-8").splitlines():
                if line.strip():
                    lines[json.loads(line)["id"]] = line
        for mid in ids:
            lines.pop(mid, None)

    def flush():
        out_path.write_text("".join(lines[mid] + "\n" for mid in M if mid in lines),
                            encoding="utf-8", newline="\n")

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
                rc, out, to = run_cargo(full)
            finally:
                for name in mutated:
                    (SRC / name).write_bytes(originals[name])
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
