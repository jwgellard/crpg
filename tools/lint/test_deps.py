#!/usr/bin/env python3
"""Self-tests for tools/lint/deps.py using temporary fixture crate trees.

Run: python -m unittest discover -s tools/lint -p "test_*.py"
"""

import shutil
import tempfile
import textwrap
import unittest
from pathlib import Path

# Ensure the lint module is importable.
import importlib.util
SPEC = importlib.util.spec_from_file_location("deps", Path(__file__).with_name("deps.py"))
deps = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(deps)

FORBID = "#![forbid(unsafe_code)]\n//! doc\n"


def _entries(entries) -> str:
    """Render one dependency table body.

    An entry is either a bare crate name (a plain version requirement) or a
    `(key, package)` pair, which renders as a renamed dependency.
    """
    out = ""
    for entry in entries:
        if isinstance(entry, tuple):
            key, package = entry
            out += f'{key} = {{ package = "{package}", version = "0.1" }}\n'
        else:
            out += f'{entry} = "0.1"\n'
    return out


def _make_tree(tmp: Path, crates: dict) -> None:
    """Create a temporary crate tree.

    `crates` maps a crate name to either a list of runtime deps, or a dict with
    any of the keys `dependencies`, `dev-dependencies`, `build-dependencies`,
    `target` (a `{cfg: {section: [deps]}}` mapping), `root` (the text of
    src/lib.rs, defaulting to a compliant stub) and `bins` (a
    `{filename: text}` mapping written under src/bin/).

    A dependency entry may be a bare name or a `(key, package)` tuple, which
    renders as `key = { package = "..." }` — the rename form.
    """
    for name, spec in crates.items():
        if isinstance(spec, list):
            spec = {"dependencies": spec}
        crate_dir = tmp / name
        (crate_dir / "src").mkdir(parents=True, exist_ok=True)

        sections = ""
        for section in deps.DEP_SECTIONS:
            entries = spec.get(section, [])
            if not entries:
                continue
            sections += f"\n[{section}]\n" + _entries(entries)

        for cfg, cfg_sections in spec.get("target", {}).items():
            for section, entries in cfg_sections.items():
                if not entries:
                    continue
                sections += f"\n[target.{cfg}.{section}]\n" + _entries(entries)

        cargo = textwrap.dedent(f"""\
            [package]
            name = "{name}"
            version = "0.1.0"
            edition = "2021"
        """) + sections
        (crate_dir / "Cargo.toml").write_text(cargo, encoding="utf-8")
        (crate_dir / "src" / "lib.rs").write_text(
            spec.get("root", FORBID), encoding="utf-8"
        )

        for filename, text in spec.get("bins", {}).items():
            bin_dir = crate_dir / "src" / "bin"
            bin_dir.mkdir(parents=True, exist_ok=True)
            (bin_dir / filename).write_text(text, encoding="utf-8")


class TreeCase(unittest.TestCase):
    """Base class that builds a fixture tree and tears it down."""

    def tree(self, crates: dict) -> Path:
        tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, tmp, ignore_errors=True)
        _make_tree(tmp, crates)
        return tmp

    def all_violations(self, crates: dict) -> list:
        tmp = self.tree(crates)
        internal, external = deps.build_graph(tmp)
        return (
            deps.check_godot(external)
            + deps.check_cycles(deps.runtime_edges(internal))
            + deps.check_allowed(internal)
            + deps.check_unsafe(tmp)
        )


class TestCleanGraph(TreeCase):
    def test_clean(self):
        self.assertEqual(self.all_violations({
            "crpg-core": [],
            "crpg-data": ["crpg-core"],
            "crpg-rules": ["crpg-core", "crpg-data"],
        }), [])

    def test_external_dev_dependency_is_fine(self):
        # proptest/serde_json in crpg-core, as ADR-0006 authorises.
        self.assertEqual(self.all_violations({
            "crpg-core": {"dev-dependencies": ["proptest", "serde_json"]},
        }), [])

    def test_legal_target_specific_dependency_is_fine(self):
        """A platform-gated edge that the table allows is not a violation."""
        self.assertEqual(self.all_violations({
            "crpg-core": [],
            "crpg-data": {"target": {"'cfg(windows)'": {"dependencies": ["crpg-core"]}}},
        }), [])


class TestCycle(TreeCase):
    def test_cycle(self):
        violations = self.all_violations({
            "crpg-core": ["crpg-data"],
            "crpg-data": ["crpg-core"],
        })
        self.assertTrue(any("cycle" in v for v in violations))

    def test_target_specific_cycle(self):
        """A cycle that only exists on one platform is still a cycle."""
        violations = self.all_violations({
            "crpg-core": ["crpg-data"],
            "crpg-data": {"target": {"'cfg(windows)'": {"dependencies": ["crpg-core"]}}},
        })
        self.assertTrue(any("cycle" in v for v in violations), violations)


class TestUpwardEdge(TreeCase):
    def test_upward(self):
        violations = self.all_violations({
            "crpg-core": ["crpg-sim"],
            "crpg-data": [],
            "crpg-sim": [],
        })
        self.assertTrue(any("allowed-edges" in v for v in violations))

    def test_upward_dev_dependency(self):
        """A dev-dependency is still an import: crpg-core takes no workspace crate."""
        violations = self.all_violations({
            "crpg-core": {"dev-dependencies": ["crpg-testkit"]},
            "crpg-testkit": [],
        })
        self.assertTrue(
            any("allowed-edges, dev-dependencies" in v for v in violations),
            violations,
        )

    def test_upward_build_dependency(self):
        violations = self.all_violations({
            "crpg-core": {"build-dependencies": ["crpg-data"]},
            "crpg-data": [],
        })
        self.assertTrue(
            any("allowed-edges, build-dependencies" in v for v in violations),
            violations,
        )


class TestTargetSpecificTables(TreeCase):
    """A `[target.<cfg>.dependencies]` table is a dependency table.

    Reading only the three top-level sections let a platform-gated edge past
    every check, which is how an upward edge and a godot dependency could both
    sit in a manifest with the lint green.
    """

    def test_upward_target_dependency(self):
        violations = self.all_violations({
            "crpg-core": {"target": {"'cfg(windows)'": {"dependencies": ["crpg-sim"]}}},
            "crpg-sim": [],
        })
        self.assertTrue(
            any("allowed-edges, target.cfg(windows).dependencies" in v
                for v in violations),
            violations,
        )

    def test_upward_target_dev_dependency(self):
        violations = self.all_violations({
            "crpg-core": {"target": {"'cfg(unix)'": {"dev-dependencies": ["crpg-sim"]}}},
            "crpg-sim": [],
        })
        self.assertTrue(
            any("target.cfg(unix).dev-dependencies" in v for v in violations),
            violations,
        )

    def test_target_dependency_on_godot(self):
        violations = self.all_violations({
            "crpg-core": {"target": {"'cfg(windows)'": {"dependencies": ["godot"]}}},
        })
        self.assertTrue(
            any("godot-only, target.cfg(windows).dependencies" in v
                for v in violations),
            violations,
        )


class TestRenamedDependencies(TreeCase):
    """`package = "..."` names the crate; the table key is just a local alias.

    Keying off the alias meant a one-line rename hid any edge from both the
    layering and the godot rule.
    """

    def test_renamed_workspace_crate_is_still_an_edge(self):
        violations = self.all_violations({
            "crpg-core": {"dependencies": [("sim", "crpg-sim")]},
            "crpg-sim": [],
        })
        self.assertTrue(
            any("crpg-core -> crpg-sim" in v for v in violations), violations
        )

    def test_renamed_godot_is_still_godot(self):
        violations = self.all_violations({
            "crpg-core": {"dependencies": [("engine", "godot")]},
        })
        self.assertTrue(
            any("godot-only" in v for v in violations), violations
        )

    def test_renamed_dependency_in_a_target_table(self):
        """Both evasions at once, which is the shape that motivated the fix."""
        violations = self.all_violations({
            "crpg-core": {"target": {"'cfg(windows)'": {"dependencies": [("engine", "godot")]}}},
        })
        self.assertTrue(
            any("godot-only, target.cfg(windows).dependencies" in v
                for v in violations),
            violations,
        )

    def test_workspace_inherited_dependency_keeps_its_key(self):
        """`{ workspace = true }` has no `package`, so the key is the crate."""
        tmp = self.tree({"crpg-data": {"dependencies": ["crpg-core"]}, "crpg-core": []})
        (tmp / "crpg-data" / "Cargo.toml").write_text(
            '[package]\nname = "crpg-data"\nversion = "0.1.0"\nedition = "2021"\n'
            "\n[dependencies]\ncrpg-core = { workspace = true }\n",
            encoding="utf-8",
        )
        internal, _ = deps.build_graph(tmp)
        self.assertEqual(internal["crpg-data"]["dependencies"][1], {"crpg-core"})


class TestUnknownCrate(TreeCase):
    def test_crate_missing_from_the_table_fails_closed(self):
        """A new crate must be added to ALLOWED, not inherit blanket permission."""
        violations = self.all_violations({
            "crpg-core": [],
            "crpg-newthing": ["crpg-core"],
        })
        self.assertTrue(
            any("not in the allowed-edges table" in v for v in violations),
            violations,
        )


class TestGodotRule(TreeCase):
    def test_non_godot_depends_on_godot(self):
        violations = self.all_violations({"crpg-core": ["godot"]})
        self.assertTrue(any("godot-only" in v for v in violations))

    def test_non_godot_dev_depends_on_godot(self):
        violations = self.all_violations({
            "crpg-core": {"dev-dependencies": ["godot"]},
        })
        self.assertTrue(
            any("godot-only, dev-dependencies" in v for v in violations), violations
        )

    def test_godot_depends_on_godot_ok(self):
        # crpg-godot is also the one crate exempt from the forbid-unsafe rule.
        self.assertEqual(self.all_violations({
            "crpg-godot": {"dependencies": ["godot"], "root": "//! doc\n"},
        }), [])

    def test_testkit_may_not_depend_on_the_godot_bridge(self):
        """Crates dev-depend on crpg-testkit; it must not drag the engine in."""
        violations = self.all_violations({
            "crpg-testkit": ["crpg-godot"],
            "crpg-godot": {"root": "//! doc\n"},
        })
        self.assertTrue(
            any("crpg-testkit -> crpg-godot" in v for v in violations), violations
        )

    def test_testkit_may_depend_on_a_simulation_crate(self):
        self.assertEqual(self.all_violations({
            "crpg-testkit": ["crpg-sim"],
            "crpg-sim": [],
        }), [])


class TestUnsafeRule(TreeCase):
    def test_missing_forbid_attribute_fails(self):
        violations = self.all_violations({
            "crpg-core": {"root": "//! no forbid attribute here\n"},
        })
        self.assertTrue(any("forbid(unsafe_code)" in v for v in violations), violations)

    def test_bom_prefixed_root_still_counts(self):
        """Several stub roots carry a UTF-8 BOM; it must not read as missing."""
        self.assertEqual(self.all_violations({
            "crpg-core": {"root": "﻿" + FORBID},
        }), [])

    def test_crate_with_no_root_fails(self):
        tmp = self.tree({"crpg-core": []})
        (tmp / "crpg-core" / "src" / "lib.rs").unlink()
        violations = deps.check_unsafe(tmp)
        self.assertTrue(any("no crate root" in v for v in violations), violations)

    def test_extra_binary_target_needs_its_own_attribute(self):
        """`src/bin/*.rs` is a separate crate root; lib.rs does not cover it."""
        violations = self.all_violations({
            "crpg-cli": {"bins": {"helper.rs": "//! no forbid attribute\nfn main() {}\n"}},
        })
        self.assertTrue(
            any("helper.rs" in v and "forbid(unsafe_code)" in v for v in violations),
            violations,
        )

    def test_compliant_extra_binary_passes(self):
        self.assertEqual(self.all_violations({
            "crpg-cli": {"bins": {"helper.rs": FORBID + "fn main() {}\n"}},
        }), [])


if __name__ == "__main__":
    unittest.main()
