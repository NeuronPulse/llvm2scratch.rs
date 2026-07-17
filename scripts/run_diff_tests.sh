#!/bin/bash
set -uo pipefail

# Unified entry point for all Python-vs-Rust differential tests.
#
# Runs, in order:
#   1. Python unit tests (llvm2scratch.tests.test_compiler)
#   2. SB3 structural diff on examples/input/ .ll files (tests/diff_test.py)
#   3. Scratchblocks text diff on examples/input/ .ll files (tests/scratchblocks_diff_test.py)
#   4. Scratchblocks text diff on generated stress programs, if available
#      (tests/scratchblocks_stress_diff_test.py or scripts/compare_py_rs_main.sh)
#   5. Parser fixtures compiler diff (scripts/compare_py_rs_fixtures.sh)
#
# Usage:
#   bash scripts/run_diff_tests.sh
#   bash scripts/run_diff_tests.sh --no-stress   # skip stress tests

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

RUST_BIN="${REPO_ROOT}/target/release/llvm2scratch"
if [ ! -x "$RUST_BIN" ]; then
  RUST_BIN="${REPO_ROOT}/target/debug/llvm2scratch"
fi

if [ ! -x "$RUST_BIN" ]; then
  echo "ERROR: Rust binary not found. Run: cargo build --release" >&2
  exit 1
fi

RUN_STRESS=1
for arg in "$@"; do
  case "$arg" in
    --no-stress) RUN_STRESS=0 ;;
    *) echo "Unknown option: $arg" >&2; exit 1 ;;
  esac
done

FAILED=0

run_step() {
  local name="$1"
  shift
  echo ""
  echo "=== $name ==="
  if "$@"; then
    echo "PASS: $name"
  else
    echo "FAIL: $name" >&2
    FAILED=$((FAILED + 1))
  fi
}

cd "$REPO_ROOT"

# 1. Python unit tests
run_step "Python unit tests" \
  python -m unittest llvm2scratch.tests.test_compiler

# 2. SB3 structural diff on examples/input/
run_step "SB3 structural diff" \
  python tests/diff_test.py

# 3. Scratchblocks diff on examples/input/
run_step "Scratchblocks diff" \
  python tests/scratchblocks_diff_test.py

# 4. Stress tests (if requested and available)
if [ "$RUN_STRESS" -eq 1 ]; then
  if [ -d "/tmp/llvm_stress_main/ll" ]; then
    run_step "Stress scratchblocks diff" \
      python tests/scratchblocks_stress_diff_test.py
    run_step "Stress optimized/unoptimized diff" \
      bash scripts/compare_py_rs_main.sh
  else
    echo ""
    echo "=== Stress tests ==="
    echo "SKIP: /tmp/llvm_stress_main/ll not found. Generate with:"
    echo "  bash scripts/generate_complex_programs.sh"
  fi
fi

# 5. Parser fixtures compiler diff
run_step "Parser fixtures diff" \
  bash scripts/compare_py_rs_fixtures.sh

echo ""
echo "============================================"
if [ "$FAILED" -eq 0 ]; then
  echo "All differential tests passed."
  exit 0
else
  echo "$FAILED differential test group(s) failed."
  exit 1
fi
