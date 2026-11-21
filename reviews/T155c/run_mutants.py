#!/usr/bin/env python3
"""Mutant driver for T155c (the new lines of crates/crpg-server/src/quic.rs).

Applies one mutant at a time by exact text replacement in the source file the
mutant table cites, runs
`cargo test -p crpg-server --locked -j 2 --no-fail-fast --lib
 --test host_quic_move_resume --test host_quic_move --test host_quic_resume`
(T155C_FULL=1 runs the whole crate's tests instead; use it for survivors) with
CARGO_INCREMENTAL=0, records one JSON line per mutant in results.jsonl, and
restores the original bytes in a `finally`.

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
OUT = ROOT / "reviews/T155c/results.jsonl"
TIMEOUT = 900
DUMP = os.environ.get("MUTANT_DUMP")

# id -> (description, contract row, [(file, old, new), ...])
M = {}


def m(mid, desc, item, edits):
    M[mid] = (desc, item, edits)


Q = "crates/crpg-server/src/quic.rs"

m("M0", "unmutated run (must pass)", "-", [])

m("M1",
  "quic.rs on_resume (~1284, C-1): a resume on a movement (/3) connection is"
  " refused with AuthRefused before any host call", "C-1",
  [(Q, "        // (2) Decide.\n        let result = self.host.resume_peer(&request, now_ms);\n",
    "        if movement {\n"
    "            self.refuse_resume(conn, CloseCode::AuthRefused, report);\n"
    "            return Ok(());\n"
    "        }\n"
    "        // (2) Decide.\n        let result = self.host.resume_peer(&request, now_ms);\n")])

m("M2",
  "quic.rs on_resume resumed Bound (~1343, C-2): `movement` forced false", "C-2",
  [(Q, "                    framing: Framing::V2,\n                    movement,\n"
       "                    fence: accepted.granted,\n",
    "                    framing: Framing::V2,\n                    movement: false,\n"
    "                    fence: accepted.granted,\n")])

m("M2b",
  "quic.rs on_resume resumed Bound (~1343, C-2): `movement` negated", "C-2",
  [(Q, "                    framing: Framing::V2,\n                    movement,\n"
       "                    fence: accepted.granted,\n",
    "                    framing: Framing::V2,\n                    movement: !movement,\n"
    "                    fence: accepted.granted,\n")])

m("M3",
  "quic.rs drain_lane1 (~1840, C-3): the lane-1 fence check is skipped"
  " (`if self.resumable` -> `if false`)", "C-3",
  [(Q, "(a held retry is exempt).\n                    if self.resumable {\n",
    "(a held retry is exempt).\n                    if false && self.resumable {\n")])

m("M3b",
  "quic.rs drain_lane1 (~1840, C-3): the lane-1 fence check also runs on a"
  " `start` (non-resumable) adapter", "C-3",
  [(Q, "(a held retry is exempt).\n                    if self.resumable {\n",
    "(a held retry is exempt).\n                    if true {\n")])

m("M4",
  "quic.rs drain_lane1 (~1856, C-3): SessionFenced on lane 1 does not call"
  " retire_resumed", "C-3",
  [(Q, "                                report.lane1_refused += 1;\n"
       "                                self.retire_resumed(conn, Some(report));\n",
    "                                report.lane1_refused += 1;\n")])

m("M5",
  "quic.rs drain_lane1 (~1855, C-3): SessionFenced on lane 1 is not counted"
  " in lane1_refused", "C-3",
  [(Q, "                            Err(HostError::SessionFenced) => {\n"
       "                                report.lane1_refused += 1;\n"
       "                                self.retire_resumed(conn, Some(report));\n",
    "                            Err(HostError::SessionFenced) => {\n"
    "                                self.retire_resumed(conn, Some(report));\n")])

m("M6",
  "quic.rs drain_lane1 (~1861, C-3): UnknownPeer on lane 1 does not"
  " fence(EpochClosed)", "C-3",
  [(Q, "                                report.lane1_refused += 1;\n"
       "                                self.fence(conn, FenceCause::EpochClosed, Some(report));\n"
       "                                return Ok(());\n",
    "                                report.lane1_refused += 1;\n"
    "                                return Ok(());\n")])

m("M7",
  "quic.rs drain_lane1 (~1860, C-3): UnknownPeer on lane 1 counted in"
  " frames_refused instead of lane1_refused", "C-3",
  [(Q, "                            Err(HostError::UnknownPeer) => {\n"
       "                                report.lane1_refused += 1;\n",
    "                            Err(HostError::UnknownPeer) => {\n"
    "                                report.frames_refused += 1;\n")])

m("M7b",
  "quic.rs drain_lane1 (~1860, C-3): UnknownPeer on lane 1 not counted at"
  " all", "C-3",
  [(Q, "                            Err(HostError::UnknownPeer) => {\n"
       "                                report.lane1_refused += 1;\n",
    "                            Err(HostError::UnknownPeer) => {\n")])

m("M8",
  "quic.rs drain_lane1 (~1865, C-3): any other fence-check error is swallowed"
  " (Ok) instead of returned", "C-3",
  [(Q, "                            Err(error) => return Err(error),\n"
       "                        }\n"
       "                    }\n"
       "                    match self.ingest_move_one(",
    "                            Err(_) => return Ok(()),\n"
    "                        }\n"
    "                    }\n"
    "                    match self.ingest_move_one(")])

m("M9",
  "quic.rs drain_lane1 (~1848, C-3): the HostCall::CheckFrame record for"
  " lane 1 is dropped", "C-3",
  [(Q, "                        report.calls.push(HostCall::CheckFrame {\n"
       "                            conn,\n                            peer,\n"
       "                            fence,\n                            result: check.clone(),\n"
       "                        });\n                        match check {\n"
       "                            Ok(()) => {}\n"
       "                            Err(HostError::SessionFenced) => {\n"
       "                                report.lane1_refused += 1;\n",
    "                        match check {\n"
    "                            Ok(()) => {}\n"
    "                            Err(HostError::SessionFenced) => {\n"
    "                                report.lane1_refused += 1;\n")])

m("M10",
  "quic.rs retire_resumed (~2432, C-4): a held lane-1 frame is not counted in"
  " lane1_discarded", "C-4",
  [(Q, "            // T155 C-4: a held lane-1 frame is discarded too.\n"
       "            if bound.held_lane1.is_some() {\n"
       "                report.lane1_discarded += 1;\n            }\n",
    "")])

m("M10b",
  "quic.rs retire_resumed (~2433, C-4): lane1_discarded counted when NO lane-1"
  " frame was held (`is_some` -> `is_none`)", "C-4",
  [(Q, "            // T155 C-4: a held lane-1 frame is discarded too.\n"
       "            if bound.held_lane1.is_some() {\n",
    "            // T155 C-4: a held lane-1 frame is discarded too.\n"
    "            if bound.held_lane1.is_none() {\n")])

m("M10c",
  "quic.rs retire_resumed (~2434, C-4): lane1_discarded incremented by 2", "C-4",
  [(Q, "            // T155 C-4: a held lane-1 frame is discarded too.\n"
       "            if bound.held_lane1.is_some() {\n"
       "                report.lane1_discarded += 1;\n",
    "            // T155 C-4: a held lane-1 frame is discarded too.\n"
    "            if bound.held_lane1.is_some() {\n"
    "                report.lane1_discarded += 2;\n")])

PM = "    let (seq, code) = match result {\n"
m("M11",
  "quic.rs push_move (~281, C-5): rejections() omits every Move entry"
  " (push_move returns at once)", "C-5",
  [(Q, PM, "    if true {\n        return;\n    }\n" + PM)])

LOOP = ("            while let Some(trace) = lane1.next_if(|trace| trace.at <= k) {\n"
        "                push_move(trace, &mut out);\n            }\n")
m("M12",
  "quic.rs rejections (~208, C-5): Move entries appended after all calls"
  " (the per-call merge loop removed)", "C-5",
  [(Q, LOOP, "")])

m("M13",
  "quic.rs rejections (~269, C-5): the merge drops the entry at"
  " at == calls.len() (trailing loop removed)", "C-5",
  [(Q, "        for trace in lane1 {\n            push_move(trace, &mut out);\n        }\n",
    "        let _ = lane1;\n")])

m("M14",
  "quic.rs Rejection::Move Display (~452, C-5): `frame=` label altered to"
  " `frm=`", "C-5",
  [(Q, '"move conn={} peer={} frame={} len={} seq=",',
    '"move conn={} peer={} frm={} len={} seq=",')])

m("M14b",
  "quic.rs Rejection::Move Display (~463, C-5): the absent seq prints `none`"
  " instead of `-`", "C-5",
  [(Q, 'None => f.write_str("-")?,\n                }\n                write!(f, " code={}"',
    'None => f.write_str("none")?,\n                }\n                write!(f, " code={}"')])

m("M14c",
  "quic.rs Rejection::Move Display (~465, C-5): code prints `code as u8 + 1`",
  "C-5",
  [(Q, 'write!(f, " code={}", *code as u8)',
    'write!(f, " code={}", *code as u8 + 1)')])

m("M14d",
  "quic.rs Rejection::Move Display (~452, C-5): `len=` prints the frame"
  " ordinal instead of len", "C-5",
  [(Q, "                    peer.get(),\n                    frame,\n                    len\n"
       "                )?;\n                match seq {",
    "                    peer.get(),\n                    frame,\n                    frame\n"
    "                )?;\n                match seq {")])

m("M15",
  "quic.rs push_move (~291, C-5): a QueueFull Refused is no longer exempt"
  " (guard removed)", "C-5",
  [(Q, "MoveAdmission::Refused { seq, code } if *code != CodeV3::QueueFull => (*seq, *code),",
    "MoveAdmission::Refused { seq, code } => (*seq, *code),")])

m("M16",
  "quic.rs push_move (~292, C-5): a Rejected admission reports seq None",
  "C-5",
  [(Q, "MoveAdmission::Rejected { seq, code } => (Some(*seq), *code),",
    "MoveAdmission::Rejected { seq, code } => {\n            let _ = seq;\n"
    "            (None, *code)\n        }")])

m("M17",
  "quic.rs push_move (~291, C-5): a Refused admission loses its seq"
  " (always None)", "C-5",
  [(Q, "if *code != CodeV3::QueueFull => (*seq, *code),",
    "if *code != CodeV3::QueueFull => {\n            let _ = seq;\n"
    "            (None, *code)\n        }")])

m("M18",
  "quic.rs rejections (~209, C-5): merge uses `trace.at < k` (strict)", "C-5",
  [(Q, "lane1.next_if(|trace| trace.at <= k)", "lane1.next_if(|trace| trace.at < k)")])

m("M19",
  "quic.rs rejections (~209, C-5): merge uses `trace.at <= k + 1` (one"
  " call too early)", "C-5",
  [(Q, "lane1.next_if(|trace| trace.at <= k)",
    "lane1.next_if(|trace| trace.at <= k + 1)")])

m("M20",
  "quic.rs push_move (~292, C-5): a Rejected admission's code is replaced by"
  " Malformed", "C-5",
  [(Q, "MoveAdmission::Rejected { seq, code } => (Some(*seq), *code),",
    "MoveAdmission::Rejected { seq, code } => {\n            let _ = code;\n"
    "            (Some(*seq), CodeV3::Malformed)\n        }")])

m("M21",
  "quic.rs on_resume (~1274, C-1): the unreachable movement Err path reports"
  " IdleTimeout instead of EndpointFailed", "C-1",
  [(Q, "        let Ok(movement) = self.server.movement(conn) else {\n"
       "            report.resumes.push(ResumeReport {\n"
       "                conn,\n                outcome: ResumeOutcome::Lost {\n"
       "                    reason: CloseReason::EndpointFailed,",
    "        let Ok(movement) = self.server.movement(conn) else {\n"
    "            report.resumes.push(ResumeReport {\n"
    "                conn,\n                outcome: ResumeOutcome::Lost {\n"
    "                    reason: CloseReason::IdleTimeout,")])

m("M22",
  "quic.rs on_resume (~1274, C-1): the movement Err path also makes the host"
  " call (resume_peer) before returning", "C-1",
  [(Q, "        let Ok(movement) = self.server.movement(conn) else {\n"
       "            report.resumes.push(ResumeReport {",
    "        let Ok(movement) = self.server.movement(conn) else {\n"
    "            let _ = self.host.resume_peer(&request, now_ms);\n"
    "            report.resumes.push(ResumeReport {")])

m("M23",
  "quic.rs drain_lane1 (~1851, C-3): the check uses ConnectionFence::FIRST"
  " instead of the binding's fence", "C-3",
  [(Q, "(a held retry is exempt).\n                    if self.resumable {\n"
       "                        let Some((peer, fence)) =\n"
       "                            self.bound(conn).map(|bound| (bound.peer, bound.fence))\n",
    "(a held retry is exempt).\n                    if self.resumable {\n"
    "                        let Some((peer, fence)) =\n"
    "                            self.bound(conn).map(|bound| (bound.peer, ConnectionFence::FIRST))\n")])

m("M24",
  "quic.rs drain_lane1 (~1862, C-3): UnknownPeer on lane 1 fences with"
  " Disconnected instead of EpochClosed", "C-3",
  [(Q, "                                report.lane1_refused += 1;\n"
       "                                self.fence(conn, FenceCause::EpochClosed, Some(report));\n",
    "                                report.lane1_refused += 1;\n"
    "                                self.fence(conn, FenceCause::Disconnected, Some(report));\n")])

m("M25",
  "quic.rs drain_lane1 (~1857, C-3): SessionFenced on lane 1 falls through to"
  " ingest (no stop)", "C-3",
  [(Q, "                                report.lane1_refused += 1;\n"
       "                                self.retire_resumed(conn, Some(report));\n"
       "                                return Ok(());\n",
    "                                report.lane1_refused += 1;\n"
    "                                self.retire_resumed(conn, Some(report));\n")])

m("M26",
  "quic.rs drain_lane1 (~1862, C-3): UnknownPeer on lane 1 falls through to"
  " ingest (no stop)", "C-3",
  [(Q, "                                report.lane1_refused += 1;\n"
       "                                self.fence(conn, FenceCause::EpochClosed, Some(report));\n"
       "                                return Ok(());\n                            }\n"
       "                            Err(error)",
    "                                report.lane1_refused += 1;\n"
    "                                self.fence(conn, FenceCause::EpochClosed, Some(report));\n"
    "                            }\n                            Err(error)")])

m("M27",
  "quic.rs drain_lane1 (~1845, C-3): the bound-lookup else arm returns an"
  " error instead of Ok", "C-3",
  [(Q, "(a held retry is exempt).\n                    if self.resumable {\n"
       "                        let Some((peer, fence)) =\n"
       "                            self.bound(conn).map(|bound| (bound.peer, bound.fence))\n"
       "                        else {\n                            return Ok(());",
    "(a held retry is exempt).\n                    if self.resumable {\n"
    "                        let Some((peer, fence)) =\n"
    "                            self.bound(conn).map(|bound| (bound.peer, bound.fence))\n"
    "                        else {\n"
    "                            return Err(HostError::Invariant(\"x\"));")])

m("M28",
  "quic.rs drain_lane1 (~1815, C-3): the held retry is fence-checked too"
  " (a CheckFrame call is recorded before it)", "C-3",
  [(Q, "        if let Some(frame) = held {\n"
       "            match self.ingest_move_one(conn, &frame, now_ms, report)? {",
    "        if let Some(frame) = held {\n"
    "            if self.resumable {\n"
    "                if let Some((peer, fence)) =\n"
    "                    self.bound(conn).map(|bound| (bound.peer, bound.fence))\n"
    "                {\n"
    "                    let check = self.host.check_frame(peer, fence);\n"
    "                    report.calls.push(HostCall::CheckFrame {\n"
    "                        conn,\n                        peer,\n                        fence,\n"
    "                        result: check.clone(),\n                    });\n"
    "                }\n"
    "            }\n"
    "            match self.ingest_move_one(conn, &frame, now_ms, report)? {")])

m("M29",
  "quic.rs drain_lane1 (~1836, C-3): `lane1_frames += 1` for a fresh frame is"
  " removed", "C-3",
  [(Q, "                    if let Some(bound) = self.bound_mut(conn) {\n"
       "                        bound.lane1_frames += 1;\n"
       "                    }\n"
       "                    // T155 C-3",
    "                    // T155 C-3")])

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
    cmd = ["cargo", "test", "-p", "crpg-server", "--locked", "-j", "2",
           "--no-fail-fast"]
    if os.environ.get("T155C_FULL") != "1":
        cmd += ["--lib", "--test", "host_quic_move_resume", "--test",
                "host_quic_move", "--test", "host_quic_resume"]
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
                       suite="full" if os.environ.get("T155C_FULL") == "1" else "subset",
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
