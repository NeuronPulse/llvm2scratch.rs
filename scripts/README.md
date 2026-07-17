# Automation and helper scripts

This directory contains shell scripts and small Python helpers used for testing,
comparison and inspection. They are intended to be run manually or from CI.

| Script | Purpose |
|---|---|
| `run_diff_tests.sh` | **Unified entry point** for all Python-vs-Rust differential tests. Runs unit tests, sb3/scratchblocks diffs, stress tests and parser fixture comparisons. |
| `compare_py_rs_main.sh` | Compare Python and Rust scratchblocks output for generated stress programs, in both optimized and unoptimized modes. |
| `compare_py_rs_fixtures.sh` | Compile the parser fixtures with both Python and Rust implementations and report where one succeeds and the other fails. |
| `generate_complex_programs.sh` | Generate the `/tmp/llvm_stress_main/ll/*.ll` stress fixtures used by the tests above. |
| `diff_compare.py` | Helper that compares the block/top-level structure of two `.sb3` files. Used by older diff tests. |
| `inspect_pen_demo_init.py` | Helper that prints the `!init` procedure and memory lists of `examples/output/pen_demo.sb3` for debugging. |
