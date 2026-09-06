#!/usr/bin/env python3
"""Determinism lint for the CRPG simulation crates.

Scans every .rs file under crates/crpg-core/, crates/crpg-rules/ and
crates/crpg-sim/ and fails on patterns that would make the deterministic
simulation non-deterministic:

All three crates:
  - HashMap / HashSet iteration (use IndexMap or BTreeMap)
  - SystemTime / Instant / std::time:: (wall clock is not sim time)
  - std::thread
  - rand:: / thread_rng (use crpg_core's seeded RNG)

crpg-core and crpg-rules only:
  - f32 / f64 (rules maths uses integers or fixed-point)

crpg-sim only:
  - f64 (E006-A: spatial positions are f32 per spec 2.4, outside the rules
    path; f64 has no legitimate sim use. Unsuffixed literals stay allowed in
    sim — they infer to f32 at f32-typed sites. Everything a rule reads is
    Fx16_16 or an integer, enforced one layer down in crpg-rules.)

What counts as code:

  - **Doctests are code.** A fenced block inside a `///` or `//!` comment is
    compiled and executed by `cargo test`, and `crpg-core/AGENTS.md` says the
    bans hold "anywhere, including tests". Its lines are scanned. Fences tagged
    `text` or `ignore` are not compiled, so they are not scanned.
  - **Prose is not code.** Ordinary `//` lines, doc-comment prose outside a
    fence, and `/* ... */` block comments (nesting included) are skipped, so a
    comment that merely names a banned type is not a violation.

Block comments are tracked by a deliberately simple scanner, biased toward
over-reporting because this lint's dangerous failure mode is silence:

  - It stops looking for `/*` on a line once it meets a string literal, so a
    `/*` inside a string never opens a comment. The cost is that a real block
    comment sharing a line with a string literal is still scanned, which
    over-reports; the escape hatch covers it.
  - An unterminated block comment at end of file is reported, rather than
    silently swallowing the rest of the file.

Lines that are comments are skipped. Any line ending in
  // determinism-ok: <reason>
is skipped, provided <reason> is non-empty.

Usage: python tools/lint/determinism.py [ROOT]
ROOT defaults to the repo root; an explicit ROOT is used by the self-tests.
Exit 0 if clean, 1 otherwise.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
CRATES = ["crpg-core", "crpg-rules", "crpg-sim"]

# (crate name, rule name, regex)
RULES = [
    ("both", "no-hashmap", re.compile(r"\bHashMap\b")),
    ("both", "no-hashset", re.compile(r"\bHashSet\b")),
    ("both", "no-wallclock", re.compile(r"\b(SystemTime|Instant)\b|std::time::")),
    ("both", "no-thread", re.compile(r"std::thread")),
    ("both", "no-external-rng", re.compile(r"\brand::|thread_rng")),
    # `\bf(?:32|64)\b` missed every suffixed float literal: in `1.5f64` the
    # character before `f` is a digit, so there is no word boundary and the
    # type annotation was the only form the ban caught. A lookbehind for a
    # letter instead of a word boundary catches `1.5f64`, `1.0_f64` and
    # `to_f32`, while still leaving an identifier like `buf32` alone.
    ("no-float-crates", "no-float", re.compile(r"(?<![A-Za-z])f(?:32|64)\b")),
    # E006-A: `f64` is banned in crpg-sim too (`f32`-spatial only). A dedicated
    # scope so the f32 positions spec 2.4 requires keep passing.
    ("no-f64-crates", "no-f64", re.compile(r"(?<![A-Za-z])f64\b")),
    # Unsuffixed float literals: `1.5`, `0.5`, `1e3`, `1.0e-2`, `.5`.
    # Requires a digit after `.` so `1..` (range) and `x.foo()` don't match.
    # Each alternative requires a non-identifier character before it so a
    # leading-dot literal `.0` in a tuple field access like `self.0` is not
    # mistaken for a float. The leading-dot alternative also rejects a
    # preceding `.` so a range with a multi-digit bound (`0..120`) does not
    # surface its tail (`.120`) as a float.
    ("no-float-crates", "no-float", re.compile(
        r"(?:"
        r"(?<![A-Za-z0-9_])[0-9]+\.[0-9]+(?:[eE][+-]?[0-9]+)?"
        r"|(?<![A-Za-z0-9_])[0-9]+[eE][+-]?[0-9]+"
        r"|(?<![A-Za-z0-9_])(?<!\.)\.[0-9]+(?:[eE][+-]?[0-9]+)?"
        r")\b"
    )),
]

ESCAPE_RE = re.compile(r"// determinism-ok:\s*(\S.*)?\s*$")
COMMENT_RE = re.compile(r"^\s*//")
# `///` and `//!` are doc comments; `////` is an ordinary comment.
DOC_RE = re.compile(r"^(\s*)(///(?!/)|//!)(.*)$")
# Fence info strings that rustdoc does not compile.
UNCOMPILED_FENCE_TAGS = {"text", "ignore"}


def strip_block_comments(lines):
    """Blank out `/* ... */` spans, returning (scannable_lines, unterminated).

    Comment characters are replaced with spaces rather than removed, so column
    positions and any trailing `// determinism-ok:` survive intact. Nesting is
    honoured, as Rust allows it.
    """
    out = []
    depth = 0
    for raw in lines:
        chars = list(raw)
        i = 0
        while i < len(chars):
            if depth == 0:
                if chars[i] == '"' or raw.startswith("//", i):
                    # A string literal or a line comment: nothing after this on
                    # the line can open a block comment in a way worth chasing.
                    break
                if raw.startswith("/*", i):
                    depth = 1
                    chars[i] = chars[i + 1] = " "
                    i += 2
                    continue
                i += 1
            else:
                if raw.startswith("*/", i):
                    depth -= 1
                    chars[i] = chars[i + 1] = " "
                    i += 2
                    continue
                if raw.startswith("/*", i):
                    depth += 1
                    chars[i] = chars[i + 1] = " "
                    i += 2
                    continue
                chars[i] = " "
                i += 1
        if depth > 0:
            # The remainder of the line is inside a comment.
            chars[i:] = " " * (len(chars) - i)
        out.append("".join(chars))
    return out, depth > 0


def mask_string_literals(line):
    """Replace the contents of string and char literals with spaces.

    Preserves column positions so that trailing ``// determinism-ok:`` markers
    and banned-token searches still work at the right offsets.  This prevents
    a banned token inside a string from being flagged (strings are not code)
    and prevents a ``// determinism-ok:`` marker inside a string from
    suppressing violations on the same line.
    """
    chars = list(line)
    i = 0
    n = len(chars)
    while i < n:
        c = chars[i]
        if c == '"':
            # Regular string literal.  Mask the content between quotes.
            i += 1
            while i < n:
                if chars[i] == '\\':
                    # Skip the escaped character – do not let \" end the
                    # string, and do not let the escaped char be masked.
                    chars[i] = ' '
                    i += 1
                    if i < n:
                        chars[i] = ' '
                        i += 1
                elif chars[i] == '"':
                    i += 1
                    break
                else:
                    chars[i] = ' '
                    i += 1
        elif c == "'":
            # Character literal: 'x', '\n', '\'', etc.
            i += 1
            while i < n:
                if chars[i] == '\\':
                    chars[i] = ' '
                    i += 1
                    if i < n:
                        chars[i] = ' '
                        i += 1
                elif chars[i] == "'":
                    i += 1
                    break
                else:
                    chars[i] = ' '
                    i += 1
        else:
            i += 1
    return ''.join(chars)


def fence_is_compiled(info: str) -> bool:
    """True if rustdoc compiles a fenced doc block with this info string."""
    tags = {t.strip().lower() for t in info.replace("`", "").split(",")}
    return not (tags & UNCOMPILED_FENCE_TAGS)


def scan_targets(text):
    """Yield (lineno, scannable_text) for every line that is really code.

    Doc-comment prose and ordinary comments are dropped; the body of a compiled
    doctest fence is kept, with the `///` marker stripped so the content is
    scanned as the code it becomes.
    """
    lines = text.splitlines()
    scannable, unterminated = strip_block_comments(lines)
    in_fence = False
    fence_compiled = False
    for lineno, line in enumerate(scannable, 1):
        doc = DOC_RE.match(line)
        if doc:
            indent, marker, content = doc.groups()
            stripped = content.strip()
            if stripped.startswith("```"):
                if in_fence:
                    in_fence = False
                else:
                    in_fence = True
                    fence_compiled = fence_is_compiled(stripped.lstrip("`"))
                continue
            if in_fence and fence_compiled:
                # Scan the doctest body as code, keeping the column alignment.
                yield lineno, " " * (len(indent) + len(marker)) + content
            continue
        if in_fence:
            # A fence that a non-doc line interrupts is malformed; stop
            # treating following lines as doctest body.
            in_fence = False
        if not line.strip() or COMMENT_RE.match(line):
            continue
        yield lineno, line
    if unterminated:
        yield len(lines), "\x00unterminated"


def check_file(crate_name, path):
    found = []
    text = path.read_text(encoding="utf-8-sig")
    for lineno, line in scan_targets(text):
        if line == "\x00unterminated":
            found.append((lineno, "unterminated-block-comment", line))
            continue
        # Strings and char literals are not code: mask their contents so a
        # banned token inside a string is not flagged and a `// determinism-ok:`
        # marker inside a string cannot suppress rules on this line.
        masked = mask_string_literals(line)
        # Skip escapes, requiring a non-empty reason.
        m = ESCAPE_RE.search(masked)
        if m:
            if not (m.group(1) or "").strip():
                found.append((lineno, "escape-no-reason", masked))
            continue
        applies = ("both",)
        if crate_name in ("crpg-core", "crpg-rules"):
            applies = ("both", "no-float-crates")
        elif crate_name == "crpg-sim":
            applies = ("both", "no-f64-crates")
        for scope, rule_name, pattern in RULES:
            if scope not in applies:
                continue
            if pattern.search(masked):
                found.append((lineno, rule_name, masked))
    return found


def main(args):
    root = pathlib.Path(args[0]) if args else ROOT
    violations = []
    for crate in CRATES:
        base = root / "crates" / crate
        if not base.is_dir():
            continue
        for path in sorted(base.rglob("*.rs")):
            for lineno, rule_name, line in check_file(crate, path):
                violations.append((path, lineno, rule_name, line))

    for path, lineno, rule_name, line in violations:
        print(f"VIOLATION {path}:{lineno} {rule_name} {line.strip()}")
    return 1 if violations else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
