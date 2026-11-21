//! BSE security-master parser with versioned settlement classification.
//!
//! Trading eligibility is never inferred from an unknown group. The group
//! classification is selected by the declared source version and must be
//! validated against actual dated files before that version is supported.

use trading_core::{
    Isin, //
    ListingStatus,
    SettlementRegime,
};

use crate::csv::{
    Limits, //
    column,
    parse_document,
};
use crate::observation::{
    Exclusion, //
    Rejection,
    SnapshotScope,
    SourceBatch,
};

/// Settlement classification for a BSE group in one source version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BseGroupClass {
    pub settlement_regime: SettlementRegime,
    pub z_category: bool,
}

/// Versioned group classification. Unknown source versions are rejected.
pub fn bse_group_classes(
    source_version: &str,
) -> Result<Vec<(&'static str, BseGroupClass)>, String> {
    match source_version {
        "bse-security-master-v1" | "bse-security-master-v2" => Ok(known_group_classes()),
        other => Err(format!("unsupported BSE source version {other}")),
    }
}

/// The group-to-regime mapping shared by both declared source versions.
/// Groups outside this table are never inferred from a row: the caller records
/// them as rejections so an unknown settlement regime cannot gain eligibility.
fn known_group_classes() -> Vec<(&'static str, BseGroupClass)> {
    let trade = ["T", "MT", "TS", "IT", "Z", "R", "ZP", "P", "Y", "ZY", "XT"];
    let standard = ["A", "B", "M", "MS", "S", "X", "XC", "XD"];
    let mut rows = Vec::new();
    for group in trade {
        rows.push((
            group,
            BseGroupClass {
                settlement_regime: SettlementRegime::TradeForTrade,
                z_category: matches!(group, "Z" | "ZP" | "ZY"),
            },
        ));
    }
    for group in standard {
        rows.push((
            group,
            BseGroupClass {
                settlement_regime: SettlementRegime::Standard,
                z_category: false,
            },
        ));
    }
    rows
}

/// One accepted BSE cash-market record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BseCashRow {
    pub line: usize,
    pub scrip_code: String,
    pub symbol: String,
    pub name: String,
    pub isin: Isin,
    pub settlement_group: String,
    pub group_class: BseGroupClass,
    pub tick_paise: i64,
    pub market_lot: i64,
    pub status: ListingStatus,
}

/// Column layout for one declared BSE source version.
///
/// The standardised file replaces the legacy names with ISO 20022 tags. The
/// mapping is fixed by BSE's `SCRIP_STANDARDISED_FORMAT.docx` shipped with the
/// dated source; no column is guessed from its position or sample value.
struct BseLayout {
    scrip: &'static str,
    symbol: &'static str,
    name: &'static str,
    isin: &'static [&'static str],
    tick: &'static [&'static str],
    lot: &'static str,
    type_flag: &'static [&'static str],
    group: &'static str,
    status: &'static str,
    /// Only the standardised contract declares suspend/inactive as exclusions.
    standardised: bool,
}

fn layout_for_version(source_version: &str) -> Result<BseLayout, String> {
    match source_version {
        "bse-security-master-v1" => Ok(BseLayout {
            scrip: "SCRIP CODE",
            symbol: "INSTRUMENT CODE",
            name: "SCRIP NAME",
            isin: &["ISIN CODE", "ISIN"],
            tick: &["NEW TICK SIZE"],
            lot: "MARKET LOT",
            type_flag: &["SECURITY TYPE FLAG", "SECURITY TYPE", "SECURITY_TYPE_FLAG"],
            group: "GROUP NAME",
            status: "STATUS",
            standardised: false,
        }),
        "bse-security-master-v2" => Ok(BseLayout {
            scrip: "FININSTRMID",
            symbol: "TCKRSYMB",
            name: "FININSTRMNM",
            isin: &["ISIN"],
            // Field 58 (TickSz) is the authoritative tick size in BSE's
            // standardised contract. Field 9 (BidIntrvl) is the semantic
            // successor of v1's NEW TICK SIZE and is kept only as a fallback;
            // in the dated fixture the two agree on every EQ row.
            tick: &["TICKSZ", "BIDINTRVL"],
            lot: "NEWBRDLOTQTY",
            type_flag: &["SCTYTPFLG"],
            group: "SCTYSRS",
            status: "STS",
            standardised: true,
        }),
        other => Err(format!("unsupported BSE source version {other}")),
    }
}

/// Parse BSE security-master bytes under a declared source version.
pub fn parse_bse_master(
    bytes: &[u8],
    source_version: &str,
    scope: SnapshotScope,
    limits: Limits,
) -> Result<SourceBatch<BseCashRow>, String> {
    let classes = bse_group_classes(source_version)?;
    let layout = layout_for_version(source_version)?;
    let document = parse_document(bytes, limits).map_err(|error| error.to_string())?;
    let scrip_column = column(&document.header, &[layout.scrip])?;
    let symbol_column = column(&document.header, &[layout.symbol])?;
    let name_column = column(&document.header, &[layout.name])?;
    let isin_column = column(&document.header, layout.isin)?;
    let tick_column = column(&document.header, layout.tick)?;
    let lot_column = column(&document.header, &[layout.lot])?;
    let type_column = column(&document.header, layout.type_flag)?;
    let group_column = column(&document.header, &[layout.group])?;
    let status_column = column(&document.header, &[layout.status])?;

    let mut batch = SourceBatch {
        source_id: "bse.cash_security_master".to_owned(),
        source_version: source_version.to_owned(),
        scope,
        accepted: Vec::new(),
        excluded: Vec::new(),
        rejected: Vec::new(),
        physical_rows: document.rows.len(),
    };
    let mut scrips = std::collections::BTreeSet::new();
    let mut isins = std::collections::BTreeSet::new();
    for row in document.rows {
        let line = row.line;
        let scrip_code = row.fields[scrip_column].trim().to_owned();
        let symbol = row.fields[symbol_column].trim().to_owned();
        if scrip_code.is_empty() || symbol.is_empty() {
            batch.rejected.push(Rejection {
                line,
                reason: "empty scrip code or security id".to_owned(),
            });
            continue;
        }
        if !row.fields[type_column].trim().eq_ignore_ascii_case("EQ") {
            batch.excluded.push(Exclusion {
                line,
                rule: format!(
                    "security type {} outside bse_cash scope",
                    row.fields[type_column].trim()
                ),
            });
            continue;
        }
        let group = row.fields[group_column].trim().to_uppercase();
        let group_class = match classes.iter().find(|(name, _)| *name == group) {
            Some((_, class)) => *class,
            None => {
                batch.rejected.push(Rejection {
                    line,
                    reason: format!("unclassified BSE group {group}; eligibility refused"),
                });
                continue;
            }
        };
        let isin_text = row.fields[isin_column].trim();
        let isin = match Isin::new(isin_text) {
            Ok(value) => value,
            Err(error) => {
                batch.rejected.push(Rejection {
                    line,
                    reason: format!("invalid ISIN {isin_text}: {error}"),
                });
                continue;
            }
        };
        let name = row.fields[name_column].trim().to_owned();
        if name.is_empty() {
            batch.rejected.push(Rejection {
                line,
                reason: "empty security name; no fallback identity is inferred".to_owned(),
            });
            continue;
        }
        let tick_paise = match market_types::fixed::parse_scaled(row.fields[tick_column].trim(), 2)
        {
            Ok(value) if value > 0 => value,
            _ => {
                batch.rejected.push(Rejection {
                    line,
                    reason: "tick size is not a positive paise value".to_owned(),
                });
                continue;
            }
        };
        let market_lot = match row.fields[lot_column].trim().parse::<i64>() {
            Ok(value) if value > 0 => value,
            _ => {
                batch.rejected.push(Rejection {
                    line,
                    reason: "market lot is not a positive integer".to_owned(),
                });
                continue;
            }
        };
        let status_text = row.fields[status_column].trim().to_uppercase();
        let status = match status_text.as_str() {
            "A" | "ACTIVE" => ListingStatus::Active,
            // BSE's standardised contract declares S (suspend) and I (inactive)  # prose-audit-ignore: BSE ticker symbol identifier
            // as real, non-tradable states. They are excluded by that declared
            // rule, not silently treated as active and never inferred as errors.
            "S" | "SUSPEND" | "I" | "INACTIVE" if layout.standardised => {
                batch.excluded.push(Exclusion {
                    line,
                    rule: format!("instrument status {status_text} is not active"),
                });
                continue;
            }
            other => {
                batch.rejected.push(Rejection {
                    line,
                    reason: format!("unsupported instrument status {other}"),
                });
                continue;
            }
        };
        let isin_key = isin.as_str().to_owned();
        if scrips.contains(&scrip_code) || isins.contains(&isin_key) {
            batch.rejected.push(Rejection {
                line,
                reason: format!("duplicate scrip code or ISIN for {scrip_code} / {isin_key}"),
            });
            continue;
        }
        scrips.insert(scrip_code.clone());
        isins.insert(isin_key);
        batch.accepted.push(BseCashRow {
            line,
            scrip_code,
            symbol,
            name,
            isin,
            settlement_group: group,
            group_class,
            tick_paise,
            market_lot,
            status,
        });
    }
    Ok(batch)
}
