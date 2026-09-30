#!/usr/bin/env bash
# Local native preflight: runs the repository's existing gates in a fixed
# order and stops at the first failure (T031). It provisions nothing, installs
# nothing, and never passes --target. See tools/README.md.
#
# Usage: bash tools/preflight.sh [--crate <package>] [--check-only]
#
# Exit codes: 0 all gates passed; 1 a gate failed (the command and its exit
# code are printed); 2 invalid arguments, missing prerequisite, or malformed
# cargo metadata (no modifying gate has run).

set -u

usage() {
    # Builtins only: the declared tools may be the only executables on PATH.
    local line
    while IFS= read -r line; do
        printf '%s\n' "$line"
    done <<'EOF'
Usage: bash tools/preflight.sh [--crate <package>] [--check-only]

Runs the local native gates, in order, stopping at the first failure:
  1. rustc -vV
  2. cargo metadata package validation
  3. cargo fmt --all (skipped with --check-only), then cargo fmt --all -- --check
  4. cargo clippy (workspace, or -p <package>) --all-targets --locked -- -D warnings
  5. cargo test (workspace, or -p <package>) --locked
  6. python3 tools/lint/deps.py
  7. python3 tools/lint/determinism.py
  8. python3 -m unittest discover -s tools/lint -p "test_*.py"
  9. cargo deny check (always the workspace graph)
 10. git diff --check

Options:
  --crate <package>  run crate gates for one workspace member package
  --check-only       do not rewrite formatting; only check it
  --help, -h         show this help and exit

A crate run does not satisfy the full-workspace or second-native-target
acceptance of any task; both modes are local checks, not a CI result.
EOF
}

fail_usage() {
    printf 'preflight: %s\n' "$1" >&2
    printf 'Run with --help for usage.\n' >&2
    exit 2
}

fail_prereq() {
    printf 'preflight: %s\n' "$1" >&2
    exit 2
}

crate=""
crate_set=0
check_only=0
while [ "$#" -gt 0 ]; do
    case "$1" in
        --help | -h)
            usage
            exit 0
            ;;
        --crate)
            if [ "$crate_set" -eq 1 ]; then
                fail_usage "--crate given more than once"
            fi
            if [ "$#" -lt 2 ]; then
                fail_usage "--crate needs a package name"
            fi
            if [ -z "$2" ]; then
                fail_usage "--crate package name is empty"
            fi
            crate="$2"
            crate_set=1
            shift 2
            ;;
        --check-only)
            if [ "$check_only" -eq 1 ]; then
                fail_usage "--check-only given more than once"
            fi
            check_only=1
            shift
            ;;
        *)
            fail_usage "unexpected argument: $1"
            ;;
    esac
done

# Resolve the repository root from this script's own location.
source_path="${BASH_SOURCE[0]}"
case "$source_path" in
    */*) script_dir="${source_path%/*}" ;;
    *) script_dir="." ;;
esac
if ! root="$(cd -- "$script_dir/.." && pwd -P)"; then
    fail_prereq "cannot resolve the repository root from $source_path"
fi

for required in Cargo.toml rust-toolchain.toml tools/lint/deps.py tools/lint/determinism.py; do
    if [ ! -f "$root/$required" ]; then
        fail_prereq "missing $required under $root"
    fi
done

for tool in rustc cargo python3 git; do
    if ! command -v "$tool" > /dev/null 2>&1; then
        fail_prereq "required executable not found on PATH: $tool"
    fi
done

cd -- "$root" || fail_prereq "cannot enter $root"

if ! python3 -c 'import sys; sys.exit(0 if sys.version_info >= (3, 11) else 1)'; then
    fail_prereq "python3 3.11 or newer is required"
fi

# Runs one gate, passing its output through; stops on a nonzero exit.
gate() {
    printf '==> %s\n' "$*"
    "$@"
    local code=$?
    if [ "$code" -ne 0 ]; then
        printf 'preflight: FAILED (exit %d): %s\n' "$code" "$*" >&2
        exit 1
    fi
}

gate rustc -vV

printf '==> %s\n' "cargo metadata --no-deps --format-version 1 --locked"
metadata="$(cargo metadata --no-deps --format-version 1 --locked)"
code=$?
if [ "$code" -ne 0 ]; then
    printf 'preflight: FAILED (exit %d): %s\n' "$code" \
        "cargo metadata --no-deps --format-version 1 --locked" >&2
    exit 1
fi
# Exit 0: valid (and, in crate mode, an exact workspace member); 3: malformed
# metadata; 4: not a workspace member.
membership='
import json, sys
try:
    data = json.loads(sys.stdin.read())
    members = data["workspace_members"]
    packages = data["packages"]
    if not isinstance(members, list) or not members or not isinstance(packages, list):
        raise ValueError("no workspace members")
    ids = set(members)
    names = sorted(p["name"] for p in packages if p["id"] in ids)
    if len(names) != len(ids):
        raise ValueError("workspace member without a package entry")
except Exception as error:
    print(f"preflight: malformed cargo metadata: {error}", file=sys.stderr)
    sys.exit(3)
wanted = sys.argv[1]
if wanted and wanted not in names:
    print(f"preflight: {wanted!r} is not a workspace member package", file=sys.stderr)
    sys.exit(4)
'
printf '%s' "$metadata" | python3 -c "$membership" "$crate"
code=$?
if [ "$code" -ne 0 ]; then
    exit 2
fi

if [ "$check_only" -eq 0 ]; then
    gate cargo fmt --all
fi
gate cargo fmt --all -- --check
if [ "$crate_set" -eq 1 ]; then
    gate cargo clippy -p "$crate" --all-targets --locked -- -D warnings
    gate cargo test -p "$crate" --locked
else
    gate cargo clippy --workspace --all-targets --locked -- -D warnings
    gate cargo test --workspace --locked
fi
gate python3 tools/lint/deps.py
gate python3 tools/lint/determinism.py
gate python3 -m unittest discover -s tools/lint -p "test_*.py"
gate cargo deny check
gate git diff --check

if [ "$crate_set" -eq 1 ]; then
    printf 'preflight: all crate gates passed for %s (not a full-workspace result)\n' "$crate"
else
    printf 'preflight: all workspace gates passed\n'
fi
