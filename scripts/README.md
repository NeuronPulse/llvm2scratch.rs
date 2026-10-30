# Automation and helper scripts

Shell scripts and Python helpers used for testing, comparison and inspection.
Run them manually or via CI. The single entry point is **`run_diff_tests.sh`**,
which chains most of the checks below (and is what `.github/workflows/ci.yml` runs).

## Naming convention

| Prefix | Meaning |
|---|---|
| `run_*` | Entry point / orchestrator that runs several steps |
| `check_*` | Verification gate — non-zero exit on failure |
| `gen_*` | Generator that produces fixtures or inputs |
| `inspect_*` | Read-only debugging/inspection helper (never used by gates) |

## Scripts

| Script | Purpose |
|---|---|
| `run_diff_tests.sh` | **Entry point.** Unit tests → SB3/scratchblocks diffs → byte-parity gate → stress tests → parser fixtures → VM execution tests. Flags: `--no-stress`, `--no-vm`, `--no-strict`. |
| `run_vm_tests.sh` | Run the headless VM execution suite (`tests/vm_execution_test.py`), auto-fetching the vendored scratch-vm deps first. |
| `check_byte_parity.py` | **Parity gate.** Byte-for-byte `project.json` comparison with automatic diff classification into root-cause categories (C1–C8). Fails on any non-exempt difference (uuid etc.) and prints a per-category tally. Supports `--json` for CI. Accepts explicit inputs, else uses `examples/input/`. |
| `check_stress_parity.py` | Scratchblocks parity for generated stress programs (`/tmp/llvm_stress_main/ll/*.ll`), both optimized and unoptimized. |
| `check_fixture_parity.py` | Compile the parser fixtures with both implementations and assert exact scratchblocks parity for fixtures that Python supports. |
| `gen_stress_programs.sh` | Compile the stress-program C sources with clang into `/tmp/llvm_stress_main/ll/*.ll`. Must be run before `check_stress_parity.py`. |
| `inspect_sb3_diff.py` | Compare the block/top-level structure of two `.sb3` files. Standalone debugging tool. |
| `inspect_pen_demo_init.py` | Print the `!init` procedure and memory lists of `examples/output/pen_demo.sb3`. Standalone debugging tool. |

Removed for being redundant (history still in git): a stale duplicate of the byte-parity
script, a thin `.sh` wrapper that only forwarded to `check_fixture_parity.py`, and their docs.

## Typical local run

```bash
cargo build --release
bash scripts/gen_stress_programs.sh   # optional, needed for step "Stress scratchblocks diff"
bash scripts/run_diff_tests.sh        # everything else
bash scripts/run_diff_tests.sh --no-stress --no-vm   # quick subset
```
