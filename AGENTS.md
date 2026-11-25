# Agent instructions

Nirnay owns independent NDC consumption and verification, accepted research datasets, research,
simulation, portfolio, risk and OMS. Fieldbook owns source acquisition, raw storage,
normalization, reference authority and NDC publication.

- Use `README.md` and `engineering/README.md` to locate the owning documents. Read only the relevant
  ADR, contract, implementation and tests.
- Read `engineering/verification-policy.md` for claim states. Read `engineering/data-governance.md`
  for data, rights, acceptance or retention work.
- Read `engineering/capabilities.json` and `claims/ledger.json` before changing capability or claim
  records. Preserve their canonical/superseded relationship.
- Check Workspace `CURRENT-STATE.json` before actions involving rights, datasets, experiments, paper
  operation, budget or execution. A valid signature or successful verifier result does not grant
  rights.
- Keep domain and application logic independent of infrastructure and external SDKs. Ports belong to
  the consuming inner layer; adapters implement them at outer boundaries.
- Keep money, prices, fees, quantities and P&L in exact domain representations. Do not use binary
  floating point for authoritative financial state.
- Strategies emit intents. They do not access broker SDKs, OMS internals, risk permits, cash or
  positions. Risk and OMS remain independent authorities.
- Preserve source identity, event ordering, clock domains, point-in-time and knowledge-time
  semantics, lineage and deterministic replay.
- Bound every parser, buffer, queue, decompressor, retry and worker. Use immutable, versioned
  resource profiles selected by trusted local policy. Producer declarations never raise the
  verifier's local ceiling.
- For research, define the hypothesis, null, falsifier, data specification, costs and validation
  design before parameter search. Preserve failed trials. Keep the M4 holdout untouched and follow
  D11: no synthetic market-shaped research evidence.
- Inspect the base commit, affected callers and tests. Implement one coherent change; add
  regression, negative, boundary, replay and recovery tests as applicable.
- Run targeted tests, then the required `make check` and `make assure` gates. Report exact commands,
  exits, counts, ignored tests and environment.
- Keep live routing and unapproved data activity disabled. A passing build, test or CI run does not
  authorize them.
- Preserve dated evidence and owner-maintained artifacts. Update capabilities and claims only when
  source and evidence support the change.
- Proceed autonomously within the task and current authority. Report exact commits, commands, CI
  head, evidence and unresolved gaps.
