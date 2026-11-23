#!/usr/bin/env python3
"""Byte-level Rust/C++ differential for the pure numeric corpus.

Both drivers receive the exact same corpus bytes from one in-memory read. For
each file the check compares stdout byte for byte and the exit-status class
(success versus failure). A missing driver, missing corpus or any mismatch
fails closed. No numeric tolerance is applied: the contracts specify exact
equality. D11 retired the order-book corpus; only pure numeric vectors remain.

Run with explicit driver paths so the gate cannot silently select a stale
binary from a previous build.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path

NUMERIC_FILES = ("parse.txt", "format.txt", "rescale.txt", "invalid.txt")


def run_driver(binary: str, payload: bytes) -> tuple[int, bytes]:
    """Run one driver on the payload and return its exit code and stdout."""
    result = subprocess.run(
        [binary],
        input=payload,
        capture_output=True,
        check=False,
        timeout=60,
    )
    return result.returncode, result.stdout


def compare(
    corpus: Path,
    pairs: tuple[tuple[str, str, str, tuple[str, ...]], ...],
) -> dict:
    """Compare each corpus file byte for byte between the two drivers."""
    report: dict = {"schema_version": 2, "cases": {}, "gate_passed": True}
    for label, cpp_binary, rust_binary, names in pairs:
        for binary in (cpp_binary, rust_binary):
            if not Path(binary).is_file():
                raise RuntimeError(f"{label}: driver missing: {binary}")
        for name in names:
            path = corpus / label / name
            if not path.is_file():
                raise RuntimeError(f"{label}: corpus file missing: {path}")
            payload = path.read_bytes()
            case_id = f"{label}/{name}"
            cpp_code, cpp_out = run_driver(cpp_binary, payload)
            rust_code, rust_out = run_driver(rust_binary, payload)
            same_class = (cpp_code == 0) == (rust_code == 0)
            same_output = cpp_out == rust_out
            passed = same_class and same_output
            report["cases"][case_id] = {
                "input_sha256": hashlib.sha256(payload).hexdigest(),
                "input_bytes": len(payload),
                "cpp_exit": cpp_code,
                "rust_exit": rust_code,
                "stdout_sha256": hashlib.sha256(cpp_out).hexdigest(),
                "stdout_bytes": len(cpp_out),
                "exit_class_identical": same_class,
                "stdout_identical": same_output,
                "passed": passed,
            }
            if not passed:
                report["gate_passed"] = False
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", required=True, type=Path)
    parser.add_argument("--cpp-numeric", required=True)
    parser.add_argument("--rust-numeric", required=True)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    pairs = (
        ("numeric", args.cpp_numeric, args.rust_numeric, NUMERIC_FILES),
    )
    try:
        report = compare(args.corpus, pairs)
    except (RuntimeError, OSError, subprocess.SubprocessError) as error:
        print(f"differential gate FAIL: {error}", file=sys.stderr)
        return 2
    text = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(text, encoding="utf-8")
    print(text)
    return 0 if report["gate_passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
