# Owner directive on the data freeze — 2026-11-22

## Document control

| Field | Value |
|---|---|
| Record | `evidence/OWNER-DIRECTIVE-2026-11-22.md` |
| Date | 2026-11-22 |
| State | Immutable dated record; documentation only, not code and not an experiment |
| Authority | Owner directive as recorded under the freeze mandate for this worktree |
| Effect | Governs research, procurement and documentation activity in this freeze |
| Rights effect | None. No right is granted, changed or recorded as permitted here |

This record fixes the owner's directive in substance. It does not accept a
dataset, authorize an experiment, change a data-right, or permit a provider
operation. Dated records and the four artifacts under `evidence/m4/` remain
immutable.

## 1. Directive

D1. No synthetic market data may be generated, transformed, extended, reused or
admitted as research evidence. The prohibition covers OHLCV and prices; trades,
quotes and depth; market-shaped calendars and session schedules; instrument
universes; corporate-action histories; and strategy or portfolio returns
derived from any of those forms.

D2. Existing synthetic artifacts are historical. They must not be reused,
regenerated or extended, and they must not be used as research evidence. They
retain their dated meaning as records of the work in which they were created.

D3. Pure mathematical unit tests using scalars and abstract timestamps remain
allowed. They must not represent invented market observations and must not
claim exchange-calendar validity. A fixture that models a session, a holiday, a
bar anchor or a market-shaped sequence is not a pure mathematical test.

D4. No provider retrieval, storage, replay, backup or internal analytics may be
performed without written permission and owner approval for the exact
operation, scope, instruments, fields, date range and retention period. An
entitlement named in a prior record is not a grant of a new operation.

D5. All rights recorded as `UNKNOWN` remain `UNKNOWN`. No public page, tariff,
sample file, prior session or local artifact changes a right to `PERMITTED`.

D6. No strategy registration, alpha search, backtest, parameter tuning, holdout
evaluation or profitability statement may proceed until a qualified real
dataset is approved under the dataset acceptance contract. The absence of a
profitability claim is not a substitute for that approval.

D7. The consumed M4 holdout is immutable. It must not be reopened, re-inspected,
rescored, reselected or reused under a new label.

D8. The incremental data budget is INR 0. No paid subscription, paid vendor
engagement or purchase order is authorized. Published tariffs may be cited as
context only.

D9. The next work item is a source-availability report built from official
documentation and existing entitlements only. No credential, API call,
download or provider contact may be used until explicitly authorized by the
owner.

D10. Every new record must report its status in one of the five categories in
section 2. A record that does not fit a category is not admitted as evidence.

## 2. Reporting categories

| Category | Meaning under this directive |
|---|---|
| implemented | Code or a record exists and its behavior is described; this is not real-market verification. |
| tested with abstract mathematical values | Checks use scalars or abstract timestamps only and assert no market observation or exchange-calendar validity. |
| tested against official reference data | Checks compare a claim with official public documentation already cited; no dataset is ingested and no right is implied. |
| verified against permitted real observations | Dated evidence and written permission for the exact operation exist; unavailable here and therefore blocked. |
| blocked | The required authorization, right, dataset or evidence is absent; no claim is admitted. |

## 3. STOP rule

If evidence or authorization is missing, STOP. A missing item must be recorded
as an owner dependency. A synthetic artifact, a public page, a paid tariff or a
prior session must not be substituted for the missing evidence. Work resumes
only after the owner supplies the written evidence or explicit authorization.

## 4. Relation to other records

- `engineering/research/dataset-acceptance-contract.md` is BLOCKED; this record
  does not change that state.
- `research/protocols/NSE-CASH-MOMENTUM-PROTOCOL-v1.md` remains Proposed and
  BLOCKED; amendment 1 applies the synthetic-data prohibition to it.
- `evidence/M4-2-1-CALENDAR-CADENCE-ADDENDUM-2026-11-22.md` and the earlier M4
  records describe implemented work only and are not real-market evidence.
- The zero-cost source-availability report is SOURCE_DOCUMENTATION_ONLY; it is
  not a dataset acceptance decision.

## 5. Scope limits

No provider service, credential, keychain, account, download, experiment, test
fixture or live endpoint was accessed to produce this record. No code path,
rights registry, capability record or claim record is changed by this record.
