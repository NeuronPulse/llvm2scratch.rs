#!/usr/bin/env python3
"""
Differential testing: compile the same LLVM IR with both Python and Rust versions,
then compare the output .sb3 files for structural equivalence.

Usage:
    python3 tests/diff_test.py [test_file_or_dir ...]

If no arguments given, tests all .ll files under examples/input/.
"""

import sys
import os
import json
import zipfile
import tempfile
import subprocess
import shutil
from pathlib import Path
from dataclasses import dataclass, field
from typing import Optional

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = ROOT / "target" / "debug" / "llvm2scratch"
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

def normalize_blocks(blocks: dict) -> dict:
    """Normalize blocks by removing UIDs and cosmetic differences."""
    normalized = {}
    for uid, block in blocks.items():
        if isinstance(block, dict):
            nb = {}
            for k, v in block.items():
                if k in ("opcode", "next", "parent"):
                    nb[k] = v
                elif k == "inputs":
                    nb[k] = normalize_inputs(v)
                elif k == "fields":
                    nb[k] = v
                else:
                    nb[k] = v
            normalized[uid] = nb
        else:
            normalized[uid] = block
    return normalized

def normalize_inputs(inputs: dict) -> dict:
    normalized = {}
    for k, v in inputs.items():
        if isinstance(v, list):
            nv = []
            for item in v:
                if isinstance(item, str) and len(item) > 20:
                    nv.append("<UID>")
                else:
                    nv.append(item)
            normalized[k] = nv
        else:
            normalized[k] = v
    return normalized

def extract_sb3_structure(sb3_path: str) -> Optional[dict]:
    """Extract the structural content from an .sb3 file for comparison."""
    try:
        with zipfile.ZipFile(sb3_path) as z:
            pj_names = [n for n in z.namelist() if n.endswith("project.json")]
            if not pj_names:
                return None
            with z.open(pj_names[0]) as pj:
                data = json.load(pj)
    except Exception:
        return None

    structure = {
        "targets": [],
    }

    for target in data.get("targets", []):
        t_info = {
            "isStage": target.get("isStage"),
            "name": target.get("name"),
            "variables": {},
            "lists": {},
            "blocks_count": len(target.get("blocks", {})),
            "blocks_opcodes": [],
        }

        for var_name, var_data in target.get("variables", {}).items():
            if isinstance(var_data, list) and len(var_data) >= 2:
                t_info["variables"][str(var_data[1])] = var_data[0]
            else:
                t_info["variables"][str(var_name)] = var_data

        for list_name, list_data in target.get("lists", {}).items():
            if isinstance(list_data, list) and len(list_data) >= 2:
                # Scratch lists format: [name, values]
                key = str(list_data[0]) if list_data[0] is not None else str(list_name)
                t_info["lists"][key] = list_data[1]
            else:
                t_info["lists"][str(list_name)] = list_data

        opcodes = []
        for block_uid, block in target.get("blocks", {}).items():
            if isinstance(block, dict):
                opcodes.append(block.get("opcode", "unknown"))
        opcodes.sort()
        t_info["blocks_opcodes"] = opcodes

        structure["targets"].append(t_info)

    return structure

def compare_structures(py_struct: dict, rs_struct: dict) -> list:
    """Compare two sb3 structures and return list of differences."""
    diffs = []

    py_targets = [t for t in py_struct["targets"] if not t["isStage"]]
    rs_targets = [t for t in rs_struct["targets"] if not t["isStage"]]

    if len(py_targets) != len(rs_targets):
        diffs.append(f"Target count: Python={len(py_targets)}, Rust={len(rs_targets)}")
        return diffs

    for i, (pt, rt) in enumerate(zip(py_targets, rs_targets)):
        prefix = f"Target[{i}]"

        if pt["name"] != rt["name"]:
            diffs.append(f"{prefix} name: Python={pt['name']}, Rust={rt['name']}")

        if pt["blocks_count"] != rt["blocks_count"]:
            diffs.append(f"{prefix} blocks_count: Python={pt['blocks_count']}, Rust={rt['blocks_count']}")

        py_ops = pt["blocks_opcodes"]
        rs_ops = rt["blocks_opcodes"]
        if py_ops != rs_ops:
            py_op_counts = {}
            rs_op_counts = {}
            for op in py_ops:
                py_op_counts[op] = py_op_counts.get(op, 0) + 1
            for op in rs_ops:
                rs_op_counts[op] = rs_op_counts.get(op, 0) + 1
            all_ops = sorted(set(list(py_op_counts.keys()) + list(rs_op_counts.keys())))
            for op in all_ops:
                pc = py_op_counts.get(op, 0)
                rc = rs_op_counts.get(op, 0)
                if pc != rc:
                    diffs.append(f"{prefix} opcode '{op}' count: Python={pc}, Rust={rc}")

        py_vars = set(pt["variables"].keys())
        rs_vars = set(rt["variables"].keys())
        if py_vars != rs_vars:
            only_py = py_vars - rs_vars
            only_rs = rs_vars - py_vars
            if only_py:
                diffs.append(f"{prefix} variables only in Python: {sorted(only_py)}")
            if only_rs:
                diffs.append(f"{prefix} variables only in Rust: {sorted(only_rs)}")

        py_lists = set(pt["lists"].keys())
        rs_lists = set(rt["lists"].keys())
        if py_lists != rs_lists:
            only_py = py_lists - rs_lists
            only_rs = rs_lists - py_lists
            if only_py:
                diffs.append(f"{prefix} lists only in Python: {sorted(only_py)}")
            if only_rs:
                diffs.append(f"{prefix} lists only in Rust: {sorted(only_rs)}")

    return diffs

def compile_python(ll_path: str, out_path: str, memory_size: int = 4096) -> tuple:
    """Compile with Python version. Returns (success, error_msg)."""
    try:
        old_cwd = os.getcwd()
        os.chdir(str(ROOT))
        if str(ROOT) not in sys.path:
            sys.path.insert(0, str(ROOT))
        from llvm2scratch.compiler import compile as py_compile, Config
        from llvm2scratch import scratch

        cfg = Config(
            memory_size=memory_size,
            compiler_opt=False,
            compiler_minify=False,
            opt_passes=set(),
        )
        with open(ll_path) as f:
            proj = py_compile(f.read(), cfg)
        proj.export(out_path, scratch.Format.Project3)
        os.chdir(old_cwd)
        return True, ""
    except Exception as e:
        os.chdir(old_cwd)
        import traceback
        return False, traceback.format_exc()

def compile_rust(ll_path: str, out_path: str, memory_size: int = 4096) -> tuple:
    """Compile with Rust version. Returns (success, error_msg)."""
    try:
        result = subprocess.run(
            [str(RUST_BIN), ll_path, out_path, "--no-optimize", "-m", str(memory_size)],
            capture_output=True, text=True, timeout=30
        )
        if result.returncode == 0:
            return True, ""
        else:
            return False, result.stderr + result.stdout
    except subprocess.TimeoutExpired:
        return False, "Timeout"
    except Exception as e:
        return False, str(e)

def run_diff_test(ll_path: str) -> DiffResult:
    """Run a single differential test."""
    name = Path(ll_path).stem
    result = DiffResult(name=name)

    with tempfile.TemporaryDirectory() as tmpdir:
        py_out = os.path.join(tmpdir, "py_out.sb3")
        rs_out = os.path.join(tmpdir, "rs_out.sb3")

        py_ok, py_err = compile_python(ll_path, py_out)
        result.py_ok = py_ok
        result.py_error = py_err

        rs_ok, rs_err = compile_rust(ll_path, rs_out)
        result.rs_ok = rs_ok
        result.rs_error = rs_err

        if py_ok and rs_ok:
            py_struct = extract_sb3_structure(py_out)
            rs_struct = extract_sb3_structure(rs_out)

            if py_struct is None:
                result.diff_details.append("Failed to extract Python sb3 structure")
            if rs_struct is None:
                result.diff_details.append("Failed to extract Rust sb3 structure")

            if py_struct and rs_struct:
                diffs = compare_structures(py_struct, rs_struct)
                result.diff_details = diffs
                result.equivalent = len(diffs) == 0
        elif py_ok and not rs_ok:
            result.diff_details.append(f"Rust compilation failed: {rs_err[:200]}")
        elif not py_ok and rs_ok:
            result.diff_details.append(f"Python compilation failed: {py_err[:200]}")
        else:
            result.diff_details.append(f"Both failed: Python={py_err[:100]}, Rust={rs_err[:100]}")

    return result

def generate_simple_ll_tests() -> list:
    """Generate simple LLVM IR test cases programmatically."""
    tests = []

    tests.append(("simple_ret0", """\
define i32 @main() {
  ret i32 0
}
"""))

    tests.append(("simple_ret42", """\
define i32 @main() {
  ret i32 42
}
"""))

    tests.append(("simple_add", """\
define i32 @main() {
  %result = add i32 20, 22
  ret i32 %result
}
"""))

    tests.append(("simple_sub", """\
define i32 @main() {
  %result = sub i32 100, 58
  ret i32 %result
}
"""))

    tests.append(("simple_mul", """\
define i32 @main() {
  %result = mul i32 6, 7
  ret i32 %result
}
"""))

    tests.append(("simple_local_var", """\
define i32 @main() {
  %x = alloca i32
  store i32 10, ptr %x
  %val = load i32, ptr %x
  ret i32 %val
}
"""))

    tests.append(("simple_param", """\
define i32 @add(i32 %a, i32 %b) {
  %result = add i32 %a, %b
  ret i32 %result
}

define i32 @main() {
  %result = call i32 @add(i32 3, i32 4)
  ret i32 %result
}
"""))

    tests.append(("simple_global", """\
@my_global = global i32 42

define i32 @main() {
  %val = load i32, ptr @my_global
  ret i32 %val
}
"""))

    tests.append(("simple_gep_array", """\
define i32 @main() {
  %arr = alloca [10 x i32]
  %ptr = getelementptr [10 x i32], ptr %arr, i32 0, i32 5
  store i32 99, ptr %ptr
  %val = load i32, ptr %ptr
  ret i32 %val
}
"""))

    tests.append(("simple_if", """\
define i32 @main() {
  %cond = icmp sgt i32 10, 5
  br i1 %cond, label %then, label %else

then:
  br label %end

else:
  br label %end

end:
  ret i32 0
}
"""))

    tests.append(("simple_loop", """\
define i32 @main() {
entry:
  br label %loop

loop:
  %i = phi i32 [ 0, %entry ], [ %next, %loop ]
  %next = add i32 %i, 1
  %cond = icmp slt i32 %next, 10
  br i1 %cond, label %loop, label %exit

exit:
  ret i32 %next
}
"""))

    tests.append(("simple_struct_alloca", """\
%Pair = type { i32, i32 }

define i32 @main() {
  %p = alloca %Pair
  %first = getelementptr %Pair, ptr %p, i32 0, i32 0
  %second = getelementptr %Pair, ptr %p, i32 0, i32 1
  store i32 10, ptr %first
  store i32 20, ptr %second
  %v1 = load i32, ptr %first
  %v2 = load i32, ptr %second
  %sum = add i32 %v1, %v2
  ret i32 %sum
}
"""))

    tests.append(("simple_ptr_ops", """\
define i32 @main() {
  %p1 = alloca i32
  %p2 = alloca i32
  store i32 100, ptr %p1
  %v = load i32, ptr %p1
  store i32 %v, ptr %p2
  %result = load i32, ptr %p2
  ret i32 %result
}
"""))

    tests.append(("simple_neg", """\
define i32 @main() {
  %result = sub i32 0, 42
  ret i32 %result
}
"""))

    tests.append(("simple_and_or", """\
define i32 @main() {
  %a = and i32 255, 15
  %b = or i32 240, 15
  %result = add i32 %a, %b
  ret i32 %result
}
"""))

    return tests

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
            elif p.suffix == ".ll":
                ll_files.append(p)
    else:
        ll_files = sorted(LL_INPUTS.glob("*.ll"))

    # Remove skipped files unless they were explicitly requested as a file.
    explicit_files = {
        Path(arg).name for arg in sys.argv[1:]
        if Path(arg).is_file() and Path(arg).suffix == ".ll"
    }
    ll_files = [
        f for f in ll_files
        if f.name not in SKIP_FILES or f.name in explicit_files
    ]

    print(f"=== Differential Testing: Python vs Rust ===\n")
    print(f"Rust binary: {RUST_BIN}")
    print(f"Test files: {len(ll_files)} .ll files + generated tests\n")

    # Test existing .ll files
    for ll_file in ll_files:
        print(f"Testing {ll_file.name}...", end=" ", flush=True)
        r = run_diff_test(str(ll_file))
        results.append(r)
        if r.equivalent:
            print("✅ EQUIVALENT")
        elif r.py_ok and r.rs_ok:
            print(f"⚠️  DIFFERENT ({len(r.diff_details)} diffs)")
        elif not r.py_ok and not r.rs_ok:
            print("❌ BOTH FAILED")
        elif not r.py_ok:
            print("⚠️  PYTHON ONLY FAILED")
        else:
            print("⚠️  RUST ONLY FAILED")

    # Test generated simple cases
    print(f"\n--- Generated simple tests ---\n")
    generated = generate_simple_ll_tests()
    for name, ir in generated:
        print(f"Testing {name}...", end=" ", flush=True)
        with tempfile.NamedTemporaryFile(mode='w', suffix='.ll', delete=False) as f:
            f.write(ir)
            f.flush()
            ll_path = f.name
        try:
            r = run_diff_test(ll_path)
            r.name = name
            results.append(r)
            if r.equivalent:
                print("✅ EQUIVALENT")
            elif r.py_ok and r.rs_ok:
                print(f"⚠️  DIFFERENT ({len(r.diff_details)} diffs)")
            elif not r.py_ok and not r.rs_ok:
                print("❌ BOTH FAILED")
            elif not r.py_ok:
                print("⚠️  PYTHON ONLY FAILED")
            else:
                print("⚠️  RUST ONLY FAILED")
        finally:
            os.unlink(ll_path)

    # Summary
    print(f"\n{'='*60}")
    print(f"SUMMARY")
    print(f"{'='*60}")

    total = len(results)
    equivalent = sum(1 for r in results if r.equivalent)
    both_ok_diff = sum(1 for r in results if r.py_ok and r.rs_ok and not r.equivalent)
    py_fail = sum(1 for r in results if not r.py_ok)
    rs_fail = sum(1 for r in results if not r.rs_ok)
    both_fail = sum(1 for r in results if not r.py_ok and not r.rs_ok)

    print(f"Total tests:     {total}")
    print(f"Equivalent:      {equivalent} ✅")
    print(f"Different:       {both_ok_diff} ⚠️")
    print(f"Python failed:   {py_fail}")
    print(f"Rust failed:     {rs_fail}")
    print(f"Both failed:     {both_fail} ❌")

    # Show details for non-equivalent results
    if both_ok_diff > 0 or rs_fail > 0:
        print(f"\n{'='*60}")
        print(f"DETAILED DIFFERENCES")
        print(f"{'='*60}")
        for r in results:
            if r.equivalent:
                continue
            if not r.py_ok and not r.rs_ok:
                continue
            print(f"\n--- {r.name} ---")
            for d in r.diff_details:
                print(f"  {d}")
            if not r.py_ok:
                print(f"  Python error: {r.py_error[:2000]}")
            if not r.rs_ok:
                print(f"  Rust error: {r.rs_error[:2000]}")

    return 0 if equivalent == total else 1

if __name__ == "__main__":
    sys.exit(main())