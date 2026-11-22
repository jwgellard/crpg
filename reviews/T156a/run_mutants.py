#!/usr/bin/env python3
"""Mutant driver for T156a (core/mod.rs, core/authority/v4.rs; crpg-godot lib tests).

Applies one mutant at a time by exact text replacement, runs
`cargo test -p crpg-godot --locked -j 2 --lib` with CARGO_INCREMENTAL=0,
records one JSON line per mutant in results.jsonl, and restores the original
bytes in a `finally`.

usage: run_mutants.py [--check] [--baseline] [ids...]
       --check    applies every anchor without cargo (no results written).
       --baseline runs the unmutated tree once (no results written).
       no ids: all rows, in table order.
"""
import json
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "crates/crpg-godot/src/core"
OUT = ROOT / "reviews/T156a/results.jsonl"
TIMEOUT = 600
DUMP = os.environ.get("MUTANT_DUMP")

MOD = "mod.rs"
V4 = "authority/v4.rs"

M = {}
_n = [0]


def m(desc, item, edits):
    _n[0] += 1
    M["M%02d" % _n[0]] = (desc, item, edits)


# ---- A-1: the NpcDriver branch (mod.rs) --------------------------------------
m("open_v4_ai_with passes NpcDriver::Peer (wrapper opens a peer session)", "A-1",
  [(MOD, "            EncounterMode::TurnBased,\n            NpcDriver::ServerAi,\n",
    "            EncounterMode::TurnBased,\n            NpcDriver::Peer,\n")])
m("open_v4_with passes NpcDriver::ServerAi (existing open_v4 loses the peer NPC)",
  "A-1/A-6",
  [(MOD, "            EncounterMode::TurnBased,\n            NpcDriver::Peer,\n",
    "            EncounterMode::TurnBased,\n            NpcDriver::ServerAi,\n")])
m("open_v4_rt_with passes NpcDriver::ServerAi (RT open loses the peer NPC)",
  "A-1/A-6",
  [(MOD, "            EncounterMode::RealTime,\n            NpcDriver::Peer,\n",
    "            EncounterMode::RealTime,\n            NpcDriver::ServerAi,\n")])
m("open_v4_ai_with opens RealTime instead of TurnBased", "A-1",
  [(MOD, "            EncounterMode::TurnBased,\n            NpcDriver::ServerAi,\n",
    "            EncounterMode::RealTime,\n            NpcDriver::ServerAi,\n")])
m("open_v4_mode forwards NpcDriver::Peer to open_v4_guarded (driver dropped)",
  "A-1",
  [(MOD, "                mode,\n                driver,\n            )\n",
    "                mode,\n                NpcDriver::Peer,\n            )\n")])
m("npc_actors: ServerAi branch yields None (no actors clone)", "A-1",
  [(MOD, "NpcDriver::ServerAi => Some(prefix.npc_actors.clone()),",
    "NpcDriver::ServerAi => None,")])
m("npc_actors: Peer branch also clones (clone under Peer)", "A-1",
  [(MOD, "            NpcDriver::Peer => None,\n",
    "            NpcDriver::Peer => Some(prefix.npc_actors.clone()),\n")])
m("rows: every combatant's side replaced by the empty string", "A-1",
  [(MOD, "(*entity, prefix.presentation.combatant(*placement).1)",
    "(*entity, String::new())")])
m("rows: bound order reversed", "A-1/A-2",
  [(MOD, "                    .bound\n                    .iter()\n                    .map(|(entity, placement)| {\n                        (*entity, prefix.presentation.combatant",
    "                    .bound\n                    .iter()\n                    .rev()\n                    .map(|(entity, placement)| {\n                        (*entity, prefix.presentation.combatant")])
m("ServerAiHook built with an empty actors list", "A-1",
  [(MOD, "ServerAiHook::new(npc, sides, npc_actors).into_hook()",
    "ServerAiHook::new(npc, sides, Vec::new()).into_hook()")])
m("ai_sides Err arm: fault swallowed (AI hook built from empty Sides, open succeeds)",
  "A-1/A-2",
  [(MOD, "                    Err(fault) => return self.fail_fault(fault),\n                }\n            }\n            (_, _)",
    "                    Err(_) => ServerAiHook::new(npc, crpg_ai::Sides::new(Vec::new()).unwrap(), npc_actors).into_hook(),\n                }\n            }\n            (_, _)")])

# ---- A-2: ai_sides ------------------------------------------------------------
m("ai_sides: ascending numbering reversed (names iterated rev)", "A-2",
  [(V4, "        .into_iter()\n        .enumerate()\n        .map(|(index, side)| (side, SideId(index as u32)))",
    "        .into_iter()\n        .rev()\n        .enumerate()\n        .map(|(index, side)| (side, SideId(index as u32)))")])
m("ai_sides: numbering starts at 1", "A-2",
  [(V4, "SideId(index as u32)))", "SideId(index as u32 + 1)))")])
m("ai_sides: every side is SideId(0)", "A-2",
  [(V4, "SideId(index as u32)))", "SideId(0 * index as u32)))")])
m("ai_sides: duplicate maps to code BadArgument", "A-2",
  [(V4, "        code: BridgeCode::StartFailed,\n        detail: error.to_string(),\n    })\n}\n\n/// The server's MVP AI",
    "        code: BridgeCode::BadArgument,\n        detail: error.to_string(),\n    })\n}\n\n/// The server's MVP AI")])
m("ai_sides: duplicate detail text dropped", "A-2",
  [(V4, "        detail: error.to_string(),\n    })\n}\n\n/// The server's MVP AI",
    "        detail: String::new(),\n    })\n}\n\n/// The server's MVP AI")])

# ---- A-3: AiPacer -------------------------------------------------------------
m("AI_STEP_DELAY_MS 600 -> 300", "A-3",
  [(V4, "pub(crate) const AI_STEP_DELAY_MS: u64 = 600;",
    "pub(crate) const AI_STEP_DELAY_MS: u64 = 300;")])
m("pacer: not-the-AI's-turn no longer disarms", "A-3",
  [(V4, "        if !ai_turn {\n            self.due = None;\n            return false;",
    "        if !ai_turn {\n            return false;")])
m("pacer: arm uses wrapping_add (no saturation)", "A-3",
  [(V4, "Some(now_ms.saturating_add(AI_STEP_DELAY_MS));\n                false",
    "Some(now_ms.wrapping_add(AI_STEP_DELAY_MS));\n                false")])
m("pacer: arming step returns true (fires at once)", "A-3",
  [(V4, "AI_STEP_DELAY_MS));\n                false", "AI_STEP_DELAY_MS));\n                true")])
m("pacer: fire test `now_ms >= due` -> `>`", "A-3",
  [(V4, "Some(due) if now_ms >= due => {", "Some(due) if now_ms > due => {")])
m("pacer: firing does not disarm", "A-3",
  [(V4, "            Some(due) if now_ms >= due => {\n                self.due = None;\n                true",
    "            Some(due) if now_ms >= due => {\n                true")])
m("pacer: firing re-arms one delay ahead instead of disarming", "A-3",
  [(V4, "            Some(due) if now_ms >= due => {\n                self.due = None;\n                true",
    "            Some(due) if now_ms >= due => {\n                self.due = Some(now_ms.saturating_add(AI_STEP_DELAY_MS));\n                true")])

# ---- A-4: ServerAiHook::run ---------------------------------------------------
m("run: is_closed check removed (take_delivery_entries still returns Closed)",
  "A-4(1)",
  [(V4, "discarding its report.\n    pub(crate) fn run(&mut self, host: &mut Host, now_ms: u64) -> Result<(), HostError> {\n        if host.is_closed() {\n            return Err(HostError::Closed);\n        }\n",
    "discarding its report.\n    pub(crate) fn run(&mut self, host: &mut Host, now_ms: u64) -> Result<(), HostError> {\n")])
m("run: delivery acknowledgement removed", "A-4(2)",
  [(V4, "host.acknowledge_delivery(self.npc, last.ordinal)?;\n        }\n        let ai_turn",
    "let _ = last;\n        }\n        let ai_turn")])
m("run: acknowledges the first entry, not the last", "A-4(2)",
  [(V4, "host.acknowledge_delivery(self.npc, last.ordinal)?;\n        }\n        let ai_turn",
    "host.acknowledge_delivery(self.npc, entries[0].ordinal)?;\n        }\n        let ai_turn")])
m("run: ai_turn inverted (`is_empty()` without the negation)", "A-4(3)",
  [(V4, "let ai_turn = !host.legal_intents(self.npc)?.is_empty();",
    "let ai_turn = host.legal_intents(self.npc)?.is_empty();")])
m("run: ai_turn always true (legal-options gate removed)", "A-4(3)",
  [(V4, "let ai_turn = !host.legal_intents(self.npc)?.is_empty();",
    "let ai_turn = true;")])
m("run: pacer gets now = 0", "A-4(4)",
  [(V4, "if self.pacer.step(now_ms, ai_turn) {", "if self.pacer.step(0, ai_turn) {")])
m("run: drive_ai gets an empty actors list", "A-4(4)",
  [(V4, "host.drive_ai(&self.sides, &self.actors, MVP_AI_BUDGET)?;",
    "host.drive_ai(&self.sides, &[], MVP_AI_BUDGET)?;")])
m("run: drive_ai budget decisions_per_turn = 0", "A-4(4)",
  [(V4, "host.drive_ai(&self.sides, &self.actors, MVP_AI_BUDGET)?;",
    "host.drive_ai(&self.sides, &self.actors, crpg_server::host::AiBudget { decisions_per_turn: 0, ..MVP_AI_BUDGET })?;")])
m("run: drive_ai called twice per firing", "A-4(4)",
  [(V4, "            host.drive_ai(&self.sides, &self.actors, MVP_AI_BUDGET)?;\n",
    "            host.drive_ai(&self.sides, &self.actors, MVP_AI_BUDGET)?;\n            host.drive_ai(&self.sides, &self.actors, MVP_AI_BUDGET)?;\n")])
m("run: drive_ai error swallowed", "A-4(4)",
  [(V4, "host.drive_ai(&self.sides, &self.actors, MVP_AI_BUDGET)?;",
    "let _ = host.drive_ai(&self.sides, &self.actors, MVP_AI_BUDGET);")])
m("run: drain and legal-options read moved after the pacer step/drive_ai",
  "A-4",
  [(V4, "        let entries = host.take_delivery_entries(self.npc)?;\n        if let Some(last) = entries.last() {\n            host.acknowledge_delivery(self.npc, last.ordinal)?;\n        }\n        let ai_turn = !host.legal_intents(self.npc)?.is_empty();\n        if self.pacer.step(now_ms, ai_turn) {\n            host.drive_ai(&self.sides, &self.actors, MVP_AI_BUDGET)?;\n        }\n",
    "        let ai_turn = !host.legal_intents(self.npc)?.is_empty();\n        if self.pacer.step(now_ms, ai_turn) {\n            host.drive_ai(&self.sides, &self.actors, MVP_AI_BUDGET)?;\n        }\n        let entries = host.take_delivery_entries(self.npc)?;\n        if let Some(last) = entries.last() {\n            host.acknowledge_delivery(self.npc, last.ordinal)?;\n        }\n")])
m("into_hook: run gets now = 0", "A-4",
  [(V4, "MVP_AI_BUDGET)?;\n        }\n        Ok(())\n    }\n\n    /// The boxed hook `LocalHost::set_ai` takes: `run` on the moved `self`.\n    pub(crate) fn into_hook(mut self) -> AiHook {\n        Box::new(move |host: &mut Host, now_ms: u64| self.run(host, now_ms))",
    "MVP_AI_BUDGET)?;\n        }\n        Ok(())\n    }\n\n    /// The boxed hook `LocalHost::set_ai` takes: `run` on the moved `self`.\n    pub(crate) fn into_hook(mut self) -> AiHook {\n        Box::new(move |host: &mut Host, now_ms: u64| self.run(host, 0 * now_ms))")])

FILES = [MOD, V4]


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
    cmd = ["cargo", "test", "-p", "crpg-godot", "--locked", "-j", "2", "--lib"]
    p = subprocess.Popen(cmd, cwd=ROOT, env=env, stdout=subprocess.PIPE,
                         stderr=subprocess.STDOUT, text=True, encoding="utf-8",
                         errors="replace")
    try:
        out, _ = p.communicate(timeout=TIMEOUT)
        return p.returncode, out, False
    except subprocess.TimeoutExpired:
        subprocess.run(["taskkill", "/F", "/T", "/PID", str(p.pid)],
                       capture_output=True)
        try:
            out, _ = p.communicate(timeout=30)
        except Exception:
            out = ""
        return -1, out, True


def classify(rc, out, timed_out):
    if timed_out:
        return "killed", ["<timeout>"]
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
        print("baseline", "pass" if status == "survived" else status, "rc", rc,
              failed, tail, flush=True)
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
