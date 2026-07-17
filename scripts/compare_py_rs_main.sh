#!/bin/bash
set -uo pipefail

# Compare Python and Rust compiler outputs for the generated complex programs
# under /tmp/llvm_stress_main/ll/. Both optimized and unoptimized modes are
# checked; unoptimized mode verifies exact scratchblocks parity.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
RUST_BIN="${REPO_ROOT}/target/release/llvm2scratch"
STRESS_DIR="/tmp/llvm_stress_main/ll"

if [ ! -x "$RUST_BIN" ]; then
  echo "ERROR: Rust binary not found at $RUST_BIN" >&2
  echo "Run: cargo build --release" >&2
  exit 1
fi

if [ ! -d "$STRESS_DIR" ]; then
  echo "ERROR: Stress test directory not found: $STRESS_DIR" >&2
  echo "Generate complex programs first (see scripts/generate_complex_programs.sh)." >&2
  exit 1
fi

WORK_DIR="/tmp/py_rs_main_compare"
mkdir -p "$WORK_DIR"

run_mode() {
  local mode=$1
  local optimize_arg=$2
  local report="$WORK_DIR/report_${mode}.txt"
  echo "=== Python vs Rust complex main diff ($mode) ===" > "$report"

  local total=0 both_ok=0 sb_mismatch=0 py_ok_rust_fail=0 both_fail=0 py_fail_rust_ok=0

  for ll in "$STRESS_DIR"/*.ll; do
    total=$((total + 1))
    local name stem
    name=$(basename "$ll")
    stem="${name%.ll}"
    local py_out="$WORK_DIR/py_${mode}/${stem}.sb3"
    local rs_out="$WORK_DIR/rs_${mode}/${stem}.sb3"
    local py_sb="$WORK_DIR/py_${mode}/${stem}.sb"
    local rs_sb="$WORK_DIR/rs_${mode}/${stem}.sb"
    local py_log="$WORK_DIR/py_${mode}/${stem}.log"
    local rs_log="$WORK_DIR/rs_${mode}/${stem}.log"
    mkdir -p "$(dirname "$py_out")" "$(dirname "$rs_out")"

    python -m llvm2scratch.cli "$ll" -o "$py_out" $optimize_arg --debug-scratchblocks "$py_sb" > "$py_log" 2>&1
    local py_code=$?

    "$RUST_BIN" "$ll" "$rs_out" $optimize_arg --debug-scratchblocks "$rs_sb" > "$rs_log" 2>&1
    local rs_code=$?

    if [ "$py_code" -eq 0 ] && [ "$rs_code" -ne 0 ]; then
      py_ok_rust_fail=$((py_ok_rust_fail + 1))
      echo "PY_OK_RS_FAIL: $name" >> "$report"
      head -n 2 "$rs_log" | sed 's/^/  /' >> "$report"
    elif [ "$py_code" -ne 0 ] && [ "$rs_code" -ne 0 ]; then
      both_fail=$((both_fail + 1))
      echo "BOTH_FAIL: $name" >> "$report"
    elif [ "$py_code" -eq 0 ] && [ "$rs_code" -eq 0 ]; then
      both_ok=$((both_ok + 1))
      if ! python3 - "$py_sb" "$rs_sb" "$name" <<'PY'
import difflib, re, sys

def normalize(text: str) -> str:
    marker = "<false::extension> // Known false block using an empty boolean input"
    if marker in text:
        text = text[text.find(marker) + len(marker):]
    text = text.strip()
    tmp_pattern = re.compile(r"%!?tmp:[a-zA-Z0-9]+")
    counter = 0
    seen: dict[str, str] = {}
    def repl(m: re.Match) -> str:
        nonlocal counter
        name = m.group(0)
        if name not in seen:
            seen[name] = f"__TMP_{counter}__"
            counter += 1
        return seen[name]
    return tmp_pattern.sub(repl, text)

py_sb, rs_sb, name = sys.argv[1:4]
with open(py_sb) as f:
    py_text = normalize(f.read())
with open(rs_sb) as f:
    rs_text = normalize(f.read())
if py_text == rs_text:
    sys.exit(0)
diff = list(difflib.unified_diff(
    py_text.splitlines(), rs_text.splitlines(),
    fromfile="python", tofile="rust", lineterm="", n=2,
))
print(f"SCRATCHBLOCKS_MISMATCH: {name} ({len(diff)} diff lines)")
for line in diff[:40]:
    print(line)
sys.exit(1)
PY
      then
        sb_mismatch=$((sb_mismatch + 1))
        echo "SCRATCHBLOCKS_MISMATCH: $name" >> "$report"
      fi
    else
      py_fail_rust_ok=$((py_fail_rust_ok + 1))
      echo "PY_FAIL_RS_OK: $name" >> "$report"
    fi
  done

  echo "" >> "$report"
  echo "$mode: total=$total, both_ok=$both_ok, scratchblocks_mismatch=$sb_mismatch, py_ok_rust_fail=$py_ok_rust_fail, both_fail=$both_fail, py_fail_rust_ok=$py_fail_rust_ok" >> "$report"
  cat "$report"

  # Rust must match Python scratchblocks and must not fail where Python succeeds.
  if [ "$sb_mismatch" -gt 0 ] || [ "$py_ok_rust_fail" -gt 0 ]; then
    return 1
  fi
  return 0
}

cd "$REPO_ROOT"
FAILED=0
run_mode "optimized" "" || FAILED=1
echo ""
run_mode "unoptimized" "-O none" || FAILED=1

exit "$FAILED"
