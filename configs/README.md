`paper.toml` is a configuration example, not a guarantee of working feed
connectivity. `live.toml` is disabled and incomplete. Never enable
live mode by editing a boolean alone; the release process must add independent
risk authorization, broker-specific controls, reconciliation, operational
monitoring, recovery tests and human approval.

`providers/` is frozen under RRP-004. `source-products.v1.json` records all source rights as
`UNKNOWN` and is not a grant; the instrument maps, venue maps, watchlists and examples are
legacy local inputs, not canonical identity or universe data. No new provider map, watchlist
or entitlement example is permitted there without an approved migration record. See
`evidence/SOURCE-INGESTION-FREEZE-2026-11-23.md`.
