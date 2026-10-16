#!/usr/bin/env python3
"""PostToolUse hook: report the session's exact context size (A0-6b).

Reads a Claude Code hook JSON object on stdin with ``session_id`` and
``transcript_path``, and takes the size from the last transcript line carrying
``message.usage``: ``input_tokens + cache_read_input_tokens +
cache_creation_input_tokens``, a missing field counting 0. Only the last 4 MB of
the transcript is read, since transcripts reach tens of MB.

At ``CRPG_CONTEXT_WARN`` (default 150000) the notice asks the model to finish
the package in hand; at ``CRPG_CONTEXT_STOP`` (default 200000), and again every
25000 tokens above it, it asks for an immediate handover. Each level is announced
at most once per session; the last announced level lives in
``<repo>/.agents/context-meter/<session_id>.json``, where the repo is the first
ancestor of the transcript's ``cwd`` (or the process cwd) that holds ``.git``.

The hook always exits 0 and writes at most one stderr line, so a hook bug can
never block or disturb work.
"""

from __future__ import annotations

import json
import os
import sys

TAIL_BYTES = 4 * 1024 * 1024
REPEAT_STEP = 25000
DEFAULT_WARN = 150000
DEFAULT_STOP = 200000
WARN_MESSAGE = (
    "Context meter: about {n}k tokens (warn at {warn}k). Finish the package in "
    "hand, start no new one, and plan the handover (docs/AGENT_TEAM_PLAN.md §8)."
)
STOP_MESSAGE = (
    "Context meter: about {n}k tokens (stop at {stop}k). Hand over now: update "
    "the handover in docs/PROJECT_STATE.md, naming every run still going, "
    "commit, and tell the user to start a fresh session "
    "(docs/AGENT_TEAM_PLAN.md §8)."
)


def env_int(name: str, default: int) -> int:
    try:
        return int(os.environ.get(name, ""))
    except (TypeError, ValueError):
        return default


def read_tail(path: str) -> str:
    """Decode the last 4 MB of *path*, dropping any partial leading line."""
    size = os.path.getsize(path)
    with open(path, "rb") as handle:
        start = max(0, size - TAIL_BYTES)
        handle.seek(start)
        data = handle.read()
    if start > 0:
        # The first line of the window is cut mid-line unless it starts at a
        # newline; drop it rather than feed JSON a truncated object.
        newline = data.find(b"\n")
        data = data[newline + 1 :] if newline != -1 else b""
    return data.decode("utf-8", errors="replace")


def _to_int(value: object) -> int:
    return value if isinstance(value, int) and not isinstance(value, bool) else 0


def scan_transcript(text: str) -> tuple[int | None, str | None]:
    """The last usage line's size and the last transcript ``cwd``.

    Malformed and non-object lines are skipped; the size is ``None`` when no
    line carries ``message.usage``.
    """
    size: int | None = None
    cwd: str | None = None
    for raw in text.split("\n"):
        raw = raw.strip()
        if not raw:
            continue
        try:
            obj = json.loads(raw)
        except ValueError:
            continue
        if not isinstance(obj, dict):
            continue
        if isinstance(obj.get("cwd"), str) and obj["cwd"]:
            cwd = obj["cwd"]
        message = obj.get("message")
        usage = message.get("usage") if isinstance(message, dict) else None
        if isinstance(usage, dict):
            size = (
                _to_int(usage.get("input_tokens"))
                + _to_int(usage.get("cache_read_input_tokens"))
                + _to_int(usage.get("cache_creation_input_tokens"))
            )
    return size, cwd


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


def level_for(tokens: int, warn: int, stop: int) -> int:
    """0 below *warn*, 1 at *warn*, then 2 + one per ``REPEAT_STEP`` at *stop*."""
    if tokens < warn:
        return 0
    if tokens < stop:
        return 1
    return 2 + (tokens - stop) // REPEAT_STEP


def notice(level: int, tokens: int, warn: int, stop: int) -> str:
    if level == 1:
        return WARN_MESSAGE.format(n=tokens // 1000, warn=warn // 1000)
    return STOP_MESSAGE.format(n=tokens // 1000, stop=stop // 1000)


def state_path(root: str, session_id: str) -> str:
    safe = "".join(ch if ch.isalnum() or ch in "-_." else "_" for ch in session_id)
    return os.path.join(root, ".agents", "context-meter", (safe or "session") + ".json")


def read_level(path: str | None) -> int:
    if not path:
        return 0
    try:
        with open(path, encoding="utf-8") as handle:
            data = json.load(handle)
        level = data.get("level") if isinstance(data, dict) else None
        return level if isinstance(level, int) and not isinstance(level, bool) else 0
    except (OSError, ValueError):
        return 0


def write_level(path: str | None, level: int) -> None:
    """Best-effort write of *level*; a failure leaves the notice standing."""
    if not path:
        return
    try:
        os.makedirs(os.path.dirname(path), exist_ok=True)
        temp = path + ".tmp"
        with open(temp, "w", encoding="utf-8") as handle:
            json.dump({"level": level}, handle)
        os.replace(temp, path)
    except OSError:
        pass


def main() -> int:
    try:
        try:
            data = json.loads(sys.stdin.read() or "{}")
        except ValueError as exc:
            raise ValueError(f"bad stdin: {exc}") from exc
        if not isinstance(data, dict):
            raise ValueError("hook input is not an object")
        session_id = data.get("session_id")
        transcript = data.get("transcript_path")
        if not isinstance(session_id, str) or not session_id:
            raise ValueError("no session_id in hook input")
        if not isinstance(transcript, str) or not transcript:
            raise ValueError("no transcript_path in hook input")

        tokens, cwd = scan_transcript(read_tail(transcript))
        if tokens is None:
            raise ValueError("no message.usage line in transcript")

        warn = env_int("CRPG_CONTEXT_WARN", DEFAULT_WARN)
        stop = env_int("CRPG_CONTEXT_STOP", DEFAULT_STOP)
        level = level_for(tokens, warn, stop)
        if level == 0:
            return 0

        root = find_root(cwd if cwd else os.getcwd())
        path = state_path(root, session_id) if root else None
        if level <= read_level(path):
            return 0
        payload = {
            "hookSpecificOutput": {
                "hookEventName": "PostToolUse",
                "additionalContext": notice(level, tokens, warn, stop),
            }
        }
        # Print before recording: an unwritable state file must not stop the
        # notice from reaching the model.
        sys.stdout.write(json.dumps(payload) + "\n")
        write_level(path, level)
        return 0
    except Exception as exc:  # noqa: BLE001 - a hook bug must not block work.
        sys.stderr.write(f"context_meter.py: hook error, skipping: {exc}\n")
        return 0


if __name__ == "__main__":
    raise SystemExit(main())
