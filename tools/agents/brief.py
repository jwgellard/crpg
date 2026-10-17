#!/usr/bin/env python3
"""Brief skeleton generator (A0-5) for the agent-team loop.

Turns ``tasks/<ID>.md`` into the `docs/agents/TEMPLATES.md` §1 skeleton. It
selects the task's *current contract* -- the last Exact contract/Re-contract
section plus every Contract amendment after it, or the base sections when there
is none -- and points at Behaviour and Test by line range, copying only the
Interface byte-exact. Stdlib only; no network.

Usage:
    python tools/agents/brief.py ID [--out PATH] [--json]

Exit 0 on success, 2 on a usage error or a missing task file (`no-task`).
--json prints one object instead of the brief, describing every `##` section.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import recall  # noqa: E402  (sibling module, same checkout)

HEADING_RE = re.compile(r"^(#{1,6})[ \t]+(.*?)[ \t]*$")
DATE_RE = re.compile(r"\s*[—–]\s*\d{4}-\d{2}-\d{2}\s*\(UTC\).*$", re.IGNORECASE)
# A numbered or lettered prefix: `2. `, `B§14. `, `C§8.7 `.
PREFIX_RE = re.compile(r"^(?:[a-z]?§?[0-9]+(?:\.[0-9]+)*[.)]?\s+)+")
TITLE_ID_RE_TEMPLATE = r"^{id}\b[ \t]*[—–-][ \t]*(.*)$"
CRATE_RE = re.compile(r"crpg-[a-z0-9]+(?:-[a-z0-9]+)*")

BASE_SECTIONS = (
    "task",
    "crate(s)",
    "purpose",
    "interface",
    "behaviour",
    "constraints",
    "test",
    "definition of done",
    "out of scope",
    "read first",
)

EXACT_PREFIXES = ("exact contract", "re-contract")
AMENDMENT_PREFIXES = ("contract amendment", "amendment")

# Decision-shaped base sections (A0-3b). Deliberately narrow: after prefix
# stripping, a heading is contract only when its normalised name is exactly
# `decision`, or starts with one of these tails. `Decisions — 2026-10-05 (UTC)`,
# `Decisions (D19–D26)` and `Amendment decisions …` stay history.
DECISION_PREFIXES = ("decisions this contract makes", "what must not change")

ACCEPT_PREFIXES = ("cargo ", "python ", "bash ", "pwsh ")

STOP_TEXT = (
    "the contract is ambiguous · a second crate is needed · a dependency is missing ·\n"
    "a golden would change · a test would have to be weakened · `unsafe` outside\n"
    "crpg-godot · an edit to crpg-contracts or rust-toolchain.toml."
)
REPORT_TEXT = "Reply with the report format in docs/agents/TEMPLATES.md §2, at most 40 lines."

FIXED_SECTIONS = [
    ("Goal", "<MANAGER: goal>"),
    ("Files you own", "<MANAGER: files you own>"),
    ("Example to follow (follow-on tasks)", "<MANAGER: example to follow>"),
    ("Out of scope", "<MANAGER: out of scope>"),
    ("Size budget", "<MANAGER: size budget>"),
]


# --------------------------------------------------------------------------
# classification


def normalize_heading(text: str) -> str:
    """Lowercase a heading, dropping a trailing `— <date> (UTC)` and a colon tail."""
    norm = text.strip()
    norm = DATE_RE.sub("", norm)
    norm = norm.split(":", 1)[0]
    return norm.strip().lower()


def _strip_prefix(norm: str) -> str:
    """Drop a leading `2. `, `B§14. ` or `C§8.7 ` prefix from a normalised name."""
    return PREFIX_RE.sub("", norm).strip()


def _matches_base(name: str, base: str) -> bool:
    """True when `name` is `base`, or starts with it as a whole word.

    The rest of the name must be empty or start with a non-alphanumeric
    delimiter (` `, `(`, `—`, `:`, `/`, …), so `Tasks` is not the `Task`
    base section but `Behaviour / suites` (tasks/T018c.md:37) is the
    `Behaviour` one. `Test` alone also accepts a plain plural: `Tests`,
    `Tests (…)`.
    """
    if not name.startswith(base):
        return False
    rest = name[len(base):]
    if rest == "" or not rest[0].isalnum():
        return True
    return base == "test" and rest.startswith("s")


def _starts_with_base(name: str) -> bool:
    return any(_matches_base(name, base) for base in BASE_SECTIONS)


def _decision_name(text: str) -> str:
    """A decision-relevant normalised name that keeps any date/parenthetical
    suffix, so dated `Decisions — 2026-…` headings stay distinguishable from
    the bare `Decisions` base section (which is contract)."""
    norm = text.strip()
    norm = norm.split(":", 1)[0]
    norm = PREFIX_RE.sub("", norm).strip()
    return norm.lower()


def _is_decision_base(name: str) -> bool:
    """True for the decision-shaped base sections (`tasks/T097.md:44`, `:63`).

    `name` is the date-stripped normalised name; the bare-word check uses the
    raw heading text passed separately, so this helper is only a fallback.
    """
    return name == "decision" or name.startswith(DECISION_PREFIXES)


def classify_heading(text: str) -> str:
    """`contract` or `history` for one `##` heading, per the A0-5 rules."""
    norm = _strip_prefix(normalize_heading(text))
    if norm.startswith("amendment decisions"):
        return "history"
    if norm.startswith(EXACT_PREFIXES) or norm.startswith("contract amendment"):
        return "contract"
    if norm.startswith("amendment"):
        return "contract"
    if _starts_with_base(norm):
        return "contract"
    decision = _decision_name(text)
    if decision == "decision" or decision == "decisions" or decision.startswith(DECISION_PREFIXES):
        return "contract"
    return "history"


def is_exact(text: str) -> bool:
    return _strip_prefix(normalize_heading(text)).startswith(EXACT_PREFIXES)


def is_amendment(text: str) -> bool:
    norm = _strip_prefix(normalize_heading(text))
    if norm.startswith("amendment decisions"):
        return False
    return norm.startswith(AMENDMENT_PREFIXES)


def select_current(sections: list[dict]) -> list[dict]:
    """The current contract: the last Exact/Re-contract plus later amendments."""
    exact = [index for index, section in enumerate(sections) if is_exact(section["text"])]
    if exact:
        start = exact[-1]
        current = [sections[start]]
        current += [s for s in sections[start + 1 :] if is_amendment(s["text"])]
        return current
    return [s for s in sections if classify_heading(s["text"]) == "contract"]


# --------------------------------------------------------------------------
# structure


def parse(text: str) -> tuple[list[dict], list[dict], int]:
    """Return (all headings, level-2 sections, total line count) with offsets."""
    raw_lines = text.split("\n")
    offsets = [0]
    for raw in raw_lines:
        offsets.append(offsets[-1] + len(raw) + 1)
    total_lines = len(raw_lines) - 1 if raw_lines and raw_lines[-1] == "" else len(raw_lines)

    headings: list[dict] = []
    inside_fence = False
    for index, raw in enumerate(raw_lines):
        line = raw.rstrip("\r")
        if line.strip().startswith("```") or line.strip().startswith("~~~"):
            inside_fence = not inside_fence
            continue
        if inside_fence:
            continue
        match = HEADING_RE.match(line)
        if not match:
            continue
        headings.append(
            {
                "level": len(match.group(1)),
                "text": match.group(2).strip(),
                "line": index + 1,
                "start": offsets[index],
                "body": min(offsets[index] + len(raw) + 1, len(text)),
                "end": len(text),
            }
        )
    for first in range(len(headings)):
        end = len(text)
        end_line = total_lines
        for later in range(first + 1, len(headings)):
            if headings[later]["level"] <= headings[first]["level"]:
                end = headings[later]["start"]
                end_line = headings[later]["line"] - 1
                break
        headings[first]["end"] = end
        headings[first]["end_line"] = end_line

    sections = [heading for heading in headings if heading["level"] == 2]
    return headings, sections, total_lines


def extract_title(text: str, task_id: str) -> str:
    pattern = re.compile(TITLE_ID_RE_TEMPLATE.format(id=re.escape(task_id)), re.IGNORECASE)
    inside_fence = False
    for raw in text.split("\n"):
        line = raw.rstrip("\r")
        if line.strip().startswith("```") or line.strip().startswith("~~~"):
            inside_fence = not inside_fence
            continue
        if inside_fence:
            continue
        match = HEADING_RE.match(line)
        if not match:
            continue
        heading = match.group(2).strip()
        titled = pattern.match(heading)
        return titled.group(1).strip() if titled else heading
    return task_id


def extract_crates(text: str, sections: list[dict]) -> list[str]:
    names: list[str] = []
    for section in sections:
        name = _strip_prefix(normalize_heading(section["text"]))
        if not (name.startswith("task") or name.startswith("crate(s)")):
            continue
        for crate in CRATE_RE.findall(text[section["body"] : section["end"]]):
            if crate not in names:
                names.append(crate)
    return names


def _section_interface(text: str, headings: list[dict], section: dict) -> tuple[dict, str] | None:
    if "interface" in normalize_heading(section["text"]):
        return section, text[section["body"] : section["end"]]
    for heading in headings:
        if heading["level"] in (3, 4) and section["start"] <= heading["start"] < section["end"]:
            if "interface" in normalize_heading(heading["text"]):
                return heading, text[heading["body"] : heading["end"]]
    return None


def build_interface(text: str, headings: list[dict], current: list[dict]) -> str:
    found_index = None
    content = None
    for index, section in enumerate(current):
        found = _section_interface(text, headings, section)
        if found is not None:
            found_index, content = index, found[1]
            break
    if content is None:
        return "<MANAGER: no Interface section found>"
    parts = [content]
    for section in current[found_index + 1 :]:
        # Only an amendment may revise the Interface; a later base section
        # (e.g. `## Behaviour … (V Interface)`) must not be appended.
        if not is_amendment(section["text"]):
            continue
        found = _section_interface(text, headings, section)
        if found is None:
            continue
        if not parts[-1].endswith("\n"):
            parts.append("\n")
        parts.append(f"From {section['text']}:\n")
        parts.append(found[1])
    return "".join(parts)


def _is_test_or_dod(text: str) -> bool:
    name = _strip_prefix(normalize_heading(text))
    return _matches_base(name, "test") or _matches_base(name, "definition of done")


def _scope_headings(section: dict, headings: list[dict]) -> list[dict]:
    """`section` followed by every heading nested inside it, in document order."""
    return [section] + [
        heading
        for heading in headings
        if heading["level"] > section["level"]
        and section["start"] <= heading["start"] < section["end"]
    ]


def acceptance_headings(section: dict, headings: list[dict]) -> list[dict]:
    """The subsections of `section` whose fenced blocks carry its commands.

    A `Test`/`Tests` or `Definition of done` subsection counts at any level
    and with any numbered/lettered prefix, and so does every heading nested
    inside one, because the commands sit in the deepest fence:
    `#### C§8.7 Commands` under `### C§8. Tests (`tasks/T023c.md`).
    """
    levels: list[int] = []
    found: list[dict] = []
    for heading in _scope_headings(section, headings):
        while levels and heading["level"] <= levels[-1]:
            levels.pop()
        if _is_test_or_dod(heading["text"]):
            levels.append(heading["level"])
        if levels:
            found.append(heading)
    return found


def _fenced_lines(body: str) -> list[str]:
    inside = False
    lines: list[str] = []
    for raw in body.split("\n"):
        line = raw.rstrip("\r")
        stripped = line.strip()
        if stripped.startswith("```") or stripped.startswith("~~~"):
            inside = not inside
            continue
        if inside:
            lines.append(stripped)
    return lines


def extract_acceptance(text: str, headings: list[dict], current: list[dict], crate: str | None) -> str:
    commands: list[str] = []
    for section in current:
        for heading in acceptance_headings(section, headings):
            for line in _fenced_lines(text[heading["body"] : heading["end"]]):
                if line.startswith(ACCEPT_PREFIXES) and line not in commands:
                    commands.append(line)
    if not commands:
        commands = ["<MANAGER: acceptance commands>"]
    target = crate if crate else "<crate>"
    commands.append(
        f"bash tools/preflight.sh --crate {target}   "
        f"(Windows: tools/preflight.ps1 -Crate {target})"
    )
    body = "\n".join(f"- {command}" for command in commands)
    return (
        body
        + "\nPassing the tests that exist is not done. Done is the contract's "
        "Behaviour, with a\ntest for each numbered item."
    )


def section_reference(task_id: str, section: dict) -> str:
    return f'tasks/{task_id}.md §"{section["text"]}" (lines {section["line"]}–{section["end_line"]})'


# --------------------------------------------------------------------------
# build


def build(task_id: str, text: str, root: str) -> tuple[str, dict]:
    headings, sections, _ = parse(text)
    current = select_current(sections)
    current_lines = {section["line"] for section in current}
    crates = extract_crates(text, sections)
    title = extract_title(text, task_id)
    crate = crates[0] if len(crates) == 1 else None

    route = (
        f"Route: <MANAGER: role> on <MANAGER: model> · Slot <MANAGER> · "
        f"Branch task/{task_id}-<MANAGER: slug> · Attempt 1"
    )
    references = " and ".join(section_reference(task_id, section) for section in current)
    contract = f"Contract: {references}. The contract wins over this brief."

    read_first = ["- AGENTS.md"]
    read_first += [f"- crates/{name}/AGENTS.md" for name in crates]
    read_first += [f"- {section_reference(task_id, section)}" for section in current]

    interface = build_interface(text, headings, current)

    # Title words, deduplicated case-insensitively: `Zebra zebra` must score
    # like `zebra` in recall, not double it.
    title_words: list[str] = []
    seen: set[str] = set()
    for word in re.findall(r"[0-9A-Za-z_]+", title):
        key = word.lower()
        if key in seen:
            continue
        seen.add(key)
        title_words.append(word)
    related_lines = [recall.format_hit(hit) for hit in recall.search(title_words, root)[:10]]
    related = "\n".join(related_lines) if related_lines else "None found."

    acceptance = extract_acceptance(text, headings, current, crate)

    blocks: list[str] = [
        f"# Brief — {task_id} · {title}",
        route,
        contract,
        "",
        "## Goal",
        FIXED_SECTIONS[0][1],
        "",
        "## Files you own",
        FIXED_SECTIONS[1][1],
        "",
        "## Read first, and nothing else unless you must",
        "\n".join(read_first),
        "",
        "## Interface",
        interface.rstrip("\n") if interface.endswith("\n") else interface,
        "",
        "## Example to follow (follow-on tasks)",
        FIXED_SECTIONS[2][1],
        "",
        "## Related memory",
        related,
        "",
        "## Acceptance — your stopping condition",
        acceptance,
        "",
        "## Out of scope",
        FIXED_SECTIONS[3][1],
        "",
        "## Stop and report instead of working around if",
        STOP_TEXT,
        "",
        "## Size budget",
        FIXED_SECTIONS[4][1],
        "",
        "## Commit",
        f"`git add` your files and commit once as `{task_id}: <summary>`. Never push, merge,\n"
        "rebase or switch branches.",
        "",
        "## Report",
        REPORT_TEXT,
    ]
    brief = "\n".join(blocks) + "\n"

    document = {
        "tool": "brief",
        "version": 1,
        "task": task_id,
        "sections": [
            {
                "heading": section["text"],
                "start": section["line"],
                "end": section["end_line"],
                "kind": classify_heading(section["text"]),
                "current": section["line"] in current_lines,
            }
            for section in sections
        ],
        "bytes_in": len(text.encode("utf-8")),
        "bytes_out": len(brief.encode("utf-8")),
    }
    return brief, document


# --------------------------------------------------------------------------
# driver


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="brief.py",
        description="Write the TEMPLATES §1 brief skeleton for a task; exit 0 or 2.",
    )
    parser.add_argument("task", metavar="ID", help="the task ID, e.g. T023b")
    parser.add_argument("--out", metavar="PATH", help="write the brief to PATH instead of stdout")
    parser.add_argument("--json", action="store_true", help="print one JSON object")
    return parser


def main(argv: list[str] | None = None) -> int:
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
    args = build_parser().parse_args(argv)
    task_id = args.task
    try:
        root = recall.find_repo_root()
    except recall.UsageError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2

    path = os.path.join(root, "tasks", task_id + ".md")
    if not os.path.isfile(path):
        if args.json:
            print(
                json.dumps(
                    {
                        "tool": "brief",
                        "version": 1,
                        "task": task_id,
                        "error": "no-task",
                        "sections": [],
                        "bytes_in": 0,
                        "bytes_out": 0,
                    }
                )
            )
        else:
            print("no-task")
        return 2

    with open(path, encoding="utf-8", newline="") as handle:
        text = handle.read()

    try:
        brief, document = build(task_id, text, root)
    except recall.UsageError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2

    if args.out:
        with open(args.out, "w", encoding="utf-8", newline="") as handle:
            handle.write(brief)
    if args.json:
        print(json.dumps(document))
    elif not args.out:
        sys.stdout.write(brief)
    return 0


if __name__ == "__main__":
    sys.exit(main())
