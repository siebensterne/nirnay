#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Temporal wire-compatibility goldens for the chrono -> jiff migration.
//!
//! Every literal below was captured from chrono 0.4.45 at baseline. jiff must
//! reproduce it byte for byte, otherwise a persisted format or wire contract
//! has silently changed.

#[cfg(feature = "market-shaped-tests")]
use india_domain::InstrumentWire;
use jiff::{SignedDuration, Timestamp, civil::Date};

fn ts(nanos: i32) -> Timestamp {
    Timestamp::new(1_763_366_400, nanos).unwrap()
}

#[test]
fn date_display_and_serde_match_chrono_golden() {
    for (parts, golden) in [
        ((2026, 11, 17), "2026-11-17"),
        ((2024, 2, 29), "2024-02-29"),
        ((2026, 1, 31), "2026-01-31"),
        ((2000, 2, 29), "2000-02-29"),
        ((1999, 12, 31), "1999-12-31"),
    ] {
        let date = Date::new(parts.0, parts.1, parts.2).unwrap();
        assert_eq!(date.to_string(), golden);
        assert_eq!(
            serde_json::to_string(&date).unwrap(),
            format!("\"{golden}\"")
        );
    }
}

#[test]
fn timestamp_autosi_and_fixed_precision_match_chrono_golden() {
    let cases = [
        (
            0,
            "2025-11-17T08:00:00Z",
            "2025-11-17T08:00:00.000Z",
            "2025-11-17T08:00:00.000000Z",
            "2025-11-17T08:00:00.000000000Z",
        ),
        (
            123_000_000,
            "2025-11-17T08:00:00.123Z",
            "2025-11-17T08:00:00.123Z",
            "2025-11-17T08:00:00.123000Z",
            "2025-11-17T08:00:00.123000000Z",
        ),
        (
            123_456_000,
            "2025-11-17T08:00:00.123456Z",
            "2025-11-17T08:00:00.123Z",
            "2025-11-17T08:00:00.123456Z",
            "2025-11-17T08:00:00.123456000Z",
        ),
        (
            123_456_789,
            "2025-11-17T08:00:00.123456789Z",
            "2025-11-17T08:00:00.123Z",
            "2025-11-17T08:00:00.123456Z",
            "2025-11-17T08:00:00.123456789Z",
        ),
    ];
    for (nanos, auto, millis, micros, nanos_str) in cases {
        let timestamp = ts(nanos);
        // Display, AutoSi and serde are the same canonical string.
        assert_eq!(timestamp.to_string(), auto);
        assert_eq!(
            serde_json::to_string(&timestamp).unwrap(),
            format!("\"{auto}\"")
        );
        // Fixed-precision variants always emit the full digit count, even 0.
        assert_eq!(
            timestamp.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
            millis
        );
        assert_eq!(
            timestamp.strftime("%Y-%m-%dT%H:%M:%S%.6fZ").to_string(),
            micros
        );
        assert_eq!(
            timestamp.strftime("%Y-%m-%dT%H:%M:%S%.9fZ").to_string(),
            nanos_str
        );
    }
}

#[test]
fn date_parsing_utc_conversion_and_offset_handling() {
    let parsed = Date::strptime("%Y-%m-%d", "2026-11-17").unwrap();
    assert_eq!(parsed.to_string(), "2026-11-17");

    let with_offset = "2026-11-17T12:34:56+05:30".parse::<Timestamp>().unwrap();
    assert_eq!(with_offset.to_string(), "2026-11-17T07:04:56Z");
    assert_eq!(
        with_offset.to_string(),
        "2026-11-17T12:34:56+05:30"
            .parse::<Timestamp>()
            .unwrap()
            .to_string()
    );

    let with_nanos = "2026-11-17T12:34:56.123456789Z"
        .parse::<Timestamp>()
        .unwrap();
    assert_eq!(
        i64::try_from(with_nanos.as_nanosecond()).unwrap(),
        1_794_918_896_123_456_789
    );
    assert_eq!(with_nanos.to_string(), "2026-11-17T12:34:56.123456789Z");
}

#[test]
fn month_end_and_leap_year_arithmetic() {
    // jiff clamps day-of-month, matching chrono's checked_add_months.
    let month_end = Date::new(2026, 1, 31)
        .unwrap()
        .checked_add(jiff::Span::new().months(1))
        .unwrap();
    assert_eq!(month_end.to_string(), "2026-02-28");

    let leap = Date::new(2024, 2, 29).unwrap();
    assert_eq!(
        leap.checked_add(jiff::Span::new().years(1))
            .unwrap()
            .to_string(),
        "2025-02-28"
    );
    assert_eq!(Date::new(2000, 2, 29).unwrap().to_string(), "2000-02-29");
    assert!(Date::new(2023, 2, 29).is_err());
    assert!(Date::new(2100, 2, 29).is_err());

    let next_day = Date::new(2026, 12, 31)
        .unwrap()
        .checked_add(jiff::Span::new().days(1))
        .unwrap();
    assert_eq!(next_day.to_string(), "2027-01-01");

    // SignedDuration arithmetic used by permit expiry follows the same clock.
    let expiry =
        "2026-11-15T00:00:00Z".parse::<Timestamp>().unwrap() + SignedDuration::from_secs(86_400);
    assert_eq!(expiry.to_string(), "2026-11-16T00:00:00Z");
}

#[test]
fn timestamp_as_nanosecond_overflow_fails_closed() {
    assert_eq!(
        i64::try_from(ts(0).as_nanosecond()).unwrap(),
        1_763_366_400_000_000_000
    );
    assert!(i64::try_from(Timestamp::MAX.as_nanosecond()).is_err());
    assert!(i64::try_from(Timestamp::MIN.as_nanosecond()).is_err());
}

#[cfg(feature = "market-shaped-tests")]
#[test]
fn replay_and_persisted_metadata_decode_identically() {
    // A real repo fixture: its date fields must decode to the same strings.
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/india/");
    let text = std::fs::read_to_string(format!("{path}nse_cash_equity.json")).unwrap();
    let wire: InstrumentWire = serde_json::from_str(&text).unwrap();
    assert_eq!(wire.listings[0].effective_from.to_string(), "2020-01-01");

    // Persisted metadata encoded by chrono must round-trip byte for byte.
    let baseline = r#"{"trade_date":"2026-11-17","recorded_at":"2025-11-17T08:00:00.123456789Z"}"#;
    let decoded: serde_json::Value = serde_json::from_str(baseline).unwrap();
    let date: Date = serde_json::from_value(decoded["trade_date"].clone()).unwrap();
    let recorded: Timestamp = serde_json::from_value(decoded["recorded_at"].clone()).unwrap();
    assert_eq!(date.to_string(), "2026-11-17");
    assert_eq!(recorded.to_string(), "2025-11-17T08:00:00.123456789Z");
    assert_eq!(serde_json::to_string(&date).unwrap(), "\"2026-11-17\"");
    assert_eq!(
        serde_json::to_string(&recorded).unwrap(),
        "\"2025-11-17T08:00:00.123456789Z\""
    );

    // Re-encoding the same decoded values must reproduce the baseline bytes.
    let reencoded = format!(
        "{{\"trade_date\":{},\"recorded_at\":{}}}",
        serde_json::to_string(&date).unwrap(),
        serde_json::to_string(&recorded).unwrap()
    );
    assert_eq!(reencoded, baseline);
}
