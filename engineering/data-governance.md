# Data governance

For every source, record provider, account/application entitlement reference,
permitted products/segments, retention, caching, display, analysis and
redistribution rights, effective dates, contractual limits and owner.

Do not infer permission from API availability. Do not publish captured exchange
or broker payloads, instrument masters, user/account identifiers, credentials,
or derived data where the contract restricts redistribution. Public code and
market-data rights are separate. Synthetic fixtures must be labelled as such.

## Frozen legacy acquisition boundary

Under RRP-004 the Nirnay acquisition, raw-archive, decoder, recorder, provider-mapping and
provider-config paths are legacy frozen and are not the canonical source authority. Fieldbook
owns acquisition, raw storage, normalization and effective-dated reference data under
ADR-0001. All source rights remain `UNKNOWN` and dataset acceptance stays `BLOCKED`; no
provider operation is authorized. See `evidence/SOURCE-INGESTION-FREEZE-2026-11-23.md` and
`engineering/source-ingestion-freeze.v1.json`.
