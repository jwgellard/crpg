#!/usr/bin/env python3
"""Mutant driver for T157e (crates/crpg-godot/src/core, campaigns/demo/adventure).

Applies one mutant at a time by exact text replacement, runs the crate's tests
with CARGO_INCREMENTAL=0, records one JSON line per mutant in results.jsonl,
and restores the original bytes in a `finally`.

usage: run_mutants.py [--check] [--baseline] [--full] [ids...]
       --check    applies every anchor without cargo (no results written).
       --baseline runs the unmutated tree once (no results written).
       --full     use the full `cargo test -p crpg-godot` command (default: fast,
                  `--lib`). Full results go to results_full.jsonl.
       no ids: all rows, in table order.
"""
import json
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FULL = "--full" in sys.argv
OUT = ROOT / ("reviews/T157e/results_full.jsonl" if FULL else "reviews/T157e/results.jsonl")
TIMEOUT = 900
DUMP = os.environ.get("MUTANT_DUMP")
BASE = ["cargo", "test", "-p", "crpg-godot", "--locked", "-j", "2", "--no-fail-fast"]
FAST = BASE + ["--lib"]
FULLCMD = BASE

MOD = "crates/crpg-godot/src/core/mod.rs"
CLI = "crates/crpg-godot/src/core/client.rs"
CV4 = "crates/crpg-godot/src/core/client_v4.rs"
GOB = "campaigns/demo/adventure/creatures/goblin.json"
HERO = "campaigns/demo/adventure/creatures/hero.json"
STRIKE = "campaigns/demo/adventure/abilities/strike.json"
ENC = "campaigns/demo/adventure/encounters/first-blood.json"

M = {}
_n = [0]


def m(desc, item, edits):
    _n[0] += 1
    M["M%02d" % _n[0]] = (desc, item, edits)


RIGHTS = "const WALK_RIGHTS: PauseRights = PauseRights {\n    pause: true,\n    unpause: true,\n};"
WALKCALL = "            EncounterMode::RealTime,\n            NpcDriver::ServerAi,\n            WALK_RIGHTS,\n"
P_ULID = 'Ulid::from_str("0000000000000000000000000P").unwrap()'
MOVE_CHECK = "            && movement == current.movement()\n            && movement_max == current.movement_max();"

# ---- K-1: open_v4_walk ------------------------------------------------------------
m("WALK_RIGHTS.pause true -> false", "K-1",
  [(MOD, RIGHTS, RIGHTS.replace("pause: true,\n    unpause", "pause: false,\n    unpause"))])
m("WALK_RIGHTS.unpause true -> false", "K-1",
  [(MOD, RIGHTS, RIGHTS.replace("unpause: true", "unpause: false"))])
m("open_v4_walk_with binds PauseRights::default() instead of WALK_RIGHTS", "K-1",
  [(MOD, WALKCALL + "            true,\n",
    "            EncounterMode::RealTime,\n            NpcDriver::ServerAi,\n            PauseRights::default(),\n            true,\n")])
m("open_v4_walk_with own_pools true -> false", "K-1",
  [(MOD, WALKCALL + "            true,\n", WALKCALL + "            false,\n")])
m("grant: pools true -> false", "K-1",
  [(MOD, "                        pools: true,\n                        effects: false,",
    "                        pools: false,\n                        effects: false,")])
m("grant: effects false -> true", "K-1",
  [(MOD, "                        effects: false,\n                        stats: false,",
    "                        effects: true,\n                        stats: false,")])
m("grants in descending EntityId order", "K-1",
  [(MOD, "            owned.sort_unstable();\n", "            owned.sort_unstable_by(|a, b| b.cmp(a));\n")])
m("open_v4_rt_with passes own_pools true", "K-1 (other opens keep defaults)",
  [(MOD, "            EncounterMode::RealTime,\n            NpcDriver::Peer,\n            PauseRights::default(),\n            false,\n",
    "            EncounterMode::RealTime,\n            NpcDriver::Peer,\n            PauseRights::default(),\n            true,\n")])
m("open_v4_walk_with opens turn-based instead of real time", "K-1",
  [(MOD, WALKCALL, WALKCALL.replace("RealTime", "TurnBased"))])
m("open_v4_walk_with uses NpcDriver::Peer instead of ServerAi", "K-1",
  [(MOD, WALKCALL, WALKCALL.replace("ServerAi", "Peer"))])

# ---- K-2: movement_pool -------------------------------------------------------------
m("table.movement_pool never assigned", "K-2",
  [(MOD, "        table.movement_pool = movement_pool;\n", "        let _ = movement_pool;\n")])
m("movement_pool is the action pool P instead of movement.turn.pool", "K-2",
  [(MOD, "            .map(|movement| movement.turn.pool);\n",
    "            .map(|_| " + P_ULID + ");\n")])
m("movement_pool is Some(P) when the ruleset declares no movement", "K-2",
  [(MOD, "            .map(|movement| movement.turn.pool);\n",
    "            .map(|movement| movement.turn.pool)\n            .or_else(|| Some(" + P_ULID + "));\n")])

# ---- K-3: BridgeCore::request_mode ---------------------------------------------------
m("request_mode: 0 and 1 swapped", "K-3",
  [(MOD, "            0 => WireMode::TurnBased,\n            1 => WireMode::RealTime,\n",
    "            0 => WireMode::RealTime,\n            1 => WireMode::TurnBased,\n")])
m("request_mode: other modes accepted as turn-based", "K-3",
  [(MOD, '            _ => return self.fail(BridgeCode::BadArgument, Some("bad mode")),\n',
    "            _ => WireMode::TurnBased,\n")])
m("request_mode: last controlled actor instead of first", "K-3",
  [(MOD, "            .controlled()\n            .first()\n", "            .controlled()\n            .last()\n")])
m("request_mode: 'no controlled actor' fault code NoSuchOption -> BadArgument", "K-3",
  [(MOD, '                code: BridgeCode::NoSuchOption,\n                detail: "no controlled actor".to_string(),',
    '                code: BridgeCode::BadArgument,\n                detail: "no controlled actor".to_string(),')])

# ---- K-4: ClientV4::request_mode -----------------------------------------------------
m("ClientV4::request_mode: outstanding-intent refusal removed", "K-4",
  [(CV4, '        if self.pending.is_some() {\n            return Err(fault(BridgeCode::IntentPending, "an intent is outstanding"));\n        }\n        self.send_intent(actor, IntentBodyV4::RequestMode { mode }, link)',
    "        self.send_intent(actor, IntentBodyV4::RequestMode { mode }, link)")])
m("ClientV4::request_mode sends EndTurn instead of RequestMode", "K-4",
  [(CV4, "        self.send_intent(actor, IntentBodyV4::RequestMode { mode }, link)",
    "        let _ = mode;\n        self.send_intent(actor, IntentBodyV4::EndTurn, link)")])

# ---- K-5: movement columns -----------------------------------------------------------
m("rebuild: pool match `fact.pool == wanted` -> `!=`", "K-5",
  [(CV4, ".find(|fact| fact.pool == wanted)", ".find(|fact| fact.pool != wanted)")])
m("rebuild: first disclosed pool taken regardless of id", "K-5",
  [(CV4, ".find(|fact| fact.pool == wanted)", ".find(|_| wanted == wanted)")])
m("rebuild: movement pushes fact.max instead of fact.current", "K-5",
  [(CV4, "movement.push(i64::from(fact.current));", "movement.push(i64::from(fact.max));")])
m("rebuild: movement_max pushes fact.current instead of fact.max", "K-5",
  [(CV4, "movement_max.push(i64::from(fact.max));", "movement_max.push(i64::from(fact.current));")])
m("rebuild: withheld/absent rows push 0 instead of -1", "K-5",
  [(CV4, "                None => {\n                    movement.push(-1);\n                    movement_max.push(-1);\n                }",
    "                None => {\n                    movement.push(0);\n                    movement_max.push(0);\n                }")])
m("rebuild: movement dropped from the unchanged check", "K-5 / invariant 6",
  [(CV4, MOVE_CHECK, "            && movement_max == current.movement_max();")])
m("v2 client: movement column filled with 0 instead of -1", "K-5",
  [(CLI, "            movement: vec![-1; rows],\n", "            movement: vec![0; rows],\n")])


# ---- K-6: demo data -------------------------------------------------------------------
m("data: goblin faction dropped (null)", "K-6",
  [(GOB, '"faction": "0000000000000000000000000T",', '"faction": null,')])
m("data: strike range 98304 -> 65536", "K-6",
  [(STRIKE, '"range": 98304,', '"range": 65536,')])
m("data: encounter engage distance 262144 -> 131072", "K-6",
  [(ENC, '"distance": 262144', '"distance": 131072')])
m("data: hero health 1310720 -> 655360", "K-6",
  [(HERO, '"health": 1310720,', '"health": 655360,')])

FILES = sorted({e[0] for _, _, edits in M.values() for e in edits})


def read_originals():
    return {name: (ROOT / name).read_bytes() for name in FILES}


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
                    (ROOT / name).write_bytes(data)
                rc, out, to = run_cargo()
            finally:
                for name in mutated:
                    (ROOT / name).write_bytes(originals[name])
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
            (ROOT / name).write_bytes(data)


if __name__ == "__main__":
    main()
