//! Run a deterministic paper session from an immutable PIT dataset.
//!
//! This binary is an application adapter, not a new execution engine. It reads
//! canonical `MarketEvent` rows, converts them to exact `trading-core` quote
//! types, emits one operator-configured strategy intent, and drives the public
//! `PaperEngine` API. The engine owns risk, order lifecycle and the account
//! journal. No broker SDK or live routing is involved.

use std::collections::{
    BTreeMap, //
    BTreeSet,
};
use std::io::{
    BufRead, //
    BufReader,
};
use std::path::{
    Path, //
    PathBuf,
};

use anyhow::{
    Context, //
    bail,
    ensure,
};
use clap::Parser;
use cost_model::{
    ChargeBook, //
    ChargeRate,
};
use paper_engine::{
    Engine, //
    EngineConfig,
    PaperProfile,
    ReconcileInput,
};
use risk_engine::RiskPolicy;
use serde::{
    Deserialize, //
    Serialize,
};
use sha2::{
    Digest, //
    Sha256,
};
use trading_core::{
    AccountId, //
    AlgoId,
    Continuity,
    DecisionRecord,
    EventPayload,
    EventTimes,
    Instrument,
    InstrumentCatalog,
    InstrumentId,
    InstrumentType,
    IntentId,
    Isin,
    JournalRef,
    Listing,
    ListingStatus,
    LogicalNanos,
    OrderIntent,
    Paise,
    PricePaise,
    Product,
    Quantity,
    QuoteSnapshot,
    RunId,
    Segment,
    SessionPhase,
    SettlementRegime,
    Side,
    StrategyId,
    TickAssignment,
    TradeDate,
    UtcNanos,
    Venue,
};

/// Operator-owned, versioned run configuration. Every money value is an
/// integer; no decimal or floating-point conversion occurs in this adapter.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunSpec {
    schema_version: u32,
    stream_id: String,
    run_id: String,
    account_id: u64,
    strategy_id: u64,
    algo_id: u64,
    strategy_version: String,
    starting_cash_paise: i64,
    trade_date: String,
    max_quote_age_ns: u64,
    instrument: InstrumentSpec,
    risk: RiskSpec,
    strategy: StrategySpec,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstrumentSpec {
    instrument_id: String,
    isin: String,
    venue: String,
    venue_symbol: String,
    market_lot: u64,
    tick_paise: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RiskSpec {
    max_order_quantity: u64,
    max_order_notional_paise: i64,
    max_position_quantity: u64,
    max_gross_exposure_paise: i64,
    max_daily_loss_paise: i64,
    max_open_orders: u32,
    max_orders_per_second: u32,
    max_price_deviation_ppm: u32,
    allow_short: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StrategySpec {
    max_entry_price_paise: i64,
    order_quantity: u64,
}

/// Only the fields needed for quote conversion are parsed. Unknown fields in
/// a dataset row are accepted because the dataset schema is owned by Python.
#[derive(Debug, Deserialize)]
struct DatasetEvent {
    event_id: String,
    instrument_id: String,
    venue: String,
    event_kind: String,
    received_utc_ns: u64,
    source_timestamp_ns: Option<u64>,
    local_sequence: Option<u64>,
    bids: Vec<Level>,
    asks: Vec<Level>,
}

#[derive(Debug, Deserialize)]
struct Level {
    price: String,
    quantity: i64,
}

#[derive(Debug, Serialize)]
struct RunReport {
    schema_version: u32,
    stream_id: String,
    dataset_sha256: String,
    input_rows: u64,
    quotes_accepted: u64,
    /// Whether the decision was recorded in the paper journal. This does not
    /// assert risk acceptance, order creation, dispatch or a fill.
    decision_recorded: bool,
    decision_deduplicated: bool,
    journal_records: usize,
    journal_sha256: String,
    evidence_sequence: u64,
    evidence_hash: String,
    verification: String,
    paper_fill_model_notice: &'static str,
}

#[derive(Debug, Parser)]
#[command(
    name = "paper-engine-run",
    about = "Run a deterministic paper session from an immutable PIT dataset"
)]
struct Args {
    /// Directory containing an immutable dataset (manifest.json, events.jsonl).
    #[arg(long)]
    dataset: PathBuf,
    /// Operator-owned run specification.
    #[arg(long)]
    spec: PathBuf,
    /// Destination directory for the account journal and run report.
    #[arg(long)]
    output: PathBuf,
    /// Reopen an existing journal and verify idempotent replay.
    #[arg(long)]
    replay: bool,
}

fn quantity(value: u64) -> anyhow::Result<Quantity> {
    let signed = i64::try_from(value).context("quantity outside i64 range")?;
    Quantity::new(signed).context("invalid quantity")
}

fn decimal_to_paise(text: &str, field: &str) -> anyhow::Result<Paise> {
    let value = market_types::fixed::parse_scaled(text, 2)
        .with_context(|| format!("{field} is not an exact paise decimal"))?;
    ensure!(value > 0, "{field} must be positive");
    Ok(Paise(value))
}

fn parse_venue(text: &str) -> anyhow::Result<Venue> {
    match text {
        "NSE" => Ok(Venue::Nse),
        "BSE" => Ok(Venue::Bse),
        other => bail!("unsupported venue `{other}`; expected NSE or BSE"),
    }
}

fn parse_date(text: &str) -> anyhow::Result<TradeDate> {
    let parts: Vec<&str> = text.split('-').collect();
    ensure!(parts.len() == 3, "trade_date must be YYYY-MM-DD");
    let year = parts[0].parse::<i16>().context("invalid trade_date year")?;
    let month = parts[1].parse::<i8>().context("invalid trade_date month")?;
    let day = parts[2].parse::<i8>().context("invalid trade_date day")?;
    TradeDate::new(year, month, day)
        .ok()
        .context("invalid trade_date calendar value")
}

fn level_quantity(level: Option<&Level>, field: &str) -> anyhow::Result<Quantity> {
    let raw = level
        .with_context(|| format!("{field} is missing"))?
        .quantity;
    ensure!(raw > 0, "{field} must be positive");
    Quantity::new(raw).context("invalid quantity")
}

fn level_price(level: Option<&Level>, field: &str) -> anyhow::Result<PricePaise> {
    let text = &level.with_context(|| format!("{field} is missing"))?.price;
    let paise = decimal_to_paise(text, field)?;
    PricePaise::new(paise).context("invalid price")
}

fn times_for(event: &DatasetEvent) -> anyhow::Result<EventTimes> {
    let received = i64::try_from(event.received_utc_ns).context("receive time outside i64")?;
    ensure!(received > 0, "receive time must be positive");
    let source = match event.source_timestamp_ns {
        Some(value) => {
            let nanos = i64::try_from(value).context("source timestamp outside i64")?;
            Some(UtcNanos::new(nanos))
        }
        None => None,
    };
    Ok(EventTimes {
        source_time_utc: source,
        received_time_utc: UtcNanos::new(received),
        available_time: LogicalNanos::new(event.received_utc_ns),
        recorded_time_utc: UtcNanos::new(received),
    })
}

fn event_sequence(event: &DatasetEvent) -> anyhow::Result<u64> {
    if let Some(sequence) = event.local_sequence {
        ensure!(sequence > 0, "local_sequence must be positive");
        return Ok(sequence);
    }
    let digest = Sha256::digest(event.event_id.as_bytes());
    let bytes: [u8; 8] = digest[..8]
        .try_into()
        .context("short deterministic sequence digest")?;
    Ok(u64::from_le_bytes(bytes) | 1)
}

fn quote_for(
    event: &DatasetEvent,
    instrument_id: &InstrumentId,
    venue: Venue,
) -> anyhow::Result<QuoteSnapshot> {
    let bid_price = if event.bids.is_empty() {
        None
    } else {
        Some(level_price(event.bids.first(), "bids[0].price")?)
    };
    let ask_price = if event.asks.is_empty() {
        None
    } else {
        Some(level_price(event.asks.first(), "asks[0].price")?)
    };
    let bid_quantity = if bid_price.is_some() {
        level_quantity(event.bids.first(), "bids[0].quantity")?
    } else {
        Quantity::new(0)?
    };
    let ask_quantity = if ask_price.is_some() {
        level_quantity(event.asks.first(), "asks[0].quantity")?
    } else {
        Quantity::new(0)?
    };
    let quote = QuoteSnapshot {
        instrument_id: instrument_id.clone(),
        venue,
        bid_price,
        bid_quantity,
        ask_price,
        ask_quantity,
        continuity: Continuity::Snapshot,
    };
    quote
        .validate()
        .context("dataset quote failed domain validation")?;
    Ok(quote)
}

fn engine_config(spec: &RunSpec, instrument: Instrument) -> anyhow::Result<EngineConfig> {
    let account = AccountId::new(spec.account_id).context("invalid account_id")?;
    let strategy = StrategyId::new(spec.strategy_id).context("invalid strategy_id")?;
    let algo = AlgoId::new(spec.algo_id).context("invalid algo_id")?;
    let trade_date = parse_date(&spec.trade_date)?;
    let instrument_id = InstrumentId::new(&spec.instrument.instrument_id)
        .context("invalid instrument_id in spec")?;
    let risk = RiskPolicy {
        version: "dataset-paper-v1".to_owned(),
        allowed_accounts: BTreeSet::from([account]),
        allowed_strategies: BTreeSet::from([strategy]),
        allowed_algos: BTreeSet::from([algo]),
        allowed_instruments: BTreeSet::from([instrument_id]),
        max_order_quantity: quantity(spec.risk.max_order_quantity)
            .context("invalid risk max_order_quantity")?,
        max_order_notional_paise: Paise(spec.risk.max_order_notional_paise),
        max_position_quantity: quantity(spec.risk.max_position_quantity)
            .context("invalid risk max_position_quantity")?,
        max_gross_exposure_paise: Paise(spec.risk.max_gross_exposure_paise),
        max_daily_loss_paise: Paise(spec.risk.max_daily_loss_paise),
        max_open_orders: spec.risk.max_open_orders,
        max_orders_per_second: spec.risk.max_orders_per_second,
        max_price_deviation_ppm: spec.risk.max_price_deviation_ppm,
        max_quote_age_ns: spec.max_quote_age_ns,
        allow_short: spec.risk.allow_short,
        max_strategy_exposure_paise: Paise(spec.risk.max_gross_exposure_paise),
        max_sector_exposure_paise: None,
        stress_scenario: None,
        max_stress_loss_paise: None,
        max_participation_ppm: None,
        greek_limits: None,
        max_borrow_quantity: None,
        max_financing_cost_paise: None,
    };
    let rates = [Product::Delivery, Product::Intraday]
        .into_iter()
        .flat_map(|product| [Side::Buy, Side::Sell].map(|side| (product, side)))
        .map(|(product, side)| ChargeRate {
            effective_from: trade_date,
            product,
            side,
            brokerage_ppb: 0,
            stt_ppb: 0,
            exchange_ppb: 0,
            sebi_ppb: 0,
            stamp_ppb: 0,
            gst_ppb: 0,
        })
        .collect::<Vec<_>>();
    Ok(EngineConfig {
        stream_id: spec.stream_id.clone(),
        run_id: RunId::parse(&spec.run_id).context("invalid run_id")?,
        account_id: account,
        starting_cash: Paise(spec.starting_cash_paise),
        trade_date,
        instruments: InstrumentCatalog::new(vec![instrument])
            .context("invalid instrument catalog")?,
        risk_policy: risk,
        charge_book: ChargeBook::new(rates).context("invalid charge book")?,
        paper_profile: PaperProfile {
            slippage_ticks: 0,
            consume_displayed_liquidity_only: true,
        },
    })
}

fn catalog_instrument(spec: &RunSpec) -> anyhow::Result<Instrument> {
    let trade_date = parse_date(&spec.trade_date)?;
    let id = InstrumentId::new(&spec.instrument.instrument_id)
        .context("invalid instrument_id in spec")?;
    let tick = TickAssignment {
        tick_paise: spec.instrument.tick_paise,
        reference_close_paise: None,
        reference_close_date: None,
        reference_source: None,
        reference_sha256_hex: None,
        effective_month: None,
        schedule_revision: "dataset-paper-v1".to_owned(),
        schedule_effective_from: None,
        schedule_sha256_hex: None,
        calendar_revision: None,
        calendar_sha256_hex: None,
    };
    Ok(Instrument {
        instrument_id: id,
        instrument_type: InstrumentType::Equity,
        isin: Some(Isin::new(&spec.instrument.isin).context("invalid ISIN in spec")?),
        name: spec.instrument.venue_symbol.clone(),
        underlying_instrument_id: None,
        expiry_date: None,
        strike_price_paise: None,
        option_kind: None,
        listings: vec![Listing {
            venue: parse_venue(&spec.instrument.venue)?,
            segment: Segment::Cash,
            venue_symbol: spec.instrument.venue_symbol.clone(),
            token: None,
            effective_from: trade_date,
            effective_to: None,
            status: ListingStatus::Active,
            settlement_regime: SettlementRegime::Standard,
            z_category: false,
            settlement_group: None,
            terms: vec![trading_core::ListingTerms {
                effective_from: trade_date,
                effective_to: None,
                tick_assignment: tick,
                market_lot: i64::try_from(spec.instrument.market_lot)
                    .context("market_lot outside i64 range")?,
                quantity_increment: i64::try_from(spec.instrument.market_lot)
                    .context("market_lot outside i64 range")?,
            }],
        }],
        corporate_actions: Vec::new(),
    })
}

fn dataset_digest(path: &Path) -> anyhow::Result<String> {
    let bytes = std::fs::read(path).context("reading dataset events.jsonl")?;
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

fn journal_digest(path: &Path) -> anyhow::Result<String> {
    let mut hasher = Sha256::new();
    let mut paths = vec![path.to_path_buf()];
    while let Some(current) = paths.pop() {
        let mut entries = std::fs::read_dir(&current)
            .with_context(|| format!("reading account journal directory {}", current.display()))?
            .collect::<Result<Vec<_>, _>>()
            .context("listing account journal files")?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let child = entry.path();
            let kind = entry.file_type().context("reading journal entry type")?;
            if kind.is_dir() {
                paths.push(child);
            } else if kind.is_file() {
                hasher.update(child.to_string_lossy().as_bytes());
                hasher.update([0]);
                let bytes = std::fs::read(&child).context("reading account journal file")?;
                hasher.update((bytes.len() as u64).to_be_bytes());
                hasher.update(&bytes);
            }
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Domain separator for the decision build-identity digest.
const BUILD_IDENTITY_DOMAIN: &[u8] = b"paper-run-decision-build-identity-v1\0";

/// Reject an unbound, malformed or zero build identity before a decision is
/// constructed. A decision may only carry a digest derived from the executing
/// artifact and the run inputs.
fn validate_build_identity(hex: &str) -> anyhow::Result<()> {
    ensure!(
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            && hex.bytes().any(|byte| byte != b'0'),
        "decision build identity must be a non-zero 64-character lowercase hex digest"
    );
    Ok(())
}

/// Compute a deterministic build identity from explicit binary bytes.
///
/// This binds a decision to the binary artifact actually executed, together
/// with the operator spec bytes and the strategy version/configuration
/// digest. It is not a claim about source-code truth or rights over the
/// inputs; it identifies what this process ran.
fn build_identity_digest(
    executable_bytes: &[u8],
    spec_bytes: &[u8],
    strategy_version: &str,
    configuration_hash: &str,
) -> anyhow::Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(BUILD_IDENTITY_DOMAIN);
    hasher.update((executable_bytes.len() as u64).to_be_bytes());
    hasher.update(executable_bytes);
    hasher.update((spec_bytes.len() as u64).to_be_bytes());
    hasher.update(spec_bytes);
    hasher.update(strategy_version.as_bytes());
    hasher.update([0]);
    hasher.update(configuration_hash.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    validate_build_identity(&digest)?;
    Ok(digest)
}

/// Bind the decision to the exact binary artifact running this session.
/// Replaying the same binary with the same spec reproduces the same digest.
fn source_build_hash(
    spec_bytes: &[u8],
    strategy_version: &str,
    configuration_hash: &str,
) -> anyhow::Result<String> {
    let executable = std::env::current_exe().context("locating current executable")?;
    let executable_bytes = std::fs::read(&executable)
        .with_context(|| format!("reading executing binary {}", executable.display()))?;
    build_identity_digest(
        &executable_bytes,
        spec_bytes,
        strategy_version,
        configuration_hash,
    )
}

fn run(args: Args) -> anyhow::Result<()> {
    ensure!(args.spec.is_file(), "run spec does not exist");
    let events_path = args.dataset.join("events.jsonl");
    ensure!(events_path.is_file(), "dataset events.jsonl does not exist");
    let spec_bytes = std::fs::read(&args.spec).context("reading run spec")?;
    let spec: RunSpec = serde_json::from_slice(&spec_bytes).context("invalid run spec")?;
    ensure!(
        spec.schema_version == 1,
        "unsupported run spec schema_version"
    );
    ensure!(
        spec.strategy.order_quantity > 0
            && spec
                .strategy
                .order_quantity
                .is_multiple_of(spec.instrument.market_lot),
        "order_quantity must be a positive multiple of market_lot"
    );

    if !args.replay {
        std::fs::create_dir(&args.output).context("creating run output directory")?;
    }
    let journal_path = args.output.join("journal.jsonl");
    ensure!(
        args.replay || !journal_path.exists(),
        "run output identity already exists; use --replay"
    );

    let instrument_id = InstrumentId::new(&spec.instrument.instrument_id)
        .context("invalid instrument_id in spec")?;
    let venue = parse_venue(&spec.instrument.venue)?;
    let instrument = catalog_instrument(&spec)?;
    let config = engine_config(&spec, instrument)?;
    let startup = EventTimes {
        source_time_utc: None,
        received_time_utc: UtcNanos::new(1),
        available_time: LogicalNanos::new(1),
        recorded_time_utc: UtcNanos::new(1),
    };
    let report_path = args.output.join("run-report.json");
    let replay_evidence: Option<JournalRef> = if args.replay && report_path.exists() {
        let bytes = std::fs::read(&report_path).context("reading prior run report")?;
        let prior: serde_json::Value =
            serde_json::from_slice(&bytes).context("invalid prior run report")?;
        let sequence = prior["evidence_sequence"]
            .as_u64()
            .context("prior report lacks evidence sequence")?;
        let hash_hex = prior["evidence_hash"]
            .as_str()
            .context("prior report lacks evidence hash")?
            .to_owned();
        Some(JournalRef { sequence, hash_hex })
    } else {
        None
    };
    let mut engine =
        Engine::open(&journal_path, config, startup).context("opening paper engine")?;
    let start_times = EventTimes {
        source_time_utc: None,
        received_time_utc: UtcNanos::new(2),
        available_time: LogicalNanos::new(2),
        recorded_time_utc: UtcNanos::new(2),
    };
    if engine.account_view().is_none() {
        engine
            .set_session_phase(
                SessionPhase::Continuous,
                "dataset-paper-v1",
                start_times.clone(),
            )
            .context("setting continuous session")?;
        engine
            .reconcile(
                ReconcileInput {
                    positions: Some(BTreeMap::new()),
                    active_orders: Some(BTreeSet::new()),
                },
                "dataset-paper-startup",
                start_times.clone(),
            )
            .context("reconciling starting account state")?;
    }

    let file = std::fs::File::open(&events_path).context("opening dataset events")?;
    let mut input_rows = 0_u64;
    let mut last_received_ns = 0_u64;
    let mut quotes_accepted = 0_u64;
    let mut trigger: Option<(DatasetEvent, OrderIntent)> = None;
    let mut evidence: Vec<JournalRef> = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line.with_context(|| format!("reading dataset row {}", index + 1))?;
        if line.trim().is_empty() {
            bail!("dataset contains a blank row at {}", index + 1);
        }
        let event: DatasetEvent = serde_json::from_str(&line)
            .with_context(|| format!("invalid dataset row {}", index + 1))?;
        ensure!(
            event.instrument_id == spec.instrument.instrument_id,
            "dataset row {} is for an instrument not in the run spec",
            index + 1
        );
        ensure!(
            event.venue == spec.instrument.venue,
            "dataset row {} is for a venue not in the run spec",
            index + 1
        );
        ensure!(
            matches!(
                event.event_kind.as_str(),
                "quote" | "depth" | "last_trade_observation"
            ),
            "dataset row {} has unsupported event_kind {}",
            index + 1,
            event.event_kind
        );
        last_received_ns = last_received_ns.max(event.received_utc_ns);
        let times = times_for(&event)?;
        let quote = quote_for(&event, &instrument_id, venue)
            .with_context(|| format!("invalid quote at dataset row {}", index + 1))?;
        let outcome = engine
            .on_quote(
                format!("dataset:{}", event.venue),
                event_sequence(&event)?,
                quote.clone(),
                times.clone(),
            )
            .with_context(|| format!("quote rejected at dataset row {}", index + 1))?;
        if outcome != paper_engine::QuoteOutcome::Late {
            quotes_accepted += 1;
            if let Some(record) = engine.records().last() {
                evidence = vec![JournalRef {
                    sequence: record.sequence,
                    hash_hex: record.hash.hex(),
                }];
            }
        }
        if trigger.is_none()
            && let Some(ask) = quote.ask_price
            && ask.get().0 <= spec.strategy.max_entry_price_paise
        {
            {
                let digest = Sha256::digest(format!("{}:{}", spec.run_id, event.event_id));
                let bytes: [u8; 8] = digest[..8].try_into().context("short intent digest")?;
                let intent = OrderIntent {
                    intent_id: IntentId::new(u64::from_le_bytes(bytes) | 1)
                        .context("invalid deterministic intent id")?,
                    account_id: AccountId::new(spec.account_id)?,
                    strategy_id: StrategyId::new(spec.strategy_id)?,
                    strategy_version: spec.strategy_version.clone(),
                    algo_id: AlgoId::new(spec.algo_id)?,
                    instrument_id: instrument_id.clone(),
                    venue,
                    side: Side::Buy,
                    product: Product::Delivery,
                    quantity: quantity(spec.strategy.order_quantity)?,
                    limit_price: ask,
                    reason: "dataset_paper_first_eligible_quote".to_owned(),
                    evidence: evidence.clone(),
                    expires_at: LogicalNanos::new(u64::MAX),
                };
                trigger = Some((event, intent));
            }
        }
        input_rows += 1;
    }
    ensure!(input_rows > 0, "dataset is empty");
    ensure!(quotes_accepted > 0, "no quotes were accepted by the engine");

    let mut decision_recorded = false;
    let mut decision_deduplicated = false;
    if let Some((event, intent)) = trigger {
        let configuration_hash = format!("{:x}", Sha256::digest(&spec_bytes));
        let source_build_hash =
            source_build_hash(&spec_bytes, &spec.strategy_version, &configuration_hash)?;
        let decision = DecisionRecord {
            strategy_id: intent.strategy_id,
            strategy_version: intent.strategy_version.clone(),
            source_build_hash,
            configuration_hash,
            dataset_hash: dataset_digest(&events_path)?,
            input_refs: match &replay_evidence {
                Some(reference) => vec![reference.clone()],
                None => intent.evidence.clone(),
            },
            features_fixed_point: BTreeMap::new(),
            rationale: format!("first eligible dataset quote {}", event.event_id),
            intent: Some(intent),
        };
        let intent_id = decision
            .intent
            .as_ref()
            .map(|intent| intent.intent_id)
            .context("decision has no intent")?;
        let already_recorded = engine.records().iter().any(|record| {
            matches!(
                &record.body.payload,
                EventPayload::DecisionMade { decision }
                    if decision.intent.as_ref().is_some_and(|intent| intent.intent_id == intent_id)
            )
        });
        if already_recorded {
            decision_deduplicated = true;
        } else {
            let submit_ns =
                i64::try_from(last_received_ns + 1).context("submit time outside i64 range")?;
            let submit_times = EventTimes {
                source_time_utc: None,
                received_time_utc: UtcNanos::new(submit_ns),
                available_time: LogicalNanos::new(last_received_ns + 1),
                recorded_time_utc: UtcNanos::new(submit_ns),
            };
            let outcome = engine
                .submit_decision(decision, submit_times)
                .context("submitting strategy decision")?;
            decision_recorded = true;
            decision_deduplicated = outcome.deduplicated;
        }
    }
    if !args.replay {
        engine
            .reconcile(
                ReconcileInput {
                    positions: None,
                    active_orders: None,
                },
                "dataset-paper-final",
                EventTimes {
                    source_time_utc: None,
                    received_time_utc: UtcNanos::new(i64::MAX - 1),
                    available_time: LogicalNanos::new(u64::MAX - 1),
                    recorded_time_utc: UtcNanos::new(i64::MAX - 1),
                },
            )
            .context("final account reconciliation")?;
    }

    let evidence_ref = replay_evidence.clone().unwrap_or_else(|| {
        evidence.first().cloned().unwrap_or(JournalRef {
            sequence: 0,
            hash_hex: "0".repeat(64),
        })
    });
    let report = RunReport {
        schema_version: 1,
        stream_id: spec.stream_id.clone(),
        dataset_sha256: dataset_digest(&events_path)?,
        input_rows,
        quotes_accepted,
        decision_recorded,
        decision_deduplicated,
        journal_records: engine.records().len(),
        journal_sha256: journal_digest(&journal_path)?,
        evidence_sequence: evidence_ref.sequence,
        evidence_hash: evidence_ref.hash_hex.clone(),
        verification: format!("{:?}", engine.verification_report()),
        paper_fill_model_notice: "displayed-liquidity paper fill model; not an exchange matching engine or historical execution simulation",
    };
    if report_path.exists() {
        let prior_bytes = std::fs::read(&report_path).context("reading existing run report")?;
        let prior: serde_json::Value =
            serde_json::from_slice(&prior_bytes).context("invalid existing run report")?;
        for (field, expected) in [
            ("dataset_sha256", serde_json::json!(report.dataset_sha256)),
            ("journal_records", serde_json::json!(report.journal_records)),
            ("journal_sha256", serde_json::json!(report.journal_sha256)),
            ("verification", serde_json::json!(report.verification)),
        ] {
            if prior[field] != expected {
                bail!(
                    "replay changed {field}: prior={} current={}",
                    prior[field],
                    expected
                );
            }
        }
    }
    let bytes = serde_json::to_vec_pretty(&report).context("serializing run report")?;
    std::fs::write(&report_path, bytes).context("writing run report")?;
    eprintln!(
        "paper_run rows={rows} quotes={quotes} decision={decision} dedup={dedup} records={records}",
        rows = report.input_rows,
        quotes = report.quotes_accepted,
        decision = report.decision_recorded,
        dedup = report.decision_deduplicated,
        records = report.journal_records
    );
    Ok(())
}

fn main() -> anyhow::Result<()> {
    run(Args::parse())
}

#[cfg(test)]
mod tests {
    use super::{
        build_identity_digest, //
        validate_build_identity,
    };

    const EXECUTABLE_BYTES: &[u8] = b"synthetic-executable-artifact-bytes";
    const CONFIGURATION_HASH: &str =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn build_identity_is_nonzero_lowercase_hex_and_input_bound() {
        let first = build_identity_digest(
            EXECUTABLE_BYTES,
            b"spec-alpha",
            "strategy-v1",
            CONFIGURATION_HASH,
        )
        .unwrap_or_default();
        assert_eq!(first.len(), 64, "digest must be 64 characters");
        assert!(
            first
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "digest must be lowercase hex"
        );
        assert!(
            first.bytes().any(|byte| byte != b'0'),
            "digest must be non-zero"
        );

        let repeated = build_identity_digest(
            EXECUTABLE_BYTES,
            b"spec-alpha",
            "strategy-v1",
            CONFIGURATION_HASH,
        )
        .unwrap_or_default();
        assert_eq!(
            first, repeated,
            "same artifact and spec must reproduce the digest"
        );

        let changed_spec = build_identity_digest(
            EXECUTABLE_BYTES,
            b"spec-beta",
            "strategy-v1",
            CONFIGURATION_HASH,
        )
        .unwrap_or_default();
        assert_ne!(first, changed_spec, "spec bytes must change the digest");

        let changed_binary = build_identity_digest(
            b"other-executable-artifact-bytes",
            b"spec-alpha",
            "strategy-v1",
            CONFIGURATION_HASH,
        )
        .unwrap_or_default();
        assert_ne!(first, changed_binary, "binary bytes must change the digest");
    }

    #[test]
    fn build_identity_rejects_zero_and_unbound_values() {
        let rejected = [
            "0".repeat(64),
            "g".repeat(64),
            "A".repeat(64),
            "abc".to_owned(),
        ];
        for value in rejected {
            assert!(
                validate_build_identity(&value).is_err(),
                "unbound value {value} must be rejected"
            );
        }
        assert!(
            validate_build_identity(CONFIGURATION_HASH).is_ok(),
            "a well-formed digest must be accepted"
        );
    }
}
