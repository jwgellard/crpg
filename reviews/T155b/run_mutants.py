#!/usr/bin/env python3
"""Mutant driver for T155b (crpg-net-quic resumable movement endpoint).

Applies one mutant at a time by exact text replacement in the source file the
mutant table cites (the files the mutant table cites), runs
`cargo test -p crpg-net-quic --locked -j 2`
with CARGO_INCREMENTAL=0, records one JSON line per mutant in results.jsonl,
and restores the original bytes in a `finally`.

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
OUT = ROOT / "reviews/T155b/results.jsonl"
TIMEOUT = 900
# Optional: MUTANT_DUMP names a directory to keep each mutant's cargo output in
# (diagnosis aid; not part of the record).
DUMP = os.environ.get("MUTANT_DUMP")

# id -> (description, contract row, [(file, old, new), ...])
M = {}


def m(mid, desc, item, edits):
    M[mid] = (desc, item, edits)


# ---- T155b mutants (tasks/T155.md N-1 to N-4, crpg-net-quic) ---------------
SRV = "crates/crpg-net-quic/src/server.rs"
CLI = "crates/crpg-net-quic/src/client.rs"

m("M0", "unmutated run of the whole crpg-net-quic test suite (must pass)", "-", [])

BIND = "        Self::bind_setup(config, &[Framing::V2], true, true)\n"
m("M1", "server.rs:256 (N-1): bind_movement_resumable passes resumable=false", "N-1",
  [(SRV, BIND, "        Self::bind_setup(config, &[Framing::V2], false, true)\n")])
m("M2", "server.rs:256 (N-1): bind_movement_resumable passes movement=false", "N-1",
  [(SRV, BIND, "        Self::bind_setup(config, &[Framing::V2], true, false)\n")])

ACC = ("        record.answer = Some(Answer::Resume(answer.to_vec()));\n"
       "        record.reader_active = true;\n"
       "        record.lane1_reader_active = record.movement;\n")
m("M3", "server.rs:463 (N-3): accept_resume does not set lane1_reader_active", "N-3",
  [(SRV, ACC, ACC.replace("        record.lane1_reader_active = record.movement;\n", ""))])
m("M4", "server.rs:463 (N-3): accept_resume sets lane1_reader_active = true unconditionally", "N-3",
  [(SRV, ACC, ACC.replace("= record.movement;", "= true;"))])

LIM = ("        limits.validate().map_err(ConnectError::Config)?;\n"
       "        session\n            .encode_for(true)\n")
m("M5", "client.rs:384 (N-4 (1)): connect_movement_resume skips limits.validate()", "N-4(1)",
  [(CLI, LIM, "        session\n            .encode_for(true)\n")])

ENC = ("        session\n            .encode_for(true)\n"
       "            .map_err(|_| ConnectError::Handshake)?;\n")
m("M6", "client.rs:385 (N-4 (2)): the session.encode_for(true) check is skipped", "N-4(2)",
  [(CLI, ENC, "        let _ = &session;\n")])
m("M7", "client.rs:386 (N-4 (2)): session checked with encode_for(false) (movement flag inverted)", "N-4(2)",
  [(CLI, ENC, ENC.replace("encode_for(true)", "encode_for(false)"))])

REQ = (ENC + "        if request.is_empty() || request.len() > MAX_RESUME_FRAME_BYTES {\n")
m("M8", "client.rs:392 (N-4 (3)): the empty-request check is removed", "N-4(3)",
  [(CLI, REQ, ENC + "        if request.len() > MAX_RESUME_FRAME_BYTES {\n")])
m("M9", "client.rs:392 (N-4 (3)): the over-length request check is removed", "N-4(3)",
  [(CLI, REQ, ENC + "        if request.is_empty() {\n")])
m("M10", "client.rs:392 (N-4 (3)): the length bound is off by one (>= instead of >)", "N-4(3)",
  [(CLI, REQ, ENC + "        if request.is_empty() || request.len() >= MAX_RESUME_FRAME_BYTES {\n")])

FR = ("        let request_frame = framed_channel(Channel::Resume, request, MAX_RESUME_FRAME_BYTES)\n"
      "            .map_err(|_| ConnectError::Handshake)?;\n"
      "        let mismatch = Arc::new(AtomicBool::new(false));\n"
      "        let quinn_config = tls::client_movement_config(config.pin, mismatch.clone(), &limits)\n")
m("M11", "client.rs:395 (N-4 (4)): the opening frame is framed on Channel::Lane0, not Resume", "N-4(4)",
  [(CLI, FR, FR.replace("Channel::Resume", "Channel::Lane0"))])
m("M12", "client.rs:398 (N-4 (5)): the quinn config offers the /2 ALPN (tls::client_config), not /3", "N-4(5)",
  [(CLI, FR, FR.replace("tls::client_movement_config(config.pin, mismatch.clone(), &limits)",
                        "tls::client_config(config.pin, mismatch.clone(), ALPN_LANE0_V2, &limits)"))])

ATT = ("            kind: Kind::Resume {\n                request_frame,\n"
       "                welcome: session,\n            },\n"
       "            framing: Framing::V2,\n            movement: true,\n")
m("M13", "client.rs:424 (N-4 (6)): Kind::Resume carries a zeroed epoch in the session welcome", "N-4(6)",
  [(CLI, ATT, ATT.replace("welcome: session,", "welcome: Welcome { epoch: [0; 16], ..session },"))])
m("M14", "client.rs:427 (N-4 (6)): the attempt's framing is V1", "N-4(6)",
  [(CLI, ATT, ATT.replace("framing: Framing::V2", "framing: Framing::V1"))])
m("M15", "client.rs:428 (N-4 (6)): the attempt's movement is false", "N-4(6)",
  [(CLI, ATT, ATT.replace("movement: true", "movement: false"))])

RET = ("                        limits,\n                        framing: Framing::V2,\n"
       "                        movement: true,\n                    },\n                    answer,\n")
m("M16", "client.rs:445 (N-4 result): the returned client's movement is false", "N-4",
  [(CLI, RET, RET.replace("movement: true", "movement: false"))])
m("M17", "client.rs:444 (N-4 result): the returned client's framing is V1", "N-4",
  [(CLI, RET, RET.replace("framing: Framing::V2", "framing: Framing::V1"))])
m("M18", "client.rs:447 (N-4 result): the returned answer payload is empty", "N-4",
  [(CLI, RET, RET.replace("                    answer,\n", "                    { let _ = answer; Vec::new() },\n"))])

H = ("            // T155 N-4 (7): on `/3` lane 1 opens after the answer, as after\n"
     "            // a welcome, inside the same timer.\n"
     "            let lane1 = if movement {\n")
m("M19", "client.rs:1124 (N-4 (7)): handshake resume path never opens stream 2 (if false)", "N-4(7)",
  [(CLI, H, H.replace("if movement {", "if false {"))])
m("M20", "client.rs:1124 (N-4 (7)): handshake resume path opens stream 2 when NOT movement (if !movement)", "N-4(7)",
  [(CLI, H, H.replace("if movement {", "if !movement {"))])

HB = (H + "                Some(\n                    connection\n                        .open_bi()\n"
      "                        .await\n                        .map_err(|error| lost_error(&error))?,\n"
      "                )\n            } else {\n                None\n            };\n")
OPEN = HB[HB.index("            let lane1 = if movement {"):]
m("M21", "client.rs:1124 (N-4 (7)): stream 2 is opened before the request is written/answer read, not after", "N-4(7)",
  [(CLI, HB, ""),
   (CLI, "            send.write_all(&request_frame)\n",
    OPEN + "            send.write_all(&request_frame)\n")])
m("M22", "client.rs:1128 (N-4 (7)): an open_bi failure after the answer maps to Handshake, not lost_error", "N-4(7)",
  [(CLI, HB, HB.replace("lost_error(&error)", "ConnectError::Handshake"))])

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
    cmd = ["cargo", "test", "-p", "crpg-net-quic", "--locked", "-j", "2"]
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
    failed = []
    suite = "?"
    for ln in out.splitlines():
        t = ln.strip()
        if t.startswith("Running "):
            suite = t.split()[1].replace("\\", "/").split("/")[-1]
        if ln.startswith("test ") and ln.rstrip().endswith("FAILED"):
            failed.append(suite + "::" + ln.split()[1])
    if rc == 0 and not failed:
        return "survived", []
    return "killed", failed


def main():
    args = sys.argv[1:]
    check = "--check" in args
    ids = [a for a in args if a != "--check"] or list(M)
    originals = read_originals()
    # results.jsonl holds batches: keep the rows of earlier batches, replace the
    # rows this invocation runs, and write the file back in table order.
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
