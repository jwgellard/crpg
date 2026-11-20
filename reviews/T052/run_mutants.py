#!/usr/bin/env python3
"""Mutant driver for T052 (crates/crpg-sim: continuation.rs and the T052 lines
of world.rs, history.rs, area.rs, tick.rs).

Applies one mutant at a time by exact text replacement (CRLF-aware, the anchor
must occur exactly once), runs the T052 test union

  cargo test -p crpg-sim --locked -j 2 --lib --test continuation
      --test continuation_contract --test world --test tick --test history
      --test history_golden --test history_v2_golden --test history_v3_golden

with CARGO_INCREMENTAL=0 (timeout 900 s), records one JSON line per mutant in
results.jsonl, and restores the original bytes in a `finally`.

usage: run_mutants.py [--check] [--full] [ids...]
       --check  applies and restores every anchor without cargo (no results).
       --full   runs the full `cargo test -p crpg-sim --locked -j 2` instead
                and writes results-full.jsonl (for survivors of the union run).
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
OUT = ROOT / "reviews/T052/results.jsonl"
OUT_FULL = ROOT / "reviews/T052/results-full.jsonl"
TIMEOUT = 900
TIMEOUT_FULL = 2400
DUMP = os.environ.get("MUTANT_DUMP")

CT = "continuation.rs"
WD = "world.rs"
HS = "history.rs"
AR = "area.rs"
TK = "tick.rs"
FILES = [CT, WD, HS, AR, TK]

M = {}


def m(mid, desc, item, edits):
    M[mid] = (desc, item, edits)


# ---- continuation.rs ------------------------------------------------------
m("M01", "continuation.rs wait_system: saturating_sub(1) -> saturating_sub(2)",
  "B-3",
  [(CT, "record.remaining.saturating_sub(1)", "record.remaining.saturating_sub(2)")])

m("M02", "continuation.rs wait_system: a record at 1 stays 1 (decrement only when remaining > 1)",
  "B-3",
  [(CT,
    "        record.remaining = record.remaining.saturating_sub(1);\n",
    "        if record.remaining > 1 {\n"
    "            record.remaining -= 1;\n"
    "        }\n")])

m("M03", "continuation.rs wait_system: a record at 0 wraps (wrapping_sub instead of saturating_sub)",
  "B-3",
  [(CT, "record.remaining.saturating_sub(1)", "record.remaining.wrapping_sub(1)")])

m("M04", "continuation.rs Display: DialogueOpen renders as NoDialogue",
  "Errors",
  [(CT, '            Self::DialogueOpen { .. } => "DialogueOpen",\n',
    '            Self::DialogueOpen { .. } => "NoDialogue",\n')])

m("M05", 'continuation.rs Display: suffix " at script/state" -> " at script"',
  "Errors",
  [(CT, 'write!(f, "{name} at script/state")', 'write!(f, "{name} at script")')])

# ---- tick.rs --------------------------------------------------------------
m("M06", "tick.rs run_systems: wait_system call removed", "B-3",
  [(TK, "    crate::continuation::wait_system(world);\n", "")])

m("M07", "tick.rs run_systems: wait_system moved before the encounter clock", "B-3",
  [(TK,
    "    encounter_clock_system(world, facts);\n"
    "    // §10 stage 2 (timers, durations, refreshes) — T052: script waits count down.\n"
    "    crate::continuation::wait_system(world);\n",
    "    crate::continuation::wait_system(world);\n"
    "    encounter_clock_system(world, facts);\n")])

m("M08", "tick.rs run_systems: wait_system moved after the movement stage", "B-3",
  [(TK,
    "    // §10 stage 2 (timers, durations, refreshes) — T052: script waits count down.\n"
    "    crate::continuation::wait_system(world);\n",
    ""),
   (TK,
    "    crate::movement::movement_system(world, facts);\n",
    "    crate::movement::movement_system(world, facts);\n"
    "    crate::continuation::wait_system(world);\n")])

# ---- world.rs: despawn ----------------------------------------------------
m("M09", "world.rs despawn: continuations.remove(id) removed", "B-8",
  [(WD, "        self.continuations.remove(id);\n", "")])

m("M10", "world.rs despawn: dialogues.remove(id) removed", "B-8",
  [(WD, "        self.dialogues.remove(id);\n", "")])

m("M11", "world.rs despawn: quests cleared as well (quests must be untouched)", "B-8",
  [(WD, "        self.dialogues.remove(id);\n",
    "        self.dialogues.remove(id);\n        self.quests.clear();\n")])

# ---- world.rs: load checks -------------------------------------------------
m("M12", "world.rs Deserialize L1: continuation liveness check made never-fail", "B-10",
  [(WD,
    "        for (id, _) in repr.continuations.iter() {\n"
    "            if !repr.entities.contains(id) {\n",
    "        for (id, _) in repr.continuations.iter() {\n"
    "            if false && !repr.entities.contains(id) {\n")])

m("M13", "world.rs Deserialize L2: dialogue liveness check made never-fail", "B-10",
  [(WD,
    "        for (id, _) in repr.dialogues.iter() {\n"
    "            if !repr.entities.contains(id) {\n",
    "        for (id, _) in repr.dialogues.iter() {\n"
    "            if false && !repr.entities.contains(id) {\n")])

m("M14", "world.rs Deserialize L3: quest order `>=` -> `>` (duplicates accepted)", "B-10",
  [(WD, "any(|pair| pair[0].quest >= pair[1].quest)", "any(|pair| pair[0].quest > pair[1].quest)")])

m("M15", "world.rs Deserialize L3: windows(2) -> windows(3) (adjacent pair never compared at the end)", "B-10",
  [(WD, "            .windows(2)\n            .any(|pair| pair[0].quest >= pair[1].quest)",
    "            .windows(3)\n            .any(|pair| pair[0].quest >= pair[1].quest)")])

m("M16", "world.rs Deserialize L3: compares state instead of quest", "B-10",
  [(WD, "any(|pair| pair[0].quest >= pair[1].quest)", "any(|pair| pair[0].state >= pair[1].state)")])

m("M17", "world.rs Deserialize L1 text: 'dangling continuation' -> 'dangling dialogue'", "B-10",
  [(WD, '"dangling continuation for non-live {id:?}"', '"dangling dialogue for non-live {id:?}"')])

m("M18", "world.rs Deserialize L2 text: 'dangling dialogue' -> 'dangling continuation'", "B-10",
  [(WD, '"dangling dialogue for non-live {id:?}"', '"dangling continuation for non-live {id:?}"')])

m("M19", "world.rs Deserialize L3 text: 'ascending order' -> 'strictly ascending order'", "B-10",
  [(WD, '"quest progress breaks ascending order"', '"quest progress breaks strictly ascending order"')])

m("M20", "world.rs Deserialize: L1 and L2 swapped (dialogue liveness checked first)", "B-10",
  [(WD,
    "        for (id, _) in repr.continuations.iter() {\n"
    "            if !repr.entities.contains(id) {\n"
    "                return Err(serde::de::Error::custom(format!(\n"
    "                    \"dangling continuation for non-live {id:?}\"\n"
    "                )));\n"
    "            }\n"
    "        }\n"
    "        for (id, _) in repr.dialogues.iter() {\n"
    "            if !repr.entities.contains(id) {\n"
    "                return Err(serde::de::Error::custom(format!(\n"
    "                    \"dangling dialogue for non-live {id:?}\"\n"
    "                )));\n"
    "            }\n"
    "        }\n",
    "        for (id, _) in repr.dialogues.iter() {\n"
    "            if !repr.entities.contains(id) {\n"
    "                return Err(serde::de::Error::custom(format!(\n"
    "                    \"dangling dialogue for non-live {id:?}\"\n"
    "                )));\n"
    "            }\n"
    "        }\n"
    "        for (id, _) in repr.continuations.iter() {\n"
    "            if !repr.entities.contains(id) {\n"
    "                return Err(serde::de::Error::custom(format!(\n"
    "                    \"dangling continuation for non-live {id:?}\"\n"
    "                )));\n"
    "            }\n"
    "        }\n")])

# ---- world.rs: serialization shape and round trip -----------------------------
m("M21", "world.rs World.continuations: skip_serializing_if removed", "B-1",
  [(WD,
    '    #[serde(default, skip_serializing_if = "ComponentStore::is_empty")]\n'
    "    continuations: ComponentStore<ScriptContinuation>,\n",
    "    #[serde(default)]\n"
    "    continuations: ComponentStore<ScriptContinuation>,\n")])

m("M22", "world.rs World.dialogues: skip_serializing_if removed", "B-1",
  [(WD,
    '    #[serde(default, skip_serializing_if = "ComponentStore::is_empty")]\n'
    "    dialogues: ComponentStore<OpenDialogue>,\n",
    "    #[serde(default)]\n"
    "    dialogues: ComponentStore<OpenDialogue>,\n")])

m("M23", "world.rs World.quests: skip_serializing_if removed", "B-1",
  [(WD,
    '    #[serde(default, skip_serializing_if = "Vec::is_empty")]\n'
    "    quests: Vec<QuestProgress>,\n",
    "    #[serde(default)]\n"
    "    quests: Vec<QuestProgress>,\n")])

m("M24", "world.rs Deserialize result: continuations dropped on load", "B-11",
  [(WD, "            continuations: repr.continuations,\n",
    "            continuations: ComponentStore::new(),\n")])

m("M25", "world.rs Deserialize result: dialogues dropped on load", "B-11",
  [(WD, "            dialogues: repr.dialogues,\n",
    "            dialogues: ComponentStore::new(),\n")])

m("M26", "world.rs Deserialize result: quests dropped on load", "B-11",
  [(WD, "            quests: repr.quests,\n", "            quests: Vec::new(),\n")])

# ---- world.rs: due / spawn / set_continuation ---------------------------------
m("M27", "world.rs due_continuations: `remaining == 0` -> `remaining <= 1`", "B-4",
  [(WD, ".filter(|(_, record)| record.remaining == 0)", ".filter(|(_, record)| record.remaining <= 1)")])

m("M28", "world.rs due_continuations: `remaining == 0` -> `remaining != 0`", "B-4",
  [(WD, ".filter(|(_, record)| record.remaining == 0)", ".filter(|(_, record)| record.remaining != 0)")])

m("M29", "world.rs due_continuations: due.sort() removed (store order, not EntityId order)", "B-4",
  [(WD, "        due.sort();\n        due\n", "        due\n")])

m("M30", "world.rs spawn_continuation: the record is not stored", "B-2",
  [(WD, "        self.continuations.insert(id, c);\n        id\n", "        let _ = c;\n        id\n")])

m("M31", "world.rs spawn_continuation: holder also gets an open dialogue", "B-2",
  [(WD, "        self.continuations.insert(id, c);\n        id\n",
    "        self.continuations.insert(id, c);\n"
    "        self.dialogues.insert(\n"
    "            id,\n"
    "            OpenDialogue {\n"
    "                dialogue: c.graph,\n"
    "                node: c.node,\n"
    "            },\n"
    "        );\n"
    "        id\n")])

m("M32", "world.rs set_continuation: returns the new record instead of the old one", "B-5",
  [(WD, "            Some(record) => Ok(std::mem::replace(record, c)),\n",
    "            Some(record) => {\n"
    "                let _ = std::mem::replace(record, c);\n"
    "                Ok(c)\n"
    "            }\n")])

m("M33", "world.rs set_continuation: the record is not replaced (old returned)", "B-5",
  [(WD, "            Some(record) => Ok(std::mem::replace(record, c)),\n",
    "            Some(record) => Ok(*record),\n")])

m("M34", "world.rs set_continuation: non-holder error is NotLive instead of NotContinuation", "B-5",
  [(WD, "            None => Err(ContinuationError::NotContinuation { entity }),\n",
    "            None => Err(ContinuationError::NotLive { entity }),\n")])

# ---- world.rs: dialogues ---------------------------------------------------------
m("M35", "world.rs open_dialogue: NotLive check removed", "B-6",
  [(WD,
    "        if !self.contains(participant) {\n"
    "            return Err(ContinuationError::NotLive {\n"
    "                entity: participant,\n"
    "            });\n"
    "        }\n",
    "")])

m("M36", "world.rs open_dialogue: DialogueOpen check removed (open replaces)", "B-6",
  [(WD,
    "        if self.dialogues.contains(participant) {\n"
    "            return Err(ContinuationError::DialogueOpen {\n"
    "                entity: participant,\n"
    "            });\n"
    "        }\n",
    "")])

m("M37", "world.rs open_dialogue: a not-live participant is refused as DialogueOpen", "B-6",
  [(WD,
    "            return Err(ContinuationError::NotLive {\n"
    "                entity: participant,\n"
    "            });\n",
    "            return Err(ContinuationError::DialogueOpen {\n"
    "                entity: participant,\n"
    "            });\n")])

m("M38", "world.rs set_dialogue_node: returns the new node instead of the old one", "B-6",
  [(WD, "            Some(open) => Ok(std::mem::replace(&mut open.node, node)),\n",
    "            Some(open) => {\n"
    "                open.node = node;\n"
    "                Ok(node)\n"
    "            }\n")])

m("M39", "world.rs set_dialogue_node: none-open refusal is NotLive instead of NoDialogue", "B-6",
  [(WD,
    "            None => Err(ContinuationError::NoDialogue {\n"
    "                entity: participant,\n"
    "            }),\n",
    "            None => Err(ContinuationError::NotLive {\n"
    "                entity: participant,\n"
    "            }),\n")])

m("M40", "world.rs close_dialogue: reads the record without removing it", "B-6",
  [(WD, "        self.dialogues\n            .remove(participant)\n",
    "        self.dialogues\n            .get(participant)\n            .copied()\n")])

# ---- world.rs: quests ---------------------------------------------------------------
m("M41", "world.rs set_quest_state: replacing an existing quest returns None", "B-7",
  [(WD, "            Ok(index) => Some(std::mem::replace(&mut self.quests[index].state, state)),\n",
    "            Ok(index) => {\n"
    "                self.quests[index].state = state;\n"
    "                None\n"
    "            }\n")])

m("M42", "world.rs set_quest_state: new quest appended (push) instead of inserted at its ascending position", "B-7",
  [(WD, "                self.quests.insert(index, QuestProgress { quest, state });\n",
    "                let _ = index;\n"
    "                self.quests.push(QuestProgress { quest, state });\n")])

m("M43", "world.rs remove_quest: returns the state without removing the entry", "B-7",
  [(WD, "        Some(self.quests.remove(index).state)\n",
    "        Some(self.quests[index].state)\n")])

m("M44", "world.rs quest_state: always reads the first entry", "B-7",
  [(WD, "            .map(|index| self.quests[index].state)\n",
    "            .map(|_index| self.quests[0].state)\n")])

# ---- history.rs ------------------------------------------------------------------------
m("M45", "history.rs spawn_continuation: commits no events (Spawned dropped)", "B-12",
  [(HS,
    "        for envelope in &drained {\n"
    "            events.push(translate_legacy(&envelope.payload));\n"
    "        }\n"
    "        self.commit(world, events, tick)?;\n"
    "        Ok(id)\n"
    "    }\n"
    "\n"
    "    /// Journaled [`World::set_continuation`]",
    "        let _ = &drained;\n"
    "        self.commit(world, events, tick)?;\n"
    "        Ok(id)\n"
    "    }\n"
    "\n"
    "    /// Journaled [`World::set_continuation`]")])

m("M46", "history.rs set_continuation: refusal mapped to StringLimit instead of Script(e)", "B-12",
  [(HS,
    "            .set_continuation(entity, c)\n            .map_err(HistoryError::Script)?;\n",
    "            .set_continuation(entity, c)\n            .map_err(|_| HistoryError::StringLimit)?;\n")])

m("M47", "history.rs open_dialogue: staged world is not committed", "B-12",
  [(HS,
    "            .open_dialogue(participant, d)\n"
    "            .map_err(HistoryError::Script)?;\n"
    "        self.commit_script(world)\n",
    "            .open_dialogue(participant, d)\n"
    "            .map_err(HistoryError::Script)?;\n"
    "        drop(world);\n"
    "        Ok(())\n")])

m("M48", "history.rs set_dialogue_node: staged world is not committed", "B-12",
  [(HS,
    "            .set_dialogue_node(participant, node)\n"
    "            .map_err(HistoryError::Script)?;\n"
    "        self.commit_script(world)?;\n",
    "            .set_dialogue_node(participant, node)\n"
    "            .map_err(HistoryError::Script)?;\n"
    "        drop(world);\n")])

m("M49", "history.rs close_dialogue: staged world is not committed", "B-12",
  [(HS,
    "            .close_dialogue(participant)\n"
    "            .map_err(HistoryError::Script)?;\n"
    "        self.commit_script(world)?;\n",
    "            .close_dialogue(participant)\n"
    "            .map_err(HistoryError::Script)?;\n"
    "        drop(world);\n")])

m("M50", "history.rs set_quest_state: staged world is not committed", "B-12",
  [(HS,
    "        let old = world.set_quest_state(quest, state);\n"
    "        self.commit_script(world)?;\n",
    "        let old = world.set_quest_state(quest, state);\n"
    "        drop(world);\n")])

m("M51", "history.rs remove_quest: staged world is not committed", "B-12",
  [(HS,
    "        let old = world.remove_quest(quest);\n"
    "        self.commit_script(world)?;\n",
    "        let old = world.remove_quest(quest);\n"
    "        drop(world);\n")])

m("M52", "history.rs HistoryError Display: Script arm renders a fixed text", "B-12",
  [(HS, '            Self::Script(error) => write!(f, "{error}"),\n',
    '            Self::Script(_) => write!(f, "Script at history"),\n')])

m("M53", "history.rs HistoryError::source: Script arm removed", "B-12",
  [(HS, "            Self::Script(error) => Some(error),\n", "")])

# ---- area.rs ---------------------------------------------------------------------------------
m("M54", "area.rs preflight: continuation OR dialogue -> AND", "B-9",
  [(AR, "source.continuations().contains(entity) || source.dialogues().contains(entity)",
    "source.continuations().contains(entity) && source.dialogues().contains(entity)")])

m("M55", "area.rs preflight: continuation holders no longer refused", "B-9",
  [(AR, "source.continuations().contains(entity) || source.dialogues().contains(entity)",
    "source.dialogues().contains(entity)")])

m("M56", "area.rs preflight: dialogue participants no longer refused", "B-9",
  [(AR, "source.continuations().contains(entity) || source.dialogues().contains(entity)",
    "source.continuations().contains(entity)")])

m("M57", "area.rs preflight: ScriptState refused as ScheduledEntity", "B-9",
  [(AR, "        return Err(AreaError::ScriptState { entity });\n",
    "        return Err(AreaError::ScheduledEntity { entity });\n")])

m("M58", "area.rs preflight: script-state check moved before the ScheduledEntity check", "B-9",
  [(AR,
    "    // T052 (Reading 10): a continuation or an open dialogue cannot move, and\n"
    "    // would otherwise be silently dropped. Checked last.\n"
    "    if source.continuations().contains(entity) || source.dialogues().contains(entity) {\n"
    "        return Err(AreaError::ScriptState { entity });\n"
    "    }\n",
    ""),
   (AR,
    "    if source.timeline().contains(entity) {\n"
    "        return Err(AreaError::ScheduledEntity { entity });\n",
    "    if source.continuations().contains(entity) || source.dialogues().contains(entity) {\n"
    "        return Err(AreaError::ScriptState { entity });\n"
    "    }\n"
    "    if source.timeline().contains(entity) {\n"
    "        return Err(AreaError::ScheduledEntity { entity });\n")])

m("M59", "area.rs preflight: script-state check moved before the PlacementInUse check", "B-9",
  [(AR,
    "    // T052 (Reading 10): a continuation or an open dialogue cannot move, and\n"
    "    // would otherwise be silently dropped. Checked last.\n"
    "    if source.continuations().contains(entity) || source.dialogues().contains(entity) {\n"
    "        return Err(AreaError::ScriptState { entity });\n"
    "    }\n",
    ""),
   (AR,
    "    // T139 T1: one record per placement across areas (B2, K10).\n",
    "    if source.continuations().contains(entity) || source.dialogues().contains(entity) {\n"
    "        return Err(AreaError::ScriptState { entity });\n"
    "    }\n"
    "    // T139 T1: one record per placement across areas (B2, K10).\n")])

m("M60", 'area.rs Display: ScriptState renders as "ScheduledEntity"', "B-9",
  [(AR, '            Self::ScriptState { .. } => "ScriptState",\n',
    '            Self::ScriptState { .. } => "ScheduledEntity",\n')])


# Brief: 35-45 rows. The drafted rows below are pruned to 45 (least weight on the
# B-3/B-4/B-8/B-9/B-10/B-12 items, or a near-duplicate of a kept row), then the
# survivors are numbered M01.. in table order.
_DROPPED = {"M03", "M05", "M16", "M31", "M33", "M34", "M37", "M39", "M44",
            "M49", "M51", "M52", "M53", "M57", "M60"}
M = {f"M{i:02d}": v for i, (k, v) in enumerate(
    ((k, v) for k, v in M.items() if k not in _DROPPED), 1)}


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


UNION = ["--lib", "--test", "continuation", "--test", "continuation_contract",
         "--test", "world", "--test", "tick", "--test", "history",
         "--test", "history_golden", "--test", "history_v2_golden",
         "--test", "history_v3_golden"]


def run_cargo(full):
    env = dict(os.environ, CARGO_INCREMENTAL="0")
    cmd = ["cargo", "test", "-p", "crpg-sim", "--locked", "-j", "2"]
    if not full:
        cmd += UNION
    timeout = TIMEOUT_FULL if full else TIMEOUT
    p = subprocess.Popen(cmd, cwd=ROOT, env=env, stdout=subprocess.PIPE,
                         stderr=subprocess.STDOUT, text=True, encoding="utf-8",
                         errors="replace")
    try:
        out, _ = p.communicate(timeout=timeout)
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
    full = "--full" in args
    ids = [a for a in args if not a.startswith("--")] or list(M)
    out_path = OUT_FULL if full else OUT
    originals = read_originals()
    lines = {}
    if not check:
        out_path.parent.mkdir(parents=True, exist_ok=True)
        if out_path.exists():
            for line in out_path.read_text(encoding="utf-8").splitlines():
                if line.strip():
                    lines[json.loads(line)["id"]] = line
        for mid in ids:
            lines.pop(mid, None)

    def flush():
        out_path.write_text(
            "".join(lines[mid] + "\n" for mid in M if mid in lines), encoding="utf-8")

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
