#!/bin/bash
set -uo pipefail

# Compare Python and Rust compiler support for the parser fixtures under
# vendor/llvm-ir-parser/tests/fixtures. Fixtures rarely contain a main(), so the
# first defined function is used as the entrypoint.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
RUST_BIN="${REPO_ROOT}/target/release/llvm2scratch"
FIXTURES_DIR="${REPO_ROOT}/vendor/llvm-ir-parser/tests/fixtures"

if [ ! -x "$RUST_BIN" ]; then
  echo "ERROR: Rust binary not found at $RUST_BIN" >&2
  echo "Run: cargo build --release" >&2
  exit 1
fi

WORK_DIR="/tmp/py_rs_fixtures_compare"
mkdir -p "$WORK_DIR/py" "$WORK_DIR/rs"
report="$WORK_DIR/report.txt"

echo "=== Python vs Rust parser fixtures diff ===" > "$report"
total=0
py_ok_rust_fail=0
both_fail=0
both_ok=0
py_fail_rust_ok=0

for ll in "$FIXTURES_DIR"/*.ll; do
  total=$((total + 1))
  name=$(basename "$ll")
  stem="${name%.ll}"
  py_out="$WORK_DIR/py/${stem}.sb3"
  rs_out="$WORK_DIR/rs/${stem}.sb3"
  py_log="$WORK_DIR/py/${stem}.log"
  rs_log="$WORK_DIR/rs/${stem}.log"

  entrypoint=$(grep -m1 -E '^define[[:space:]]+.*[[:space:]]@' "$ll" 2>/dev/null | sed -E 's/.*@([A-Za-z0-9_]+).*/\1/' || true)
  if [ -z "$entrypoint" ]; then
    entrypoint="main"
  fi

  python -m llvm2scratch.cli "$ll" -o "$py_out" --entrypoint "$entrypoint" > "$py_log" 2>&1
  py_code=$?

  "$RUST_BIN" "$ll" "$rs_out" --entrypoint "$entrypoint" > "$rs_log" 2>&1
  rs_code=$?

  if [ "$py_code" -eq 0 ] && [ "$rs_code" -ne 0 ]; then
    py_ok_rust_fail=$((py_ok_rust_fail + 1))
    echo "PY_OK_RS_FAIL: $name" >> "$report"
    head -n 2 "$rs_log" | sed 's/^/  /' >> "$report"
  elif [ "$py_code" -ne 0 ] && [ "$rs_code" -ne 0 ]; then
    both_fail=$((both_fail + 1))
  elif [ "$py_code" -eq 0 ] && [ "$rs_code" -eq 0 ]; then
    both_ok=$((both_ok + 1))
  else
    py_fail_rust_ok=$((py_fail_rust_ok + 1))
    echo "PY_FAIL_RS_OK: $name" >> "$report"
    head -n 2 "$py_log" | sed 's/^/  PY: /' >> "$report"
  fi
done

echo "" >> "$report"
echo "Fixtures: total=$total, both_ok=$both_ok, py_ok_rust_fail=$py_ok_rust_fail, both_fail=$both_fail, py_fail_rust_ok=$py_fail_rust_ok" >> "$report"
cat "$report"
