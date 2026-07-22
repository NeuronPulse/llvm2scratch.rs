#!/usr/bin/env python3
"""strict_parity.py — 强制 minify 下 Python vs Rust project.json 逐字节对拍。

对 examples/input 里(或指定的).ll 文件,用完全相同的 flag(强制 minify、
关优化)分别跑 Python 和 Rust,抽出 project.json 原始字节比对。

用法:
    python scripts/strict_parity.py [file_or_dir ...] [--show-diff] [--json]

只有 project.json 原始字节完全相等才算 MATCH。Python 是权威参考:
Python 编译失败的用例跳过(标 PY_FAIL),不计入不通过。
"""
import argparse
import difflib
import json
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = ROOT / "target" / "release" / "llvm2scratch"
if not RUST_BIN.exists():
    RUST_BIN = ROOT / "target" / "debug" / "llvm2scratch"
TIMEOUT = 120

# 强制 minify(逐字节一致的前提)、关优化、替换 hacked 块、单精灵目标。
COMMON_PY = ["-T", "scratch3", "--replace-hacked-blocks", "-O", "none", "-M", "general"]
COMMON_RS = ["-T", "scratch3", "--replace-hacked-blocks", "--no-optimize", "-M", "general"]


def extract(sb3: Path) -> str | None:
    try:
        with zipfile.ZipFile(sb3) as z:
            for n in z.namelist():
                if n.endswith("project.json"):
                    return z.read(n).decode("utf-8")
    except Exception:
        return None
    return None


def run(cmd: list[str], log: Path) -> int:
    with open(log, "w") as f:
        try:
            return subprocess.run(cmd, cwd=ROOT, stdout=f, stderr=subprocess.STDOUT,
                                  timeout=TIMEOUT).returncode
        except subprocess.TimeoutExpired:
            f.write("\nTIMEOUT\n")
            return 124


def compare_one(ll: Path, tmp: Path) -> dict:
    stem = ll.stem
    py_out, rs_out = tmp / f"{stem}_py.sb3", tmp / f"{stem}_rs.sb3"
    py_log, rs_log = tmp / f"{stem}_py.log", tmp / f"{stem}_rs.log"

    py_cmd = ["python", "-m", "llvm2scratch.cli", str(ll), "-o", str(py_out)] + COMMON_PY
    rs_cmd = [str(RUST_BIN), str(ll), str(rs_out)] + COMMON_RS

    py_rc = run(py_cmd, py_log)
    if py_rc != 0:
        return {"name": ll.name, "status": "PY_FAIL", "detail": py_log.read_text()[-500:]}
    rs_rc = run(rs_cmd, rs_log)
    if rs_rc != 0:
        return {"name": ll.name, "status": "RS_FAIL", "detail": rs_log.read_text()[-500:]}

    py_json, rs_json = extract(py_out), extract(rs_out)
    if py_json is None:
        return {"name": ll.name, "status": "PY_EXTRACT_FAIL", "detail": ""}
    if rs_json is None:
        return {"name": ll.name, "status": "RS_EXTRACT_FAIL", "detail": ""}
    if py_json == rs_json:
        return {"name": ll.name, "status": "MATCH", "detail": "", "bytes": len(py_json)}

    # pretty-print 后 diff,便于阅读结构差异;同时记原始字节差异行数。
    try:
        py_pp = json.dumps(json.loads(py_json), indent=2, ensure_ascii=False).splitlines()
        rs_pp = json.dumps(json.loads(rs_json), indent=2, ensure_ascii=False).splitlines()
    except Exception:
        py_pp, rs_pp = py_json.splitlines(), rs_json.splitlines()
    diff = list(difflib.unified_diff(py_pp, rs_pp, "python", "rust", lineterm="", n=1))
    nchg = sum(1 for l in diff if l and l[0] in "+-" and not l.startswith(("+++", "---")))
    return {"name": ll.name, "status": "DIFF", "detail": f"{nchg} changed lines",
            "diff": "\n".join(diff), "py_bytes": len(py_json), "rs_bytes": len(rs_json)}


def collect(inputs: list[str]) -> list[Path]:
    files: list[Path] = []
    for a in inputs:
        p = Path(a)
        if p.is_dir():
            files.extend(sorted(p.glob("*.ll")))
        elif p.suffix == ".ll":
            files.append(p)
    return files


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("inputs", nargs="*", default=[str(ROOT / "examples" / "input")])
    ap.add_argument("--show-diff", action="store_true")
    ap.add_argument("--max-diff", type=int, default=60)
    ap.add_argument("--json", action="store_true", help="emit machine-readable summary")
    args = ap.parse_args()

    if not RUST_BIN.exists():
        print(f"ERROR: no rust binary at {RUST_BIN}; run cargo build --release", file=sys.stderr)
        return 1

    files = collect(args.inputs)
    if not files:
        print("No .ll files found.", file=sys.stderr)
        return 1

    results = []
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        for ll in files:
            r = compare_one(ll, tmp)
            results.append(r)
            mark = {"MATCH": "✅", "DIFF": "❌", "PY_FAIL": "⏭️ ",
                    "RS_FAIL": "💥", "PY_EXTRACT_FAIL": "❓", "RS_EXTRACT_FAIL": "❓"}.get(r["status"], "?")
            print(f"  {mark} [{r['status']}] {r['name']}: {r['detail'][:80]}", flush=True)
            if args.show_diff and r["status"] == "DIFF":
                for line in r["diff"].splitlines()[:args.max_diff]:
                    print(f"      {line}")

    n = len(results)
    match = sum(1 for r in results if r["status"] == "MATCH")
    diff = sum(1 for r in results if r["status"] == "DIFF")
    rs_fail = sum(1 for r in results if r["status"] == "RS_FAIL")
    py_fail = sum(1 for r in results if r["status"] == "PY_FAIL")
    print(f"\nSummary: total={n} match={match} diff={diff} rs_fail={rs_fail} py_fail={py_fail}")

    if args.json:
        print(json.dumps({"results": [{k: v for k, v in r.items() if k != "diff"}
                                       for r in results]}, ensure_ascii=False))
    # 通过条件:除 Python 自身不支持的用例外,全部 MATCH。
    return 0 if diff == 0 and rs_fail == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
