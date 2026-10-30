#!/usr/bin/env python3
"""Doc-move check for documentation trims (A0-16, lesson L8).

Doc trims that move prose from a crate's ``AGENTS.md`` into its architecture
doc -- or condense one file -- have dropped whole blocks while the worker
reported "no info lost". This tool compares the base text of one or more
``--from`` files with the concatenated head text of one or more ``--to``
files and reports every block whose 8-word shingles mostly disappeared.

Verdict first, exit 0 when nothing was lost, 1 when a block was, and 2 on a
usage or git error. Standard library only; UTF-8 (``utf-8-sig``) everywhere.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

SHINGLE_WORDS = 8
SHINGLE_STRIDE = 4
MAX_DETAIL_LINES = 40

# A list item (`-`, `*`, `N.`) or a table row starts a new block.
LIST_ITEM = re.compile(r"^\s*(?:[-*]|\d+\.|\|)\s")


def git_show(ref: str, path: str) -> str:
    """The text of *path* at *ref*, or empty when the path is absent."""
    git_path = path.replace("\\", "/") if os.name == "nt" else path
    result = subprocess.run(
        ["git", "show", f"{ref}:{git_path}"],
        capture_output=True,
        text=True,
        encoding="utf-8-sig",
        errors="replace",
    )
    return result.stdout if result.returncode == 0 else ""


def read_head(path: str) -> str:
    """The working-tree text of *path*, or empty when the file is absent."""
    try:
        return Path(path).read_text(encoding="utf-8-sig")
    except OSError:
        return ""


def valid_ref(ref: str) -> bool:
    result = subprocess.run(
        ["git", "rev-parse", "--verify", "--quiet", f"{ref}^{{commit}}"],
        capture_output=True,
        text=True,
    )
    return result.returncode == 0


def normalise(text: str) -> str:
    """Collapse whitespace and markdown punctuation, then lower-case."""
    return re.sub(r"[\s`*_>|-]+", " ", text).strip().lower()


def split_blocks(text: str) -> list[tuple[int, str]]:
    """Split *text* into ``(first line number, block text)`` pairs."""
    blocks: list[tuple[int, str]] = []
    buffer: list[str] = []
    start = 0

    def flush() -> None:
        if buffer:
            blocks.append((start, " ".join(buffer)))
            buffer.clear()

    for number, raw in enumerate(text.splitlines(), 1):
        if not raw.strip() or raw.startswith("#"):
            flush()
            continue
        if LIST_ITEM.match(raw) and buffer:
            flush()
        if not buffer:
            start = number
        buffer.append(raw.strip())
    flush()
    return blocks


def shingles(words: list[str]) -> list[str]:
    """8-word shingles at stride 4, always at least one."""
    stop = max(1, len(words) - (SHINGLE_WORDS - 1))
    return [
        " ".join(words[index : index + SHINGLE_WORDS])
        for index in range(0, stop, SHINGLE_STRIDE)
    ]


def collect(
    base: str,
    head: str | None,
    from_paths: list[str],
    to_paths: list[str],
    min_chars: int,
    threshold: float,
) -> tuple[int, list[dict]]:
    """Compare every base ``--from`` block against the concatenated head text."""
    head_text = "\n".join(
        git_show(head, path) if head else read_head(path) for path in to_paths
    )
    flat_head = normalise(head_text).replace("\n", " ")

    checked = 0
    lost: list[dict] = []
    for path in from_paths:
        for line, block in split_blocks(git_show(base, path)):
            normalised = normalise(block)
            if len(normalised) < min_chars:
                continue
            checked += 1
            pieces = shingles(normalised.split())
            hits = sum(1 for piece in pieces if piece in flat_head)
            ratio = hits / len(pieces)
            if ratio < threshold:
                lost.append(
                    {
                        "path": path,
                        "line": line,
                        "ratio": round(ratio, 2),
                        "text": block[:100],
                    }
                )
    return checked, lost


def render_json(verdict: str, checked: int, lost: list[dict]) -> str:
    return json.dumps(
        {"verdict": verdict, "checked": checked, "lost": lost}, ensure_ascii=False
    )


def render_text(verdict: str, checked: int, lost: list[dict]) -> str:
    lines = [f"{verdict} lost: {checked} blocks checked, {len(lost)} lost"]
    for entry in lost[:MAX_DETAIL_LINES]:
        lines.append(
            f"  {entry['ratio']:.2f}  {entry['path']}:{entry['line']}  {entry['text']}"
        )
    if len(lost) > MAX_DETAIL_LINES:
        lines.append(f"… and {len(lost) - MAX_DETAIL_LINES} more")
    return "\n".join(lines)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="lost.py",
        description="Check that a doc trim did not lose blocks (A0-16, L8).",
    )
    parser.add_argument("--base", required=True, metavar="REF")
    parser.add_argument("--head", metavar="REF", default=None)
    parser.add_argument(
        "--from",
        dest="from_paths",
        action="append",
        required=True,
        metavar="PATH",
    )
    parser.add_argument("--to", dest="to_paths", action="append", metavar="PATH")
    parser.add_argument("--min-chars", type=int, default=25, metavar="N")
    parser.add_argument("--threshold", type=float, default=0.5, metavar="F")
    parser.add_argument("--json", action="store_true")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    for label, ref in (("base", args.base), ("head", args.head)):
        if ref is not None and not valid_ref(ref):
            sys.stderr.write(f"lost.py: bad ref {ref!r} ({label})\n")
            return 2

    to_paths = args.to_paths or args.from_paths
    checked, lost = collect(
        args.base,
        args.head,
        args.from_paths,
        to_paths,
        args.min_chars,
        args.threshold,
    )
    verdict = "FAIL" if lost else "PASS"
    if args.json:
        print(render_json(verdict, checked, lost))
    else:
        print(render_text(verdict, checked, lost))
    return 1 if lost else 0


if __name__ == "__main__":
    raise SystemExit(main())
