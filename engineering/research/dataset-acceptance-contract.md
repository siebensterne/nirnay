# Dataset acceptance contract (M4.2)

This document defines the evidence required before a market dataset is accepted
for strategy research. It is a contract, not a certification: every field below
must be satisfied before a dataset may be used for a registered experiment that
will be described as research on real market data.

## Status

**BLOCKED.** All 21 source-product rights in
`configs/providers/source-products.v1.json` are `UNKNOWN`, and no written
permission is held. No captured provider data may be used as a research dataset
until the applicable capture, local storage, replay, retention and internal
analytics rights are established and recorded. The synthetic pilot recorded in
`evidence/M4-RESEARCH-EVIDENCE-2026-11-21.md` exercises the process only; it is
not evidence about a licensed dataset.

## Required evidence

| Requirement | Accepted evidence | Current state |
|---|---|---|
| Historical universe | Point-in-time constituents including delisted securities, with effective dates and source digest | Unavailable for licensed data; synthetic fixture is a single instrument |
| Instrument identity | Effective-dated symbol/security mappings and venue identity | Synthetic fixture only |
| Corporate actions | Splits, bonuses, dividends, mergers, corrections, each with knowledge time and revision lineage | Unavailable; synthetic fixture has no actions |
| Market history | Consistent daily prices and volumes for the selected universe and date range | Synthetic close-only bars, no volume |
| Calendar | Sessions, holidays and applicable trading status | Not acquired; rights-blocked |
| Costs | Effective-dated fees, taxes and charges for the intended venue and product | Synthetic `AlphaCosts` assumptions only |
| Provenance | Source identity, version, digest, timestamp semantics and corrections | Synthetic fixture digest recorded |
| Rights | Written permission covering the exact operations, instruments, fields, date range, accounts and retention period | `UNKNOWN` for all products |

## Point-in-time declaration semantics

`dataset_point_in_time` is a narrow boolean on an immutable experiment record.
Its meaning is fixed by the required `point_in_time_basis` declaration:

- `none` certifies no point-in-time property; a record cannot set the boolean
  true under this basis.
- `chronological_only` certifies only that observations are ordered by their
  declared timestamps, so a return at index `t` uses inputs timestamped at or
  before `t`. Ordered timestamps alone **do not** certify point-in-time universe
  membership, listings, corporate-action correctness, delisting coverage or
  revision knowledge times. Such a dataset must not be described as
  point-in-time correct for universe or corporate actions.
- `universe_and_corporate_actions` additionally requires a named universe
  version and non-empty listing and corporate-action source lineage in the
  immutable dataset declaration. It certifies those dimensions within the
  declared dataset version only; source authenticity, completeness, entitlements
  and calendar correctness remain separate evidence requirements.

## Bar cadence and annualisation

A registered record declares `bar_kind`, `bar_interval_ns`,
`timestamp_semantics`, `calendar_id`, `session_definition`,
`annualization_basis` and `periods_per_year`. Fixed-sampling daily and minute
bars must carry a positive interval; tick and event bars must declare
`not_applicable`. The registered cadence is checked against the ordered bar
timestamps before any search runs.

Named annualisation bases carry a fixed period count and bar family; an
arbitrary positive factor requires the explicit `synthetic_declared` basis. The
registered `periods_per_year` is the only annualisation factor used by a
registered search; a CLI value is an assertion that must match it. Raw timestamp
differences do not define exchange sessions. Holidays, overnight breaks and
auctions require calendar and session semantics and are not inferable from raw
diffs; no missing bar is invented by the validator. A real-market series with
such gaps must be normalised under a declared calendar before registration.

## Acceptance rule

A dataset is accepted only when every row above is satisfied by dated evidence
and the registry rights for the intended operations are `PERMITTED` with the
matching document reference, scope and review date. A missing row is a failure,
not an assumption. `UNKNOWN` and `PROHIBITED` are denials at the recorder and
replay boundaries and must remain so until an owner supplies the evidence.

## Scope rule

A daily, low-turnover factor or momentum baseline does not require L3 queue
data. A queue-sensitive or intraday strategy must first define the trade,
quote, depth and timestamp data it needs; MBP snapshots cannot establish exact
FIFO position. No dataset choice implies that a strategy has alpha.
