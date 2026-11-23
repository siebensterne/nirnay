# RRP-005 consolidation record — Nirnay — 2026-11-23

**Status:** dated documentation and auxiliary consolidation record. This record
changes no data right, contacts no provider, accepts no dataset, registers no
experiment, enables no live execution and rewrites no Git history. No commit was
created in this worktree.

**Scope:** repository `nirnay`, worktree branch `chore/rrp-005`, reviewed base
`2c4f457` (merge of PR #135, RRP-004).

**Authority:** `engineering/retirement/RRP-001.md` §4 priorities P1 row 5 and P2
rows 1/3, §6 binding rules 1, 3, 4, 6 and 7;
`engineering/retirement/RETIREMENT-ACCEPTANCE.md` Gate RRP-005;
`engineering/architecture/decisions/ADR-0001-data-ownership.md` (Accepted
2026-11-23); `evidence/SOURCE-INGESTION-FREEZE-2026-11-23.md` and
`engineering/source-ingestion-freeze.v1.json` (RRP-004 metadata freeze).

## 1. Invariants

1. No gate was removed, skipped or weakened. The `Makefile`, CI workflows, style
   gate wiring and checker self-test inventory are unchanged.
2. No approved NDC or ADR bytes were edited. Version identifiers, status lines
   and registered digests were preserved.
3. No capability assertion was silently deleted. `claims/ledger.json` remains
   the canonical claim record; `engineering/capabilities.json` retains all 53
   `CAP-xxx` entries.
4. No source-ingestion, raw-archive, decoder, recorder or acquisition module was
   deleted. The only code removed is the explicitly approved duplicate Python
   namespace `research/python/src/quantlab/simulation/`.
5. Dated evidence published before this record is immutable. Historical
   statements that name a path moved or removed here resolve through the path
   map in §5; the dated files themselves were not edited.

## 2. Authority per topic

| Topic | Canonical authority | Status after RRP-005 |
|---|---|---|
| Claims | `claims/ledger.json` | Canonical; enforced by `scripts/check_repo.py`. CLM-041 scope and statement updated truthfully for the namespace removal. |
| Capabilities | `engineering/capabilities.json` | Superseded inventory; explicit `authority` block names the canonical ledger and the consolidation record. All CAP entries retained. |
| NDC v2 requirements | `engineering/ndc-consumer-requirements-v2-proposed.md` | Approved bytes unchanged; active consumer version. |
| NDC v1 requirements | `engineering/contracts/historical/ndc-consumer-requirements-v1.md` | Historical; bytes and digest unchanged; moved to the versioned contract area. |
| Source-ingestion ownership | `engineering/architecture/decisions/ADR-0001-data-ownership.md`, `engineering/source-ingestion-freeze.v1.json` | Accepted ownership and frozen paths unchanged; `provider-environments.md` now reflects that ownership. |
| Data rights | `configs/providers/source-products.v1.json` | All operations `UNKNOWN`; no right granted or implied by this record. |
| Code admission and enforcement | `AGENTS.md`, `CONTRIBUTING.md`, `Makefile`, CI workflows, `engineering/anti-patterns.md` | Anti-patterns reduced to an enforceable subset; E0–E8 roadmap, protected-main rules, pre-merge checklist and verification profiles retained. |
| Python simulation namespace | `research/python/src/quantlab/sim/` | Sole namespace; hosts the relocated `cost_assumptions_digest`. |
| Legacy promotion history | `evidence/MIGRATION-MAP-ARCHIVE-2026-11-23.md` | Archived verbatim; the CSV was removed from the active tree. |
| SBOM | `scripts/supply_chain.py` plus `make sbom` | Build artifact; checked-in copy marked as a stale snapshot with its regeneration command. |
| Provider Python environments | `engineering/provider-environments.md` | Isolation gates unchanged; live capture runbook replaced with the frozen-path ownership index. |

## 3. Audits and decisions

### 3.1 Reference catalogue removal

`engineering/reference-projects.json` (40176 bytes, SHA-256
`80f1db7a3fb7c80a29ff649bbbf081db9fdaaca4fd66c94d887e78fa70c3ec18`) was a
curated `reference_only` catalogue. A repository-wide content audit found one
consumer: the former §2.2 authority list in `engineering/anti-patterns.md`. No
script, gate, schema, ADR, CI job or decision consumed the file; no project was
promoted from it into a dependency. The §2.2 list was updated in the
consolidation, and the catalogue was removed. No replacement catalogue was
created because nothing depended on one.

### 3.2 Migration ledger archive

`engineering/migration-map.csv` (4206 bytes, SHA-256
`6192b639fbbcb79635b066459cb0d3e8a389da4454890c2975d2a44b51d50f85`) was
archived byte-identically in `evidence/MIGRATION-MAP-ARCHIVE-2026-11-23.md`
and then removed. Mutable references in `README.md` and
`engineering/extras-audit.md` now point at the archive. Dated evidence that
names the former path is immutable and resolves through §5.

### 3.3 Provider environments rewrite

`engineering/provider-environments.md` was rewritten to state the RRP-004
frozen paths, their Nirnay responsibility, the Fieldbook target owner and the
replacement condition for each path. The owner-local live capture runbook and
provider readiness claims were removed from the live document; the dated
observation record `evidence/M3-1-REAL-SOURCE-STATUS-2026-11-20.md` was not
edited. The Python environment conflict, isolation jobs, `provider-boundary`
canary and CI mapping are unchanged. The KDB-X section remains a research-plane
note over an already admitted input; no decode or capture command is included.

### 3.4 NDC v1 historical move

`engineering/ndc-consumer-requirements-v1.md` was moved with `git mv` to
`engineering/contracts/historical/ndc-consumer-requirements-v1.md`. Bytes are
unchanged: 27394 bytes, SHA-256
`25315c57de753bde33b0f8044e70646f24b19c8af7ca9fdbdd2e53682a53b8bd`, matching
the workspace `NDC-REGISTRY.md` entry. `engineering/contracts/historical/README.md`
records the path map and digest. Mutable references in `README.md` and
`research/python/src/quantlab/dataset_admission.py` were updated.

### 3.5 Duplicate Python namespace removal

A repository-wide import and reference audit covered Python imports, tests,
configs, schemas, scripts, gates, CI, docs and evidence. Consumers of
`quantlab.simulation` were:

- `research/python/tests/test_simulation_namespace.py` — asserted that the
  shims re-exported identical canonical objects.
- `research/python/tests/test_research_package_shims.py` — imported the
  `cost_assumptions_digest` helper.
- `configs/research/jobs.v1.json` — listed the shim `__init__.py` in `code_paths`.
- `engineering/capabilities.json` CAP-046 and `claims/ledger.json` CLM-041 —
  described the aliases in their scope text.

All three files in `research/python/src/quantlab/simulation/` were re-export
shims, except one helper defined in `__init__.py`. No required external consumer
exists, so the shim package was removed and `quantlab.sim` is the sole namespace.
`cost_assumptions_digest` was relocated into `quantlab.sim/__init__.py` using
the same `market.canonical_bytes` function, so its digest behavior is unchanged.
Tests now import canonical objects and passed after the edit. The job manifest's
`code_paths` entry was removed and its `code_digest` recomputed with the
repository's `environment_digest` helper (51 paths to 50). No experiment was
registered, run or reinterpreted; no dataset or right changed.

### 3.6 Claims and capabilities reconciliation

`claims/ledger.json` is canonical and remains enforced by `check_repo.py`.
CLM-041 keeps its ID, status and falsifier purpose, with a scope that names the
canonical `sim` files and a statement describing the RRP-005 removal.
`engineering/capabilities.json` gained an `authority` block that marks the file
as a superseded inventory, names `claims/ledger.json` as canonical, names this
record and states that no CAP assertion was deleted. CAP-046 retains its entry
and records `superseded_by: claims/ledger.json#CLM-041`.

### 3.7 SBOM disposition

`sbom.cdx.json` was not deleted. Its CycloneDX `metadata.properties` now carries
`nirnay:snapshot-status` naming the checked-in copy a stale snapshot and giving
the regeneration command `make sbom` /
`python3 scripts/supply_chain.py sbom sbom.cdx.json`, plus a reference to this
record. CI regenerates the file from the exact commit and records its digest;
the regeneration intentionally drops the snapshot-status marker.

### 3.8 Anti-patterns reduction

`engineering/anti-patterns.md` was reduced from 124879 bytes (SHA-256
`311267df7205d8d1f7e915f35858ec2f529eeecbd1867340b3efb677dc96c188`) to 35720
bytes (SHA-256 `8964f4fe4213c3057b7d5bfc9a38853ef352e165be4bba7c4f374ab1f760c7d`).
The reduced document keeps the severity classes, enforcement tags and
non-waivable rules, one row per retained stable rule ID with its severity and
tag, the E0–E8 enforcement roadmap, the protected-main requirements, the
reviewer questions, the minimum pre-merge checklist and the verification
profiles including the toolchain change policy. The former §2.2 authority list
no longer names the removed reference catalogue; it names the canonical claim
ledger and the superseded capability inventory. The `STA-010` rule cited by
`engineering/decisions/ADR-M1-006-ordering-and-partitions.md` is retained with
its original wording.

## 4. Removed and moved paths

| Path | Bytes | SHA-256 at base | Disposition | Replacement |
|---|---:|---|---|---|
| `engineering/reference-projects.json` | 40176 | `80f1db7a3fb7c80a29ff649bbbf081db9fdaaca4fd66c94d887e78fa70c3ec18` | removed | none required; audit in §3.1 |
| `engineering/migration-map.csv` | 4206 | `6192b639fbbcb79635b066459cb0d3e8a389da4454890c2975d2a44b51d50f85` | archived then removed | `evidence/MIGRATION-MAP-ARCHIVE-2026-11-23.md` |
| `research/python/src/quantlab/simulation/__init__.py` | n/a | `9f6e6a1fb5aad27e8ccad54ca7c8f47da9bb74a7a466229d958245df409735e2` | removed | `research/python/src/quantlab/sim/__init__.py` |
| `research/python/src/quantlab/simulation/event_replay.py` | n/a | `f6a6cff3ed45bf2b6798601e399404e20f07c7349ef8656d00212f6094cbd240` | removed | `research/python/src/quantlab/sim/replay.py` |
| `research/python/src/quantlab/simulation/execution.py` | n/a | `6f7542bd266a672c39ab0536672e795f50fe5029df0a79fe2ab82195653b3d65` | removed | `research/python/src/quantlab/sim/execution/` |
| `engineering/ndc-consumer-requirements-v1.md` | 27394 | `25315c57de753bde33b0f8044e70646f24b19c8af7ca9fdbdd2e53682a53b8bd` | moved, bytes unchanged | `engineering/contracts/historical/ndc-consumer-requirements-v1.md` |

## 5. Path resolution map

| Former reference | Resolves to | Basis |
|---|---|---|
| `engineering/migration-map.csv` | `evidence/MIGRATION-MAP-ARCHIVE-2026-11-23.md` | byte-identical archive |
| `engineering/ndc-consumer-requirements-v1.md` | `engineering/contracts/historical/ndc-consumer-requirements-v1.md` | `git mv`, unchanged digest |
| `research/python/src/quantlab/simulation/` | `research/python/src/quantlab/sim/` | import audit, §3.5 |
| `engineering/reference-projects.json` | no replacement; audit result in §3.1 | no consumer found |
| `engineering/anti-patterns.md:307-432,910-923` (ADR-M1-007 historical line citation) | section 6 and section 11 of the consolidated document | approved ADR text not edited |

Immutable records that still name a former path were not edited:
`evidence/VERIFICATION-2026-11-17.md`,
`evidence/SBE-GAP-2-INVENTORY-2026-11-19.md`,
`evidence/SBE-GAP-2-PROMOTION-2026-11-19.md`,
`evidence/TEST-RETIREMENT-NIRNAY-2026-11-23.md`,
`engineering/architecture/reviews/FIELDBOOK-REVIEW-REFRESH-2026-11-23.md`,
`engineering/ndc-consumer-requirements-v2-proposed.md`, and the workspace
`NDC-REGISTRY.md`. The approved v2 document and the workspace registry retain
historical v1 path text; their digests were not changed. Updating those two
external records is a paired workspace task outside this worktree and is listed
as a residual item in §7.

## 6. Gate verification

All commands were run from the worktree root with no commit created.

| Command | Exit | Result |
|---|---:|---|
| `make check` | 0 | pass; 288 Python tests passed, 199 deselected, 2 subtests passed; Rust all-target tests and Clippy passed; schema validation passed |
| `make check-static` | 0 | pass; includes `docs-check`, `freeze-check`, `shellcheck-check`, `generated-paths-check`, `pollution-check`, `cargo fmt --check`, `cargo xtask check`, `cargo deny check` and `style` |
| `make docs-check` | 0 | pass; canonical=5, dated=4, no broken local links in checked documents |
| `make prose-style` | 0 | pass; 12 + 6 self-tests, 0 findings and 0 advisory candidates |
| `make source-style` | 0 | pass; 26 self-tests, 0 added-line findings |
| `make python-ruff-style` | 0 | pass; Ruff clean on added Python lines |
| `python3 scripts/check_style_gate.py` | 0 | pass; style gate wiring and rule inventory intact |
| `python3 scripts/validate_contracts.py` | 0 | pass; 56 active JSON files, 43 TOML files, 25 schemas, 1 registry instance |
| `PYTHONPATH=research/python/src python3 -m pytest -q research/python/tests/test_simulation_namespace.py research/python/tests/test_research_package_shims.py` | 0 | pass; 10 passed, 2 deselected |

Repository-wide scan found no remaining import of `quantlab.simulation` and no
broken local Markdown link introduced by this consolidation. The one existing
broken link in `engineering/references/awesome-style-guide-master.md` is a
vendored third-party document and is outside this change.

## 7. Residual items outside this worktree

1. The workspace `NDC-REGISTRY.md` row "NDC v1 consumer document (historical)"
   still names the former path `engineering/ndc-consumer-requirements-v1.md`.
   The workspace repository was not edited. The registry's digest value is
   correct and unchanged; only the path text needs a paired workspace update.
2. `engineering/ndc-consumer-requirements-v2-proposed.md` is approved Nirnay
   bytes and was not edited; its note that the v1 document "keeps its path" is
   historical. Path resolution is provided by §5.
3. Gate RRP-005 acceptance in the workspace `REVIEW-LOG.md` and `CURRENT-STATE.json`
   is not claimed by this record. No gate is marked PASS by wording here; the
   commands and exits in §6 are local verification only.
4. No commit was created. The parent session decides commit, paired workspace
   update and review.

## 8. Confirmation

No provider was contacted. No collection command, dataset acceptance, right
grant, experiment registration, live order path or history rewrite was
authorised or performed. No rights, data or experiment record was changed other
than the truthful code-scope update in CLM-041 and the code-path pin recomputation
in `configs/research/jobs.v1.json` described in §3.5. No gate was weakened.
