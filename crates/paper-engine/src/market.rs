//! Quote continuity and feed-health tracking.
//!
//! Derived from the internal Nirnay `crates/nirnay-engine/src/market.rs` at source snapshot
//! `cc3a34c317a86a9984401efe90fa3f43d67a1787`.
//!
//! The source-sequence watermark and gap bookkeeping are preserved. The book
//! records *displayed* top-of-book liquidity, so a matcher consuming it is a
//! deterministic fill model, not a historical execution simulator.

use std::collections::BTreeMap;

use account_journal::StoredRecord;
use thiserror::Error;
use trading_core::{
    Continuity, DomainError, DoubledPaise, EventId, EventPayload, FeedHealth, InstrumentId,
    LogicalNanos, PricePaise, QuoteSnapshot, Venue,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct BookEntry {
    pub source_id: String,
    pub source_sequence: u64,
    pub available_at: LogicalNanos,
    pub quote: QuoteSnapshot,
    pub journal_sequence: u64,
    pub journal_hash_hex: String,
    /// Identity of the journal event that produced this book entry — the
    /// fill-provenance anchor (`BookEntry.journal_event_id`).
    pub journal_event_id: EventId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct QuoteObservation {
    pub instrument_id: InstrumentId,
    pub quote: QuoteSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum QuoteDisposition {
    New,
    Duplicate,
    Late,
}

/// Deterministic remaining displayed liquidity per instrument and side.
///
/// The budget is reconciled from displayed top-of-book quantities, never
/// from wall-clock time or wall-clock quotes: a repeated quote level must
/// never replenish liquidity that a fill already consumed. Fills debit the
/// budget from the same replayable journal path that produced them, so
/// replay reconstructs exactly the consumed state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct BookLiquidity {
    bid_remaining: i64,
    ask_remaining: i64,
}

#[derive(Debug, Error)]
pub(super) enum MarketError {
    #[error("quote has no source identity or sequence")]
    MissingSource,
    #[error("source sequence {sequence} from {origin} was reused with different content")]
    ConflictingSequence { origin: String, sequence: u64 },
    #[error("logical quote time is invalid: {0}")]
    Domain(#[from] DomainError),
    #[error(
        "fill of {requested} exceeds the remaining liquidity budget of {remaining} on side {side}"
    )]
    BudgetBreached {
        requested: i64,
        remaining: i64,
        side: String,
    },
}

/// Books and liquidity are keyed by exchange as well as instrument: the same
/// ISIN can trade on NSE and BSE at different prices, and a quote observed on
/// one venue is never evidence for an order on the other.
type BookKey = (InstrumentId, Venue);

#[derive(Clone, Debug, Default)]
pub(super) struct MarketState {
    books: BTreeMap<BookKey, BookEntry>,
    liquidity: BTreeMap<BookKey, BookLiquidity>,
    expected_by_source: BTreeMap<String, u64>,
    gaps: BTreeMap<String, Option<u64>>,
    seen: BTreeMap<(String, u64), QuoteObservation>,
}

impl MarketState {
    /// Remaining displayed liquidity for the bid side (sell orders consume it)
    /// on one venue.
    pub fn bid_remaining(&self, instrument_id: &InstrumentId, venue: Venue) -> i64 {
        self.liquidity
            .get(&(instrument_id.clone(), venue))
            .map(|budget| budget.bid_remaining)
            .unwrap_or(0)
    }

    /// Remaining displayed liquidity for the ask side (buy orders consume it)
    /// on one venue.
    pub fn ask_remaining(&self, instrument_id: &InstrumentId, venue: Venue) -> i64 {
        self.liquidity
            .get(&(instrument_id.clone(), venue))
            .map(|budget| budget.ask_remaining)
            .unwrap_or(0)
    }

    /// Debit a fill against its displayed side budget. The budget is the
    /// single consumed-liquidity authority: fills beyond the budget reject
    /// (rather than conjuring liquidity), and the same debit replays from
    /// journal records, so a restart never restores consumed liquidity.
    #[allow(dead_code)]
    pub fn consume_liquidity(
        &mut self,
        instrument_id: &InstrumentId,
        venue: Venue,
        side: trading_core::Side,
        quantity: i64,
    ) -> Result<(), MarketError> {
        if quantity <= 0 {
            return Err(MarketError::BudgetBreached {
                requested: quantity,
                remaining: 0,
                side: format!("{side:?}"),
            });
        }
        let budget = self
            .liquidity
            .entry((instrument_id.clone(), venue))
            .or_default();
        let remaining = match side {
            trading_core::Side::Buy => &mut budget.ask_remaining,
            trading_core::Side::Sell => &mut budget.bid_remaining,
        };
        if quantity > *remaining {
            return Err(MarketError::BudgetBreached {
                requested: quantity,
                remaining: *remaining,
                side: format!("{side:?}"),
            });
        }
        *remaining -= quantity;
        Ok(())
    }

    /// Debit a fill that has already been validated against the remaining
    /// budget. Callers must apply the sufficiency check first; this helper
    /// has no fallible path so the projection mutation phase
    /// cannot partially commit.
    pub(super) fn debit_validated_liquidity(
        &mut self,
        instrument_id: &InstrumentId,
        venue: Venue,
        side: trading_core::Side,
        quantity: i64,
    ) {
        debug_assert!(quantity > 0);
        let Some(budget) = self.liquidity.get_mut(&(instrument_id.clone(), venue)) else {
            debug_assert!(false, "validated liquidity budget must exist");
            return;
        };
        let remaining = match side {
            trading_core::Side::Buy => &mut budget.ask_remaining,
            trading_core::Side::Sell => &mut budget.bid_remaining,
        };
        debug_assert!(quantity <= *remaining);
        *remaining -= quantity;
    }

    /// Reconcile one side's budget against the transition from the previous
    /// displayed quote to the current one:
    ///
    /// - Same price, same quantity: no replenishment (the fill-consumed
    ///   amount stays consumed).
    /// - Same price, changed quantity: adjust the remaining budget by the
    ///   displayed delta only, clamped to remain non-negative.
    /// - Price changed, or the side (re)appears: a fresh budget is
    ///   initialized from the new displayed quantity.
    /// - Side disappears: the budget is cleared.
    ///
    /// A source switch alone can never replenish: the budget keys off the
    /// displayed values, not the source identity.
    fn reconcile_side(
        remaining: i64,
        previous: Option<(PricePaise, i64)>,
        current: Option<(PricePaise, i64)>,
    ) -> i64 {
        match (previous, current) {
            (Some((prev_price, prev_qty)), Some((new_price, new_qty))) => {
                if prev_price == new_price {
                    remaining.saturating_add(new_qty - prev_qty).max(0)
                } else {
                    new_qty
                }
            }
            (_, Some((_, new_qty))) => new_qty,
            _ => 0,
        }
    }

    fn reconcile_liquidity(
        &mut self,
        instrument_id: &InstrumentId,
        venue: Venue,
        previous: Option<&QuoteSnapshot>,
        current: &QuoteSnapshot,
    ) {
        let budget = self
            .liquidity
            .entry((instrument_id.clone(), venue))
            .or_default();
        let previous_side = |side: u8| -> Option<(PricePaise, i64)> {
            let quote = previous?;
            let (price, quantity) = match side {
                0 => (quote.bid_price?, quote.bid_quantity.get()),
                _ => (quote.ask_price?, quote.ask_quantity.get()),
            };
            Some((price, quantity))
        };
        let current_side = |side: u8| -> Option<(PricePaise, i64)> {
            let (price, quantity) = match side {
                0 => (current.bid_price?, current.bid_quantity.get()),
                _ => (current.ask_price?, current.ask_quantity.get()),
            };
            Some((price, quantity))
        };
        budget.bid_remaining =
            Self::reconcile_side(budget.bid_remaining, previous_side(0), current_side(0));
        budget.ask_remaining =
            Self::reconcile_side(budget.ask_remaining, previous_side(1), current_side(1));
    }

    pub fn classify(
        &self,
        source_id: &str,
        source_sequence: u64,
        instrument_id: &InstrumentId,
        quote: &QuoteSnapshot,
    ) -> Result<QuoteDisposition, MarketError> {
        if let Some(observation) = self.seen.get(&(source_id.to_owned(), source_sequence)) {
            return if observation.instrument_id == *instrument_id && observation.quote == *quote {
                Ok(QuoteDisposition::Duplicate)
            } else {
                Err(MarketError::ConflictingSequence {
                    origin: source_id.to_owned(),
                    sequence: source_sequence,
                })
            };
        }
        if self
            .expected_by_source
            .get(source_id)
            .is_some_and(|expected| source_sequence < *expected)
        {
            return Ok(QuoteDisposition::Late);
        }
        Ok(QuoteDisposition::New)
    }

    pub fn expected_sequence(&self, source_id: &str) -> Option<u64> {
        self.expected_by_source.get(source_id).copied()
    }

    pub fn apply_record(&mut self, record: &StoredRecord) -> Result<(), MarketError> {
        let envelope = &record.body.envelope;
        let source_id = envelope
            .source_id
            .as_deref()
            .ok_or(MarketError::MissingSource)?;
        let source_sequence = envelope.source_sequence.ok_or(MarketError::MissingSource)?;
        match &record.body.payload {
            EventPayload::FeedGapDetected {
                expected_sequence, ..
            } => {
                self.gaps.insert(source_id.to_owned(), *expected_sequence);
            }
            EventPayload::LateQuoteObserved { quote, .. } => {
                let key = (source_id.to_owned(), source_sequence);
                if self
                    .seen
                    .insert(
                        key,
                        QuoteObservation {
                            instrument_id: quote.instrument_id.clone(),
                            quote: quote.clone(),
                        },
                    )
                    .is_some()
                {
                    return Err(MarketError::ConflictingSequence {
                        origin: source_id.to_owned(),
                        sequence: source_sequence,
                    });
                }
                let expected = self.expected_sequence(source_id);
                self.gaps.insert(source_id.to_owned(), expected);
            }
            EventPayload::QuoteReceived { quote } => {
                let key = (source_id.to_owned(), source_sequence);
                if self
                    .seen
                    .insert(
                        key,
                        QuoteObservation {
                            instrument_id: quote.instrument_id.clone(),
                            quote: quote.clone(),
                        },
                    )
                    .is_some()
                {
                    return Err(MarketError::ConflictingSequence {
                        origin: source_id.to_owned(),
                        sequence: source_sequence,
                    });
                }
                match quote.continuity {
                    Continuity::Snapshot => {
                        self.gaps.remove(source_id);
                        self.expected_by_source
                            .insert(source_id.to_owned(), source_sequence.saturating_add(1));
                    }
                    Continuity::Delta => {
                        let expected = self.expected_sequence(source_id);
                        if expected != Some(source_sequence) || self.gaps.contains_key(source_id) {
                            self.gaps.entry(source_id.to_owned()).or_insert(expected);
                        }
                        self.expected_by_source
                            .insert(source_id.to_owned(), source_sequence.saturating_add(1));
                    }
                }
                let key = (quote.instrument_id.clone(), quote.venue);
                let update_book = self.books.get(&key).is_none_or(|current| {
                    current.source_id != source_id || source_sequence > current.source_sequence
                });
                if update_book {
                    let previous_quote = self.books.get(&key).map(|book| book.quote.clone());
                    self.reconcile_liquidity(
                        &quote.instrument_id,
                        quote.venue,
                        previous_quote.as_ref(),
                        quote,
                    );
                    self.books.insert(
                        key,
                        BookEntry {
                            source_id: source_id.to_owned(),
                            source_sequence,
                            available_at: envelope.times.available_time,
                            quote: quote.clone(),
                            journal_sequence: record.sequence,
                            journal_hash_hex: record.hash.hex(),
                            journal_event_id: record.body.envelope.event_id.clone(),
                        },
                    );
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub fn health(
        &self,
        instrument_id: &InstrumentId,
        venue: Venue,
        now: LogicalNanos,
        maximum_age_ns: u64,
    ) -> FeedHealth {
        let Some(book) = self.books.get(&(instrument_id.clone(), venue)) else {
            return FeedHealth::NoQuote;
        };
        if let Some(expected_sequence) = self.gaps.get(&book.source_id) {
            return FeedHealth::Gapped {
                expected_sequence: *expected_sequence,
                observed_sequence: book.source_sequence,
            };
        }
        let (Some(bid), Some(ask)) = (book.quote.bid_price, book.quote.ask_price) else {
            return if book.quote.bid_price.is_none() && book.quote.ask_price.is_none() {
                FeedHealth::NoQuote
            } else {
                FeedHealth::OneSided
            };
        };
        if bid >= ask {
            return FeedHealth::Crossed;
        }
        if book.quote.bid_quantity.is_zero() || book.quote.ask_quantity.is_zero() {
            return FeedHealth::Incomplete;
        }
        let age_ns = now.elapsed_since(book.available_at).unwrap_or(u64::MAX);
        if age_ns > maximum_age_ns {
            FeedHealth::Stale { age_ns }
        } else {
            FeedHealth::Healthy { age_ns }
        }
    }

    pub fn book(&self, instrument_id: &InstrumentId, venue: Venue) -> Option<&BookEntry> {
        self.books.get(&(instrument_id.clone(), venue))
    }

    /// Exact mid price in doubled paise (M₂ = b + a) for an instrument.
    ///
    /// Positions are instrument-level, so valuation uses the most recently
    /// observed venue book for that instrument. Execution never uses this
    /// fallback: matching, feed health and fill replay all resolve the
    /// order's own venue.
    pub fn mark_price_doubled(&self, instrument_id: &InstrumentId) -> Option<DoubledPaise> {
        self.books
            .iter()
            .filter(|((id, _), _)| id == instrument_id)
            .max_by_key(|(_, entry)| entry.journal_sequence)
            .and_then(|(_, entry)| entry.quote.midpoint_doubled())
    }

    pub fn source_quote_ref(
        &self,
        instrument_id: &InstrumentId,
        venue: Venue,
    ) -> Option<(u64, String)> {
        let book = self.book(instrument_id, venue)?;
        Some((book.journal_sequence, book.journal_hash_hex.clone()))
    }

    /// Journal sequence of the quote record that produced the current book
    /// for one venue.
    pub fn book_journal_sequence(&self, instrument_id: &InstrumentId, venue: Venue) -> Option<u64> {
        self.book(instrument_id, venue)
            .map(|book| book.journal_sequence)
    }
}

#[cfg(all(test, feature = "market-shaped-tests"))]
mod tests {
    use account_journal::{Hash32, StoredRecord};
    use trading_core::{
        Continuity, EventEnvelope, EventId, EventPayload, EventTimes, InstrumentId, LogicalNanos,
        Paise, PricePaise, Quantity, QuoteSnapshot, RunId, Side, StoredBody, UtcNanos, Venue,
    };

    use super::MarketState;

    fn instrument(value: &str) -> InstrumentId {
        InstrumentId::new(value).expect("valid synthetic instrument id")
    }

    fn quote_record(
        id: &InstrumentId,
        bid: Option<(i64, i64)>,
        ask: Option<(i64, i64)>,
        source: &str,
        sequence: u64,
    ) -> StoredRecord {
        StoredRecord {
            sequence,
            previous_hash: Hash32::ZERO,
            hash: Hash32::ZERO,
            body: StoredBody {
                envelope: EventEnvelope {
                    event_id: EventId::for_run(RunId::from_bytes([7u8; 16]), sequence),
                    schema_version: 1,
                    stream_id: "test".to_owned(),
                    source_id: Some(source.to_owned()),
                    source_clock_domain: None,
                    session_id: None,
                    source_sequence: Some(sequence),
                    correlation_id: None,
                    causation_id: None,
                    times: EventTimes {
                        source_time_utc: None,
                        received_time_utc: UtcNanos::new(0),
                        available_time: LogicalNanos::new(0),
                        recorded_time_utc: UtcNanos::new(0),
                    },
                    quality: std::collections::BTreeSet::new(),
                },
                payload: EventPayload::QuoteReceived {
                    quote: QuoteSnapshot {
                        instrument_id: id.clone(),
                        venue: trading_core::Venue::Nse,
                        bid_price: bid.map(|(price, _)| PricePaise::new(Paise(price)).unwrap()),
                        bid_quantity: Quantity::new(bid.map_or(0, |(_, qty)| qty)).unwrap(),
                        ask_price: ask.map(|(price, _)| PricePaise::new(Paise(price)).unwrap()),
                        ask_quantity: Quantity::new(ask.map_or(0, |(_, qty)| qty)).unwrap(),
                        continuity: Continuity::Snapshot,
                    },
                },
            },
        }
    }

    fn budgets(state: &MarketState, id: &InstrumentId) -> (i64, i64) {
        (
            state.bid_remaining(id, Venue::Nse),
            state.ask_remaining(id, Venue::Nse),
        )
    }

    #[test]
    fn liquidity_budget_transitions_follow_the_written_rules() {
        let id = instrument("nse.cash.synidx");
        let mut state = MarketState::default();

        // First observation initializes one unit per side.
        state
            .apply_record(&quote_record(
                &id,
                Some((100, 1)),
                Some((101, 1)),
                "feed",
                1,
            ))
            .unwrap();
        assert_eq!(budgets(&state, &id), (1, 1));

        // Fill consumes one ask unit; consume the LAST ask unit too.
        state
            .consume_liquidity(&id, Venue::Nse, Side::Buy, 1)
            .unwrap();
        assert_eq!(budgets(&state, &id), (1, 0));

        // Same price, same quantity under a new sequence: NO replenishment.
        state
            .apply_record(&quote_record(
                &id,
                Some((100, 1)),
                Some((101, 1)),
                "feed",
                2,
            ))
            .unwrap();
        assert_eq!(budgets(&state, &id), (1, 0));

        // Changing book sides independently: bid side increased (2), ask unchanged.
        state
            .apply_record(&quote_record(
                &id,
                Some((100, 2)),
                Some((101, 1)),
                "feed",
                3,
            ))
            .unwrap();
        assert_eq!(budgets(&state, &id), (2, 0));

        // Ask quantity decreases while side remains consumed:
        // remaining shrinks by the shrink; clamped at zero.
        state
            .apply_record(&quote_record(
                &id,
                Some((100, 2)),
                Some((101, 1)),
                "feed",
                4,
            ))
            .unwrap();
        assert_eq!(budgets(&state, &id), (2, 0));
    }

    #[test]
    fn increased_displayed_quantity_adds_only_the_increase() {
        let id = instrument("nse.cash.synidx");
        let mut state = MarketState::default();
        state
            .apply_record(&quote_record(
                &id,
                Some((100, 5)),
                Some((101, 5)),
                "feed",
                1,
            ))
            .unwrap();
        state
            .consume_liquidity(&id, Venue::Nse, Side::Buy, 2)
            .unwrap(); // 3 ask remain
        state
            .apply_record(&quote_record(
                &id,
                Some((100, 5)),
                Some((101, 8)),
                "feed",
                2,
            ))
            .unwrap();
        assert_eq!(budgets(&state, &id).1, 6); // 3 + (8 - 5)
    }

    #[test]
    fn decreased_displayed_quantity_shrinks_remaining_budget() {
        let id = instrument("nse.cash.synidx");
        let mut state = MarketState::default();
        state
            .apply_record(&quote_record(
                &id,
                Some((100, 5)),
                Some((101, 5)),
                "feed",
                1,
            ))
            .unwrap();
        state
            .consume_liquidity(&id, Venue::Nse, Side::Sell, 1)
            .unwrap(); // 4 bid remain
        state
            .apply_record(&quote_record(
                &id,
                Some((100, 2)),
                Some((101, 5)),
                "feed",
                2,
            ))
            .unwrap();
        assert_eq!(budgets(&state, &id).0, 1); // 4 - (5 - 2)
    }

    #[test]
    fn price_change_resets_only_that_side() {
        let id = instrument("nse.cash.synidx");
        let mut state = MarketState::default();
        state
            .apply_record(&quote_record(
                &id,
                Some((100, 5)),
                Some((101, 5)),
                "feed",
                1,
            ))
            .unwrap();
        state
            .consume_liquidity(&id, Venue::Nse, Side::Buy, 5)
            .unwrap(); // ask exhausted
        state
            .apply_record(&quote_record(
                &id,
                Some((100, 5)),
                Some((102, 3)),
                "feed",
                2,
            ))
            .unwrap();
        // New ask price line: fresh budget of this level's quantity.
        assert_eq!(budgets(&state, &id), (5, 3));
    }

    #[test]
    fn disappearing_side_clears_its_budget() {
        let id = instrument("nse.cash.synidx");
        let mut state = MarketState::default();
        state
            .apply_record(&quote_record(
                &id,
                Some((100, 5)),
                Some((101, 5)),
                "feed",
                1,
            ))
            .unwrap();
        state
            .apply_record(&quote_record(&id, Some((100, 5)), None, "feed", 2))
            .unwrap();
        assert_eq!(budgets(&state, &id), (5, 0));
    }

    #[test]
    fn source_change_alone_never_replenishes() {
        let id = instrument("nse.cash.synidx");
        let mut state = MarketState::default();
        state
            .apply_record(&quote_record(
                &id,
                Some((100, 5)),
                Some((101, 5)),
                "feed",
                1,
            ))
            .unwrap();
        state
            .consume_liquidity(&id, Venue::Nse, Side::Buy, 5)
            .unwrap(); // ask exhausted
        // A fresh source with the same displayed values adds nothing.
        state
            .apply_record(&quote_record(
                &id,
                Some((100, 5)),
                Some((101, 5)),
                "other",
                1,
            ))
            .unwrap();
        assert_eq!(budgets(&state, &id), (5, 0));
    }

    #[test]
    fn bid_side_is_independent_and_exhaustion_fails_closed() {
        let id = instrument("nse.cash.synidx");
        let mut state = MarketState::default();
        state
            .apply_record(&quote_record(
                &id,
                Some((100, 2)),
                Some((101, 7)),
                "feed",
                1,
            ))
            .unwrap();
        state
            .consume_liquidity(&id, Venue::Nse, Side::Sell, 2)
            .unwrap();
        assert_eq!(budgets(&state, &id), (0, 7));
        assert!(
            state
                .consume_liquidity(&id, Venue::Nse, Side::Sell, 1)
                .is_err()
        );
        state
            .consume_liquidity(&id, Venue::Nse, Side::Buy, 7)
            .unwrap();
        assert!(
            state
                .consume_liquidity(&id, Venue::Nse, Side::Buy, 1)
                .is_err()
        );
        assert!(
            state
                .consume_liquidity(&id, Venue::Nse, Side::Buy, 0)
                .is_err()
        );
    }

    #[test]
    fn replay_reconstructs_identical_budgets() {
        let id = instrument("nse.cash.synidx");
        let records = vec![
            quote_record(&id, Some((100, 5)), Some((101, 8)), "feed", 1),
            quote_record(&id, Some((100, 5)), Some((101, 8)), "feed", 2),
            quote_record(&id, Some((100, 3)), Some((101, 8)), "feed", 3),
        ];
        let mut first = MarketState::default();
        first.apply_record(&records[0]).unwrap();
        first
            .consume_liquidity(&id, Venue::Nse, Side::Buy, 3)
            .unwrap(); // ask budget 5
        first.apply_record(&records[1]).unwrap(); // identical: no change
        first.apply_record(&records[2]).unwrap(); // bid shrinks 5 -> 3
        first
            .consume_liquidity(&id, Venue::Nse, Side::Buy, 3)
            .unwrap(); // ask budget 2

        // A fresh replay through the same record stream lands on the same
        // budget (bid 3, ask 2), with identical fills consuming identical
        // liquidity in both paths.
        let mut second = MarketState::default();
        for record in &records {
            second.apply_record(record).unwrap();
        }
        second
            .consume_liquidity(&id, Venue::Nse, Side::Buy, 3)
            .unwrap();
        second
            .consume_liquidity(&id, Venue::Nse, Side::Buy, 3)
            .unwrap();
        assert_eq!(
            (
                first.bid_remaining(&id, Venue::Nse),
                first.ask_remaining(&id, Venue::Nse)
            ),
            (
                second.bid_remaining(&id, Venue::Nse),
                second.ask_remaining(&id, Venue::Nse)
            )
        );
        // The shared sequence still guards: replaying the same source
        // sequence with a conflicting quote is a conflict.
        assert!(first.apply_record(&records[1]).is_err());
    }

    #[test]
    fn identical_observations_never_replenish_in_a_property_ramp() {
        let id = instrument("nse.cash.synidx");
        let mut state = MarketState::default();
        state
            .apply_record(&quote_record(
                &id,
                Some((100, 1)),
                Some((101, 1)),
                "feed",
                1,
            ))
            .unwrap();
        state
            .consume_liquidity(&id, Venue::Nse, Side::Buy, 1)
            .unwrap();
        // Many identical observations at fresh sequences: budget stays empty.
        for sequence in 2..50 {
            state
                .apply_record(&quote_record(
                    &id,
                    Some((100, 1)),
                    Some((101, 1)),
                    "feed",
                    sequence,
                ))
                .unwrap();
        }
        assert_eq!(budgets(&state, &id), (1, 0));
        // Full refills only come from genuine display increases.
        state
            .apply_record(&quote_record(
                &id,
                Some((100, 1)),
                Some((101, 2)),
                "feed",
                50,
            ))
            .unwrap();
        assert_eq!(budgets(&state, &id), (1, 1));
    }
}
