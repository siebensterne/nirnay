# Code-manifest integrity record — 2026-11-24

**Task:** W2.1 STEP 2, Nirnay research-manifest code integrity.
**Status:** implemented and locally tested; internal integrity only.
**Boundary:** no provider contact, market observation, dataset build, experiment
registration, rights decision or commit was made in this worktree. All test files
are arbitrary text and binary bytes under temporary directories.

## Canonical rule

`quantlab.experiments.reproducibility` declares `CODE_MANIFEST_SCHEMA_VERSION = 1`.
`environment_digest()` is the single canonical generator used for both generation
and verification; `verify_code_manifest()` recomputes from disk and compares that
result with the declared pin instead of trusting the manifest claim.

Each canonical entry carries a repository-relative POSIX path, `file_type` fixed
to `regular`, `mode` of `regular` or `executable`, byte `size` and the SHA-256 of
the exact bytes read. Size is taken from `lstat` and checked against the bytes
read. Entries are sorted by repository-relative UTF-8 path bytes before hashing.

The SHA-256 input begins with the domain prefix `NIRNAY-RESEARCH-CODE-MANIFEST-V1`
followed by a zero byte. It then contains the framed schema version, framed entry
count and framed field labels and values. Each field is framed as an eight-byte
big-endian UTF-8 length followed by that many bytes; ambiguous concatenation is
not used. Unsupported schema versions raise `ExperimentError`.

Validation rejects absolute paths, `..` traversal, symlink components checked
before resolution, targets outside the repository root, duplicate normalized
paths, directories, special files and missing files. Verification additionally
checks the declared repository commit, the exact-byte constraint digest,
constraint pinning, the runtime Python version, the job-to-environment digest
binding and, for an exact commit pin, a clean source tree.

## Recomputed research-job pin

- Rule: code-manifest schema version 1 as stated above.
- Full digest: `c032c86f0d0afb16309bb9d12be14cd9f6febbe847742f1745ad7bf8f5c717b2`
- Paths: 74 repository-relative Python paths under `research/python/src/quantlab`,
  regenerated from the present tree; the retired `demo.py` path was removed and
  newer source paths were added.
- Constraint: `constraints/python-research.txt`, SHA-256
  `e385f9cf1dbd5f06cde53d3cc6939329d03b6dbc1135d6f62e9605f4e4e07996` (unchanged).
- Declared commit: `ec0b216f07e7b7e653e8f784b18c6517d6335696` (unchanged pin).
- Jobs: `[]`. The synthetic research job remains retired; no job can consume the
  digest.

## Commit-pin limitations

The recorded commit is historical and is an ancestor of HEAD, not HEAD itself.
Local runtime is Python 3.15.2 while the pin declares Python 3.12, and this
change is uncommitted in the worktree. Exact-commit verification therefore
refuses to consume the manifest until the commit, environment and tree are
deliberately reconciled in a later authorized change. That refusal is a local
integrity outcome and makes no claim about data rights, provider entitlement,
strategy performance or live readiness.

The README `CURRENT-STATE` block still carries the pre-change code head. The
parent workflow refreshes its `code_commit` in a final prose commit after the
code commits, as required by the task ordering; this worktree change makes no
commit.

## Tests

`research/python/tests/test_code_manifest_integrity.py` adds 14 abstract tests:
same bytes and paths, one changed byte, moved path, canonical declaration order,
duplicate normalized path, missing file, symlink file and symlink directory
component, escape and traversal, absolute path, directory declared as file,
unsupported schema version, disk-recomputed verification, forged claim rejection
and dirty-tree rejection. The existing shim test was updated to declare its
temporary root explicitly under the path-safe rule.

## Gates

`make check` exited 0: static gates, Rust tests and Clippy, contract validation
and `python3 -m pytest -q` (309 passed, 182 deselected, 2 subtests passed).
`make docs-check`, `make prose-style`, `make source-style` and
`make python-ruff-style` passed. The synthetic-market source gate scanned 437
source files and found 0 prohibited patterns. No generated source, dependency,
constraint, data or rights artifact was changed.
