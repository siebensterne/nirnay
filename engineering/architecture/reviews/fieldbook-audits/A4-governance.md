# A4 — Governance audit (read-only)

Repo: `/private/var/folders/7c/m_f7hs9n445174c0x3gsy5kw0000gn/T/opencode/fieldbook-review`,
commit `aee91de`. No files modified, no commands from the repo run, no
network/provider/credential access, `/Users/tourist/Documents/fieldbook` untouched.
`docs/provenance-policy.md` is absent; `ORIGIN.md` and `NOTICE` were reviewed instead.

## Answer

**No.** The rights/certification types are genuinely fail-closed, but they are *dead code with
respect to collection and publication*. Nothing in the live collector, discovery, latency, or
export paths reads a `LicensingRegistry` or a certification verdict. A missing/unverified right
does **not** block retrieval or storage, and coverage certification does **not** block
publication. Under Nirnay's freeze (no provider retrieval without written permission), the code
must be treated as **not enforcing governance**.

## Verified implementation facts (code enforcement)

- `UsageRight`: 7 variants incl. `RealTimeAccess`, `HistoricalStorage`, `InternalProcessing`,
  `DerivedData`, `Redistribution`, `Display`, `NonDisplay`; `requires_explicit_grant()` covers
  storage/derived/redistribution (`src/governance/rights.rs:23-55`).
- `RightDecision`: `Permitted`, `Prohibited`, `ContractRequired`, `Unknown`, `Revoked`
  (`src/governance/rights.rs:108-130`). Spec lists only four decisions and omits `Revoked`
  (`docs/specifications/coverage-rights.md:44-53`) — doc drift.
- `AuthorizationQuery` identity: provider, source, dataset, product, territory (`CountryCode`),
  usage_right, recipient_class, purpose (`src/governance/rights.rs:132-157`);
  `AuthorizationIdentity` adds effective interval (`src/governance/rights.rs:159-222`).
  `DecisionAudit` requires non-empty reference/authority, revision > 0 and a 64-char lowercase
  hex document hash (`src/governance/rights.rs:224-250`).
- `grants()` denies outside the interval, denies Public recipient for retention/onward rights,
  and grants only `Permitted` (`src/governance/rights.rs:261-277`). `register()` rejects
  malformed intervals, malformed audits, same-revision decision changes and overlapping
  contradictions (`src/governance/rights.rs:347-385`). `authorize()` denies empty candidates
  (`MissingRecord`), revoked winners (`Revoked`) and non-grants (`RightNotGranted`), and returns
  an audit-bearing proof (`src/governance/rights.rs:386-447`). This is a sound in-memory
  fail-closed registry.
- **Per-source/per-venue?** Per provider, source, dataset, product, territory — but **not per
  venue**: `AuthorizationQuery` has no venue field, and `CoverageRecord::rights_query()`
  hardcodes `RecipientClass::Internal` and `Purpose::InternalAnalysis` and omits the venue from
  `CoverageScope` (`src/governance/coverage_certification.rs:486-503`). `conformance.rs`
  likewise hardcodes Internal/InternalAnalysis and uses `dataset` as product id
  (`src/integrity/conformance.rs:420-432`). Venue exists only in coverage identity
  (`src/governance/coverage_certification.rs:290-321, 544-603`).
- Certification: independent dimensions with `requirements_met()` requiring `rights ==
  Permitted` plus a separate registry proof
  (`src/governance/coverage_certification.rs:235-288`); `evaluate()` checks status, revocation,
  dimensions, operations, gaps, evidence, history, universe, identity,
  `rights.authorize(InternalProcessing)` and `quality.valid()`
  (`src/governance/coverage_certification.rs:666-755`); `certify()` returns `FailedRequirements`
  rather than promoting on failure (`src/governance/coverage_certification.rs:757-776`).
  `certifiable()` additionally requires a stored proof and non-empty evidence
  (`src/governance/coverage_certification.rs:655-664`).
- Only *production-ish* consumer of `authorize()` is `run_conformance()`
  (`src/integrity/conformance.rs:297-450`), an **offline fixture report**; it is called only
  from unit/integration tests (`tests/source_conformance.rs:205-320`). There is no CLI
  subcommand for it (`src/distribution/cli.rs:154-272`).

## Documentation policy (not code enforcement)

- Spec claims "The registry is fail-closed. A missing record, a missing right, a conditional
  right or an expired record denies the operation. No venue name or API availability implies
  permission" (`docs/specifications/coverage-rights.md:44-53`), and says it does not authorize
  new live collection (`docs/specifications/coverage-rights.md:75-81`). Code does not honor this
  outside tests.
- Spec points to `src/coverage_certification.rs` and `src/rights.rs`; actual files are
  `src/governance/*` (`docs/specifications/coverage-rights.md:10`; `src/lib.rs:34-38`).
- `ORIGIN.md` and `NOTICE` cover software license/originality only; `quality-contract.json`
  covers tool versions/lints, neither defines data rights. No `docs/provenance-policy.md`.

## Missing implementation

- No licensing data source: `FileConfig`/`Settings` have **no rights, licence, territory,
  purpose, recipient or registry fields** (`src/governance/config.rs:724-757`);
  `Settings::load()` reads TOML/env/CLI only (`src/governance/config.rs:786-856`).
  `LicensingRegistry` is in-memory, populated only via `register()` in tests/conformance tests;
  no loader, no persistence, no `at_us` wall-clock authorization at startup.
- No rights gate in `src/acquisition/**` or `src/distribution/coordinator.rs`:
  `coordinator::run_with_shutdown()` calls `validate()`, `preflight_output()`,
  `build_exchange(&settings, true)`, `bootstrap_markets(...)`, `capacity::plan(...)` only
  (`src/distribution/coordinator.rs:388-430`). The `true` is Kalshi **credential**
  `require_auth`, not entitlement (`src/distribution/coordinator.rs:121-145`).
- `certify()` / `evaluate()` have **no non-test callers** (only
  `src/governance/coverage_certification.rs:1020`). No publication/export gate:
  `AnalyticsExport` (`src/main.rs:176-188`) and `scripts/analytics/verify_export.py:63-125`
  check hashes, row counts and a `journal_provenance_status` label — never rights or coverage
  certification.
- `QualityReason::RightsDenied` is declared but never used (`src/integrity/quality.rs:76,97`).

## Defects / risks (bypass paths — not executed)

- **Default is live collection**: `cli.command.unwrap_or(Command::Collect)`
  (`src/main.rs:24-26`). Running the binary with no subcommand dials the configured venue with
  no rights check.
- **README quick start is a runnable live collection recipe** with no rights precondition:
  `cargo run --release -- --exchange polymarket --max-markets 20 ... collect`
  (`README.md:96-103`).
- **`config.example.toml` sets `environment = "production"` "because the challenge targets live
  markets"** (`config.example.toml:5-8`); `config.polymarket.example.toml` and
  `config.live-universe.toml` describe live retrieval and raise startup ceilings, with no rights
  field (`config.live-universe.toml:1-25`).
- **`Discover`, `Latency` and `NetworkProbe` subcommands contact venues**
  (`src/distribution/cli.rs:224-248`; `src/main.rs:129-175`) with no rights check.
  `NetworkProbe` documents "never authenticates ... never collects market data"
  (`src/distribution/cli.rs:236-239`) — transport only, but still provider contact.
- **Wrapper gate is self-attested and bypassable**: `scripts/run-live-evidence.sh:342-371` skips
  a venue unless `POLYMARKET_MARKET_DATA_ACCESS_CONFIRMED=1` /
  `KALSHI_READ_ONLY_ACCESS_CONFIRMED=1`; the runbook's access conditions are prose
  (`docs/live-evidence-runbook.md:3-40`). The binary itself has no equivalent gate; any of the
  above CLI paths bypasses the script.
- **Doc/code mismatch on rights scope**: spec speaks of per-country/per-source coverage and
  rights per provider/dataset/territory, but no venue dimension exists in the rights query, so
  per-venue rights cannot be expressed.
- **Two independent rights representations**: declared `CoverageDimensions.rights`
  (caller-supplied enum) and registry-derived `rights_proof`. A record can declare `Permitted`
  while registry state is stale; `evaluate` partially compensates by requiring a live proof, but
  only for `InternalProcessing`, so a grant for `HistoricalStorage`/`Redistribution` alone will
  never certify.

## Existing tests and their limits

- `src/governance/rights.rs:450-780`: ~11 unit tests cover effective dating, provider/territory
  scoping, conflicts, revocation/reinstatement, Public-retention denial, malformed
  interval/audit, proof audit identity. All in-memory; none exercises collection or storage.
- `src/governance/coverage_certification.rs:832-1288`: ~15 tests cover prerequisites, gaps,
  universe rules, revocation override, idempotency/conflict, single-oracle agreement, provider
  identity. All construct both registry and records by hand.
- `src/governance/adapter.rs:313-397`, `src/governance/modality.rs:311-443`: manifest/modality
  declaration completeness and consistency only — no rights.
- `src/integrity/conformance.rs:680-880`, `tests/source_conformance.rs:204-320`:
  `denied_right_blocks_certification` proves an empty registry fails the *fixture conformance
  report*; explicitly "not a live certification" (`src/integrity/conformance.rs:222`;
  `tests/source_conformance.rs:211-214`). No test asserts that `collect`/`discover`/`latency`
  refuses to run without rights, and no test ties certification to publication.

## Required changes

1. Define a persisted, signed licensing registry loaded from config/secure store into
   `Settings`; populate at startup with a wall-clock `at_us`; fail startup when required records
   are missing/unverified.
2. Gate `coordinator::run_with_shutdown()` on `authorize()` for `RealTimeAccess` (+
   `HistoricalStorage`/`InternalProcessing` where persistence is enabled) per
   provider/source/product/territory before `build_exchange`, discovery or any socket; extend to
   `Discover` and `Latency`; make `NetworkProbe` rights-aware or document provider contact as
   gated.
3. Remove/replace the no-argument live `Collect` default and the README live quick start with a
   fail-closed, permission-gated flow; add a rights precondition to example TOMLs and the
   live-evidence script that the binary itself enforces.
4. Make publication (`AnalyticsExport`, `verify_export`) require a valid coverage certification
   and rights proof; wire `certify()`/`evaluate()` into a real caller.
5. Add venue to `AuthorizationQuery`/`rights_query()` if per-venue rights are required; decouple
   recipient/purpose from hardcoded Internal/InternalAnalysis.
6. Tests: startup-denied-without-rights, storage-denied, publication-denied, and a documented
   no-provider-contact regression. Fix spec paths/`Revoked` drift.

## Decision: **CORRECT**

The governance model is well implemented in isolation, but collection and publication do not
fail closed: they have no rights or certification gate at all. Given the freeze on provider
retrieval without written permission, this must be corrected before reliance; no DEFER is
acceptable for the live paths.
