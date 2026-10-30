# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

An LLVM backend that compiles LLVM 19 IR (`.ll`) into MIT Scratch projects (`.sb3`/`.sprite3`) or scratchblocks text. The repo holds **two parallel implementations of the same pipeline**:

- **`llvm2scratch/`** — the original **Python** implementation. It is the **authoritative reference**: when behavior differs, Python is correct.
- **`src/`** — a **Rust** reimplementation that must produce **byte-identical** output to Python for supported inputs. Parity is enforced by differential tests, not aspirational.

When changing the Rust code, the goal is almost always to match Python's observable output (project.json bytes, scratchblocks text). Read the corresponding Python module before changing a Rust module — the Rust code frequently carries comments citing the Python line/behavior it mirrors (e.g. `scratch/mod.rs` refers to `scratch.py`).

## Build & run

```bash
cargo build                       # debug binary at target/debug/llvm2scratch
cargo build --release             # release binary at target/release/llvm2scratch (used by test scripts)
./target/debug/llvm2scratch input.ll -o output.sb3

pip install -e .                  # install the Python reference package
llvm2scratch input.ll -o output.sb3   # Python CLI (same core flags as Rust)
```

Both CLIs accept the same options (`-T/--targets`, `-U/--opt-target`, `-O/--optimizations`, `-M/--minify`, `--replace-hacked-blocks`, `--no-optimize`, `--debug-scratchblocks`, etc.). Run `--help` for the full list. Input must be compiled with **clang 19**.

## Testing

The canonical check is the differential suite (mirrors CI in `.github/workflows/ci.yml`). It requires a **release** build:

```bash
cargo build --release
cargo test --release                        # Rust unit tests
python -m pytest llvm2scratch/tests/        # Python unit tests
bash scripts/gen_stress_programs.sh         # generate /tmp/llvm_stress_main/ll/*.ll stress fixtures
bash scripts/run_diff_tests.sh              # unified Python-vs-Rust diff: unit + sb3 + scratchblocks + stress + fixtures + VM
```

`run_diff_tests.sh` flags: `--no-stress` (skip generated stress programs), `--no-vm` (skip headless VM run), `--no-strict` (skip the byte-parity gate).

Targeted checks:

```bash
cargo test --lib                                    # Rust unit tests only
python -m pytest llvm2scratch/tests/test_compiler.py::TestName::test_case   # single Python test
python tests/sb3_diff_test.py                       # .sb3 structural equivalence over examples/input/*.ll
python tests/scratchblocks_diff_test.py             # readable scratchblocks text parity
python3 scripts/check_byte_parity.py [files...]     # byte-exact project.json parity gate + diff auto-classification (C1–C8, docs/parity-analysis.md)
bash scripts/run_vm_tests.sh                        # headless execution via vendored TurboWarp scratch-vm (tests/vm/)
```

Differential tests treat Python as ground truth: cases where Python fails to compile are **skipped** (marked `PY_FAIL`), not counted as Rust failures. `tests/README.md` and `scripts/README.md` document each script.

**Parity gate**: `check_byte_parity.py` is the authoritative byte-level check (wired into `run_diff_tests.sh`). Any non-exempt difference (uuid etc.) fails it, and every difference is auto-classified into a root-cause category with a sample — use that tally, not the normalized structural tests, to judge parity progress. `docs/parity-analysis.md` explains the known categories and how to attribute new ones.

## Pipeline architecture

Both implementations follow the same four stages: **parse → generate → optimize → export**. In Rust the top-level flow lives in `src/main.rs`, calling into these modules:

1. **Parse** (`src/parser/`, Python `parser.py`): `parse_assembly` in `llvm_decode.rs` turns `.ll` text into a `DecodedModule`. Uses the vendored `llvm-ir` / `llvm-ir-parser` crates (see below). `llvm_in_rust.rs` bridges the vendored AST into this project's IR.

2. **IR** (`src/ir/`, Python `ir.py`): the project's own typed representation — `types.rs`, `values.rs`, `instructions.rs` (`Module`, `Function`, `Block`, `Instr`).

3. **Generate/compile** (`src/compiler/`, Python `compiler.py`): the heart of the project. `translate.rs` (~8.7k lines) lowers IR into a Scratch `Project`. Supporting modules: `config.rs` (`CompilerConfig` — memory sizing, variable naming, target-dependent knobs), `memory.rs` (stack/heap model over Scratch lists), `binop.rs` + `twos_complement.rs` (integer/bitwise lowering, incl. multiplication that avoids JS float precision loss — see README "Proofs"), `variable.rs` (value cost / inference), `graph_util.rs` (per-function CFG work).

4. **Scratch AST** (`src/scratch/`, Python `scratch.py`): `ast.rs` defines the block AST (`Block`, `Value`, `Op` and friends). `mod.rs` holds `Project`/`ScratchContext` and lowers the AST into Scratch's `project.json` shape. `stringify.rs` renders scratchblocks text; `uid.rs` generates block UIDs; `export.rs` writes the `.sb3`/`.sprite3` zip.

5. **Optimize** (`src/optimizer/`, Python `optimizer.py`): runs on the generated `Project` (not the IR). Two passes — `known_value_prop.rs` (value/block simplification) and `assignment_elision.rs` (inline single-use `Set Variable`s). See `optimize()` in `mod.rs`. Optimization operates on Scratch blocks, so it depends on the opt-target's block-cost model.

6. **Targets** (`src/target/`, Python `target.py`): `data/targets/*.toml` (`scratch3`, `turbowarp3`) describe per-target block performance and execution limits. `loader.rs` reads them at runtime, resolving `data/targets/` relative to the executable (falling back to cwd) — so the `data/` directory must sit next to the binary or you must run from the repo root. `BranchMethod` (proc-call vs jump-table) and max recursion depth are chosen per target.

`src/graph_util/` provides generic CFG algorithms (dominators, cycles) used during compilation.

### Parity-critical invariants

Byte-identical output depends on **ordering and ID generation matching Python exactly**. Watch for these when editing:

- ID/UID generation order must match Python. `project_to_context` in `main.rs` deliberately adds lists → blocks → costumes in Python's `get_ctx` order.
- `serde_json` is configured with `preserve_order`; `indexmap` is used where insertion order is observable. Don't swap these for `HashMap`/`BTreeMap` in code paths that feed the output.
- Minification blanks variable/list display names to `""` (the name lives once in the top-level table) — see `field_display_name` in `scratch/mod.rs`. Match Python's `minify` conditionals precisely.
- Optimizer "don't remove" / "ignore external change" sets (return-value slots, stack pointer, jump-table var) are set up in `main.rs`; changing them changes which variables survive elision.

## Vendored dependencies

`vendor/llvm-ir/` and `vendor/llvm-ir-parser/` are patched forks of the LLVM IR crates, wired in via `[patch.crates-io]` in `Cargo.toml` (published as `llvm-in-rust-ir*`). Parser-level changes to LLVM syntax support go here. `tests/vm/vendor/` holds a vendored TurboWarp scratch-vm for headless execution tests.

Git submodules under `examples/demos/` (chibicc, micropython, newlib-scratch) are large real-world inputs and are not needed for normal development.

## Docs

`docs/*.md` contain per-module notes (`compiler.md`, `scratch.md`, `parser.md`, …) describing the Python reference modules — useful when reverse-engineering intended behavior before a Rust change.
