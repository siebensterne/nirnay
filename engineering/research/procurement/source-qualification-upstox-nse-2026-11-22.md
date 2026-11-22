# Phase 1 source qualification — Upstox V3 candles and NSE public reports

Documentation only. No right is granted, changed or recorded as permitted by
this file. Both sources end with rights `UNKNOWN`, dataset `BLOCKED`, collection
`BLOCKED`, no purchase, no ingest and no experiment.

## 0. Mandate, bounds and method

Objective: qualify two source routes — Upstox Historical Candle V3 and NSE
public reports — against one rights and coverage question set, using official
public documentation only. Allowed actions: HTTP GET of official public pages,
reading existing documents in this worktree, and writing this file plus two
README document-map rows. Excluded: credentials, keychain, provider accounts,
API calls, downloads, data files, experiments, code, the main and other
worktrees, Fieldbook, live services, purchase and provider contact.

The owner's message supplied the Upstox capability list as owner-reported
official documentation. Verification against public pages was performed on the
access date and is recorded per claim. No credential was used and no endpoint
was called. `webfetch` timed out on the NSE policy page; the same public page
was retrieved with `curl` and a browser user agent. `websearch` returned HTTP
402 and was not used as evidence. Two guessed Upstox candle URLs returned HTTP
404; the canonical URL was resolved from the official documentation hub (U5).
Failed routes are recorded here and are not cited as evidence.

Status vocabulary, used verbatim per claim: Confirmed by official documentation
/ Confirmed in writing by provider / Not documented / Not offered / Pending
contract. The status "Confirmed in writing by provider" is **NONE** in this
document. A public page documents an offering or a restriction; it does not
establish an entitlement.

Access date for every claim in sections 1 to 6: 2026-11-22. This is the date
the cited page was read, not a certification of continued availability.

URL definitions are verbatim addresses and are not prose. Table cells carry
structured claims and status tokens. Prose wraps at 100 columns.

## 0.1 Evidence key and reachability

[U1] Upstox, Historical Candle Data V3 API:
`https://upstox.com/developer/api-documentation/v3/get-historical-candle-data`
HTTP 200 via `curl`; access 2026-11-22.
[U2] Upstox, Rate Limits:
`https://upstox.com/developer/api-documentation/rate-limiting`
HTTP 200 via `curl`; access 2026-11-22.
[U3] Upstox, Market Data Feed V3 API:
`https://upstox.com/developer/api-documentation/v3/get-market-data-feed`
HTTP 200 via `curl`; access 2026-11-22.
[U4] Upstox, Analytics Token:
`https://upstox.com/developer/api-documentation/analytics-token`
HTTP 200 via `curl`; access 2026-11-22.
[U5] Upstox, official documentation hub `llms.txt`:
`https://upstox.com/developer/api-documentation/llms.txt`
HTTP 200 via `curl`; access 2026-11-22. Route index, not a licence text.
[N1] NSE, Data Sharing & Usage Policy:
`https://www.nse.in/market-data/nse-data-policy` redirected to
`https://www.nse.in/static/market-data/nse-data-policy`; HTTP 200 via `curl`;
`webfetch` timed out; access 2026-11-22.
[N2] NSE, All Reports (daily reports):
`https://www.nseindia.com/all-reports` redirected to
`https://www.nseindia.in/all-reports`; HTTP 200 via `curl`; access 2026-11-22.
[N3] NSE, Historical Reports — capital market daily and monthly archives:
`https://www.nseindia.com/resources/historical-reports-capital-market-daily-monthly-archives`
redirected to the `www.nseindia.in` host; HTTP 200 via `curl`; access 2026-11-22.
[D1] `engineering/research/procurement/m43-source-qualification-dossier.md`:
NSE dossier with evidence keys E1 to E19, public-page access 2026-11-22.
Cross-reference only; its coverage, tariffs and inquiry pack are not repeated.
[D2] `evidence/OWNER-DIRECTIVE-2026-11-22.md`: owner freeze directive D1 to D10.
[D3] `evidence/DATA-RIGHTS-2026-11-20.md`: tri-state rights registry source.
[D4] `engineering/research/dataset-acceptance-contract.md`: acceptance gates.

## 1. Upstox V3 — documented capability and data inventory

Every line states one claim with its status and evidence key. All keys above
were accessed on 2026-11-22. Owner-reported claims were checked against U1 to
U4; none rests on a provider reply.

| Claim | Status and evidence |
|---|---|
| Daily OHLCV from Jan 2000; one decade per call | Confirmed by official documentation — U1 |
| Minutes 1 to 15 from Jan 2022; one month per call | Confirmed by official documentation — U1 |
| Minutes above 15; hours 1 to 5; Jan 2022 | Confirmed by official documentation — U1 |
| One quarter per call for intervals above 15 minutes | Confirmed by official documentation — U1 |
| Weekly and monthly from Jan 2000; no limit stated | Confirmed by official documentation — U1 |
| One `instrument_key` in the request path per call | Confirmed by official documentation — U1 |
| `to_date` required; `from_date` optional | Confirmed by official documentation — U1 |
| 50/s, 500/min, 2000 per 30 min; suspension possible | Confirmed by official documentation — U2 |
| Full feed: displayed bid/ask levels, LTP/LTT/LTQ | Confirmed by official documentation — U3 |
| Order-by-order tape, MBO depth, queue position | Not documented — U3, U5 |
| Candle row: bar-start time, OHLCV, open interest | Confirmed by official documentation — U1 |
| `currentTs` receive time; `ltpc.ltt` trade time | Confirmed by official documentation — U3 |
| Candle correction, revision, knowledge time | Not documented — U1 |
| Token: read-only GET, one-year validity, free | Confirmed by official documentation — U4 |
| Licence, storage, replay, analytics, trading right | Not documented — U4, D2 |
| Written provider confirmation | Confirmed in writing by provider — NONE |

Consequences of the verified inventory:

- The owner-reported daily and minute windows match U1. U1 specifies minute
  intervals 1 to 300 and hours 1 to 5, so 1/5/15-minute candles are a subset
  of a broader documented set. Per-call windows are not a statement about the
  total history available for any instrument.
- One instrument key per request is a request-shape fact (U1). A universe
  needs one call per key; rate limits (U2) are capacity, not a data right.
- Displayed depth levels in U3 are quote levels. U3 does not document an
  individual order, an aggressor-tagged print, a matching-engine sequence or a
  queue position. A five-level quote tier is not an order-by-order tape tier.
  See `engineering/research/provider-normalization.md` for the recorded
  distinction between LTP snapshots and feed timestamps.
- The candle array carries OHLCV and open interest only (U1). It has no
  exchange event identifier, revision marker, digest or universe version. The
  `+05:30` offset on the sample timestamp is documented; the bar anchor and
  session definition are Not documented (U1).
- U4 documents an access credential and where GET calls may originate,
  including a laptop or serverless function. Authentication is not a source
  licence. Owner directive D4/D5 keeps rights `UNKNOWN` despite token access.
- "Not offered" is not used: no official page examined states that a candidate
  right is refused. Silence is recorded as Not documented.

### 1.1 Rights questions for Upstox

All rows use U1 to U4 and D2 to D4; access 2026-11-22.

| Rights question | Status and finding |
|---|---|
| Retrieval for research | Pending contract; GET documented — U1, U4 |
| Local storage and retention | Not documented — U4, D2 |
| Backup and disaster-recovery copies | Not documented — U4, D2 |
| Replay and reprocessing | Not documented; no revision stream — U1 |
| Internal analytics and backtesting | Not documented — U4, D2 |
| Own-account research and trading | Not documented; D4/D5 withhold it |
| Instruments, fields, intervals, range | Pending contract; scope not licensed — U1 |
| Personal account | Not documented; token per account — U4 |
| Local versus cloud | Not documented; call origins in U4 |
| Written confirmation | Confirmed in writing by provider — NONE |

## 2. Upstox — history, universe and reference-data gaps

| Dimension | Finding, status and evidence |
|---|---|
| History | Daily/weekly/monthly from 2000 — Confirmed by official documentation — U1 |
| Minute and hour coverage | From 2022 — Confirmed by official documentation — U1 |
| Active instruments | Key addressed per request; list Not documented — U1 |
| Delisted instruments | No equity history; expired F&O route in U5 — Not documented — U1 |
| Identity changes | No effective-dated key, symbol or ISIN map — Not documented — U1 |
| Corporate actions and corrections | No action or revision field — Not documented — U1 |
| Point-in-time universe | No membership version or knowledge time — Not documented — U1, U3 |
| Index and TR benchmarks | No constituent history or TR series — Not documented — U1, U3, U4 |
| Calendar and sessions | Auction IEP in U3; bar anchor and calendar Not documented — U3 |
| Revisions and retention | No correction feed or retention term — Not documented — U1, U4 |
| Candle field semantics | OHLCV and open interest — Confirmed by official documentation — U1 |
| Feed time semantics | `currentTs`, `ltpc.ltt` — Confirmed by official documentation — U3 |
| Rights and retention metadata | No such metadata documented — Not documented — U1, U4 |

## 3. NSE public reports — availability and terms

N2 lists the CM-UDiFF Common Bhavcopy Final (zip) among daily reports and notes
that the former CSV bhavcopy was discontinued w.e.f. 08 July 2024 with a
circular reference. N3 lists capital-market daily and monthly archives. Public
availability: Confirmed by official documentation (N2, N3). That status is a
page observation. It does not establish permission to automate retrieval, keep
a local archive, replay, reprocess or use the reports for proprietary trading
research; each of those is Not documented (N1, N2, N3).

N1 states that data is provided online only through a server designated by NSE
Data, that offline media need prior approval of the MD and CEO of NSE Data,
that a Relevant Agreement governs intended use and handling, that no ownership
is conferred, that redistribution requires agreement and that spot checks are
reserved. N1 defines Research Activity as research carried out for purposes
other than trading or profit and routes researcher requests through the
Economic Policy Research Department. These are Confirmed by official
documentation (N1). The owner's stated own-account trading and profit objective
is not covered by an assumed academic or non-commercial route (N1, D2).

Dossier D1 sections B and C hold the NSE file layouts, delisting flags, action
dates, session markers, index values, discontinuation notices and digests.
This document cross-references D1 rather than duplicating it. D1 records all
registry rights as `UNKNOWN` and its verdict as `CONDITIONALLY_QUALIFIED` for
written qualification only.

### 3.1 Rights questions for NSE public reports

Every row uses N1 to N3 and D1; access 2026-11-22. Cited policy statements are
documented restrictions; none is a grant to this requester.

| Rights question | Status and finding |
|---|---|
| Automated retrieval | Pending contract; online server delivery in N1 |
| Local storage and retention | Pending contract; offline media approval in N1 |
| Backup and disaster-recovery copies | Pending contract; same offline clause |
| Replay and reprocessing | Pending contract; reverse-engineering limit in N1 |
| Internal analytics and backtesting | Pending contract; research excludes profit — N1 |
| Own-account research and trading | Pending contract; no written grant — N1, D2 |
| Instruments, fields, intervals, range | Pending contract; report families in N2/N3/D1 |
| Personal account | Pending contract; N1 non-commercial class not established |
| Local versus cloud | Pending contract; local and cloud rights Not documented |
| Written confirmation | Confirmed in writing by provider — NONE |

## 4. NSE public reports — history, universe and reference-data gaps

| Dimension | Finding, status and evidence |
|---|---|
| Historical coverage | Daily and monthly archives; earliest date Not documented — N2, N3 |
| Active instruments | Traded securities; point-in-time list Not documented — N3, D1 |
| Delisted instruments | Symbol changes in D1 B2; no public-route flag — Not documented — D1 |
| Identity changes | Symbol tables in D1 B2; ISIN history Not documented — D1 |
| Corporate actions and corrections | Notices in D1 B7; revision lineage Not documented — D1 |
| Point-in-time universe membership | No effective-dated source — Not documented — N3, D1 |
| Index and TR benchmarks | Values in D1 B8; constituents/TRI Pending contract — D1 |
| Calendar and sessions | Holiday and auction reports in D1 B6; calendar Not documented — D1 |
| Revisions and retention | Notices in D1 B7; retention Not documented — N1, D1 |
| Field and timestamp semantics | Inventories in D1 B10; semantics/digests Not documented — D1 |
| Rights and retention metadata | No such metadata — Not documented — N1, N3 |

## 5. Survivorship warning and comparative verdict

Long history for today's instrument list can still be unusable for
survivorship-free research. Daily candles from 2000 for a present-day key say
nothing about firms that were listed and then merged, delisted or renamed, nor
about which instruments belonged to an index on a past date. Neither route
documents effective-dated universe membership, delisting returns, identity
changes or corporate-action revision knowledge times. A backtest over a
current-key history must not be labelled point-in-time correct; D4 sets the
acceptance rule and D2 D1/D6 forbid substituting such a series for an approved
dataset.

- Upstox V3 candles: technical availability Confirmed by official
  documentation (U1); rights `UNKNOWN`; dataset `BLOCKED`; collection
  `BLOCKED`; no purchase; no ingest.
- NSE public reports: technical availability Confirmed by official
  documentation (N2, N3); rights `UNKNOWN`; dataset `BLOCKED`; collection
  `BLOCKED`; no purchase; no ingest. Budget stays at INR 0 (D2 D8).

Both verdicts stand. No written provider confirmation exists, no right changes
from `UNKNOWN`, no dataset is accepted under D4, and no experiment is
registered or run. "Not offered" is not used: no official page examined states
that a candidate right is refused.

## 6. Owner dependencies and STOP rule

1. Supply written rights for the exact operation, instruments, fields, date
   range, account and retention period for each source; until then D2 D4/D5
   stop collection and storage.
2. Decide whether either source enters written qualification; the existing
   inquiry pack is in D1 section F and is not re-sent here.
3. Obtain legal or compliance review of any written reply before a right is
   marked `PERMITTED`.
4. Supply effective-dated universe, delisting, identity and corporate-action
   revision sources if survivorship-free research is required.
5. Confirm the budget remains INR 0 (D2 D8) before any commercial step.
6. Ratify the ownership split in
   `engineering/architecture/decisions/ADR-0001-data-ownership.md` before any
   integration design work.

A missing item stops the work under D2 section 3. No public page, tariff,
token, sample or prior session substitutes for the missing evidence.

## 7. Review standard

- Question: do the documented capabilities match the owner-reported list, and
  do any rights follow from those pages?
- Evidence: U1 to U5 and N1 to N3, each accessed 2026-11-22; D1 to D4 by
  cross-reference.
- Assumptions: a public page documents an offering or a restriction, not an
  entitlement. Failed retrievals are recorded, not cited as support.
- Not established: any written permission, retention period, point-in-time
  universe, revision stream or personal-account entitlement.
- Failure cases: reading a candle capability as a licence; reading a free
  token as a data right; reading report availability as automation or archive
  permission; treating a current-key history as survivorship-free.
- Inspection: re-fetch each URL, record a new access date and diff every
  claim; check that no row uses "Confirmed in writing by provider" as an
  affirmative answer and that both rights rows remain `UNKNOWN`.
- Independent review: procurement and legal review of any future written
  reply; architecture review of the ownership decision before integration
  work.
