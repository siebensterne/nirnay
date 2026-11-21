# Contributing

## Code changes

1. Make one coherent change per pull request.
2. Add tests with the implementation; include negative and failure-path
   tests. Write the hypothesis and falsifier before parameter search.
3. Run `make check`: formatting, style gates, Rust and extras tests, Clippy
   `-D warnings`, `cargo deny check`, `cargo xtask check`, contract validation,
   Python tests and the documentation check.
4. For data changes, attach provenance, entitlement and quality evidence.
5. For research changes, preregister the experiment and preserve all trials.
6. For any execution change, prove risk cannot be bypassed and broker state
   can be reconciled. Live routing stays hard-disabled.
7. Update architecture decisions only when a decision changes; never use
   status prose as a substitute for tests.
8. Update `engineering/capabilities.json` and `claims/ledger.json` truthfully
   and cite executable evidence.
9. Run `cargo xtask --help` to discover the repository policy gates. Each
   subcommand (`check`, `arch`, `waivers`, `claims`) delegates to the matching
   `scripts/check_repo.py` gate; `list` prints the registered task names. The
   gate's own exit code is propagated, so a red gate fails CI.
10. Run `cargo xtask assure` (or `make assure`) before publishing a change.
    This runs `scripts/assurance/polish.sh`: it applies rustfmt, re-checks the
    full gate, repeats for `--loop N`, and fingerprints the tree so a gate
    that mutates source cannot pass. `update.sh` and `upgrade.sh` wrap the
    same gate around lockfile and toolchain bumps and restore pins on failure.

## Documentation and evidence

The documentation set is small. `README.md` is the single
current-state entry point; `AGENTS.md`, `CONTRIBUTING.md` and `SECURITY.md`
are canonical policy files. Everything else is dated evidence under
`evidence/` or `BUILD-RESULTS.md`.

- Update canonical files in the same PR as the behavior they describe. Do not
  leave a current-state claim pointing at an older commit.
- Keep the README current-state block accurate: commit, environment, CI run
  and scope. `scripts/check_docs.py` verifies it on every `make check`.
- Never edit a dated evidence file to make it current. Add a new dated file
  under `evidence/` and link it from the README document map.
- Do not add another root-level status, brief or inventory file. If a new
  document is necessary, state why an existing canonical file cannot hold it.
- Do not modify owner work: `.github/workflows/release.yml`,
  `BUILD-RESULTS.md`, `engineering/supply-chain.md` and
  `scripts/make_bundle.py`, without an explicit owner task.

No live secrets, personal account identifiers, licensed market data or
generated local datasets belong in pull requests.
