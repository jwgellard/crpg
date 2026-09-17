#!/usr/bin/env python3
"""Dependency-direction lint: enforces the layering rule in AGENTS.md.

Checks four things:

  - **Layering.** Every workspace edge must appear in the allowed-edges table
    below. A crate missing from the table is itself a violation, so adding a
    crate is a deliberate decision rather than a silent exemption.
  - **Cycles.** Over runtime `[dependencies]` only. A dev-dependency cycle is
    legal in cargo and does not break the build; an upward dev edge is still
    caught by the layering check.
  - **godot.** Only `crpg-godot` may depend on it, in any section.
  - **unsafe.** Only `crpg-godot` may use it. Every other crate root must carry
    `#![forbid(unsafe_code)]`.

`[dependencies]`, `[dev-dependencies]` and `[build-dependencies]` are all
scanned for the layering and godot rules: a test-only import is still an
import, and `crpg-core/AGENTS.md` says "no workspace crate, ever".

Two things a manifest can do that a naive reader misses, and this one does not:

  - **Target-specific tables.** A `[target.<cfg>.dependencies]` block is a
    dependency table like any other. Every `[target.*]` block is walked, and a
    violation reports the table it came from.
  - **Renamed dependencies.** An entry whose value carries `package = "godot"`
    depends on godot whatever its table key says. The `package` field wins over
    the key, so a rename cannot launder an edge past the layering or godot
    rules.

Requires Python 3.11+ (`tomllib`).
"""

import sys
import tomllib
from pathlib import Path

# Allowed edges: from_crate -> set of allowed to-crates, or None/Every.
# "Every" means any workspace crate is allowed.
# Excluded crates are stored separately.
ALLOWED: dict[str, tuple[set[str] | None, set[str]]] = {
    "crpg-core":      (set(),   set()),
    "crpg-data":      ({"crpg-core"}, set()),
    "crpg-rules":     ({"crpg-core", "crpg-data"}, set()),
    "crpg-sim":       ({"crpg-core", "crpg-data", "crpg-rules"}, set()),
    "crpg-nav":       ({"crpg-core"}, set()),
    "crpg-script":    ({"crpg-core", "crpg-data", "crpg-rules", "crpg-sim"}, set()),
    "crpg-ai":        ({"crpg-core", "crpg-rules", "crpg-sim", "crpg-nav"}, set()),
    "crpg-net":       ({"crpg-core", "crpg-data", "crpg-sim"}, set()),
    "crpg-persist":   ({"crpg-core", "crpg-data", "crpg-sim"}, set()),
    "crpg-edit":      ({"crpg-core", "crpg-data", "crpg-rules"}, set()),
    "crpg-contracts": ({"crpg-core"}, set()),
    # Test utilities may reach any simulation crate, but not the Godot bridge:
    # other crates take crpg-testkit as a dev-dependency, and a testkit that
    # pulled crpg-godot would pull the engine into every one of them. The
    # direct-edge checks below cannot see that, so the exclusion is the thing
    # enforcing it.
    "crpg-testkit":   (None, {"crpg-godot"}),
    "crpg-server":    (None, {"crpg-godot", "crpg-edit"}),
    "crpg-cli":       (None, {"crpg-godot"}),
    "crpg-godot":     (None, set()),
}

# The one crate allowed unsafe and the godot dependency (root AGENTS.md).
GODOT_CRATE = "crpg-godot"

# Sections of a Cargo.toml that create an import. A dev-dependency is still an
# import: it is how test code reaches another crate.
DEP_SECTIONS = ("dependencies", "dev-dependencies", "build-dependencies")

FORBID_UNSAFE = "#![forbid(unsafe_code)]"


def discover_crates(crates_dir: Path) -> dict[str, Path]:
    """Return {crate_name: Cargo.toml_path} for every crate in the workspace."""
    crates: dict[str, Path] = {}
    for cargo in sorted(crates_dir.glob("*/Cargo.toml")):
        with open(cargo, "rb") as f:
            data = tomllib.load(f)
        crates[data["package"]["name"]] = cargo
    return crates


def dep_tables(data: dict) -> list[tuple[str, str, dict]]:
    """Return every dependency table in a manifest.

    Each entry is `(label, section, table)`, where `section` is one of
    `DEP_SECTIONS` and `label` is what a violation message shows — the plain
    section name for a top-level table, and a qualified name such as
    `target.<cfg>.dependencies` for a target-specific one, so the reader can
    find the table the violation came from.
    """
    tables: list[tuple[str, str, dict]] = []
    for section in DEP_SECTIONS:
        table = data.get(section)
        if isinstance(table, dict):
            tables.append((section, section, table))
    targets = data.get("target")
    if isinstance(targets, dict):
        for cfg, cfg_table in sorted(targets.items()):
            if not isinstance(cfg_table, dict):
                continue
            for section in DEP_SECTIONS:
                table = cfg_table.get(section)
                if isinstance(table, dict):
                    tables.append((f"target.{cfg}.{section}", section, table))
    return tables


def real_name(key: str, value) -> str:
    """The crate a dependency entry actually names.

    An entry with `package = "godot"` depends on godot, not on whatever the
    table key calls it. The `package` field is the crate; the key is only the
    name it is imported under.
    """
    if isinstance(value, dict):
        renamed = value.get("package")
        if isinstance(renamed, str):
            return renamed
    return key


Graph = dict[str, dict[str, tuple[str, set[str]]]]


def build_graph(crates_dir: Path) -> tuple[Graph, Graph]:
    """Build internal and external dependency graphs, keyed by crate then label.

    Returns ({crate: {label: (section, {workspace_dep, ...})}},
             {crate: {label: (section, {external_dep, ...})}}).
    """
    known = discover_crates(crates_dir)
    internal: Graph = {name: {} for name in known}
    external: Graph = {name: {} for name in known}
    for name, path in known.items():
        with open(path, "rb") as f:
            data = tomllib.load(f)
        for label, section, table in dep_tables(data):
            internal[name].setdefault(label, (section, set()))
            external[name].setdefault(label, (section, set()))
            for key, value in table.items():
                dep = real_name(key, value)
                bucket = internal if dep in known else external
                bucket[name][label][1].add(dep)
    return internal, external


def runtime_edges(internal: Graph) -> dict[str, set[str]]:
    """Collapse to the runtime-only graph, for cycle detection.

    Target-specific runtime tables count: a cycle that only exists on one
    platform is still a cycle.
    """
    return {
        src: {
            dep
            for section, deps in labels.values()
            if section == "dependencies"
            for dep in deps
        }
        for src, labels in internal.items()
    }


def check_allowed(internal: Graph) -> list[str]:
    """Return violation lines for edges not in the allowed-edges table.

    Fails closed: a crate absent from ALLOWED is a violation in itself, so a
    newly added crate cannot inherit blanket permission by omission.
    """
    violations = []
    for src, labels in sorted(internal.items()):
        if src not in ALLOWED:
            violations.append(f"VIOLATION {src} (not in the allowed-edges table)")
            continue
        allowed, excluded = ALLOWED[src]
        for label, (_section, deps) in sorted(labels.items()):
            for tgt in sorted(deps):
                bad = tgt not in allowed if allowed is not None else tgt in excluded
                if bad:
                    violations.append(
                        f"VIOLATION {src} -> {tgt} (allowed-edges, {label})"
                    )
    return violations


def check_cycles(internal: dict[str, set[str]]) -> list[str]:
    """Return violation lines for dependency cycles."""
    WHITE, GRAY, BLACK = 0, 1, 2
    color = {n: WHITE for n in internal}
    violations: list[str] = []
    path: list[str] = []

    def dfs(node: str) -> None:
        color[node] = GRAY
        path.append(node)
        for nb in internal.get(node, []):
            if color[nb] == GRAY:
                cycle = path[path.index(nb):] + [nb]
                violations.append(
                    f"VIOLATION {' -> '.join(cycle)} (cycle)"
                )
            elif color[nb] == WHITE:
                dfs(nb)
        path.pop()
        color[node] = BLACK

    for node in sorted(internal):
        if color[node] == WHITE:
            dfs(node)
    return violations


def check_godot(external: Graph) -> list[str]:
    """Return violation lines if a non-crpg-godot crate depends on godot.

    Every table counts, target-specific ones included: a dev-dependency or a
    platform-gated dependency on godot would still pull the engine into a crate
    that is supposed to build without it.
    """
    violations = []
    for src, labels in sorted(external.items()):
        if src == GODOT_CRATE:
            continue
        for label, (_section, deps) in sorted(labels.items()):
            if "godot" in deps:
                violations.append(f"VIOLATION {src} -> godot (godot-only, {label})")
    return violations


def check_unsafe(crates_dir: Path) -> list[str]:
    """Return violation lines for a crate root missing the forbid attribute.

    Root AGENTS.md: only `crpg-godot` may use unsafe. The attribute is what
    actually enforces that, so its absence is the thing to catch — a crate
    without it can grow an unsafe block with nothing to stop it.

    `src/bin/*.rs` are crate roots too, and each needs its own attribute: an
    inner attribute in `lib.rs` says nothing about a second binary target.
    """
    violations = []
    for name, cargo in sorted(discover_crates(crates_dir).items()):
        if name == GODOT_CRATE:
            continue
        src = cargo.parent / "src"
        roots = [p for p in (src / "lib.rs", src / "main.rs") if p.is_file()]
        roots += sorted(p for p in src.glob("bin/*.rs") if p.is_file())
        if not roots:
            violations.append(f"VIOLATION {name} (no crate root under src/)")
            continue
        for root in roots:
            text = root.read_text(encoding="utf-8-sig")
            if FORBID_UNSAFE not in text:
                violations.append(
                    f"VIOLATION {name}:{root.name} (missing {FORBID_UNSAFE})"
                )
    return violations


def main() -> int:
    crates_dir = Path(__file__).resolve().parents[2] / "crates"
    if not crates_dir.is_dir():
        print(f"ERROR: {crates_dir} not found", file=sys.stderr)
        return 2
    internal, external = build_graph(crates_dir)
    violations = (
        check_godot(external)
        + check_cycles(runtime_edges(internal))
        + check_allowed(internal)
        + check_unsafe(crates_dir)
    )
    for v in violations:
        print(v)
    return 1 if violations else 0


if __name__ == "__main__":
    raise SystemExit(main())
