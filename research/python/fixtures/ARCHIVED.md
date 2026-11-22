# Archived synthetic research assets — non-executable

**Status:** archived under the owner directive of 2026-11-22
(`evidence/OWNER-DIRECTIVE-2026-11-22.md`, D1–D3). Effective immediately these
files are historical repository material only.

The following checked-in assets model market observations or market-shaped
schedules using invented values:

- `alpha-bars.synthetic.csv` — synthetic daily close-price bars.
- `alpha-bars-minute.seed-20261121.csv` — synthetic one-minute close-price bars.
- `canonical-events.example.jsonl` — synthetic canonical market events.
- `reference-data.example.json` — synthetic instrument, corporate-action and
  universe reference data.
- `calendars/` — synthetic and fixture market calendars and session schedules.

They are preserved unchanged as records of the work in which they were created.
They are **not** to be executed, regenerated, extended, transformed, reused in
any runnable command, or admitted as research evidence. No quickstart or active
test consumes them. Tests that consumed them are registered with the pytest
marker `market_shaped_fixture` and deselected in `pytest.ini`; see
`evidence/POLICY-BOUNDARY-REGRESSION-WITHDRAWAL-2026-11-22.md` for the exact
withdrawn coverage.

Pure mathematical tests using scalar values and abstract timestamps, and
infrastructure tests that construct no invented market observations, remain
active. This marker is a governance boundary, not a claim about source
completeness, data rights or real-market verification.
