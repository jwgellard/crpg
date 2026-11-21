#!/usr/bin/env python3
"""Mutant driver for T155a (the new lines of crates/crpg-server/src/host.rs).

Applies one mutant at a time by exact text replacement in the source file the
mutant table cites, runs
`cargo test -p crpg-server --locked -j 2 --lib --test host_reconnect
 --test host_reconnect_v4 --test host_lane1`
(T155A_FULL=1 runs the whole crate's tests instead), plus --no-fail-fast so
every binary reports with CARGO_INCREMENTAL=0,
records one JSON line per mutant in results.jsonl, and restores the original
bytes in a `finally`.

usage: run_mutants.py [--check] [ids...]
       --check applies and restores every anchor without cargo (no results).
       no ids: all rows, in table order.
"""
import json
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "reviews/T155a/results.jsonl"
TIMEOUT = 900
DUMP = os.environ.get("MUTANT_DUMP")

# id -> (description, contract row, [(file, old, new), ...])
M = {}


def m(mid, desc, item, edits):
    M[mid] = (desc, item, edits)


H = "crates/crpg-server/src/host.rs"
TAIL = "    v4.lane.receipts.clear();\n    v4.samples.clear();\n    *egress_bytes -= freed;\n"
RESUME_CALL = (
    "        reset_lane1(\n            &mut self.egress_bytes,\n"
    "            self.sessions.get_mut(&peer).expect(\"live above\"),\n        );\n"
)
RK = "fn resumable_kind(session: &Session) -> bool {\n        session.snapshot\n"

m("M0", "unmutated run (must pass)", "-", [])

m("M1",
  "host.rs resumable_kind (~5522, H-1): keeps the old `&& !v4` rule, so a v4"
  " session is again NotResumable", "H-1",
  [(H, RK,
    "fn resumable_kind(session: &Session) -> bool {\n"
    "        session.snapshot && !session.v3.as_ref().is_some_and(|v3| v3.v4.is_some())\n")])

m("M1b",
  "host.rs resumable_kind (~5522, H-1): always true, so a bind_peer session"
  " is resumable too", "H-1",
  [(H, RK,
    "fn resumable_kind(session: &Session) -> bool {\n        let _ = session;\n        true\n")])

m("M2",
  "host.rs reset_lane1 (~6224, H-2 step 2): the next_seq rollback to the first"
  " dropped slot is skipped", "H-2",
  [(H, "        v4.lane.next_seq = front.seq;\n", "        let _ = front;\n")])

m("M3",
  "host.rs reset_lane1 (~6227, H-2 step 3): the released-receipt clear is"
  " skipped", "H-2",
  [(H, TAIL, "    v4.samples.clear();\n    *egress_bytes -= freed;\n")])

m("M4",
  "host.rs reset_lane1 (~6228, H-2 step 3): the queued-sample clear inside"
  " reset_lane1 is skipped (queue_samples' own clear untouched)", "H-2",
  [(H, TAIL, "    v4.lane.receipts.clear();\n    *egress_bytes -= freed;\n")])

m("M5",
  "host.rs reset_lane1 (~6229, H-2 step 4): the egress subtraction is"
  " skipped", "H-2",
  [(H, "    *egress_bytes -= freed;\n}\n", "    let _ = (egress_bytes, freed);\n}\n")])

m("M5b",
  "host.rs reset_lane1 (~6229, H-2 step 4): the egress subtraction is one"
  " byte short", "H-2",
  [(H, "    *egress_bytes -= freed;\n}\n",
    "    *egress_bytes -= freed.saturating_sub(1);\n}\n")])

m("M6",
  "host.rs begin_grace (~5246, H-3): the reset_lane1 call at grace start is"
  " removed", "H-3",
  [(H, "            session.delivered = session.next_ordinal - 1;\n"
       "            reset_lane1(&mut self.egress_bytes, session);\n",
    "            session.delivered = session.next_ordinal - 1;\n")])

m("M7",
  "host.rs resume_peer (~5453, H-4): the reset_lane1 call in the resume"
  " commit is removed", "H-4",
  [(H, RESUME_CALL, "")])

m("M8",
  "host.rs resume_peer (~5333, H-4): a proof-refused resume of a live"
  " session resets lane 1 before returning Denied", "H-4",
  [(H, "        if !proof_ok {\n            return denied(request.epoch);\n",
    "        if !proof_ok {\n"
    "            if let Some(s) = self.sessions.get_mut(&peer) {\n"
    "                reset_lane1(&mut self.egress_bytes, s);\n"
    "            }\n"
    "            return denied(request.epoch);\n")])

m("M8b",
  "host.rs resume_peer (~5333, H-4): a proof-refused resume of a graced"
  " session resets lane 1 before returning Denied", "H-4",
  [(H, "        if !proof_ok {\n            return denied(request.epoch);\n",
    "        if !proof_ok {\n"
    "            if let Some(g) = self.graces.get_mut(&peer) {\n"
    "                reset_lane1(&mut self.egress_bytes, &mut g.session);\n"
    "            }\n"
    "            return denied(request.epoch);\n")])

m("M8c",
  "host.rs resume_peer (~5352, H-4): a fence-check-refused resume of a live"
  " session resets lane 1 before returning Denied", "H-4",
  [(H, "        if check.is_err() {\n            return denied(request.epoch);\n",
    "        if check.is_err() {\n"
    "            if let Some(s) = self.sessions.get_mut(&peer) {\n"
    "                reset_lane1(&mut self.egress_bytes, s);\n"
    "            }\n"
    "            return denied(request.epoch);\n")])

m("M9",
  "host.rs reset_lane1 (~6218, H-2 step 1): freed takes the frame count"
  " (.0) instead of the bytes (.1)", "H-2",
  [(H, "    let freed = session.lane1_egress().1;\n",
    "    let freed = session.lane1_egress().0;\n")])

m("M10",
  "host.rs reset_lane1 (~6224, H-2 step 2): next_seq rolls back to the first"
  " dropped slot's seq plus one", "H-2",
  [(H, "        v4.lane.next_seq = front.seq;\n",
    "        v4.lane.next_seq = front.seq + 1;\n")])

m("M11",
  "host.rs reset_lane1 (~6226, H-2 step 3): the slots clear is skipped", "H-2",
  [(H, "    v4.lane.slots.clear();\n", "")])

m("M12",
  "host.rs reset_lane1 (H-2 unchanged list): also clears the lane-1"
  " duplicate cache", "H-2",
  [(H, TAIL, "    v4.lane.receipts.clear();\n    v4.lane.cache.clear();\n"
             "    v4.samples.clear();\n    *egress_bytes -= freed;\n")])

m("M13",
  "host.rs reset_lane1 (H-2 unchanged list): also zeroes processed_seq", "H-2",
  [(H, TAIL, "    v4.lane.receipts.clear();\n    v4.lane.processed_seq = 0;\n"
             "    v4.samples.clear();\n    *egress_bytes -= freed;\n")])

m("M14",
  "host.rs reset_lane1 (H-2 unchanged list): also refills the lane-1 move"
  " bucket", "H-2",
  [(H, TAIL, "    v4.lane.receipts.clear();\n"
             "    v4.lane.bucket = TokenBucket::new(\n"
             "        movement_v4::POLICY_MOVE_INTENT_BURST,\n"
             "        movement_v4::POLICY_MOVE_INTENTS_PER_SEC,\n"
             "    );\n"
             "    v4.samples.clear();\n    *egress_bytes -= freed;\n")])

m("M15",
  "host.rs reset_lane1 (H-2 unchanged list): also clears the withheld set",
  "H-2",
  [(H, TAIL, "    v4.lane.receipts.clear();\n    v4.withheld.clear();\n"
             "    v4.samples.clear();\n    *egress_bytes -= freed;\n")])

m("M16",
  "host.rs reset_lane1 (H-2 unchanged list): also resets the position view",
  "H-2",
  [(H, TAIL, "    v4.lane.receipts.clear();\n"
             "    v4.positions = PositionView::new(SessionEpoch(session.epoch));\n"
             "    v4.samples.clear();\n    *egress_bytes -= freed;\n")])

m("M17",
  "host.rs resume_peer (~5453, H-4): the resume reset runs only for a graced"
  " session (a live resume keeps its lane-1 state)", "H-4",
  [(H, RESUME_CALL, "        if is_graced {\n" + RESUME_CALL + "        }\n")])

m("M18",
  "host.rs resume_peer (~5453, H-4): the resume reset runs only for a live"
  " session (a graced one relies on H-3)", "H-4",
  [(H, RESUME_CALL, "        if !is_graced {\n" + RESUME_CALL + "        }\n")])

m("M19",
  "host.rs resume_peer (~5453, H-4): the resume reset runs after the"
  " staging commit instead of before it", "H-4",
  [(H, RESUME_CALL, ""),
   (H, "        Ok(ResumeAnswer::Accepted {\n            peer,\n",
    RESUME_CALL + "        Ok(ResumeAnswer::Accepted {\n            peer,\n")])

m("M20",
  "host.rs begin_grace (~5225, H-3): a NotResumable-refused grace of a live"
  " session resets lane 1 before returning", "H-3",
  [(H, "        if !resumable {\n            return Err(HostError::NotResumable);\n        }\n"
       "        // Purge the peer's staged commands",
    "        if !resumable {\n"
    "            if let Some(s) = self.sessions.get_mut(&peer) {\n"
    "                reset_lane1(&mut self.egress_bytes, s);\n"
    "            }\n"
    "            return Err(HostError::NotResumable);\n        }\n"
    "        // Purge the peer's staged commands")])

FILES = sorted({name for _, _, edits in M.values() for name, _, _ in edits})


def read_originals():
    return {name: (ROOT / name).read_bytes() for name in FILES}


def apply(originals, edits):
    """Returns {file: mutated bytes} for the files this mutant edits."""
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
            raise ValueError(
                f"{name}: anchor matched {count} times: {old[:70]!r}"
            )
        out[name] = text.replace(old, new, 1).encode("utf-8")
    return out


def run_cargo():
    env = dict(os.environ, CARGO_INCREMENTAL="0")
    cmd = ["cargo", "test", "-p", "crpg-server", "--locked", "-j", "2"]
    if os.environ.get("T155A_FULL") != "1":
        cmd += ["--lib", "--test", "host_reconnect", "--test",
                "host_reconnect_v4", "--test", "host_lane1"]
    cmd += ["--no-fail-fast"]
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
    if "test result:" not in out or "could not compile" in out:
        return "compile-error", []
    failed = [ln.split()[1] for ln in out.splitlines()
              if ln.startswith("test ") and ln.rstrip().endswith("FAILED")]
    if rc == 0 and not failed:
        return "survived", []
    return "killed", failed


def main():
    args = sys.argv[1:]
    check = "--check" in args
    ids = [a for a in args if a != "--check"] or list(M)
    originals = read_originals()
    lines = {}
    if not check:
        OUT.parent.mkdir(parents=True, exist_ok=True)
        if OUT.exists():
            for line in OUT.read_text(encoding="utf-8").splitlines():
                if line.strip():
                    old = json.loads(line)
                    lines[old["id"]] = line
        for mid in ids:
            lines.pop(mid, None)

    def flush():
        order = list(M)
        OUT.write_text(
            "".join(lines[mid] + "\n" for mid in order if mid in lines),
            encoding="utf-8")

    try:
        for mid in ids:
            desc, item, edits = M[mid]
            rec = {"id": mid, "desc": desc, "contract": item,
                   "edits": [{"file": f, "old": o, "new": n}
                             for f, o, n in edits]}
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
                (Path(DUMP) / f"{mid}.cargo.log").write_text(
                    out, encoding="utf-8")
            status, failed = classify(rc, out, to)
            rec.update(status=status, failed_tests=failed,
                       seconds=round(time.time() - t0, 1))
            if mid == "M0" and status == "survived":
                status = "pass"
                rec["status"] = "pass"
            lines_out = out.splitlines()
            for n, ln in enumerate(lines_out):
                if "panicked at" in ln:
                    rec["panic"] = " | ".join(
                        x.strip() for x in lines_out[n:n + 4])[:600]
                    break
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
