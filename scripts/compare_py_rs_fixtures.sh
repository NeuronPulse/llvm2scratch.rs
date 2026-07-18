#!/bin/bash
set -uo pipefail

# Compare Python and Rust compiler outputs for the parser fixtures under
# vendor/llvm-ir-parser/tests/fixtures. This script delegates to a Python helper
# that extracts the first function as the entrypoint, normalizes scratchblocks
# output, and reports exact parity for fixtures that both compilers can handle.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

RUST_BIN="${REPO_ROOT}/target/release/llvm2scratch"
if [ ! -x "$RUST_BIN" ]; then
  echo "ERROR: Rust binary not found at $RUST_BIN" >&2
  echo "Run: cargo build --release" >&2
  exit 1
fi

exec python3 "${SCRIPT_DIR}/check_fixture_parity.py"
