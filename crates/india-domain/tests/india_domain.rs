#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Acceptance tests for the canonical India domain (task P1-01/P1-02).

use india_domain::{
    CorporateAction, CorporateActionKind, Instrument, InstrumentId, InstrumentType, InstrumentWire,
    Isin, Listing, ListingStatus, ListingTerms, OptionKind, Paise, Segment, SessionDay,
    SettlementRegime, TickAssignment, TickBand, TickSchedule, TradingCalendar, Venue,
    adjusted_price_paise,
};
use jiff::civil::Date;

fn d(year: i32, month: u32, day: u32) -> Date {
    Date::new(year as i16, month as i8, day as i8).unwrap()
}

fn terms() -> Vec<ListingTerms> {
    vec![ListingTerms {
        effective_from: d(2020, 1, 1),
        effective_to: None,
        tick_assignment: TickAssignment {
            tick_paise: 5,
            reference_close_paise: None,
            reference_close_date: None,
            reference_source: None,
            reference_sha256_hex: None,
            effective_month: None,
            schedule_revision: "test-fixture".to_owned(),
            schedule_effective_from: None,
            schedule_sha256_hex: None,
            calendar_revision: None,
            calendar_sha256_hex: None,
        },
        market_lot: 1,
        quantity_increment: 1,
    }]
}

fn equity_listing() -> Listing {
    Listing {
        venue: Venue::Nse,
        segment: Segment::Cash,
        venue_symbol: "RELIANCE".to_owned(),
        token: Some("2885".to_owned()),
        effective_from: d(2020, 1, 1),
        effective_to: None,
        status: ListingStatus::Active,
        settlement_regime: SettlementRegime::Standard,
        z_category: false,
        settlement_group: None,
        terms: terms(),
    }
}

fn reliance() -> Instrument {
    Instrument {
        instrument_id: InstrumentId::new("in.reliance-industries").unwrap(),
        instrument_type: InstrumentType::Equity,
        isin: Some(Isin::new("INE002A01018").unwrap()),
        name: "Reliance Industries".to_owned(),
        underlying_instrument_id: None,
        expiry_date: None,
        strike_price_paise: None,
        option_kind: None,
        listings: vec![equity_listing()],
        corporate_actions: vec![],
    }
}

#[test]
fn instrument_identity_is_never_a_venue_symbol() {
    let instrument = reliance();
    // The internal id is a stable platform identifier.
    assert_eq!(
        instrument.instrument_id.to_string(),
        "in.reliance-industries"
    );
    // The venue symbol lives only inside the scoped listing.
    assert_eq!(instrument.listings[0].venue_symbol, "RELIANCE");
    assert_eq!(instrument.listings[0].venue, Venue::Nse);
    // The same instrument can list on BSE with a different symbol.
    let mut bse = equity_listing();
    bse.venue = Venue::Bse;
    bse.venue_symbol = "500325".to_owned();
    let mut dual = reliance();
    dual.listings.push(bse);
    assert!(dual.lists_on(Venue::Nse, d(2026, 11, 8)));
    assert!(dual.lists_on(Venue::Bse, d(2026, 11, 8)));
    assert!(!reliance().lists_on(Venue::Bse, d(2026, 11, 8)));
}

#[test]
fn isin_validation_enforces_the_iso_pattern() {
    // Reliance Industries: INE002A01018 has a valid ISO 6166 check digit.
    assert!(Isin::new("INE002A01018").is_ok());
    // Same body, wrong check digit: rejected by the Luhn verification.
    assert!(
        Isin::new("INE002A01011").is_err(),
        "check digit mismatch must fail"
    );
    assert!(Isin::new("ine002a01018").is_err(), "ISIN must be uppercase");
    assert!(Isin::new("INE002A0101").is_err(), "must be 12 chars");
    assert!(
        Isin::new("INE002A0101X").is_err(),
        "check digit must be numeric"
    );
}

#[test]
fn instrument_id_rejects_venue_symbols_as_identity() {
    // Uppercase tickers are not valid internal identities.
    assert!(InstrumentId::new("RELIANCE").is_err());
    assert!(InstrumentId::new("").is_err());
    assert!(InstrumentId::new("-leading-dash").is_err());
    assert!(InstrumentId::new("in.reliance").is_ok());
}

#[test]
fn derivatives_require_underlying_expiry_and_options_require_strike() {
    let mut future = reliance();
    future.instrument_type = InstrumentType::Future;
    assert!(
        future.validate().is_err(),
        "future without underlying rejected"
    );
    // The underlying must be a different instrument (no self-reference).
    future.underlying_instrument_id = Some(InstrumentId::new("in.benchmark-index").unwrap());
    assert!(future.validate().is_err(), "future without expiry rejected");
    future.expiry_date = Some(d(2027, 1, 28));
    assert!(future.validate().is_ok());

    let mut self_ref = reliance();
    self_ref.instrument_type = InstrumentType::Future;
    self_ref.underlying_instrument_id = Some(InstrumentId::new("in.reliance-industries").unwrap());
    self_ref.expiry_date = Some(d(2027, 1, 28));
    assert!(
        self_ref.validate().is_err(),
        "a derivative may not reference itself as its underlying"
    );

    let mut option = future.clone();
    option.instrument_type = InstrumentType::Option;
    option.strike_price_paise = Some(Paise(250_000));
    option.option_kind = Some(OptionKind::Call);
    assert!(option.validate().is_ok());
    option.strike_price_paise = None;
    assert!(option.validate().is_err());
}

#[test]
fn non_index_instruments_require_isin() {
    let mut no_isin = reliance();
    no_isin.isin = None;
    assert!(no_isin.validate().is_err());
    let mut index = reliance();
    index.instrument_type = InstrumentType::Index;
    index.isin = None;
    assert!(index.validate().is_ok());
}

#[test]
fn corporate_actions_must_be_chronological_with_positive_ratios() {
    let mut instrument = reliance();
    instrument.corporate_actions = vec![
        CorporateAction {
            action: CorporateActionKind::Split,
            effective_date: d(2024, 9, 5),
            numerator: 1,
            denominator: 2,
            amount_paise: None,
            note: None,
        },
        CorporateAction {
            action: CorporateActionKind::Bonus,
            effective_date: d(2024, 1, 5),
            numerator: 1,
            denominator: 1,
            amount_paise: None,
            note: None,
        },
    ];
    assert!(
        instrument.validate().is_err(),
        "out-of-order actions rejected"
    );
    instrument.corporate_actions.reverse();
    assert!(instrument.validate().is_ok());
    instrument.corporate_actions[1].denominator = 0;
    assert!(
        instrument.validate().is_err(),
        "non-positive ratio rejected"
    );
}

#[test]
fn adjusted_price_applies_later_actions_exactly_in_integers() {
    // 1:2 split effective 2025-01-01: a raw 2024 price halves afterwards.
    let actions = vec![CorporateAction {
        action: CorporateActionKind::Split,
        effective_date: d(2025, 1, 1),
        numerator: 1,
        denominator: 2,
        amount_paise: None,
        note: None,
    }];
    let raw_2024 = Paise(250_000);
    // Adjusting a pre-action price for a later split produces the post-action
    // comparable value.
    assert_eq!(
        adjusted_price_paise(raw_2024, &actions, d(2024, 6, 1)).unwrap(),
        Paise(125_000)
    );
    // A price on/after the action is unchanged.
    assert_eq!(
        adjusted_price_paise(Paise(125_000), &actions, d(2025, 6, 1)).unwrap(),
        Paise(125_000)
    );
}

#[test]
fn calendar_distinguishes_trading_closed_and_finds_next_trading_day() {
    let calendar = TradingCalendar {
        venue: Venue::Nse,
        days: vec![
            calendar_day(d(2026, 11, 6), SessionDay::Trading),
            calendar_day(d(2026, 11, 7), SessionDay::Closed), // Saturday
            calendar_day(d(2026, 11, 8), SessionDay::Closed), // Sunday
            calendar_day(d(2026, 11, 9), SessionDay::Trading),
            calendar_day(d(2026, 11, 10), SessionDay::Special),
        ],
    };
    assert!(calendar.is_trading_day(d(2026, 11, 6)));
    assert!(!calendar.is_trading_day(d(2026, 11, 7)));
    // Special sessions count as trading days.
    assert!(calendar.is_trading_day(d(2026, 11, 10)));
    assert_eq!(
        calendar.next_trading_day(d(2026, 11, 6)),
        Some(d(2026, 11, 9))
    );
    assert_eq!(
        calendar.next_trading_day(d(2026, 11, 9)),
        Some(d(2026, 11, 10))
    );
}

fn calendar_day(date: Date, session: SessionDay) -> india_domain::CalendarDay {
    india_domain::CalendarDay {
        venue: Venue::Nse,
        date,
        session,
        note: None,
    }
}

#[test]
fn round_trips_through_json_matching_the_v2_contract() {
    let instrument = reliance();
    let encoded = serde_json::to_string(&instrument).unwrap();
    // The validated wire path: untrusted JSON -> wire -> canonical domain.
    let wire: InstrumentWire = serde_json::from_str(&encoded).unwrap();
    let decoded = Instrument::try_from(wire).unwrap();
    assert_eq!(decoded, instrument);
    // The serializer also matches the wire contract for supported types.
    let via_wire = Instrument::try_from(serde_json::from_str::<InstrumentWire>(&encoded).unwrap());
    assert_eq!(via_wire.unwrap(), instrument);
    // The v2 contract fields are present.
    let value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    for field in [
        "instrument_id",
        "instrument_type",
        "isin",
        "name",
        "listings",
        "corporate_actions",
    ] {
        assert!(value.get(field).is_some(), "missing contract field {field}");
    }
    let listing = &value["listings"][0];
    assert!(listing.get("venue_symbol").is_some());
    assert!(listing.get("terms").is_some());
}

#[test]
fn v2_wire_rejects_unsupported_types_and_unknown_fields() {
    // Fixtures for futures would not validate as wire types; assert the wire
    // schema surface itself rejects them.
    let text = r#"{"instrument_id":"in.x","instrument_type":"future","isin":"INE002A01018","name":"X","listings":[{"venue":"NSE","segment":"cash","venue_symbol":"X","effective_from":"2026-01-01","status":"active","settlement_regime":"standard","terms":[{"effective_from":"2026-01-01","tick_assignment":{"tick_paise":1,"reference_close_paise":null,"effective_month":null,"schedule_revision":"test-fixture"},"market_lot":1,"quantity_increment":1}]}],"corporate_actions":[]}"#;
    assert!(serde_json::from_str::<InstrumentWire>(text).is_err());
    let text = r#"{"instrument_id":"in.x","instrument_type":"equity","isin":"INE002A01018","name":"X","listings":[{"venue":"NSE","segment":"fno","venue_symbol":"X","effective_from":"2026-01-01","status":"active","settlement_regime":"standard","terms":[{"effective_from":"2026-01-01","tick_assignment":{"tick_paise":1,"reference_close_paise":null,"effective_month":null,"schedule_revision":"test-fixture"},"market_lot":1,"quantity_increment":1}]}],"corporate_actions":[]}"#;
    assert!(serde_json::from_str::<InstrumentWire>(text).is_err());
    let text = r#"{"instrument_id":"in.x","instrument_type":"equity","isin":"INE002A01018","name":"X","egregious":true,"listings":[{"venue":"NSE","segment":"cash","venue_symbol":"X","effective_from":"2026-01-01","status":"active","settlement_regime":"standard","terms":[{"effective_from":"2026-01-01","tick_assignment":{"tick_paise":1,"reference_close_paise":null,"effective_month":null,"schedule_revision":"test-fixture"},"market_lot":1,"quantity_increment":1}]}],"corporate_actions":[]}"#;
    assert!(serde_json::from_str::<InstrumentWire>(text).is_err());
}

#[test]
fn invalid_direct_struct_construction_still_validates() {
    // Even hand-built (non-JSON) domain objects are validated before catalog
    // admission; deserialization is not the only gate.
    let mut bad = reliance();
    bad.listings[0].venue_symbol = String::new();
    assert!(bad.validate().is_err());
    let mut bad = reliance();
    bad.listings[0].terms[0].effective_to = Some(d(2019, 1, 1));
    assert!(bad.validate().is_err());
    let mut bad = reliance();
    bad.listings[0].terms.push(ListingTerms {
        effective_from: d(2021, 1, 1),
        effective_to: None,
        tick_assignment: TickAssignment {
            tick_paise: 5,
            reference_close_paise: None,
            reference_close_date: None,
            reference_source: None,
            reference_sha256_hex: None,
            effective_month: None,
            schedule_revision: "test-fixture".to_owned(),
            schedule_effective_from: None,
            schedule_sha256_hex: None,
            calendar_revision: None,
            calendar_sha256_hex: None,
        },
        market_lot: 1,
        quantity_increment: 1,
    });
    assert!(bad.validate().is_err(), "overlapping terms rejected");
}

#[test]
fn adjustment_with_truncation_is_rejected_not_silently_corrupted() {
    // 1 paise halved is 0.5 paise: not representable. Must error, not
    // truncate to zero.
    let actions = vec![CorporateAction {
        action: CorporateActionKind::Split,
        effective_date: d(2025, 1, 1),
        numerator: 1,
        denominator: 2,
        amount_paise: None,
        note: None,
    }];
    let error = adjusted_price_paise(Paise(1), &actions, d(2024, 6, 1)).unwrap_err();
    assert!(
        format!("{error:#}").contains("not representable"),
        "unexpected error: {error:#}"
    );
    // A zero denominator is rejected as malformed input.
    let malformed = vec![CorporateAction {
        action: CorporateActionKind::Split,
        effective_date: d(2025, 1, 1),
        numerator: 1,
        denominator: 0,
        amount_paise: None,
        note: None,
    }];
    let error = adjusted_price_paise(Paise(100), &malformed, d(2024, 6, 1)).unwrap_err();
    assert!(
        format!("{error:#}").contains("non-positive"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn calendar_next_trading_day_is_bounded_by_known_data() {
    let calendar = TradingCalendar {
        venue: Venue::Nse,
        days: vec![
            calendar_day(d(2026, 11, 6), SessionDay::Trading),
            calendar_day(d(2026, 11, 7), SessionDay::Closed),
        ],
    };
    // The calendar knows nothing after 2026-11-07: must return None, never
    // scan indefinitely.
    assert_eq!(calendar.next_trading_day(d(2026, 11, 6)), None);
    assert_eq!(calendar.next_trading_day(d(2026, 11, 7)), None);
}

#[test]
fn calendar_validation_rejects_duplicates_and_out_of_order() {
    let duplicated = TradingCalendar {
        venue: Venue::Nse,
        days: vec![
            calendar_day(d(2026, 11, 6), SessionDay::Trading),
            calendar_day(d(2026, 11, 6), SessionDay::Closed),
        ],
    };
    assert!(duplicated.validate().is_err());
    let unordered = TradingCalendar {
        venue: Venue::Nse,
        days: vec![
            calendar_day(d(2026, 11, 9), SessionDay::Trading),
            calendar_day(d(2026, 11, 6), SessionDay::Trading),
        ],
    };
    assert!(unordered.validate().is_err());
    let wrong_venue = TradingCalendar {
        venue: Venue::Nse,
        days: vec![india_domain::CalendarDay {
            venue: Venue::Bse,
            date: d(2026, 11, 6),
            session: SessionDay::Trading,
            note: None,
        }],
    };
    assert!(wrong_venue.validate().is_err());
    let good = TradingCalendar {
        venue: Venue::Nse,
        days: vec![
            calendar_day(d(2026, 11, 6), SessionDay::Trading),
            calendar_day(d(2026, 11, 9), SessionDay::Trading),
        ],
    };
    assert!(good.validate().is_ok());
}

#[test]
fn calendar_month_coverage_requires_every_day() {
    // A holiday-only calendar is sparse: absence of a date is not evidence.
    let sparse = TradingCalendar {
        venue: Venue::Nse,
        days: vec![
            calendar_day(d(2026, 5, 1), SessionDay::Closed),
            calendar_day(d(2026, 5, 29), SessionDay::Trading),
        ],
    };
    assert!(!sparse.covers_month(d(2026, 5, 1)));
    assert!(!sparse.covers_month(d(2026, 4, 1)));

    // A complete May with weekends closed covers the month.
    let mut days = Vec::new();
    let mut day = d(2026, 5, 1);
    while day.month() == 5 {
        let session = if matches!(
            day.weekday(),
            jiff::civil::Weekday::Saturday | jiff::civil::Weekday::Sunday
        ) {
            SessionDay::Closed
        } else {
            SessionDay::Trading
        };
        days.push(calendar_day(day, session));
        day = day.checked_add(jiff::Span::new().days(1)).unwrap();
    }
    let complete = TradingCalendar {
        venue: Venue::Nse,
        days,
    };
    assert!(complete.covers_month(d(2026, 5, 1)));
    // It does not cover a month it has no entries for.
    assert!(!complete.covers_month(d(2026, 6, 1)));
}

#[test]
fn instrument_validation_rejects_bad_economics_and_duplicate_listings() {
    let mut instrument = reliance();
    instrument.listings[0].terms[0].market_lot = 0;
    assert!(instrument.validate().is_err(), "zero market lot rejected");
    instrument.listings[0].terms[0].market_lot = 1;
    instrument.listings[0].terms[0].tick_assignment.tick_paise = 0;
    assert!(instrument.validate().is_err(), "zero tick size rejected");
    instrument.listings[0].terms[0].tick_assignment.tick_paise = 5;
    let mut bse = equity_listing();
    bse.venue = Venue::Bse;
    bse.venue_symbol = "RELIANCE".to_owned();
    instrument.listings.push(bse);
    assert!(instrument.validate().is_ok());
    // A duplicate (venue, segment, symbol) listing whose range overlaps is
    // rejected; a disjoint historical interval for the same listing
    // identity is allowed.
    let duplicate = equity_listing();
    instrument.listings.push(duplicate);
    assert!(
        instrument.validate().is_err(),
        "overlapping duplicate listing rejected"
    );
    let mut instrument = reliance();
    let mut later = equity_listing();
    later.effective_from = d(2027, 1, 1);
    instrument.listings[0].effective_to = Some(d(2026, 12, 31));
    instrument.listings.push(later);
    assert!(
        instrument.validate().is_ok(),
        "disjoint historical listing interval accepted"
    );
    // Listing ending before it starts is rejected.
    let mut instrument = reliance();
    instrument.listings[0].effective_to = Some(d(2019, 1, 1));
    assert!(instrument.validate().is_err());
}

#[test]
fn tick_schedule_bands_cover_price_space_contiguously() {
    // NSE cash tick revision of 2025-04-15 (reviewer finding 5): the
    // exchange schedule is banded and effective-dated, never a single
    // guessed constant. Bands below come from the circular table.
    let schedule = TickSchedule {
        bands: vec![
            TickBand {
                lower_paise: 0,
                upper_paise: Some(24_999),
                tick_paise: 1,
            },
            TickBand {
                lower_paise: 25_000,
                upper_paise: Some(100_000),
                tick_paise: 5,
            },
            TickBand {
                lower_paise: 100_001,
                upper_paise: Some(500_000),
                tick_paise: 10,
            },
            TickBand {
                lower_paise: 500_001,
                upper_paise: Some(1_000_000),
                tick_paise: 50,
            },
            TickBand {
                lower_paise: 1_000_001,
                upper_paise: Some(2_000_000),
                tick_paise: 100,
            },
            TickBand {
                lower_paise: 2_000_001,
                upper_paise: None,
                tick_paise: 500,
            },
        ],
    };
    schedule.validate().unwrap();
    assert_eq!(schedule.tick_for(0), Some(1));
    assert_eq!(schedule.tick_for(24_999), Some(1));
    assert_eq!(schedule.tick_for(25_000), Some(5));
    assert_eq!(schedule.tick_for(100_000), Some(5));
    assert_eq!(schedule.tick_for(100_001), Some(10));
    assert_eq!(schedule.tick_for(500_001), Some(50));
    assert_eq!(schedule.tick_for(2_000_001), Some(500));
    assert_eq!(schedule.tick_for(999_999_999), Some(500));
    assert_eq!(schedule.tick_for(-1), None);
    assert!(schedule.is_aligned(25_000));
    assert!(!schedule.is_aligned(25_001));
    assert!(schedule.is_aligned(24_998));
    assert!(
        schedule.is_aligned(24_999),
        "sub-250 band uses 1-paise ticks"
    );
}

#[test]
fn tick_schedule_rejects_gaps_overlaps_and_bad_bounds() {
    let gap = TickSchedule {
        bands: vec![
            TickBand {
                lower_paise: 0,
                upper_paise: Some(99),
                tick_paise: 1,
            },
            TickBand {
                lower_paise: 101,
                upper_paise: None,
                tick_paise: 5,
            },
        ],
    };
    assert!(gap.validate().is_err(), "gap at 100");
    let overlap = TickSchedule {
        bands: vec![
            TickBand {
                lower_paise: 0,
                upper_paise: Some(100),
                tick_paise: 1,
            },
            TickBand {
                lower_paise: 100,
                upper_paise: None,
                tick_paise: 5,
            },
        ],
    };
    assert!(overlap.validate().is_err(), "overlap at 100");
    let bounded_tail = TickSchedule {
        bands: vec![TickBand {
            lower_paise: 0,
            upper_paise: Some(100),
            tick_paise: 1,
        }],
    };
    assert!(bounded_tail.validate().is_err(), "tail must be open-ended");
    let zero_tick = TickSchedule {
        bands: vec![TickBand {
            lower_paise: 0,
            upper_paise: None,
            tick_paise: 0,
        }],
    };
    assert!(zero_tick.validate().is_err(), "zero tick");
}

#[test]
fn wire_roundtrip_preserves_regime_and_schedule() {
    let instrument = reliance();
    let wire = instrument.to_wire().unwrap();
    assert_eq!(
        wire.listings[0].settlement_regime,
        SettlementRegime::Standard
    );
    assert_eq!(wire.listings[0].terms[0].tick_assignment.tick_paise, 5);
    assert!(!wire.listings[0].z_category);
    let back = Instrument::try_from(wire).unwrap();
    assert_eq!(back, instrument);
}

#[test]
fn tick_assignment_monthly_record_validates() {
    // NSE monthly assignment (CMTR67133): the tick for month M is fixed by
    // the M-1 month-end close, not by the live price. Computed through
    // `assign_tick` on the ingested schedule.
    let schedule = TickSchedule {
        bands: vec![
            TickBand {
                lower_paise: 0,
                upper_paise: Some(24_999),
                tick_paise: 1,
            },
            TickBand {
                lower_paise: 25_000,
                upper_paise: Some(100_000),
                tick_paise: 5,
            },
            TickBand {
                lower_paise: 100_001,
                upper_paise: Some(500_000),
                tick_paise: 10,
            },
            TickBand {
                lower_paise: 500_001,
                upper_paise: Some(1_000_000),
                tick_paise: 50,
            },
            TickBand {
                lower_paise: 1_000_001,
                upper_paise: Some(2_000_000),
                tick_paise: 100,
            },
            TickBand {
                lower_paise: 2_000_001,
                upper_paise: None,
                tick_paise: 500,
            },
        ],
    };
    // Reference close 30,000 paise (₹300) -> band 25k-100k -> 5 paise for
    // the whole of next month, even if prices later cross band edges.
    let tick = schedule.assign_tick(30_000).unwrap();
    assert_eq!(tick, 5);
    let hash = "ab".repeat(32);
    let assignment = TickAssignment {
        tick_paise: tick,
        reference_close_paise: Some(30_000),
        reference_close_date: Some(d(2026, 1, 30)),
        reference_source: Some("nse-eod-close".to_owned()),
        reference_sha256_hex: Some(hash.clone()),
        effective_month: Some(d(2026, 2, 1)),
        schedule_revision: "nse-tick-schedule:nse-tick-bands-v1".to_owned(),
        schedule_effective_from: Some(d(2025, 4, 15)),
        schedule_sha256_hex: Some(hash.clone()),
        calendar_revision: Some("nse-cash-calendar:nse-calendar-v1".to_owned()),
        calendar_sha256_hex: Some(hash.clone()),
    };
    assignment.validate().unwrap();

    // Missing lineage halves are invalid.
    let bad = TickAssignment {
        effective_month: None,
        ..assignment.clone()
    };
    assert!(bad.validate().is_err(), "monthly needs an effective month");
    let bad_month = TickAssignment {
        effective_month: Some(d(2026, 2, 2)),
        ..assignment.clone()
    };
    assert!(bad_month.validate().is_err(), "month must start on day 1");
    // The reference close must come from the month before the assignment
    // (NSE determination rule), not an arbitrary date.
    let wrong_month = TickAssignment {
        reference_close_date: Some(d(2026, 2, 10)),
        ..assignment.clone()
    };
    assert!(
        wrong_month.validate().is_err(),
        "a close from the assignment month is not the reference close"
    );
    // A price with no reproducible source cannot fix a tick.
    let no_source = TickAssignment {
        reference_sha256_hex: None,
        ..assignment.clone()
    };
    assert!(no_source.validate().is_err(), "source hash is required");
    let no_schedule_hash = TickAssignment {
        schedule_sha256_hex: None,
        ..assignment.clone()
    };
    assert!(
        no_schedule_hash.validate().is_err(),
        "schedule hash is required"
    );
    let no_calendar_revision = TickAssignment {
        calendar_revision: None,
        ..assignment.clone()
    };
    assert!(
        no_calendar_revision.validate().is_err(),
        "calendar revision is required for a monthly assignment"
    );
    let no_calendar_hash = TickAssignment {
        calendar_sha256_hex: None,
        ..assignment.clone()
    };
    assert!(
        no_calendar_hash.validate().is_err(),
        "calendar hash is required for a monthly assignment"
    );
    // A non-monthly (BSE master) assignment cannot smuggle calendar lineage.
    let stray_calendar = TickAssignment {
        tick_paise: 5,
        reference_close_paise: None,
        reference_close_date: None,
        reference_source: None,
        reference_sha256_hex: None,
        effective_month: None,
        schedule_revision: "bse-security-master:bse-scrip-v1".to_owned(),
        schedule_effective_from: Some(d(2026, 1, 1)),
        schedule_sha256_hex: Some(hash.clone()),
        calendar_revision: Some("nse-cash-calendar:nse-calendar-v1".to_owned()),
        calendar_sha256_hex: Some(hash.clone()),
    };
    assert!(
        stray_calendar.validate().is_err(),
        "non-monthly assignments carry no calendar source"
    );
    // Non-monthly (BSE master tick) is allowed with no reference close.
    TickAssignment {
        tick_paise: 5,
        reference_close_paise: None,
        reference_close_date: None,
        reference_source: None,
        reference_sha256_hex: None,
        effective_month: None,
        schedule_revision: "bse-security-master:bse-scrip-v1".to_owned(),
        schedule_effective_from: Some(d(2026, 1, 1)),
        schedule_sha256_hex: Some(hash),
        calendar_revision: None,
        calendar_sha256_hex: None,
    }
    .validate()
    .unwrap();
}
