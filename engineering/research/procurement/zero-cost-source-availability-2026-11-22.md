# Zero-cost source availability report — 2026-11-22

## 0. Mandate and method

Objective: report which sources may be accessible at an incremental cost of INR
0, using the existing qualification dossier and the official public
documentation already cited in it. Allowed sources:
`engineering/research/procurement/m43-source-qualification-dossier.md`, the
evidence keys E1 to E19 defined there, and the dated records named below.

Non-goals and exclusions: no purchase, subscription, vendor contact, credential
use, API call, file download, provider request, dataset acceptance, right
change, registration, ingestion, experiment or code change. No new network
retrieval was performed for this report; the dossier records the public-page
access date as 2026-11-22. Status vocabulary is the dossier vocabulary:
Confirmed by official documentation / Confirmed in writing by provider / Not
documented / Not offered / Pending contract. No row here uses "Confirmed in
writing by provider".

## 1. Summary verdict

| Source | Documented at INR 0 | Status | Rights effect |
|---|---|---|---|
| Existing Upstox analytics entitlement | Service existence recorded; historical daily OHLCV scope not established in writing | Not documented for the required dataset | None; all rights remain `UNKNOWN` |
| NSE public historical reports | Report families documented | Confirmed by official documentation (E6) | None; all rights remain `UNKNOWN` |
| NSE all reports | Report families documented | Confirmed by official documentation (E7) | None; all rights remain `UNKNOWN` |
| NSE Indices historical reports and public data | Index values and current constituent fields documented | Confirmed by official documentation (E8, E9) | None; all rights remain `UNKNOWN` |
| Locally held data | No local dataset assumed or opened | Not documented | None; all rights remain `UNKNOWN` |

Documented access to a public page, a sample file or a prior session is not a
licence to capture, store, replay, back up or analyse data. No operation in the
data-rights registry becomes `PERMITTED` through this report.

## 2. Source findings

### 2.1 Existing Upstox analytics entitlement

Recorded basis: `evidence/M3-1-REAL-SOURCE-STATUS-2026-11-20.md` records one
bounded 90-second read-only Upstox V3 Full capture taken on 2026-11-20 from the
keychain analytics token under explicit owner authorization for that session.
`evidence/DATA-RIGHTS-2026-11-20.md` records all product rights as `UNKNOWN`.
No API call was made for this report.

| Dimension | Finding |
|---|---|
| Exact coverage at INR 0 | Service existence is recorded. Scope for historical daily OHLCV is not established in writing. Exact date range, fields and delivery terms are Not documented. |
| Instrument universe | The recorded session covers one instrument, `NSE_EQ|INE848E01016` (NHPC). A historical point-in-time universe is Not documented. |
| Delisted coverage | Not documented. |
| Corporate-action lineage | Not documented. |
| Benchmark history | Not documented. |
| Applicable public terms | No public terms for this entitlement appear in the dossier. The recorded authorization was session-scoped and read-only; all rights remain `UNKNOWN` under `configs/providers/source-products.v1.json`. |
| Permitted operations | None by this report. Retrieval, storage, replay, backup and analytics require written permission and owner approval for the exact operation. |
| Unresolved gaps | Written scope; date range; field inventory; whether historical daily OHLCV is offered; storage, replay, backup and internal-analytics rights; retention period; owner approval for any new operation. |

### 2.2 NSE public historical reports

Recorded basis: dossier sections A, B1 to B9 and evidence E6, with E2 and E5 for
terms. Status: Confirmed by official documentation.

| Dimension | Finding |
|---|---|
| Exact coverage at INR 0 | Daily and archived bhavcopy, security-wise price-volume, closing-auction, index, P/E, P/B, dividend-yield and total-return families are documented (E6). Earliest date and full date range are Not documented. |
| Instrument universe | Report families cover traded securities and archives. Point-in-time effective-dated universe is Not documented (B1). |
| Delisted coverage | Symbol-change history tables are documented, with a note to click an old symbol to view history (B2, E7). Delisting returns are Not documented. |
| Corporate-action lineage | A corporate-actions section exists; a dated machine-readable action feed with announcement and revision knowledge times is Not documented (B5). |
| Benchmark history | Historical index data, P/E, P/B, dividend yield and total-return values are documented (B8, E6). Effective-dated benchmark constituents are Not documented. |
| Applicable public terms | Data Sharing & Usage Policy (E5): data is supplied online through an NSE Data server; offline media require prior approval of the MD and CEO of NSE Data; reverse engineering is prohibited; audit and spot-check rights are reserved. E2 adds a written-consent condition for vendor display of EOD data. No right is granted by a public page. |
| Permitted operations | None by this report. Local storage, replay, backup and internal analytics remain `UNKNOWN`; each requires written permission and owner approval. |
| Unresolved gaps | Coverage start and end dates; digests (B9); a machine-readable calendar (B6); correction knowledge times (B7); effective-dated universe and benchmark constituents; written approval for offline media and research use. |

### 2.3 NSE all reports

Recorded basis: dossier sections A, B2, B7 and evidence E7, with E5 for terms.
Status: Confirmed by official documentation.

| Dimension | Finding |
|---|---|
| Exact coverage at INR 0 | Equities, indices, mutual fund, SLBS and SME report families are documented (E7). Report discontinuation notices carry circular references and adjustment disclaimers (B7). Per-file date ranges are Not documented. |
| Instrument universe | Traded securities across the named report families. Point-in-time effective-dated universe is Not documented (B1). |
| Delisted coverage | Symbol-change history is documented (B2, E7). Delisting returns and a deleted-security history are Not documented for this route. |
| Corporate-action lineage | A corporate-actions section exists; a dated machine-readable feed with revision lineage is Not documented (B5). |
| Benchmark history | Historical index values and total-return families appear in the report families (B8, E6, E7). Effective-dated constituent history is Not documented. |
| Applicable public terms | E5 policy as in section 2.2. E7 notices record report discontinuation and adjustment disclaimers. No right is granted by a public page. |
| Permitted operations | None by this report. Storage, replay, backup and analytics require written permission and owner approval. |
| Unresolved gaps | Coverage dates; per-report field inventory; digests; machine-readable calendar; correction knowledge times; written approval for offline media and research use. |

### 2.4 NSE Indices historical reports and public data

Recorded basis: dossier sections A, B1, B2, B5, B6, B8, B9 and evidence E8, E9,
E10. Status: Confirmed by official documentation for values and current
constituent fields; Pending contract for vendor terms.

| Dimension | Finding |
|---|---|
| Exact coverage at INR 0 | Index OHLC CSV fields Date, Open, High, Low, Close and Index Name are documented (E9). Index values and constituent names, identifiers, market capitalisation, weights and prices are documented (E8). Exact date range and series inventory are Not documented. |
| Instrument universe | Current constituent names, identifiers, market capitalisation and weights. Effective-dated constituent history is Not documented (B8). |
| Delisted coverage | Delisted-constituent history is Not documented (B2, E8, E9). |
| Corporate-action lineage | Not documented (B5, E8, E9). |
| Benchmark history | Index values and total-return families are documented (E6, E7, E8, E9). The specific NIFTY 500 TRI and NIFTY 50 TRI series together with effective-dated historical constituents required by the protocol are Not documented at INR 0. |
| Applicable public terms | Data Subscription page (E8); Authorized Data Vendors page (E10) lists channels with terms Not published, recorded as Pending contract; Data Sharing & Usage Policy (E5) applies. No right is granted by a public page. |
| Permitted operations | None by this report. Storage, replay, backup and analytics require written permission and owner approval. |
| Unresolved gaps | Effective-dated constituent history; specific TRI series availability; delisted-constituent history; reconstitution-calendar content, which was Not inspected (B6); digests (B9); vendor terms. |

### 2.5 Locally held data

| Dimension | Finding |
|---|---|
| Exact coverage at INR 0 | No local dataset is assumed or opened for this report. A prior session is recorded under `.local/live-sessions/2026-11-20/upstox/` outside Git; its use as research evidence is not established. |
| Instrument universe | Not established. The prior recorded session covers one instrument only. |
| Delisted coverage | Not established. |
| Corporate-action lineage | Not established. |
| Benchmark history | Not established. |
| Applicable public terms | None established. Written provenance and permission for the exact operation, scope, date range and retention period are prerequisites. |
| Permitted operations | None by this report. No local data file was read, copied, replayed or transformed. |
| Unresolved gaps | Provenance; source identity and digest; permission for storage, replay and internal analytics; coverage bounds; calendar and corporate-action lineage. Any locally held data may be reconsidered only after provenance and permission are established in writing; none is assumed here. |

## 3. Published tariff context

Dossier section D records domestic-client INR tariffs from the April 2026 NSE
schedule (E2, E3, E4). Figures are reproduced here as context only. No purchase
is recommended, no quote was obtained, and the incremental data budget is INR
0, so none of these figures is an authorized spend.

| Product | Published annual fee (INR) | Evidence |
|---|---|---|
| EOD — Capital Market | 1,00,000 per site, per medium | E2, E4 |
| Historical Trade — Capital Market | 1,10,000 per site | E4 |
| Historical O&T — Capital Market | 12,50,000 per site | E4 |
| Education Historical Trade — Capital Market | 22,000; eligibility not published | E4 |

Fees are exclusive of taxes and levies; each additional delivery channel adds
50% of the product fixed fee (E4). Minimum term, renewal, price protection,
storage charges and exit terms have no published values and are Pending
contract. The educational schedule refers to a SEBI circular dated 24 May 2024
and 29 January 2025 but does not reproduce the number cited in the dossier.

## 4. Verdict and gates

Dataset acceptance: **BLOCKED**. The dataset acceptance contract is not
satisfied. No source here has accepted coverage bounds, a qualified calendar,
effective-dated universe and corporate-action lineage, or written rights for
the intended operations.

This report: **SOURCE_DOCUMENTATION_ONLY**. No purchase, ingestion,
registration, experiment, holdout inspection or right change is authorized. No
credential, API call, download or provider contact was used. Written scope,
written permission and owner approval are prerequisites for the next step; a
missing item stops the work under `evidence/OWNER-DIRECTIVE-2026-11-22.md`.

SEBI circular `SEBI/HO/DEPA-II/DEPA-II_SRG/P/CIR/2024/178` remains unverified
from a primary source (E19). The dossier records that the SEBI site search was
blocked and that E4 refers to dated circulars without reproducing that number.
No status beyond "unverified" is asserted here.

## 5. Evidence key used

E1 NSE paid EOD/historical subscription page; E2 EOD tariff PDF; E3 products
tariff page; E4 domestic pricing PDF; E5 NSE Data Sharing & Usage Policy; E6
NSE historical reports; E7 NSE all reports; E8 NSE Indices data subscription;
E9 NSE Indices historical data; E10 NSE Indices authorized vendors; E19 SEBI
home and search route. Full URLs and access date are in section 0.3 and the URL
reference definitions of the qualification dossier.

## 6. Scope limits

This report restates public documentation already cited in the dossier. It
does not certify entitlement, source authenticity, completeness, correctness,
or any right. It does not make a profitability, alpha, venue-realism or
live-readiness claim. All registry rights remain `UNKNOWN`. No code, fixture,
dataset, capability record or claim record was changed by this report.
