# Automation and helper scripts

This directory contains shell scripts and small Python helpers used for testing,
comparison and inspection. They are intended to be run manually or from CI.

| Script | Purpose |
|---|---|
| `run_diff_tests.sh` | **Unified entry point** for all Python-vs-Rust differential tests. Runs unit tests, SB3/scratchblocks diffs, the strict byte-parity gate, stress tests, parser fixture comparisons and VM execution tests. |
| `parity_classify.py` | **Parity gate**: byte-for-byte project.json comparison with automatic diff classification into root-cause categories (C1–C8, see `docs/parity-analysis.md`). Fails on any non-exempt difference (uuid etc.) and prints a per-category tally to drive convergence to zero. Supports `--json` for CI. |
| `check_stress_parity.py` | Compare Python and Rust scratchblocks output for generated stress programs (`/tmp/llvm_stress_main/ll/*.ll`), in both optimized and unoptimized modes. |
| `check_fixture_parity.py` | Compile the parser fixtures with both Python and Rust implementations and verify exact scratchblocks parity for fixtures that Python supports. |
| `strict_parity.py` | Earlier byte-parity script (forced minify, opt off); superseded by `parity_classify.py` and kept for ad-hoc use. |
| `compare_py_rs_fixtures.sh` | Thin wrapper around `check_fixture_parity.py`. |
| `generate_complex_programs.sh` | Generate the `/tmp/llvm_stress_main/ll/*.ll` stress fixtures used by the tests above. |
| `diff_compare.py` | Helper that compares the block/top-level structure of two `.sb3` files. Kept as a standalone inspection tool. |
| `inspect_pen_demo_init.py` | Helper that prints the `!init` procedure and memory lists of `examples/output/pen_demo.sb3` for debugging. |
