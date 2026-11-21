#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# The polish gate: bring the tree to the state CI expects and prove it.
#
# Usage:
#   scripts/assurance/polish.sh [--loop N]
#
# Iteration 1 applies rustfmt; every iteration re-checks the entire gate.
# --loop N repeats the whole gate N times: repetition exists to surface
# flaky, order-dependent or environment-sensitive behavior. A gate that
# only passes sometimes is a failing gate.
#
# Tree stability is verified inside every iteration: the gate itself must
# not mutate the tree, so a test that rewrites a fixture, stages a change or
# creates a file cannot hide behind a passing exit status. The fingerprint is
# taken before phase 0 so setup steps are covered too.
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/gate.sh"
gate_cd_root

loops=1
case "${1:-}" in
  "") ;;
  --loop) loops="${2:?--loop requires a repetition count}" ;;
  *)
    printf 'unexpected argument: %s\n' "$1" >&2
    exit 2
    ;;
esac
if (( $# > 2 )); then
  printf 'unexpected arguments: %s\n' "${*:3}" >&2
  exit 2
fi
if ! [[ "$loops" =~ ^[0-9]+$ ]] || (( loops < 1 )); then
  echo "--loop requires an integer >= 1" >&2
  exit 2
fi

tree_fingerprint() {
  {
    git status --porcelain=v1 -z
    git diff --binary HEAD
    git diff --cached --binary
  } | git hash-object --stdin
}

fingerprint_start="$(tree_fingerprint)"

check_shell_syntax() {
  local status=0 script count=0
  local list
  list="$(mktemp)"
  find scripts -type f -name '*.sh' -print0 >"$list"
  while IFS= read -r -d '' script; do
    count=$((count + 1))
    if ! bash -n "$script"; then
      printf 'syntax error: %s\n' "$script" >&2
      status=1
    fi
  done <"$list"
  rm -f "$list"
  if (( count == 0 )); then
    printf 'no shell scripts found\n' >&2
    return 1
  fi
  return "$status"
}

# ---- phase 0: certify the tools that guard the repository ----------------
run_step "shell syntax (bash -n over scripts/)" check_shell_syntax
shell_scripts=()
while IFS= read -r -d '' script; do
  shell_scripts+=("$script")
done < <(find scripts -type f -name '*.sh' -print0)
run_optional_step "shellcheck over scripts/" shellcheck "${shell_scripts[@]}"
run_step "python unit tests (scripts)" \
  python3 -m unittest discover -s scripts -p 'test_*.py'
run_step "python unit tests (scripts/assurance)" \
  python3 -m unittest discover -s scripts/assurance -p 'test_*.py'
run_step "requirement-matrix references" python3 scripts/check_matrix_tests.py
run_step "architecture traceability" python3 scripts/check_architecture.py \
  --check
run_step "generated-path policy" python3 scripts/check_generated_paths.py
run_step "style reference integrity" python3 scripts/check_style_references.py
run_step "git policy" python3 scripts/check_git_policy.py
run_step "cross-pollution guard" python3 scripts/check_no_cross_pollution.py
run_step "Rust added-line style" python3 scripts/check_rust_style.py
if cargo audit --version >/dev/null 2>&1; then
  run_step "cargo audit" cargo audit
else
  GATE_STEPS+=("cargo audit|SKIP|0")
  audit_note='== cargo audit: skipped (cargo-audit is not installed;'
  audit_note+=' install it for full coverage)'
  printf '%s\n' "$audit_note"
fi

# ---- phase 1: the CI-parity build-and-verify core, iterated --------------

iteration=1
while (( iteration <= loops )); do
  printf '\n======== polish iteration %d/%d ======== \n' "$iteration" "$loops"
  if (( iteration == 1 )); then
    run_step "cargo fmt (apply)" cargo fmt --all
  fi
  run_step "cargo fmt --check" cargo fmt --all -- --check
  run_step "cargo clippy -D warnings" cargo clippy --workspace --all-targets \
    --locked -- -D warnings
  run_step "cargo test (all targets)" cargo test --workspace --all-targets \
    --locked
  run_step "cargo test (doc)" cargo test --workspace --doc --locked
  run_step "cargo build (release)" cargo build --release --locked
  if [[ "$(uname -s)" == "Linux" ]]; then
    run_step "journal ENOSPC drill" scripts/verify-journal-enospc.sh
  else
    GATE_STEPS+=("journal ENOSPC drill|SKIP|0")
    printf '== journal ENOSPC drill: skipped on %s (Linux-only CI step)\n' \
      "$(uname -s)"
  fi
  run_step "english audit (advisory)" scripts/assurance/run_audit.sh
  run_step "english audit strict" python3 scripts/assurance/english_audit.py \
    --repo . --exclude 'references/*' --strict

  fingerprint_now="$(tree_fingerprint)"
  if [[ "$fingerprint_now" == "$fingerprint_start" ]]; then
    printf '== tree stability: pass (the gate mutated nothing)\n'
  else
    printf '== tree stability: FAIL (the gate modified the tree)\n' >&2
    git status --porcelain=v1 >&2
    git diff --stat HEAD >&2
    GATE_FAILED=1
  fi

  iteration=$((iteration + 1))
done

gate_summary
