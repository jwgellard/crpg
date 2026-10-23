#!/usr/bin/env python3
"""Worker-result verifier (A0-4) for the agent-team loop.

Checks one worker's branch in a slot against its brief and the repository's
rules, prints a verdict-first result of at most 40 lines, and exits 0 (pass),
1 (a check failed) or 2 (usage or infrastructure). It never writes to the
slot, the index or any ref: every git invocation is read-only.

Usage:
    python tools/agents/verify_task.py ID --slot PATH [--brief PATH]
        [--base REF] [--skip NAME[,NAME...]] [--json] [--full]

Options:
    --slot PATH    the worktree to inspect (required)
    --brief PATH   the brief to parse; default <slot>/BRIEF.md, or
                   .agents/briefs/<ID>.md when the slot's brief names
                   another task
    --base REF     the changed-set base; default the merge-base of HEAD and
                   origin/master in the slot
    --skip NAMES   comma-separated checks to report as skipped
    --json         print exactly one JSON object instead of the text report
    --full         lift the 12-line per-check and 40-line total caps

Checks, in order: scope, gates, protected, goldens, weakening, unsafe, lints,
docsig, todo. The test suite overrides the gate and lint commands through
CRPG_PREFLIGHT and CRPG_LINTS and pins the date through CRPG_TODAY; none of
these are needed in normal use.
"""

from __future__ import annotations

import argparse
import datetime
import json
import os
import re
import shlex
import subprocess
import sys
import time

CHECKS = [
    "scope",
    "gates",
    "protected",
    "goldens",
    "weakening",
    "unsafe",
    "lints",
    "docsig",
    "todo",
]

TOTAL = len(CHECKS)

# Built once per path pattern; glob translation is below.
_GLOB_CACHE: dict[str, re.Pattern[str]] = {}

_TEST_MARKERS = re.compile(r"#\[test\]|#\[tokio::test\]|proptest!|#\[case|#\[test_case")
_ASSERT_MARKERS = re.compile(r"\bassert\w*")
_IGNORE_MARKERS = re.compile(r"#\[ignore")
_UNSAFE = re.compile(r"\bunsafe\b")
_TODO = re.compile(r"todo!\(|unimplemented!\(")


class UsageError(Exception):
    """A condition that makes the tool exit 2 with a diagnostic."""


class InfraError(Exception):
    """A gate or lint command that could not be started exits 2 too."""


# --------------------------------------------------------------------------
# globs


def glob_match(pattern: str, path: str) -> bool:
    """Match a repo-relative path: ``**`` any depth, ``*`` one segment."""
    rx = _GLOB_CACHE.get(pattern)
    if rx is None:
        out: list[str] = []
        i = 0
        while i < len(pattern):
            char = pattern[i]
            if char == "*":
                if i + 1 < len(pattern) and pattern[i + 1] == "*":
                    j = i + 2
                    if j < len(pattern) and pattern[j] == "/":
                        out.append("(?:.*/)?")
                        i = j + 1
                    else:
                        out.append(".*")
                        i = j
                else:
                    out.append("[^/]*")
                    i += 1
            elif char == "?":
                out.append("[^/]")
                i += 1
            else:
                out.append(re.escape(char))
                i += 1
        rx = re.compile("^" + "".join(out) + "$")
        _GLOB_CACHE[pattern] = rx
    return rx.match(path) is not None


def any_glob(patterns: list[str], path: str) -> bool:
    return any(glob_match(p, path) for p in patterns)


# --------------------------------------------------------------------------
# brief


def _looks_like_path(token: str) -> bool:
    """A backticked token that reads as a path or glob, not prose or code."""
    return bool(token) and any(char in token for char in "/*?.")


def parse_brief(text: str) -> tuple[list[str], list[str], list[str]]:
    """Return (owned, protected grants, goldens grants) from a brief.

    A ``## Files you own`` bullet owns every backticked token in it that looks
    like a path or glob, so a bullet may list several paths. A bullet with no
    backticks keeps the first-token rule, cut at the first ``: `` or end.
    """
    owned: list[str] = []
    protected: list[str] = []
    goldens: list[str] = []
    section: str | None = None
    for raw in text.splitlines():
        line = raw.strip()
        if line.startswith("## "):
            heading = line[3:].strip().lower()
            if heading.startswith("files you own"):
                section = "own"
            elif heading.startswith("permissions"):
                section = "perm"
            else:
                section = None
            continue
        if section is None or not line.startswith("- "):
            continue
        content = line[2:].strip()
        if not content:
            continue
        if section == "own":
            spans = re.findall(r"`([^`]*)`", content)
            if spans:
                for span in spans:
                    token = span.strip()
                    cut = re.search(r":(?=\s|$)", token)
                    if cut:
                        token = token[: cut.start()]
                    if token and not any(ch.isspace() for ch in token) and _looks_like_path(token):
                        owned.append(token)
            else:
                token = content.split()[0].replace("`", "")
                cut = re.search(r":(?=\s|$)", token)
                if cut:
                    token = token[: cut.start()]
                if token:
                    owned.append(token)
        else:
            grant = re.match(r"(protected|goldens)\s*:\s*(\S+)", content)
            if grant:
                path = grant.group(2).replace("`", "")
                (protected if grant.group(1) == "protected" else goldens).append(path)
    return owned, protected, goldens


def owned_crates(owned: list[str], slot: str) -> list[str]:
    """Each concrete ``crates/<name>/`` an owned glob covers."""
    names: set[str] = set()
    for pattern in owned:
        parts = pattern.split("/")
        if len(parts) < 2 or parts[0] != "crates":
            continue
        second = parts[1]
        if "*" in second or "?" in second:
            base = os.path.join(slot, "crates")
            if os.path.isdir(base):
                for entry in os.listdir(base):
                    if os.path.isdir(os.path.join(base, entry)) and glob_match(
                        "crates/" + second, "crates/" + entry
                    ):
                        names.add(entry)
        else:
            names.add(second)
    return sorted(names)


# --------------------------------------------------------------------------
# git


def run_git(slot: str, args: list[str]) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(
            ["git", "-C", slot, *args],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
    except OSError as error:  # pragma: no cover - git is a prerequisite
        raise UsageError(f"cannot run git: {error}") from error


def is_worktree(slot: str) -> bool:
    result = run_git(slot, ["rev-parse", "--is-inside-work-tree"])
    return result.returncode == 0 and result.stdout.strip() == "true"


def resolve_base(slot: str, given: str | None) -> str:
    if given:
        result = run_git(slot, ["rev-parse", "--verify", "--quiet", f"{given}^{{commit}}"])
        if result.returncode != 0:
            raise UsageError(f"unknown base ref: {given}")
        return given
    result = run_git(slot, ["merge-base", "HEAD", "origin/master"])
    if result.returncode != 0 or not result.stdout.strip():
        raise UsageError("cannot resolve base: git merge-base HEAD origin/master failed")
    return result.stdout.strip()


def _numstat(slot: str, args: list[str]) -> list[tuple[int, int, str]]:
    result = run_git(slot, ["diff", "--no-renames", "--numstat", *args])
    rows: list[tuple[int, int, str]] = []
    for line in result.stdout.splitlines():
        parts = line.split("\t")
        if len(parts) < 3:
            continue
        added = int(parts[0]) if parts[0] != "-" else 0
        deleted = int(parts[1]) if parts[1] != "-" else 0
        rows.append((added, deleted, "\t".join(parts[2:])))
    return rows


def commit_range_base(slot: str, base: str) -> str:
    """The tree to diff HEAD against for the ``base..HEAD`` commit range.

    ``base..HEAD`` is the commits reachable from HEAD and not base; when base
    has moved ahead of the branch, that set is empty, so diffing ``base HEAD``
    directly would report base's own commits as changes. The merge-base is the
    fork point and yields exactly the branch's own commits.
    """
    result = run_git(slot, ["merge-base", base, "HEAD"])
    if result.returncode == 0 and result.stdout.strip():
        return result.stdout.strip()
    return base


def collect_changes(slot: str, range_base: str) -> dict[str, dict]:
    """Committed base..HEAD plus staged, unstaged and untracked files."""
    statuses: dict[str, set[str]] = {}
    for args in (
        ["diff", "--no-renames", "--name-status", "-z", range_base, "HEAD"],
        ["diff", "--no-renames", "--name-status", "-z", "--cached"],
        ["diff", "--no-renames", "--name-status", "-z"],
    ):
        result = run_git(slot, args)
        _absorb_name_status(result.stdout, statuses)
    untracked = run_git(slot, ["ls-files", "--others", "--exclude-standard", "-z"])
    for path in untracked.stdout.split("\0"):
        if path:
            statuses.setdefault(path, set()).add("?")

    files: dict[str, dict] = {}
    for path, codes in statuses.items():
        if path == "BRIEF.md":
            continue
        if "D" in codes:
            status = "D"
        elif "A" in codes:
            status = "A"
        elif "M" in codes:
            status = "M"
        else:
            status = "?"
        files[path] = {"path": path, "status": status, "added": 0, "deleted": 0}

    for args in (
        ["diff", "--no-renames", "--numstat", range_base, "HEAD"],
        ["diff", "--no-renames", "--numstat", "--cached"],
        ["diff", "--no-renames", "--numstat"],
    ):
        for added, deleted, path in _numstat(slot, args):
            if path in files:
                files[path]["added"] += added
                files[path]["deleted"] += deleted

    for path, info in files.items():
        if info["status"] != "?":
            continue
        try:
            text = open(os.path.join(slot, path), encoding="utf-8", errors="replace").read()
        except OSError:
            text = ""
        info["added"] = len(text.splitlines())
    return files


def _absorb_name_status(stdout: str, statuses: dict[str, set[str]]) -> None:
    tokens = stdout.split("\0")
    i = 0
    while i + 1 < len(tokens):
        status = tokens[i]
        if not status:
            i += 1
            continue
        statuses.setdefault(tokens[i + 1], set()).add(status[:1])
        i += 2


# --------------------------------------------------------------------------
# context


class Context:
    def __init__(
        self, slot: str, base: str, range_base: str, brief: str, owned, protected, goldens, files
    ):
        self.slot = slot
        self.base = base
        self.range_base = range_base
        self.brief = brief
        self.owned = owned
        self.protected = protected
        self.goldens = goldens
        self.files = files
        self._added: dict[str, list[str]] = {}
        self._base_text: dict[str, str | None] = {}
        self._new_text: dict[str, str | None] = {}

    def added_lines(self, path: str) -> list[str]:
        if path in self._added:
            return self._added[path]
        lines: list[str] = []
        for where in ([self.range_base, "HEAD"], ["--cached"], []):
            result = run_git(
                self.slot, ["diff", "--no-renames", "-U0", *where, "--", path]
            )
            for line in result.stdout.splitlines():
                if line.startswith("+") and not line.startswith("+++"):
                    lines.append(line[1:])
        if self.files.get(path, {}).get("status") == "?":
            try:
                text = open(
                    os.path.join(self.slot, path), encoding="utf-8", errors="replace"
                ).read()
            except OSError:
                text = ""
            lines.extend(text.splitlines())
        self._added[path] = lines
        return lines

    def base_text(self, path: str) -> str | None:
        if path not in self._base_text:
            result = run_git(self.slot, ["show", f"{self.range_base}:{path}"])
            self._base_text[path] = (
                result.stdout if result.returncode == 0 else None
            )
        return self._base_text[path]

    def new_text(self, path: str) -> str | None:
        if path not in self._new_text:
            full = os.path.join(self.slot, path)
            try:
                self._new_text[path] = open(
                    full, encoding="utf-8", errors="replace"
                ).read()
            except OSError:
                self._new_text[path] = None
        return self._new_text[path]


def _result(status: str, detail: str = "", output: list[str] | None = None) -> dict:
    return {"status": status, "detail": detail, "output": output or []}


def _ok() -> dict:
    return _result("ok")


def _fail(detail: str, output: list[str] | None = None) -> dict:
    return _result("failed", detail, output)


def _split_override(value: str) -> list[str]:
    """Split a CRPG_PREFLIGHT/CRPG_LINTS override; a bad quote is exit 2."""
    try:
        return shlex.split(value)
    except ValueError as error:
        raise InfraError(f"cannot parse command {value!r}: {error}") from error


def _run(cmd: list[str], cwd: str) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(
            cmd,
            cwd=cwd,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
    except OSError as error:
        raise InfraError(f"cannot start command {shlex.join(cmd)}: {error}") from error


def _command_lines(proc: subprocess.CompletedProcess[str]) -> list[str]:
    return (proc.stdout + proc.stderr).splitlines()


# --------------------------------------------------------------------------
# checks


def check_scope(ctx: Context) -> dict:
    offenders = [
        path for path in sorted(ctx.files) if not any_glob(ctx.owned, path)
    ]
    if offenders:
        return _fail(
            f"{len(offenders)} changed file(s) outside the brief's owns list",
            [f"unowned: {path}" for path in offenders],
        )
    return _ok()


def check_gates(ctx: Context) -> dict:
    override = os.environ.get("CRPG_PREFLIGHT", "").strip()
    crates = owned_crates(ctx.owned, ctx.slot)
    if override:
        commands = [_split_override(override)]
        labels = [override]
    elif not crates:
        commands = [
            [sys.executable, "-m", "unittest", "discover", "-s", "tools/tests", "-p", "test_*.py"],
            [sys.executable, "-m", "unittest", "discover", "-s", "tools/lint", "-p", "test_*.py"],
        ]
        labels = ["unittest tools/tests", "unittest tools/lint"]
    else:
        commands = [preflight_command(crates)]
        labels = ["preflight"]
    for cmd, label in zip(commands, labels):
        proc = _run(cmd, ctx.slot)
        if proc.returncode != 0:
            return _fail(
                f"{label} exited {proc.returncode}", _command_lines(proc)
            )
    return _ok()


def preflight_command(crates: list[str]) -> list[str]:
    windows = os.name == "nt"
    if windows:
        cmd = ["pwsh", "-NoProfile", "-File", "tools/preflight.ps1"]
        if len(crates) == 1:
            cmd += ["-Crate", crates[0]]
        return cmd + ["-CheckOnly"]
    cmd = ["bash", "tools/preflight.sh"]
    if len(crates) == 1:
        cmd += ["--crate", crates[0]]
    return cmd + ["--check-only"]


def check_protected(ctx: Context) -> dict:
    offenders = [
        path
        for path in sorted(ctx.files)
        if is_protected(path) and not any_glob(ctx.protected, path)
    ]
    if offenders:
        return _fail(
            "protected file(s) changed without a protected: grant",
            [f"protected: {path}" for path in offenders],
        )
    return _ok()


def is_protected(path: str) -> bool:
    if path in ("Cargo.lock", "deny.toml", "Cargo.toml", "rust-toolchain.toml"):
        return True
    if path == "crates/crpg-contracts" or path.startswith("crates/crpg-contracts/"):
        return True
    if path == ".github/workflows" or path.startswith(".github/workflows/"):
        return True
    return False


def check_goldens(ctx: Context) -> dict:
    offenders = [
        path
        for path in sorted(ctx.files)
        if is_golden(path) and not any_glob(ctx.goldens, path)
    ]
    if offenders:
        return _fail(
            "golden or replay file(s) changed without a goldens: grant",
            [f"golden: {path}" for path in offenders],
        )
    return _ok()


def is_golden(path: str) -> bool:
    if path.endswith(".golden") or path.endswith(".replay"):
        return True
    return "/goldens/" in "/" + path


def check_weakening(ctx: Context) -> dict:
    offenders: list[str] = []
    for path in sorted(ctx.files):
        info = ctx.files[path]
        if info["status"] not in ("M", "D"):
            continue
        base = ctx.base_text(path)
        if base is None:
            continue
        if not is_test_file(path, base):
            continue
        if info["status"] == "D" or ctx.new_text(path) is None:
            offenders.append(f"{path}: deleted")
            continue
        new = ctx.new_text(path) or ""
        before = counts(base)
        after = counts(new)
        if (
            after["test"] < before["test"]
            or after["assert"] < before["assert"]
            or after["ignore"] > before["ignore"]
        ):
            offenders.append(
                f"{path}: #[test] {before['test']}->{after['test']}, "
                f"assert {before['assert']}->{after['assert']}, "
                f"#[ignore] {before['ignore']}->{after['ignore']}"
            )
    if offenders:
        return _fail(f"{len(offenders)} test file(s) weakened", offenders)
    return _ok()


def is_test_file(path: str, base_text: str) -> bool:
    if path.endswith(".rs"):
        if re.match(r"^crates/[^/]+/tests/", path):
            return True
        return "#[test]" in base_text or "#[cfg(test)]" in base_text
    return os.path.basename(path).startswith("test_") and path.endswith(".py")


def counts(text: str) -> dict[str, int]:
    return {
        "test": len(_TEST_MARKERS.findall(text)),
        "assert": len(_ASSERT_MARKERS.findall(text)),
        "ignore": len(_IGNORE_MARKERS.findall(text)),
    }


def check_unsafe(ctx: Context) -> dict:
    offenders: list[str] = []
    for path in sorted(ctx.files):
        if not path.endswith(".rs"):
            continue
        if path == "crates/crpg-godot" or path.startswith("crates/crpg-godot/"):
            continue
        for line in ctx.added_lines(path):
            if _UNSAFE.search(line):
                offenders.append(f"{path}: {line.strip()}")
    if offenders:
        return _fail(f"{len(offenders)} new unsafe line(s) outside crpg-godot", offenders)
    return _ok()


def check_lints(ctx: Context) -> dict:
    override = os.environ.get("CRPG_LINTS", "").strip()
    if override:
        commands = [_split_override(override)]
        labels = [override]
    else:
        commands = [
            [sys.executable, "tools/lint/deps.py"],
            [sys.executable, "tools/lint/determinism.py"],
        ]
        labels = ["tools/lint/deps.py", "tools/lint/determinism.py"]
    for cmd, label in zip(commands, labels):
        proc = _run(cmd, ctx.slot)
        if proc.returncode != 0:
            return _fail(f"{label} exited {proc.returncode}", _command_lines(proc))
    return _ok()


def check_docsig(ctx: Context) -> dict:
    today = os.environ.get("CRPG_TODAY") or datetime.datetime.now(
        datetime.timezone.utc
    ).strftime("%Y-%m-%d")
    # An entry written when the work began is still right after a UTC
    # midnight: accept dates from the oldest commit in base..HEAD to today.
    # With no commits in the range, only today.
    first = today
    stamps = run_git(ctx.slot, ["log", "--format=%ct", f"{ctx.range_base}..HEAD"])
    commit_days = [
        datetime.datetime.fromtimestamp(int(s), datetime.timezone.utc).strftime("%Y-%m-%d")
        for s in stamps.stdout.split()
        if s.isdigit()
    ]
    if stamps.returncode == 0 and commit_days:
        first = min(min(commit_days), today)
    wanted = today if first == today else f"{first}..{today}"
    signature = re.compile(r"^- (\d{4}-\d{2}-\d{2}) \(UTC\) · ")

    def signed(line: str) -> bool:
        found = signature.match(line)
        return bool(found) and first <= found.group(1) <= today

    offenders: list[str] = []
    for path in sorted(ctx.files):
        if not path.endswith(".md") or ctx.files[path]["status"] == "D":
            continue
        if path in ("BRIEF.md",) or path.startswith(".claude/agents/"):
            continue
        if path.startswith(".opencode/agents/") or "fixtures" in path.split("/"):
            continue
        if not any(signed(line) for line in ctx.added_lines(path)):
            offenders.append(f"{path}: no added agent-log entry dated {wanted} (UTC)")
    if offenders:
        return _fail(f"{len(offenders)} .md file(s) without a signature dated {wanted}", offenders)
    return _ok()


def check_todo(ctx: Context) -> dict:
    offenders: list[str] = []
    for path in sorted(ctx.files):
        if not path.endswith(".rs") or not any_glob(ctx.owned, path):
            continue
        # Only crate sources count: a bake-off fixture's `todo!()` stubs are
        # its point, so everything outside crates/ is not checked.
        if not path.startswith("crates/"):
            continue
        if "/src/" not in "/" + path:
            continue
        if os.path.basename(path) == "tests.rs" or "tests" in path.split("/"):
            continue
        for line in ctx.added_lines(path):
            if _TODO.search(line):
                offenders.append(f"{path}: {line.strip()}")
    if offenders:
        return _fail(f"{len(offenders)} floating todo!()/unimplemented!()", offenders)
    return _ok()


CHECK_FUNCTIONS = {
    "scope": check_scope,
    "gates": check_gates,
    "protected": check_protected,
    "goldens": check_goldens,
    "weakening": check_weakening,
    "unsafe": check_unsafe,
    "lints": check_lints,
    "docsig": check_docsig,
    "todo": check_todo,
}


# --------------------------------------------------------------------------
# driver


def parse_skip(values: list[str]) -> set[str]:
    names: set[str] = set()
    for value in values:
        for name in value.split(","):
            name = name.strip()
            if not name:
                continue
            if name not in CHECKS:
                raise UsageError(f"unknown --skip name: {name}")
            names.add(name)
    return names


_BRIEF_HEADER = re.compile(r"^# (?:Review b|B)rief — (\S+) ·")


def default_brief_path(slot: str, task: str) -> str:
    """The brief to use without --brief: the slot's, unless it is another task's.

    A slot is reused, so its BRIEF.md may be for a different task than the
    one being verified. Only a first line naming a *different* task ID sends
    us to `.agents/briefs/<ID>.md` (in $CRPG_REPO, else this script's
    repository); a header with no ID, or this ID, keeps the slot's brief.
    """
    slot_brief = os.path.join(slot, "BRIEF.md")
    try:
        with open(slot_brief, encoding="utf-8", errors="replace") as handle:
            first = handle.readline()
    except OSError:
        return slot_brief
    match = _BRIEF_HEADER.match(first)
    if not match or match.group(1) == task:
        return slot_brief
    repo = os.environ.get("CRPG_REPO") or os.path.dirname(
        os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    )
    filed = os.path.join(repo, ".agents", "briefs", f"{task}.md")
    if os.path.isfile(filed):
        return filed
    raise UsageError(
        f"no-brief: {slot_brief} is the brief for {match.group(1)}, "
        f"and {filed} does not exist for {task}"
    )


def evaluate(args: argparse.Namespace) -> tuple[str, str | None, list[dict], dict[str, dict]]:
    skip = parse_skip(args.skip)
    slot = os.path.abspath(args.slot)
    if not os.path.isdir(slot):
        raise UsageError(f"slot is not a directory: {args.slot}")
    if not is_worktree(slot):
        raise UsageError(f"slot is not a git worktree: {args.slot}")

    brief_path = args.brief or default_brief_path(slot, args.task)
    if not os.path.isfile(brief_path):
        raise UsageError(f"brief not found: {brief_path}")
    try:
        brief_text = open(brief_path, encoding="utf-8", errors="replace").read()
    except OSError as error:
        raise UsageError(f"cannot read brief: {error}") from error
    owned, protected, goldens = parse_brief(brief_text)
    if not owned:
        raise UsageError(f"brief has no '## Files you own' bullets: {brief_path}")

    base = resolve_base(slot, args.base)
    range_base = commit_range_base(slot, base)
    files = collect_changes(slot, range_base)
    ctx = Context(slot, base, range_base, brief_text, owned, protected, goldens, files)

    checks: list[dict] = []
    for name in CHECKS:
        if name in skip:
            checks.append({"name": name, **_result("skipped")})
            continue
        result = CHECK_FUNCTIONS[name](ctx)
        checks.append({"name": name, **result})
    verdict = "FAIL" if any(c["status"] == "failed" for c in checks) else "PASS"
    return verdict, None, checks, files


def slot_name(slot: str) -> str:
    absolute = os.path.abspath(slot)
    return os.path.basename(absolute.rstrip("\\/")) or absolute


def render_json(
    task: str, verdict: str, error: str | None, checks: list[dict], files: dict
) -> str:
    document = {
        "tool": "verify_task",
        "version": 1,
        "task": task,
        "verdict": verdict,
        "error": error,
        "checks": [
            {
                "name": c["name"],
                "status": c["status"],
                "detail": c.get("detail", ""),
                "output": list(c.get("output", [])),
            }
            for c in checks
        ],
        "files": [
            {
                "path": info["path"],
                "added": info["added"],
                "deleted": info["deleted"],
                "status": info["status"],
            }
            for info in sorted(files.values(), key=lambda item: item["path"])
        ],
    }
    return json.dumps(document)


def render_text(
    task: str,
    verdict: str,
    error: str | None,
    checks: list[dict],
    name: str,
    elapsed: float,
    full: bool,
) -> str:
    failed = sum(1 for c in checks if c["status"] == "failed")
    lines = [f"{verdict} {task} verify · {name} · {failed}/{TOTAL} failed · {elapsed:.1f}s"]
    if error:
        lines.append(f"error: {error}")
    for check in checks:
        if check["status"] == "ok":
            lines.append(f"{check['name']}: ok")
        elif check["status"] == "skipped":
            lines.append(f"{check['name']}: skipped")
        else:
            lines.append(f"{check['name']}: FAILED ({check.get('detail', '')})")
    for check in checks:
        if check["status"] != "failed" or not check.get("output"):
            continue
        lines.append(f"--- {check['name']} ---")
        output = check["output"]
        if not full:
            output = output[:12]
        lines.extend(output)
    if not full and len(lines) > 40:
        lines = lines[:40]
    return "\n".join(lines) + "\n"


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="verify_task.py",
        description="Verify a worker's slot against its brief; exit 0 pass, 1 fail, 2 error.",
    )
    parser.add_argument("task", metavar="ID", help="the task ID being verified")
    parser.add_argument("--slot", required=True, help="the worktree to inspect")
    parser.add_argument("--brief", help="brief path; default <slot>/BRIEF.md")
    parser.add_argument("--base", help="changed-set base ref")
    parser.add_argument(
        "--skip",
        action="append",
        default=[],
        metavar="NAME[,NAME...]",
        help="checks to report as skipped",
    )
    parser.add_argument("--json", action="store_true", help="print one JSON object")
    parser.add_argument("--full", action="store_true", help="lift the output caps")
    return parser


def main(argv: list[str] | None = None) -> int:
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
    start = time.monotonic()
    args = build_parser().parse_args(argv)
    name = slot_name(args.slot)
    if args.json:
        try:
            verdict, _, checks, files = evaluate(args)
        except UsageError as error:
            print(render_json(args.task, "ERROR", str(error), [], {}))
            return 2
        except InfraError as error:
            print(render_json(args.task, "ERROR", str(error), [], {}))
            return 2
        print(render_json(args.task, verdict, None, checks, files))
        return 0 if verdict == "PASS" else 1

    checks: list[dict] = []
    files: dict[str, dict] = {}
    try:
        verdict, _, checks, files = evaluate(args)
    except UsageError as error:
        sys.stdout.write(
            render_text(args.task, "ERROR", str(error), checks, name, time.monotonic() - start, args.full)
        )
        return 2
    except InfraError as error:
        sys.stdout.write(
            render_text(args.task, "ERROR", str(error), checks, name, time.monotonic() - start, args.full)
        )
        return 2
    sys.stdout.write(
        render_text(args.task, verdict, None, checks, name, time.monotonic() - start, args.full)
    )
    return 0 if verdict == "PASS" else 1


if __name__ == "__main__":
    sys.exit(main())
