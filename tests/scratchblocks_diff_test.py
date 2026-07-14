#!/usr/bin/env python3
"""
Differential testing for scratchblocks text output: compile the same LLVM IR
with both Python and Rust versions, then compare the scratchblocks text output.

Usage:
    python3 tests/scratchblocks_diff_test.py [test_file_or_dir ...]

If no arguments given, tests all .ll files under examples/input/.
"""

import sys
import os
import tempfile
import subprocess
import shutil
from pathlib import Path
from dataclasses import dataclass, field

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = ROOT / "target" / "release" / "llvm2scratch"
LL_INPUTS = ROOT / "examples" / "input"


@dataclass
class DiffResult:
    name: str
    py_ok: bool = False
    rs_ok: bool = False
    equivalent: bool = False
    py_error: str = ""
    rs_error: str = ""
    diff_details: list = field(default_factory=list)


def compile_python_scratchblocks(ll_path: str, memory_size: int = 4096) -> tuple:
    """Compile with Python version and return scratchblocks text."""
    try:
        old_cwd = os.getcwd()
        os.chdir(str(ROOT))
        if str(ROOT) not in sys.path:
            sys.path.insert(0, str(ROOT))
        from llvm2scratch.compiler import compile as py_compile, Config
        from llvm2scratch import scratch, target

        opt_target = target.getTarget(target.DEFAULT_OPT_TARGET)
        cfg = Config(
            memory_size=memory_size,
            compiler_opt=False,
            compiler_minify=False,
            opt_passes=set(),
            opt_target=opt_target,
            use_branch_jump_table=opt_target.exec.preferred_branch_method == target.BranchMethod.JumpTable,
        )
        with open(ll_path) as f:
            proj = py_compile(f.read(), cfg)
        text = proj.stringify(scratchblocks=True)
        os.chdir(old_cwd)
        return True, text, ""
    except Exception as e:
        os.chdir(old_cwd)
        import traceback
        return False, "", traceback.format_exc()


def compile_rust_scratchblocks(ll_path: str, memory_size: int = 4096) -> tuple:
    """Compile with Rust version and return scratchblocks text."""
    try:
        with tempfile.TemporaryDirectory() as tmpdir:
            sb3_path = os.path.join(tmpdir, "out.sb3")
            sb_path = os.path.join(tmpdir, "out.sb3.txt")
            result = subprocess.run(
                [
                    str(RUST_BIN), ll_path, sb3_path,
                    "--debug-scratchblocks", sb_path,
                    "--no-optimize", "-m", str(memory_size),
                ],
                capture_output=True, text=True, timeout=60,
            )
            if result.returncode != 0:
                return False, "", result.stderr + result.stdout
            with open(sb_path) as f:
                text = f.read()
            return True, text, ""
    except subprocess.TimeoutExpired:
        return False, "", "Timeout"
    except Exception as e:
        return False, "", str(e)


def normalize_text(text: str) -> str:
    """Normalize scratchblocks text for comparison.

    Removes the fixed header so that only the generated code is compared, and
    renames temporary variables to placeholders because the Python reference
    uses random names while Rust uses deterministic counters.
    """
    # Header ends with the last comment line describing known false block.
    marker = "<false::extension> // Known false block using an empty boolean input"
    if marker in text:
        idx = text.find(marker)
        text = text[idx + len(marker):]
    text = text.strip()

    import re
    tmp_pattern = re.compile(r"%!?tmp:[a-zA-Z0-9]+")
    counter = 0
    seen: dict[str, str] = {}

    def replace_tmp(match: re.Match) -> str:
        nonlocal counter
        name = match.group(0)
        if name not in seen:
            seen[name] = f"__TMP_{counter}__"
            counter += 1
        return seen[name]

    return tmp_pattern.sub(replace_tmp, text)


def compare_texts(py_text: str, rs_text: str) -> list:
    """Compare two scratchblocks texts line-by-line."""
    py_norm = normalize_text(py_text)
    rs_norm = normalize_text(rs_text)

    diffs = []
    py_lines = py_norm.splitlines()
    rs_lines = rs_norm.splitlines()

    if py_lines != rs_lines:
        # Simple diff summary
        import difflib
        diff = list(difflib.unified_diff(py_lines, rs_lines, lineterm="", n=2))
        diffs.extend(diff[:50])  # Limit diff size
        if len(diff) > 50:
            diffs.append(f"... and {len(diff) - 50} more diff lines")

    return diffs


def run_diff_test(ll_path: str) -> DiffResult:
    """Run a single differential test."""
    name = Path(ll_path).stem
    result = DiffResult(name=name)

    py_ok, py_text, py_err = compile_python_scratchblocks(ll_path)
    result.py_ok = py_ok
    result.py_error = py_err

    rs_ok, rs_text, rs_err = compile_rust_scratchblocks(ll_path)
    result.rs_ok = rs_ok
    result.rs_error = rs_err

    if py_ok and rs_ok:
        diffs = compare_texts(py_text, rs_text)
        result.diff_details = diffs
        result.equivalent = len(diffs) == 0
    elif py_ok and not rs_ok:
        result.diff_details.append(f"Rust compilation failed: {rs_err[:200]}")
    elif not py_ok and rs_ok:
        result.diff_details.append(f"Python compilation failed: {py_err[:200]}")
    else:
        result.diff_details.append(f"Both failed: Python={py_err[:100]}, Rust={rs_err[:100]}")

    return result


def main():
    if not RUST_BIN.exists():
        print(f"ERROR: Rust binary not found at {RUST_BIN}")
        print("Run: cargo build --release")
        sys.exit(1)

    # Files that cannot be diff-tested because the Python reference does not
    # support the same instructions (e.g. aggregate.ll with shufflevector and
    # llvm.sadd.with.overflow.i32).
    SKIP_FILES = {"aggregate.ll"}

    results = []

    # Test .ll files from examples/input/
    ll_files = []
    if len(sys.argv) > 1:
        for arg in sys.argv[1:]:
            p = Path(arg)
            if p.is_dir():
                ll_files.extend(sorted(p.glob("*.ll")))
            elif p.is_file() and p.suffix == ".ll":
                ll_files.append(p)
    else:
        ll_files = sorted(LL_INPUTS.glob("*.ll"))

    for ll_path in ll_files:
        if ll_path.name in SKIP_FILES:
            continue
        result = run_diff_test(str(ll_path))
        results.append(result)

    # Print results
    passed = [r for r in results if r.equivalent]
    failed = [r for r in results if not r.equivalent]

    print(f"\nResults: {len(passed)}/{len(results)} passed\n")
    for r in passed:
        print(f"  PASS  {r.name}")
    for r in failed:
        print(f"  FAIL  {r.name}")
        for detail in r.diff_details[:10]:
            print(f"        {detail}")
        if len(r.diff_details) > 10:
            print(f"        ... and {len(r.diff_details) - 10} more lines")

    if failed:
        sys.exit(1)


if __name__ == "__main__":
    main()
