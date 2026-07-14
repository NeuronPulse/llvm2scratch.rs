#!/usr/bin/env python3
"""
Differential testing for scratchblocks text output on the generated complex
programs under /tmp/llvm_stress_main/ll/.

These programs are compiled with optimizations disabled so that the Rust output
matches the Python reference exactly.
"""

import sys
import os
import tempfile
import subprocess
import difflib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = ROOT / "target" / "release" / "llvm2scratch"
STRESS_DIR = Path("/tmp/llvm_stress_main/ll")


def normalize_text(text: str) -> str:
    marker = "<false::extension> // Known false block using an empty boolean input"
    if marker in text:
        text = text[text.find(marker) + len(marker):]
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


def compile_python_scratchblocks(ll_path: Path, memory_size: int = 4096) -> tuple:
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
    except Exception:
        os.chdir(old_cwd)
        import traceback
        return False, "", traceback.format_exc()


def compile_rust_scratchblocks(ll_path: Path, memory_size: int = 4096) -> tuple:
    try:
        with tempfile.TemporaryDirectory() as tmpdir:
            sb3_path = os.path.join(tmpdir, "out.sb3")
            sb_path = os.path.join(tmpdir, "out.sb3.txt")
            result = subprocess.run(
                [
                    str(RUST_BIN), str(ll_path), sb3_path,
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


def run_test(ll_path: Path) -> tuple:
    name = ll_path.stem
    py_ok, py_text, py_err = compile_python_scratchblocks(ll_path)
    rs_ok, rs_text, rs_err = compile_rust_scratchblocks(ll_path)

    if not py_ok:
        return False, f"Python failed: {py_err[:200]}"
    if not rs_ok:
        return False, f"Rust failed: {rs_err[:200]}"

    py_norm = normalize_text(py_text)
    rs_norm = normalize_text(rs_text)
    if py_norm == rs_norm:
        return True, ""

    diff = list(difflib.unified_diff(
        py_norm.splitlines(), rs_norm.splitlines(),
        fromfile="python", tofile="rust", lineterm="", n=2,
    ))
    return False, f"{len(diff)} diff lines\n" + "\n".join(diff[:40])


def main():
    if not RUST_BIN.exists():
        print(f"ERROR: Rust binary not found at {RUST_BIN}")
        print("Run: cargo build --release")
        sys.exit(1)

    if not STRESS_DIR.exists():
        print(f"Stress test directory not found: {STRESS_DIR}")
        print("Generate complex programs first.")
        sys.exit(0)

    ll_files = sorted(STRESS_DIR.glob("*.ll"))
    passed = []
    failed = []
    for ll_path in ll_files:
        ok, msg = run_test(ll_path)
        if ok:
            passed.append(ll_path.stem)
        else:
            failed.append((ll_path.stem, msg))

    print(f"\nResults: {len(passed)}/{len(ll_files)} passed\n")
    for name in passed:
        print(f"  PASS  {name}")
    for name, msg in failed:
        print(f"  FAIL  {name}")
        for line in msg.splitlines()[:10]:
            print(f"        {line}")
        if len(msg.splitlines()) > 10:
            print(f"        ... and {len(msg.splitlines()) - 10} more lines")

    if failed:
        sys.exit(1)


if __name__ == "__main__":
    main()
