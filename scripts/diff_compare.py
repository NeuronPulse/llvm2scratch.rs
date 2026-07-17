#!/usr/bin/env python3
import json
import sys
import zipfile

def load_project(path):
    path = str(path)
    if path.endswith('.sb3'):
        with zipfile.ZipFile(path) as z:
            for name in z.namelist():
                if name.endswith('project.json'):
                    with z.open(name) as f:
                        return json.load(f)
        raise ValueError(f"No project.json found in {path}")
    else:
        with open(path) as f:
            return json.load(f)

def count_top_level_blocks(blocks, start_id):
    count = 0
    current_id = start_id
    while current_id and current_id in blocks:
        block = blocks[current_id]
        if block is None:
            break
        opcode = block.get("opcode", "")
        if opcode == "procedures_prototype":
            current_id = block.get("next")
            continue
        count += 1
        current_id = block.get("next")
    return count

def get_proc_name(blocks, def_block):
    cb = def_block.get("inputs", {}).get("custom_block", {})
    if isinstance(cb, list) and len(cb) > 1:
        proto_id = cb[1]
        if isinstance(proto_id, str) and proto_id in blocks:
            proto = blocks[proto_id]
            if proto and proto.get("opcode") == "procedures_prototype":
                return proto.get("mutation", {}).get("proccode", "unknown")
    next_id = def_block.get("next")
    if next_id and next_id in blocks:
        nb = blocks[next_id]
        if nb and nb.get("opcode") == "procedures_prototype":
            return nb.get("mutation", {}).get("proccode", "unknown")
    return "unknown"

def extract_procedures(targets):
    procs = {}
    for target in targets:
        blocks = target.get("blocks", {})
        for block_id, block in blocks.items():
            if block is None:
                continue
            opcode = block.get("opcode", "")
            if opcode == "procedures_definition":
                proc_name = get_proc_name(blocks, block)
                next_id = block.get("next")
                count = count_top_level_blocks(blocks, next_id)
                procs[proc_name] = count
    return procs

py_path = sys.argv[1] if len(sys.argv) > 1 else "/tmp/py_sb3/Project/project.json"
rs_path = sys.argv[2] if len(sys.argv) > 2 else "/tmp/rs_sb3/Project/project.json"

py_data = load_project(py_path)
rs_data = load_project(rs_path)

py_procs = extract_procedures(py_data.get("targets", []))
rs_procs = extract_procedures(rs_data.get("targets", []))

all_procs = sorted(set(list(py_procs.keys()) + list(rs_procs.keys())))

header = "{:<55} {:>8} {:>8} {:>8}".format("Procedure", "Python", "Rust", "Diff")
print(header)
print("-" * len(header))

total_diff = 0
for proc in all_procs:
    py_count = py_procs.get(proc, -1)
    rs_count = rs_procs.get(proc, -1)
    diff = rs_count - py_count if py_count >= 0 and rs_count >= 0 else 0
    total_diff += diff
    diff_str = "{:+d}".format(diff) if diff != 0 else "+0"
    marker = " <<<" if diff != 0 else " OK"
    print("{:<55} {:>8} {:>8} {:>8}{}".format(proc, py_count, rs_count, diff_str, marker))

print("\nTotal diff: {:+d}".format(total_diff))