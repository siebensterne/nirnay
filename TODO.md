# TODO

- [ ] Re-run the numeric differential on the new corpus root in CI.
- [ ] Confirm the release bundle gates after removing the results record.
- [ ] Wire production startup to `RecoveryJournalRoot::load` in the startup
      app; that wiring is outside this package's ownership.
- [ ] Provision the owner-controlled journal directory, signing key, binding
      sidecars and monotonic terminal anchors consumed by that loader.
- [ ] Provision and protect the per-journal trusted high-water records
      (`oms_high_water.json`, `account_high_water.json`) alongside that loader;
      they must be advanced only by a verified load and must outlive any
      journal/anchor replacement.
