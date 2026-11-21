#!/usr/bin/env python3
"""Mutant driver for T040a (crates/crpg-server: the T040a lines of quic.rs and run.rs).

Applies one mutant at a time by exact text replacement (CRLF-aware, the anchor
must occur exactly once), runs the T040a test union

  cargo test -p crpg-server --locked -j 2 --lib --test run_intake_observer
      --test run_loop --test run_dedicated --test host_quic

with CARGO_INCREMENTAL=0 (timeout 900 s), records one JSON line per mutant in
results.jsonl, and restores the original bytes in a `finally`.

usage: run_mutants.py [--check] [--full] [ids...]
       --check  applies and restores every anchor without cargo (no results).
       --full   runs the full `cargo test -p crpg-server --locked -j 2` instead
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
SRC = ROOT / "crates/crpg-server/src"
OUT = ROOT / "reviews/T040a/results.jsonl"
OUT_FULL = ROOT / "reviews/T040a/results-full.jsonl"
TIMEOUT = 900
TIMEOUT_FULL = 2400
DUMP = os.environ.get("MUTANT_DUMP")

Q = "quic.rs"
R = "run.rs"
FILES = [Q, R]

M = {}


def m(mid, desc, item, edits):
    M[mid] = (desc, item, edits)


# ---- quic.rs rejections(): B-1 sources and order ----------------------------
m("M01", "quic.rs rejections: hello loop iterates take(0) (refused hellos never listed)",
  "B-1 hellos (T1)",
  [(Q, "        for decision in &self.hellos {\n",
    "        for decision in self.hellos.iter().take(0) {\n")])

m("M02", "quic.rs rejections: Hello.code is a constant ProtocolViolation, not the refusal's code",
  "B-1/B-2 hello code (T1)",
  [(Q, "                out.push(Rejection::Hello {\n                    conn: decision.conn,\n                    code: *code,\n",
    "                out.push(Rejection::Hello {\n                    conn: decision.conn,\n                    code: CloseCode::ProtocolViolation,\n")])

m("M03", "quic.rs rejections: resume loop iterates take(0) (refused resumes never listed)",
  "B-1 resumes",
  [(Q, "        for resume in &self.resumes {\n",
    "        for resume in self.resumes.iter().take(0) {\n")])

m("M04", "quic.rs rejections: Resume.code is a constant ProtocolViolation",
  "B-1 resumes",
  [(Q, "                out.push(Rejection::Resume {\n                    conn: resume.conn,\n                    code: *code,\n",
    "                out.push(Rejection::Resume {\n                    conn: resume.conn,\n                    code: CloseCode::ProtocolViolation,\n")])

m("M05", "quic.rs rejections: resumes listed before hellos (two loops swapped)",
  "B-1 order",
  [(Q,
    "        for decision in &self.hellos {\n"
    "            if let HelloOutcome::Refused { code } = &decision.outcome {\n"
    "                out.push(Rejection::Hello {\n"
    "                    conn: decision.conn,\n"
    "                    code: *code,\n"
    "                });\n"
    "            }\n"
    "        }\n"
    "        for resume in &self.resumes {\n"
    "            if let ResumeOutcome::Refused { code } = &resume.outcome {\n"
    "                out.push(Rejection::Resume {\n"
    "                    conn: resume.conn,\n"
    "                    code: *code,\n"
    "                });\n"
    "            }\n"
    "        }\n",
    "        for resume in &self.resumes {\n"
    "            if let ResumeOutcome::Refused { code } = &resume.outcome {\n"
    "                out.push(Rejection::Resume {\n"
    "                    conn: resume.conn,\n"
    "                    code: *code,\n"
    "                });\n"
    "            }\n"
    "        }\n"
    "        for decision in &self.hellos {\n"
    "            if let HelloOutcome::Refused { code } = &decision.outcome {\n"
    "                out.push(Rejection::Hello {\n"
    "                    conn: decision.conn,\n"
    "                    code: *code,\n"
    "                });\n"
    "            }\n"
    "        }\n")])

m("M06", "quic.rs rejections: calls iterated in reverse order",
  "B-1 order of calls (T3b/T2)",
  [(Q, "        for call in &self.calls {\n", "        for call in self.calls.iter().rev() {\n")])

m("M07", "quic.rs rejections: Ingest QueueFull exclusion guard removed",
  "B-1 ingest QueueFull (T4)",
  [(Q, "                } if *code != RejectionCode::QueueFull => out.push(Rejection::Ingest {\n",
    "                } => out.push(Rejection::Ingest {\n")])

m("M08", "quic.rs rejections: Ingest guard flipped (`!=` -> `==` QueueFull)",
  "B-1 ingest QueueFull (T3b/T4)",
  [(Q, "if *code != RejectionCode::QueueFull =>", "if *code == RejectionCode::QueueFull =>")])

m("M09", "quic.rs rejections: Ingest.len reported as len + 1",
  "B-1 ingest fields (T3b)",
  [(Q, "                    len: *len,\n                    code: *code,\n                }),\n",
    "                    len: *len + 1,\n                    code: *code,\n                }),\n")])

m("M10", "quic.rs rejections: IngestTransfer arm disabled (`if false` guard)",
  "B-1 transfer refusal",
  [(Q, "                    result: TransferDisposition::Refused(refusal),\n                } => out.push(Rejection::Transfer {\n",
    "                    result: TransferDisposition::Refused(refusal),\n                } if false => out.push(Rejection::Transfer {\n")])

m("M11", "quic.rs rejections: CheckFrame arm disabled (`if false` guard)",
  "B-1 CheckFrame error",
  [(Q, "                    result: Err(error),\n                    ..\n                } => out.push(Rejection::Frame {\n",
    "                    result: Err(error),\n                    ..\n                } if false => out.push(Rejection::Frame {\n")])

m("M12", "quic.rs rejections: Pump arm disabled (`if false` guard)",
  "B-1 pump admission (T2/T3a)",
  [(Q, "                HostCall::Pump { summary } => {\n",
    "                HostCall::Pump { summary } if false => {\n")])

m("M13", "quic.rs rejections: retained_in_ingress exclusion removed (`if true`)",
  "B-1 retained head (T2/T4)",
  [(Q, "                            if !result.retained_in_ingress {\n",
    "                            if true {\n")])

m("M14", "quic.rs rejections: retained_in_ingress exclusion flipped (only retained listed)",
  "B-1 retained head (T2)",
  [(Q, "                            if !result.retained_in_ingress {\n",
    "                            if result.retained_in_ingress {\n")])

m("M15", "quic.rs rejections: Admission.cached constant false",
  "B-1 admission fields (T2)",
  [(Q, "                                    cached: result.cached,\n",
    "                                    cached: false,\n")])

m("M16", "quic.rs rejections: Admission.seq constant None",
  "B-1 admission fields (T2)",
  [(Q, "                                    seq: result.seq,\n",
    "                                    seq: None,\n")])

m("M17", "quic.rs rejections: summary.results iterated in reverse",
  "B-1 admission order (T2)",
  [(Q, "                    for result in &summary.results {\n",
    "                    for result in summary.results.iter().rev() {\n")])

m("M18", "quic.rs rejections: Ingest arm disabled entirely (`if false` guard instead of the QueueFull guard)",
  "B-1 ingest refusal (T3b)",
  [(Q, "                } if *code != RejectionCode::QueueFull => out.push(Rejection::Ingest {\n",
    "                } if false => out.push(Rejection::Ingest {\n")])

# ---- quic.rs Display: B-2 ---------------------------------------------------
m("M19", "quic.rs Display Hello: `*code as u32` -> `*code as u8`",
  "B-2 hello code width (T1)",
  [(Q, '                write!(f, "hello conn={} code={}", conn.get(), *code as u32)\n',
    '                write!(f, "hello conn={} code={}", conn.get(), *code as u8)\n')])

m("M20", 'quic.rs Display Resume: prefix "resume" -> "hello"',
  "B-2 resume text",
  [(Q, '                write!(f, "resume conn={} code={}", conn.get(), *code as u32)\n',
    '                write!(f, "hello conn={} code={}", conn.get(), *code as u32)\n')])

m("M21", "quic.rs Display Ingest: len printed as len + 1",
  "B-2 ingest text (T3b)",
  [(Q, "                frame,\n                len,\n                *code as u8\n",
    "                frame,\n                len + 1,\n                *code as u8\n")])

m("M22", "quic.rs Display Transfer: peer field prints conn.get()",
  "B-2 transfer text",
  [(Q, '                "transfer conn={} peer={} frame={} len={} refusal={:?}",\n                conn.get(),\n                peer.get(),\n',
    '                "transfer conn={} peer={} frame={} len={} refusal={:?}",\n                conn.get(),\n                conn.get(),\n')])

m("M23", 'quic.rs Display Admission: None seq renders "0" instead of "-"',
  "B-2 admission seq (T2/T3a)",
  [(Q, '                    None => f.write_str("-")?,\n',
    '                    None => f.write_str("0")?,\n')])

m("M24", "quic.rs Display Admission: cached printed negated",
  "B-2 admission cached (T2)",
  [(Q, '                write!(f, " code={} cached={}", *code as u8, cached)\n',
    '                write!(f, " code={} cached={}", *code as u8, !cached)\n')])

m("M25", "quic.rs Display Frame: conn and peer arguments swapped",
  "B-2 frame text",
  [(Q, '                "frame conn={} peer={} error={}",\n                conn.get(),\n                peer.get(),\n',
    '                "frame conn={} peer={} error={}",\n                peer.get(),\n                conn.get(),\n')])

# ---- run.rs observer: B-3 ---------------------------------------------------
m("M26", "run.rs QuicHosting::intake: observer call removed",
  "B-3 observer called (T5)",
  [(R, "            observer(now_ms, &report);\n",
    "            let _ = (&observer, now_ms, &report);\n")])

m("M27", "run.rs QuicHosting::intake: observer receives 0 instead of now_ms",
  "B-3 observer now_ms (T5)",
  [(R, "            observer(now_ms, &report);\n", "            observer(0, &report);\n")])

m("M28", "run.rs QuicHosting::intake: observer called twice",
  "B-3 exactly once (T5)",
  [(R, "            observer(now_ms, &report);\n",
    "            observer(now_ms, &report);\n            observer(now_ms, &report);\n")])

m("M29", "run.rs set_intake_observer: a None never clears the observer",
  "B-3 cleared observer (T5)",
  [(R, "        self.observer = observer;\n",
    "        if observer.is_some() {\n            self.observer = observer;\n        }\n")])

m("M30", "run.rs QuicHosting::intake: on a failed pump the observer is still called with an empty report, then the error is returned (quic.rs QuicPumpReport::new made pub(crate) so it compiles; first variant, which swallowed the error, was killed by fa_6_intake_error_is_the_loop_error only)",
  "B-3 Ok only",
  [(R, "        let report = self.quic.pump(now_ms)?;\n",
    "        let report = match self.quic.pump(now_ms) {\n"
    "            Ok(report) => report,\n"
    "            Err(e) => {\n"
    "                if let Some(observer) = self.observer.as_mut() {\n"
    "                    observer(now_ms, &QuicPumpReport::new());\n"
    "                }\n"
    "                return Err(e);\n"
    "            }\n"
    "        };\n"),
   (Q, "impl QuicPumpReport {\n    fn new() -> Self {\n",
    "impl QuicPumpReport {\n    pub(crate) fn new() -> Self {\n")])


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


UNION = ["--lib", "--test", "run_intake_observer", "--test", "run_loop",
         "--test", "run_dedicated", "--test", "host_quic"]


def run_cargo(full):
    env = dict(os.environ, CARGO_INCREMENTAL="0")
    cmd = ["cargo", "test", "-p", "crpg-server", "--locked", "-j", "2"]
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
