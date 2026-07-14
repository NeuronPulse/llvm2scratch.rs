#!/bin/bash
set -uo pipefail

# Compile the example C programs in examples/complex/ to LLVM IR at multiple
# optimization levels. The generated .ll files are placed under
# /tmp/llvm_stress_main/ll/ where the stress tests expect them.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC_DIR="${SCRIPT_DIR}/../examples/complex"
OUT_DIR="/tmp/llvm_stress_main/ll"

if [ ! -d "$SRC_DIR" ]; then
  echo "ERROR: Source directory not found: $SRC_DIR" >&2
  exit 1
fi

mkdir -p "$OUT_DIR"

CC=${CC:-clang}
if ! command -v "$CC" >/dev/null 2>&1; then
  echo "ERROR: clang not found. Set CC to a working C compiler." >&2
  exit 1
fi

for cfile in "$SRC_DIR"/*.c; do
  stem=$(basename "$cfile" .c)
  for level in 0 1 2 3; do
    out="$OUT_DIR/${stem}_O${level}.ll"
    "$CC" -S -emit-llvm -O"$level" -m32 "$cfile" -o "$out"
    echo "Generated $out"
  done
done

echo "Done. Output in $OUT_DIR"
