# Fieldbook review refresh and ownership reconciliation — 2026-11-23

**Status:** read-only reconciliation record. Not an acceptance certificate, not an
ADR ratification, not authorisation to collect, and not a research result.

## 1. Question

Does the earlier Fieldbook review at `aee91de` remain valid at the pinned workspace
baseline `28eb877`, and what is the current status of the ownership decision and the
NDC semantic contract across the two repositories?

## 2. Baseline refresh

Between the reviewed commit `aee91de` and the pinned baseline `28eb877` six commits
landed on Fieldbook `main`: three merges and three changes covering fixture
capabilities and connector invocation in conformance, evidence identity through
arbitration and replay, and staged benchmark methodology with trial and allocation
metrics.

Those commits touch acquisition conformance, replay evidence identity and performance
methodology. They do not add a rights gate to provider egress, do not change the
distribution verifier, and do not add a release index. Therefore the principal
findings of the earlier review remain valid at `28eb877`:

- A1 architecture: CORRECT with documentation-path and unimplemented-identity
  corrections.
- A2 durability: CORRECT for the local finalized-run fault model; DEFER for
  cross-domain backup and restoration.
- A3 semantics: CORRECT; the live path remains aggregated L2 for crypto and
  prediction-market venues, with no NSE/BSE cash-equity implementation.
- A4 governance: at `28eb877`, rights types remain unwired to collection and
  publication. This finding is now superseded by W1 work described below.
- A5 distribution: CORRECT; non-mutation and atomic publication hold, while the
  verifier defect, release index, rights proof and NDC fields remain open.

This refresh is a targeted reconciliation, not a new line-by-line audit. Live captures
are operator-local, CI and live evidence were not independently reproduced, and no
provider was contacted.

## 3. W1 remediation after the pinned baseline

Subsequent Fieldbook `main` at `afc6f287` includes PR #40, which changes the earlier
A4 conclusion. The CLI no longer starts provider activity without an explicit
subcommand. Rights are loaded from a fail-closed file, require venue and operation
identity, and deny `UNKNOWN`, `PENDING`, expired, missing and venue-less records.
Authorization is checked before HTTP client construction and websocket resolution,
and for collect, discovery, latency, network-probe and analytics export. Each decision
writes a scoped audit event. Self-attested environment gates were removed, and offline
tests prove denial without provider contact. All five venue HTTP clients now use the
gated egress boundary; full workspace tests pass (709 passed, 0 failed, 4 ignored).

The A5 verifier defect, release index, rights proof bound to a release, canonical
instrument identity and corrections/as-of fields remain open. W1 does not close them.

The Nirnay side of W1, merged at `b997d48`, requires an admitted published release
with written rights, a passing coverage certificate, matching release digests and
point-in-time lineage before a research job can run. The rights registry is unchanged:
21 rights remain `UNKNOWN`, and no dataset is accepted.

## 4. Ownership decision status

The ownership split remains the recommended model:

| Responsibility | Proposed owner | Status in code |
|---|---|---|
| Source acquisition, raw archive, normalization, reference data, releases | Fieldbook | Implemented for non-Indian venues; no NSE/BSE product |
| Research, experiments, simulation, portfolio, risk | Nirnay | Present; no experiment may run without an accepted dataset |
| Eventual execution | Nirnay | Disabled pending separate authorisation |

Nirnay ADR-0001 remains **Proposed design decision**. Fieldbook's own ADR-0001 concerns
instrument and venue reference identity, not cross-repository ownership; its identity
migration is still deferred. Owner ratification of responsible people and formal
acceptance of the cross-repository decision have not occurred. This record does not
ratify either ADR.

## 5. NDC v1 semantic contract status

The ten semantic requirement groups are agreed as requirements, not as a frozen
schema. The release-manifest and event-envelope separation stands: dataset-wide facts
are published once in an immutable release manifest, while per-event fields carry
source and instrument identity, typed payload, source and receive times, clock domain,
sequence and continuity, raw digest and lineage, and revision and as-of information.

Current status across the two repositories:

- Fieldbook publishes digests, coverage metadata and a derived release directory with
  atomic publication; it lacks a release index, a rights proof, canonical instrument
  identity, gap rows and correction lineage.
- Nirnay defines consumer validation, ordered checks and rejection codes in
  `engineering/ndc-consumer-requirements-v1.md`, and refuses any research job without
  an admitted release.
- No transport, queue, shared crate or database has been selected. No India connector,
  scraper, collector, data collection or backtest is part of this reconciliation.

## 6. Gate sequence and freeze

W1 is complete in both repositories. The remaining sequence is unchanged: owner
ratification of ownership roles, approval of the NDC semantic definitions, then source
qualification, dataset acceptance and only afterwards a registered experiment.
No stage here authorises live execution.

Freeze holds: incremental data budget INR 0, all 21 rights `UNKNOWN`, dataset
acceptance `BLOCKED`, the M4 holdout untouched, no experiment registered, no provider
contact, no new collector, database, queue, transport or strategy work.
