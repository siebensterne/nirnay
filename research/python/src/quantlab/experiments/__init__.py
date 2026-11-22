"""Experiment registry, registered search and split-geometry package.

The original validation and hashing API is preserved. The registry contract,
the registered grid-search adapter and the walk-forward declaration adapter
share the same holdout and complete-accounting rules; none claims alpha.
"""
from __future__ import annotations

from .calendar import calendar_binding, calendar_digest, expected_timestamps, resolve_calendar
from .cadence import (assert_dataset_origin, dataset_origin, is_synthetic_record,
                      validate_bar_cadence)
from .coverage import (OBSERVATION_STATUSES, explicit_status_observation,
                       validate_status_observations)
from .parameter_search import CLAIM_BOUNDARY, generate_configs, grid_search
from .registry import (DuplicateExperimentError, ExperimentError, ExperimentRegistry,
                       HoldoutClaim, HoldoutReuseError, Receipt, StoredRecord, append_result,
                       assert_holdout_key, build_search_space, claim_kind_for, content_hash,
                       holdout_key, holdout_key_for, make_attempt, register, validate_claim,
                       validate_experiment, validate_registered_experiment, verify_integrity)
from .schema import (canonical_json, freeze_experiment, freeze_registered_experiment,
                     load_experiment, recompute_report_hash, validate_complete_trial_accounting,
                     validate_period_geometry)
from .walk_forward import (WalkForwardDeclaration, build_walk_forward_declarations,
                           search_space_for_declaration, validate_split_geometry)

__all__ = [
    "CLAIM_BOUNDARY",
    "OBSERVATION_STATUSES",
    "DuplicateExperimentError",
    "ExperimentError",
    "ExperimentRegistry",
    "HoldoutClaim",
    "HoldoutReuseError",
    "Receipt",
    "StoredRecord",
    "WalkForwardDeclaration",
    "append_result",
    "assert_dataset_origin",
    "assert_holdout_key",
    "build_search_space",
    "build_walk_forward_declarations",
    "calendar_binding",
    "calendar_digest",
    "canonical_json",
    "claim_kind_for",
    "content_hash",
    "dataset_origin",
    "freeze_experiment",
    "freeze_registered_experiment",
    "explicit_status_observation",
    "expected_timestamps",
    "generate_configs",
    "grid_search",
    "holdout_key",
    "holdout_key_for",
    "is_synthetic_record",
    "load_experiment",
    "make_attempt",
    "recompute_report_hash",
    "search_space_for_declaration",
    "register",
    "resolve_calendar",
    "validate_bar_cadence",
    "validate_claim",
    "validate_complete_trial_accounting",
    "validate_experiment",
    "validate_period_geometry",
    "validate_status_observations",
    "validate_registered_experiment",
    "validate_split_geometry",
    "verify_integrity",
]
