#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# NZ-API-001 cross-execution driver for the Fieldbook B15 streaming producer.
#
# Set NIRNAY_B15_PRODUCER_BIN to an executable that accepts `--out DIR` and
# writes a release under DIR. The Rust harness invokes that binary, feeds the
# result to verify_release_streaming and records digests with --nocapture.
# With no binary, the harness prints an explicit skip reason. No producer
# output is synthesized by this script.

# shellcheck shell=bash
set -euo pipefail

binary="${NIRNAY_B15_PRODUCER_BIN:-}"

if [[ -z "$binary" ]]; then
  printf 'SKIP NZ-API-001 cross-execution: NIRNAY_B15_PRODUCER_BIN is unset; no Fieldbook B15 producer binary was supplied.\n'
elif [[ ! -x "$binary" ]]; then
  printf 'SKIP NZ-API-001 cross-execution: %s is not executable.\n' "$binary"
else
  printf 'NZ-API-001 producer binary: %s\n' "$binary"
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$binary"
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$binary"
  else
    printf 'SKIP NZ-API-001 binary digest: no SHA-256 command is installed.\n'
  fi
  printf 'NZ-API-001 producer commit: %s\n' "${NIRNAY_B15_PRODUCER_COMMIT:-not-supplied}"
fi

cargo test -p release-verifier --lib --locked nz_api001_cross_execution_tests -- --nocapture
