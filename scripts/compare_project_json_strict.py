#!/usr/bin/env python3
"""Strict byte-for-byte comparison of Python vs Rust project.json output.

For each candidate .ll file, compile with both compilers using identical flags,
extract Project/project.json from the resulting .sb3, and compare raw UTF-8
strings.  Two outputs count as a match only if they are byte-for-byte identical.

Cases where the Python compiler fails are ignored, per project convention that
Python is the authoritative reference and unsupported inputs are out of scope.
"""

import argparse
import difflib
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = ROOT / "target" / "release" / "llvm2scratch"
PY_BIN = "python"
TIMEOUT = 120


def extract_project_json(sb3_path: Path) -> str | None:
    """Return the raw project.json string from an .sb3 archive, or None."""
    try:
        with zipfile.ZipFile(sb3_path) as z:
            for name in z.namelist():
                if name.endswith("project.json"):
                    with z.open(name) as f:
                        return f.read().decode("utf-8")
    except Exception:
        return None
    return None


def run_compiler(cmd: list[str], log: Path) -> subprocess.CompletedProcess:
    with open(log, "w") as f:
        return subprocess.run(
            cmd,
            cwd=ROOT,
            stdout=f,
            stderr=subprocess.STDOUT,
            timeout=TIMEOUT,
        )


def compare_one(
    ll_path: Path,
    tmpdir: Path,
    optimize: bool,
) -> tuple[str, str, str | None]:
    """Compile one .ll file and return (status, detail, diff_or_none)."""
    stem = ll_path.stem
    py_out = tmpdir / f"{stem}_py.sb3"
    rs_out = tmpdir / f"{stem}_rs.sb3"
    py_log = tmpdir / f"{stem}_py.log"
    rs_log = tmpdir / f"{stem}_rs.log"

    optimize_arg_py = ["-O", "none"] if not optimize else []
    optimize_arg_rs = ["--no-optimize"] if not optimize else []

    common_args = ["-T", "scratch3", "--replace-hacked-blocks"]

    py_cmd = [
        PY_BIN,
        "-m",
        "llvm2scratch.cli",
        str(ll_path),
        "-o",
        str(py_out),
    ] + common_args + optimize_arg_py

    rs_cmd = [
        str(RUST_BIN),
        str(ll_path),
        str(rs_out),
    ] + common_args + optimize_arg_rs

    py_res = run_compiler(py_cmd, py_log)
    rs_res = run_compiler(rs_cmd, rs_log)

    if py_res.returncode != 0:
        return "PY_FAIL", f"Python failed (see {py_log})", None
    if rs_res.returncode != 0:
        return "RS_FAIL", f"Rust failed (see {rs_log})", None

    py_json = extract_project_json(py_out)
    rs_json = extract_project_json(rs_out)

    if py_json is None:
        return "PY_EXTRACT_FAIL", "Could not extract Python project.json", None
    if rs_json is None:
        return "RS_EXTRACT_FAIL", "Could not extract Rust project.json", None

    if py_json == rs_json:
        return "MATCH", "byte-for-byte match", None

    diff = list(
        difflib.unified_diff(
            py_json.splitlines(),
            rs_json.splitlines(),
            fromfile="python",
            tofile="rust",
            lineterm="",
            n=2,
        )
    )
    return "DIFF", f"project.json differs ({len(diff)} diff lines)", "\n".join(diff)


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Strict byte-for-byte Python vs Rust project.json comparison."
    )
    parser.add_argument(
        "inputs",
        nargs="+",
        help=".ll files or directories containing .ll files to compare",
    )
    parser.add_argument(
        "--unoptimized",
        action="store_true",
        help="Disable optimizations (default: optimized)",
    )
    parser.add_argument(
        "--show-diff",
        action="store_true",
        help="Print unified diff for each mismatch",
    )
    args = parser.parse_args()

    if not RUST_BIN.exists():
        print(f"ERROR: Rust binary not found at {RUST_BIN}", file=sys.stderr)
        print("Run: cargo build --release", file=sys.stderr)
        return 1

    ll_files: list[Path] = []
    for arg in args.inputs:
        p = Path(arg)
        if p.is_dir():
            ll_files.extend(sorted(p.glob("*.ll")))
        elif p.suffix == ".ll":
            ll_files.append(p)
        else:
            print(f"WARNING: skipping non-.ll argument {p}", file=sys.stderr)

    if not ll_files:
        print("No .ll files found.", file=sys.stderr)
        return 1

    optimize = not args.unoptimized
    mode_label = "optimized" if optimize else "unoptimized"
    print(f"=== Strict project.json comparison ({mode_label}) ===", flush=True)

    stats = {
        "total": 0,
        "match": 0,
        "diff": 0,
        "py_fail": 0,
        "rs_fail": 0,
    }

    with tempfile.TemporaryDirectory() as tmpdir:
        tmpdir = Path(tmpdir)
        for ll in ll_files:
            stats["total"] += 1
            status, detail, diff = compare_one(ll, tmpdir, optimize)
            print(f"  [{status}] {ll.name}: {detail}", flush=True)
            if status == "DIFF" and args.show_diff and diff:
                for line in diff.splitlines()[:80]:
                    print(f"    {line}")
                if len(diff.splitlines()) > 80:
                    print(f"    ... ({len(diff.splitlines())} total diff lines)")
            if status == "MATCH":
                stats["match"] += 1
            elif status == "DIFF":
                stats["diff"] += 1
            elif status == "PY_FAIL":
                stats["py_fail"] += 1
            elif status == "RS_FAIL":
                stats["rs_fail"] += 1

    print(f"\nSummary ({mode_label}):", flush=True)
    print(f"  total={stats['total']}, match={stats['match']}, diff={stats['diff']}, "
          f"py_fail={stats['py_fail']}, rs_fail={stats['rs_fail']}", flush=True)

    return 0 if stats["diff"] == 0 and stats["rs_fail"] == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
