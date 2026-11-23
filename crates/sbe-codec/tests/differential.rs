#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Arbitrary-byte codec tests for the active SBE decoder.
//!
//! D11 retired the market-shaped golden capture and its C++ dump. These tests
//! construct protocol bytes in-process from an empty-book message; no market
//! observation or invented order book is used.

use sbe_codec::{
    MIN_CHUNK_BYTES, //
    SNAPSHOT_TEMPLATE,
    SbeError,
    SbeMessage,
    decode_capture,
    decode_message,
    encode_message,
    format_dump_line,
};

fn protocol_bytes() -> Vec<u8> {
    let message = SbeMessage {
        template_id: SNAPSHOT_TEMPLATE,
        received_at_us: 1,
        exchange_timestamp_us: 1,
        symbol: "protocol-probe".to_owned(),
        bids: Vec::new(),
        asks: Vec::new(),
        price_level: None,
        quantity: None,
        side: None,
    };
    encode_message(&message).expect("encode arbitrary-byte probe")
}

#[test]
fn encode_decode_round_trip_is_exact() {
    let bytes = protocol_bytes();
    let decoded = decode_message(&bytes, 0).expect("decode probe");
    assert_eq!(
        decoded.message.symbol, "protocol-probe",
        "symbol round-trips"
    );
    let again = encode_message(&decoded.message).expect("canonical encode");
    assert_eq!(again, bytes, "canonical re-encoding is byte-identical");
    assert_eq!(
        decoded.bytes_consumed,
        bytes.len(),
        "whole message consumed"
    );
    assert_eq!(
        MIN_CHUNK_BYTES, 1024,
        "the retained harness minimum chunk size is preserved"
    );
}

#[test]
fn dump_line_is_deterministic() {
    let bytes = protocol_bytes();
    let message = decode_message(&bytes, 0).expect("decode probe").message;
    let first = format_dump_line(&message).expect("dump line");
    let second = format_dump_line(&message).expect("dump line");
    assert_eq!(first, second, "dump formatting is deterministic");
    assert!(!first.is_empty(), "dump line is non-empty");
}

#[test]
fn malformed_cases_fail_closed() {
    let bytes = protocol_bytes();

    let mut truncated = bytes.clone();
    truncated.resize(10, 0);
    assert!(
        decode_capture(&truncated, 4096).is_err(),
        "a truncated header is refused"
    );

    let mut bad_template = bytes.clone();
    bad_template[2] = 99;
    assert!(
        decode_capture(&bad_template, 4096).is_err(),
        "an unknown template is refused"
    );

    let mut bad_schema = bytes.clone();
    bad_schema[4] = 99;
    assert!(
        decode_capture(&bad_schema, 4096).is_err(),
        "an unknown schema id is refused"
    );

    let mut bad_version = bytes.clone();
    bad_version[6] = 99;
    assert!(
        decode_capture(&bad_version, 4096).is_err(),
        "an unknown schema version is refused"
    );

    let mut bad_group = bytes.clone();
    bad_group[24 + 1] = 0x7f;
    assert!(
        decode_capture(&bad_group, 4096).is_err(),
        "an invalid group block length is refused"
    );

    assert!(
        decode_capture(&bytes, 512).is_err(),
        "a chunk below the minimum is refused"
    );
}

#[test]
fn torn_message_fails_closed() {
    // A filler message crosses the chunk boundary when followed by another.
    let filler = SbeMessage {
        template_id: SNAPSHOT_TEMPLATE,
        received_at_us: 1,
        exchange_timestamp_us: 1,
        symbol: "x".repeat(4050),
        bids: Vec::new(),
        asks: Vec::new(),
        price_level: None,
        quantity: None,
        side: None,
    };
    let mut torn = encode_message(&filler).expect("4086-byte filler message");
    assert_eq!(torn.len(), 4086, "the filler ends just before the boundary");
    torn.extend_from_slice(&protocol_bytes());
    assert_eq!(
        decode_capture(&torn, 4096),
        Err(SbeError::MessageTear),
        "the tear guard rejects a message crossing the chunk boundary"
    );
}

#[test]
fn invalid_utf8_symbol_fails_closed() {
    let mut bytes = protocol_bytes();
    bytes[68] = 0xff;
    assert_eq!(
        decode_capture(&bytes, 4096),
        Err(SbeError::InvalidSymbol),
        "a non-UTF-8 symbol is refused rather than decoded lossily"
    );
}
