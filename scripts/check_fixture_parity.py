#!/usr/bin/env python3
"""Compare Python vs Rust compiler output for parser fixtures.

Fixtures rarely define main(), so the first defined function is used as the
entrypoint. When both compilers succeed, the generated scratchblocks text is
normalized and compared for exact parity. Python failures (including emitted
L2S ERROR blocks) are reported separately and do not count as mismatches.
"""
import difflib
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = ROOT / "target" / "release" / "llvm2scratch"
FIXTURES_DIR = ROOT / "vendor" / "llvm-ir-parser" / "tests" / "fixtures"
PY_BIN = "python"


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


def get_entrypoint(ll_path: Path) -> str:
    with open(ll_path) as f:
        for line in f:
            m = re.search(r"@([A-Za-z0-9_]+)\s*\(", line)
            if m:
                return m.group(1)
    return "main"


def run_compiler(cmd: list, log: Path) -> int:
    with open(log, "w") as f:
        try:
            result = subprocess.run(
                cmd, cwd=ROOT, stdout=f, stderr=subprocess.STDOUT, timeout=30
            )
            return result.returncode
        except subprocess.TimeoutExpired:
            f.write("TIMEOUT\n")
            return -1


def main() -> int:
    if not RUST_BIN.exists():
        print(f"ERROR: Rust binary not found at {RUST_BIN}", file=sys.stderr)
        print("Run: cargo build --release", file=sys.stderr)
        return 1

    total = 0
    both_ok = 0
    mismatch = 0
    py_ok_rust_fail = 0
    py_fail_rust_ok = 0
    both_fail = 0

    with tempfile.TemporaryDirectory() as tmpdir:
        for ll in sorted(FIXTURES_DIR.glob("*.ll")):
            total += 1
            entrypoint = get_entrypoint(ll)
            py_out = Path(tmpdir) / f"{ll.stem}_py.sb3"
            rs_out = Path(tmpdir) / f"{ll.stem}_rs.sb3"
            py_sb = Path(tmpdir) / f"{ll.stem}_py.sb"
            rs_sb = Path(tmpdir) / f"{ll.stem}_rs.sb"
            py_log = Path(tmpdir) / f"{ll.stem}_py.log"
            rs_log = Path(tmpdir) / f"{ll.stem}_rs.log"

            py_cmd = [
                PY_BIN, "-m", "llvm2scratch.cli", str(ll), "-o", str(py_out),
                "--entrypoint", entrypoint, "--debug-scratchblocks", str(py_sb),
            ]
            rs_cmd = [
                str(RUST_BIN), str(ll), str(rs_out), "--entrypoint", entrypoint,
                "--debug-scratchblocks", str(rs_sb),
            ]

            py_code = run_compiler(py_cmd, py_log)
            rs_code = run_compiler(rs_cmd, rs_log)

            if py_code == 0 and rs_code != 0:
                py_ok_rust_fail += 1
                print(f"PY_OK_RS_FAIL: {ll.name}")
                for line in rs_log.read_text().splitlines()[:2]:
                    print(f"  {line}")
                continue

            if py_code != 0 and rs_code != 0:
                both_fail += 1
                continue

            if py_code != 0 and rs_code == 0:
                py_fail_rust_ok += 1
                print(f"PY_FAIL_RS_OK: {ll.name}")
                for line in py_log.read_text().splitlines()[:2]:
                    print(f"  PY: {line}")
                continue

            try:
                py_text = py_sb.read_text()
            except Exception:
                py_text = ""
            if "L2S ERROR" in py_text:
                py_fail_rust_ok += 1
                print(f"PY_FAIL_RS_OK: {ll.name} (Python emitted L2S ERROR block)")
                continue

            both_ok += 1
            try:
                py_norm = normalize(py_text)
                rs_norm = normalize(rs_sb.read_text())
            except Exception as e:
                print(f"READ_ERROR: {ll.name}: {e}")
                mismatch += 1
                continue

            if py_norm != rs_norm:
                mismatch += 1
                diff = list(
                    difflib.unified_diff(
                        py_norm.splitlines(),
                        rs_norm.splitlines(),
                        fromfile="python",
                        tofile="rust",
                        lineterm="",
                        n=2,
                    )
                )
                print(f"SCRATCHBLOCKS_MISMATCH: {ll.name} ({len(diff)} diff lines)")
                for line in diff[:40]:
                    print(line)

    print()
    print(
        f"Fixtures: total={total}, both_ok={both_ok}, mismatch={mismatch}, "
        f"py_ok_rust_fail={py_ok_rust_fail}, both_fail={both_fail}, "
        f"py_fail_rust_ok={py_fail_rust_ok}"
    )

    if mismatch > 0 or py_ok_rust_fail > 0:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
