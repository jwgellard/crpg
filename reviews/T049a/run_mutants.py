#!/usr/bin/env python3
"""Mutant driver for T049a (smooth.rs, client_v4.rs, mod.rs; lib + SR1 tests).

Applies one mutant at a time by exact text replacement, runs
`cargo test -p crpg-godot --locked -j 2 --lib --test source_rules_smooth`
with CARGO_INCREMENTAL=0, records one JSON line per mutant in results.jsonl,
and restores the original bytes in a `finally`.

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
OUT = ROOT / "reviews/T049a/results.jsonl"
TIMEOUT = 600
DUMP = os.environ.get("MUTANT_DUMP")

SM = "smooth.rs"
CV = "client_v4.rs"
MOD = "mod.rs"
OBS = "            self.smooth.observe(self.replica.positions());\n"

M = {}
_n = [0]


def m(desc, item, edits):
    _n[0] += 1
    M["S%02d" % _n[0]] = (desc, item, edits)


# ---- B-S1: clock offset -----------------------------------------------------
m("observe_clock: offset keeps the maximum instead of the minimum", "B-S1",
  [(SM, "Some(current) => current.min(observed),",
    "Some(current) => current.max(observed),")])
m("observe_clock: tick not multiplied by TICK_MS (arrival - tick)", "B-S1",
  [(SM, "i128::from(now_ms) - ms(server_tick);",
    "i128::from(now_ms) - i128::from(server_tick);")])
m("observe_clock: later sample overwrites the offset (no min)", "B-S1",
  [(SM, "Some(current) => current.min(observed),",
    "Some(_) => observed,")])
m("ClientV4::step: observe_clock call removed", "B-S1/B-S4",
  [(CV, "self.smooth.observe_clock(sample.server_tick, now_ms);", "")])
m("ClientV4::step: observe_clock gets now_ms = 0", "B-S1/B-S4",
  [(CV, "self.smooth.observe_clock(sample.server_tick, now_ms);",
    "self.smooth.observe_clock(sample.server_tick, 0);")])

# ---- B-S2: buffers ----------------------------------------------------------
m("observe: append test `newest.tick < sample.tick` -> `<=`", "B-S2",
  [(SM, "Some(newest) if newest.tick < sample.tick => {",
    "Some(newest) if newest.tick <= sample.tick => {")])
m("observe: bound test `len > 8` -> `len >= 8` (keeps 7)", "B-S2",
  [(SM, "while buffer.len() > SAMPLES_PER_ENTITY {",
    "while buffer.len() >= SAMPLES_PER_ENTITY {")])
m("observe: bound pops the back (newest) instead of the front", "B-S2",
  [(SM, "buffer.pop_front();", "buffer.pop_back();")])
m("observe: same tick, new value keeps the old newest (replace is a no-op)",
  "B-S2",
  [(SM, "*back = sample;", "*back = newest;")])
m("observe: lower tick appends without clearing (restart removed)", "B-S2",
  [(SM, "                        buffer.clear();\n", "")])
m("observe: removal of buffers no longer disclosed deleted", "B-S2",
  [(SM, "        self.buffers.retain(|id, _| positions.contains_key(id));\n", "")])
m("observe: sample.moving always false", "B-S2",
  [(SM, "moving: state.moving,", "moving: false,")])
m("ClientV4::step: observe call removed from the frame loop", "B-S4",
  [(CV, OBS, "")])
m("ClientV4::step: observe moved after the frame loop", "B-S4",
  [(CV, OBS, ""),
   (CV, "        if refused {\n            self.pending = None;\n",
    "        self.smooth.observe(self.replica.positions());\n"
    "        if refused {\n            self.pending = None;\n")])

# ---- B-S3: render -----------------------------------------------------------
m("render: hold test `render_at >= newest_at` -> `>`", "B-S3(2)",
  [(SM, "if render_at >= newest_at {", "if render_at > newest_at {")])
m("render: hold test -> `render_at > newest_at + 1000` (implementer's)",
  "B-S3(2)",
  [(SM, "if render_at >= newest_at {",
    "if render_at > newest_at + 1000 {")])
m("render: oldest test `render_at <= oldest_at` -> `<`", "B-S3(3)",
  [(SM, "if render_at <= oldest_at {", "if render_at < oldest_at {")])
m("render: before-the-oldest returns the newest point", "B-S3(3)",
  [(SM, "return Some(to_i64(oldest.point));",
    "return Some(to_i64(newest.point));")])
m("render: delay subtracted -> added (`+ RENDER_DELAY_MS`)", "B-S3",
  [(SM, "- i128::from(RENDER_DELAY_MS);", "+ i128::from(RENDER_DELAY_MS);")])
m("render: clock offset ignored in render_at", "B-S3",
  [(SM, "let render_at = i128::from(now_ms) - offset_ms - i128::from(RENDER_DELAY_MS);",
    "let render_at = i128::from(now_ms) - i128::from(RENDER_DELAY_MS);")])
m("render: no-clock case returns [0,0,0] instead of the newest point",
  "B-S3(1)",
  [(SM, "            return Some(to_i64(newest.point));\n        };\n",
    "            return Some([0; 3]);\n        };\n")])
m("render: pair test `a_at <= render_at` -> `<`", "B-S3(4)",
  [(SM, "if a_at <= render_at && render_at < c_at {",
    "if a_at < render_at && render_at < c_at {")])
m("interpolate: rest start `max` -> `min` (whole gap used)", "B-S3(4)/R3",
  [(SM, "a_at.max(c_at - i128::from(TICK_MS))",
    "a_at.min(c_at - i128::from(TICK_MS))")])
m("interpolate: `if a.moving` -> `if !a.moving`", "B-S3(4)/R3",
  [(SM, "let start = if a.moving {", "let start = if !a.moving {")])
m("interpolate: truncating division -> div_euclid (floor)", "B-S3(4)",
  [(SM, "(from + (to - from) * elapsed / span) as i64",
    "(from + ((to - from) * elapsed).div_euclid(span)) as i64")])
m("interpolate: span = c_at - a_at instead of c_at - start", "B-S3(4)",
  [(SM, "let span = c_at - start;", "let span = c_at - a_at;")])
m("ms(): u64 multiply then widen (not i128 widening first)", "B-S3/R5",
  [(SM, "i128::from(tick) * i128::from(TICK_MS)",
    "(tick * TICK_MS) as i128")])

# ---- B-S4/B-S5: client rows and the session member ---------------------------
m("ClientV4::render_positions: unsmoothed fallback returns zeros", "B-S4",
  [(CV, "None => out.extend_from_slice(&positions[3 * index..3 * index + 3]),",
    "None => out.extend_from_slice(&[0, 0, 0]),")])
m("ClientV4::render_positions: rows emitted in reverse order", "B-S4",
  [(CV, "for (index, id) in net_ids.iter().enumerate() {",
    "for (index, id) in net_ids.iter().enumerate().rev() {")])
m("BridgeCore::poll: last_poll_ms recording removed", "B-S5",
  [(MOD, "        self.last_poll_ms = now_ms;\n", "")])
m("BridgeCore::render_positions: OpenV4 arm passes now = 0", "B-S5",
  [(MOD, "live.client.render_positions(self.last_poll_ms)",
    "live.client.render_positions(0)")])
m("BridgeCore::render_positions: OpenV4 arm returns the unsmoothed view",
  "B-S5",
  [(MOD, "State::OpenV4(live) => live.client.render_positions(self.last_poll_ms),",
    "State::OpenV4(_live) => self.view.positions().to_vec(),")])

FILES = [SM, CV, MOD]


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
    cmd = ["cargo", "test", "-p", "crpg-godot", "--locked", "-j", "2", "--lib",
           "--test", "source_rules_smooth"]
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
