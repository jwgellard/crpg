#!/usr/bin/env python3
"""PreToolUse hook: block edits to protected files (A0-6, TEMPLATES §6).

Reads one Claude Code hook JSON object on stdin, resolves the edited file's
repository root by walking up to the first ``.git`` entry, and matches the
path relative to that root against the protected rules. Exit 0 allows, exit 2
blocks with the reason on stderr. A hook bug must never block work: malformed
input, a missing ``file_path`` or a path outside any repository all exit 0.
"""

from __future__ import annotations

import json
import os
import sys

WINDOWS = os.name == "nt"

# Root-level manifests and toolchain/config files. Matched by basename; the
# root ``Cargo.toml`` is matched separately so crate manifests stay editable.
NAMED_FILES = ("rust-toolchain.toml", "Cargo.lock", "deny.toml")
PROTECTED_DIRS = ("crates/crpg-contracts", ".github/workflows")


def norm(path: str) -> str:
    """Relative path with '/' separators, lowercased on Windows."""
    path = path.replace("\\", "/")
    return path.lower() if WINDOWS else path


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


def rule_for(rel: str) -> str | None:
    """The protected rule matching *rel*, or None. First match wins."""
    rel = norm(rel)
    parts = rel.split("/")
    base = parts[-1]
    if base == "cargo.toml" and len(parts) == 1:
        return "Cargo.toml"
    for name in NAMED_FILES:
        if base == norm(name):
            return name
    for directory in PROTECTED_DIRS:
        if rel == directory or rel.startswith(directory + "/"):
            return directory + "/**"
    if "goldens" in parts:
        return "goldens/"
    if base.endswith(".golden"):
        return "*.golden"
    if base.endswith(".replay"):
        return "*.replay"
    return None


def allowlisted(rel: str) -> bool:
    listed = os.environ.get("CRPG_ALLOW_PROTECTED", "")
    wanted = {norm(entry.strip()) for entry in listed.split(",") if entry.strip()}
    return norm(rel) in wanted


def main() -> int:
    try:
        data = json.loads(sys.stdin.read() or "{}")
        file_path = (data.get("tool_input") or {}).get("file_path")
        if not isinstance(file_path, str) or not file_path:
            raise ValueError("no file_path in hook input")
        absolute = os.path.abspath(file_path)
        root = find_root(os.path.dirname(absolute))
        if root is None:
            return 0
        rel = os.path.relpath(absolute, root).replace(os.sep, "/")
        rule = rule_for(rel)
        if rule is None or allowlisted(rel):
            return 0
        sys.stderr.write(
            f"protect.py: {rel} is protected ({rule}); ask the user, "
            f"then re-run with CRPG_ALLOW_PROTECTED={rel}\n"
        )
        return 2
    except Exception as exc:  # noqa: BLE001 - a hook bug must not block work.
        sys.stderr.write(f"protect.py: hook error, allowing: {exc}\n")
        return 0


if __name__ == "__main__":
    raise SystemExit(main())
