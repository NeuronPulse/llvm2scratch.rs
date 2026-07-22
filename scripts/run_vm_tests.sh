#!/bin/bash
set -uo pipefail

# Run headless VM execution tests for llvm2scratch compiled Scratch projects.
#
# This script uses a vendored copy of TurboWarp/scratch-vm. If the vendor
# directory is missing, it will be cloned automatically from GitHub over HTTPS.
#
# Usage:
#   bash scripts/run_vm_tests.sh [fixture_name_or_path ...]

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

VM_DIR="${REPO_ROOT}/tests/vm"
VENDOR_DIR="${VM_DIR}/vendor"
NODE_MODULES_DIR="${VM_DIR}/node_modules"
RUST_BIN="${REPO_ROOT}/target/release/llvm2scratch"

if [ ! -x "$RUST_BIN" ]; then
  RUST_BIN="${REPO_ROOT}/target/debug/llvm2scratch"
fi

if [ ! -x "$RUST_BIN" ]; then
  echo "ERROR: Rust binary not found. Run: cargo build --release" >&2
  exit 1
fi

cd "$REPO_ROOT"

# Ensure the TurboWarp VM dependencies are available.
ensure_vendor() {
  local repo="$1"
  local path="$2"
  if [ -d "$path/.git" ]; then
    return 0
  fi
  if [ -d "$path" ]; then
    echo "WARNING: $path exists but is not a git clone; removing it" >&2
    rm -rf "$path"
  fi
  echo "Cloning $repo into $path ..."
  mkdir -p "$VENDOR_DIR"
  git clone --depth 1 "https://github.com/$repo.git" "$path"
}

ensure_vendor "TurboWarp/scratch-vm" "${VENDOR_DIR}/scratch-vm"
ensure_vendor "TurboWarp/scratch-parser" "${VENDOR_DIR}/scratch-parser"
ensure_vendor "TurboWarp/scratch-render-fonts" "${VENDOR_DIR}/scratch-render-fonts"

# Patch scratch-vm to reference the locally cloned dependencies. This avoids
# npm trying to fetch them from GitHub again.
SCRATCH_VM_PKG="${VENDOR_DIR}/scratch-vm/package.json"
if [ -f "$SCRATCH_VM_PKG" ]; then
  sed -i \
    -e 's|"scratch-parser": *"github:TurboWarp/scratch-parser#master"|"scratch-parser": "file:../scratch-parser"|' \
    -e 's|"scratch-render-fonts": *"github:TurboWarp/scratch-render-fonts#master"|"scratch-render-fonts": "file:../scratch-render-fonts"|' \
    "$SCRATCH_VM_PKG"
fi

# Install Node dependencies if the environment is not ready.
needs_install=0
if [ ! -d "$NODE_MODULES_DIR" ]; then
  needs_install=1
elif [ ! -d "${NODE_MODULES_DIR}/scratch-vm" ]; then
  needs_install=1
elif [ ! -d "${NODE_MODULES_DIR}/jszip" ]; then
  needs_install=1
elif [ ! -d "${NODE_MODULES_DIR}/scratch-storage" ]; then
  needs_install=1
fi

if [ "$needs_install" -eq 1 ]; then
  echo "Installing VM test Node dependencies ..."
  (
    cd "$VM_DIR" && npm install
  )
fi

# scratch-vm's local file: dependencies are not always linked by npm when the
# package is installed from a local directory. Create the symlinks explicitly.
link_vendor_module() {
  local name="$1"
  local target="${VENDOR_DIR}/${name}"
  local link="${NODE_MODULES_DIR}/${name}"

  if [ -L "$link" ]; then
    # Remove broken or outdated symlinks.
    if [ "$(readlink "$link")" != "../vendor/${name}" ]; then
      rm -f "$link"
    fi
  elif [ -e "$link" ]; then
    rm -rf "$link"
  fi

  if [ ! -e "$link" ]; then
    ln -s "../vendor/${name}" "$link"
  fi
}

link_vendor_module "scratch-parser"
link_vendor_module "scratch-render-fonts"

echo ""
echo "=== VM execution tests ==="
python3 "${VM_DIR}/../vm_execution_test.py" "$@"
