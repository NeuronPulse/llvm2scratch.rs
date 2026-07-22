#!/usr/bin/env python3
"""
VM execution tests for llvm2scratch compiled Scratch projects.

For each fixture pair in tests/vm/fixtures/ (<name>.ll + <name>.json):
  1. Compile the .ll to .sb3 with the Rust llvm2scratch binary.
  2. Run the .sb3 headlessly using TurboWarp/scratch-vm.
  3. Compare final variables/lists and execution trace against the expected JSON.

Usage:
    python3 tests/vm_execution_test.py [fixture_name_or_path ...]
"""

import json
import math
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = ROOT / "target" / "release" / "llvm2scratch"
VM_DIR = ROOT / "tests" / "vm"
RUNNER = VM_DIR / "run_sb3.js"
FIXTURES_DIR = VM_DIR / "fixtures"

DEFAULT_COMPILE_ARGS = ["-T", "scratch3", "--replace-hacked-blocks"]
DEFAULT_TIMEOUT_MS = 5000
DEFAULT_TRACE_SAMPLE_MS = 50
FLOAT_TOL = 1e-9


class VmTestResult:
    def __init__(self, name: str):
        self.name = name
        self.ok = False
        self.errors: list[str] = []
        self.actual: dict | None = None


def is_close(a, b, tol=FLOAT_TOL):
    """Compare two numbers with a small tolerance."""
    if isinstance(a, bool) or isinstance(b, bool):
        return a == b
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        if math.isnan(a) and math.isnan(b):
            return True
        if math.isinf(a) and math.isinf(b):
            return (a > 0) == (b > 0)
        return abs(a - b) <= tol
    return a == b


def match_value(actual, expected, path: str, errors: list[str]) -> bool:
    """Recursively compare an actual value against an expected value."""
    if isinstance(expected, list):
        if not isinstance(actual, list):
            errors.append(f"{path}: expected list, got {type(actual).__name__}")
            return False
        if len(actual) != len(expected):
            errors.append(f"{path}: list length mismatch: expected {len(expected)}, got {len(actual)}")
            return False
        ok = True
        for i, (ea, ee) in enumerate(zip(actual, expected)):
            if not match_value(ea, ee, f"{path}[{i}]", errors):
                ok = False
        return ok
    if isinstance(expected, dict):
        if not isinstance(actual, dict):
            errors.append(f"{path}: expected object, got {type(actual).__name__}")
            return False
        ok = True
        for key, ee in expected.items():
            if key not in actual:
                errors.append(f"{path}: missing key {key!r}")
                ok = False
            elif not match_value(actual[key], ee, f"{path}.{key}", errors):
                ok = False
        return ok
    if not is_close(actual, expected):
        errors.append(f"{path}: expected {expected!r}, got {actual!r}")
        return False
    return True


def compile_fixture(ll_path: Path, sb3_path: Path, compile_args: list[str]) -> tuple[bool, str]:
    """Compile an LLVM IR fixture to .sb3 using the Rust binary."""
    cmd = [str(RUST_BIN), str(ll_path), str(sb3_path), *compile_args]
    try:
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=120,
            cwd=str(ROOT),
        )
        if result.returncode != 0:
            return False, f"Command: {' '.join(cmd)}\n{result.stderr}{result.stdout}"
        return True, ""
    except subprocess.TimeoutExpired:
        return False, "Compilation timeout"
    except Exception as e:
        return False, str(e)


def run_sb3(sb3_path: Path, options: dict) -> tuple[bool, dict | str]:
    """Run a compiled .sb3 through the headless VM runner."""
    cmd = ["node", str(RUNNER), str(sb3_path)]
    try:
        result = subprocess.run(
            cmd,
            input=json.dumps(options),
            capture_output=True,
            text=True,
            timeout=(options.get("timeout_ms", DEFAULT_TIMEOUT_MS) / 1000.0) + 10,
        )
        if result.returncode != 0:
            return False, f"Runner error:\n{result.stderr}{result.stdout}"
        try:
            return True, json.loads(result.stdout)
        except json.JSONDecodeError as e:
            return False, f"Invalid JSON from runner: {e}\n{result.stdout[:500]}"
    except subprocess.TimeoutExpired:
        return False, "Runner timeout"
    except Exception as e:
        return False, str(e)


def check_trace(trace: list[dict], assertions: list[dict], errors: list[str]):
    """Evaluate trace assertions against the recorded execution trace."""
    for idx, assertion in enumerate(assertions):
        kind = assertion.get("type")
        if kind == "contains":
            expected_vars = assertion.get("variables", {})
            expected_lists = assertion.get("lists", {})
            found = False
            for snap in trace:
                snap_errs = []
                vars_ok = match_value(snap.get("variables", {}), expected_vars, "variables", snap_errs)
                lists_ok = match_value(snap.get("lists", {}), expected_lists, "lists", snap_errs)
                if vars_ok and lists_ok:
                    found = True
                    break
            if not found:
                errors.append(f"trace[{idx}] contains assertion failed: {json.dumps(assertion, ensure_ascii=False)}")
        elif kind == "eventually":
            expected_vars = assertion.get("variables", {})
            expected_lists = assertion.get("lists", {})
            if not trace:
                errors.append(f"trace[{idx}] eventually assertion failed: trace is empty")
                continue
            snap = trace[-1]
            snap_errs = []
            vars_ok = match_value(snap.get("variables", {}), expected_vars, "variables", snap_errs)
            lists_ok = match_value(snap.get("lists", {}), expected_lists, "lists", snap_errs)
            if not (vars_ok and lists_ok):
                errors.extend([f"trace[{idx}] eventually: {e}" for e in snap_errs])
        elif kind == "monotonic":
            var_name = assertion.get("variable")
            direction = assertion.get("direction", "increasing")
            values = [snap.get("variables", {}).get(var_name) for snap in trace if var_name in snap.get("variables", {})]
            if len(values) < 2:
                errors.append(f"trace[{idx}] monotonic assertion needs at least two samples for {var_name!r}")
                continue
            for i in range(1, len(values)):
                if direction == "increasing" and values[i] < values[i - 1]:
                    errors.append(f"trace[{idx}] monotonic increasing failed for {var_name!r} at step {i}: {values[i - 1]} -> {values[i]}")
                    break
                elif direction == "decreasing" and values[i] > values[i - 1]:
                    errors.append(f"trace[{idx}] monotonic decreasing failed for {var_name!r} at step {i}: {values[i - 1]} -> {values[i]}")
                    break
        else:
            errors.append(f"trace[{idx}] unknown assertion type: {kind!r}")


def run_fixture(fixture_path: Path) -> VmTestResult:
    """Run a single fixture and return the result."""
    name = fixture_path.stem
    result = VmTestResult(name)
    ll_path = fixture_path.with_suffix(".ll")
    json_path = fixture_path.with_suffix(".json")

    if not ll_path.exists():
        result.errors.append(f"Missing {ll_path}")
        return result
    if not json_path.exists():
        result.errors.append(f"Missing {json_path}")
        return result

    with open(json_path, encoding="utf-8") as f:
        spec = json.load(f)

    compile_args = DEFAULT_COMPILE_ARGS + spec.get("compile_args", [])
    timeout_ms = spec.get("timeout_ms", DEFAULT_TIMEOUT_MS)
    trace_sample_ms = spec.get("trace_sample_ms", DEFAULT_TRACE_SAMPLE_MS)
    expected = spec.get("expected", {})

    # Determine which variables/lists to trace. Default to only the ones
    # mentioned in expectations to keep output small.
    trace_vars = set(expected.get("variables", {}).keys())
    trace_lists = set(expected.get("lists", {}).keys())
    for assertion in expected.get("trace", []):
        trace_vars.update(assertion.get("variables", {}).keys())
        trace_lists.update(assertion.get("lists", {}).keys())

    runner_options = {
        "timeout_ms": timeout_ms,
        "trace_sample_ms": trace_sample_ms,
    }
    # Always pass trace_vars and trace_lists (even when empty) so the runner
    # filters out large lists like "!ASCII lookup" that would overflow stdout.
    # When empty, the runner returns no variables/lists of that category,
    # which is fine since we only check what's in the expected section.
    # However, always include "!return value" so we can check the final result
    # even when the fixture doesn't explicitly list it.
    trace_vars.add("!return value")
    runner_options["trace_vars"] = sorted(trace_vars)
    runner_options["trace_lists"] = sorted(trace_lists)

    with tempfile.TemporaryDirectory(prefix=f"vm_test_{name}_") as tmpdir:
        sb3_path = Path(tmpdir) / f"{name}.sb3"
        ok, err = compile_fixture(ll_path, sb3_path, compile_args)
        if not ok:
            result.errors.append(f"Compilation failed: {err}")
            return result

        ok, output = run_sb3(sb3_path, runner_options)
        if not ok:
            result.errors.append(f"VM execution failed: {output}")
            return result
        assert isinstance(output, dict)

        result.actual = output
        if output.get("timeout"):
            result.errors.append("VM execution timed out")

        final = output.get("final", {})
        match_value(final.get("variables", {}), expected.get("variables", {}), "final.variables", result.errors)
        match_value(final.get("lists", {}), expected.get("lists", {}), "final.lists", result.errors)

        trace = output.get("trace", [])
        check_trace(trace, expected.get("trace", []), result.errors)

    if not result.errors:
        result.ok = True
    return result


def discover_fixtures() -> list[Path]:
    """Discover all fixture stems in tests/vm/fixtures/."""
    fixtures = []
    if not FIXTURES_DIR.exists():
        return fixtures
    for json_path in sorted(FIXTURES_DIR.glob("*.json")):
        ll_path = json_path.with_suffix(".ll")
        if ll_path.exists():
            fixtures.append(json_path.with_suffix(""))
    return fixtures


def main():
    if not RUST_BIN.exists():
        print(f"ERROR: Rust binary not found at {RUST_BIN}")
        print("Run: cargo build --release")
        sys.exit(1)
    if not RUNNER.exists():
        print(f"ERROR: VM runner not found at {RUNNER}")
        sys.exit(1)

    if len(sys.argv) > 1:
        fixture_paths = []
        for arg in sys.argv[1:]:
            p = Path(arg)
            if p.is_dir():
                for json_path in sorted(p.glob("*.json")):
                    ll_path = json_path.with_suffix(".ll")
                    if ll_path.exists():
                        fixture_paths.append(json_path.with_suffix(""))
            else:
                fixture_paths.append(p.with_suffix(""))
    else:
        fixture_paths = discover_fixtures()

    if not fixture_paths:
        print("No fixtures found.")
        sys.exit(0)

    results = [run_fixture(fp) for fp in fixture_paths]
    passed = [r for r in results if r.ok]
    failed = [r for r in results if not r.ok]

    print(f"\nVM Execution Results: {len(passed)}/{len(results)} passed\n")
    for r in passed:
        print(f"  PASS  {r.name}")
    for r in failed:
        print(f"  FAIL  {r.name}")
        for err in r.errors:
            print(f"        {err}")
        if r.actual is not None:
            print(f"        actual final: {json.dumps(r.actual.get('final'), ensure_ascii=False)}")

    if failed:
        sys.exit(1)


if __name__ == "__main__":
    main()
