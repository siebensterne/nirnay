#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Independent, stdlib-only verifier for the corrected benchmark evidence.

The artifact under test retains the ordered raw observation vector of every
measurement stage. Nothing is inferred from histograms and nothing is
recomputed from a live run: this checker is JSON-only.

DIGEST INPUT FORMAT (binding, identical to ``benches/production_components.rs``):
SHA-256 over the UTF-8 bytes of newline-delimited lines of the exact form
``stage_id|repetition_index|process_index|observation_ns_integer``. Each line
is terminated by one LF, including the last; there is no trailing blank line.
Lines are ordered by process index, then by within-process repetition index,
then by observation order within that repetition. Histograms and quantiles are
never digest inputs.

Quantiles use lower-nearest-rank: sort pooled observations ascending, then
take the element at zero-based ``min(n - 1, floor(fraction * n))``. Tail
quantiles are provisional evidence, never a gate.

The ``provenance_binding`` object additionally declares the revision whose
packaged files were measured. Normal mode recomputes the tree, harness,
verifier, Cargo.lock, binary, workload and retained-observation digests from
those packaged files and from the artifact itself; a missing git object or a
drifted digest is a failure, never a warning.

Run ``--self-test`` for positive and negative adversarial cases. Exit status
is zero only when every declared digest, count, quantile, binding and
correctness check matches the retained observations.

In ``--artifact`` mode the expected measured revision defaults to the
historical corrected-baseline revision (``MEASUREMENT_REVISION``) so that
existing artifacts keep verifying unchanged. ``--measured-revision`` supplies
that expectation explicitly for a later final-head measurement; it changes
only which revision is expected in the provenance binding, never any other
check. Every packaged file digest is still recomputed from the revision the
artifact itself declares via read-only ``git show``.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import statistics
import subprocess
import tempfile
from pathlib import Path
from typing import Any

BASE_SHA = "360c80711f6a10174f60787f6f08c89df51aca85"
DEFAULT_ARTIFACT = Path(
    "evidence/performance/neutral-production-corrected-baseline-2026-11-28.json"
)
FRACTIONS: dict[str, float] = {
    "p50": 0.50,
    "p90": 0.90,
    "p95": 0.95,
    "p99": 0.99,
    "p999": 0.999,
}
STAGE_IDS = frozenset(
    {
        "risk_evaluate_risk",
        "oms_fence",
        "oms_create_fenced",
        "oms_accept_risk_approved_fenced",
        "oms_claim_fenced",
        "oms_mark_dispatching_fenced",
        "oms_acknowledge_fenced",
        "oms_apply_typed_fill_fenced",
        "journal_fence",
        "journal_append_fenced",
        "paper_submit_decision",
        "paper_reconcile",
        "replay_recovery",
        "pipeline_total",
    }
)
SMALL_SAMPLE_THRESHOLD = 100
ROUND_DIGITS = 6
PROCESS_COUNT = 5
MEASUREMENT_REVISION = "bd6d7bbdee89b2abde80b5a27453ab2b311b9c7e"
BRANCH_BASE_REVISION = "9e1231d7e213be23498de11dfcb3419f7d0fc4e1"
PROVENANCE_KEY = "provenance_binding"
PROVENANCE_SCHEMA = "nirnay-benchmark-evidence-provenance-v1"
PACKAGED_PATHS: dict[str, str] = {
    "benchmark_harness_digest": "benches/production_components.rs",
    "benchmark_verifier_digest": "scripts/verify_benchmark_evidence.py",
    "cargo_lock_digest": "Cargo.lock",
}
SELF_TEST_PACKAGED_FILES: dict[str, bytes] = {
    path: f"fixture:{path}\n".encode() for path in PACKAGED_PATHS.values()
}
SELF_TEST_TREE = "b" * 40


def reject_duplicate_keys(pairs: list[tuple[str, object]]) -> dict[str, object]:
    """Decode a JSON object while rejecting duplicate keys fail-closed."""
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def load_artifact(path: Path) -> dict[str, object]:
    """Load one JSON artifact as an object, or raise a descriptive error."""
    text = path.read_text(encoding="utf-8")
    payload = json.loads(text, object_pairs_hook=reject_duplicate_keys)
    if not isinstance(payload, dict):
        raise ValueError(f"{path}: top-level JSON value must be an object")
    return payload


def lower_nearest_rank(values: list[int], fraction: float) -> int:
    """Return the documented lower-nearest-rank quantile of integer samples."""
    if not values:
        raise ValueError("cannot quantile an empty observation vector")
    ordered = sorted(values)
    index = min(len(ordered) - 1, math.floor(fraction * len(ordered)))
    return ordered[index]


def digest_observations(stage_id: str, groups: list[dict[str, Any]]) -> str:
    """Hash the declared ordered representation for one stage."""
    digest = hashlib.sha256()
    ordered = sorted(
        groups, key=lambda group: (group["process_index"], group["repetition_index"])
    )
    for group in ordered:
        process_index = group["process_index"]
        repetition_index = group["repetition_index"]
        for value in group["values_ns"]:
            line = f"{stage_id}|{repetition_index}|{process_index}|{value}\n"
            digest.update(line.encode("utf-8"))
    return digest.hexdigest()


def is_integer(value: object) -> bool:
    """Return whether a decoded JSON value is an integer but not a boolean."""
    return isinstance(value, int) and not isinstance(value, bool)


def is_sha256_hex(value: object) -> bool:
    """Return whether a value is a 64-character lowercase SHA-256 digest."""
    return (
        isinstance(value, str)
        and len(value) == 64
        and all(character in "0123456789abcdef" for character in value)
    )


def is_git_object_id(value: object) -> bool:
    """Return whether a value is a 40-character lowercase git object id."""
    return (
        isinstance(value, str)
        and len(value) == 40
        and all(character in "0123456789abcdef" for character in value)
    )


def aggregate_digest(entries: list[tuple[str, str]]) -> str:
    """Hash newline-delimited ``left|right`` lines in their canonical order."""
    return hashlib.sha256(
        "".join(f"{left}|{right}\n" for left, right in entries).encode("utf-8")
    ).hexdigest()


def measured_revision_files(revision: object, root: Path) -> tuple[dict[str, bytes], str]:
    """Read packaged files and the tree digest at a declared revision via git."""
    if not is_git_object_id(revision):
        return {}, ""
    files: dict[str, bytes] = {}

    def git_bytes(*arguments: str) -> bytes:
        """Run one read-only git command and return its stdout bytes."""
        result = subprocess.run(
            ["git", *arguments],
            cwd=root,
            capture_output=True,
            check=False,
        )
        if result.returncode != 0:
            message = result.stderr.decode("utf-8", errors="replace").strip()
            raise ValueError(f"git {' '.join(arguments)} failed: {message}")
        return result.stdout

    for path in PACKAGED_PATHS.values():
        files[path] = git_bytes("show", f"{revision}:{path}")
    tree = git_bytes("rev-parse", f"{revision}^{{tree}}").decode("ascii").strip()
    return files, tree


def verify_provenance(
    payload: dict[str, object],
    revision_files: dict[str, bytes],
    revision_tree: str,
    expected_revision: str = MEASUREMENT_REVISION,
) -> list[str]:
    """Recompute all declared provenance digests from revision-packaged inputs.

    ``expected_revision`` is the revision the artifact must declare as measured.
    It defaults to the historical corrected-baseline revision; the CLI flag
    ``--measured-revision`` overrides only this expectation.
    """
    errors: list[str] = []
    binding = payload.get(PROVENANCE_KEY)
    if not isinstance(binding, dict):
        return [f"{PROVENANCE_KEY}: missing evidence schema object"]
    if binding.get("schema") != PROVENANCE_SCHEMA:
        errors.append("provenance schema identifier mismatch")
    if binding.get("production_source_revision") != expected_revision:
        errors.append(
            "production_source_revision is not the measured revision "
            f"(expected {expected_revision})"
        )
    disclosure = binding.get("measurement_revision_vs_base")
    if (
        not isinstance(disclosure, str)
        or expected_revision not in disclosure
        or BRANCH_BASE_REVISION not in disclosure
        or "not measured" not in disclosure.lower()
    ):
        errors.append("measurement_revision_vs_base does not disclose unmeasured source")

    if not is_git_object_id(binding.get("production_source_tree_digest")):
        errors.append("production_source_tree_digest is not a git tree object id")
    elif binding["production_source_tree_digest"] != revision_tree:
        errors.append(
            "production_source_tree_digest mismatch (git revision tree "
            f"{revision_tree})"
        )

    for key, path in PACKAGED_PATHS.items():
        declared = binding.get(key)
        if not is_sha256_hex(declared):
            errors.append(f"{key} is not a SHA-256 digest")
            continue
        content = revision_files.get(path)
        if content is None:
            errors.append(f"{key}: packaged file {path} at measured revision is absent")
            continue
        recomputed = hashlib.sha256(content).hexdigest()
        if declared != recomputed:
            errors.append(f"{key} mismatch (recomputed {recomputed})")

    retained = payload.get("harness_source_sha256")
    if not isinstance(retained, dict):
        errors.append("harness_source_sha256 is missing for cross-binding")
    else:
        for key, path in (
            ("benchmark_harness_digest", "benches/production_components.rs"),
            ("benchmark_verifier_digest", "scripts/verify_benchmark_evidence.py"),
        ):
            if retained.get(path) != binding.get(key):
                errors.append(f"{key} does not match retained harness_source_sha256")

    environment = payload.get("environment")
    if not isinstance(environment, dict):
        errors.append("environment is missing for provenance comparison")
        environment = {}
    rustc = binding.get("rustc_version")
    toolchain = environment.get("toolchain")
    if (
        not isinstance(rustc, str)
        or not rustc.startswith("rustc ")
        or not isinstance(toolchain, str)
        or not toolchain.startswith(rustc)
    ):
        errors.append("rustc_version is not reflected in environment.toolchain")
    for key in ("target_triple", "build_profile", "compiler_flags"):
        if not isinstance(binding.get(key), str) or binding[key] != environment.get(key):
            errors.append(f"{key} does not match environment.{key}")

    binary = binding.get("benchmark_binary_digest")
    if not is_sha256_hex(binary):
        errors.append("benchmark_binary_digest is not a SHA-256 digest")
    workload_entries: list[tuple[int, str]] = []
    processes = payload.get("processes")
    if not isinstance(processes, list):
        errors.append("processes are missing for provenance comparison")
        processes = []
    for index, process in enumerate(processes):
        if not isinstance(process, dict):
            continue
        if process.get("binary_digest_sha256") != binary:
            errors.append(f"process[{index}]: binary_digest_sha256 drift")
        workload = process.get("workload_digest_sha256")
        process_index = process.get("process_index")
        if not is_sha256_hex(workload) or not is_integer(process_index):
            errors.append(f"process[{index}]: malformed workload digest binding")
        else:
            workload_entries.append((process_index, workload))

    workload_digest = binding.get("workload_digest")
    if not is_sha256_hex(workload_digest):
        errors.append("workload_digest is not a SHA-256 digest")
    else:
        recomputed = aggregate_digest(
            [(str(index), digest) for index, digest in sorted(workload_entries)]
        )
        if workload_digest != recomputed:
            errors.append(f"workload_digest mismatch (recomputed {recomputed})")

    raw_entries: list[tuple[str, str]] = []
    stages = payload.get("stages")
    if not isinstance(stages, list):
        errors.append("stages are missing for provenance comparison")
        stages = []
    for stage in stages:
        if not isinstance(stage, dict):
            continue
        stage_id = stage.get("stage_id")
        stage_digest = stage.get("raw_observations_digest_sha256")
        if not isinstance(stage_id, str) or not is_sha256_hex(stage_digest):
            errors.append("stage raw_observations_digest_sha256 is malformed")
        else:
            raw_entries.append((stage_id, stage_digest))
    raw_digest = binding.get("raw_observation_digest")
    if not is_sha256_hex(raw_digest):
        errors.append("raw_observation_digest is not a SHA-256 digest")
    else:
        recomputed = aggregate_digest(sorted(raw_entries))
        if raw_digest != recomputed:
            errors.append(f"raw_observation_digest mismatch (recomputed {recomputed})")
    return errors


def verify_stage(stage: dict[str, object], process_count: int) -> list[str]:
    """Recompute every digest, count, quantile and spread of one stage."""
    errors: list[str] = []
    stage_id = stage.get("stage_id")
    if not isinstance(stage_id, str) or stage_id not in STAGE_IDS:
        return [f"unknown or malformed stage_id: {stage_id!r}"]

    groups = stage.get("observations")
    if not isinstance(groups, list) or not groups:
        return [f"{stage_id}: missing ordered observations"]

    by_process: dict[int, list[int]] = {}
    for group in groups:
        if not isinstance(group, dict):
            errors.append(f"{stage_id}: observation group is not an object")
            continue
        process_index = group.get("process_index")
        repetition_index = group.get("repetition_index")
        values = group.get("values_ns")
        if not is_integer(process_index) or not is_integer(repetition_index):
            errors.append(f"{stage_id}: non-integer process/repetition index")
            continue
        if not isinstance(values, list) or not values:
            errors.append(f"{stage_id}: empty observation vector")
            continue
        if not all(is_integer(value) and value >= 0 for value in values):
            errors.append(f"{stage_id}: observation is not a non-negative integer")
            continue
        by_process.setdefault(process_index, []).extend(values)
    if errors:
        return errors

    ordered_groups = [group for group in groups if isinstance(group, dict)]
    expected_digest = digest_observations(stage_id, ordered_groups)
    if stage.get("raw_observations_digest_sha256") != expected_digest:
        errors.append(f"{stage_id}: digest mismatch (recomputed {expected_digest})")

    pooled = [value for group in ordered_groups for value in group["values_ns"]]
    if stage.get("sample_count_pooled") != len(pooled):
        errors.append(
            f"{stage_id}: pooled sample count mismatch "
            f"(recomputed {len(pooled)})"
        )
    counts = [len(by_process.get(index, [])) for index in range(process_count)]
    if stage.get("sample_count_per_process") != counts:
        errors.append(
            f"{stage_id}: per-process sample count mismatch (recomputed {counts})"
        )

    declared_quantiles = stage.get("quantiles_ns_pooled")
    if not isinstance(declared_quantiles, dict):
        errors.append(f"{stage_id}: missing pooled quantiles")
    else:
        for label, fraction in FRACTIONS.items():
            expected = lower_nearest_rank(pooled, fraction)
            if declared_quantiles.get(label) != expected:
                errors.append(
                    f"{stage_id}: {label} mismatch (recomputed {expected})"
                )

    small_sample = len(pooled) < SMALL_SAMPLE_THRESHOLD
    if stage.get("small_sample") is not small_sample:
        errors.append(f"{stage_id}: small_sample mismatch (recomputed {small_sample})")
    provisional = stage.get("provisional_quantiles")
    required_provisional = {"p99", "p999"}
    if small_sample:
        required_provisional |= {"p90", "p95"}
    if not isinstance(provisional, list) or not required_provisional.issubset(
        set(provisional)
    ):
        errors.append(f"{stage_id}: provisional tail quantiles not declared")

    spread = stage.get("between_process_spread")
    if not isinstance(spread, dict):
        errors.append(f"{stage_id}: missing between-process spread")
    else:
        per_process_p50 = [
            lower_nearest_rank(by_process[index], 0.50)
            for index in range(process_count)
            if by_process.get(index)
        ]
        if spread.get("per_process_p50_ns") != per_process_p50:
            errors.append(f"{stage_id}: per-process p50 spread mismatch")
        if len(per_process_p50) == process_count:
            expected_spread = {
                "min_ns": min(per_process_p50),
                "max_ns": max(per_process_p50),
                "mean_ns": round(statistics.fmean(per_process_p50), ROUND_DIGITS),
                "population_sd_ns": round(
                    statistics.pstdev(per_process_p50), ROUND_DIGITS
                ),
            }
            for key, expected in expected_spread.items():
                if spread.get(key) != expected:
                    errors.append(f"{stage_id}: spread {key} mismatch")
    return errors


def verify_process(
    process: dict[str, object], index: int, expected_source_sha: str = BASE_SHA
) -> list[str]:
    """Verify binding and correctness declarations of one process record."""
    errors: list[str] = []
    label = f"process[{index}]"
    if process.get("process_index") != index:
        errors.append(f"{label}: process_index mismatch")
    if process.get("source_sha") != expected_source_sha:
        errors.append(
            f"{label}: source_sha is not the expected source SHA "
            f"({expected_source_sha})"
        )
    load = process.get("start_load_average")
    if not isinstance(load, str) or not load.strip():
        errors.append(f"{label}: start_load_average was not recorded")
    if process.get("checks_passed") is not True:
        errors.append(f"{label}: in-harness checks did not pass")
    digests = process.get("correctness_digests")
    if not isinstance(digests, dict):
        errors.append(f"{label}: missing before/after correctness digests")
    else:
        before = digests.get("before")
        after = digests.get("after")
        if not isinstance(before, dict) or not isinstance(after, dict):
            errors.append(f"{label}: malformed correctness digests")
        elif before != after:
            errors.append(f"{label}: before/after correctness digests differ")
    return errors


def verify_artifact(
    payload: dict[str, object],
    revision_files: dict[str, bytes] | None = None,
    revision_tree: str = "",
    expected_revision: str = MEASUREMENT_REVISION,
    expected_source_sha: str = BASE_SHA,
) -> list[str]:
    """Verify the whole corrected-baseline artifact; return all findings."""
    errors: list[str] = []
    errors.extend(
        verify_provenance(
            payload, revision_files or {}, revision_tree, expected_revision
        )
    )
    if payload.get("schema_version") != 2:
        errors.append("schema_version must be 2")
    if payload.get("base_sha") != BASE_SHA:
        errors.append("base_sha is not the recorded base SHA")
    if payload.get("performance_gate") is not False:
        errors.append("artifact must declare performance_gate=false")
    if payload.get("resource_maximum") is not False:
        errors.append("artifact must declare resource_maximum=false")
    disposition = payload.get("historical_artifact_disposition")
    if not isinstance(disposition, str) or "not silently repaired" not in disposition:
        errors.append("historical artifact disposition is not declared")
    digest_format = payload.get("digest_input_format")
    if not isinstance(digest_format, str) or "repetition_index" not in digest_format:
        errors.append("digest input format is not declared")
    method = payload.get("quantile_method")
    if not isinstance(method, str) or "floor" not in method:
        errors.append("lower-nearest-rank quantile method is not declared")

    processes = payload.get("processes")
    if not isinstance(processes, list) or len(processes) != PROCESS_COUNT:
        errors.append(f"expected {PROCESS_COUNT} process records")
    else:
        seeds: set[int] = set()
        indices: set[int] = set()
        for index, process in enumerate(processes):
            if not isinstance(process, dict):
                errors.append(f"process[{index}] is not an object")
                continue
            errors.extend(verify_process(process, index, expected_source_sha))
            seed = process.get("seed")
            if not is_integer(seed):
                errors.append(f"process[{index}]: malformed seed")
            else:
                if seed in seeds:
                    errors.append(f"process[{index}]: duplicate seed {seed}")
                seeds.add(seed)
            process_index = process.get("process_index")
            if is_integer(process_index):
                if process_index in indices:
                    errors.append(f"process[{index}]: duplicate process_index")
                indices.add(process_index)

    stages = payload.get("stages")
    if not isinstance(stages, list):
        errors.append("missing stages array")
    else:
        stage_ids = [
            stage.get("stage_id") for stage in stages if isinstance(stage, dict)
        ]
        if set(stage_ids) != STAGE_IDS or len(stage_ids) != len(STAGE_IDS):
            errors.append("stage set is incomplete or duplicated")
        for stage in stages:
            if not isinstance(stage, dict):
                errors.append("stage entry is not an object")
                continue
            errors.extend(verify_stage(stage, PROCESS_COUNT))
    return errors


def write_json(path: Path, payload: dict[str, object]) -> None:
    """Write one JSON payload for self-test fixtures."""
    path.write_text(json.dumps(payload, sort_keys=True), encoding="utf-8")


def synthetic_stage(stage_id: str, small: bool) -> dict[str, object]:
    """Build a small but internally consistent stage fixture."""
    per_group = 8 if small else 60
    values = [100 + offset for offset in range(per_group)]
    groups: list[dict[str, object]] = []
    for process_index in range(PROCESS_COUNT):
        for repetition_index in range(2):
            groups.append(
                {
                    "process_index": process_index,
                    "repetition_index": repetition_index,
                    "values_ns": [value + process_index for value in values],
                }
            )
    pooled = [value for group in groups for value in group["values_ns"]]
    quantiles = {
        label: lower_nearest_rank(pooled, fraction)
        for label, fraction in FRACTIONS.items()
    }
    per_process = [
        [
            value
            for group in groups
            if group["process_index"] == index
            for value in group["values_ns"]
        ]
        for index in range(PROCESS_COUNT)
    ]
    p50 = [lower_nearest_rank(samples, 0.50) for samples in per_process]
    provisional = ["p90", "p95", "p99", "p999"] if small else ["p99", "p999"]
    return {
        "stage_id": stage_id,
        "observations": groups,
        "raw_observations_digest_sha256": digest_observations(stage_id, groups),
        "sample_count_pooled": len(pooled),
        "sample_count_per_process": [len(samples) for samples in per_process],
        "quantiles_ns_pooled": quantiles,
        "small_sample": small,
        "provisional_quantiles": provisional,
        "between_process_spread": {
            "per_process_p50_ns": p50,
            "min_ns": min(p50),
            "max_ns": max(p50),
            "mean_ns": round(statistics.fmean(p50), ROUND_DIGITS),
            "population_sd_ns": round(statistics.pstdev(p50), ROUND_DIGITS),
        },
    }


def synthetic_artifact() -> dict[str, object]:
    """Build a valid fixture with all stages, five processes and provenance."""
    processes: list[dict[str, object]] = []
    for index in range(PROCESS_COUNT):
        processes.append(
            {
                "process_index": index,
                "seed": 202611281001 + index,
                "source_sha": BASE_SHA,
                "start_load_average": f"1.0 1.{index} 1.{index}",
                "checks_passed": True,
                "binary_digest_sha256": "c" * 64,
                "workload_digest_sha256": "d" * 64,
                "correctness_digests": {
                    "before": {"event_order": "a" * 64},
                    "after": {"event_order": "a" * 64},
                },
            }
        )
    stages = [
        synthetic_stage(stage_id, small=stage_id == "replay_recovery")
        for stage_id in sorted(STAGE_IDS)
    ]
    toolchain = "rustc 1.99.1 (58cf8f9c3 2026-10-13) | cargo 1.99.1 (5f94df478 2026-08-27)"
    environment = {
        "toolchain": toolchain,
        "target_triple": "aarch64-apple-darwin",
        "build_profile": "bench profile inherits release (lto=thin, codegen-units=1, panic=abort)",
        "compiler_flags": "debug_assertions=false RUSTFLAGS= CARGO_ENCODED_RUSTFLAGS=",
    }
    file_digests = {
        key: hashlib.sha256(SELF_TEST_PACKAGED_FILES[path]).hexdigest()
        for key, path in PACKAGED_PATHS.items()
    }
    binding = {
        "schema": PROVENANCE_SCHEMA,
        "production_source_revision": MEASUREMENT_REVISION,
        "production_source_tree_digest": SELF_TEST_TREE,
        **file_digests,
        "rustc_version": "rustc 1.99.1 (58cf8f9c3 2026-10-13)",
        "target_triple": environment["target_triple"],
        "build_profile": environment["build_profile"],
        "compiler_flags": environment["compiler_flags"],
        "benchmark_binary_digest": "c" * 64,
        "workload_digest": aggregate_digest(
            [(str(index), "d" * 64) for index in range(PROCESS_COUNT)]
        ),
        "raw_observation_digest": aggregate_digest(
            [(stage["stage_id"], stage["raw_observations_digest_sha256"]) for stage in stages]
        ),
        "measurement_revision_vs_base": (
            f"Measured at {MEASUREMENT_REVISION}; the later base "
            f"{BRANCH_BASE_REVISION} contains changes NOT measured"
        ),
    }
    return {
        "schema_version": 2,
        "base_sha": BASE_SHA,
        "performance_gate": False,
        "resource_maximum": False,
        "historical_artifact_disposition": (
            "benchmark-stability-2026-11-28.json is historical and not silently repaired"
        ),
        "digest_input_format": "SHA-256 stage_id|repetition_index|process_index|obs",
        "quantile_method": "lower-nearest-rank index = floor(fraction*n)",
        "environment": environment,
        "harness_source_sha256": {
            "benches/production_components.rs": file_digests["benchmark_harness_digest"],
            "scripts/verify_benchmark_evidence.py": file_digests[
                "benchmark_verifier_digest"
            ],
        },
        PROVENANCE_KEY: binding,
        "processes": processes,
        "stages": stages,
    }


def run_self_test() -> bool:
    """Run positive and negative cases; return True only if all behave."""
    failures: list[str] = []
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        valid = synthetic_artifact()
        valid_path = root / "valid.json"
        write_json(valid_path, valid)
        if verify_artifact(
            load_artifact(valid_path), SELF_TEST_PACKAGED_FILES, SELF_TEST_TREE
        ):
            failures.append("positive case unexpectedly failed")

        # The explicit-revision path must accept the same valid fixture when
        # the expectation matches, and reject it when the expectation drifts.
        if verify_artifact(
            load_artifact(valid_path),
            SELF_TEST_PACKAGED_FILES,
            SELF_TEST_TREE,
            expected_revision=MEASUREMENT_REVISION,
        ):
            failures.append("explicit-revision positive case unexpectedly failed")
        if not verify_artifact(
            load_artifact(valid_path),
            SELF_TEST_PACKAGED_FILES,
            SELF_TEST_TREE,
            expected_revision="e" * 40,
        ):
            failures.append("wrong expected revision was silently accepted")

        negative_cases: dict[str, Any] = {}

        bad_digest = json.loads(json.dumps(valid))
        bad_digest["stages"][0]["observations"][0]["values_ns"][0] += 1
        negative_cases["altered observation"] = bad_digest

        wrong_count = json.loads(json.dumps(valid))
        wrong_count["stages"][0]["sample_count_pooled"] += 1
        negative_cases["wrong sample count"] = wrong_count

        wrong_quantile = json.loads(json.dumps(valid))
        wrong_quantile["stages"][0]["quantiles_ns_pooled"]["p99"] += 1
        negative_cases["wrong quantile"] = wrong_quantile

        duplicate_seed = json.loads(json.dumps(valid))
        duplicate_seed["processes"][1]["seed"] = duplicate_seed["processes"][0]["seed"]
        negative_cases["duplicate seed"] = duplicate_seed

        missing_load = json.loads(json.dumps(valid))
        missing_load["processes"][2]["start_load_average"] = None
        negative_cases["missing start load"] = missing_load

        gated = json.loads(json.dumps(valid))
        gated["performance_gate"] = True
        negative_cases["performance gate"] = gated

        maximum = json.loads(json.dumps(valid))
        maximum["resource_maximum"] = True
        negative_cases["resource maximum"] = maximum

        silent = json.loads(json.dumps(valid))
        silent["historical_artifact_disposition"] = "historical data"
        negative_cases["silent historical repair"] = silent

        wrong_base = json.loads(json.dumps(valid))
        wrong_base["base_sha"] = "0" * 64
        negative_cases["wrong base SHA"] = wrong_base

        mislabeled = json.loads(json.dumps(valid))
        for stage in mislabeled["stages"]:
            if stage["stage_id"] == "replay_recovery":
                stage["small_sample"] = False
        negative_cases["mislabelled small sample"] = mislabeled

        reordered = json.loads(json.dumps(valid))
        values = reordered["stages"][0]["observations"][0]["values_ns"]
        values[0], values[1] = values[1], values[0]
        negative_cases["reordered observations"] = reordered

        for key in (
            "benchmark_harness_digest",
            "benchmark_verifier_digest",
            "cargo_lock_digest",
        ):
            drifted = json.loads(json.dumps(valid))
            drifted[PROVENANCE_KEY][key] = "f" * 64
            negative_cases[f"{key} drift"] = drifted

        # Explicit-revision regression guards: a wrong declared revision and
        # harness digest drift must both fail when the expectation is supplied
        # by the CLI flag rather than taken from the historical default.
        wrong_revision = json.loads(json.dumps(valid))
        wrong_revision[PROVENANCE_KEY]["production_source_revision"] = "e" * 40
        negative_cases["wrong declared revision"] = wrong_revision

        explicit_harness_drift = json.loads(json.dumps(valid))
        explicit_harness_drift[PROVENANCE_KEY]["benchmark_harness_digest"] = "e" * 64
        negative_cases["harness digest drift explicit"] = explicit_harness_drift

        source_drift = json.loads(json.dumps(valid))
        source_drift["processes"][3]["source_sha"] = "e" * 40
        negative_cases["source sha drift"] = source_drift

        dup_key = '{"schema_version":2,"schema_version":2}'
        dup_path = root / "duplicate.json"
        dup_path.write_text(dup_key, encoding="utf-8")
        try:
            load_artifact(dup_path)
            failures.append("duplicate JSON key was silently accepted")
        except ValueError:
            pass

        explicit_cases = {
            "harness digest drift explicit": MEASUREMENT_REVISION,
            "wrong declared revision": MEASUREMENT_REVISION,
        }
        for name, payload in negative_cases.items():
            path = root / f"{name.replace(' ', '_')}.json"
            write_json(path, payload)
            if not verify_artifact(
                load_artifact(path),
                SELF_TEST_PACKAGED_FILES,
                SELF_TEST_TREE,
                expected_revision=explicit_cases.get(name, MEASUREMENT_REVISION),
            ):
                failures.append(f"negative case was silently accepted: {name}")

    for failure in failures:
        print(f"SELF-TEST FAIL: {failure}")
    # positive default, positive explicit, wrong-expectation negative, duplicate
    # key negative, plus every payload-driven negative case.
    case_count = len(negative_cases) + 4
    print(f"self-test: {case_count} cases, {len(failures)} failure(s)")
    return not failures


def main() -> int:
    """Parse arguments and verify either the fixture suite or one artifact."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--self-test", action="store_true", help="run positive and negative cases"
    )
    parser.add_argument(
        "--artifact",
        type=Path,
        default=DEFAULT_ARTIFACT,
        help=f"artifact to verify (default: {DEFAULT_ARTIFACT})",
    )
    parser.add_argument(
        "--measured-revision",
        default=None,
        metavar="SHA",
        help=(
            "expected production_source_revision in the provenance binding "
            f"(default: historical {MEASUREMENT_REVISION}); override for a "
            "later final-head measurement"
        ),
    )
    args = parser.parse_args()
    # Selecting an explicit revision also retargets the per-process source_sha
    # expectation (the harness records `git rev-parse HEAD` at measurement
    # time). With no flag both expectations stay on their historical values.
    expected_revision = args.measured_revision or MEASUREMENT_REVISION
    expected_source_sha = args.measured_revision or BASE_SHA
    if args.self_test:
        return 0 if run_self_test() else 1
    try:
        payload = load_artifact(args.artifact)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"VERIFY FAIL: cannot read artifact: {error}")
        return 1
    binding = payload.get(PROVENANCE_KEY)
    revision = (
        binding.get("production_source_revision") if isinstance(binding, dict) else None
    )
    repo_root = Path(__file__).resolve().parent.parent
    try:
        revision_files, revision_tree = measured_revision_files(revision, repo_root)
    except ValueError as error:
        print(f"VERIFY FAIL: cannot read measured revision: {error}")
        return 1
    errors = verify_artifact(
        payload,
        revision_files,
        revision_tree,
        expected_revision=expected_revision,
        expected_source_sha=expected_source_sha,
    )
    if errors:
        for error in errors:
            print(f"VERIFY FAIL: {error}")
        print(f"verification failed with {len(errors)} finding(s)")
        return 1
    print(
        f"verified {args.artifact}: provenance, stage digests, counts and quantiles reproduce"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
