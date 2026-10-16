#!/usr/bin/env python3
"""Repository memory search (A0-5) for the agent-team loop.

Searches the repository's memory -- lessons, `E` records, ADRs, decision
batches, task files and git-log subjects -- for the words given on the command
line, ranking whole-word hits by source weight, then path, then line. Stdlib
only; no network.

Usage:
    python tools/agents/recall.py WORD [WORD ...] [--limit N] [--json] [--full]

Output starts with `FOUND <n> hits for <words>` or `NONE for <words>`, then up
to --limit hit lines. The text report is capped at 40 lines unless --full;
--json prints one object with every hit. Exit 0 on success, 2 on a usage error
or outside a git checkout.
"""

from __future__ import annotations

import argparse
import glob
import json
import os
import re
import subprocess
import sys

DEFAULT_LIMIT = 15
TOTAL_CAP = 40
TRIM = 120

# glob pattern (relative to the repo root) -> weight. Order is the search
# order used only to keep the tie-break stable; ranking sorts afterwards.
SOURCES = [
    ("tasks/E*.md", 3),
    ("docs/adr/*.md", 2),
    ("tasks/DECISIONS-*.md", 2),
    ("tasks/T*.md", 1),
]


class UsageError(Exception):
    """A condition that makes the tool exit 2 with a diagnostic."""


def run_git(root: str, args: list[str]) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(
            ["git", "-C", root, *args],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
    except OSError as error:  # pragma: no cover - git is a prerequisite
        raise UsageError(f"cannot run git: {error}") from error


def find_repo_root(start: str | None = None) -> str:
    """The git toplevel of the checkout that holds this script."""
    here = start or os.path.dirname(os.path.abspath(__file__))
    result = run_git(here, ["rev-parse", "--show-toplevel"])
    if result.returncode != 0 or not result.stdout.strip():
        raise UsageError("not inside a git checkout")
    return result.stdout.strip()


def word_pattern(word: str) -> re.Pattern[str]:
    """Case-insensitive whole-word match: ``net`` never matches ``network``."""
    return re.compile(
        r"(?<![0-9A-Za-z_])" + re.escape(word) + r"(?![0-9A-Za-z_])",
        re.IGNORECASE,
    )


def source_files(root: str) -> list[tuple[str, int]]:
    """Every (absolute path, weight) to search, sorted for determinism."""
    entries: list[tuple[str, int]] = []
    lessons = os.path.join(root, "docs", "agents", "LESSONS.md")
    if os.path.isfile(lessons):
        entries.append((lessons, 3))
    for pattern, weight in SOURCES:
        for path in glob.glob(os.path.join(root, *pattern.split("/"))):
            entries.append((path, weight))
    return sorted(entries, key=lambda item: item[0])


def dedupe_words(words: list[str]) -> list[str]:
    """Drop empty words and case-insensitive repeats, keeping the first spelling."""
    seen: set[str] = set()
    unique: list[str] = []
    for word in words:
        key = word.lower()
        if not key or key in seen:
            continue
        seen.add(key)
        unique.append(word)
    return unique


def search(words: list[str], root: str) -> list[dict]:
    """All hits for ``words`` under ``root``, ranked and deterministic."""
    patterns = [word_pattern(word) for word in dedupe_words(words)]
    hits: list[dict] = []
    for path, weight in source_files(root):
        relative = os.path.relpath(path, root).replace("\\", "/")
        try:
            with open(path, encoding="utf-8", errors="replace") as handle:
                text = handle.read()
        except OSError:  # pragma: no cover - a race, not a normal path
            continue
        heading = ""
        for number, raw in enumerate(text.splitlines(), start=1):
            if raw.startswith("## "):
                heading = raw[3:].strip()
            matched = sum(1 for pattern in patterns if pattern.search(raw))
            if matched:
                hits.append(
                    {
                        "kind": "file",
                        "path": relative,
                        "line": number,
                        "heading": heading,
                        "text": raw.strip(),
                        "weight": weight,
                        "score": weight * matched,
                    }
                )

    log = run_git(root, ["log", "--format=%h %s"])
    if log.returncode == 0:
        for index, raw in enumerate(log.stdout.splitlines()):
            # Match the subject only: the abbreviated hash must never be a hit.
            digest, _, subject = raw.partition(" ")
            matched = sum(1 for pattern in patterns if pattern.search(subject))
            if matched:
                hits.append(
                    {
                        "kind": "git",
                        "path": "git",
                        "line": index,
                        "heading": "",
                        "text": raw.strip(),
                        "weight": 1,
                        "score": matched,
                        "hash": digest,
                        "subject": subject,
                    }
                )

    # Score descending, then path, then line; git hits carry a stable index.
    hits.sort(key=lambda hit: (-hit["score"], hit["path"], hit["line"]))
    return hits


def trim(text: str) -> str:
    text = text.strip()
    return text if len(text) <= TRIM else text[:TRIM]


def format_hit(hit: dict) -> str:
    if hit["kind"] == "git":
        return f"git {hit['hash']} · {trim(hit['subject'])}"
    return f"{hit['path']}:{hit['line']} · {hit['heading']} · {trim(hit['text'])}"


def words_label(words: list[str]) -> str:
    return " ".join(words)


def render_text(words: list[str], hits: list[dict], limit: int, full: bool) -> str:
    if hits:
        lines = [f"FOUND {len(hits)} hits for {words_label(words)}"]
    else:
        lines = [f"NONE for {words_label(words)}"]
    for hit in hits[: max(0, limit)]:
        lines.append(format_hit(hit))
    if not full and len(lines) > TOTAL_CAP:
        lines = lines[:TOTAL_CAP]
    return "\n".join(lines) + "\n"


def render_json(words: list[str], hits: list[dict]) -> str:
    document = {
        "tool": "recall",
        "version": 1,
        "words": list(words),
        "count": len(hits),
        "hits": [dict(hit) for hit in hits],
    }
    return json.dumps(document)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="recall.py",
        description="Search repository memory for words; exit 0, or 2 on a usage error.",
    )
    parser.add_argument("words", metavar="WORD", nargs="+", help="whole words to find")
    parser.add_argument("--limit", type=int, default=DEFAULT_LIMIT, help="hit lines to print")
    parser.add_argument("--json", action="store_true", help="print one JSON object")
    parser.add_argument("--full", action="store_true", help="lift the 40-line cap")
    return parser


def main(argv: list[str] | None = None) -> int:
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
    args = build_parser().parse_args(argv)
    words = dedupe_words(args.words)
    try:
        root = find_repo_root()
        hits = search(words, root)
    except UsageError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2
    if args.json:
        print(render_json(words, hits))
    else:
        sys.stdout.write(render_text(words, hits, args.limit, args.full))
    return 0


if __name__ == "__main__":
    sys.exit(main())
