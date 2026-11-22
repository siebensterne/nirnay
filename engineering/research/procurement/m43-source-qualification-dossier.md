# M4.3 procurement/source-qualification dossier — NSE cash history

Research and documentation only. No rights are granted, changed or recorded as
permitted by this file. `configs/providers/source-products.v1.json` was not
edited; every right for every source product remains `UNKNOWN`.

## 0. Mandate, method and evidence key

### 0.1 Objective and bounds

Objective: assemble one procurement/source-qualification dossier for NSE cash
market history from public primary sources, then state a decision and a written
inquiry pack. Allowed files: this dossier and one README document-map row.
Non-goals: buying data, signing terms, configuring products, granting rights,
running trading experiments, or writing code. Exclusions: main worktree, Fieldbook
code, provider data, credentials, rights registry and live services. Acceptance:
every claim carries an evidence key and an access date; status vocabulary is
restricted to the five forms listed below; no written confirmation is asserted.

Status vocabulary, used verbatim: Confirmed by official documentation /
Confirmed in writing by provider / Not documented / Not offered / Pending
contract. No claim in this dossier uses "Confirmed in writing by provider":
no written provider statement exists in this worktree.

### 0.2 Retrieval record and limitations

Access date for every item below: 2026-11-22. The supplied `webfetch` route
timed out on all six supplied URLs, and `websearch` returned HTTP 402. The same
public URLs were retrieved with `curl` using a browser user agent; PDF text was
extracted with `pdftotext`; sample archives were inspected with `unzip` and
`head`. No credentials, accounts or paywalled areas were accessed. These are
public-page observations on the access date, not a certification of entitlement.

The verbatim owner message with the ten rights questions was not supplied to
this research subagent. The ten questions in sections C and F are a draft
derived from the seven-operation rights model in
`evidence/DATA-RIGHTS-2026-11-20.md` and the task brief. The owner must review
and adjust them before transmission; that review is an owner dependency.

SEBI circular `SEBI/HO/DEPA-II/DEPA-II_SRG/P/CIR/2024/178` was not verified
from a primary source. `https://www.sebi.gov.in` loaded; the site search POST
was blocked with HTTP 530 and the public sitemap contained no matching entry.
The NSE domestic pricing schedule (E4) refers to a SEBI circular dated
24 May 2024 and 29 January 2025, but does not reproduce that circular number.
The circular reference remains unverified.

### 0.3 Evidence key

Every claim below cites one or more of these keys. Each was accessed on
2026-11-22 and is a primary NSE/NSE Indices page, official PDF, or official
sample file. SEBI key E19 records a failed primary-source verification.

[E1] NSE, Paid End of the day/Historical Data page (updated 02/09/2026).
[E2] NSE, End of the Day Data tariff PDF, April 2026.
[E3] NSE, Products Tariff page, pricing effective 01 April 2026 (updated 22/07/2026).
[E4] NSE, Market Data Pricing — Domestic clients PDF, April 2026, marked
     "Non-Confidential".
[E5] NSE, Data Sharing & Usage Policy page (updated 11/12/2025).
[E6] NSE, Historical Reports — capital market daily/monthly archives page.
[E7] NSE, All Reports page (equities, indices, mutual fund, SLBS, SME).
[E8] NSE Indices, Data Subscription page.
[E9] NSE Indices, Historical Data page.
[E10] NSE Indices, Authorized Data Vendors page.
[E11] NSE, CM End of Day technical specification PDF, 13 October 2025.
[E12] NSE, CM Historical Trade Data technical specification PDF.
[E13] NSE, Historical Order & Trade Data specification PDF v1.19, 2026.
[E14] NSE, official Sample Files for CM Trade Data archive.
[E15] NSE, official Sample Files for CM Order & Trade Data archive.
[E16] NSE, official Sample Files for WDM Trade Data archive.
[E17] NSE, Data for Research page (Economic Policy Research).
[E18] NSE, Data for Analytics page (NSE Data Room), updated 04/11/2026.
[E19] SEBI home page and search route; circular not verified.

Verbatim URL reference definitions follow section G. Prose wraps at 100 columns;
URL definitions are verbatim addresses and are not prose.

## A. Source comparison

| Source / product | Status | Evidence |
|---|---|---|
| NSE public reports | Confirmed by official documentation | E6, E7 |
| NSE Data & Analytics EOD | Confirmed by official documentation | E1, E2, E11 |
| NSE Historical Trade Data | Confirmed by official documentation | E1, E12, E14 |
| NSE Historical Order & Trade | Confirmed by official documentation | E1, E13, E15 |
| NSE Indices subscription | Confirmed by official documentation | E8, E9 |
| Authorised index vendors | Pending contract | E8, E10 |

Documented offerings behind section A:

- Public reports: daily and archived bhavcopy, security-wise price-volume,
  closing-auction, index, P/E/P/B/dividend-yield and total-return families.
- EOD: binary end-of-day bhavcopy with security and trade details for CM, F&O,
  CD and WDM; specification and tariff published.
- Historical Trade: trade-level database with bhavcopy, masters, snapshots and
  circulars directories; sample archive published.
- Historical Order & Trade: order and trade ticks, Full and Trim variants, SFTP
  and cloud delivery; specification and sample archive published.
- NSE Indices: index values and constituent names, identifiers, market
  capitalisation, weights and prices; CSV index OHLC download.
- Authorised vendors: Bloomberg, FactSet, IHS Markit, MSCI, Rimes and Thomson
  Reuters are named as channels; terms are not published (E8, E10).

Notes on section A:

- "Not offered" is not used: no official page examined states that a candidate
  product is unavailable. Absence of documentation is recorded as "Not
  documented", not as a refusal.
- Vendor status is "Pending contract" because listing a vendor is documented,
  while the licence, price and field scope for this requester are not.
- E1 states that Market Data is provided online through an NSE Data server and
  that offline media need prior approval (E5); this constrains the local
  storage design and is carried into section C.

## B. Coverage matrix against the nine owner dimensions

Each line states a claim with one permitted status. A "Scope:" note narrows the
claim to documented sub-fields; it does not change the status.

### B1 Universe (point-in-time, including effective dates)

- Public reports: report families cover traded securities and archives;
  point-in-time effective-dated universe Not documented. Status: Confirmed by
  official documentation. Scope: report families (E6, E7).
- EOD subscription: daily security files and price-volume files documented;
  no effective-dated universe version stated. Status: Confirmed by official
  documentation. Scope: file coverage (E11).
- Historical Trade Data: monthly Masters snapshots under dated directories,
  symbol plus series uniqueness stated. Status: Confirmed by official
  documentation (E12, E14).
- Historical Order & Trade Data: symbol and series in order and trade records.
  Status: Confirmed by official documentation. Scope: fields (E13, E15).
- NSE Indices: constituent names, identifiers, market capitalisation and
  weights. Status: Confirmed by official documentation. Scope: fields (E8).
- Authorised vendor: effective-dated universe terms Not documented (E10).
  Status: Not documented.

### B2 Delisted securities

- Public reports: symbol-change history tables are documented, with a note to
  click an old symbol to view history. Status: Confirmed by official
  documentation. Scope: symbol changes only (E7).
- EOD subscription: delisting coverage Not documented (E11). Status: Not
  documented.
- Historical Trade Data: Masters has a Deleted flag; dated Masters snapshots
  exist. Status: Confirmed by official documentation. Scope: the flag (E12, E14).
- Historical Order & Trade Data: delisted-instrument coverage Not documented
  (E13). Status: Not documented.
- NSE Indices: delisted-constituent history Not documented (E8, E9). Status:
  Not documented.
- Authorised vendor: Not documented (E10).

### B3 ISIN / exchange identity / validity

- Public reports: security files and symbol-change history documented; ISIN
  field in report archives Not documented. Status: Confirmed by official
  documentation. Scope: symbol history (E7).
- EOD subscription: the CM specification examined does not list an ISIN field.
  Status: Not documented (E11).
- Historical Trade Data: Masters fields are ISIN, Symbol, Series, Name and a
  Deleted flag; sample row shows a blank ISIN for at least one instrument.
  Status: Confirmed by official documentation (E12, E14).
- Historical Order & Trade Data: symbol and series documented; ISIN Not
  documented. Status: Confirmed by official documentation. Scope: fields (E13).
- NSE Indices: "identifiers" stated without a named field list. Status: Not
  documented for specific identifier fields (E8).
- Authorised vendor: Not documented (E10).

### B4 Raw OHLCV and traded value

- Public reports: bhavcopy and price-volume archives documented. Status:
  Confirmed by official documentation (E6, E7).
- EOD subscription: documented fields include previous close, open, high, low,
  close, net traded value, net traded quantity and trades. Status: Confirmed by
  official documentation (E11).
- Historical Trade Data: bhavcopy fields include open, high, low, closing price,
  LTP, traded quantity, value of shares traded and number of trades. Status:
  Confirmed by official documentation (E12, E14).
- Historical Order & Trade Data: trade price and quantity and index-value file
  documented. Status: Confirmed by official documentation (E13, E15).
- NSE Indices: index OHLC CSV fields documented; these are index values, not
  security bars. Status: Confirmed by official documentation (E9).
- Authorised vendor: Not documented (E10).

### B5 Corporate actions with announcement, effective and revision dates

- Public reports: a corporate-actions section exists; dated machine-readable
  action feed Not documented. Status: Not documented (E6, E7).
- EOD subscription: the corporate-action file documents Record Date, BC Start
  Date, BC End Date, Expiry Date, ND Start Date, ND End Date and Purpose.
  Announcement timestamps and revision lineage Not documented. Status:
  Confirmed by official documentation. Scope: action-date fields (E11).
- Historical Trade Data: corporate-action flags are documented as ex-date
  indicators; dated action records Not documented. Status: Not documented.
  Scope: the owner's full requirement (E12).
- Historical Order & Trade Data: Not documented (E13). Status: Not documented.
- NSE Indices: Not documented (E8, E9). Status: Not documented.
- Authorised vendor: Not documented (E10).

### B6 Calendar with holidays, special sessions and auctions

- Public reports: holiday pages, closing-auction archives and call-auction
  material documented; machine-readable calendar Not documented. Status:
  Confirmed by official documentation. Scope: report families (E6, E7).
- EOD subscription: no holiday or session-calendar file in the examined
  specification. Status: Not documented (E11).
- Historical Trade Data: a circulars database is documented as a history of
  market design; a structured calendar is Not documented. Status: Not
  documented for the full requirement (E12, E14).
- Historical Order & Trade Data: record indicator distinguishes Pre-Open,
  Regular Market and Closing Auction; version history records dated changes.
  Status: Confirmed by official documentation. Scope: session markers (E13).
- NSE Indices: a reconstitution-calendar link is present; content Not inspected
  and trading-calendar coverage Not documented. Status: Not documented (E8).
- Authorised vendor: Not documented (E10).

### B7 Corrections

- Public reports: report discontinuation notices with circular references and
  adjustment disclaimers are documented. Status: Confirmed by official
  documentation. Scope: notices only (E7).
- EOD subscription: feed-level correction records with knowledge time Not
  documented. Status: Not documented (E11).
- Historical Trade Data: no correction-feed field documented. Status: Not
  documented (E12).
- Historical Order & Trade Data: specification version history carries dated
  revisions; data-level correction records Not documented. Status: Confirmed by
  official documentation. Scope: specification revisions only (E13).
- NSE Indices: Not documented (E8, E9). Status: Not documented.
- Authorised vendor: Not documented (E10).

### B8 Benchmark values and constituent history

- Public reports: historical index data, P/E, P/B, dividend yield and total
  return values documented. Status: Confirmed by official documentation (E6,
  E7).
- EOD subscription: index-related files appear in the CM file list; effective-
  dated constituent history Not documented. Status: Not documented. Scope: the
  full requirement (E11).
- Historical Trade Data: an Index directory and a sample index file are
  documented. Status: Confirmed by official documentation. Scope: values (E12,
  E14).
- Historical Order & Trade Data: a CASH_Index sample file is documented.
  Status: Confirmed by official documentation. Scope: values (E13, E15).
- NSE Indices: historical CSV fields are Date, Open, High, Low, Close and Index
  Name; constituent data fields are documented for subscription. Effective-
  dated constituent history Not documented. Status: Confirmed by official
  documentation. Scope: values and current constituent fields (E8, E9).
- Authorised vendor: Not documented (E10).

### B9 Provenance and digests

- Public reports: source identity and archive paths documented; file digests
  Not documented. Status: Not documented. Scope: digests (E6, E7).
- EOD subscription, Historical Trade, Historical Order & Trade: specifications
  document file names, dated directory structures and version history; no
  checksum, hash or digest field found. Status: Not documented. Scope: digests
  (E11, E12, E13).
- NSE Indices: Not documented (E8, E9).
- Authorised vendor: Not documented (E10).
- Consequence: Nirnay must compute and bind its own digests on receipt; no
  provider-supplied digest may be assumed.

### B10 Sample-field inventory where official sample files are documented

- CM End of Day: technical specification only; no EOD sample archive listed.
  Documented fields include Market Type, Series, Symbol, Security Name,
  Previous Close Value, Open Price, High Price, Low Price, Close Price, Net
  Traded Value, Net Traded Quantity, Corporate Indicator, Trades and 52-week
  high and low (E11). Status: Confirmed by official documentation.
- CM Historical Trade sample archive contains Bhavcopy, Circulars, Index,
  Masters, PDBhavcopy, Snapshot and Trades components (E14). Bhavcopy sample
  rows are pipe-delimited with date, symbol, series and price-volume fields;
  Masters sample rows are ISIN | Symbol | Series | Name | Deleted flag; Trades
  sample rows are flag | trade number | symbol | series | time | price |
  quantity; Snapshot sample rows carry order id, symbol, series, quantity,
  price, time, side and flags (E12, E14). Status: Confirmed by official
  documentation.
- CM Historical Order & Trade sample archive contains CASH_Index, CASH_Orders
  and CASH_Trades fixed-length files; length and field definitions are in the
  specification, including record indicator, segment, order number, jiffy
  timestamp, buy/sell indicator, activity type, symbol and series (E13, E15).
  Status: Confirmed by official documentation.
- WDM Historical Trade sample is a CSV whose header lists TRADE_NO, TRADE_TIME,
  SECTYPE, SECURITY, ISSUE_NAME, TRADE_TYPE, TRADE_DT, SETTL_DT, TRADE_QTY,
  PRICE_RATE, PRICE_YIELD, ISSUE_DATE, COUPON FREQUENCY and MAT_DATE (E16).
  Status: Confirmed by official documentation.
- Sample inspection was field-inventory only. No distribution claim, no
  completeness claim and no licence claim follows from sample access.

## C. Rights matrix

All registry rights remain `UNKNOWN`; no row below grants a right. Licence
scope and retention period are recorded as `UNKNOWN` where public documents
are silent. Evidence E5 is the public Data Sharing & Usage Policy; E4 is the
published tariff; E2 adds a written-consent condition for vendor display of
EOD data to clients; E18 describes a separate research-room route.

### C1 Mapping of Nirnay's seven operations

- Capture — licence scope `UNKNOWN`; retention period `UNKNOWN`.
  Restriction: data is supplied online through an NSE Data server; order forms
  and a Relevant Agreement govern handling (E5). Status: Pending contract.
- Store — licence scope `UNKNOWN`; retention period `UNKNOWN`.
  Restriction: offline media such as hard drives or removable devices are not
  permitted without prior approval of the MD and CEO of NSE Data (E5).
  Status: Pending contract.
- Replay — licence scope `UNKNOWN`; retention period `UNKNOWN`.
  Restriction: reverse engineering is prohibited and no ownership is conferred
  (E5). Status: Pending contract.
- Backup — licence scope `UNKNOWN`; retention period `UNKNOWN`.
  Restriction: offline copies fall under the same approval clause (E5).
  Status: Pending contract.
- Internal analytics — licence scope `UNKNOWN`; retention period `UNKNOWN`.
  Restriction: the policy's definition of Research Activity is for purposes
  other than trading or profit; conflict-of-interest and confidentiality
  declarations apply to research data (E5). Status: Pending contract.
- External redistribution — licence scope `UNKNOWN`; retention period
  `UNKNOWN`. Restriction: redistribution is prohibited except as agreed in
  the Relevant Agreement; vendor clients require written NSE consent for EOD
  (E2, E5). Status: Pending contract.
- Retention — licence scope `UNKNOWN`; retention period `UNKNOWN`.
  Restriction: the policy reserves audit and spot-check rights over records
  and electronic documents (E5). Status: Pending contract.

### C2 Mapping of the ten owner questions (draft — owner review required)

- Q1 Local storage on disk: scope `UNKNOWN`; retention `UNKNOWN`; offline-media
  approval clause applies (E5). Status: Pending contract.
- Q2 Offline replay after the subscription ends: scope `UNKNOWN`; retention
  `UNKNOWN`; no public statement (E5). Status: Pending contract.
- Q3 Backup and disaster-recovery copies: scope `UNKNOWN`; retention `UNKNOWN`;
  offline-media clause applies (E5). Status: Pending contract.
- Q4 Internal quantitative research use: scope `UNKNOWN`; retention `UNKNOWN`;
  research-route conditions documented (E5, E17, E18). Status: Pending contract.
- Q5 Own-account systematic trading from derived signals: scope `UNKNOWN`;
  retention `UNKNOWN`; research definition excludes trading or profit (E5).
  Status: Pending contract.
- Q6 External redistribution, client delivery or publication: scope `UNKNOWN`;
  retention `UNKNOWN`; restriction in E2 and E5. Status: Pending contract.
- Q7 Maximum retention period and deletion after termination: scope `UNKNOWN`;
  retention period `UNKNOWN`; no published period (E4, E5). Status: Pending
  contract.
- Q8 Licence scope, named user, site count and display medium: scope `UNKNOWN`;
  tariff is per annum per site and per medium (E4). Status: Pending contract.
- Q9 Minimum term, renewal and price protection: scope `UNKNOWN`; no published
  term (E3, E4). Status: Pending contract.
- Q10 Taxes, delivery channels, storage charges and exit terms: scope
  `UNKNOWN`; taxes extra and extra delivery channels add 50% of product fixed
  fee (E4). Status: Pending contract.

### C3 Research-route observations

- E17 states that EPR facilitates access to datasets for approved research
  projects in academic and policy research. Status: Confirmed by official
  documentation.
- E18 describes a cloud-based NSE Data Room, access under NSE Data Dissemination
  and Operational Guidelines, proposals to EPR, referee approval and time-bound
  grant access. Status: Confirmed by official documentation. Scope: the route.
  Status: Not documented. Scope: local storage and replay.

## D. Financial comparison from published tariffs

All figures below are domestic-client INR amounts from the April 2026 NSE
schedule (E4), cross-checked against the standalone EOD tariff (E2). E3 states
pricing effective 01 April 2026. No quote, discount or VAT/GST-inclusive total
was obtained. No purchase is recommended.

| Product | Fee INR/annum | Unit note | Evidence |
|---|---|---|---|
| EOD — Capital Market | 1,00,000 | per site, per medium | E2, E4 |
| EOD — Futures & Options | 1,00,000 | per site, per medium | E2, E4 |
| Historical Trade — Capital Market | 1,10,000 | per site | E4 |
| Historical Trade — Futures & Options | 1,10,000 | per site | E4 |
| Historical O&T — Capital Market | 12,50,000 | per site, single site | E4 |
| Historical O&T — F&O | 12,50,000 | per site, single site | E4 |
| Historical O&T — CM global | 31,30,000 | per site, multi-site | E4 |
| Education EOD O&T — CM | 6,25,000 | Students / Researchers | E4 |
| Education Historical Trade — CM | 22,000 | Students / Researchers | E4 |
| Education Historical O&T — CM | 6,25,000 | Students / Researchers | E4 |

Tariff conditions:

- Fees are exclusive of taxes and levies as may be applicable (E2, E4).
- The EOD schedule states that charges are per medium of data display and that
  website and mobile app count as two mediums (E2, E4).
- An additional 50% of product fixed fee applies for each additional delivery
  channel (E4).
- The educational schedule mentions a SEBI circular dated 24 May 2024 and
  29 January 2025 restricting near-real-time data for educational purposes
  (E4). The owner's cited circular number was not verified (E19).
- Eligibility of one individual for Students / Researchers pricing is not
  published. Status: Pending contract.
- Minimum term, renewal terms, price protection, storage charges, exit terms,
  refunds and termination obligations: no published values found. Status:
  Pending contract for every item.

Scope questions for the owner, not costed here:

- 5-year nominal fee questions: which single product and site count, and does
  any multi-year commitment change the published annual rate?
- 10-year nominal fee questions: does a renewal reset the rate, and is an
  archived copy permitted after the final term?
- 15-year nominal fee questions: is long-term retention priced separately, and
  would an additional delivery channel apply?
- Each question requires a written response; the tariff alone does not answer
  commitment, renewal or retention pricing.

## E. Final decision

`CONDITIONALLY_QUALIFIED`.

Public documentation supports coverage for the NSE cash history dimensions:
universe fields, ISIN and deleted flags, OHLCV and traded value, dated
corporate-action fields, session markers, index values and documented sample
files (E11 to E16). Rights for local storage, replay, backup, own-account
trading use and retention are not established in writing, and the tariff does
not answer term, renewal or exit questions (E4, E5). Written confirmation and a
budget decision are therefore prerequisites. This decision qualifies the source
for further written qualification only; it does not accept a dataset under
`engineering/research/dataset-acceptance-contract.md`.

Owner dependencies, stated explicitly:

1. Decide the budget ceiling among the published product tiers (E4).
2. Decide the product set, segment set, site count and delivery channel.
3. Decide the date range and whether delisted securities and corporate-action
   revision history are mandatory.
4. Review and correct the ten draft questions in section C2 before sending.
5. Send the written request in section F to the NSE Data Sales contact in E1
   and, for the research route, the EPR contact in E18.
6. Obtain legal or compliance review of the written reply before any right is
   marked `PERMITTED`.
7. Confirm whether the owner's cited SEBI circular is in scope; this dossier
   could not verify it from a primary source (E19).

No second decision token is asserted. The seven owner items remain open before
any dataset acceptance or right change.

## F. Inquiry pack — copy-ready written request

Subject: Written data-use qualification — NSE cash market historical data,
individual research requester in India

Body:

The requester is one individual resident in India. The intended use is internal
quantitative research on NSE cash market data and potential own-account
systematic trading. There are no clients, no external redistribution and no
public data service. Data would be held locally on the individual's own storage
and replayed offline for research. Please confirm the points below in writing.

Product and field request:

- Product names and exact segment scope: CM EOD, CM Historical Trade Data and
  CM Historical Order & Trade Data, with Full versus Trim variant stated.
- Field inventory for each product, including ISIN, symbol, series, validity or
  deleted flag, OHLCV, traded value, corporate-action dates, ex-date flags and
  session markers.
- Date-range availability for each product, including earliest date, monthly
  Masters snapshots, and any gaps or known corrections.
- Confirmation of which official sample files apply to the quoted product, and
  whether those samples carry the same field layout as the licensed feed.

Ten rights questions (draft — owner review required before transmission):

- Q1 Is local storage on the individual's own disk permitted, and on what
  conditions?
- Q2 Is offline replay after the subscription period permitted, and for how
  long?
- Q3 Are backup and disaster-recovery copies permitted, including their count
  and location?
- Q4 Is internal quantitative research on the data permitted, including
  publication-neutral internal notes?
- Q5 May signals derived from the data be used for the individual's own-account
  systematic trading, and is a separate trading or derivative-use licence
  required?
- Q6 Are external redistribution, client delivery or any public data service
  permitted? A negative confirmation is acceptable.
- Q7 What is the maximum retention period, and what deletion or certification
  is required after termination or lapse?
- Q8 What is the exact licence scope: named individual, number of users, number
  of sites and number of display mediums?
- Q9 What are the minimum term, renewal process and any price protection for
  the published annual tariffs?
- Q10 What taxes apply, what do additional delivery channels cost, are storage
  charges separate, and what are the exit and termination terms?

Commercial request:

- Confirm the published domestic annual fee for each requested product, the
  tax treatment, the minimum term and the renewal basis.
- State whether the requester qualifies for any Students / Researchers category
  and the evidence required for that category.
- State whether a multi-year commitment changes the published annual rate.

Closing: please reply on letterhead or from an authorised NSE Data Sales address
with a document reference and effective date for each confirmation.

## G. Review standard by deliverable

### G.A Source comparison

- Question: which NSE and vendor products are documented, and with what status?
- Evidence: E1 to E10; access date 2026-11-22.
- Assumptions: a public product page counts as official documentation, not as
  an entitlement.
- Not established: vendor terms, contractual scope, and any written
  confirmation.
- Failure cases: treating a vendor listing as a licence; treating a tariff as a
  quote; treating a sample as licensed data.
- Inspection: compare each table row against the cited page text and the
  status vocabulary in section 0.1.
- Independent review: procurement or legal review of product names and statuses.
- Owner dependency: choose products and contact sources (owner items 1, 2, 5).

### G.B Coverage matrix

- Question: do the nine owner dimensions have documented fields?
- Evidence: E6 to E16; access date 2026-11-22.
- Assumptions: field lists in specifications and sample headers are evidence of
  field availability, not of completeness or correctness.
- Not established: effective-dated constituent history, revision knowledge
  times, provider digests and a machine-readable calendar.
- Failure cases: reading an ex-date flag as a full corporate-action record;
  reading index OHLC as security OHLC; reading a sample as a completeness
  guarantee.
- Inspection: replay the cited field lists, then check each matrix line against
  the exact dimension wording.
- Independent review: data-engineering review of field semantics.
- Owner dependency: decide which gaps are mandatory (owner item 3).

### G.C Rights matrix

- Question: which of the seven operations and ten questions have written
  permission?
- Evidence: E2, E4, E5, E17, E18; access date 2026-11-22.
- Assumptions: the public policy binds the subscription relationship; silence
  is not permission.
- Not established: every licence scope and every retention period.
- Failure cases: changing a registry right on the strength of a public page;
  treating research-project access as a trading licence.
- Inspection: confirm that no row uses "Confirmed in writing by provider" and
  that every right remains `UNKNOWN`.
- Independent review: compliance or legal review of the written reply.
- Owner dependency: send questions and obtain reply (owner items 4, 5, 6, 7).

### G.D Financial comparison

- Question: which domestic tariffs are published, and which commercial terms
  are absent?
- Evidence: E2, E3, E4; access date 2026-11-22.
- Assumptions: numbers are transcribed from the April 2026 domestic schedule.
- Not established: taxes, minimum term, renewal, storage charges and exit
  terms.
- Not established: budget authority and product choice.
- Failure cases: treating a published tariff as a quote; assuming educational
  eligibility; multiplying an annual fee into a committed multi-year total.
- Inspection: re-read the PDF tables for section 3.0, 4.0 and 8.0 and compare
  with the 5/10/15-year question list.
- Independent review: finance review of any future written quote.
- Owner dependency: budget decision and product scope (owner items 1, 2).

### G.E Final decision

- Question: is the source qualified, rejected or awaiting owner input?
- Evidence: sections A to D; access date 2026-11-22.
- Assumptions: conditional qualification concerns source evidence only.
- Not established: dataset acceptance under the acceptance contract.
- Failure cases: treating `CONDITIONALLY_QUALIFIED` as permission to ingest or
  to change the rights registry.
- Inspection: check the decision token and the seven owner dependencies.
- Independent review: owner sign-off.
- Owner dependency: all seven listed items.

### G.F Inquiry pack

- Question: does the request describe the intended use exactly and ask the ten
  rights questions?
- Evidence: E1, E4, E5, E8, E18; access date 2026-11-22.
- Assumptions: the task brief supplies the intended-use description; the ten
  questions are drafted, not quoted from an owner document.
- Not established: owner approval of the draft wording.
- Failure cases: adding clients, redistribution or public-service language;
  omitting own-account trading; sending before owner review.
- Inspection: compare the body against the brief phrase by phrase and count ten
  questions.
- Independent review: owner and legal review before transmission.
- Owner dependency: review and send (owner items 4, 5).

### G.G Review standard itself

- Question: can each deliverable be re-checked from primary sources?
- Evidence: evidence key in section 0.3 and URL definitions below.
- Assumptions: public pages may change after the access date.
- Not established: future availability of the cited pages.
- Failure cases: citing a page without an access date; mixing SEBI secondary
  references with a verified circular.
- Inspection: re-fetch each URL, record a new access date, and diff the claim.
- Independent review: second researcher for every material claim.
- Owner dependency: decide whether a fresh retrieval is required (owner item 7).

## URL reference definitions

[E1]: https://www.nse.in/static/market-data/eod-historical-data-subscription
[E2]: https://nsearchives.nseindia.com/web/mediaattachment/2026-04/Download_End_of_the_Day_Data_Tariff_20260424120810.pdf
[E3]: https://www.nse.in/static/market-data/products-tariff
[E4]: https://nsearchives.nseindia.com/web/mediaattachment/2026-04/Download_Pricing_file_-_Domestic_clients_20260424122229.pdf
[E5]: https://www.nse.in/market-data/nse-data-policy
[E6]: https://www.nseindia.com/resources/historical-reports-capital-market-daily-monthly-archives
[E7]: https://www.nseindia.com/all-reports
[E8]: https://www.niftyindices.com/offerings/data-subscription
[E9]: https://www.niftyindices.com/reports/historical-data
[E10]: https://www.niftyindices.com/resources/authorized-data-vendors
[E11]: https://nsearchives.nseindia.com/web/mediaattachment/2025-11/EOD_CM_20251120105001.pdf
[E12]: https://nsearchives.nseindia.com/web/sites/default/files/inline-files/Data_details_CM_1.pdf
[E13]: https://nsearchives.nseindia.com/web/mediaattachment/2026-09/NSE_Hist_Order_Trade_Data_1.19_20260902144045.pdf
[E14]: https://nsearchives.nseindia.com/web/sites/default/files/inline-files/Download%20Sample%20Files%20for%20CM%20%28.zip%29_0.zip
[E15]: https://nsearchives.nseindia.com/web/sites/default/files/inline-files/Sample_CMsegment.zip
[E16]: https://nsearchives.nseindia.com/content/press/Sample_WDM.zip
[E17]: https://www.nseindia.com/static/research/initiatives-data-for-research
[E18]: https://www.nseindia.com/static/research/initiatives-data-for-analytics
[E19]: https://www.sebi.gov.in
