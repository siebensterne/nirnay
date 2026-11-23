"""Typed event-driven research simulation core.

All money and prices are ``Decimal``; every decision is derived from recorded
events and seeds.  This is a research tool, not an exchange matching engine.
"""
from __future__ import annotations

import hashlib
from dataclasses import asdict, is_dataclass

from ..market import canonical_bytes
from .calibration import (
    ArtifactError,
    ArtifactResolver,
    CalibrationArtifact,
    calibration_statement,
    cost_provenance,
)
from .adapters import (
    market_event_to_sim_events,
    market_events_to_sim_events,
    quarantine_reason,
)
from .core import (
    EngineConfig,
    ReplaySeeds,
    SimulationEngine,
    Strategy,
    StrategyContext,
    SimResult,
    reduce,
)
from .events import (
    EVENT_TYPES,
    AuctionPhase,
    BookDelta,
    BookSnapshot,
    ConfigChange,
    EventEnvelope,
    ExecutionReport,
    InstrumentChange,
    MarketStatus,
    OrderAdd,
    OrderCancel,
    OrderIntent,
    OrderModify,
    SimBookLevel,
    SimulationEvent,
    TradePrint,
    UnmarkedResidual,
    event_from_dict,
    event_hash,
    event_ordering_key,
    event_to_dict,
)
from .execution.base import (
    BaselineParams,
    ConservativeMbpModel,
    ExecutionModel,
    LatencyDistribution,
    QueueTruthUnavailable,
    default_cost_model,
    default_impact,
)
from .execution.costs import (
    BorrowTerms,
    CostModel,
    FeeEntry,
    FeeSchedule,
    FundingTerms,
    ImpactConfig,
    MarginTerms,
    SlippageEntry,
    SlippageSchedule,
)
from .replay import (
    ComparisonResult,
    RunRecord,
    compare_records,
    read_record,
    replay_in_fresh_process,
    write_record,
)
from .state import (
    BookState,
    ChargeRecord,
    ExposureState,
    MarkState,
    OrderState,
    QueueState,
    RiskBudgets,
    SimState,
    empty_state,
    state_hash,
)
from .store import DuplicateTrial, TrialConflict, TrialStore, TrialStoreError


def cost_assumptions_digest(costs: object) -> str:
    """Digest the exact cost assumptions supplied to a simulation."""
    if is_dataclass(costs) and not isinstance(costs, type):
        body: object = asdict(costs)
    elif isinstance(costs, dict):
        body = costs
    else:
        raise ValueError("cost assumptions must be a dataclass or object")
    return hashlib.sha256(canonical_bytes(body)).hexdigest()


__all__ = [
    "EVENT_TYPES",
    "ArtifactError",
    "ArtifactResolver",
    "AuctionPhase",
    "BaselineParams",
    "BookDelta",
    "BookSnapshot",
    "BookState",
    "BorrowTerms",
    "CalibrationArtifact",
    "ChargeRecord",
    "ComparisonResult",
    "ConfigChange",
    "ConservativeMbpModel",
    "CostModel",
    "DuplicateTrial",
    "EngineConfig",
    "EventEnvelope",
    "ExecutionModel",
    "ExecutionReport",
    "ExposureState",
    "FeeEntry",
    "FeeSchedule",
    "FundingTerms",
    "ImpactConfig",
    "InstrumentChange",
    "LatencyDistribution",
    "MarginTerms",
    "MarketStatus",
    "MarkState",
    "OrderAdd",
    "OrderCancel",
    "OrderIntent",
    "OrderModify",
    "OrderState",
    "QueueState",
    "QueueTruthUnavailable",
    "ReplaySeeds",
    "RiskBudgets",
    "RunRecord",
    "SimBookLevel",
    "SimResult",
    "SimState",
    "SimulationEngine",
    "SimulationEvent",
    "SlippageEntry",
    "SlippageSchedule",
    "Strategy",
    "StrategyContext",
    "TradePrint",
    "TrialConflict",
    "TrialStore",
    "TrialStoreError",
    "UnmarkedResidual",
    "calibration_statement",
    "compare_records",
    "cost_assumptions_digest",
    "cost_provenance",
    "default_cost_model",
    "default_impact",
    "empty_state",
    "event_from_dict",
    "event_hash",
    "event_ordering_key",
    "event_to_dict",
    "market_event_to_sim_events",
    "market_events_to_sim_events",
    "quarantine_reason",
    "read_record",
    "reduce",
    "replay_in_fresh_process",
    "state_hash",
    "write_record",
]
