#!/usr/bin/env python3
"""PostToolUse hook: rustfmt an edited ``.rs`` file (A0-6, TEMPLATES §6).

Reads one Claude Code hook JSON object on stdin and, when ``file_path`` ends
in ``.rs`` and exists, runs ``rustfmt --edition 2021`` on it. Always exits 0;
a rustfmt failure or a missing rustfmt prints one stderr line. Other files are
ignored silently.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys


def main() -> int:
    try:
        data = json.loads(sys.stdin.read() or "{}")
        file_path = (data.get("tool_input") or {}).get("file_path")
    except Exception as exc:  # noqa: BLE001 - a hook bug must not block work.
        sys.stderr.write(f"fmt_one.py: hook error, skipping: {exc}\n")
        return 0
    if not isinstance(file_path, str) or not file_path.lower().endswith(".rs"):
        return 0
    if not os.path.isfile(file_path):
        return 0
    rustfmt = shutil.which("rustfmt")
    if rustfmt is None:
        sys.stderr.write("fmt_one.py: rustfmt not found on PATH; skipped\n")
        return 0
    try:
        result = subprocess.run(
            [rustfmt, "--edition", "2021", file_path],
            capture_output=True,
            text=True,
        )
    except OSError as exc:
        sys.stderr.write(f"fmt_one.py: could not run rustfmt: {exc}\n")
        return 0
    if result.returncode != 0:
        detail = (result.stderr or result.stdout or "").strip().splitlines()
        reason = detail[0] if detail else f"exit {result.returncode}"
        sys.stderr.write(f"fmt_one.py: rustfmt failed on {file_path}: {reason}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
