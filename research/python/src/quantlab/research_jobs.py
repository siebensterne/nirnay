"""Local, opt-in research job manifest and runner.

A checked-in manifest declares a job id, its immutable synthetic dataset
identity and digest, the pinned environment it was recorded against and a
read-only command. The runner validates the manifest, refuses a dataset-hash
or environment-pin mismatch, executes the command in dry-run mode on a freshly
built synthetic fixture and appends one structured run record through the
existing append-only experiment registry.

This is deliberately not a scheduler, daemon, database service or distributed
executor. No cron, queue, allocation or remote compute is involved. The emitted
record is an internal local observation, not a latency, throughput, readiness
or data-rights claim.
"""
from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from jsonschema import Draft202012Validator

from .bench import synthetic_events
from .cli import _load_reference_data
from .dataset_admission import DatasetAdmissionError, admit_published_dataset
from .experiments import (ExperimentError, ExperimentRegistry, build_search_space,
                          calendar_binding, content_hash, holdout_key, make_attempt)
from .experiments.reproducibility import bind_published_dataset, environment_digest
from .experiments.schema import NOT_APPLICABLE_INTERVAL, SYNTHETIC_ANNUALIZATION_BASIS
from .point_in_time import PITDatasetBuilder, write_dataset

ROOT = Path(__file__).resolve().parents[4]
FIXTURES = ROOT / "research" / "python" / "fixtures"
SCHEMA_PATH = ROOT / "schemas" / "research" / "research-job-manifest.v1.schema.json"
DATASET_ID = "research-job-synthetic-nse-session"
DATASET_SIZE = 200
DATASET_SEED = 7
DATASET_CREATED_AT_NS = 1_800_000_000_000_000_000
UNIVERSE_ID = "fixture-universe"
FEATURE_VERSION = "features.v1"
COMMIT = "unspecified"
COMMAND_PREFIX = ("python", "-m", "quantlab.cli", "verify-dataset")
DATASET_PLACEHOLDER = "{dataset}"
SCHEMA_VERSION = 1

_VALIDATION = {"start_index": 81, "end_index": 120}
_HOLDOUT = {"start_index": 121, "end_index": 160}
_TRAIN = {"start_index": 0, "end_index": 80}
_CONFIG = {"family": "research-job-dry-run", "mode": "synthetic-local"}
_REGISTERED_AT = "2026-11-21T00:00:00Z"


@dataclass(frozen=True, slots=True)
class JobRunResult:
    """Structured, append-only run record assembled from a registry receipt."""

    job_id: str
    commit: str
    command: str
    exit_code: int
    duration_ms: float
    output_sha256: str
    dataset_sha256: str
    dataset_id: str
    dataset_version: str
    code_digest: str
    constraint_sha256: str
    verify_status: str
    dry_run: bool
    synthetic: bool
    registry_sequence: int
    record_hash: str
    integrity_status: str

    def as_dict(self) -> dict[str, Any]:
        """Return a canonical mapping of the structured run record."""
        return {
            "job_id": self.job_id,
            "commit": self.commit,
            "command": self.command,
            "exit_code": self.exit_code,
            "duration_ms": self.duration_ms,
            "output_sha256": self.output_sha256,
            "dataset_sha256": self.dataset_sha256,
            "dataset_id": self.dataset_id,
            "dataset_version": self.dataset_version,
            "code_digest": self.code_digest,
            "constraint_sha256": self.constraint_sha256,
            "verify_status": self.verify_status,
            "dry_run": self.dry_run,
            "synthetic": self.synthetic,
            "registry_sequence": self.registry_sequence,
            "record_hash": self.record_hash,
            "integrity_status": self.integrity_status,
        }


def package_code_paths() -> list[str]:
    """List the quantlab source paths bound by the manifest code digest."""
    package = ROOT / "research" / "python" / "src" / "quantlab"
    return [
        str(path.relative_to(ROOT))
        for path in sorted(package.rglob("*.py"))
        if path.is_file()
    ]


def environment_block(*, commit: str = COMMIT, root: Path = ROOT) -> dict[str, Any]:
    """Compute the pinned-environment block from the files currently on disk."""
    constraint = "constraints/python-research.txt"
    constraint_path = Path(root) / constraint
    constraint_sha256 = hashlib.sha256(constraint_path.read_bytes()).hexdigest()
    paths = package_code_paths()
    return {
        "python_version": "3.12",
        "python_constraint_file": constraint,
        "constraint_sha256": constraint_sha256,
        "code_paths": paths,
        "code_digest": environment_digest(paths, root=root),
        "commit": commit,
    }


def build_synthetic_dataset(out_dir: str | Path, *, size: int = DATASET_SIZE,
                            seed: int = DATASET_SEED) -> tuple[Path, dict[str, Any]]:
    """Build and publish a deterministic synthetic point-in-time dataset."""
    events = synthetic_events(size, seed)
    references = _load_reference_data(FIXTURES / "reference-data.example.json")
    rows, manifest = PITDatasetBuilder(references).build(
        events,
        dataset_id=DATASET_ID,
        created_at_utc_ns=DATASET_CREATED_AT_NS,
        universe_id=UNIVERSE_ID,
    )
    dataset_dir = write_dataset(rows, manifest, Path(out_dir) / "dataset")
    return dataset_dir, bind_published_dataset(dataset_dir)


def default_job(environment: dict[str, Any], binding: dict[str, Any], *,
                job_id: str = "bench-research-dry-run") -> dict[str, Any]:
    """Build a valid job declaration bound to a computed synthetic dataset."""
    return {
        "job_id": job_id,
        "dataset_ref": f"{binding['dataset_id']}:synthetic-local",
        "dataset_id": binding["dataset_id"],
        "dataset_version": binding["dataset_version"],
        "dataset_sha256": binding["dataset_manifest_sha256"],
        "code_digest": environment["code_digest"],
        "constraint_file": environment["python_constraint_file"],
        "command": [*COMMAND_PREFIX, DATASET_PLACEHOLDER],
        "schedule_hint": "manual-opt-in",
        "dry_run": True,
    }


def load_schema() -> dict[str, Any]:
    """Load the checked-in JSON schema for the research job manifest."""
    try:
        return json.loads(SCHEMA_PATH.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ExperimentError(f"research job manifest schema is unreadable: {exc}") from exc


def validate_manifest(payload: Any) -> dict[str, Any]:
    """Validate manifest shape and cross-field invariants; return the payload."""
    if not isinstance(payload, dict):
        raise ExperimentError("research job manifest must be a JSON object")
    errors = sorted(
        Draft202012Validator(load_schema()).iter_errors(payload),
        key=lambda error: str(error.path),
    )
    if errors:
        first = errors[0]
        location = "/".join(str(part) for part in first.absolute_path) or "$"
        raise ExperimentError(f"research job manifest invalid at {location}: {first.message}")
    environment = payload["environment"]
    seen: set[str] = set()
    for job in payload["jobs"]:
        job_id = job["job_id"]
        if job_id in seen:
            raise ExperimentError(f"duplicate research job id: {job_id}")
        seen.add(job_id)
        if not job["dataset_version"].startswith(job["dataset_id"] + ":"):
            raise ExperimentError(f"job {job_id} dataset_version does not match dataset_id")
        if job["code_digest"] != environment["code_digest"]:
            raise ExperimentError(f"job {job_id} code_digest differs from the environment pin")
        if job["constraint_file"] != environment["python_constraint_file"]:
            raise ExperimentError(f"job {job_id} constraint_file differs from the environment pin")
        if tuple(job["command"][:len(COMMAND_PREFIX)]) != COMMAND_PREFIX:
            raise ExperimentError(f"job {job_id} command is not an allowlisted quantlab command")
        if job["command"][len(COMMAND_PREFIX):] != [DATASET_PLACEHOLDER]:
            raise ExperimentError(f"job {job_id} command must take exactly one dataset placeholder")
    return payload


def load_manifest(path: str | Path) -> dict[str, Any]:
    """Read and validate a checked-in research job manifest."""
    try:
        payload = json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ExperimentError(f"could not read research job manifest: {exc}") from exc
    return validate_manifest(payload)


def assert_pinned_environment(environment: dict[str, Any], job: dict[str, Any],
                              *, root: Path = ROOT) -> dict[str, str]:
    """Refuse an unpinned environment or one that differs from the job pin."""
    requested = Path(environment["python_constraint_file"])
    if requested.is_absolute():
        raise ExperimentError("unpinned environment: constraint file must be repository-relative")
    resolved = (Path(root) / requested).resolve()
    if Path(root).resolve() not in resolved.parents:
        raise ExperimentError("unpinned environment: constraint file escapes the repository")
    if resolved.is_symlink() or not resolved.is_file():
        raise ExperimentError("unpinned environment: constraint file is missing or redirected")
    content = resolved.read_bytes()
    actual = hashlib.sha256(content).hexdigest()
    if actual != environment["constraint_sha256"]:
        raise ExperimentError("unpinned environment: constraint file digest mismatch")
    text = content.decode("utf-8")
    if environment["python_version"] not in text:
        raise ExperimentError("unpinned environment: Python version is not pinned in constraints")
    for number, line in enumerate(text.splitlines(), 1):
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        if "==" not in stripped:
            raise ExperimentError(
                f"unpinned environment: dependency at line {number} is not pinned"
            )
    digest = environment_digest(environment["code_paths"], root=root)
    if digest != environment["code_digest"]:
        raise ExperimentError("unpinned environment: declared code digest mismatch")
    if (job["code_digest"] != digest or
            job["constraint_file"] != environment["python_constraint_file"]):
        raise ExperimentError("unpinned environment: job does not use the declared environment pin")
    return {"constraint_sha256": actual, "code_digest": digest}


def assert_dataset_binding(job: dict[str, Any], binding: dict[str, Any]) -> None:
    """Refuse a stale dataset hash or a dataset identity that moved."""
    if binding["dataset_manifest_sha256"] != job["dataset_sha256"]:
        raise ExperimentError(
            "stale dataset hash: synthetic dataset digest differs from the manifest"
        )
    if binding["dataset_id"] != job["dataset_id"]:
        raise ExperimentError("stale dataset binding: dataset_id differs from the manifest")
    if binding["dataset_version"] != job["dataset_version"]:
        raise ExperimentError("stale dataset binding: dataset_version differs from the manifest")


def _resolve_command(command: list[str], dataset_dir: Path) -> list[str]:
    """Substitute the dataset placeholder and confirm the command stays read-only."""
    if tuple(command[:len(COMMAND_PREFIX)]) != COMMAND_PREFIX:
        raise ExperimentError("refusing a command outside the read-only quantlab allowlist")
    if command[len(COMMAND_PREFIX):] != [DATASET_PLACEHOLDER]:
        raise ExperimentError("refusing a command without exactly one dataset placeholder")
    return [sys.executable, *command[1:-1], str(dataset_dir)]


def _execute(argv: list[str], *, root: Path = ROOT) -> tuple[str, str, int, float]:
    """Execute the allowlisted dry-run command and capture its measured outcome."""
    src = Path(root) / "research" / "python" / "src"
    env = os.environ.copy()
    env["PYTHONPATH"] = str(src) + os.pathsep + env.get("PYTHONPATH", "")
    started = time.perf_counter()
    completed = subprocess.run(argv, cwd=root, env=env, capture_output=True,
                               text=True, check=False)
    duration_ms = (time.perf_counter() - started) * 1000.0
    return completed.stdout, completed.stderr, completed.returncode, duration_ms


def _experiment_id(job: dict[str, Any], output_sha256: str) -> str:
    return f"JOB-{job['job_id']}-{output_sha256[:16]}"


def _registered_record(job: dict[str, Any], environment: dict[str, Any],
                       binding: dict[str, Any], output_sha256: str) -> dict[str, Any]:
    """Build a valid pre-registered experiment record declaring the run inputs."""
    space = build_search_space("research-job-dry-run", [_CONFIG], train_period=_TRAIN)
    return {
        "experiment_id": _experiment_id(job, output_sha256),
        "hypothesis": "The declared read-only dry-run command verifies its synthetic dataset.",
        "economic_rationale": "Reproducibility probe only; no trading or economic claim is made.",
        "dataset_ref": job["dataset_ref"],
        "dataset_manifest_sha256": binding["dataset_manifest_sha256"],
        "dataset_point_in_time": True,
        "code_revision": environment.get("commit", COMMIT),
        "config": {
            "job_id": job["job_id"],
            "command": " ".join(job["command"]),
            "commit": environment.get("commit", COMMIT),
            "dataset_sha256": job["dataset_sha256"],
            "code_digest": job["code_digest"],
            "constraint_file": job["constraint_file"],
            "schedule_hint": job["schedule_hint"],
            "dry_run": True,
            "synthetic": True,
        },
        "status": "registered",
        "registered_at_utc": _REGISTERED_AT,
        "falsification_rule": (
            "Reject if the dry-run command exits non-zero or the dataset fails verification."
        ),
        "dataset_id": binding["dataset_id"],
        "dataset_origin": "SYNTHETIC",
        "dataset_version": binding["dataset_version"],
        "dataset_lineage": binding["dataset_lineage"],
        "universe_id": UNIVERSE_ID,
        "feature_version": FEATURE_VERSION,
        "strategy_code_hash": job["code_digest"],
        "search_space": space,
        "attempted_configs": [],
        "validation_period": dict(_VALIDATION),
        "holdout_period": dict(_HOLDOUT),
        "holdout_key": holdout_key(binding["dataset_version"], UNIVERSE_ID, FEATURE_VERSION,
                                   _VALIDATION, _HOLDOUT),
        "cost_model": {"assumptions": {"source": "synthetic-local-no-calibration"},
                       "cost_model_sha256": content_hash(
                           {"source": "synthetic-local-no-calibration"})},
        "execution_assumptions": {"mode": "dry-run", "synthetic": True,
                                  "execution_latency_model": "not_modelled",
                                  "no_live_routing": True},
        "bar_kind": "other",
        "bar_interval_ns": NOT_APPLICABLE_INTERVAL,
        "bar_timezone": "UTC",
        "bar_anchor": "observation_time",
        "timestamp_semantics": "observation_time",
        **calendar_binding("synthetic:event-sequence-v1"),
        "session_definition": "Synthetic canonical event sequence; no exchange session",
        "annualization_basis": SYNTHETIC_ANNUALIZATION_BASIS,
        "periods_per_year": 1.0,
        "point_in_time_basis": "chronological_only",
    }


def _terminal_record(registered: dict[str, Any], job: dict[str, Any],
                     environment: dict[str, Any], pins: dict[str, str],
                     binding: dict[str, Any], *, exit_code: int, duration_ms: float,
                     output_sha256: str, verify_status: str) -> dict[str, Any]:
    """Build the terminal experiment record carrying the observed run record."""
    run_fields = {
        "commit": environment.get("commit", COMMIT),
        "command": " ".join(job["command"]),
        "exit_code": exit_code,
        "duration_ms": duration_ms,
        "output_sha256": output_sha256,
        "dataset_sha256": job["dataset_sha256"],
        "dataset_manifest_sha256": binding["dataset_manifest_sha256"],
        "code_digest": pins["code_digest"],
        "constraint_sha256": pins["constraint_sha256"],
        "verify_status": verify_status,
        "dry_run": True,
        "synthetic": True,
    }
    status = "completed" if exit_code == 0 else "rejected"
    attempts = [make_attempt(_CONFIG, {"exit_code": exit_code, "verify_status": verify_status})]
    return {
        **registered,
        "status": status,
        "attempted_configs": attempts,
        "result": {"status": status, "run_record_sha256": content_hash(run_fields)},
        "results": run_fields,
        "validation": {"dataset_point_in_time": True, "holdout_used_for_selection": False,
                       "claim_boundary": "internal_integrity_only"},
        "diagnostics": {"uses_final_holdout": False,
                        "claim_boundary": "local_synthetic_observation_not_deployment_evidence"},
    }


def run_job(manifest_path: str | Path, out_dir: str | Path, registry_path: str | Path, *,
            job_id: str | None = None, admission: str | Path | None = None,
            root: Path = ROOT) -> JobRunResult:
    """Validate, dry-run and append one structured run record for a manifest job."""
    manifest = load_manifest(manifest_path)
    environment = manifest["environment"]
    jobs = manifest["jobs"]
    job = next((item for item in jobs if item["job_id"] == job_id), None) if job_id else jobs[0]
    if job is None:
        raise ExperimentError(f"research job id not found in manifest: {job_id}")
    pins = assert_pinned_environment(environment, job, root=root)
    dataset_dir, binding = build_synthetic_dataset(out_dir)
    if admission is None:
        raise DatasetAdmissionError(
            "research jobs require a dataset-admission record before any research use"
        )
    admit_published_dataset(dataset_dir, admission)
    assert_dataset_binding(job, binding)

    argv = _resolve_command(job["command"], dataset_dir)
    stdout, _stderr, exit_code, duration_ms = _execute(argv, root=root)
    output_sha256 = hashlib.sha256(stdout.encode("utf-8")).hexdigest()
    verify_status = "verified" if exit_code == 0 else "not_verified"

    registry = ExperimentRegistry(registry_path)
    registered = _registered_record(job, environment, binding, output_sha256)
    registry.register(registered)
    terminal = _terminal_record(registered, job, environment, pins, binding,
                                exit_code=exit_code, duration_ms=duration_ms,
                                output_sha256=output_sha256, verify_status=verify_status)
    receipt = registry.append_result(terminal)
    integrity = registry.verify_integrity(dataset_dir=dataset_dir)
    return JobRunResult(
        job_id=job["job_id"],
        commit=environment.get("commit", COMMIT),
        command=" ".join(job["command"]),
        exit_code=exit_code,
        duration_ms=duration_ms,
        output_sha256=output_sha256,
        dataset_sha256=binding["dataset_manifest_sha256"],
        dataset_id=binding["dataset_id"],
        dataset_version=binding["dataset_version"],
        code_digest=pins["code_digest"],
        constraint_sha256=pins["constraint_sha256"],
        verify_status=verify_status,
        dry_run=True,
        synthetic=True,
        registry_sequence=receipt.sequence,
        record_hash=receipt.record_hash,
        integrity_status=integrity["status"],
    )
