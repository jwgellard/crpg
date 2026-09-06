#!/usr/bin/env python3
"""Self-tests for the determinism lint.

Builds temporary fixture crate trees and asserts the lint produces the expected
violations. Run: python -m unittest discover -s tools/lint -p "test_*.py"
"""

import pathlib
import re
import subprocess
import sys
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
SCRIPT = HERE / "determinism.py"

CLEAN_SRC = """
pub fn apply(world: &mut World) {
    let v: Vec<i32> = vec![1, 2, 3];
    for x in &v {
        let _ = x;
    }
}
"""

HASHMAP_SRC = """
use std::collections::HashMap;
pub fn f() {
    let mut h = HashMap::new();
    h.insert(1, 2);
    for (k, _) in &h {
        let _ = k;
    }
}
"""

HASHSET_SRC = """
use std::collections::HashSet;
pub fn f() {
    let mut s = HashSet::new();
    s.insert(1);
}
"""

WALLCLOCK_SRC = """
use std::time::Instant;
pub fn f() {
    let start = Instant::now();
    let _ = start;
}
"""

THREAD_SRC = """
pub fn f() {
    std::thread::spawn(|| {});
}
"""

RNG_SRC = """
pub fn f() {
    let mut rng = rand::thread_rng();
    let _ = rng.gen::<u32>();
}
"""

COMMENT_SRC = """
// HashMap iteration is banned, but this is just a comment.
// rand::thread_rng is banned too.
pub fn f() { let x = 1; let _ = x; }
"""

ESCAPE_OK_SRC = """
pub fn f() {
    let mut h = std::collections::HashMap::new(); // determinism-ok: only used as a lookup, never iterated
    let _ = h;
}
"""

ESCAPE_NO_REASON_SRC = """
pub fn f() {
    let h = std::collections::HashMap::new(); // determinism-ok:
    let _ = h;
}
"""

FLOAT_SRC = """
pub fn ac() -> f64 { 10.5 }
pub fn hp() -> i32 { 20 }
"""

FLOAT_SIM_F32_SRC = """
pub struct Transform { x: f32, y: f32 }
"""

FLOAT_SIM_F64_SRC = """
pub struct Transform { x: f64, y: f64 }
"""

# A suffixed float literal. `\bf64\b` never matched this: the character before
# `f` is a digit, so there is no word boundary there.
FLOAT_LITERAL_SRC = """
pub fn weight() -> i32 {
    let scale = 1.5f64;
    let _ = scale;
    20
}
"""

FLOAT_UNDERSCORE_LITERAL_SRC = """
pub fn weight() -> i32 {
    let scale = 1.0_f32;
    let _ = scale;
    20
}
"""

# An unsuffixed float literal defaults to f64 and must be caught too.
UNSUFFIXED_FLOAT_SRC = """
pub fn weight() -> i32 {
    let x = 1.5;
    let _ = x;
    20
}
"""

EXPONENT_FLOAT_SRC = """
pub fn weight() -> i32 {
    let x = 1e3;
    let _ = x;
    20
}
"""

# An unsuffixed float inside a string literal is not code.
FLOAT_IN_STRING_SRC = """
pub fn f() {
    let s = "value is 1.5";
    let _ = s;
}
"""

# A range and a member access on a bare integer are not floats.
RANGE_AND_MEMBER_SRC = """
pub fn f() {
    let n = 1;
    for i in 1..10 {
        let _ = i;
    }
    let _ = n;
    x::foo().bar();
}
"""

RANGE_ONLY_SRC = """
pub fn f() {
    let n = 1;
    let _ = n;
    for i in 0..n {
        let _ = i;
    }
}
"""

PLAIN_INT_SRC = """
pub fn f() {
    let x = 9;
    let _ = x;
}
"""

INT_METHOD_SRC = """
pub fn f() {
    let x = 1u32.pow(2);
    let _ = x;
}
"""

HEX_INT_SRC = """
pub fn f() {
    let x = 0x1F;
    let _ = x;
}
"""

CHAR_LITERAL_SRC = """
pub fn f() {
    let c = 'a';
    let _ = c;
}
"""

# A `// determinism-ok:` marker inside a string literal must not let a real
# violation on the same line escape.
ESCAPE_INSIDE_STRING_SRC = """
pub fn f() {
    let s = "// determinism-ok: not a real escape";
    let _ = s;
    let mut h = std::collections::HashMap::new();
    let _ = h;
}
"""

# An identifier that merely ends in the banned token must not fire.
FLOAT_LOOKALIKE_SRC = """
pub fn f() {
    let buf32 = [0u8; 32];
    let _ = buf32;
}
"""

# A doctest is compiled and executed by `cargo test`. crpg-core/AGENTS.md says
# the bans hold "anywhere, including tests", so the fence body is code.
DOCTEST_SRC = """
/// Does a thing.
///
/// ```
/// use std::collections::HashMap;
/// let mut m = HashMap::new();
/// m.insert("a", 1.5f64);
/// ```
pub fn documented() {}
"""

# Prose inside a doc comment, outside any fence, is not code.
DOC_PROSE_SRC = """
/// Callers must not reach for a HashMap here, and f64 is banned outright.
///
/// See the module docs.
pub fn documented() {}
"""

# `text` and `ignore` fences are not compiled by rustdoc.
DOCTEST_TEXT_FENCE_SRC = """
/// An illustration, not code:
///
/// ```text
/// HashMap<StatId, f64>
/// ```
pub fn documented() {}
"""

DOCTEST_IGNORE_FENCE_SRC = """
/// ```ignore
/// let m: HashMap<u32, f64> = HashMap::new();
/// ```
pub fn documented() {}
"""

# An inner doc comment carries doctests too.
INNER_DOCTEST_SRC = """
//! Module docs.
//!
//! ```
//! let m = std::collections::HashMap::new();
//! ```
"""

BLOCK_COMMENT_SRC = """
/* HashMap and f64 are named here, but this is a comment. */
pub fn f() { let x = 1; let _ = x; }
"""

MULTILINE_BLOCK_COMMENT_SRC = """
/*
 * HashMap iteration is banned.
 * So is f64.
 */
pub fn f() { let x = 1; let _ = x; }
"""

NESTED_BLOCK_COMMENT_SRC = """
/* outer /* inner mentions HashMap */ still a comment, f64 */
pub fn f() { let x = 1; let _ = x; }
"""

# The block comment ends; code after it on the same line is still code.
BLOCK_COMMENT_THEN_CODE_SRC = """
pub fn f() {
    /* a note */ let m = std::collections::HashMap::new();
    let _ = m;
}
"""

# A `/*` inside a string literal must not open a comment and hide the rest of
# the file. This is the lint's dangerous direction, so it is pinned.
BLOCK_COMMENT_IN_STRING_SRC = """
pub fn f() {
    let s = "/*";
    let _ = s;
}
pub fn g() {
    let m = std::collections::HashMap::new();
    let _ = m;
}
"""

UNTERMINATED_BLOCK_COMMENT_SRC = """
pub fn f() { let x = 1; let _ = x; }
/* this comment never closes, so everything after it would be swallowed
pub fn g() {
    let m = std::collections::HashMap::new();
}
"""

# A crate root carrying a UTF-8 BOM, as several stubs in this repo do. The BOM
# must not hide a violation on line 1.
BOM_SRC = "﻿use std::collections::HashMap;\n"


def make_crate(root, name, sources):
    crate_dir = root / "crates" / name
    crate_dir.mkdir(parents=True)
    for fname, content in sources.items():
        (crate_dir / fname).write_text(content, encoding="utf-8")


def run_lint(root):
    return subprocess.run(
        [sys.executable, str(SCRIPT), str(root)],
        capture_output=True,
        text=True,
    )


# A violation line is `VIOLATION <path>:<lineno> <rule> <rest...>`.  The path
# may contain spaces (e.g. `C:\Users\Jane Doe\...`), so it is matched against
# the `:<lineno> ` suffix rather than whitespace-splitting the whole line.
VIOLATION_RE = re.compile(r"^VIOLATION.*?:(\d+)\s+(\S+)")


def _rule_from_violation(line):
    """Extract the rule name from a single `VIOLATION <path>:<lineno> <rule>` line."""
    m = VIOLATION_RE.match(line)
    if m:
        return m.group(2)
    return None


def rules_of(root):
    """Return (exit_code, [rule_name, ...]) for a lint run over `root`."""
    proc = run_lint(root)
    rules = []
    for line in proc.stdout.splitlines():
        if line.startswith("VIOLATION"):
            rule = _rule_from_violation(line)
            if rule is not None:
                rules.append(rule)
    return proc.returncode, rules


class LintCase(unittest.TestCase):
    """Base class providing a one-crate fixture run."""

    def lint_one(self, crate, src, fname="lib.rs"):
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d)
            make_crate(root, crate, {fname: src})
            return rules_of(root)

    def assert_clean(self, crate, src, why):
        code, rules = self.lint_one(crate, src)
        self.assertEqual(code, 0, f"{why}, got {rules}")

    def assert_flags(self, crate, src, rule, why):
        code, rules = self.lint_one(crate, src)
        self.assertEqual(code, 1, why)
        self.assertIn(rule, rules, why)


class TestClean(LintCase):
    def test_clean_tree_passes(self):
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d)
            for crate in ("crpg-core", "crpg-rules", "crpg-sim"):
                make_crate(root, crate, {"lib.rs": CLEAN_SRC})
            code, rules = rules_of(root)
            self.assertEqual(code, 0, f"clean tree should pass, got {rules}")


class TestBannedPatterns(LintCase):
    """The `both`-scope rules apply to every linted crate."""

    CASES = [
        ("no-hashmap", "crpg-core", HASHMAP_SRC),
        ("no-hashmap", "crpg-rules", HASHMAP_SRC),
        ("no-hashmap", "crpg-sim", HASHMAP_SRC),
        ("no-hashset", "crpg-core", HASHSET_SRC),
        ("no-hashset", "crpg-rules", HASHSET_SRC),
        ("no-wallclock", "crpg-core", WALLCLOCK_SRC),
        ("no-wallclock", "crpg-rules", WALLCLOCK_SRC),
        ("no-thread", "crpg-rules", THREAD_SRC),
        ("no-external-rng", "crpg-core", RNG_SRC),
        ("no-external-rng", "crpg-rules", RNG_SRC),
    ]

    def test_banned_patterns_are_detected(self):
        for expected_rule, crate, src in self.CASES:
            with self.subTest(crate=crate, rule=expected_rule):
                code, rules = self.lint_one(crate, src)
                self.assertEqual(code, 1, f"{crate}/{expected_rule} should fail")
                self.assertIn(expected_rule, rules)


class TestSkips(LintCase):
    def test_banned_pattern_in_comment_passes(self):
        self.assert_clean("crpg-rules", COMMENT_SRC, "comment should be skipped")

    def test_escape_with_reason_passes(self):
        self.assert_clean("crpg-rules", ESCAPE_OK_SRC, "valid escape should pass")

    def test_escape_without_reason_fails(self):
        self.assert_flags(
            "crpg-rules", ESCAPE_NO_REASON_SRC, "escape-no-reason",
            "escape with no reason should fail",
        )


class TestFloatScope(LintCase):
    """Floats are banned in crpg-core and crpg-rules; f64 is banned in crpg-sim.

    crpg-sim holds spatial positions, which spec 2.4 puts in f32 outside the
    rules path — so f32 passes there but f64 fails (E006-A). If that decision
    is ever revisited, this test is the place it shows up.
    """

    def test_float_in_core_fails(self):
        self.assert_flags("crpg-core", FLOAT_SRC, "no-float", "f64 in crpg-core")

    def test_float_in_rules_fails(self):
        self.assert_flags("crpg-rules", FLOAT_SRC, "no-float", "f64 in crpg-rules")

    def test_f32_in_sim_passes(self):
        self.assert_clean(
            "crpg-sim", FLOAT_SIM_F32_SRC, "f32 positions are allowed in sim",
        )

    def test_f64_in_sim_fails(self):
        self.assert_flags(
            "crpg-sim", FLOAT_SIM_F64_SRC, "no-f64", "f64 in crpg-sim",
        )

    def test_f64_suffix_literal_in_sim_fails(self):
        self.assert_flags(
            "crpg-sim", FLOAT_LITERAL_SRC, "no-f64",
            "1.5f64 is an f64 even in sim",
        )

    def test_f32_suffix_literal_in_sim_passes(self):
        self.assert_clean(
            "crpg-sim", FLOAT_UNDERSCORE_LITERAL_SRC,
            "1.0_f32 is an f32, allowed in sim",
        )

    def test_unsuffixed_literal_in_sim_passes(self):
        self.assert_clean(
            "crpg-sim", UNSUFFIXED_FLOAT_SRC,
            "unsuffixed literals infer to f32 at f32 sites; allowed in sim",
        )


class TestFloatLiterals(LintCase):
    """A suffixed float literal is a float.

    `\\bf64\\b` matched only the type-annotation form: in `1.5f64` the character
    before `f` is a digit, so the word boundary the rule relied on was not
    there and the literal went through.
    """

    def test_suffixed_literal_fails(self):
        self.assert_flags(
            "crpg-rules", FLOAT_LITERAL_SRC, "no-float", "1.5f64 is a float",
        )

    def test_underscore_suffixed_literal_fails(self):
        self.assert_flags(
            "crpg-rules", FLOAT_UNDERSCORE_LITERAL_SRC, "no-float",
            "1.0_f32 is a float",
        )

    def test_identifier_ending_in_the_token_passes(self):
        self.assert_clean(
            "crpg-rules", FLOAT_LOOKALIKE_SRC, "buf32 is not a float",
        )


class TestUnsupportedFloatLiterals(LintCase):
    """An unsuffixed float literal defaults to f64 and is banned in crpg-rules."""

    def test_float_literal_unsuffixed_fails(self):
        self.assert_flags(
            "crpg-rules", UNSUFFIXED_FLOAT_SRC, "no-float", "1.5 is a float",
        )

    def test_float_literal_exponent_fails(self):
        self.assert_flags(
            "crpg-rules", EXPONENT_FLOAT_SRC, "no-float", "1e3 is a float",
        )

    def test_float_literal_in_string_pass(self):
        self.assert_clean(
            "crpg-rules", FLOAT_IN_STRING_SRC, '"1.5" inside a string is not code',
        )

    def test_plain_integer_stays_clean(self):
        self.assert_clean(
            "crpg-rules", PLAIN_INT_SRC, "a plain integer is not a float",
        )

    def test_integer_with_method_stays_clean(self):
        self.assert_clean(
            "crpg-rules", INT_METHOD_SRC, "1u32.pow(2) is not a float",
        )

    def test_range_and_member_access_pass(self):
        self.assert_clean(
            "crpg-rules", RANGE_AND_MEMBER_SRC,
            "a range and a member access are not floats",
        )

    def test_range_only_pass(self):
        self.assert_clean(
            "crpg-rules", RANGE_ONLY_SRC,
            "0..n and 1..10 ranges are not floats",
        )

    def test_multi_digit_range_bound_pass(self):
        self.assert_clean(
            "crpg-rules",
            'pub fn f() { for _ in 0..120 { } }\n',
            "0..120 is a range, not a float",
        )

    def test_hex_int_pass(self):
        self.assert_clean(
            "crpg-rules", HEX_INT_SRC, "0x1F is an integer, not a float",
        )

    def test_char_literal_pass(self):
        self.assert_clean(
            "crpg-rules", CHAR_LITERAL_SRC, "'a' is a char, not a float",
        )


class TestStringEscapeMasking(LintCase):
    """A `// determinism-ok:` marker inside a string must not suppress rules."""

    def test_escape_marker_inside_string_does_not_suppress(self):
        self.assert_flags(
            "crpg-rules", ESCAPE_INSIDE_STRING_SRC, "no-hashmap",
            "a marker inside a string literal must not disable the line",
        )

    def test_rule_from_violation_handles_spaced_path(self):
        synthetic = "VIOLATION C:\\Users\\Jane Doe\\x.rs:3 no-hashmap std"
        self.assertEqual(_rule_from_violation(synthetic), "no-hashmap")


class TestDoctests(LintCase):
    """A doctest is compiled and run, so it is code the bans apply to."""

    def test_doctest_body_is_scanned(self):
        code, rules = self.lint_one("crpg-core", DOCTEST_SRC)
        self.assertEqual(code, 1, "a doctest using HashMap and f64 should fail")
        self.assertIn("no-hashmap", rules)
        self.assertIn("no-float", rules)

    def test_inner_doc_comment_doctest_is_scanned(self):
        self.assert_flags(
            "crpg-core", INNER_DOCTEST_SRC, "no-hashmap",
            "a //! doctest is a doctest",
        )

    def test_doc_prose_outside_a_fence_is_not_scanned(self):
        self.assert_clean(
            "crpg-core", DOC_PROSE_SRC, "prose naming a banned type is not code",
        )

    def test_text_fence_is_not_scanned(self):
        self.assert_clean(
            "crpg-core", DOCTEST_TEXT_FENCE_SRC, "a ```text fence is not compiled",
        )

    def test_ignore_fence_is_not_scanned(self):
        self.assert_clean(
            "crpg-core", DOCTEST_IGNORE_FENCE_SRC,
            "a ```ignore fence is not compiled",
        )


class TestBlockComments(LintCase):
    """`/* ... */` is prose, and prose naming a banned type is not a violation."""

    def test_single_line_block_comment_passes(self):
        self.assert_clean(
            "crpg-rules", BLOCK_COMMENT_SRC, "a block comment is not code",
        )

    def test_multiline_block_comment_passes(self):
        self.assert_clean(
            "crpg-rules", MULTILINE_BLOCK_COMMENT_SRC,
            "a multi-line block comment is not code",
        )

    def test_nested_block_comment_passes(self):
        self.assert_clean(
            "crpg-rules", NESTED_BLOCK_COMMENT_SRC,
            "Rust block comments nest",
        )

    def test_code_after_a_block_comment_is_still_scanned(self):
        self.assert_flags(
            "crpg-rules", BLOCK_COMMENT_THEN_CODE_SRC, "no-hashmap",
            "closing a block comment must not disable the rest of the line",
        )

    def test_block_comment_opener_in_a_string_does_not_swallow_the_file(self):
        self.assert_flags(
            "crpg-rules", BLOCK_COMMENT_IN_STRING_SRC, "no-hashmap",
            'a "/*" string literal must not open a comment',
        )

    def test_unterminated_block_comment_is_reported(self):
        self.assert_flags(
            "crpg-rules", UNTERMINATED_BLOCK_COMMENT_SRC,
            "unterminated-block-comment",
            "silently swallowing the rest of a file is the failure to avoid",
        )


class TestEncoding(LintCase):
    def test_bom_does_not_hide_a_violation_on_line_one(self):
        self.assert_flags(
            "crpg-rules", BOM_SRC, "no-hashmap", "BOM should not mask line 1",
        )


if __name__ == "__main__":
    unittest.main()
