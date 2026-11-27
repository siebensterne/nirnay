# TODO

- [ ] Re-run the numeric differential on the new corpus root in CI.
- [ ] Confirm the release bundle gates after removing the results record.
- [ ] Integrate the implemented library startup boundary `RecoveryJournalRoot::load`
      into deployment with real durable adapters, owner-controlled paths and
      recovery qualification; the boundary returns NOT-READY until that
      integration exists.
- [ ] Provision the owner-controlled journal directory, signing key, binding
      sidecars and monotonic terminal anchors consumed by that loader.
- [ ] Provision and protect the per-journal trusted high-water records
      (`oms_high_water.json`, `account_high_water.json`) alongside that loader;
      they must be advanced only by a verified load and must outlive any
      journal/anchor replacement.
- [ ] Keep the EXP-M4-0001 synthetic holdout recorded as consumed once
      post-selection and do not reuse it.
