//! Typed risk placeholders and the deterministic permit-to-intent binding.
//!
//! These values are synthetic placeholders by design: they let the risk
//! engine and authority exercise fail-closed control logic with exact domain
//! types while real sector, depth, volatility, Greeks and borrow masters stay
//! out of scope. The digest is an unkeyed integrity check over order terms; it
//! is not a signature and grants no authenticity (see capability records).

use serde::{
    Deserialize, //
    Serialize,
};
use sha2::{
    Digest, //
    Sha256,
};

use crate::{
    OrderIntent, //
    Paise,
    Quantity,
};

/// Displayed depth and average daily volume, both in lots. Synthetic until a
/// measured venue-depth feed is authorized.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiquidityDepth {
    pub displayed_quantity: Quantity,
    pub adv_quantity: Quantity,
}

/// Per-instrument Greeks. Sensitivities are fixed-point integers scaled by
/// 10^6; delta is carried as exact signed notional paise. Synthetic until a
/// strike/vol surface and derivatives products are authorized.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Greeks {
    pub delta_notional_paise: i64,
    pub gamma_scaled: i64,
    pub vega_scaled: i64,
    pub theta_scaled: i64,
}

/// Synthetic short-borrow evidence: available lots and exact financing cost.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BorrowAvailability {
    pub available_quantity: Quantity,
    pub financing_cost_paise: Paise,
}

/// Canonical SHA-256 binding of a permit identity to the exact order terms it
/// authorized. Both the authority (at issuance) and the OMS (at `create`)
/// compute this same value; a tampered intent or permit field cannot match.
///
/// The preimage names the reservation identity fields that the OMS carries in
/// `PermitRef` plus the complete `OrderIntent`. The risk assessment is not
/// included because the OMS does not depend on the authority by design: the
/// policy version is the typed proxy for the assessment context. This is an
/// integrity digest only — no key, signature, KMS identity or authenticity is
/// claimed.
#[must_use]
pub fn permit_digest(
    permit_id: u64,
    policy_version: &str,
    snapshot_revision: u64,
    intent: &OrderIntent,
) -> String {
    let notional = i128::from(intent.limit_price.get().0) * i128::from(intent.quantity.get());
    let preimage = serde_json::json!({
        "domain": "nirnay.permit.intent.v1",
        "permit_id": permit_id,
        "policy_version": policy_version,
        "reservation": {
            "permit_id": permit_id,
            "intent_id": intent.intent_id,
            "account_id": intent.account_id,
            "strategy_id": intent.strategy_id,
            "instrument_id": intent.instrument_id,
            "venue": intent.venue,
            "side": intent.side,
            "quantity": intent.quantity,
            "reserved_notional_paise": notional,
            "snapshot_revision": snapshot_revision,
        },
        "intent": intent,
    })
    .to_string();
    let raw = Sha256::digest(preimage.as_bytes());
    raw.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(all(test, feature = "market-shaped-tests"))]
mod tests {
    use super::permit_digest;
    use crate::{
        AccountId, AlgoId, InstrumentId, IntentId, JournalRef, LogicalNanos, OrderIntent, Paise,
        PricePaise, Product, Quantity, Side, StrategyId, Venue,
    };

    fn intent() -> OrderIntent {
        OrderIntent {
            intent_id: IntentId::new(1).expect("fixture intent id"),
            account_id: AccountId::new(1).expect("fixture account id"),
            strategy_id: StrategyId::new(2).expect("fixture strategy id"),
            strategy_version: "strategy-v1".to_owned(),
            algo_id: AlgoId::new(3).expect("fixture algo id"),
            instrument_id: InstrumentId::new("nse.cash.synidx").expect("fixture instrument id"),
            venue: Venue::Nse,
            side: Side::Buy,
            product: Product::Intraday,
            quantity: Quantity::new(5).expect("fixture quantity"),
            limit_price: PricePaise::new(Paise(1_000)).expect("fixture price"),
            reason: "deterministic digest fixture".to_owned(),
            evidence: vec![JournalRef {
                sequence: 1,
                hash_hex: "ab".repeat(32),
            }],
            expires_at: LogicalNanos::new(100_000),
        }
    }

    #[test]
    fn permit_digest_is_deterministic_and_binds_every_intent_field() {
        let baseline = permit_digest(7, "risk-v1", 3, &intent());
        assert_eq!(
            baseline,
            permit_digest(7, "risk-v1", 3, &intent()),
            "recomputation must be byte-identical"
        );
        assert_eq!(baseline.len(), 64, "digest must be SHA-256 hex");

        let mut changed = intent();
        changed.limit_price = PricePaise::new(Paise(1_001)).expect("changed price");
        assert_ne!(
            baseline,
            permit_digest(7, "risk-v1", 3, &changed),
            "a tampered price must change the digest"
        );

        let mut changed = intent();
        changed.reason.push('!');
        assert_ne!(
            baseline,
            permit_digest(7, "risk-v1", 3, &changed),
            "a tampered reason must change the digest"
        );

        assert_ne!(
            baseline,
            permit_digest(8, "risk-v1", 3, &intent()),
            "a different permit id must change the digest"
        );
        assert_ne!(
            baseline,
            permit_digest(7, "risk-v2", 3, &intent()),
            "a different policy version must change the digest"
        );
    }
}
