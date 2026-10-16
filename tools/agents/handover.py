#!/usr/bin/env python3
"""SessionStart hook: re-print the handover after a compaction (A0-6).

Reads one Claude Code hook JSON object on stdin, finds the repository root from
its ``cwd`` (falling back to the process cwd), and prints the ``## Handover``
block from ``docs/PROJECT_STATE.md`` up to the next ``## `` heading, prefixed
by one line. A missing file or block prints nothing and exits 0.
"""

from __future__ import annotations

import json
import os
import sys

HEADING = "## Handover"
PREFIX = "Handover from docs/PROJECT_STATE.md (re-injected after compaction):"


def find_root(start: str) -> str | None:
    """Walk up from *start* to the first directory containing ``.git``."""
    cur = os.path.abspath(start)
    while True:
        if os.path.isdir(os.path.join(cur, ".git")) or os.path.isfile(
            os.path.join(cur, ".git")
        ):
            return cur
        parent = os.path.dirname(cur)
        if parent == cur:
            return None
        cur = parent


def handover_block(text: str) -> list[str] | None:
    """The ``## Handover`` heading and its lines up to the next heading."""
    lines = text.splitlines()
    try:
        start = next(i for i, line in enumerate(lines) if line.strip() == HEADING)
    except StopIteration:
        return None
    block: list[str] = []
    for line in lines[start:]:
        if block and line.startswith("## "):
            break
        block.append(line)
    while block and not block[-1].strip():
        block.pop()
    return block


def main() -> int:
    cwd: object = None
    try:
        data = json.loads(sys.stdin.read() or "{}")
        if isinstance(data, dict):
            cwd = data.get("cwd")
    except Exception:  # noqa: BLE001 - fall back to the process cwd.
        cwd = None
    start = cwd if isinstance(cwd, str) and cwd else os.getcwd()
    root = find_root(start)
    if root is None:
        return 0
    path = os.path.join(root, "docs", "PROJECT_STATE.md")
    try:
        with open(path, encoding="utf-8") as handle:
            text = handle.read()
    except OSError:
        return 0
    block = handover_block(text)
    if not block:
        return 0
    sys.stdout.write(PREFIX + "\n" + "\n".join(block) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
