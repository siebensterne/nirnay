// SPDX-License-Identifier: Apache-2.0
//! Neutral-workload measurements of REAL Nirnay production components.
//!
//! This harness calls the real functions: `risk_engine::evaluate_risk`, the
//! fenced authoritative-OMS store facade (`fence`, `create_fenced`,
//! `accept_risk_approved_fenced`, `claim_fenced`, `mark_dispatching_fenced`,
//! `acknowledge_fenced`, `apply_typed_fill_fenced`), `account_journal::fence`
//! and `append_fenced`, replay/recovery (`OmsStore::recover`, `Journal::open`,
//! `Engine::open`), and `paper_engine::Engine::submit_decision` / `reconcile`.
//! No production path is reimplemented and no analogue is relabelled.
//!
//! The workload is neutral: opaque byte payloads derived from a seed plus
//! monotonically increasing `u64` sequence values. There are no instruments,
//! quotes, books, prices, trades, invented market observations, provider,
//! broker, live-execution path or market-shaped test source. Fixed identity,
//! calendar, sentinel and reference-data values below are opaque scaffolding
//! required by the real production APIs; they are identical in every
//! repetition, carry no measurement meaning and never encode workload data.
//! Risk cases deliberately supply no instrument and no quote, so risk
//! evaluation exercises the real fail-closed path. Paper decisions carry no
//! order intent, so no quote, book or fill model is driven.
//!
//! IMPORTANT: these are neutral-workload measurements of real components.
//! They are NOT a production latency or throughput certificate, NOT
//! nanosecond/HFT timing, NOT horizontal-scale evidence, and NO resource
//! maxima are set. Correctness checks run before and after the measured pass;
//! event order, journal/account bytes, risk decisions and replay/reconciliation
//! state must be byte-identical or the harness fails.
//!
//! Usage:
//!   cargo bench -p paper-runner --bench production_components --no-run
//!   cargo bench -p paper-runner --bench production_components -- --self-test
//!   cargo bench -p paper-runner --bench production_components -- --run --out RESULT.json

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};

use account_journal::{
    FencingToken as JournalFencingToken, //
    Journal,
};
use authoritative_oms::{
    BrokerOrderId, //
    ChargeStatus,
    ClientOrderId,
    FencingToken as OmsFencingToken,
    FillEvent,
    OmsStore,
    PermitRef,
};
use paper_engine::{
    Engine, //
    EngineConfig,
    PaperProfile,
    ReconcileInput,
};
use risk_engine::{
    RiskContext, //
    RiskPolicy,
    ValidatedRiskPolicy,
    evaluate_risk,
};
use sha2::{
    Digest, //
    Sha256,
};
use trading_core::{
    AccountId, //
    AlgoId,
    DataQualityFlag,
    DecisionRecord,
    EventEnvelope,
    EventId,
    EventPayload,
    EventTimes,
    FeedHealth,
    Instrument,
    InstrumentCatalog,
    InstrumentId,
    InstrumentType,
    IntentId,
    Isin,
    JournalRef,
    Listing,
    ListingStatus,
    ListingTerms,
    LogicalNanos,
    OrderIntent,
    Paise,
    PricePaise,
    Product,
    Quantity,
    RunId,
    Segment,
    SessionPhase,
    SettlementRegime,
    Side,
    StoredBody,
    StrategyId,
    TickAssignment,
    TradeDate,
    UtcNanos,
    Venue,
    permit_digest,
};

const BASE_SHA: &str = "08867faa14d03f2cfb313223ea41f63740b2587e";
const OWNER: &str = "neutral-production-bench";
const WORKER: &str = "neutral-bench-worker";
const STREAM_ID: &str = "neutral-production-bench-account";
const POLICY_VERSION: &str = "neutral-bench-risk-v1";
const RUN_ID_BYTES: [u8; 16] = [0x4e; 16];

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

fn digest_bytes(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn digest_hex(text: &str) -> String {
    digest_bytes(text.as_bytes())
}

fn escape_json(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other if other.is_control() => {
                out.push_str("\\u");
                out.push_str(&format!("{:04x}", other as u32));
            }
            other => out.push(other),
        }
    }
    out
}

struct Json {
    fields: Vec<String>,
}

impl Json {
    fn new() -> Self {
        Self { fields: Vec::new() }
    }

    fn string(&mut self, key: &str, value: &str) {
        self.raw(key, &format!("\"{}\"", escape_json(value)));
    }

    fn uint(&mut self, key: &str, value: u64) {
        self.raw(key, &value.to_string());
    }

    fn uint128(&mut self, key: &str, value: u128) {
        self.raw(key, &value.to_string());
    }

    fn float(&mut self, key: &str, value: f64) {
        if value.is_finite() {
            self.raw(key, &format!("{value:.6}"));
        } else {
            self.raw(key, "null");
        }
    }

    fn boolean(&mut self, key: &str, value: bool) {
        self.raw(key, if value { "true" } else { "false" });
    }

    fn object(&mut self, key: &str, value: &Json) {
        self.raw(key, &value.render());
    }

    fn array(&mut self, key: &str, values: &[Json]) {
        let rendered: Vec<String> = values.iter().map(Json::render).collect();
        self.raw(key, &format!("[{}]", rendered.join(",")));
    }

    fn raw(&mut self, key: &str, value: &str) {
        self.fields
            .push(format!("\"{}\":{}", escape_json(key), value));
    }

    fn render(&self) -> String {
        format!("{{{}}}", self.fields.join(","))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    RiskEvaluate,
    OmsFence,
    OmsCreateFenced,
    OmsAcceptRiskApprovedFenced,
    OmsClaimFenced,
    OmsMarkDispatchingFenced,
    OmsAcknowledgeFenced,
    OmsApplyTypedFillFenced,
    JournalFence,
    JournalAppendFenced,
    PaperSubmitDecision,
    PaperReconcile,
    ReplayRecovery,
    PipelineTotal,
}

impl Stage {
    fn all() -> [Stage; 14] {
        [
            Stage::RiskEvaluate,
            Stage::OmsFence,
            Stage::OmsCreateFenced,
            Stage::OmsAcceptRiskApprovedFenced,
            Stage::OmsClaimFenced,
            Stage::OmsMarkDispatchingFenced,
            Stage::OmsAcknowledgeFenced,
            Stage::OmsApplyTypedFillFenced,
            Stage::JournalFence,
            Stage::JournalAppendFenced,
            Stage::PaperSubmitDecision,
            Stage::PaperReconcile,
            Stage::ReplayRecovery,
            Stage::PipelineTotal,
        ]
    }

    fn as_str(self) -> &'static str {
        match self {
            Stage::RiskEvaluate => "risk_evaluate_risk",
            Stage::OmsFence => "oms_fence",
            Stage::OmsCreateFenced => "oms_create_fenced",
            Stage::OmsAcceptRiskApprovedFenced => "oms_accept_risk_approved_fenced",
            Stage::OmsClaimFenced => "oms_claim_fenced",
            Stage::OmsMarkDispatchingFenced => "oms_mark_dispatching_fenced",
            Stage::OmsAcknowledgeFenced => "oms_acknowledge_fenced",
            Stage::OmsApplyTypedFillFenced => "oms_apply_typed_fill_fenced",
            Stage::JournalFence => "journal_fence",
            Stage::JournalAppendFenced => "journal_append_fenced",
            Stage::PaperSubmitDecision => "paper_submit_decision",
            Stage::PaperReconcile => "paper_reconcile",
            Stage::ReplayRecovery => "replay_recovery",
            Stage::PipelineTotal => "pipeline_total",
        }
    }
}

struct Config {
    self_test: bool,
    run: bool,
    help: bool,
    events: u64,
    warmup_events: u64,
    repetitions: usize,
    payload_bytes: usize,
    seed: u64,
    out: Option<PathBuf>,
}

impl Config {
    fn defaults() -> Self {
        Self {
            self_test: false,
            run: false,
            help: false,
            events: 256,
            warmup_events: 16,
            repetitions: 3,
            payload_bytes: 64,
            seed: 0x4e69_726e_6179_0000,
            out: None,
        }
    }
}

#[derive(Clone)]
struct Event {
    sequence: u64,
    payload: Vec<u8>,
}

fn generate_events(seed: u64, count: u64, size: usize) -> Vec<Event> {
    let mut events = Vec::with_capacity(count as usize);
    for sequence in 0..count {
        let mut state = seed ^ sequence.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let mut payload = Vec::with_capacity(size);
        for index in 0..size {
            state ^= state.wrapping_shl(13);
            state ^= state.wrapping_shr(7);
            state ^= state.wrapping_shl(17);
            let mixed = state ^ (index as u64).wrapping_mul(0xd1b5_4a32_d192_ed03);
            payload.push(mixed as u8);
        }
        events.push(Event { sequence, payload });
    }
    events
}

fn logical_time(sequence: u64) -> LogicalNanos {
    LogicalNanos::new(sequence + 1)
}

fn event_times(sequence: u64) -> EventTimes {
    EventTimes {
        source_time_utc: None,
        received_time_utc: UtcNanos::new(1),
        available_time: logical_time(sequence),
        recorded_time_utc: UtcNanos::new(1),
    }
}

fn trade_date() -> Result<TradeDate, String> {
    TradeDate::new(2026, 1, 1).map_err(|error| format!("invalid fixed trade date: {error}"))
}

fn instrument_id() -> Result<InstrumentId, String> {
    InstrumentId::new("neutral.component.bench")
        .map_err(|error| format!("invalid fixed instrument identity: {error}"))
}

fn scaffold_instrument() -> Result<Instrument, String> {
    let date = trade_date()?;
    let identity = instrument_id()?;
    let isin =
        Isin::new("INE002A01018").map_err(|error| format!("invalid scaffold ISIN: {error}"))?;
    Ok(Instrument {
        instrument_id: identity,
        instrument_type: InstrumentType::Equity,
        isin: Some(isin),
        name: "neutral component benchmark scaffold".to_owned(),
        underlying_instrument_id: None,
        expiry_date: None,
        strike_price_paise: None,
        option_kind: None,
        listings: vec![Listing {
            venue: Venue::Nse,
            segment: Segment::Cash,
            venue_symbol: "NEUTRAL".to_owned(),
            token: None,
            effective_from: date,
            effective_to: None,
            status: ListingStatus::Active,
            settlement_regime: SettlementRegime::Standard,
            z_category: false,
            settlement_group: None,
            terms: vec![ListingTerms {
                effective_from: date,
                effective_to: None,
                tick_assignment: TickAssignment {
                    tick_paise: 1,
                    reference_close_paise: None,
                    reference_close_date: None,
                    reference_source: None,
                    reference_sha256_hex: None,
                    effective_month: None,
                    schedule_revision: "neutral-bench-v1".to_owned(),
                    schedule_effective_from: None,
                    schedule_sha256_hex: None,
                    calendar_revision: None,
                    calendar_sha256_hex: None,
                },
                market_lot: 1,
                quantity_increment: 1,
            }],
        }],
        corporate_actions: Vec::new(),
    })
}

fn risk_policy() -> Result<RiskPolicy, String> {
    let account = AccountId::new(1).map_err(|error| format!("fixed account id: {error}"))?;
    let strategy = StrategyId::new(2).map_err(|error| format!("fixed strategy id: {error}"))?;
    let algo = AlgoId::new(3).map_err(|error| format!("fixed algo id: {error}"))?;
    Ok(RiskPolicy {
        version: POLICY_VERSION.to_owned(),
        allowed_accounts: BTreeSet::from([account]),
        allowed_strategies: BTreeSet::from([strategy]),
        allowed_algos: BTreeSet::from([algo]),
        allowed_instruments: BTreeSet::from([instrument_id()?]),
        max_order_quantity: Quantity::new(1).map_err(|error| format!("fixed quantity: {error}"))?,
        max_order_notional_paise: Paise(1),
        max_position_quantity: Quantity::new(1)
            .map_err(|error| format!("fixed position limit: {error}"))?,
        max_gross_exposure_paise: Paise(1),
        max_daily_loss_paise: Paise(1),
        max_open_orders: 1,
        max_orders_per_second: 1,
        max_price_deviation_ppm: 1_000_000,
        max_quote_age_ns: 1,
        allow_short: false,
        max_strategy_exposure_paise: Paise(1),
        max_sector_exposure_paise: None,
        stress_scenario: None,
        max_stress_loss_paise: None,
        max_participation_ppm: None,
        greek_limits: None,
        max_borrow_quantity: None,
        max_financing_cost_paise: None,
    })
}

fn engine_config() -> Result<EngineConfig, String> {
    let date = trade_date()?;
    let account = AccountId::new(1).map_err(|error| format!("fixed account id: {error}"))?;
    // Fixed, zero-rate schedule. The target type is inferred from the
    // EngineConfig field; no market observation is introduced.
    let rates = serde_json::json!([
        {"effective_from": "2026-01-01", "product": "delivery", "side": "Buy", "brokerage_ppb": 0, "stt_ppb": 0, "exchange_ppb": 0, "sebi_ppb": 0, "stamp_ppb": 0, "gst_ppb": 0},
        {"effective_from": "2026-01-01", "product": "delivery", "side": "Sell", "brokerage_ppb": 0, "stt_ppb": 0, "exchange_ppb": 0, "sebi_ppb": 0, "stamp_ppb": 0, "gst_ppb": 0},
        {"effective_from": "2026-01-01", "product": "intraday", "side": "Buy", "brokerage_ppb": 0, "stt_ppb": 0, "exchange_ppb": 0, "sebi_ppb": 0, "stamp_ppb": 0, "gst_ppb": 0},
        {"effective_from": "2026-01-01", "product": "intraday", "side": "Sell", "brokerage_ppb": 0, "stt_ppb": 0, "exchange_ppb": 0, "sebi_ppb": 0, "stamp_ppb": 0, "gst_ppb": 0}
    ]);
    let charge_book = serde_json::from_value(rates)
        .map_err(|error| format!("fixed zero-rate charge book: {error}"))?;
    Ok(EngineConfig {
        stream_id: STREAM_ID.to_owned(),
        run_id: RunId::from_bytes(RUN_ID_BYTES),
        account_id: account,
        starting_cash: Paise(1),
        trade_date: date,
        instruments: InstrumentCatalog::new(vec![scaffold_instrument()?])
            .map_err(|error| format!("fixed instrument catalog: {error}"))?,
        risk_policy: risk_policy()?,
        charge_book,
        paper_profile: PaperProfile {
            slippage_ticks: 0,
            consume_displayed_liquidity_only: true,
        },
    })
}

struct Workload {
    oms_path: PathBuf,
    account_path: PathBuf,
    paper_path: PathBuf,
    oms: Option<OmsStore>,
    oms_token: Option<OmsFencingToken>,
    attempt: Option<i64>,
    journal: Option<Journal>,
    journal_token: Option<JournalFencingToken>,
    engine: Option<Engine>,
    policy: ValidatedRiskPolicy,
}

impl Workload {
    fn new(label: &str) -> Result<Self, String> {
        let root = env::temp_dir().join(format!(
            "nirnay-prod-components-{label}-{}-{:016x}",
            std::process::id(),
            0x5a5a_u64
        ));
        if root.exists() {
            fs::remove_dir_all(&root)
                .map_err(|error| format!("clearing {}: {error}", root.display()))?;
        }
        fs::create_dir_all(&root)
            .map_err(|error| format!("creating {}: {error}", root.display()))?;
        let oms_path = root.join("oms.journal");
        let account_path = root.join("account.journal");
        let paper_path = root.join("paper.journal");
        let policy = ValidatedRiskPolicy::from_policy(&risk_policy()?)
            .map_err(|error| format!("validating fixed risk policy: {error}"))?;
        let mut oms = OmsStore::open(&oms_path).map_err(|error| format!("opening OMS: {error}"))?;
        let oms_token = oms
            .fence(OWNER)
            .map_err(|error| format!("OMS fence: {error}"))?;
        let mut journal = Journal::open(&account_path, STREAM_ID)
            .map_err(|error| format!("opening account journal: {error}"))?;
        let journal_token = journal
            .fence(OWNER)
            .map_err(|error| format!("account journal fence: {error}"))?;
        let engine = Engine::open(&paper_path, engine_config()?, event_times(0))
            .map_err(|error| format!("opening paper engine: {error}"))?;
        Ok(Self {
            oms_path,
            account_path,
            paper_path,
            oms: Some(oms),
            oms_token: Some(oms_token),
            attempt: None,
            journal: Some(journal),
            journal_token: Some(journal_token),
            engine: Some(engine),
            policy,
        })
    }

    fn intent(&self, event: &Event) -> Result<OrderIntent, String> {
        let evidence_hash = digest_hex(&hex(&event.payload));
        Ok(OrderIntent {
            intent_id: IntentId::new(event.sequence + 1)
                .map_err(|error| format!("fixed intent id: {error}"))?,
            account_id: AccountId::new(1).map_err(|error| format!("fixed account id: {error}"))?,
            strategy_id: StrategyId::new(2)
                .map_err(|error| format!("fixed strategy id: {error}"))?,
            strategy_version: "neutral-bench-v1".to_owned(),
            algo_id: AlgoId::new(3).map_err(|error| format!("fixed algo id: {error}"))?,
            instrument_id: instrument_id()?,
            venue: Venue::Nse,
            side: Side::Buy,
            product: Product::Delivery,
            quantity: Quantity::new(1).map_err(|error| format!("fixed quantity: {error}"))?,
            limit_price: PricePaise::new(Paise(1))
                .map_err(|error| format!("fixed sentinel price: {error}"))?,
            reason: "neutral-workload-benchmark".to_owned(),
            evidence: vec![JournalRef {
                sequence: 1,
                hash_hex: evidence_hash,
            }],
            expires_at: LogicalNanos::new(1_000_000_000_u64.wrapping_add(event.sequence)),
        })
    }

    fn fill(&self, event: &Event) -> Result<FillEvent, String> {
        let client = ClientOrderId::new(event.sequence + 1)
            .map_err(|error| format!("fixed client order id: {error}"))?;
        Ok(FillEvent {
            schema_version: 1,
            execution_id: format!("neutral-exec-{}", event.sequence),
            broker_event_id: format!("neutral-fill-{}", event.sequence),
            client_order_id: client,
            oms_order_id: client.to_string(),
            side: Side::Buy,
            quantity: 1,
            price_paise: 1,
            currency: "INR".to_owned(),
            broker_timestamp_ns: 1,
            received_at_ns: event.sequence as i64 + 1,
            source: "neutral-production-benchmark".to_owned(),
            account_scope: "neutral".to_owned(),
            charges_paise: 0,
            charge_status: ChargeStatus::Estimated,
            provenance: "opaque-neutral-workload".to_owned(),
        })
    }

    fn risk(&self, event: &Event) -> Result<String, String> {
        let intent = self.intent(event)?;
        let context = RiskContext {
            account_id: AccountId::new(1).map_err(|error| format!("fixed account id: {error}"))?,
            strategy_id: StrategyId::new(2)
                .map_err(|error| format!("fixed strategy id: {error}"))?,
            venue: Venue::Nse,
            instrument: None,
            quote: None,
            feed_health: FeedHealth::NoQuote,
            trade_date: trade_date()?,
            now: logical_time(event.sequence),
            position_quantity: 0,
            pending_buy_quantity: Quantity::ZERO,
            pending_sell_quantity: Quantity::ZERO,
            gross_exposure_paise: Paise(0),
            pending_gross_exposure_paise: Paise(0),
            strategy_exposure_paise: Paise(0),
            sector_label: None,
            sector_exposure_paise: Paise(0),
            positions: BTreeMap::new(),
            marks_paise: BTreeMap::new(),
            depth: None,
            greeks: None,
            is_derivative: false,
            borrow: None,
            daily_pnl: Paise(0),
            open_orders: 0,
            orders_last_second: 0,
            session_open: false,
            account_snapshot_available: false,
            kill_switch_engaged: false,
            reconciliation_blocked: false,
            duplicate_intent: false,
        };
        let assessment = evaluate_risk(&intent, &self.policy, &context);
        let mut checks = String::new();
        for check in &assessment.checks {
            checks.push(if check.passed { '1' } else { '0' });
        }
        Ok(format!(
            "{}|{}|{}|{}",
            assessment.intent_id, assessment.policy_version, assessment.accepted, checks
        ))
    }

    fn oms_fence(&mut self) -> Result<String, String> {
        let token = {
            let store = self
                .oms
                .as_mut()
                .ok_or_else(|| "OMS writer was taken by replay".to_owned())?;
            store
                .fence(OWNER)
                .map_err(|error| format!("OMS fence: {error}"))?
        };
        let term = token.term().get();
        self.oms_token = Some(token);
        Ok(format!("term={term}"))
    }

    fn oms_create(&mut self, event: &Event) -> Result<String, String> {
        let client = ClientOrderId::new(event.sequence + 1)
            .map_err(|error| format!("fixed client order id: {error}"))?;
        let intent = self.intent(event)?;
        let store = self
            .oms
            .as_mut()
            .ok_or_else(|| "OMS writer was taken by replay".to_owned())?;
        let token = self
            .oms_token
            .as_ref()
            .ok_or_else(|| "no current OMS fencing token".to_owned())?;
        let permit = PermitRef {
            permit_id: event.sequence + 1,
            policy_version: POLICY_VERSION.to_owned(),
            snapshot_revision: 1,
            digest: permit_digest(event.sequence + 1, POLICY_VERSION, 1, &intent),
        };
        store
            .create_fenced(token, client, &intent, &permit, event.sequence as i64 + 1)
            .map_err(|error| format!("create_fenced: {error}"))?;
        Ok(format!("{}|{}", client, permit.digest))
    }

    fn oms_accept(&mut self, event: &Event) -> Result<String, String> {
        let store = self
            .oms
            .as_mut()
            .ok_or_else(|| "OMS writer was taken by replay".to_owned())?;
        let token = self
            .oms_token
            .as_ref()
            .ok_or_else(|| "no current OMS fencing token".to_owned())?;
        let client = ClientOrderId::new(event.sequence + 1)
            .map_err(|error| format!("fixed client order id: {error}"))?;
        store
            .accept_risk_approved_fenced(token, client, event.sequence as i64 + 2)
            .map_err(|error| format!("accept_risk_approved_fenced: {error}"))?;
        Ok(client.to_string())
    }

    fn oms_claim(&mut self, event: &Event) -> Result<String, String> {
        let claim = {
            let store = self
                .oms
                .as_mut()
                .ok_or_else(|| "OMS writer was taken by replay".to_owned())?;
            let token = self
                .oms_token
                .as_ref()
                .ok_or_else(|| "no current OMS fencing token".to_owned())?;
            store
                .claim_fenced(token, WORKER, event.sequence as i64 + 3, 1_000)
                .map_err(|error| format!("claim_fenced: {error}"))?
        };
        let claim = claim.ok_or_else(|| "no pending OMS command to claim".to_owned())?;
        self.attempt = Some(claim.attempt_id);
        Ok(format!("{}|{}", claim.attempt_id, claim.client_order_id))
    }

    fn oms_dispatch(&mut self, event: &Event) -> Result<String, String> {
        let store = self
            .oms
            .as_mut()
            .ok_or_else(|| "OMS writer was taken by replay".to_owned())?;
        let token = self
            .oms_token
            .as_ref()
            .ok_or_else(|| "no current OMS fencing token".to_owned())?;
        let attempt = self
            .attempt
            .ok_or_else(|| "no claimed OMS attempt to dispatch".to_owned())?;
        store
            .mark_dispatching_fenced(token, attempt, event.sequence as i64 + 4)
            .map_err(|error| format!("mark_dispatching_fenced: {error}"))?;
        Ok(attempt.to_string())
    }

    fn oms_acknowledge(&mut self, event: &Event) -> Result<String, String> {
        let store = self
            .oms
            .as_mut()
            .ok_or_else(|| "OMS writer was taken by replay".to_owned())?;
        let token = self
            .oms_token
            .as_ref()
            .ok_or_else(|| "no current OMS fencing token".to_owned())?;
        let attempt = self
            .attempt
            .ok_or_else(|| "no claimed OMS attempt to acknowledge".to_owned())?;
        let broker = BrokerOrderId::new(format!("neutral-broker-{}", event.sequence))
            .map_err(|error| format!("fixed broker identity: {error}"))?;
        let accepted = store
            .acknowledge_fenced(
                token,
                attempt,
                &broker,
                &format!("neutral-ack-{}", event.sequence),
                event.sequence as i64 + 5,
            )
            .map_err(|error| format!("acknowledge_fenced: {error}"))?;
        Ok(format!("{attempt}|{}|{accepted}", broker.as_str()))
    }

    fn oms_fill(&mut self, event: &Event) -> Result<String, String> {
        let fill = self.fill(event)?;
        let store = self
            .oms
            .as_mut()
            .ok_or_else(|| "OMS writer was taken by replay".to_owned())?;
        let token = self
            .oms_token
            .as_ref()
            .ok_or_else(|| "no current OMS fencing token".to_owned())?;
        let applied = store
            .apply_typed_fill_fenced(token, fill)
            .map_err(|error| format!("apply_typed_fill_fenced: {error}"))?;
        Ok(format!("{}|{applied}", event.sequence))
    }

    fn journal_fence(&mut self) -> Result<String, String> {
        let token = {
            let journal = self
                .journal
                .as_mut()
                .ok_or_else(|| "account journal was taken by replay".to_owned())?;
            journal
                .fence(OWNER)
                .map_err(|error| format!("account journal fence: {error}"))?
        };
        let term = token.term().get();
        self.journal_token = Some(token);
        Ok(format!("term={term}"))
    }

    fn journal_append(&mut self, event: &Event) -> Result<String, String> {
        let journal = self
            .journal
            .as_mut()
            .ok_or_else(|| "account journal was taken by replay".to_owned())?;
        let token = self
            .journal_token
            .as_ref()
            .ok_or_else(|| "no current account journal fencing token".to_owned())?;
        let payload_hex = hex(&event.payload);
        let body = StoredBody {
            envelope: EventEnvelope {
                event_id: EventId::for_run(RunId::from_bytes(RUN_ID_BYTES), event.sequence + 1),
                schema_version: 1,
                stream_id: STREAM_ID.to_owned(),
                source_id: None,
                source_clock_domain: None,
                session_id: None,
                source_sequence: None,
                correlation_id: None,
                causation_id: None,
                times: event_times(event.sequence),
                quality: BTreeSet::<DataQualityFlag>::new(),
            },
            payload: EventPayload::SessionStateObserved {
                trade_date: trade_date()?,
                phase: SessionPhase::Closed,
                calendar_version: digest_hex(&payload_hex),
            },
        };
        let record = journal
            .append_fenced(token, body)
            .map_err(|error| format!("append_fenced: {error}"))?;
        Ok(format!("{}|{}", record.sequence, record.hash.hex()))
    }

    fn paper_submit(&mut self, event: &Event) -> Result<String, String> {
        let engine = self
            .engine
            .as_mut()
            .ok_or_else(|| "paper engine was taken by replay".to_owned())?;
        let payload_hash = digest_hex(&hex(&event.payload));
        let decision = DecisionRecord {
            strategy_id: StrategyId::new(2)
                .map_err(|error| format!("fixed strategy id: {error}"))?,
            strategy_version: "neutral-bench-v1".to_owned(),
            source_build_hash: payload_hash.clone(),
            configuration_hash: digest_hex("neutral-bench-config-v1"),
            dataset_hash: payload_hash,
            input_refs: Vec::new(),
            features_fixed_point: BTreeMap::new(),
            rationale: "neutral-workload-decision".to_owned(),
            intent: None,
        };
        let outcome = engine
            .submit_decision(decision, event_times(event.sequence))
            .map_err(|error| format!("submit_decision: {error}"))?;
        Ok(format!(
            "{}|{}|{}",
            outcome.risk.is_some(),
            outcome.order_id.is_some(),
            outcome.deduplicated
        ))
    }

    fn paper_reconcile(&mut self, event: &Event) -> Result<String, String> {
        let engine = self
            .engine
            .as_mut()
            .ok_or_else(|| "paper engine was taken by replay".to_owned())?;
        let report = engine
            .reconcile(
                ReconcileInput {
                    positions: None,
                    active_orders: None,
                },
                format!("neutral-reconcile-{}", event.sequence),
                event_times(event.sequence),
            )
            .map_err(|error| format!("reconcile: {error}"))?;
        Ok(format!(
            "{}|{}|{}|{}",
            report.status as u8,
            report.expected_positions.len(),
            report.expected_active_orders.len(),
            report.differences.len()
        ))
    }

    fn oms_state(&self) -> Result<String, String> {
        let store = self
            .oms
            .as_ref()
            .ok_or_else(|| "OMS writer was taken by replay".to_owned())?;
        Ok(store.state_digest().to_owned())
    }

    fn oms_journal_order(&self) -> Result<String, String> {
        let text = fs::read_to_string(&self.oms_path)
            .map_err(|error| format!("reading OMS journal: {error}"))?;
        let mut canonical = String::new();
        for line in text.lines() {
            let value: serde_json::Value =
                serde_json::from_str(line).map_err(|error| format!("OMS journal line: {error}"))?;
            canonical.push_str(&format!(
                "{}|{}|{}\n",
                value
                    .get("seq")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                value
                    .get("op")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
                value
                    .get("digest")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
            ));
        }
        Ok(canonical)
    }

    fn replay(&mut self) -> Result<String, String> {
        self.engine.take();
        self.oms.take();
        self.journal.take();
        let engine = Engine::open(&self.paper_path, engine_config()?, event_times(0))
            .map_err(|error| format!("engine replay: {error}"))?;
        let oms =
            OmsStore::recover(&self.oms_path).map_err(|error| format!("OMS recovery: {error}"))?;
        let journal = Journal::open(&self.account_path, STREAM_ID)
            .map_err(|error| format!("account journal replay: {error}"))?;
        let recovered_oms_state = oms.state_digest().to_owned();
        let mut canonical = format!("oms_state={recovered_oms_state}\n");
        canonical.push_str(&format!(
            "engine_state={}\n",
            digest_hex(&engine_state_text(&engine))
        ));
        canonical.push_str(&format!(
            "account_state={}\n",
            digest_hex(&account_state_text(&journal))
        ));
        self.engine = Some(engine);
        self.oms = Some(oms);
        self.journal = Some(journal);
        self.oms_token = Some(
            self.oms
                .as_mut()
                .ok_or_else(|| "OMS writer missing after recovery".to_owned())?
                .fence(OWNER)
                .map_err(|error| format!("post-recovery OMS fence: {error}"))?,
        );
        self.journal_token = Some(
            self.journal
                .as_mut()
                .ok_or_else(|| "account journal missing after replay".to_owned())?
                .fence(OWNER)
                .map_err(|error| format!("post-replay journal fence: {error}"))?,
        );
        Ok(canonical)
    }

    fn all_event_ops(&mut self, event: &Event) -> Result<String, String> {
        let mut canonical = String::new();
        canonical.push_str(&format!("risk|{}\n", self.risk(event)?));
        canonical.push_str(&format!("oms_fence|{}\n", self.oms_fence()?));
        canonical.push_str(&format!("oms_create|{}\n", self.oms_create(event)?));
        canonical.push_str(&format!("oms_accept|{}\n", self.oms_accept(event)?));
        canonical.push_str(&format!("oms_claim|{}\n", self.oms_claim(event)?));
        canonical.push_str(&format!("oms_dispatch|{}\n", self.oms_dispatch(event)?));
        canonical.push_str(&format!("oms_ack|{}\n", self.oms_acknowledge(event)?));
        canonical.push_str(&format!("oms_fill|{}\n", self.oms_fill(event)?));
        canonical.push_str(&format!("journal_append|{}\n", self.journal_append(event)?));
        canonical.push_str(&format!("paper_submit|{}\n", self.paper_submit(event)?));
        canonical.push_str(&format!(
            "paper_reconcile|{}\n",
            self.paper_reconcile(event)?
        ));
        Ok(canonical)
    }

    /// Unmeasured setup for a stage measured in isolation. Every prerequisite
    /// is a fenced production call; only the selected stage call is timed.
    fn prerequisites(&mut self, stage: Stage, event: &Event) -> Result<String, String> {
        let mut canonical = String::new();
        if matches!(
            stage,
            Stage::OmsCreateFenced
                | Stage::OmsAcceptRiskApprovedFenced
                | Stage::OmsClaimFenced
                | Stage::OmsMarkDispatchingFenced
                | Stage::OmsAcknowledgeFenced
                | Stage::OmsApplyTypedFillFenced
        ) {
            canonical.push_str(&format!("oms_fence|{}\n", self.oms_fence()?));
            canonical.push_str(&format!("oms_create|{}\n", self.oms_create(event)?));
        }
        if matches!(
            stage,
            Stage::OmsClaimFenced
                | Stage::OmsMarkDispatchingFenced
                | Stage::OmsAcknowledgeFenced
                | Stage::OmsApplyTypedFillFenced
        ) {
            canonical.push_str(&format!("oms_accept|{}\n", self.oms_accept(event)?));
        }
        if matches!(
            stage,
            Stage::OmsMarkDispatchingFenced
                | Stage::OmsAcknowledgeFenced
                | Stage::OmsApplyTypedFillFenced
        ) {
            canonical.push_str(&format!("oms_claim|{}\n", self.oms_claim(event)?));
        }
        if matches!(
            stage,
            Stage::OmsAcknowledgeFenced | Stage::OmsApplyTypedFillFenced
        ) {
            canonical.push_str(&format!("oms_dispatch|{}\n", self.oms_dispatch(event)?));
        }
        if matches!(stage, Stage::OmsApplyTypedFillFenced) {
            canonical.push_str(&format!("oms_ack|{}\n", self.oms_acknowledge(event)?));
        }
        Ok(canonical)
    }
}

fn engine_state_text(engine: &Engine) -> String {
    let mut canonical = String::new();
    for record in engine.records() {
        canonical.push_str(&format!(
            "{}|{}|{}|{}\n",
            record.sequence,
            record.body.envelope.event_id,
            record.hash.hex(),
            record.body.payload.kind()
        ));
    }
    canonical.push_str(&format!("{:?}\n", engine.account_view()));
    canonical.push_str(&format!("{:?}\n", engine.verification_report()));
    canonical
}

fn account_state_text(journal: &Journal) -> String {
    let mut canonical = String::new();
    for record in journal.records() {
        canonical.push_str(&format!(
            "{}|{}|{}\n",
            record.sequence,
            record.body.envelope.event_id,
            record.hash.hex()
        ));
    }
    canonical.push_str(&format!("{:?}\n", journal.verification_report()));
    canonical
}

fn tree_digest(path: &Path) -> Result<String, String> {
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    if path.is_dir() {
        let mut pending = vec![(path.to_path_buf(), String::new())];
        while let Some((directory, prefix)) = pending.pop() {
            let mut children = fs::read_dir(&directory)
                .map_err(|error| format!("listing {}: {error}", directory.display()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("directory entry: {error}"))?;
            children.sort_by_key(std::fs::DirEntry::file_name);
            for child in children {
                let name = child.file_name().to_string_lossy().to_string();
                let child_prefix = if prefix.is_empty() {
                    name
                } else {
                    format!("{prefix}/{name}")
                };
                let kind = child
                    .file_type()
                    .map_err(|error| format!("file type: {error}"))?;
                if kind.is_dir() {
                    pending.push((child.path(), child_prefix));
                } else if kind.is_file() {
                    let bytes = fs::read(child.path())
                        .map_err(|error| format!("reading {}: {error}", child.path().display()))?;
                    entries.push((child_prefix, bytes));
                }
            }
        }
    } else if path.is_file() {
        let bytes =
            fs::read(path).map_err(|error| format!("reading {}: {error}", path.display()))?;
        let name = path
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_default();
        entries.push((name, bytes));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut hasher = Sha256::new();
    for (name, bytes) in entries {
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(&bytes);
    }
    Ok(hex(&hasher.finalize()))
}

struct Snapshot {
    event_order: String,
    journal_bytes: String,
    risk: String,
    replay: String,
    reconciliation: String,
}

fn correctness_snapshot(label: &str, events: &[Event]) -> Result<Snapshot, String> {
    let mut workload = Workload::new(label)?;
    let mut risk_canonical = String::new();
    let mut reconciliation_canonical = String::new();
    for event in events {
        risk_canonical.push_str(&format!("{}|{}\n", event.sequence, workload.risk(event)?));
        workload.oms_fence()?;
        workload.oms_create(event)?;
        workload.oms_accept(event)?;
        workload.oms_claim(event)?;
        workload.oms_dispatch(event)?;
        workload.oms_acknowledge(event)?;
        workload.oms_fill(event)?;
        workload.journal_append(event)?;
        workload.paper_submit(event)?;
        reconciliation_canonical.push_str(&format!(
            "{}|{}\n",
            event.sequence,
            workload.paper_reconcile(event)?
        ));
    }
    let mut order = String::new();
    order.push_str(&workload.oms_journal_order()?);
    order.push_str(&account_state_text(
        workload
            .journal
            .as_ref()
            .ok_or_else(|| "account journal missing before replay".to_owned())?,
    ));
    order.push_str(&engine_state_text(
        workload
            .engine
            .as_ref()
            .ok_or_else(|| "paper engine missing before replay".to_owned())?,
    ));
    let mut bytes = String::new();
    bytes.push_str(&format!("oms={}\n", tree_digest(&workload.oms_path)?));
    bytes.push_str(&format!(
        "account={}\n",
        tree_digest(&workload.account_path)?
    ));
    bytes.push_str(&format!("paper={}\n", tree_digest(&workload.paper_path)?));
    bytes.push_str(&format!("oms_state={}\n", workload.oms_state()?));
    let replay = workload.replay()?;
    Ok(Snapshot {
        event_order: digest_hex(&order),
        journal_bytes: digest_hex(&bytes),
        risk: digest_hex(&risk_canonical),
        replay: digest_hex(&replay),
        reconciliation: digest_hex(&reconciliation_canonical),
    })
}

struct CaseResult {
    stage: Stage,
    samples: Vec<u128>,
    events: u64,
    bytes: u64,
    elapsed_ns: u128,
    error_count: u64,
    state: String,
}

fn run_case(stage: Stage, events: &[Event], config: &Config) -> Result<CaseResult, String> {
    {
        let mut warm = Workload::new("warmup")?;
        let warmup = generate_events(config.seed, config.warmup_events, config.payload_bytes);
        for event in &warmup {
            let _observed = warm.all_event_ops(event)?;
        }
    }
    let mut workload = Workload::new(stage.as_str())?;
    let mut samples = Vec::new();
    let mut canonical = String::new();
    let mut processed = 0_u64;
    let mut processed_bytes = 0_u64;
    if stage == Stage::ReplayRecovery {
        for event in events {
            let _prepared = workload.all_event_ops(event)?;
            processed = processed.wrapping_add(1);
            processed_bytes = processed_bytes.wrapping_add(event.payload.len() as u64);
        }
        let item_started = Instant::now();
        canonical.push_str(&workload.replay()?);
        samples.push(item_started.elapsed().as_nanos());
    } else {
        for event in events {
            let prepared = workload.prerequisites(stage, event)?;
            let item_started = Instant::now();
            let observed = match stage {
                Stage::RiskEvaluate => workload.risk(event)?,
                Stage::OmsFence => workload.oms_fence()?,
                Stage::OmsCreateFenced => workload.oms_create(event)?,
                Stage::OmsAcceptRiskApprovedFenced => workload.oms_accept(event)?,
                Stage::OmsClaimFenced => workload.oms_claim(event)?,
                Stage::OmsMarkDispatchingFenced => workload.oms_dispatch(event)?,
                Stage::OmsAcknowledgeFenced => workload.oms_acknowledge(event)?,
                Stage::OmsApplyTypedFillFenced => workload.oms_fill(event)?,
                Stage::JournalFence => workload.journal_fence()?,
                Stage::JournalAppendFenced => workload.journal_append(event)?,
                Stage::PaperSubmitDecision => workload.paper_submit(event)?,
                Stage::PaperReconcile => workload.paper_reconcile(event)?,
                Stage::PipelineTotal => workload.all_event_ops(event)?,
                Stage::ReplayRecovery => String::new(),
            };
            samples.push(item_started.elapsed().as_nanos());
            canonical.push_str(&prepared);
            canonical.push_str(&format!(
                "{}|{}|{}\n",
                event.sequence,
                stage.as_str(),
                observed
            ));
            processed = processed.wrapping_add(1);
            processed_bytes = processed_bytes.wrapping_add(event.payload.len() as u64);
        }
    }
    let elapsed_ns: u128 = samples.iter().copied().sum();
    Ok(CaseResult {
        stage,
        samples,
        events: processed,
        bytes: processed_bytes,
        elapsed_ns,
        error_count: 0,
        state: digest_hex(&canonical),
    })
}

fn percentile(sorted: &[u128], fraction: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let index = ((sorted.len() as f64 - 1.0) * fraction).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

fn latency_json(samples: &[u128]) -> Json {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let mut histogram: BTreeMap<u128, u64> = BTreeMap::new();
    for sample in &sorted {
        *histogram.entry(*sample).or_insert(0) += 1;
    }
    let mut json = Json::new();
    for (label, fraction) in [
        ("p50", 0.50),
        ("p90", 0.90),
        ("p95", 0.95),
        ("p99", 0.99),
        ("p999", 0.999),
    ] {
        let value = percentile(&sorted, fraction);
        json.uint128(label, value);
        let beyond = sorted.iter().filter(|sample| **sample > value).count();
        json.uint(&format!("count_beyond_{label}"), beyond as u64);
    }
    json.uint128("max", sorted.last().copied().unwrap_or(0));
    json.uint("sample_count", sorted.len() as u64);
    let histogram_json: Vec<Json> = histogram
        .iter()
        .map(|(value, count)| {
            let mut item = Json::new();
            item.uint128("latency_ns", *value);
            item.uint("count", *count);
            item
        })
        .collect();
    json.array("raw_histogram_ns", &histogram_json);
    json
}

fn bindings_json(
    config: &Config,
    source_sha: &str,
    binary_digest: &str,
    target: &str,
    workload: &str,
) -> Json {
    let mut json = Json::new();
    json.string("source_sha", source_sha);
    json.string("base_sha", BASE_SHA);
    json.string("binary_digest_sha256", binary_digest);
    json.string("target_triple", target);
    json.string("workload_digest_sha256", workload);
    json.uint128("seed", config.seed as u128);
    json.uint("event_count", config.events);
    json.uint("warmup_events", config.warmup_events);
    json.uint("repetitions", config.repetitions as u64);
    json
}

fn case_json(result: &CaseResult, bindings: &Json) -> Json {
    let elapsed_seconds = result.elapsed_ns as f64 / 1_000_000_000.0;
    let throughput = if elapsed_seconds > 0.0 {
        result.events as f64 / elapsed_seconds
    } else {
        0.0
    };
    let bytes_per_second = if elapsed_seconds > 0.0 {
        result.bytes as f64 / elapsed_seconds
    } else {
        0.0
    };
    let mut json = Json::new();
    json.string("stage", result.stage.as_str());
    json.object("bindings", bindings);
    json.uint("samples", result.samples.len() as u64);
    json.uint("events_processed", result.events);
    json.uint("bytes_processed", result.bytes);
    json.uint128("elapsed_ns", result.elapsed_ns);
    json.float("throughput_events_per_sec", throughput);
    json.float("throughput_bytes_per_sec", bytes_per_second);
    json.object("latency_ns", &latency_json(&result.samples));
    json.uint("error_count", result.error_count);
    json.string("raw_result_digest_sha256", &result.state);
    json
}

fn command_text(program: &str, args: &[&str]) -> String {
    match Command::new(program).args(args).output() {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        }
        _ => "unavailable".to_owned(),
    }
}

fn cpu_text() -> String {
    let brand = command_text("sysctl", &["-n", "machdep.cpu.brand_string"]);
    if brand != "unavailable" {
        return brand;
    }
    match fs::read_to_string("/proc/cpuinfo") {
        Ok(text) => text
            .lines()
            .find(|line| line.starts_with("model name"))
            .unwrap_or("unavailable")
            .to_owned(),
        Err(_) => "unavailable".to_owned(),
    }
}

fn target_triple() -> String {
    let version = command_text("rustc", &["-vV"]);
    for line in version.lines() {
        if let Some(host) = line.strip_prefix("host: ") {
            return host.trim().to_owned();
        }
    }
    format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)
}

fn compiler_flags() -> String {
    let rustflags = env::var("RUSTFLAGS").unwrap_or_default();
    let encoded = env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_default();
    format!(
        "debug_assertions={} RUSTFLAGS={rustflags} CARGO_ENCODED_RUSTFLAGS={encoded}",
        cfg!(debug_assertions)
    )
}

fn workload_canonical(config: &Config, stages: &[Stage]) -> String {
    let mut text = format!(
        "seed={:016x}|events={}|warmup={}|repetitions={}|payload_bytes={}|mode={}\n",
        config.seed,
        config.events,
        config.warmup_events,
        config.repetitions,
        config.payload_bytes,
        if config.self_test {
            "self-test"
        } else {
            "full"
        }
    );
    for stage in stages {
        text.push_str(&format!("{}\n", stage.as_str()));
    }
    text
}

fn parse_u64(value: &str, flag: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("invalid value for {flag}: {value}"))
}

fn parse_usize(value: &str, flag: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|_| format!("invalid value for {flag}: {value}"))
}

fn parse_seed(value: &str) -> Result<u64, String> {
    let (radix, digits) = match value.strip_prefix("0x") {
        Some(rest) => (16_u32, rest),
        None => (10_u32, value),
    };
    u64::from_str_radix(digits, radix).map_err(|_| format!("invalid seed: {value}"))
}

fn parse_args() -> Result<Config, String> {
    let mut config = Config::defaults();
    let args: Vec<String> = env::args().skip(1).collect();
    let mut position = 0_usize;
    while position < args.len() {
        let arg = args[position].clone();
        position += 1;
        if arg == "--self-test" {
            config.self_test = true;
            continue;
        }
        if arg == "--run" {
            config.run = true;
            continue;
        }
        if arg == "--help" {
            config.help = true;
            continue;
        }
        if matches!(
            arg.as_str(),
            "--test" | "--nocapture" | "--exact" | "--list"
        ) {
            continue;
        }
        if arg == "--bench" {
            if position < args.len() && !args[position].starts_with('-') {
                position += 1;
            }
            continue;
        }
        let (key, inline) = match arg.split_once('=') {
            Some((key, value)) => (key.to_owned(), Some(value.to_owned())),
            None => (arg, None),
        };
        let value = match inline {
            Some(value) => value,
            None => {
                let next = args
                    .get(position)
                    .ok_or_else(|| format!("missing value for {key}"))?;
                position += 1;
                next.clone()
            }
        };
        match key.as_str() {
            "--events" => config.events = parse_u64(&value, &key)?,
            "--warmup-events" => config.warmup_events = parse_u64(&value, &key)?,
            "--repetitions" => config.repetitions = parse_usize(&value, &key)?,
            "--payload-bytes" => config.payload_bytes = parse_usize(&value, &key)?,
            "--seed" => config.seed = parse_seed(&value)?,
            "--out" => config.out = Some(PathBuf::from(value)),
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if config.self_test && config.run {
        return Err("--self-test and --run are mutually exclusive".to_owned());
    }
    // Cargo invokes harness=false bench binaries with --bench under
    // `cargo test --all-targets`. Without an explicit --run, stay in the tiny
    // deterministic self-test mode; the baseline is strictly opt-in.
    if !config.run {
        config.self_test = true;
    }
    if config.self_test {
        config.events = 8;
        config.warmup_events = 2;
        config.repetitions = 1;
        config.payload_bytes = 32;
        config.seed = 0x4e69_726e_6179_0000;
    }
    if config.events == 0 {
        return Err("--events must be at least 1".to_owned());
    }
    if config.repetitions == 0 {
        return Err("--repetitions must be at least 1".to_owned());
    }
    if config.payload_bytes == 0 {
        return Err("--payload-bytes must be at least 1".to_owned());
    }
    Ok(config)
}

fn print_help() {
    eprintln!("real-component production benchmark (neutral opaque workload only)");
    eprintln!("  --self-test            tiny deterministic mode (default)");
    eprintln!("  --run                  opt-in baseline matrix");
    eprintln!("  --events N             measured events per stage");
    eprintln!("  --warmup-events N      unmeasured warm-up events");
    eprintln!("  --repetitions N        measured repetitions");
    eprintln!("  --payload-bytes N      opaque payload size");
    eprintln!("  --seed N               decimal or 0x-prefixed seed");
    eprintln!("  --out PATH             JSON artifact destination");
}

fn filesystem_text(path: &Path) -> String {
    let target = path.parent().unwrap_or_else(|| Path::new("."));
    let rendered = target.to_string_lossy();
    command_text("df", &["-Pk", rendered.as_ref()])
}

fn run_main() -> Result<(), String> {
    let config = parse_args()?;
    if config.help {
        print_help();
        return Ok(());
    }
    let stages = Stage::all();
    let stages: Vec<Stage> = stages.into_iter().collect();
    let events = generate_events(config.seed, config.events, config.payload_bytes);
    let workload_digest = digest_hex(&workload_canonical(&config, &stages));

    let before = correctness_snapshot("correctness-pre", &events)?;
    let mut results = Vec::with_capacity(stages.len());
    let mut raw_canonical = String::new();
    let mut total_duration_ns = 0_u128;
    for stage in &stages {
        let mut combined = CaseResult {
            stage: *stage,
            samples: Vec::new(),
            events: 0,
            bytes: 0,
            elapsed_ns: 0,
            error_count: 0,
            state: String::new(),
        };
        for repetition in 0..config.repetitions {
            let result = run_case(*stage, &events, &config)
                .map_err(|error| format!("stage {}: {error}", stage.as_str()))?;
            combined.samples.extend(result.samples.iter().copied());
            combined.events = combined.events.wrapping_add(result.events);
            combined.bytes = combined.bytes.wrapping_add(result.bytes);
            combined.elapsed_ns = combined.elapsed_ns.wrapping_add(result.elapsed_ns);
            combined.error_count = combined.error_count.wrapping_add(result.error_count);
            raw_canonical.push_str(&format!(
                "{}|{}|{}\n",
                stage.as_str(),
                repetition,
                result.state
            ));
        }
        combined.state = digest_hex(&format!("{}|{}", stage.as_str(), combined.state));
        total_duration_ns = total_duration_ns.wrapping_add(combined.elapsed_ns);
        results.push(combined);
    }
    let after = correctness_snapshot("correctness-post", &events)?;
    let checks = [
        (
            "event_order_identical",
            before.event_order == after.event_order,
        ),
        (
            "journal_account_bytes_identical",
            before.journal_bytes == after.journal_bytes,
        ),
        ("risk_decisions_identical", before.risk == after.risk),
        ("replay_state_identical", before.replay == after.replay),
        (
            "reconciliation_state_identical",
            before.reconciliation == after.reconciliation,
        ),
        ("all_stages_present", results.len() == stages.len()),
        (
            "all_samples_present",
            results.iter().all(|result| !result.samples.is_empty()),
        ),
        (
            "no_stage_errors",
            results.iter().all(|result| result.error_count == 0),
        ),
        (
            "quantiles_ordered",
            results.iter().all(|result| {
                let mut sorted = result.samples.clone();
                sorted.sort_unstable();
                percentile(&sorted, 0.50) <= percentile(&sorted, 0.90)
                    && percentile(&sorted, 0.90) <= percentile(&sorted, 0.95)
                    && percentile(&sorted, 0.95) <= percentile(&sorted, 0.99)
                    && percentile(&sorted, 0.99) <= percentile(&sorted, 0.999)
            }),
        ),
    ];
    if checks.iter().any(|(_, passed)| !*passed) {
        return Err(format!(
            "correctness or shape check failed: {}",
            checks
                .iter()
                .filter(|(_, passed)| !*passed)
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    if config.self_test {
        let pipeline = &results[results.len() - 1];
        if pipeline.events != config.events {
            return Err(format!(
                "self-test event count: expected {}, got {}",
                config.events, pipeline.events
            ));
        }
    }
    let raw_digest = digest_hex(&raw_canonical);
    let out = config.out.clone().unwrap_or_else(|| {
        env::temp_dir()
            .join("nirnay-production-components")
            .join(if config.self_test {
                "self-test.json"
            } else {
                "baseline.json"
            })
    });
    if let Some(parent) = out.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("creating {}: {error}", parent.display()))?;
    }
    let source_sha = command_text("git", &["rev-parse", "HEAD"]);
    let binary_path = env::current_exe().map_err(|error| format!("current exe: {error}"))?;
    let binary_bytes = fs::read(&binary_path)
        .map_err(|error| format!("reading {}: {error}", binary_path.display()))?;
    let binary_digest = digest_bytes(&binary_bytes);
    let target = target_triple();
    let bindings = bindings_json(
        &config,
        &source_sha,
        &binary_digest,
        &target,
        &workload_digest,
    );

    let mut artifact = Json::new();
    artifact.uint("schema_version", 1);
    artifact.string("harness", "production_components");
    artifact.string(
        "mode",
        if config.self_test {
            "self-test"
        } else {
            "full"
        },
    );
    artifact.string("label", "neutral-production-baseline-2026-11-27");
    artifact.string(
        "disclaimer",
        "neutral-workload measurements of real Nirnay components; not a production latency/throughput certificate, not nanosecond/HFT, not horizontal scale; no resource maxima are set",
    );
    artifact.string(
        "scope",
        "Real production functions driven by opaque byte payloads and monotonic u64 sequence values; no instruments, quotes, books, prices, trades, invented market observations, provider, broker or live-execution path. Fixed API scaffolding has no measurement meaning.",
    );
    artifact.string(
        "stage_separation",
        "one stage timer wraps one production API call; unmeasured fenced prerequisite calls prepare isolated stages; pipeline_total wraps the whole per-event sequence; replay_recovery has one replay observation per repetition; elapsed_ns is the sum of target-call samples",
    );
    artifact.string(
        "quantile_method",
        "nearest-rank: sort ascending, index = round((n-1)*fraction); count_beyond_X counts strictly greater samples; raw_histogram_ns is lossless",
    );
    artifact.string("source_sha", &source_sha);
    artifact.string("base_sha", BASE_SHA);
    artifact.string("binary_digest_sha256", &binary_digest);
    artifact.string(
        "toolchain",
        &format!(
            "{} | {}",
            command_text("rustc", &["--version"]),
            command_text("cargo", &["--version"])
        ),
    );
    artifact.string(
        "active_toolchain",
        &command_text("rustup", &["show", "active-toolchain"]),
    );
    artifact.string("compiler_flags", &compiler_flags());
    artifact.string(
        "build_profile",
        if cfg!(debug_assertions) {
            "debug/test profile (debug_assertions enabled)"
        } else {
            "bench profile inherits release (lto=thin, codegen-units=1, panic=abort)"
        },
    );
    artifact.string("target_triple", &target);
    artifact.string("os", std::env::consts::OS);
    artifact.string("arch", std::env::consts::ARCH);
    artifact.string("cpu", &cpu_text());
    artifact.string("machine", &command_text("uname", &["-a"]));
    artifact.string("filesystem", &filesystem_text(&out));
    artifact.string("artifact_path", &out.to_string_lossy());
    artifact.string("workload_digest_sha256", &workload_digest);
    artifact.string("raw_result_digest_sha256", &raw_digest);
    artifact.uint128("seed", config.seed as u128);
    artifact.uint("event_count", config.events);
    artifact.uint("warmup_events", config.warmup_events);
    artifact.uint("repetitions", config.repetitions as u64);
    artifact.uint("payload_bytes", config.payload_bytes as u64);
    artifact.uint128("duration_ns", total_duration_ns);
    artifact.uint("stage_count", results.len() as u64);
    let check_json: Vec<Json> = checks
        .iter()
        .map(|(name, passed)| {
            let mut item = Json::new();
            item.string("name", name);
            item.boolean("passed", *passed);
            item
        })
        .collect();
    artifact.array("checks", &check_json);
    artifact.boolean("checks_passed", checks.iter().all(|(_, passed)| *passed));
    let digests = [
        ("event_order", &before.event_order, &after.event_order),
        (
            "journal_account_bytes",
            &before.journal_bytes,
            &after.journal_bytes,
        ),
        ("risk_decisions", &before.risk, &after.risk),
        ("replay_state", &before.replay, &after.replay),
        (
            "reconciliation_state",
            &before.reconciliation,
            &after.reconciliation,
        ),
    ];
    let digest_json: Vec<Json> = digests
        .iter()
        .map(|(name, before_digest, after_digest)| {
            let mut item = Json::new();
            item.string("name", name);
            item.string("before_sha256", before_digest);
            item.string("after_sha256", after_digest);
            item
        })
        .collect();
    artifact.array("correctness_digests", &digest_json);
    artifact.array(
        "measurements",
        &results
            .iter()
            .map(|result| case_json(result, &bindings))
            .collect::<Vec<Json>>(),
    );
    let mut text = artifact.render();
    text.push('\n');
    fs::write(&out, text.as_bytes())
        .map_err(|error| format!("writing {}: {error}", out.display()))?;
    eprintln!(
        "production-components-bench: mode={} stages={} events={} reps={} raw_digest={} artifact={}",
        if config.self_test {
            "self-test"
        } else {
            "full"
        },
        stages.len(),
        config.events,
        config.repetitions,
        raw_digest,
        out.display()
    );
    Ok(())
}

fn main() {
    match run_main() {
        Ok(()) => {}
        Err(message) => {
            eprintln!("production-components-bench: {message}");
            std::process::exit(2);
        }
    }
}
