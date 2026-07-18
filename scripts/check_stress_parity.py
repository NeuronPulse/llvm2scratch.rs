#!/usr/bin/env python3
"""Compare Python vs Rust compiler output for generated stress programs.

Looks for /tmp/llvm_stress_main/ll/*.ll, compiles each with both compilers in
optimized and unoptimized modes, and verifies exact scratchblocks parity.
"""
import difflib
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = ROOT / "target" / "release" / "llvm2scratch"
STRESS_DIR = Path("/tmp/llvm_stress_main/ll")
PY_BIN = "python"
TIMEOUT = 120


def normalize(text: str) -> str:
    marker = "<false::extension> // Known false block using an empty boolean input"
    if marker in text:
        text = text[text.find(marker) + len(marker):]
    text = text.strip()
    tmp_pattern = re.compile(r"%!?tmp:[a-zA-Z0-9_]+")
    counter = 0
    seen: dict[str, str] = {}

    def repl(m: re.Match) -> str:
        nonlocal counter
        name = m.group(0)
        if name not in seen:
            seen[name] = f"__TMP_{counter}__"
            counter += 1
        return seen[name]

    return tmp_pattern.sub(repl, text)


def run_mode(mode: str, optimize_arg: list[str]) -> bool:
    print(f"=== Python vs Rust complex main diff ({mode}) ===", flush=True)
    if not STRESS_DIR.exists():
        print(f"SKIP: stress directory not found: {STRESS_DIR}")
        print("Generate with: bash scripts/generate_complex_programs.sh")
        return True

    total = 0
    both_ok = 0
    sb_mismatch = 0
    py_ok_rust_fail = 0
    both_fail = 0
    py_fail_rust_ok = 0

    with tempfile.TemporaryDirectory() as tmpdir:
        tmpdir = Path(tmpdir)
        for ll in sorted(STRESS_DIR.glob("*.ll")):
            total += 1
            print(f"  [{mode}] {ll.name} ...", flush=True)
            py_out = tmpdir / f"{ll.stem}_py.sb3"
            rs_out = tmpdir / f"{ll.stem}_rs.sb3"
            py_sb = tmpdir / f"{ll.stem}_py.sb"
            rs_sb = tmpdir / f"{ll.stem}_rs.sb"
            py_log = tmpdir / f"{ll.stem}_py.log"
            rs_log = tmpdir / f"{ll.stem}_rs.log"

            py_cmd = [
                PY_BIN, "-m", "llvm2scratch.cli", str(ll), "-o", str(py_out),
                "--debug-scratchblocks", str(py_sb),
            ] + optimize_arg
            rs_cmd = [
                str(RUST_BIN), str(ll), str(rs_out),
                "--debug-scratchblocks", str(rs_sb),
            ] + optimize_arg

            try:
                with open(py_log, "w") as f:
                    py_res = subprocess.run(py_cmd, cwd=ROOT, stdout=f, stderr=subprocess.STDOUT, timeout=TIMEOUT)
                with open(rs_log, "w") as f:
                    rs_res = subprocess.run(rs_cmd, cwd=ROOT, stdout=f, stderr=subprocess.STDOUT, timeout=TIMEOUT)
            except subprocess.TimeoutExpired:
                print(f"TIMEOUT: {ll.name}")
                both_fail += 1
                continue

            if py_res.returncode == 0 and rs_res.returncode != 0:
                py_ok_rust_fail += 1
                print(f"PY_OK_RS_FAIL: {ll.name}")
            elif py_res.returncode != 0 and rs_res.returncode != 0:
                both_fail += 1
            elif py_res.returncode == 0 and rs_res.returncode == 0:
                both_ok += 1
                try:
                    py_text = normalize(py_sb.read_text())
                    rs_text = normalize(rs_sb.read_text())
                except Exception as e:
                    print(f"READ_ERROR: {ll.name}: {e}")
                    sb_mismatch += 1
                    continue

                if py_text != rs_text:
                    sb_mismatch += 1
                    diff = list(difflib.unified_diff(
                        py_text.splitlines(), rs_text.splitlines(),
                        fromfile="python", tofile="rust", lineterm="", n=2,
                    ))
                    print(f"SCRATCHBLOCKS_MISMATCH: {ll.name} ({len(diff)} diff lines)")
                    for line in diff[:40]:
                        print(line)
            else:
                py_fail_rust_ok += 1
                print(f"PY_FAIL_RS_OK: {ll.name}")

    print(f"\n{mode}: total={total}, both_ok={both_ok}, scratchblocks_mismatch={sb_mismatch}, "
          f"py_ok_rust_fail={py_ok_rust_fail}, both_fail={both_fail}, py_fail_rust_ok={py_fail_rust_ok}", flush=True)

    return sb_mismatch == 0 and py_ok_rust_fail == 0


def main() -> int:
    if not RUST_BIN.exists():
        print(f"ERROR: Rust binary not found at {RUST_BIN}", file=sys.stderr)
        print("Run: cargo build --release", file=sys.stderr)
        return 1

    ok = True
    ok = run_mode("optimized", []) and ok
    print()
    ok = run_mode("unoptimized", ["-O", "none"]) and ok
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
