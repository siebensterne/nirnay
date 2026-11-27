// SPDX-License-Identifier: Apache-2.0
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Deterministic startup-transaction tests for the paper-runner integration
//! boundary. All fixtures are neutral: no market-shaped data, provider contact,
//! experiment, holdout, broker or live execution is involved.
//!
//! This file is the regression contract for the incomplete integration: the
//! application must acquire ownership before loading journals, drive the
//! library loader, then reconstruct the authority, and must stay NOT-READY.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use paper_runner::{
    PaperMode, PersistentStartup, RecoveredStartup, StartupReadiness, startup, startup_persistent,
};
use risk_authority::authority::{
    AccountRecoveryRecord, DailyPnlBasis, DailyPnlCalendar, DailyPnlRecord, DailyPnlRecovery,
    ExecutionRecoveryRecord, daily_pnl_positions_digest,
};
use risk_authority::{
    ACCOUNT_ANCHOR_FILE, ACCOUNT_HIGH_WATER_FILE, ACCOUNT_JOURNAL_FILE, ACCOUNT_JOURNAL_ID,
    AuthorityOwnership, BINDING_SUFFIX, DAILY_PNL_FILE, FileControlStore, FileDecisionStore,
    InMemoryDecisionStore, JournalIdentity, NoopControlStore, OMS_ANCHOR_FILE, OMS_EXECUTIONS_FILE,
    OMS_HIGH_WATER_FILE, OMS_JOURNAL_ID, OMS_PERMITS_FILE, ObservationSource, OmsPermitRecord,
    OwnerSigningKey, RecoveryAnchor, RecoveryAuthenticity, RecoveryHighWater, genesis_digest,
    recovery_binding_record,
};
use risk_engine::{RiskPolicy, ValidatedRiskPolicy};
use trading_core::{AccountId, AlgoId, InstrumentId, Paise, Quantity, StrategyId, TradeDate};

const ACCOUNT: u64 = 1;
const STRATEGY: u64 = 2;
const ALGO: u64 = 3;
const INSTRUMENT: &str = "paper.neutral";

fn account() -> AccountId {
    AccountId::new(ACCOUNT).expect("account")
}

fn date() -> TradeDate {
    TradeDate::new(2026, 1, 1).expect("date")
}

fn instrument_id() -> InstrumentId {
    InstrumentId::new(INSTRUMENT).expect("instrument")
}

fn policy() -> ValidatedRiskPolicy {
    let raw = RiskPolicy {
        version: "paper-startup-neutral".to_owned(),
        allowed_accounts: [account()].into_iter().collect(),
        allowed_strategies: [StrategyId::new(STRATEGY).expect("strategy")]
            .into_iter()
            .collect(),
        allowed_algos: [AlgoId::new(ALGO).expect("algo")].into_iter().collect(),
        allowed_instruments: [instrument_id()].into_iter().collect(),
        max_order_quantity: Quantity::new(100).expect("quantity"),
        max_order_notional_paise: Paise(1_000_000),
        max_position_quantity: Quantity::new(100).expect("quantity"),
        max_gross_exposure_paise: Paise(1_000_000),
        max_daily_loss_paise: Paise(5_000),
        max_open_orders: 100,
        max_orders_per_second: 100,
        max_price_deviation_ppm: 100_000,
        max_quote_age_ns: 1_000,
        allow_short: false,
        max_strategy_exposure_paise: Paise(1_000_000),
        max_sector_exposure_paise: None,
        stress_scenario: None,
        max_stress_loss_paise: None,
        max_participation_ppm: None,
        greek_limits: None,
        max_borrow_quantity: None,
        max_financing_cost_paise: None,
    };
    ValidatedRiskPolicy::from_policy(&raw).expect("neutral policy")
}

fn owner_key() -> OwnerSigningKey {
    OwnerSigningKey::new(b"paper-startup-neutral-owner-key-20261127".to_vec()).expect("owner key")
}

fn oms_identity() -> JournalIdentity {
    JournalIdentity::new(ObservationSource::Oms, account(), OMS_JOURNAL_ID)
}

fn account_identity() -> JournalIdentity {
    JournalIdentity::new(
        ObservationSource::AccountJournal,
        account(),
        ACCOUNT_JOURNAL_ID,
    )
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) {
    let text = serde_json::to_string(value).expect("serialize fixture");
    std::fs::write(path, text).expect("write fixture");
}

fn write_json_lines<T: serde::Serialize>(path: &Path, records: &[T]) {
    let mut text = String::new();
    for record in records {
        text.push_str(&serde_json::to_string(record).expect("serialize record"));
        text.push('\n');
    }
    std::fs::write(path, text).expect("write journal");
}

/// Lay down a complete, neutral, owner-signed recovery root. OMS journals are
/// empty; the account journal has one zero-delta event so account-snapshot
/// evidence exists. No market-shaped data or execution fact is present.
fn write_neutral_owner_root(root: &Path) {
    let empty_permits: Vec<OmsPermitRecord> = Vec::new();
    let empty_executions: Vec<ExecutionRecoveryRecord> = Vec::new();
    write_json_lines(&root.join(OMS_PERMITS_FILE), &empty_permits);
    write_json_lines(&root.join(OMS_EXECUTIONS_FILE), &empty_executions);

    let mut event =
        AccountRecoveryRecord::new("paper-neutral".to_owned(), account(), instrument_id(), 0);
    event.seal(1, genesis_digest());
    let account_records = vec![event];
    write_json_lines(&root.join(ACCOUNT_JOURNAL_FILE), &account_records);

    let calendar = DailyPnlCalendar::new(
        "Asia/Kolkata".to_owned(),
        date(),
        -86_400_000_000_000,
        86_400_000_000_000,
    );
    let positions = BTreeMap::new();
    let mut record = DailyPnlRecord::new(
        account(),
        date(),
        "INR".to_owned(),
        DailyPnlBasis::RealizedPlusUnrealized,
        &calendar,
        0,
    )
    .with_journal(1, account_records[0].digest.clone())
    .with_digests(
        daily_pnl_positions_digest(&positions),
        genesis_digest().to_owned(),
    );
    record.seal(1, genesis_digest());
    let daily = DailyPnlRecovery {
        record: Some(record),
        valuations: Vec::new(),
        calendar: Some(calendar),
    };
    write_json(&root.join(DAILY_PNL_FILE), &daily);

    let key = owner_key();
    write_json(
        &root.join(format!("{OMS_PERMITS_FILE}{BINDING_SUFFIX}")),
        &recovery_binding_record(&key, &oms_identity(), &empty_permits, &empty_executions)
            .expect("oms binding"),
    );
    write_json(
        &root.join(format!("{ACCOUNT_JOURNAL_FILE}{BINDING_SUFFIX}")),
        &recovery_binding_record(&key, &account_identity(), &account_records, &daily)
            .expect("account binding"),
    );

    let (oms_sequence, oms_digest) = (0_u64, genesis_digest().to_owned());
    let (account_sequence, account_digest) = (1_u64, account_records[0].digest.clone());
    write_json(
        &root.join(OMS_ANCHOR_FILE),
        &RecoveryAnchor::seal(
            &key,
            &oms_identity(),
            oms_sequence.max(1),
            oms_sequence,
            &oms_digest,
        )
        .expect("oms anchor"),
    );
    write_json(
        &root.join(ACCOUNT_ANCHOR_FILE),
        &RecoveryAnchor::seal(
            &key,
            &account_identity(),
            account_sequence.max(1),
            account_sequence,
            &account_digest,
        )
        .expect("account anchor"),
    );
    write_json(
        &root.join(OMS_HIGH_WATER_FILE),
        &RecoveryHighWater::seal(&key, &oms_identity(), oms_sequence.max(1))
            .expect("oms high water"),
    );
    write_json(
        &root.join(ACCOUNT_HIGH_WATER_FILE),
        &RecoveryHighWater::seal(&key, &account_identity(), account_sequence.max(1))
            .expect("account high water"),
    );
}

struct Fixture {
    _root: tempfile::TempDir,
    root: PathBuf,
    base: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root = tempfile::tempdir().expect("neutral fixture root");
        write_neutral_owner_root(root.path());
        let path = root.path().to_path_buf();
        Self {
            _root: root,
            base: path.join(label),
            root: path,
        }
    }

    fn config(&self) -> PersistentStartup {
        PersistentStartup {
            recovery_root: self.root.clone(),
            authenticity: RecoveryAuthenticity::OwnerSignature(owner_key()),
            journal_ownership_path: self.base.with_extension("journal.lock"),
            authority_ownership_path: self.base.with_extension("authority.lock"),
            control: Box::new(FileControlStore::new(
                self.base.with_extension("control.jsonl"),
            )),
            decisions: Box::new(FileDecisionStore::new(
                self.base.with_extension("decisions.jsonl"),
            )),
            sectors: BTreeMap::new(),
        }
    }

    fn recover(&self) -> Result<RecoveredStartup, String> {
        startup_persistent(account(), policy(), Vec::new(), date(), self.config())
    }
}

fn expect_startup_err(result: Result<RecoveredStartup, String>, message: &str) -> String {
    match result {
        Ok(_) => panic!("{message}"),
        Err(error) => error,
    }
}

#[test]
fn production_startup_wires_the_recovery_loader() {
    let source = include_str!("../src/lib.rs");
    assert!(
        source.contains("AuthorityOwnership::acquire"),
        "startup must acquire account ownership before journal loading"
    );
    assert!(
        source.contains("RecoveryJournalRoot"),
        "startup must drive the library recovery loader"
    );
    assert!(
        source.contains("RecoveryJournalRoot::new"),
        "startup must pin the owner-controlled recovery root"
    );
    let acquire = source
        .find("AuthorityOwnership::acquire")
        .expect("ownership before load");
    let load = source
        .find("RecoveryJournalRoot::new")
        .expect("loader wiring");
    assert!(
        acquire < load,
        "ownership acquisition must appear before journal loading"
    );
}

#[test]
fn executable_has_an_explicit_persistent_entry_point() {
    let source = include_str!("../src/bin/paper_run.rs");
    assert!(
        source.contains("--persistent"),
        "the executable must expose an explicit persistent startup option"
    );
    let library = include_str!("../src/lib.rs");
    assert!(
        library.contains("pub fn startup_persistent"),
        "the library must expose the cohesive persistent startup API"
    );
}

#[test]
fn persistent_startup_fails_without_recovery_root_or_adapters() {
    let missing = match startup(
        PaperMode::PersistentPaper,
        account(),
        policy(),
        Vec::new(),
        date(),
        None,
    ) {
        Ok(_) => panic!("persistent mode must fail closed without adapters"),
        Err(error) => error,
    };
    assert!(
        missing.contains("recovery") && missing.contains("adapter"),
        "missing adapters error names recovery adapters: {missing}"
    );

    let root = tempfile::tempdir().expect("temporary root");
    let base = root.path().join("no-adapters");
    let non_durable = PersistentStartup {
        recovery_root: root.path().to_path_buf(),
        authenticity: RecoveryAuthenticity::OwnerSignature(owner_key()),
        journal_ownership_path: base.with_extension("journal.lock"),
        authority_ownership_path: base.with_extension("authority.lock"),
        control: Box::new(NoopControlStore),
        decisions: Box::new(InMemoryDecisionStore::new()),
        sectors: BTreeMap::new(),
    };
    let error = expect_startup_err(
        startup_persistent(account(), policy(), Vec::new(), date(), non_durable),
        "non-durable stores must fail closed",
    );
    assert!(
        error.contains("NOT-READY"),
        "failure is explicitly not-ready: {error}"
    );
}

#[test]
fn ownership_is_acquired_before_the_recovery_loader() {
    let fixture = Fixture::new("competing-owner");
    // Make the loader itself fail. If startup reached it, the error would name
    // a missing journal rather than a competing owner.
    let missing_journal = fixture.root.join(OMS_PERMITS_FILE);
    std::fs::remove_file(&missing_journal).expect("remove journal");

    let competitor = AuthorityOwnership::acquire(fixture.base.with_extension("journal.lock"))
        .expect("first owner");
    let error = expect_startup_err(
        fixture.recover(),
        "a competing owner must fail the transaction",
    );
    assert!(
        error.contains("ownership"),
        "ownership is checked before the loader: {error}"
    );
    assert!(
        !error.contains("recovery load failed"),
        "the loader was never reached: {error}"
    );
    drop(competitor);

    // After the competitor releases, a loader failure is reported instead.
    let error = expect_startup_err(fixture.recover(), "a missing journal must fail closed");
    assert!(
        error.contains("NOT-READY") && error.contains("load"),
        "loader failure is fail-closed and explicit: {error}"
    );
}

#[test]
fn loader_failure_prevents_authority_construction() {
    let fixture = Fixture::new("loader-failure");
    std::fs::remove_file(fixture.root.join(ACCOUNT_ANCHOR_FILE)).expect("remove terminal anchor");

    let error = expect_startup_err(fixture.recover(), "loader must fail");
    assert!(
        error.contains("NOT-READY"),
        "loader failure is NOT-READY: {error}"
    );
    assert!(
        !fixture.base.with_extension("authority.lock").exists(),
        "no authority was constructed, so its ownership lock was never taken"
    );
    assert!(
        !fixture.base.with_extension("journal.lock").exists(),
        "the pre-load ownership guard was released on failure"
    );
}

#[test]
fn valid_neutral_fixtures_yield_a_recovered_authority_still_not_ready() {
    let fixture = Fixture::new("valid-neutral");
    let result = fixture.recover().expect("neutral startup succeeds");
    assert_eq!(
        result.readiness,
        StartupReadiness::NotReady,
        "startup must remain not-ready"
    );
    assert_eq!(
        result.startup.mode,
        PaperMode::PersistentPaper,
        "mode must be persistent paper"
    );
    assert!(
        result.startup.identity.label.contains("recovered"),
        "identity is the explicit recovered label"
    );
    assert!(
        !result.startup.authority.is_simulation(),
        "a recovered authority is not a simulation authority"
    );
    assert!(
        result.startup.authority.startup_error().is_none(),
        "account and OMS reconciliation completed"
    );
    if !result
        .not_ready_reasons
        .iter()
        .any(|reason| reason.contains("observations"))
    {
        eprintln!("readiness reasons: {:?}", result.not_ready_reasons);
    }
    assert!(
        result
            .not_ready_reasons
            .iter()
            .any(|reason| reason.contains("observations")),
        "readiness still requires owner observations"
    );
    assert!(
        fixture.base.with_extension("journal.lock").exists(),
        "ownership is held for the lifetime of the recovered startup"
    );
}

#[test]
fn executable_default_is_ephemeral_and_has_no_external_adapter() {
    let binary = include_str!("../src/bin/paper_run.rs");
    assert!(
        binary.contains("run_paper("),
        "the default executable path uses the ephemeral harness"
    );
    assert!(
        binary.contains("mode="),
        "the default run is explicitly labelled in the executable output"
    );
    let library = include_str!("../src/lib.rs");
    for forbidden in [
        "reqwest",
        "upstox_feed_proto",
        "fyers",
        "kite",
        "live_execution",
        "LiveBroker",
    ] {
        assert!(
            !library.contains(forbidden),
            "no external execution or provider path in the harness: {forbidden}"
        );
    }
}
