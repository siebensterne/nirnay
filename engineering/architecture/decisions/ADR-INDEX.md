# Architecture decision record index

This index fixes the canonical IDs used for architecture decision records in
Nirnay and names the Fieldbook records cited from Nirnay documents. IDs are
namespaces, not renames: existing filenames and record status stay as
published.

## Nirnay canonical IDs

- `NR-ARCH-ADR-0001` — `ADR-0001-data-ownership.md`, status **Accepted 2026-11-23**.
  Data ownership between Nirnay and the future complementary platform:
  acquisition, raw storage, normalization and point-in-time reference data stay
  with the platform; Nirnay owns research and experiments. This index neither
  renames the file nor changes its status.

## External references (Fieldbook)

- `FB-ADR-0001` — Fieldbook
  `docs/adr/0001-instrument-and-venue-reference-model.md`, status Accepted.
  External reference for the instrument and venue reference model; not a
  Nirnay decision.
- `FB-ADR-0002` — Fieldbook
  `docs/adr/0002-persist-transport-observations-without-changing-receive-order.md`,
  status Accepted. External reference for transport observations and receive
  ordering; not a Nirnay decision.

Fieldbook ADRs stay owned by that repository. The IDs above identify them when
Nirnay documentation cites them, and confer no authority inside Nirnay.
