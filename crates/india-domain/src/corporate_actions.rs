//! Bitemporal corporate-action events with immutable source lineage.
//!
//! `share_ratio_numerator / share_ratio_denominator` means new shares per old
//! share. A 2-for-1 split is 2/1, so a pre-ex price view is multiplied by 1/2.
//! Raw prices are never overwritten. Dividends are cash-flow events, not price
//! adjustments. Complex actions fail closed until an approved transform exists.

use jiff::{Timestamp, civil::Date};
use serde::{Deserialize, Serialize};

use crate::{CorporateActionKind, InstrumentId, Paise, SourceProvenance};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorporateActionEvent {
    pub action_id: String,
    pub instrument_id: InstrumentId,
    pub kind: CorporateActionKind,
    pub effective_date: Date,
    pub announced_at: Timestamp,
    /// Knowledge time: when this exact source record entered the system.
    pub recorded_at: Timestamp,
    pub share_ratio_numerator: i64,
    pub share_ratio_denominator: i64,
    pub amount_paise: Option<i64>,
    pub provenance: SourceProvenance,
}

impl CorporateActionEvent {
    pub fn validate(&self) -> Result<(), anyhow::Error> {
        anyhow::ensure!(
            !self.action_id.trim().is_empty(),
            "corporate action id must not be empty"
        );
        anyhow::ensure!(
            self.share_ratio_numerator > 0 && self.share_ratio_denominator > 0,
            "corporate action share ratio must be positive"
        );
        anyhow::ensure!(
            self.amount_paise.is_none_or(|amount| amount >= 0),
            "corporate action amount cannot be negative"
        );
        anyhow::ensure!(
            self.recorded_at >= self.announced_at,
            "corporate action is backdated before public announcement"
        );
        self.provenance.validate()?;
        anyhow::ensure!(
            self.recorded_at >= self.provenance.retrieved_at,
            "corporate action is backdated before source retrieval"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct CorporateActionLedger {
    records: Vec<CorporateActionEvent>,
}

impl CorporateActionLedger {
    pub fn append(&mut self, record: CorporateActionEvent) -> Result<(), anyhow::Error> {
        record.validate()?;
        anyhow::ensure!(
            !self.records.iter().any(|existing| {
                existing.action_id == record.action_id
                    || (existing.instrument_id == record.instrument_id
                        && existing.effective_date == record.effective_date
                        && existing.kind == record.kind)
            }),
            "duplicate or contradictory corporate-action slot for {} on {}",
            record.instrument_id,
            record.effective_date
        );
        self.records.push(record);
        self.records.sort_by(|a, b| {
            (
                a.instrument_id.as_str(),
                a.effective_date,
                a.recorded_at,
                &a.action_id,
            )
                .cmp(&(
                    b.instrument_id.as_str(),
                    b.effective_date,
                    b.recorded_at,
                    &b.action_id,
                ))
        });
        Ok(())
    }

    #[must_use]
    pub fn records(&self) -> &[CorporateActionEvent] {
        &self.records
    }

    /// Only records both announced and ingested by the caller's knowledge time.
    #[must_use]
    pub fn known_as_of(
        &self,
        instrument_id: &InstrumentId,
        knowledge_at: Timestamp,
    ) -> Vec<&CorporateActionEvent> {
        self.records
            .iter()
            .filter(|record| {
                &record.instrument_id == instrument_id
                    && record.announced_at <= knowledge_at
                    && record.recorded_at <= knowledge_at
            })
            .collect()
    }

    /// Split/bonus/consolidation view for a raw historical price. Returns the
    /// exact adjusted price and applied event IDs. Unknown/complex transforms
    /// are not approximated; callers must add an independently reviewed rule.
    pub fn adjusted_pre_action_price(
        &self,
        instrument_id: &InstrumentId,
        raw_price: Paise,
        observation_date: Date,
        knowledge_at: Timestamp,
    ) -> Result<(Paise, Vec<String>), anyhow::Error> {
        anyhow::ensure!(raw_price.0 > 0, "raw price must be positive");
        let mut price = i128::from(raw_price.0);
        let mut lineage = Vec::new();
        for record in self.known_as_of(instrument_id, knowledge_at) {
            if record.effective_date <= observation_date {
                continue;
            }
            match record.kind {
                CorporateActionKind::Split
                | CorporateActionKind::Bonus
                | CorporateActionKind::Consolidation => {
                    let numerator = i128::from(record.share_ratio_denominator);
                    let denominator = i128::from(record.share_ratio_numerator);
                    let scaled = price.checked_mul(numerator).ok_or_else(|| {
                        anyhow::anyhow!("corporate-action adjustment overflows i128")
                    })?;
                    anyhow::ensure!(
                        scaled % denominator == 0,
                        "corporate-action adjustment is not exactly representable in paise"
                    );
                    price = scaled / denominator;
                    lineage.push(record.action_id.clone());
                }
                CorporateActionKind::Dividend => { /* preserve as a separately booked cash event */
                }
                CorporateActionKind::Rights
                | CorporateActionKind::SymbolChange
                | CorporateActionKind::Delisting => {
                    anyhow::bail!(
                        "corporate action {} requires an approved point-in-time transform",
                        record.action_id
                    );
                }
            }
        }
        anyhow::ensure!(
            price > 0 && price <= i128::from(i64::MAX),
            "adjusted price is outside the paise domain"
        );
        Ok((Paise(price as i64), lineage))
    }
}

#[cfg(all(test, feature = "market-shaped-tests"))]
mod tests {
    use super::*;
    use crate::{Segment, Venue, provenance::provenance_for_source};

    fn record(
        kind: CorporateActionKind,
        retrieved: Timestamp,
        announced: Timestamp,
        recorded: Timestamp,
    ) -> CorporateActionEvent {
        let provenance = provenance_for_source(
            "exchange-corporate-actions",
            Venue::Nse,
            Segment::Cash,
            retrieved,
            "fixture-v1",
            "unit-test",
            b"id,date,ratio\nCA1,2026-11-20,2/1\n",
        );
        CorporateActionEvent {
            action_id: "CA1".into(),
            instrument_id: InstrumentId::new("in.test").unwrap(),
            kind,
            effective_date: Date::new(2026, 11, 20).unwrap(),
            announced_at: announced,
            recorded_at: recorded,
            share_ratio_numerator: 2,
            share_ratio_denominator: 1,
            amount_paise: None,
            provenance,
        }
    }

    #[test]
    fn split_is_bitemporal_exact_and_source_hashed() {
        let retrieved = "2026-11-10T10:00:00Z".parse::<Timestamp>().unwrap();
        let announced = "2026-11-09T10:00:00Z".parse::<Timestamp>().unwrap();
        let recorded = "2026-11-10T10:01:00Z".parse::<Timestamp>().unwrap();
        let mut ledger = CorporateActionLedger::default();
        let action = record(CorporateActionKind::Split, retrieved, announced, recorded);
        let id = action.instrument_id.clone();
        ledger.append(action).unwrap();
        let observation = Date::new(2026, 11, 12).unwrap();
        let before_knowledge = "2026-11-10T10:00:30Z".parse::<Timestamp>().unwrap();
        assert!(ledger.known_as_of(&id, before_knowledge).is_empty());
        let (adjusted, lineage) = ledger
            .adjusted_pre_action_price(&id, Paise(10_000), observation, recorded)
            .unwrap();
        assert_eq!(adjusted, Paise(5_000));
        assert_eq!(lineage, vec!["CA1"]);
        assert_eq!(ledger.records()[0].provenance.sha256_hex.len(), 64);
    }

    #[test]
    fn action_cannot_be_backdated_and_duplicates_are_rejected() {
        let retrieved = "2026-11-10T10:00:00Z".parse::<Timestamp>().unwrap();
        let announced = "2026-11-09T10:00:00Z".parse::<Timestamp>().unwrap();
        let before_retrieval = "2026-11-10T09:59:00Z".parse::<Timestamp>().unwrap();
        let mut bad = record(
            CorporateActionKind::Split,
            retrieved,
            announced,
            before_retrieval,
        );
        assert!(bad.validate().is_err());
        bad.recorded_at = retrieved;
        let mut ledger = CorporateActionLedger::default();
        ledger.append(bad.clone()).unwrap();
        assert!(ledger.append(bad).is_err());
    }
}
