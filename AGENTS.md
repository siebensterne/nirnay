# Agent instructions

Nirnay owns NDC consumption and verification, accepted research datasets,
research, simulation, portfolio, risk and OMS. Fieldbook owns acquisition,
raw storage, normalization, reference authority and NDC publication.

- Use `README.md` and `engineering/README.md` to locate owning documents.
  Read only the relevant ADR, contract, implementation and tests.
- Read `engineering/verification-policy.md` for claims and
  `engineering/data-governance.md` for rights, data and retention.
- Read `claims/ledger.json`; `engineering/capabilities.json` is superseded.
  Check workspace `CURRENT-STATE.json` before rights, dataset, experiment,
  paper-operation, budget or execution actions. A verifier result grants no
  rights.
- Keep domain logic independent of infrastructure and SDKs. Ports belong to
  the inner layer; adapters implement them at boundaries.
- Keep money, prices, fees, quantities and P&L exact; no binary floating
  point for authoritative state.
- Strategies emit intents only; risk and OMS remain independent authorities.
- Preserve source identity, event ordering, clock domains, point-in-time
  semantics, lineage and deterministic replay.
- Bound parsers, buffers, queues, decompressors, retries and workers. Profiles
  are immutable, versioned and locally selected; producer declarations never
  raise the verifier ceiling.
- For research, fix hypothesis, null, falsifier, data, costs and validation
  before parameter search. Preserve failed trials. Keep the M4 holdout
  untouched; D11 forbids synthetic market-shaped evidence.
- Inspect base commit, callers and tests. Make one coherent change with
  regression, negative, boundary, replay and recovery tests. Run targeted
  tests, then `make check` and `make assure`; report commands and exits.
- Keep live routing and unapproved data activity disabled; a green gate does
  not authorize them.
- Preserve dated evidence. Update claims only with source and evidence
  support. Proceed within current authority; report commits, CI head and
  gaps.
