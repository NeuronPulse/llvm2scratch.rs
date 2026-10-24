#!/usr/bin/env python3
"""parity_classify.py — Python vs Rust 对拍 + 差异自动分类。

对每个 .ll 输入：
  1. 用完全相同的 flag 分别跑 Python（权威参考）与 Rust；
  2. 抽取 project.json 原始字节做严格比对；
  3. 不一致时解析 JSON 做结构化 diff，并把每条差异自动归类到
     已知的根因类别（见 CATEGORY_RULES），输出计数与样本。

目标：把产物差异收敛到零（uuid 等无害字段除外）。本工具既是门禁
（有未豁免差异即非零退出），也是收敛进度表（按类别计数）。

用法:
    python3 scripts/parity_classify.py [file_or_dir ...] [--json] [--show-diff]

类别（与 docs/parity-analysis.md 的根因编号对应）:
    TMP_NUMBERING    C1  临时变量编号分歧 (calculate_wide_sum_diff 探索期分配)
    PARAM_PREFIX     C2  参数名缺少 % 前缀 (GetParam / ProcedureDef params)
    EMPTY_FIELDS     C3  Op 块缺少 "fields": {}
    FOREACH_REPLACE  C5  --replace-hacked-blocks 时 for_each 未转 reptimes
    KNOWN_STR_NUM    C6  Known("Infinity") 字符串 vs 数值
    BLOCK_LABEL      C7  块标签/proc 名编号规则 (llvmlite 密集编号 vs 源标签)
    ENCODING_ONLY    C4  解析后一致但字节不同 (JSON 转义/数字格式)
    OTHER                 未分类，需人工归因

状态:
    MATCH       字节一致
    PY_FAIL     Python 编译失败（按项目约定跳过，不计失败）
    RS_FAIL     Rust 编译失败（门禁失败）
"""

import argparse
import difflib
import json
import re
import subprocess
import sys
import tempfile
import zipfile
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = ROOT / "target" / "release" / "llvm2scratch"
if not RUST_BIN.exists():
    RUST_BIN = ROOT / "target" / "debug" / "llvm2scratch"
TIMEOUT = 180

# 强制 minify + 关优化 + 替换 hacked 块 + 单目标：字节可比的前提。
COMMON_PY = ["-T", "scratch3", "--replace-hacked-blocks", "-O", "none", "-M", "general"]
COMMON_RS = ["-T", "scratch3", "--replace-hacked-blocks", "--no-optimize", "-M", "general"]

# 无害字段豁免：命中这些 path 的差异不计入门禁。
EXEMPT_PATH_RE = re.compile(r"(^|\.)(uuid|blockId)(\.|$)", re.I)

TMP_RE = re.compile(r"^%!tmp:\d+$")
UUID_RE = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", re.I)


def extract_project_json(sb3: Path) -> str | None:
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


def structural_diff(a, b, path="$", out=None):
    """递归结构化 diff，返回 [(path, py_value, rs_value)]。"""
    if out is None:
        out = []
    if type(a) is not type(b):
        out.append((path, a, b))
        return out
    if isinstance(a, dict):
        for k in a:
            if k not in b:
                out.append((f"{path}.{k}", a[k], "<MISSING>"))
        for k in b:
            if k not in a:
                out.append((f"{path}.{k}", "<MISSING>", b[k]))
        for k in a:
            if k in b:
                structural_diff(a[k], b[k], f"{path}.{k}", out)
    elif isinstance(a, list):
        if len(a) != len(b):
            out.append((path, a, b))
            return out
        for i, (x, y) in enumerate(zip(a, b)):
            structural_diff(x, y, f"{path}[{i}]", out)
    elif a != b:
        out.append((path, a, b))
    return out


def _unescape_equal(a: str, b: str) -> bool:
    """两条字符串是否仅差 JSON 转义（如 \\uff05 vs 原字符）。"""
    if a == b:
        return True
    # 嵌套 json.dumps 场景：两侧都是序列化后的 JSON 文本
    try:
        if json.loads(a) == json.loads(b):
            return True
    except Exception:
        pass
    # 裸转义场景：把 \uXXXX / \n / \t 等反转义后比较
    def unesc(s: str) -> str:
        try:
            return json.loads(f'"{s}"')
        except Exception:
            return s
    return unesc(a) == unesc(b)


def _param_prefix_only(a: str, b: str) -> bool:
    """序列化 JSON 数组字符串仅差 % 前缀（如 ["%a"] vs ["a"]）→ C2。"""
    try:
        la, lb = json.loads(a), json.loads(b)
    except Exception:
        return False
    if not isinstance(la, list) or not isinstance(lb, list) or len(la) != len(lb):
        return False
    for x, y in zip(la, lb):
        if x == y:
            continue
        if isinstance(x, str) and isinstance(y, str) and x.startswith("%") and x[1:] == y:
            continue
        return False
    return True


def classify(path: str, a, b, foreach_blocks: set[str] | None = None) -> str:
    """把一条结构化差异归入根因类别。

    foreach_blocks: opcode 为 control_repeat/control_for_each 之别的块 ID 集合，
    这些块下的所有差异都归因于 C5。
    """
    if EXEMPT_PATH_RE.search(path):
        return "EXEMPT"
    m = re.search(r"blocks\.([^.]+)\.", path)
    if foreach_blocks and m and m.group(1) in foreach_blocks:
        return "FOREACH_REPLACE"
    # C7: 标签/proc 名类字符串（respond:0:return vs respond:1:return、argumentnames 等）
    if isinstance(a, str) and isinstance(b, str):
        if UUID_RE.match(a) or UUID_RE.match(b):
            return "EXEMPT"
        if TMP_RE.match(a) and TMP_RE.match(b):
            return "TMP_NUMBERING"
        if a.startswith("%") and b == a[1:]:
            return "PARAM_PREFIX"
        if b.startswith("%") and a == b[1:]:
            return "PARAM_PREFIX"
        if _unescape_equal(a, b):
            return "ESCAPING"
        if _param_prefix_only(a, b):
            return "PARAM_PREFIX"
        return "BLOCK_LABEL"
    # C3: 空 fields 只在 Python 侧
    if isinstance(a, dict) and a == {} and b == "<MISSING>":
        return "EMPTY_FIELDS"
    if a == "<MISSING>" and isinstance(b, dict) and b == {}:
        return "EMPTY_FIELDS"
    if a == "<MISSING>" or b == "<MISSING>":
        # 一侧缺失的键：fields/inputs/opcode 组合差异
        if path.endswith(".fields") or path.endswith(".inputs") or path.endswith(".opcode"):
            return "BLOCK_LABEL"
        return "OTHER"
    # C6: 输入字面量类型 10(str) vs 4(num)
    if path.endswith("[1][0]") and a in (4, 10) and b in (4, 10) and a != b:
        return "KNOWN_STR_NUM"
    # C5: control_repeat vs control_for_each（opcode 差异）
    if path.endswith(".opcode") and isinstance(a, str) and isinstance(b, str):
        if {a, b} == {"control_repeat", "control_for_each"}:
            return "FOREACH_REPLACE"
        return "BLOCK_LABEL"
    return "OTHER"


CATEGORY_DESC = {
    "TMP_NUMBERING": "C1 临时变量编号分歧",
    "PARAM_PREFIX": "C2 参数名缺少 % 前缀",
    "EMPTY_FIELDS": "C3 Op 块缺少 fields:{}",
    "ESCAPING": "C4 非 ASCII/转义不一致（json ensure_ascii）",
    "FOREACH_REPLACE": "C5 for_each 未替换为 reptimes",
    "KNOWN_STR_NUM": "C6 Known 字符串/数值类型不一致",
    "BLOCK_LABEL": "C7 块标签/proc 名编号规则差异",
    "ENCODING_ONLY": "解析后一致但字节不同（整体序列化格式）",
    "OTHER": "未分类，需人工归因",
}


def compare_one(ll: Path, tmp: Path) -> dict:
    stem = ll.stem
    py_out, rs_out = tmp / f"{stem}_py.sb3", tmp / f"{stem}_rs.sb3"
    py_log, rs_log = tmp / f"{stem}_py.log", tmp / f"{stem}_rs.log"

    py_cmd = ["python", "-m", "llvm2scratch.cli", str(ll), "-o", str(py_out)] + COMMON_PY
    rs_cmd = [str(RUST_BIN), str(ll), str(rs_out)] + COMMON_RS

    py_rc = run(py_cmd, py_log)
    if py_rc != 0:
        return {"name": ll.name, "status": "PY_FAIL"}
    rs_rc = run(rs_cmd, rs_log)
    if rs_rc != 0:
        return {"name": ll.name, "status": "RS_FAIL",
                "detail": rs_log.read_text()[-300:]}

    py_json, rs_json = extract_project_json(py_out), extract_project_json(rs_out)
    if py_json is None or rs_json is None:
        return {"name": ll.name, "status": "EXTRACT_FAIL"}
    if py_json == rs_json:
        return {"name": ll.name, "status": "MATCH"}

    try:
        py_doc, rs_doc = json.loads(py_json), json.loads(rs_json)
    except Exception:
        py_doc = rs_doc = None

    if py_doc == rs_doc:
        return {"name": ll.name, "status": "DIFF", "cats": {"ENCODING_ONLY": 1},
                "samples": [("ENCODING_ONLY", "$", py_json[:80], rs_json[:80])]}

    diffs = structural_diff(py_doc, rs_doc)
    # 块级归因：先找 opcode 为 control_repeat/control_for_each 之别的块，
    # 这些块下的 inputs/fields 等差异全部归入 C5。
    foreach_blocks = {
        re.search(r"blocks\.([^.]+)\.opcode$", path).group(1)
        for path, a, b in diffs
        if path.endswith(".opcode")
        and isinstance(a, str) and isinstance(b, str)
        and {a, b} == {"control_repeat", "control_for_each"}
    }
    cats = Counter()
    samples = {}
    for path, a, b in diffs:
        c = classify(path, a, b, foreach_blocks)
        cats[c] += 1
        samples.setdefault(c, (path, a, b))
    return {"name": ll.name, "status": "DIFF", "cats": dict(cats),
            "samples": {k: v for k, v in samples.items()}}


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
    ap.add_argument("--json", action="store_true", help="machine-readable summary")
    ap.add_argument("--show-diff", action="store_true", help="print per-category samples")
    args = ap.parse_args()

    if not RUST_BIN.exists():
        print("ERROR: no rust binary; run cargo build --release", file=sys.stderr)
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
            if r["status"] == "MATCH":
                print(f"  [MATCH] {r['name']}", flush=True)
            elif r["status"] == "PY_FAIL":
                print(f"  [PY_FAIL] {r['name']} (skipped)", flush=True)
            elif r["status"] == "RS_FAIL":
                print(f"  [RS_FAIL] {r['name']}: {r.get('detail', '')[-120:]}", flush=True)
            else:
                cats = ", ".join(f"{k}={v}" for k, v in sorted(r["cats"].items()))
                print(f"  [DIFF] {r['name']}: {cats}", flush=True)
                if args.show_diff:
                    for c, sample in sorted(r.get("samples", {}).items()):
                        if isinstance(sample, tuple):
                            path, a, b = sample
                        else:
                            path, a, b = sample[0]
                        print(f"      [{c}] {path}")
                        print(f"        py={json.dumps(a, ensure_ascii=False)[:120]}")
                        print(f"        rs={json.dumps(b, ensure_ascii=False)[:120]}")

    grand = Counter()
    for r in results:
        if r["status"] == "DIFF":
            grand.update(r["cats"])
    n_match = sum(1 for r in results if r["status"] == "MATCH")
    n_pyfail = sum(1 for r in results if r["status"] == "PY_FAIL")
    n_rsfail = sum(1 for r in results if r["status"] == "RS_FAIL")
    n_diff = sum(1 for r in results if r["status"] == "DIFF")

    print(f"\nSummary: total={len(results)} match={n_match} diff={n_diff} "
          f"rs_fail={n_rsfail} py_fail={n_pyfail}")
    if grand:
        print("差异分类累计（收敛目标：全部为 0，EXEMPT 除外）:")
        for c, cnt in grand.most_common():
            print(f"  {cnt:5d}  {c}  ({CATEGORY_DESC.get(c, '?')})")

    if args.json:
        print(json.dumps({
            "results": [{k: v for k, v in r.items() if k != "samples"} for r in results],
            "grand_total": dict(grand),
        }, ensure_ascii=False))

    # 门禁：除 EXEMPT 外任何差异、任何 Rust 失败都算不通过。
    blocking = sum(cnt for c, cnt in grand.items() if c != "EXEMPT")
    return 0 if (n_diff == 0 and n_rsfail == 0) else 1


if __name__ == "__main__":
    sys.exit(main())
