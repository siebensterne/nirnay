# Test retirement record — Nirnay — 2026-11-23

**Status:** executed under owner directive D11 and the RRP-002 gate. This is a new dated record;
historical W1.1/W1.2 and 2026-11-23 policy counts and digests are preserved unchanged. No
provider was contacted, no right was changed, no experiment was run, no synthetic replacement
data was created and no Git history was rewritten. No commit was created in this worktree.

## Scope and base

- Repository: `nirnay`; worktree branch `chore/rrp-002`; reviewed base
`f6842d2b6438c657c4d4eaec8edb3920193d2181`.
- Authority: `engineering/retirement/RRP-001.md` (D11), `RETIREMENT-ACCEPTANCE.md` (RRP-002),
`NIRNAY-RETIREMENT-MANIFEST.csv`, `DEPENDENCY-CROSSWALK.csv`.
- All 76 manifest rows were hash- and size-verified against the reviewed base before removal.
Every one of the 58 `DELETE` rows was removed individually with `git rm`; blobs and history are
preserved. The 4 `KEEP` pure numeric corpus files and the 14 `CONDITIONAL` test files were
retained in the tree.
- Source-ingestion and raw-archive paths (`india-reference`, `market-archive`, `upstox-tools`,
`fyers-importer`, `fyers-recorder`, `upstox-feed-proto` decoder source) are untouched. They
belong to the RRP-004 freeze/migrate gate, not to RRP-002.

## Removed paths (per-row path, SHA-256, size at reviewed head)

| Manifest group | Count |
|---|---:|
| D11-A synthetic strategy generator/golden | 3 |
| D11-B FYERS synthetic session fixture | 3 |
| D11-C archived synthetic research fixtures | 14 |
| D11-D synthetic India reference records | 13 |
| D11-E synthetic feed fixture | 1 |
| D11-F synthetic fundamentals | 2 |
| D11-G market-shaped KDB golden | 4 |
| D11-H event/simulation golden | 3 |
| D11-H market-state oracle | 3 |
| D11-H order-book golden corpus | 5 |
| D11-H order-book verification corpus | 7 |
| **Total** | **58** |

Each row below carries the exact manifest path, SHA-256 of the reviewed bytes and byte size. The
CSV itself is the hash record outside the active tree; this record repeats it for review.

| Path | SHA-256 | Size (bytes) |
|---|---|---:|
| `crates/strategy-runtime/tests/generate_golden.py` | `cd70e7666a2ac3a155915795534937dce647a6f02f1f8285f1777850d9b676c0` | 4400 |
| `crates/strategy-runtime/tests/golden/artifact-v1.json` | `3e2f35aaca4069a508e1e535ef181e436b6b0e3d08366fc0307ffca047a66775` | 640 |
| `crates/strategy-runtime/tests/golden/ema-rational-v1.json` | `7865871616df84f246d0201d458e94a18bdc09f937b341889de7ef47e65d5f7d` | 3649 |
| `apps/fyers-importer/tests/fixtures/session/fyers-sdk-observations.jsonl` | `7a5408d02a5eab7111e61c23762f897d3fafe2612234830130ed259f59b47e80` | 1320 |
| `apps/fyers-importer/tests/fixtures/session/manifest.json` | `5f1dfb65adf6841709cf6f061b0f0a98ab52e4fb335ad8d67e24b505aba042a7` | 518 |
| `apps/fyers-importer/tests/generate_fixture.py` | `7e9d2de55e59fdaf5010d17894c40159bdaae6bac649b9a85158a4c87d7b5fe5` | 2644 |
| `research/python/fixtures/ARCHIVED.md` | `8e4fa63e469f34e52ce86e0b3dcf2d857e7ca3f441945897c6412578f3b2c90b` | 1523 |
| `research/python/fixtures/README.md` | `9d7e7ea4f2245d0a57b843b603b600017b818795bc8aadde6a04558b069959f3` | 1818 |
| `research/python/fixtures/alpha-bars-minute.seed-20261121.csv` | `587dd91454e2e64def729d64db93910b35eb52b05cb056f7b48456824142b61b` | 20892 |
| `research/python/fixtures/alpha-bars.synthetic.csv` | `0348c1437ad9930ea6d6d143b25b6bf3cd2ede9664ee3c58adbae2cb5b539023` | 14698 |
| `research/python/fixtures/calendars/market-fixture-daily-v1.json` | `c7e05dd8308637a019ce46437e59a1cc04c39a654f9ba97fd95034c27c9a7eff` | 618 |
| `research/python/fixtures/calendars/market-fixture-minute-v1.json` | `4cd50e85c491a7749e7897f4e1a5ba65fbb75c4e5d4ed547b81da6fcbd671446` | 766 |
| `research/python/fixtures/calendars/market-fixture-overnight-v1.json` | `508743afd27c6771ae1ec20d9f8571f2aa535946689c892816cb07a9c562cd28` | 548 |
| `research/python/fixtures/calendars/synthetic-event-sequence-v1.json` | `d21c4d1e367a2c837c7a2cbffe064bc0cbde331445bf3b47dde78d5009640dc8` | 498 |
| `research/python/fixtures/calendars/synthetic-fixed-15m-v1.json` | `19d38a66a73cf2ad3730d4abf614dbf4ab7cf4dae172310fc4b9b3030e51ce9e` | 461 |
| `research/python/fixtures/calendars/synthetic-fixed-1d-v1.json` | `6207bb7a38402bac10e50af282afa157ccb58c10f55e0470a68d067267878d8d` | 454 |
| `research/python/fixtures/calendars/synthetic-fixed-1m-v1.json` | `54240d7449d71e9516dc0fc548def33c57e503abcbee1b04375113b6047cb7b1` | 455 |
| `research/python/fixtures/calendars/synthetic-fixed-5m-v1.json` | `a6c040ce57e54b3c04a9d6c9538bc810420892a6da0ebd66e048989ec64c83c9` | 457 |
| `research/python/fixtures/canonical-events.example.jsonl` | `c6738a966f6e3eab1a9614a4426f45a9d9dfeb0b6a5a96f1523b847151032b1c` | 2696 |
| `research/python/fixtures/reference-data.example.json` | `a9003e2b7c934af7a0ed5bcf6a96dfbb24c4568fc963d312ffe9496dec1adcbe` | 1665 |
| `fixtures/india/bse_listing.json` | `c0b79ff3ddd7c5f8f9aae71288771284710f1c1483e7238270fd75f02117a7d6` | 869 |
| `fixtures/india/etf.json` | `0f7f98de3bbe39b6e6bb4402e2be47a871fb546a77fb2d15119cfd936577d7d4` | 814 |
| `fixtures/india/invalid_date.json` | `0a42d576e6c9cd569c006b44f36b539990ea04438409adb95082c73374d7aa86` | 836 |
| `fixtures/india/invalid_instrument_id.json` | `b2da881a272344a646dce2f3212102ccacc5d796a9497f0dac1c9fb49993c7c3` | 822 |
| `fixtures/india/invalid_isin_check_digit.json` | `61f74a06cac8c6553910c322cd46bdd2d7040a4d483539092c41f25e46608552` | 836 |
| `fixtures/india/invalid_isin_format.json` | `7f4d66c1d26ff9d43bf7a88e544ded5ee72cb83541045d84402b15790574176b` | 835 |
| `fixtures/india/missing_listing_symbol.json` | `1512293dbf89cce0d7ce6894add6d255fa39d2174c0e966f333abcb77831feeb` | 802 |
| `fixtures/india/negative_market_lot.json` | `0b9cff3412b6cec9ea6266f56b546695c65cbf11d7b6336f0ffb1273fd4ae387` | 837 |
| `fixtures/india/nse_cash_equity.json` | `1b65951705f21eb8cf03eb1416b801a80d8bfc37e6e9c1a6436ed10751a7b953` | 836 |
| `fixtures/india/overlapping_listing_terms.json` | `32be4389914ddbdfa8204993860c4e7b076b38c5763702324edef3d1d0992a0f` | 1188 |
| `fixtures/india/two_venue_listings.json` | `01e71b6de194a3231688723f15e2d547eb827ebf2ebb8b73bda34b1164bae33c` | 1674 |
| `fixtures/india/unknown_property.json` | `384d2456399d3cac933f1367fed02137d28cd5dfdb4f395f470f003280a2adc3` | 879 |
| `fixtures/india/zero_tick_size.json` | `06c239a10d87ea802dc2fb228ad21f90586d96ba6b9111e0360836de7160478e` | 836 |
| `crates/upstox-feed-proto/tests/fixtures/market-data-feed-v3.jsonl` | `2264bec0d5b81832f687acc3cafe84afc1a2d6197fa95cb81fb933332c735300` | 763 |
| `research/fundamentals/fixtures/company-identities.example.jsonl` | `579bfc74003a9d7019cfee6a67f04faed40fc221c7a6b1964845c04f53eec393` | 539 |
| `research/fundamentals/fixtures/financial-facts.example.jsonl` | `c4ed414eb1e0c6060d644b3386e9ff8aa912c1c9d94504da5448fbd4ba51a419` | 1023 |
| `analytics/kdb/tests/mbp-unit.csv` | `61663e5e57710865508bf2c35d829d44b9d5f45e6c16d5b02c500e18d7c1db5b` | 680 |
| `apps/kdb-lab/tests/golden/mbp-metrics-v1.csv` | `b3b6566e24097a14432e3dadd19ca7e8f81ac43a9b31bec5106c92c690d36a61` | 296 |
| `apps/kdb-lab/tests/golden/mbp-observation-v1.csv` | `40811f73f7d3e9a4b1a73920b0a794db332585d040be891dba297f185f6178a5` | 363 |
| `apps/kdb-lab/tests/golden/mbp-report-v1.json` | `814138d498e18d740776ea081a45dc6eb0a4f0fea4251a40f1365acfb9846982` | 890 |
| `crates/market-event/tests/golden/derived-event-v1.json` | `fbe84b01854d2cc7fab588793e6f3b1077ce7a1da4be824679c90e8c4c4bc301` | 1147 |
| `research/python/tests/golden/simulation-event.valid.json` | `5fe6c9580b7397963fa030ceeb37a04b482cc6423b6e66d31c8fbbbe8ffefbf0` | 739 |
| `research/python/tests/golden/simulation-run-record.valid.json` | `9965f16e028dea9111477fa91e21943d4dd2a0d9e44466b7af50eaa538ea96b5` | 6173 |
| `verification/cpp-oracle/src/book_driver.cpp` | `c03e41e31745011147e7b6233518e70a6fa31bb978a503a0ea1c760f1d56c0ec` | 1872 |
| `verification/cpp-oracle/src/market_state.cpp` | `e88c8c82cf617e828b9f2de35aae13f35bf71eda1b91e51b9f20c62248940e8a` | 3760 |
| `verification/cpp-oracle/src/market_state.h` | `577c42bd85820101eb9c1a76ce460b18faf6969a40838fad4185ac09db28c4fe` | 2175 |
| `crates/sbe-codec/tests/golden/baseline_pre_types_book.meta` | `8de47ced45b61677931981ff81ec965f3f3276e796ae5db828be5ac6bad78242` | 400 |
| `crates/sbe-codec/tests/golden/cpp_canonical_dump.txt` | `aa555aae39f9cd30f911e533d1a2227f2d6a14425c906b472be7473c3b2a2a5c` | 944 |
| `crates/sbe-codec/tests/golden/cpp_canonical_dump.txt.sha256` | `9a3b662b526f94574312baf7e91d80e582431614aa6138bb9757e48a532dca27` | 65 |
| `crates/sbe-codec/tests/golden/sample_replay.sbe` | `022cb71b7f2c7d9e5200a60096b3ffcd39dbe8c03bd69cd1c92a196d6aac6590` | 4096 |
| `crates/sbe-codec/tests/golden/sample_replay.sbe.sha256` | `2ea601be5aefb2116aff10e689c3a257d2474b48914675adc7b32ebe5da7ffd0` | 97 |
| `verification/corpus/book/basic.txt` | `1f4f4a86f20d31f8f1305216b597a82f77f0d3f097dbe0e1cc5f9ebf84daf72f` | 211 |
| `verification/corpus/book/boundaries.txt` | `7517990a5e425d168cec2ecb847616cf767b06ec838f3bfcd6d9a92eab23bc59` | 115 |
| `verification/corpus/book/invalid_negative.txt` | `ee35b188c03bd43d004e0409fb06823213095897cda91cf5e7139eea6899c3cb` | 48 |
| `verification/corpus/book/invalid_op.txt` | `f5f4d1a4a4a9de3c10de60e13598d3766aed26d3aee940ebe05efdd8c2af4305` | 52 |
| `verification/corpus/book/invalid_overflow.txt` | `20d28cf08ee89d5078f849624c647d310de14f057d421177b60c10e673350320` | 63 |
| `verification/corpus/book/ordering.txt` | `6f0aac16474249a9830356ced7646bfae116683e09bb2ccedda4c9606fa61584` | 211 |
| `verification/corpus/book/replay.txt` | `7aeec422df75325ff95f75f3a02071d07fd5c39262f054c1131e6651f620f6e6` | 230 |

## Additional retired test source (not a manifest path)

- `crates/market-book/src/bin/book_driver.rs` — market-shaped order-book differential driver,
removed when the book corpus and C++ market-state oracle left the tree.
- `apps/kdb-lab/tests/q_replay.rs` — golden-replay integration target; its only test replayed a
synthetic market golden.
- `research/python/tests/test_fyers_projection.py` — every case consumed the retired fabricated
FYERS journal; no pure case remained.

## Function-level removals in retained test modules

Per the manifest `CONDITIONAL` rows, production crates and modules were kept. The following
market-shaped cases/builders were removed in place:

- `crates/strategy-runtime/tests/parity.rs`: generator-parity, synthetic artifact, golden-vector
and generated-price cases; retained artifact-schema, forged-artifact and event-surface checks,
now built from pure contract text.
- `crates/market-event/tests/kernel_contract.rs`: derived-event golden assertion and ignored
golden-emission case; retained abstract state-machine/invariant cases.
- `crates/sbe-codec/tests/differential.rs`: golden capture, C++ dump and golden-reencoding
cases; retained arbitrary-byte, malformed-input, tear and invalid-UTF-8 codec cases.
- `crates/india-domain/tests/{temporal_compat,india_domain}.rs`: cases reading fabricated India
listings; retained pure timestamp/civil-date and schema-boundary cases.
- `crates/upstox-feed-proto/tests/decoder.rs`: captured synthetic feed-frame case and its base64
helper; retained in-process protocol and schema-hash cases.
- `apps/fyers-importer/src/main.rs`: module of fake-session verification/import cases; importer
source retained for RRP-004 migration.
- `apps/fundamental-tools/tests/contract.rs`: cases that included invented financial-fact and
company-identity files; retained empty-input digest, CLI fail-closed and contract-vector cases.
- `apps/kdb-lab/src/{observation,metrics,reference,report}.rs` and `src/main.rs`: `include_str!`
golden modules, `check-golden` command and golden replay helper; retained CLI parsing and pure
numeric contracts.
- `apps/paper-runner/tests/certification.rs`: 6 end-to-end synthetic paper/session scenarios;
retained 2 manifest-boundary audit tests.
- `apps/paper-runner/tests/recovery_drill.rs`,
`crates/paper-engine/tests/{capsule,engine,liquidity,paper_run,provenance,venue_matching,common/mod}.rs`:
all cases/builders constructed invented quotes, books, fills or datasets; files retained with
retirement notices, targets removed from `Cargo.toml`.
- `crates/risk-authority/tests/authority.rs`: quote-derived feed-health case; 27 rights, permit,
owner-token, concurrency and schema checks retained.
- `crates/risk-authority/tests/performance.rs`: seeded synthetic authorization benchmark and its
builders; scalar percentile test retained.
- `crates/risk-authority/tests/risk_controls.rs`: 4 invented sector/stress/greek/borrow
scenarios; permit-digest tamper audit retained.
- `crates/risk-engine/tests/risk.rs`: 11 market-observation, venue-eligibility, depth, greek,
borrow and stress cases plus the cross-venue builder; 16 scalar/bound/invariant cases retained.
- `verification/cpp-oracle/tests/numeric_orderbook_test.cpp`: order-book update and tick/lot
cases from the retired market-state header; pure parse/rescale/rational cases retained.

## Gating, commands and documentation updates

- `Makefile`: removed `fundamental-fixture-check`, `kdb-golden`, `kdb-q-test`, the `check`
dependency on the fixture check and the `kdb-golden`/`kdb-q-test` arguments; `bench-risk-oms` no
longer invokes the retired risk-authority synthetic workload; `cpp-test` builds only the pure
numeric oracle.
- `.github/workflows/ci.yml`: numeric differential job now builds only the numeric Rust driver,
passes no `--cpp-book`/`--rust-book`, and compares only `verification/corpus/numeric/`.
- `scripts/check_numeric_differential.py`: book corpus pair and arguments removed; schema
version 2; pure numeric corpus only.
- `Cargo.toml` targets for `strategy-runtime`, `fundamental-tools`, `india-domain` (india
fixture cases), `kdb-lab`, `market-book`, `paper-engine`, `paper-runner`, `risk-authority` and
`risk-engine`: removed `required-features` from retained pure/rights test targets and removed
fully retired targets. The `market-shaped-tests` feature is kept only where gated source test
modules still exist as the detector for reintroduced synthetic tests; it is not enabled by any
gate command.
- `pytest.ini` and `market_shaped_test_inventory.json`: the marker and the collection-time
detector are retained as rejection machinery, annotated as D11 retirement records;
`source_preserved` is now false; no previously prohibited test was re-enabled. The `conftest.py`
AST detector is unchanged in behaviour.
- Active `README.md`, `apps/kdb-lab/README.md`, `verification/cpp-oracle/README.md`,
`engineering/{capabilities,claims,extras-audit,migration-map,provider-environments,q-kdb-style}.md`,
`engineering/decisions/ADR-FND-001-*.md` and `claims/ledger.json` were updated so no active
reference points at a removed path. Historical evidence and dated records were left
byte-identical.

## Historical counts preserved (not current coverage)

| Record | Count | Status |
|---|---|---|
| W1.1-B Nirnay Python (`ba21945`) | 243 passed / 199 deselected / 2 subtests | historical, unchanged |
| W1.1-D Nirnay Python (`e975be9`) | 251 passed / 199 deselected / 2 subtests | historical, unchanged |
| W1.2-A synthetic policy record | 23 modules / 189 static names / 199 collected deselected / 90 marker-selected | historical, unchanged |
| W1.2-B Nirnay Rust | 711 preserved functions / 73 default / 638 withdrawn / 1 doctest | historical, unchanged |

## Current counts measured after removal

- `python3 -m pytest -q`: **288 passed, 199 deselected, 0 failed, 2 subtests passed**; the 4
FYERS projection cases were removed with their fixture, so no fixture-dependent test is skipped.
The 199 deselected count is unchanged: no prohibited test was re-enabled.
- `python3 -m pytest -q -m market_shaped_fixture --collect-only`: no tests collected (487
deselected).
- `cargo test --workspace --all-targets --locked`: **129 passed, 0 failed, 0 ignored**.
- `cargo test --workspace --doc --locked`: **1 passed, 0 failed**.
- `make cpp-test`: **1/1 CTest passed** (pure numeric oracle);
`scripts/check_numeric_differential.py` on `verification/corpus/numeric/` with both numeric
drivers: **gate_passed=true**.
- `make check` and every component static gate (docs, prose style, source style, Ruff added
lines, Rust style, generated paths, pollution, schema, Clippy `-D warnings`) passed in this
worktree.

## Withdrawn coverage

- Synthetic strategy golden generation, artifact approval claims and generated EMA vectors.
- Fabricated FYERS sessions, invented Upstox feed frames and fabricated India listing/ETF
calendar records.
- Archived synthetic price bars, reference data, canonical events and market calendars.
- Invented issuer identities/financial facts and their fixture validation command.
- KDB MBP observation/metrics/report goldens, q golden replay and golden-driven unit cases.
- Market-shaped event/simulation goldens and schema-vector golden assertions.
- Order-book golden corpus, verification book corpus, C++ market-state oracle, book differential
driver and Rust/C++ book comparison.
- End-to-end paper certification, recovery drill, engine/liquidity/provenance/venue-matching and
market-feed health scenarios; seeded authorization and synthetic benchmark cases.

## Retained coverage and production boundary

Pure scalar and exact-numeric tests, abstract timestamp tests, rights/authorization and
owner-token tests, schema and boundary checks, arbitrary-byte codec/storage tests,
hashing/digest tests and the pure numeric corpus remain active. No production crate, engine,
risk engine, decoder, acquisition module, generic transport module or
source-ingestion/raw-archive module was deleted. The only production-adjacent source edits
removed golden `include_str!` test modules and the KDB `check-golden` command; runtime logic is
unchanged.

## Residual gates

- No commit was created; acceptance in `REVIEW-LOG.md` and an exact new commit are still required.
- RRP-003 (Fieldbook product scope), RRP-004 (Nirnay ingestion freeze/migrate), RRP-005, RRP-006
and RRP-007 remain open.
- Rights remain `UNKNOWN`; no provider contact, collection, experiment, live execution or
data-rights change occurred.
