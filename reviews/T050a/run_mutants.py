#!/usr/bin/env python3
"""Mutant driver for T050a (crates/crpg-godot/src/core/mod.rs, lib tests).

Applies one mutant at a time by exact text replacement in
`crates/crpg-godot/src/core/mod.rs`, runs
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
OUT = ROOT / "reviews/T050a/results.jsonl"
TIMEOUT = 600
DUMP = os.environ.get("MUTANT_DUMP")

MOD = "mod.rs"
GUARD_CALL = ("        let prefix = match self.open_prefix(campaign_dir, encounter_id, "
              "player_side, seed, mode) {\n")
PRECHECK = (
    "        if seed < 0 || (!encounter_id.is_empty() && Ulid::from_str(encounter_id).is_err()) {\n"
    "            return self.fail(BridgeCode::BadArgument, Some(\"bad argument\"));\n"
    "        }\n"
    "        let result = catch_unwind(AssertUnwindSafe(|| {\n"
    "            self.open_v4_guarded(")
ARMS = ("State::Closed | State::Open(_) | State::Fenced(_) => Vec::new(),")

M = {}


def m(mid, desc, item, edits):
    M[mid] = (desc, item, edits)


# ---- W-1: mode wiring -------------------------------------------------------
m("A01", "open_v4_with: passes EncounterMode::RealTime instead of TurnBased",
  "W-1",
  [(MOD, "            clock,\n            EncounterMode::TurnBased,\n",
    "            clock,\n            EncounterMode::RealTime,\n")])

m("A02", "open_v4_rt_with: passes EncounterMode::TurnBased instead of RealTime",
  "W-1",
  [(MOD, "            clock,\n            EncounterMode::RealTime,\n",
    "            clock,\n            EncounterMode::TurnBased,\n")])

m("A03", "open_v4_guarded: open_prefix gets constant TurnBased (mode ignored)",
  "W-1",
  [(MOD, GUARD_CALL,
    "        let prefix = match self.open_prefix(campaign_dir, encounter_id, "
    "player_side, seed, EncounterMode::TurnBased) {\n")])

m("A04", "open_prefix: start_encounter_in_mode gets constant RealTime"
  " (v2 open and TB v4 become real time)", "W-1/W-4",
  [(MOD, "start_encounter_in_mode(&parts.spec(encounter), mode)",
    "start_encounter_in_mode(&parts.spec(encounter), EncounterMode::RealTime)")])

m("A05", "open_on (v2): open_prefix gets RealTime instead of TurnBased",
  "W-4",
  [(MOD, "            seed,\n            EncounterMode::TurnBased,\n        ) {\n",
    "            seed,\n            EncounterMode::RealTime,\n        ) {\n")])

m("A06", "open_v4_mode: already-open check hoisted before the bad-argument"
  " check (steps swapped)", "W-1/W5",
  [(MOD, PRECHECK,
    "        if !matches!(self.state, State::Closed) {\n"
    "            return self.fail(BridgeCode::AlreadyOpen, Some(\"session already open\"));\n"
    "        }\n" + PRECHECK)])

m("A07", "open_v4_mode: negative-seed test removed from the pre-guard check"
  " (falls through to open_prefix 'negative seed')", "W-1/W5",
  [(MOD, PRECHECK,
    "        if (!encounter_id.is_empty() && Ulid::from_str(encounter_id).is_err()) {\n"
    "            return self.fail(BridgeCode::BadArgument, Some(\"bad argument\"));\n"
    "        }\n"
    "        let result = catch_unwind(AssertUnwindSafe(|| {\n"
    "            self.open_v4_guarded(")])

m("A08", "open_v4_mode: malformed-encounter-id test removed from the pre-guard"
  " check", "W-1/W5",
  [(MOD, PRECHECK,
    "        if seed < 0 {\n"
    "            return self.fail(BridgeCode::BadArgument, Some(\"bad argument\"));\n"
    "        }\n"
    "        let result = catch_unwind(AssertUnwindSafe(|| {\n"
    "            self.open_v4_guarded(")])

ALREADY_TAIL = (
    "                return self.fail(BridgeCode::AlreadyOpen, Some(\"session already open\"))\n"
    "            }\n"
    "            State::Closed => {}\n"
    "        }\n")

m("A09", "open_v4_guarded: already-open return code AlreadyOpen -> BadArgument",
  "W-1/W5",
  [(MOD, ALREADY_TAIL + GUARD_CALL,
    ALREADY_TAIL.replace("AlreadyOpen", "BadArgument") + GUARD_CALL)])

m("A10", "open_v4_guarded: already-open check skipped for a live OpenV4 state"
  " (arm `State::OpenV4(_)` moved to the Closed arm)", "W-1/W5",
  [(MOD,
    "            State::Open(_) | State::OpenV4(_) | State::Fenced(_) => {\n" + ALREADY_TAIL + GUARD_CALL,
    "            State::Open(_) | State::Fenced(_) => {\n"
    + ALREADY_TAIL.replace("State::Closed => {}", "State::Closed | State::OpenV4(_) => {}")
    + GUARD_CALL)])

m("A11", "open_prefix: failed start_encounter_in_mode returns code NoEncounter"
  " instead of StartFailed", "W-1",
  [(MOD,
    "        if let Err(error) = history.start_encounter_in_mode(&parts.spec(encounter), mode) {\n"
    "            return Err(self.fail(BridgeCode::StartFailed, Some(&error.to_string())));\n",
    "        if let Err(error) = history.start_encounter_in_mode(&parts.spec(encounter), mode) {\n"
    "            return Err(self.fail(BridgeCode::NoEncounter, Some(&error.to_string())));\n")])

m("A12", "open_v4_rt: public wrapper calls open_v4_with (turn-based) instead of"
  " open_v4_rt_with", "W-1",
  [(MOD,
    "        self.open_v4_rt_with(\n            campaign_dir,\n            encounter_id,\n"
    "            player_side,\n            seed,\n            SystemClock::new(),\n        )\n",
    "        self.open_v4_with(\n            campaign_dir,\n            encounter_id,\n"
    "            player_side,\n            seed,\n            SystemClock::new(),\n        )\n")])

# ---- W-2: controlled() ------------------------------------------------------
m("A13", "open_v4_guarded: controlled.sort_unstable() removed", "W-2",
  [(MOD, "        controlled.sort_unstable();\n", "")])

m("A14", "open_v4_guarded: controlled sorted descending", "W-2",
  [(MOD, "        controlled.sort_unstable();\n",
    "        controlled.sort_unstable_by(|a, b| b.cmp(a));\n")])

m("A15", "open_v4_guarded: controlled id offset by one after client::wide"
  " (client::wide(id.get()) + 1)", "W-2",
  [(MOD, "Ok(Some(id)) => controlled.push(client::wide(id.get())),",
    "Ok(Some(id)) => controlled.push(client::wide(id.get()) + 1),")])

m("A16", "open_v4_guarded: only the first player actor is mapped (.take(1))",
  "W-2",
  [(MOD, "        for entity in &player_actors {\n",
    "        for entity in player_actors.iter().take(1) {\n")])

m("A17", "open_v4_guarded: replica id looked up through the npc peer instead"
  " of the player peer", "W-2",
  [(MOD, "match host.replica_id(player, *entity) {",
    "match host.replica_id(npc, *entity) {")])

m("A18", "open_v4_guarded: LiveV4 stores an empty controlled list", "W-2",
  [(MOD, "            seen_refused: 0,\n            controlled,\n",
    "            seen_refused: 0,\n            controlled: Vec::new(),\n")])

m("A19", "open_v4_guarded: real-time open clears controlled", "W-2",
  [(MOD, "        controlled.sort_unstable();\n",
    "        controlled.sort_unstable();\n"
    "        if matches!(mode, EncounterMode::RealTime) {\n"
    "            controlled.clear();\n        }\n")])

m("A20", "open_v4_guarded: turn-based open clears controlled", "W-2",
  [(MOD, "        controlled.sort_unstable();\n",
    "        controlled.sort_unstable();\n"
    "        if matches!(mode, EncounterMode::TurnBased) {\n"
    "            controlled.clear();\n        }\n")])

m("A21", "controlled(): OpenV4 arm returns an empty Vec", "W-2",
  [(MOD, "State::OpenV4(live) => live.controlled.clone(),",
    "State::OpenV4(_live) => Vec::new(),")])

m("A22", "controlled(): Open (v2) arm returns vec![0]", "W-2",
  [(MOD, ARMS,
    "State::Closed | State::Fenced(_) => Vec::new(),\n"
    "            State::Open(_) => vec![0],")])

m("A23", "controlled(): Fenced arm returns vec![0]", "W-2",
  [(MOD, ARMS,
    "State::Closed | State::Open(_) => Vec::new(),\n"
    "            State::Fenced(_) => vec![0],")])

m("A24", "controlled(): Closed arm returns vec![0]", "W-2",
  [(MOD, ARMS,
    "State::Open(_) | State::Fenced(_) => Vec::new(),\n"
    "            State::Closed => vec![0],")])

m("A25", "open_v4_guarded: defensive lookup-failure arm returns BadArgument"
  " instead of StartFailed (arm unreachable by design)", "W-2",
  [(MOD,
    "                    return self.fail(BridgeCode::StartFailed, Some(\"no player replica id\"))\n",
    "                    return self.fail(BridgeCode::BadArgument, Some(\"no player replica id\"))\n")])

# ---- W-3: render_positions() -----------------------------------------------
m("A26", "render_positions(): returns an empty Vec", "W-3",
  [(MOD, "        self.view.positions().to_vec()\n", "        Vec::new()\n")])

m("A27", "render_positions(): returns net_ids() instead of positions()", "W-3",
  [(MOD, "        self.view.positions().to_vec()\n",
    "        self.view.net_ids().to_vec()\n")])

m("A28", "render_positions(): first three entries only (.take(3))", "W-3",
  [(MOD, "        self.view.positions().to_vec()\n",
    "        self.view.positions().iter().copied().take(3).collect()\n")])

FILES = [MOD]


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
        print("baseline", status, "rc", rc, failed, tail, flush=True)
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
