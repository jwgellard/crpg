#!/usr/bin/env python3
"""Mutant driver for T151c (apps/client/tests/movement.gd, M1-M5).

Applies one mutant at a time by exact text replacement in
`crates/crpg-godot/src/shell.rs`, runs
  cargo build -p crpg-godot --features engine --locked -j 2   (CARGO_INCREMENTAL=0)
  Godot4 --headless --path apps/client -s res://tests/movement.gd
and records one JSON line per mutant in results.jsonl. The original bytes are
restored in a `finally`. Untracked *.uid files under apps/client are deleted
after every mutant. Run it from Git Bash (Godot4 is a windowed program).

usage: run_mutants.py [--check] [--baseline] [ids...]
"""
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "crates/crpg-godot/src"
OUT = ROOT / "reviews/T151c/results.jsonl"
BUILD_TIMEOUT = 600
GODOT_TIMEOUT = 120
SH = "shell.rs"

M = {}


def m(mid, desc, item, edits):
    M[mid] = (desc, item, edits)


OPEN_V4_CALL = (
    "        let code = self.core.open_v4(\n"
    "            &campaign_dir.to_string(),\n"
    "            &encounter_id.to_string(),\n"
    "            &player_side.to_string(),\n"
    "            seed,\n"
    "        );\n"
)

RECEIPTS_HEAD = (
    "        self.view\n"
    "            .move_receipts()\n"
    "            .iter()\n"
)

# ---- T151c mutant table (tasks/T151.md, B-C1, B-C2, tests M1-M5) -----------
m("C01", "shell.rs open_v4: calls core.open (v2 entry point) instead of core.open_v4",
  "B-C1",
  [(SH, OPEN_V4_CALL, OPEN_V4_CALL.replace("core.open_v4(", "core.open("))])

m("C02", "shell.rs open_v4: the `opened_at = Some(Instant::now())` assignment removed",
  "B-C1",
  [(SH,
    "        let code = code.as_i64();\n"
    "        if code == 0 {\n"
    "            self.opened_at = Some(Instant::now());\n"
    "        }\n"
    "        code\n"
    "    }\n\n"
    "    /// Turn-based move",
    "        let code = code.as_i64();\n"
    "        code\n"
    "    }\n\n"
    "    /// Turn-based move")])

m("C03", "shell.rs open_v4: encounter_id and player_side arguments swapped",
  "B-C1",
  [(SH, OPEN_V4_CALL,
    OPEN_V4_CALL.replace(
        "&encounter_id.to_string(),\n            &player_side.to_string(),",
        "&player_side.to_string(),\n            &encounter_id.to_string(),"))])

m("C04", "shell.rs open_v4: player_side replaced by the constant \"\" (default side)",
  "B-C1",
  [(SH, OPEN_V4_CALL,
    OPEN_V4_CALL.replace("&player_side.to_string(),", "\"\","))])

m("C05", "shell.rs declare_move: revision argument replaced by the constant 0",
  "B-C1",
  [(SH, "self.core.declare_move(revision, x, y, z)",
    "self.core.declare_move(0, x, y, z)")])

m("C06", "shell.rs declare_move: x and z swapped in the core call",
  "B-C1",
  [(SH, "self.core.declare_move(revision, x, y, z)",
    "self.core.declare_move(revision, z, y, x)")])

m("C07", "shell.rs declare_move: y and z swapped in the core call",
  "B-C1",
  [(SH, "self.core.declare_move(revision, x, y, z)",
    "self.core.declare_move(revision, x, z, y)")])

m("C08", "shell.rs move_to: actor argument replaced by the constant 2",
  "B-C1",
  [(SH, "self.core.move_to(actor, x, y, z)",
    "self.core.move_to(2, x, y, z)")])

m("C09", "shell.rs move_to: calls core.halt(actor) instead of core.move_to (wrong entry point)",
  "B-C1",
  [(SH,
    "    fn move_to(&mut self, actor: i64, x: i64, y: i64, z: i64) -> i64 {\n"
    "        self.core.move_to(actor, x, y, z).as_i64()\n",
    "    fn move_to(&mut self, actor: i64, x: i64, y: i64, z: i64) -> i64 {\n"
    "        let _ = (x, y, z);\n"
    "        self.core.halt(actor).as_i64()\n")])

m("C10", "shell.rs move_to: destination (x, y, z) replaced by the constant (0, 0, 0)",
  "B-C1",
  [(SH, "self.core.move_to(actor, x, y, z)",
    "self.core.move_to(actor, 0, 0, 0)")])

m("C11", "shell.rs halt: calls core.move_to(actor, 0, 0, 0) instead of core.halt (wrong entry point)",
  "B-C1",
  [(SH, "        self.core.halt(actor).as_i64()\n",
    "        self.core.move_to(actor, 0, 0, 0).as_i64()\n")])

m("C12", "shell.rs halt: return code replaced by the constant 0 (core call still made)",
  "B-C1",
  [(SH, "        self.core.halt(actor).as_i64()\n",
    "        let _ = self.core.halt(actor).as_i64();\n        0\n")])

m("C13", "shell.rs halt: actor argument replaced by the constant 2",
  "B-C1",
  [(SH, "self.core.halt(actor)", "self.core.halt(2)")])

m("C14", "shell.rs positions: getter returns an empty array",
  "B-C2",
  [(SH, "PackedInt64Array::from(self.view.positions())",
    "PackedInt64Array::new()")])

m("C15", "shell.rs positions: copied elements reversed",
  "B-C2",
  [(SH, "PackedInt64Array::from(self.view.positions())",
    "PackedInt64Array::from(self.view.positions().iter().rev().copied().collect::<Vec<i64>>())")])

m("C16", "shell.rs position_known: copies moving() instead of position_known()",
  "B-C2",
  [(SH, "PackedByteArray::from(self.view.position_known())",
    "PackedByteArray::from(self.view.moving())")])

m("C17", "shell.rs moving: returns all zeros of the right length",
  "B-C2",
  [(SH, "PackedByteArray::from(self.view.moving())",
    "PackedByteArray::from(vec![0u8; self.view.moving().len()])")])

m("C18", "shell.rs move_receipts: getter returns an empty array (.take(0))",
  "B-C2",
  [(SH, RECEIPTS_HEAD, RECEIPTS_HEAD + "            .take(0)\n")])

m("C19", "shell.rs move_receipts: receipts iterated newest first (.rev())",
  "B-C2",
  [(SH, RECEIPTS_HEAD, RECEIPTS_HEAD + "            .rev()\n")])

m("C20", "shell.rs move_receipts: copies receipts() (lane-0 receipts) instead of move_receipts()",
  "B-C2",
  [(SH, RECEIPTS_HEAD,
    RECEIPTS_HEAD.replace(".move_receipts()", ".receipts()"))])

FILES = [SH]


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


def run(cmd, env, timeout):
    p = subprocess.Popen(cmd, cwd=ROOT, env=env, stdout=subprocess.PIPE,
                         stderr=subprocess.STDOUT, text=True, encoding="utf-8",
                         errors="replace")
    try:
        out, _ = p.communicate(timeout=timeout)
        return p.returncode, out, False
    except subprocess.TimeoutExpired:
        p.kill()
        try:
            out, _ = p.communicate(timeout=30)
        except Exception:
            out = ""
        return -1, out, True


def clean_uids():
    for p in (ROOT / "apps/client").rglob("*.uid"):
        tracked = subprocess.run(["git", "ls-files", "--error-unmatch", str(p)],
                                 cwd=ROOT, capture_output=True).returncode == 0
        if not tracked:
            p.unlink()


def build():
    env = dict(os.environ, CARGO_INCREMENTAL="0")
    env.pop("CARGO_TARGET_DIR", None)
    env.pop("GDRUST_MAIN_EXTENSION", None)
    return run(["cargo", "build", "-p", "crpg-godot", "--features", "engine",
                "--locked", "-j", "2"], env, BUILD_TIMEOUT)


def godot():
    exe = shutil.which("Godot4") or "Godot4"
    try:
        return run([exe, "--headless", "--path", "apps/client", "-s",
                    "res://tests/movement.gd"], dict(os.environ), GODOT_TIMEOUT)
    finally:
        clean_uids()


def evaluate():
    rc, out, to = build()
    if to:
        return "killed", {"detail": "build timeout"}
    if rc != 0:
        return "compile-error", {"detail": [ln for ln in out.splitlines()
                                            if ln.startswith("error")][:3]}
    rc, out, to = godot()
    tail = [ln for ln in out.splitlines() if "movement.gd" in ln][-3:]
    if to:
        return "killed", {"exit_code": "timeout", "godot": tail}
    if rc != 0:
        return "killed", {"exit_code": rc, "godot": tail}
    return "survived", {"exit_code": rc, "godot": tail}


def main():
    args = sys.argv[1:]
    check = "--check" in args
    baseline = "--baseline" in args
    ids = [a for a in args if not a.startswith("--")] or list(M)
    originals = read_originals()
    if baseline:
        try:
            status, extra = evaluate()
        finally:
            for name, data in originals.items():
                (SRC / name).write_bytes(data)
        print("baseline", status, extra, flush=True)
        sys.exit(0 if status == "survived" else 1)
    lines = {}
    if not check:
        if OUT.exists():
            for line in OUT.read_text(encoding="utf-8").splitlines():
                if line.strip():
                    lines[json.loads(line)["id"]] = line
        for mid in ids:
            lines.pop(mid, None)

    def flush():
        OUT.write_text("".join(lines[k] + "\n" for k in M if k in lines),
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
            try:
                for name, data in mutated.items():
                    (SRC / name).write_bytes(data)
                status, extra = evaluate()
            finally:
                for name in mutated:
                    (SRC / name).write_bytes(originals[name])
            rec.update(status=status, seconds=round(time.time() - t0, 1), **extra)
            if status == "survived":
                rec["class"] = "UNCLASSIFIED"
            lines[mid] = json.dumps(rec)
            flush()
            print(mid, status, extra.get("exit_code", extra.get("detail")), flush=True)
    finally:
        for name, data in originals.items():
            (SRC / name).write_bytes(data)


if __name__ == "__main__":
    main()
