# Tests

Two layers, both treating the **Python implementation as ground truth**:

1. **Differential / output-parity tests** — compile the same input with both
   implementations and compare the output. These catch porting regressions.
2. **VM execution tests** — actually run the *Rust-compiled* `.sb3` in a headless
   Scratch VM and assert observables. These catch behaviour regressions that
   byte-parity cannot see (parity can be "correct" while both sides are wrong).

Run everything with `bash scripts/run_diff_tests.sh` (see `scripts/README.md`).

## Differential tests

| File | What it compares | Inputs |
|---|---|---|
| `sb3_diff_test.py` | Full `.sb3` project structure | `examples/input/*.ll` |
| `scratchblocks_diff_test.py` | Readable scratchblocks text | `examples/input/*.ll` |
| `vm_execution_test.py` | Runtime variables / lists of the generated project | `vm/fixtures/*.ll` |

Notes:

- These use **normalized** comparison (they tolerate some formatting), so they can
  go green on differences that matter. The authoritative byte-level check is
  `scripts/check_byte_parity.py` — judge parity by that, not by these.
- Inputs that the Python reference cannot compile are **skipped** (`PY_FAIL`), not
  reported as Rust failures.

## VM execution tests

Layout:

```
tests/vm_execution_test.py   # driver: compile → run in VM → compare against expectations
tests/vm/run_sb3.js          # headless runner (Node, vendored TurboWarp scratch-vm)
tests/vm/fixtures/<name>.ll  # input program
tests/vm/fixtures/<name>.json# compile flags + expected variables/lists/trace
tests/vm/vendor/             # scratch-vm checkout, ignored by git
```

Commands:

```bash
python3 tests/vm_execution_test.py                          # all fixtures
python3 tests/vm_execution_test.py tests/vm/fixtures/gcd_vm # one fixture (no extension)
bash scripts/run_vm_tests.sh                                # fetch vendor deps + run all
```

A fixture `.json` understands:

| Key | Meaning |
|---|---|
| `compile_args` | Extra args passed to the Rust binary (fixtures pin `--no-optimize` unless they mean to test the optimizer) |
| `timeout_ms` | VM execution budget |
| `trace_sample_ms` | Sampling interval for the execution trace |
| `expected.variables` / `expected.lists` | Exact final state assertions |
| `expected.trace` | Ordered assertions of type `eventually` / `always` / `never` |
| `notes` (optional) | Free text explaining non-obvious expectations — put here anything a future reader would trip over |

> When adding a fixture, derive the expectations from the **Python** reference when the
> two implementations disagree — Python is authoritative. Say so in `notes` (see
> `preseed_stack_vm.json` for a worked example).

## Coverage notes

Covered by fixtures: arithmetic, loops, recursion, switch, aggregates/structs, arrays and
pointer arithmetic, `setjmp`, list IO, memory read/write, **floating point**
(`float_math_vm`), **unsigned semantics** (`unsigned_ops_vm`), **wide i64**
(`wide_bitwise_vm`), plus the demo-only switches `--preseed-stack` and progress vars.

Known gaps — do not assume these are covered anywhere:

- **Rust is ahead of the reference in places.** Any input Python cannot compile is
  skipped by the differential and byte-parity checks (`PY_FAIL`), so those paths are
  untested against ground truth. `wide_bitwise_vm` is the extreme case: Python fails on
  the entire i64 lowering path, so that fixture only guards Rust behaviour.
- **`-M gen-lut-runtime`** is unimplemented in Rust *and* crashes the Python reference —
  there is currently no test for it at all.
