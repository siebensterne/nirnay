# Nirnay developer commands. The task runner (cargo xtask) owns the gate
# definitions; the targets below are the deterministic entry points.
# Command surface (`make help` prints the same list):
#   check           full gate: static checks, Rust/Python tests, contracts
#   check-static    static checks only; no python-test or rust-test
#   freeze-check    RRP-004 frozen ingestion paths guard (with self-test)
#   synthetic-market-policy-check  D11 source-pattern guard (with self-test)
#   ci              full CI parity task
#   assure          full assurance superset
#   lint            Clippy with warnings denied (cargo xtask lint)
#   test            Rust all-target and doc tests (cargo xtask test)
#   fmt             non-mutating Rust formatting check
#   style           all source and standards style gates
#   bundle          build the release zip from BUNDLE_OUT/BUNDLE_ARTIFACTS
#   bundle-verify   verify that same zip and its extracted BUNDLE_DIR
#   tools           install the pinned Python tools
# Gate scope is defined by the targets below and by the task runner.
# Gate steps must not leave Python bytecode in the source tree: the generated
# path gate inspects ignored untracked files as well as tracked ones.
export PYTHONDONTWRITEBYTECODE := 1
export PYTEST_ADDOPTS := $(strip $(PYTEST_ADDOPTS) -p no:cacheprovider)

.PHONY: help ci assure check check-static lint test perf bench-ingress bench-risk-oms freeze-check synthetic-market-policy-check shellcheck-check generated-paths-check pollution-check style rust-style source-style python-ruff-style q-style standards-check style-gate python-format-check rust-test python-test python-check cpp-test cpp-format-check schema-check verify-local fmt constraints sbom supply-chain wheels packaging-test bundle bundle-verify git-policy kdb-analyze archive-verify tools

help:
	@echo "check           full gate: static checks, Rust/Python tests, contracts"
	@echo "check-static    static checks only; no python-test or rust-test"
	@echo "freeze-check    RRP-004 frozen ingestion paths guard (with self-test)"
	@echo "synthetic-market-policy-check  D11 source-pattern guard (with self-test)"
	@echo "ci              full CI parity task"
	@echo "assure          full assurance superset"
	@echo "lint            Clippy with warnings denied (cargo xtask lint)"
	@echo "test            Rust all-target and doc tests (cargo xtask test)"
	@echo "fmt             non-mutating Rust formatting check"
	@echo "style           all source and standards style gates"
	@echo "bundle          build the release zip from BUNDLE_OUT/BUNDLE_ARTIFACTS"
	@echo "bundle-verify   verify that same zip and its extracted BUNDLE_DIR"
	@echo "tools           install the pinned Python tools"

ci:
	cargo xtask ci

assure:
	cargo xtask assure
check: check-static
	$(MAKE) rust-test
	$(MAKE) schema-check
	$(MAKE) python-test
check-static:
	$(MAKE) freeze-check
	$(MAKE) synthetic-market-policy-check
	$(MAKE) shellcheck-check
	$(MAKE) generated-paths-check
	$(MAKE) pollution-check
	cargo fmt --all -- --check
	cargo xtask check
	cargo deny check
	$(MAKE) style
lint:
	cargo xtask lint
test:
	cargo xtask test
freeze-check:
	python3 scripts/check_source_ingestion_freeze.py --self-test
	python3 scripts/check_source_ingestion_freeze.py
synthetic-market-policy-check:
	python3 scripts/check_synthetic_market_policy.py --self-test
	python3 scripts/check_synthetic_market_policy.py
shellcheck-check:
	python3 scripts/check_shellcheck.py
tools:
	python3 -m pip install --disable-pip-version-check \
		ruff==0.17.5 shellcheck-py==0.11.0.1
generated-paths-check:
	python3 scripts/check_generated_paths.py
pollution-check:
	python3 scripts/check_no_cross_pollution.py
style: rust-style source-style python-ruff-style q-style standards-check \
	style-gate
rust-style:
	python3 scripts/check_rust_style.py --self-test
	python3 scripts/check_rust_style.py
source-style:
	python3 scripts/check_source_style.py --self-test
	python3 scripts/check_source_style.py
python-ruff-style:
	python3 scripts/ruff_gate.py scripts/check_ruff_added_lines.py scripts/check_quality_contract.py scripts/check_license_provenance.py scripts/ruff_gate.py
	python3 scripts/check_ruff_added_lines.py --self-test
	python3 scripts/check_ruff_added_lines.py
q-style:
	python3 scripts/check_q_style.py --self-test
	python3 scripts/check_q_style.py
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

# Opt-in benchmarks. Remaining targets are Rust harnesses that print BENCH-*
# lines with measured counters and are deliberately absent from `check` and CI
# pass/fail gates. Emitted numbers are not throughput or latency certificates;
# any dated result record must state its own scope and measurement fields.
# Under D11 the synthetic Python benchmark targets were retired
# with their invented market observations and must not be restored.
bench-ingress:
	cargo test -p market-archive --test performance --features market-shaped-tests -- --ignored --nocapture --skip recovery_child

bench-risk-oms:
# D11 retired the synthetic risk-authority throughput workload; only the
# opt-in authoritative-oms measurement remains.
	cargo test -p authoritative-oms --test performance --features market-shaped-tests -- --ignored --nocapture

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
BUNDLE_DIR ?= ../nirnay-quant-bundle
BUNDLE_ARTIFACTS ?= ../artifacts
bundle:
	python3 scripts/make_bundle.py --source-dir . --artifacts $(BUNDLE_ARTIFACTS) --output $(BUNDLE_OUT)
bundle-verify:
	rm -rf "$(BUNDLE_DIR)"
	mkdir -p "$(BUNDLE_DIR)"
	cd "$(BUNDLE_DIR)" && unzip -q "$(abspath $(BUNDLE_OUT))"
	python3 scripts/verify_bundle.py --bundle $(BUNDLE_DIR) --zip $(BUNDLE_OUT) --self-test
git-policy:
	python3 scripts/check_git_policy.py --strict
CPP_BUILD_DIR ?= /tmp/nirnay-cpp-numeric-$(shell pwd | cksum | cut -d' ' -f 1)
cpp-test:
	cmake -S tests/oracles/cpp-numeric -B $(CPP_BUILD_DIR) -DCMAKE_BUILD_TYPE=Release
	cmake --build $(CPP_BUILD_DIR) --parallel
	ctest --test-dir $(CPP_BUILD_DIR) --output-on-failure
CLANG_FORMAT ?= clang-format
CPP_SOURCES := $(shell find tests/oracles/cpp-numeric -type f \( -name '*.c' -o -name '*.cc' -o -name '*.cpp' -o -name '*.cxx' -o -name '*.h' -o -name '*.hpp' -o -name '*.hxx' \) 2>/dev/null)
cpp-format-check:
	@for f in $(CPP_SOURCES); do $(CLANG_FORMAT) --style=file --dry-run --Werror "$$f" || exit 1; done
verify-local: python-test python-check schema-check cpp-test


# KDB-X analytics: owner-local, read-only. Requires a locally installed q.
# D11 retired the synthetic golden corpus and its regeneration/verification
# commands; only the analyze command on an explicitly supplied input remains.
KDB_INPUT ?= .local/perf/kdb/book.csv
KDB_SOURCE ?= upstox-live
KDB_OUT ?= .local/perf/kdb-lab-run

kdb-analyze:
	cargo run --locked -p kdb-lab --bin nirnay-kdb -- analyze --input $(KDB_INPUT) --source $(KDB_SOURCE) --out $(KDB_OUT)

# Verify a full archive's source commit is recoverable from its Git bundle.
ARCHIVE ?= /Users/tourist/Documents/nirnay-full-2026-11-20.zip
archive-verify:
	python3 scripts/verify_archive_bundle.py $(ARCHIVE)
