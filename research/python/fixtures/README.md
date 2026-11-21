# Research fixtures

All files in this directory are **synthetic examples**. They demonstrate canonical event shape, reference-data lineage, and repeatable CLI wiring. They are not NSE observations, official instrument masters, exchange corporate-action notices, licensed data, or strategy-performance evidence.

- `canonical-events.example.jsonl`: canonical `MarketEvent` rows with synthetic capture-frame hashes.
- `reference-data.example.json`: bitemporal provider listing, universe membership, and a 2-for-1 split example. The SHA-256 values are fixture provenance IDs, not hashes of external official source documents.
- `alpha-bars.synthetic.csv`: original 420-bar close series used by the consumed
  M4 pilot; its 60-second bars were mislabelled as daily in that registration.
  Retained as an immutable historical artifact, not as a cadence reference.
- `alpha-bars-minute.seed-20261121.csv`: new deterministic 600-bar close series
  with 60-second (minute) spacing, generated from seed 20261121 by an integer
  recurrence. It is paired with a registration declaring `bar_kind: minute`,
  `bar_interval_ns: 60000000000`, a synthetic calendar and an explicit
  annualisation basis. It is synthetic test data, not market data.

A real dataset must replace every fixture source with licensed, independently acquired source artifacts and accurate source-retrieval/knowledge timestamps. Do not label output built from these files as market data.
