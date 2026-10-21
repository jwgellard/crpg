#!/usr/bin/env python3
"""Fake verifier for tools/tests/test_oc.py (stands in for A0-4 verify_task.py).

Usage: fake_verify.py <ID> --slot <path> --json [--skip gates]

Behaviour from $FAKE_VERIFY_MODE:
  pass   exit 0, verdict PASS, two files and three ok checks.
  fail   exit 1, verdict FAIL, one failed check with a long output tail.
  error  exit 2, verdict ERROR.
  empty  exit 0, verdict PASS, no files (the worker changed nothing).
The received argv is appended to $FAKE_VERIFY_LOG for --skip gates checks.
"""

from __future__ import annotations

import json
import os
import sys


def main() -> int:
    log = os.environ.get("FAKE_VERIFY_LOG", "")
    if log:
        with open(log, "a", encoding="utf-8") as handle:
            handle.write(json.dumps(sys.argv[1:]) + "\n")
    task = sys.argv[1] if len(sys.argv) > 1 else "T000"
    fail_tasks = {
        piece.strip()
        for piece in os.environ.get("FAKE_VERIFY_FAIL_TASKS", "").split(",")
        if piece.strip()
    }
    mode = os.environ.get("FAKE_VERIFY_MODE", "pass")
    if mode == "empty":
        payload = {
            "tool": "verify_task",
            "version": 1,
            "task": task,
            "verdict": "PASS",
            "error": None,
            "checks": [{"name": "scope", "status": "ok", "detail": "", "output": []}],
            "files": [],
        }
        sys.stdout.write(json.dumps(payload))
        return 0
    if mode == "fail" or task in fail_tasks:
        payload = {
            "tool": "verify_task",
            "version": 1,
            "task": task,
            "verdict": "FAIL",
            "error": None,
            "checks": [
                {"name": "scope", "status": "ok", "detail": "", "output": []},
                {"name": "clippy", "status": "failed", "detail": "2 errors",
                 "output": [f"error line {i}" for i in range(30)]},
            ],
            "files": [{"path": "src/lib.rs", "added": 10, "deleted": 2, "status": "modified"}],
        }
        sys.stdout.write(json.dumps(payload))
        return 1
    if mode == "fail-big":
        payload = {
            "tool": "verify_task",
            "version": 1,
            "task": "T000",
            "verdict": "FAIL",
            "error": None,
            "checks": [
                {"name": f"check-{i}", "status": "failed", "detail": "bad",
                 "output": [f"check {i} line {j}" for j in range(30)]}
                for i in range(5)
            ],
            "files": [{"path": "src/lib.rs", "added": 10, "deleted": 2, "status": "modified"}],
        }
        sys.stdout.write(json.dumps(payload))
        return 1
    if mode == "error":
        payload = {
            "tool": "verify_task",
            "version": 1,
            "task": "T000",
            "verdict": "ERROR",
            "error": "boom",
            "checks": [],
            "files": [],
        }
        sys.stdout.write(json.dumps(payload))
        return 2
    payload = {
        "tool": "verify_task",
        "version": 1,
        "task": "T000",
        "verdict": "PASS",
        "error": None,
        "checks": [
            {"name": "scope", "status": "ok", "detail": "", "output": []},
            {"name": "gates", "status": "ok", "detail": "", "output": []},
            {"name": "goldens", "status": "ok", "detail": "unchanged", "output": []},
        ],
        "files": [
            {"path": "src/lib.rs", "added": 212, "deleted": 3, "status": "modified"},
            {"path": "tests/t.rs", "added": 160, "deleted": 0, "status": "added"},
        ],
    }
    sys.stdout.write(json.dumps(payload))
    return 0


if __name__ == "__main__":
    sys.exit(main())
