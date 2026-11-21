.PHONY: ci assure check perf bench-ingress bench-research bench-experiments bench-latency bench-risk-oms docs-check shellcheck-check generated-paths-check pollution-check style rust-style source-style python-ruff-style q-style prose-style references-check standards-check style-gate python-format-check rust-test python-test python-check cpp-test cpp-format-check schema-check fundamental-fixture-check verify-local fmt constraints sbom supply-chain wheels packaging-test bundle bundle-verify git-policy kdb-analyze kdb-golden kdb-q-test archive-verify tools

ci:
	cargo xtask ci

assure:
	cargo xtask assure
check:
	$(MAKE) docs-check
	$(MAKE) shellcheck-check
	$(MAKE) generated-paths-check
	$(MAKE) pollution-check
	$(MAKE) fmt
	cargo xtask check
	cargo deny check
	$(MAKE) style
	$(MAKE) rust-test
	$(MAKE) schema-check
	$(MAKE) fundamental-fixture-check
	$(MAKE) python-test
docs-check:
	python3 scripts/check_docs.py --self-test
	python3 scripts/check_docs.py
shellcheck-check:
	python3 scripts/check_shellcheck.py
tools:
	python3 -m pip install --disable-pip-version-check \
		ruff==0.17.5 shellcheck-py==0.11.0.1
generated-paths-check:
	python3 scripts/check_generated_paths.py
pollution-check:
	python3 scripts/check_no_cross_pollution.py
style: rust-style source-style python-ruff-style q-style prose-style \
	references-check standards-check style-gate
rust-style:
	python3 scripts/check_rust_style.py --self-test
	python3 scripts/check_rust_style.py
source-style:
	python3 scripts/check_source_style.py --self-test
	python3 scripts/check_source_style.py
python-ruff-style:
	python3 scripts/ruff_gate.py scripts/check_ruff_added_lines.py scripts/check_quality_contract.py scripts/check_style_references.py scripts/check_license_provenance.py scripts/ruff_gate.py
	python3 scripts/check_ruff_added_lines.py --self-test
	python3 scripts/check_ruff_added_lines.py
prose-style:
	python3 scripts/check_prose_style.py --self-test
	python3 scripts/check_prose_style.py
	python3 scripts/check_prose_unified.py --self-test
	python3 scripts/check_prose_unified.py
q-style:
	python3 scripts/check_q_style.py --self-test
	python3 scripts/check_q_style.py
references-check:
	python3 scripts/check_style_references.py --self-test
	python3 scripts/check_style_references.py
standards-check:
	python3 scripts/check_quality_contract.py --self-test
	python3 scripts/check_quality_contract.py
	python3 scripts/check_license_provenance.py --self-test
	python3 scripts/check_license_provenance.py
style-gate:
	python3 scripts/check_style_gate.py --self-test
	python3 scripts/check_style_gate.py
# Whole-tree Ruff remains advisory until the existing Python corpus is
# reformatted in a dedicated PR. Added Python lines are already binding via
# python-ruff-style, which uses the same ruff.toml policy.
python-format-check:
	python3 scripts/ruff_gate.py .
	python3 -m ruff format --check --config ruff.toml .
# Reproducible cold-build measurement and budget enforcement. Kept separate
# from `check` so the normal gate cycle stays fast.
perf:
	python3 scripts/check_compile_budget.py

# Opt-in benchmarks. They print BENCH-* lines with measured counters and are
# deliberately absent from `check` and from CI pass/fail gates. Emitted numbers
# are not throughput or latency certificates; see evidence/PERFORMANCE-TEMPLATE.md
# for the fields required of any dated result record.
bench-ingress:
	cargo test -p market-archive --test performance -- --ignored --nocapture --skip recovery_child

bench-research:
	PYTHONPATH=research/python/src python3 -m quantlab.bench --suite core --repeats 5 --sizes 200,800 --seed 7

bench-experiments:
	PYTHONPATH=research/python/src python3 -m quantlab.bench --suite experiments --workers 4 --repeats 5 --size 800 --seed 7

# Historical-query and end-to-end data latency on synthetic PIT fixtures. Also
# opt-in and excluded from check/CI; emits BENCH-HISTORICAL-QUERY and
# BENCH-E2E-LATENCY lines with measured distributions and no thresholds.
bench-latency:
	PYTHONPATH=research/python/src python3 -m quantlab.bench --suite latency --repeats 5 --size 800 --seed 7

bench-risk-oms:
	cargo test -p risk-authority --test performance -- --ignored --nocapture
	cargo test -p authoritative-oms --test performance -- --ignored --nocapture

rust-test:
	cargo test --workspace --all-targets --locked
	cargo clippy --workspace --all-targets --locked -- -D warnings
fmt:
	cargo fmt --all -- --check
python-test:
	python3 -m pytest -q
python-check:
	python3 -m compileall -q research/python/src apps/fyers-recorder/src apps/kite-ops/src scripts ops
	python3 scripts/check_repo.py check
schema-check:
	python3 scripts/validate_contracts.py
fundamental-fixture-check:
	python3 scripts/check_fundamental_contract_vectors.py --self-test
	python3 scripts/check_fundamental_contract_vectors.py
	cargo run --locked -p fundamental-tools --bin nirnay-fundamental -- validate-facts --input research/fundamentals/fixtures/financial-facts.example.jsonl
	cargo run --locked -p fundamental-tools --bin nirnay-fundamental -- validate-companies --input research/fundamentals/fixtures/company-identities.example.jsonl
constraints:
	uv pip compile research/python/pyproject.toml --extra dev --extra research --extra security \
		--python-version 3.12 --no-emit-package nirnay-quantlab -o constraints/python-research.txt
	uv pip compile apps/kite-ops/pyproject.toml --extra test \
		--python-version 3.12 --no-emit-package nirnay-kite-ops -o constraints/python-kite-ops.txt
	uv pip compile apps/fyers-recorder/pyproject.toml --extra test \
		--python-version 3.12 --no-emit-package nirnay-fyers-recorder -o constraints/python-fyers-recorder.txt
sbom:
	python3 scripts/supply_chain.py sbom sbom.cdx.json
supply-chain:
	python3 scripts/supply_chain.py pins
	python3 scripts/supply_chain.py constraints
	$(MAKE) sbom
wheels:
	rm -rf dist research/python/build apps/kite-ops/build apps/fyers-recorder/build
	python3 -m build --wheel research/python --outdir dist
	python3 -m build --wheel apps/kite-ops --outdir dist
	python3 -m build --wheel apps/fyers-recorder --outdir dist
packaging-test: wheels
	python3 scripts/check_wheels.py --dist dist
BUNDLE_OUT ?= ../nirnay-quant-bundle.zip
BUNDLE_ARTIFACTS ?= ../artifacts
bundle:
	python3 scripts/make_bundle.py --source-dir . --artifacts $(BUNDLE_ARTIFACTS) --output $(BUNDLE_OUT)
bundle-verify:
	python3 scripts/verify_bundle.py --bundle $(BUNDLE_DIR) --zip $(BUNDLE_OUT) --self-test
git-policy:
	python3 scripts/check_git_policy.py --strict
CPP_BUILD_DIR ?= /tmp/nirnay-cpp-oracle-$(shell pwd | cksum | cut -d' ' -f 1)
cpp-test:
	cmake -S verification/cpp-oracle -B $(CPP_BUILD_DIR) -DCMAKE_BUILD_TYPE=Release
	cmake --build $(CPP_BUILD_DIR) --parallel
	ctest --test-dir $(CPP_BUILD_DIR) --output-on-failure
CLANG_FORMAT ?= clang-format
CPP_SOURCES := $(shell find verification/cpp-oracle -type f \( -name '*.c' -o -name '*.cc' -o -name '*.cpp' -o -name '*.cxx' -o -name '*.h' -o -name '*.hpp' -o -name '*.hxx' \) 2>/dev/null)
cpp-format-check:
	@for f in $(CPP_SOURCES); do $(CLANG_FORMAT) --style=file --dry-run --Werror "$$f" || exit 1; done
verify-local: python-test python-check schema-check cpp-test

# KDB-X analytics: owner-local, read-only. Requires a locally installed q;
# the Rust golden tests in `make check` validate the same contract without q.
KDB_INPUT ?= .local/perf/kdb/book.csv
KDB_SOURCE ?= upstox-live
KDB_OUT ?= .local/perf/kdb-lab-run

kdb-analyze:
	cargo run --locked -p kdb-lab --bin nirnay-kdb -- analyze --input $(KDB_INPUT) --source $(KDB_SOURCE) --out $(KDB_OUT)

kdb-golden:
	cargo run --locked -p kdb-lab --bin nirnay-kdb -- check-golden

# In-process q unit tests of the checked-in library (k4unit CSV model).
Q_BIN ?= /Users/tourist/.kx/bin/q
KDB_Q_DIR ?= analytics/kdb
kdb-q-test:
	$(Q_BIN) $(KDB_Q_DIR)/mbp-test.q \
	  $(abspath apps/kdb-lab/tests/golden/mbp-observation-v1.csv) \
	  $(abspath $(KDB_Q_DIR)/tests/empty.csv) \
	  $(abspath $(KDB_Q_DIR)/tests/mbp-unit.csv)

# Verify a full archive's source commit is recoverable from its Git bundle.
ARCHIVE ?= /Users/tourist/Documents/nirnay-full-2026-11-20.zip
archive-verify:
	python3 scripts/verify_archive_bundle.py $(ARCHIVE)
