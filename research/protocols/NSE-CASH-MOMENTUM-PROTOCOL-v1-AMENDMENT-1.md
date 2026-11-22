# Amendment 1 to the NSE Cash-Equity Momentum Research Protocol v1

## 0. Document control

| Field | Value |
|---|---|
| Amendment ID | `NSE-CASH-MOMENTUM-PROTOCOL-v1-AMENDMENT-1` |
| Date authored | 2026-11-22 |
| Amended document | `research/protocols/NSE-CASH-MOMENTUM-PROTOCOL-v1.md` (v1, unchanged) |
| Supersedes | The sentence quoted in section 1 below |
| Claim state | **Proposed** and **BLOCKED**; no experiment is registered or run |
| Governing authority | `evidence/OWNER-DIRECTIVE-2026-11-22.md` |
| Digest sidecar | `research/protocols/NSE-CASH-MOMENTUM-PROTOCOL-v1-AMENDMENT-1.md.sha256` |

This is a versioned amendment to v1. It does not rewrite the original protocol,
any dated evidence record, or any artifact under `evidence/m4/`. Where v1 and
this amendment differ, this amendment governs. V1 remains a historical design
record.

## 1. Explicit supersession

The following v1 sentence is superseded in full:

> Synthetic fixtures may exercise code paths but must not inform any parameter below.

That sentence appeared in v1 section 0. It is retained here only as the quoted
historical text being superseded. It has no authorization effect. It must not
be read as permission to use synthetic data to exercise a code path, to inform a
parameter, or to satisfy a protocol requirement.

## 2. Permitted inputs

No synthetic market data may be used for any protocol purpose. The prohibition
covers OHLCV and prices, trades and quotes and depth, market-shaped calendars
and session schedules, instrument universes, corporate-action histories, and
strategy or portfolio returns derived from any of those forms.

The only inputs permitted under this protocol are official reference
documentation and qualified real data. Qualified real data means a dataset that
has passed the dataset acceptance contract with written rights for the exact
operations and with dated evidence for every required field. A public report, a
published tariff, a sample archive, a prior provider session, a checked-in
fixture, or a generated series is not qualified real data.

Pure mathematical unit tests with scalars and abstract timestamps are outside
this protocol. They assert arithmetic only, represent no invented market
observation, and claim no exchange-calendar validity. A test that models a
session, a holiday, a bar anchor, a price path or a market-shaped sequence is
not a pure mathematical test and is not an input to this protocol.

## 3. Missing-price rule reconciliation

V1 section 3.3 permits substitution of the nearest valid price within two
sessions of a missing endpoint price. That rule is superseded. The following
rule governs:

1. No nearest-price substitution and no forward-fill may be applied across a
   data gap, a trading suspension, a halt, a delisting, or a missing session.
2. A price must not be invented where none existed. A bar absent from the
   qualified dataset stays absent. A coverage status on an absent observation
   records the absence; it does not create a price.
3. The actual observation date of every value used in a score or a return is
   preserved in the run record. A value must not be relabelled to an earlier or
   later date to fill a gap.
4. A security with a missing endpoint price, or with a price whose observation
   date does not match the required endpoint under the accepted calendar, is
   unscored for that formation. The exclusion is counted and reported.
5. Any substitution of one observation for another requires a separate
   versioned amendment. That amendment must state the economic interpretation
   of the substitution, preserve the actual observation date of each value, and
   identify the condition that makes the substitution admissible. No
   substitution is admitted by this amendment.

## 4. Calendar and coverage boundary

Session and holiday semantics must come from the qualified dataset declaration
and official reference documentation, not from a test fixture. Raw timestamp
differences do not define exchange sessions. Calendar provenance states
`TEST_FIXTURE`, `EXCHANGE_PUBLISHED`, `PROVIDER_PUBLISHED` and `OWNER_VERIFIED`
are recorded for every calendar artifact; a `TEST_FIXTURE` calendar cannot
satisfy market acceptance. Coverage bounds must be taken from the dataset
manifest, independently of the first and last observed timestamps.

## 5. Status and dependencies

This protocol remains Proposed and BLOCKED. Dataset acceptance and explicit
owner approval are prerequisites for registration, for a split freeze, for an
experiment run and for any holdout claim. No right is granted by this
amendment. All rights recorded as `UNKNOWN` remain `UNKNOWN`; the incremental
data budget is INR 0; no paid subscription or vendor is authorized.

Owner dependencies: written rights determination with document references and
review date; a qualified real dataset with effective-dated universe, actions,
calendar, costs and benchmark constituents; independent review sign-off; and a
dated freeze record for the split dates. If any of these is missing, work
stops under `evidence/OWNER-DIRECTIVE-2026-11-22.md`.

## 6. Review standard

| Dimension | Standard for this amendment |
|---|---|
| Question | Does the v1 protocol now exclude synthetic market data and unsubstantiated price substitution from every protocol purpose? |
| Evidence | This amendment, the unchanged v1 document, the owner directive and the dataset acceptance contract. |
| Assumptions | No dataset has been accepted; no right is established by a public page or a prior session. |
| Not established | Profitability, data rights, benchmark-constituent availability, cost-schedule completeness, dataset acceptance and exchange-calendar validity. |
| Failure cases | A synthetic artifact informs a parameter; a fixture calendar certifies market origin; a missing price is filled or substituted without a separate versioned amendment; a run is registered before acceptance and owner approval. |
| Inspection | Compare this text with v1 section 0 and section 3.3; confirm the v1 file digest is unchanged; confirm no registration, holdout or rights record was modified. |
| Independent review | Required before registration and before any holdout claim, covering financial method, point-in-time correctness, data rights and cost dating. |
| Owner dependency | Acceptance, rights, cost and benchmark sources, independent review and the split freeze record; none is satisfiable by an agent edit. |
