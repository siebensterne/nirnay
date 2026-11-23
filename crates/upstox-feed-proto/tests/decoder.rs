#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! M4.1 decoder tests: synthetic messages plus the two captured frames.

#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use prost::Message;
    use upstox_feed_proto::decode::{
        FeedKind, FeedResponseType, decode_feed_response, decode_feed_response_diagnostic,
        response_type_from_raw,
    };
    use upstox_feed_proto::proto::{
        Feed, FeedResponse, FirstLevelWithGreeks, FullFeed, IndexFullFeed, Ltpc, MarketFullFeed,
        MarketInfo, MarketLevel, MarketOhlc, Ohlc, OptionGreeks, Quote, StatusInfo,
        feed::FeedUnion, full_feed::FullFeedUnion,
    };

    fn encode(response: &FeedResponse) -> Vec<u8> {
        response.encode_to_vec()
    }

    fn ltpc(iep: Option<f64>) -> Ltpc {
        Ltpc {
            ltp: 80.39,
            ltt: 1794219944214,
            ltq: 100,
            cp: 81.04,
            iep,
        }
    }

    #[test]
    fn decodes_all_top_level_types() {
        for (raw, expected) in [
            (0, FeedResponseType::InitialFeed),
            (1, FeedResponseType::LiveFeed),
            (2, FeedResponseType::MarketInfo),
        ] {
            let response = FeedResponse {
                r#type: raw,
                feeds: HashMap::new(),
                current_ts: 1794219944214,
                market_info: None,
            };
            let bytes = encode(&response);
            let diag = decode_feed_response_diagnostic(&bytes).expect("decode");
            assert_eq!(diag.response_type_raw, raw);
            assert_eq!(diag.response_type, expected);
            assert_eq!(diag.current_ts, 1794219944214);
            assert!(!diag.has_market_info);
        }
    }

    #[test]
    fn preserves_unknown_enum_value() {
        let response = FeedResponse {
            r#type: 99,
            feeds: HashMap::new(),
            current_ts: 0,
            market_info: None,
        };
        let bytes = encode(&response);
        let diag = decode_feed_response_diagnostic(&bytes).expect("decode");
        assert_eq!(diag.response_type, FeedResponseType::Unknown(99));
        assert_eq!(diag.response_type_raw, 99);
        assert_eq!(response_type_from_raw(99), FeedResponseType::Unknown(99));
    }

    #[test]
    fn decodes_ltpc_feed_variant() {
        let mut feeds = HashMap::new();
        feeds.insert(
            "NSE_EQ|INE848E01016".to_owned(),
            Feed {
                feed_union: Some(FeedUnion::Ltpc(ltpc(None))),
                request_mode: 0,
            },
        );
        let bytes = encode(&FeedResponse {
            r#type: 1,
            feeds,
            current_ts: 0,
            market_info: None,
        });
        let diag = decode_feed_response_diagnostic(&bytes).expect("decode");
        assert_eq!(diag.feed_keys, vec!["NSE_EQ|INE848E01016".to_owned()]);
        assert_eq!(diag.feeds[0].kind, FeedKind::Ltpc);
        let decoded = decode_feed_response(&bytes).expect("decode raw");
        let feed = &decoded.feeds["NSE_EQ|INE848E01016"];
        match &feed.feed_union {
            Some(FeedUnion::Ltpc(value)) => {
                assert_eq!(value.ltp, 80.39);
                assert!(value.iep.is_none());
            }
            _ => std::panic!("expected Ltpc"),
        }
    }

    #[test]
    fn decodes_market_full_feed_variant() {
        let market_ff = MarketFullFeed {
            ltpc: Some(ltpc(Some(79.75))),
            market_level: Some(MarketLevel {
                bid_ask_quote: vec![Quote {
                    bid_q: 10,
                    bid_p: 80.0,
                    ask_q: 10,
                    ask_p: 80.5,
                }],
            }),
            option_greeks: Some(OptionGreeks {
                delta: 0.5,
                theta: 0.0,
                gamma: 0.0,
                vega: 0.0,
                rho: 0.0,
            }),
            market_ohlc: Some(MarketOhlc {
                ohlc: vec![Ohlc {
                    interval: "day".to_owned(),
                    open: 81.3,
                    high: 81.38,
                    low: 79.23,
                    close: 80.39,
                    vol: 17190932,
                    ts: 1794162600000,
                }],
            }),
            atp: 79.75,
            vtt: 17190932,
            oi: 0.0,
            iv: 0.0,
            tbq: 0.0,
            tsq: 14624.0,
            iep: 0.0,
            rp: 80.12,
            ieq: 0,
            iiq_total: 0,
            iiq_m: 0,
            cas_eligible: true,
        };
        let mut feeds = HashMap::new();
        feeds.insert(
            "NSE_EQ|INE848E01016".to_owned(),
            Feed {
                feed_union: Some(FeedUnion::FullFeed(FullFeed {
                    full_feed_union: Some(FullFeedUnion::MarketFf(market_ff)),
                })),
                request_mode: 1,
            },
        );
        let bytes = encode(&FeedResponse {
            r#type: 1,
            feeds,
            current_ts: 0,
            market_info: None,
        });
        let diag = decode_feed_response_diagnostic(&bytes).expect("decode");
        assert_eq!(diag.feeds[0].kind, FeedKind::FullFeed);
        let decoded = decode_feed_response(&bytes).expect("decode raw");
        match &decoded.feeds["NSE_EQ|INE848E01016"].feed_union {
            Some(FeedUnion::FullFeed(full)) => match &full.full_feed_union {
                Some(FullFeedUnion::MarketFf(market)) => {
                    assert_eq!(market.vtt, 17190932);
                    assert!(market.ltpc.as_ref().expect("ltpc").iep.is_some());
                }
                _ => std::panic!("expected MarketFf"),
            },
            _ => std::panic!("expected FullFeed"),
        }
    }

    #[test]
    fn decodes_index_full_feed_variant() {
        let index_ff = IndexFullFeed {
            ltpc: Some(ltpc(None)),
            market_ohlc: Some(MarketOhlc { ohlc: Vec::new() }),
        };
        let mut feeds = HashMap::new();
        feeds.insert(
            "NSE_INDEX|NIFTY 50".to_owned(),
            Feed {
                feed_union: Some(FeedUnion::FullFeed(FullFeed {
                    full_feed_union: Some(FullFeedUnion::IndexFf(index_ff)),
                })),
                request_mode: 1,
            },
        );
        let bytes = encode(&FeedResponse {
            r#type: 1,
            feeds,
            current_ts: 0,
            market_info: None,
        });
        let decoded = decode_feed_response(&bytes).expect("decode raw");
        match &decoded.feeds["NSE_INDEX|NIFTY 50"].feed_union {
            Some(FeedUnion::FullFeed(full)) => match &full.full_feed_union {
                Some(FullFeedUnion::IndexFf(_)) => (),
                _ => std::panic!("expected IndexFf"),
            },
            _ => std::panic!("expected FullFeed"),
        }
    }

    #[test]
    fn decodes_first_level_with_greeks_variant() {
        let first = FirstLevelWithGreeks {
            ltpc: Some(ltpc(None)),
            first_depth: Some(Quote {
                bid_q: 10,
                bid_p: 80.0,
                ask_q: 10,
                ask_p: 80.5,
            }),
            option_greeks: Some(OptionGreeks {
                delta: 0.1,
                theta: 0.0,
                gamma: 0.0,
                vega: 0.0,
                rho: 0.0,
            }),
            vtt: 100,
            oi: 0.0,
            iv: 0.0,
        };
        let mut feeds = HashMap::new();
        feeds.insert(
            "NSE_FO|OPT".to_owned(),
            Feed {
                feed_union: Some(FeedUnion::FirstLevelWithGreeks(first)),
                request_mode: 2,
            },
        );
        let bytes = encode(&FeedResponse {
            r#type: 1,
            feeds,
            current_ts: 0,
            market_info: None,
        });
        let diag = decode_feed_response_diagnostic(&bytes).expect("decode");
        assert_eq!(diag.feeds[0].kind, FeedKind::FirstLevelWithGreeks);
    }

    #[test]
    fn decodes_market_info_maps() {
        let mut segment = HashMap::new();
        segment.insert("NSE_EQ".to_owned(), 2_i32);
        let status = StatusInfo {
            status: "OPEN".to_owned(),
            updated_time: 123,
        };
        let mut cas = HashMap::new();
        cas.insert("NSE_EQ".to_owned(), status.clone());
        let mut pre = HashMap::new();
        pre.insert("NSE_EQ".to_owned(), status);
        let bytes = encode(&FeedResponse {
            r#type: 2,
            feeds: HashMap::new(),
            current_ts: 0,
            market_info: Some(MarketInfo {
                segment_status: segment,
                cas_market_status: cas,
                pre_open_session_status: pre,
            }),
        });
        let diag = decode_feed_response_diagnostic(&bytes).expect("decode");
        assert!(diag.has_market_info);
        let decoded = decode_feed_response(&bytes).expect("decode raw");
        let info = decoded.market_info.expect("market info");
        assert_eq!(info.segment_status["NSE_EQ"], 2);
        assert_eq!(info.cas_market_status["NSE_EQ"].status, "OPEN");
        assert_eq!(info.pre_open_session_status["NSE_EQ"].updated_time, 123);
    }

    #[test]
    fn decodes_optional_iep_present_and_absent() {
        let with = FeedResponse {
            r#type: 1,
            feeds: {
                let mut m = HashMap::new();
                m.insert(
                    "K".to_owned(),
                    Feed {
                        feed_union: Some(FeedUnion::Ltpc(ltpc(Some(1.5)))),
                        request_mode: 0,
                    },
                );
                m
            },
            current_ts: 0,
            market_info: None,
        };
        let without = FeedResponse {
            r#type: 1,
            feeds: {
                let mut m = HashMap::new();
                m.insert(
                    "K".to_owned(),
                    Feed {
                        feed_union: Some(FeedUnion::Ltpc(ltpc(None))),
                        request_mode: 0,
                    },
                );
                m
            },
            current_ts: 0,
            market_info: None,
        };
        let decoded_with = decode_feed_response(&encode(&with)).expect("decode");
        let decoded_without = decode_feed_response(&encode(&without)).expect("decode");
        match &decoded_with.feeds["K"].feed_union {
            Some(FeedUnion::Ltpc(v)) => assert_eq!(v.iep, Some(1.5)),
            _ => std::panic!("expected Ltpc"),
        }
        match &decoded_without.feeds["K"].feed_union {
            Some(FeedUnion::Ltpc(v)) => assert!(v.iep.is_none()),
            _ => std::panic!("expected Ltpc"),
        }
    }

    #[test]
    fn accepts_unknown_field() {
        let mut bytes = encode(&FeedResponse {
            r#type: 1,
            feeds: HashMap::new(),
            current_ts: 1,
            market_info: None,
        });
        bytes.extend_from_slice(&[0x98, 0x06, 0x2A]);
        let diag = decode_feed_response_diagnostic(&bytes).expect("decode with unknown field");
        assert_eq!(diag.response_type, FeedResponseType::LiveFeed);
    }

    #[test]
    fn decodes_unset_feed_with_unknown_request_mode() {
        let mut feeds = HashMap::new();
        feeds.insert(
            "NSE_EQ|INE848E01016".to_owned(),
            Feed {
                feed_union: None,
                request_mode: 99,
            },
        );
        let bytes = encode(&FeedResponse {
            r#type: 1,
            feeds,
            current_ts: 0,
            market_info: None,
        });
        let diag = decode_feed_response_diagnostic(&bytes).expect("decode unset feed");
        assert_eq!(diag.feeds.len(), 1);
        assert_eq!(diag.feeds[0].kind, FeedKind::Unset);
        assert_eq!(diag.feeds[0].request_mode_raw, 99);
    }

    #[test]
    fn rejects_truncated_and_malformed() {
        assert!(decode_feed_response(&[0x01]).is_err());
        assert!(decode_feed_response(&[]).is_ok());
        assert!(decode_feed_response(b"not protobuf at all").is_err());
        let mut bytes = encode(&FeedResponse {
            r#type: 1,
            feeds: HashMap::new(),
            current_ts: 1794219944214,
            market_info: None,
        });
        bytes.truncate(bytes.len() / 2);
        assert!(
            decode_feed_response(&bytes).is_err(),
            "truncated protobuf payload must fail"
        );
    }

    #[test]
    fn diagnostic_is_deterministic_and_sorted() {
        let mut feeds = HashMap::new();
        feeds.insert(
            "B".to_owned(),
            Feed {
                feed_union: Some(FeedUnion::Ltpc(ltpc(None))),
                request_mode: 0,
            },
        );
        feeds.insert(
            "A".to_owned(),
            Feed {
                feed_union: Some(FeedUnion::Ltpc(ltpc(None))),
                request_mode: 0,
            },
        );
        let bytes = encode(&FeedResponse {
            r#type: 1,
            feeds,
            current_ts: 0,
            market_info: None,
        });
        let first = decode_feed_response_diagnostic(&bytes).expect("decode");
        let second = decode_feed_response_diagnostic(&bytes).expect("decode");
        assert_eq!(first, second);
        assert_eq!(first.feed_keys, vec!["A".to_owned(), "B".to_owned()]);
    }

    #[test]
    fn schema_hash_matches_source_record() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workspace = manifest_dir
            .parent()
            .and_then(|p| p.parent())
            .expect("workspace");
        let schema_path = workspace.join("schemas/upstox/v3/MarketDataFeedV3.proto");
        let source_path = workspace.join("schemas/upstox/v3/source.json");
        let schema_bytes = std::fs::read(&schema_path).expect("schema");
        let source_text = std::fs::read_to_string(&source_path).expect("source.json");
        let source: serde_json::Value = serde_json::from_str(&source_text).expect("source json");
        let expected = source["canonical_source"]["sha256"]
            .as_str()
            .expect("sha in source");
        assert_eq!(sha256_hex(&schema_bytes), expected.to_owned());
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let digest = hasher.finalize();
        let mut hex = String::with_capacity(64);
        for byte in digest.iter() {
            hex.push_str(&format!("{byte:02x}"));
        }
        hex
    }
}
