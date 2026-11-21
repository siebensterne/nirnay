#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# Non-blocking English and terminology audit.
#
# Usage:
#   scripts/assurance/run_audit.sh [--history] [OUTPUT]
#
# The command writes a JSON report and exits zero when findings are present.
# Findings are review candidates, not automatic failures. Operational errors
# (for example, a missing Git repository) still return non-zero.
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
history=()
if [[ "${1:-}" == "--history" ]]; then
  history=(--include-history)
  shift
fi

output="${1:-${repo_root}/build/assurance/english-audit.json}"
mkdir -p "$(dirname "${output}")"

# Vendored reference documents are byte-exact third-party material, verified
# by scripts/check_style_references.py. Their wording is not authored prose.
audit_excludes=(--exclude 'references/*')

set +e
if [[ "${#history[@]}" -gt 0 ]]; then
  python3 "${repo_root}/scripts/assurance/english_audit.py" \
    --repo "${repo_root}" \
    "${history[@]}" \
    "${audit_excludes[@]}" \
    --json-out "${output}"
else
  python3 "${repo_root}/scripts/assurance/english_audit.py" \
    --repo "${repo_root}" \
    "${audit_excludes[@]}" \
    --json-out "${output}"
fi
status=$?
set -e

if [[ "${status}" -eq 2 ]]; then
  exit 2
fi

printf 'English audit report: %s\n' "${output}"
exit 0
