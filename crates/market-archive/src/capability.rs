//! Machine-readable source-product capability contract.
//!
//! A product declares what it supplies. Missing capabilities are
//! rejections, never inferences. Rights and entitlements are recorded as
//! verification state only; this module never grants or upgrades them.

use std::path::Path;

use serde::{
    Deserialize, //
    Serialize,
};
use thiserror::Error;

const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CapabilityError {
    #[error("registry has schema version {0}, expected 1")]
    UnsupportedSchema(u32),
    #[error("registry could not be read")]
    Unreadable,
    #[error("registry is invalid: {0}")]
    InvalidRegistry(String),
    #[error("no declared product for provider `{0}`, product `{1}`, venue `{2}`, segment `{3}`")]
    UnknownProduct(String, String, String, String),
    #[error("product supplies {declared} levels; requirement needs {required}")]
    InsufficientDepth { declared: u32, required: u32 },
    #[error("product depth model is {declared:?}; requirement needs {required:?}")]
    DepthModelMismatch {
        declared: DepthModel,
        required: DepthModel,
    },
    #[error("product does not supply trade prints")]
    NoTradePrints,
    #[error("product does not supply a clean snapshot")]
    NoSnapshot,
    #[error("product declares no source sequence continuity")]
    NoSourceContinuity,
    #[error("product does not grant the verified right to {0}")]
    MissingRight(String),
    #[error("product entitlement `{0}` has not been verified")]
    EntitlementUnverified(String),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DepthModel {
    /// Aggregated price levels with a displayed order count.
    Mbp,
    /// Individual orders with identities and lifecycle events.
    Mbo,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateMode {
    SnapshotOnly,
    Delta,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceSequence {
    Unavailable,
    Connection,
    ProviderInstrument,
    Channel,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryMode {
    Reconnect,
    FreshSnapshot,
    Retransmission,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolRef {
    pub name: String,
    pub schema_digest: Option<String>,
    pub schema_reference: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntitlementRef {
    pub reference: String,
    pub verified: bool,
}

/// Permission state for one data-right. UNKNOWN never grants a right.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum PermissionState {
    Unknown,
    Permitted,
    Prohibited,
}

impl PermissionState {
    pub fn is_permitted(self) -> bool {
        self == Self::Permitted
    }
}

/// Where a permission claim comes from. Owner reports are not written evidence.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSource {
    OwnerReported,
    ProviderAgreement,
    ExchangeRules,
    InternalPolicy,
    None,
}

/// Exact identity of a declared source product. A partial identity never
/// authorizes an operation: venue and segment are never inferred.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductKey {
    pub provider: String,
    pub product: String,
    pub venue: String,
    pub segment: String,
}

impl ProductKey {
    pub fn new(
        provider: impl Into<String>,
        product: impl Into<String>,
        venue: impl Into<String>,
        segment: impl Into<String>,
    ) -> Self {
        Self {
            provider: provider.into(),
            product: product.into(),
            venue: venue.into(),
            segment: segment.into(),
        }
    }
}

/// A data-use operation. The vocabulary matches the seven written-evidence
/// classes in the registry rights review.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DataOperation {
    Capture,
    Store,
    Replay,
    Backup,
    InternalAnalytics,
    ExternalRedistribution,
    Retention,
}

impl DataOperation {
    /// All operations, in the canonical registry order. Use this for exhaustive
    /// fail-closed checks so a newly added operation cannot be skipped.
    pub const ALL: [Self; 7] = [
        Self::Capture,
        Self::Store,
        Self::Replay,
        Self::Backup,
        Self::InternalAnalytics,
        Self::ExternalRedistribution,
        Self::Retention,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Capture => "capture",
            Self::Store => "store",
            Self::Replay => "replay",
            Self::Backup => "backup",
            Self::InternalAnalytics => "internal_analytics",
            Self::ExternalRedistribution => "external_redistribution",
            Self::Retention => "retention",
        }
    }

    /// Persistence operations additionally require a verified entitlement.
    pub fn is_persistence(self) -> bool {
        matches!(
            self,
            Self::Store | Self::Replay | Self::Backup | Self::Retention
        )
    }
}

/// One permission with its evidence provenance and review scope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RightEvidence {
    pub state: PermissionState,
    pub source: EvidenceSource,
    pub document_reference: String,
    pub scope: String,
    pub review_date: String,
}

/// A concrete retention duration with provenance. A missing duration is an
/// explicit absence of a written retention policy; it never grants retention.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionPeriod {
    /// Days, when a written policy supplies one. `None` is not "unlimited".
    pub duration_days: Option<u32>,
    pub basis: EvidenceSource,
    pub document_reference: String,
    pub review_date: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataRights {
    pub capture: RightEvidence,
    pub store: RightEvidence,
    pub replay: RightEvidence,
    pub backup: RightEvidence,
    pub internal_analytics: RightEvidence,
    /// Formerly `redistribute`; renamed to match the written-evidence class.
    pub external_redistribution: RightEvidence,
    pub retention: RightEvidence,
    pub retention_period: RetentionPeriod,
}

impl DataRights {
    pub fn right_for(&self, operation: DataOperation) -> &RightEvidence {
        match operation {
            DataOperation::Capture => &self.capture,
            DataOperation::Store => &self.store,
            DataOperation::Replay => &self.replay,
            DataOperation::Backup => &self.backup,
            DataOperation::InternalAnalytics => &self.internal_analytics,
            DataOperation::ExternalRedistribution => &self.external_redistribution,
            DataOperation::Retention => &self.retention,
        }
    }
}

/// Written-evidence review status for the whole registry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RightsReview {
    pub status: String,
    pub review_date: String,
    pub required_evidence: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceLimits {
    pub max_symbols: u32,
    pub connections: u32,
    pub requests_per_second: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceProduct {
    pub provider: String,
    pub product: String,
    pub venue: String,
    pub segment: String,
    pub instrument_scope: String,
    pub depth_model: DepthModel,
    pub depth_levels: u32,
    pub trade_prints: bool,
    pub order_level_identity: bool,
    pub snapshot: bool,
    pub incremental_updates: UpdateMode,
    pub source_sequence: SourceSequence,
    pub recovery: Vec<RecoveryMode>,
    pub protocol: ProtocolRef,
    pub entitlement: EntitlementRef,
    pub rights: DataRights,
    pub limits: SourceLimits,
}

impl SourceProduct {
    pub fn key(&self) -> ProductKey {
        ProductKey::new(
            self.provider.clone(),
            self.product.clone(),
            self.venue.clone(),
            self.segment.clone(),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceProductRegistry {
    pub schema_version: u32,
    pub registry_id: String,
    pub rights_review: RightsReview,
    pub products: Vec<SourceProduct>,
}

/// What a consumer requires before it may activate on a feed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarketDataRequirement {
    pub provider: String,
    pub product: String,
    pub venue: String,
    pub segment: String,
    pub depth_model: DepthModel,
    pub min_depth: u32,
    pub requires_trade_prints: bool,
    pub requires_clean_snapshot: bool,
    pub requires_source_continuity: bool,
    pub requires_capture: bool,
    pub requires_store: bool,
    pub requires_replay: bool,
    pub requires_backup: bool,
    pub requires_internal_analytics: bool,
    pub requires_external_redistribution: bool,
    pub requires_retention: bool,
}

impl SourceProductRegistry {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, CapabilityError> {
        let bytes = std::fs::read(path).map_err(|_| CapabilityError::Unreadable)?;
        Self::from_bytes(&bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CapabilityError> {
        let registry: Self = serde_json::from_slice(bytes)
            .map_err(|error| CapabilityError::InvalidRegistry(error.to_string()))?;
        if registry.schema_version != SCHEMA_VERSION {
            return Err(CapabilityError::UnsupportedSchema(registry.schema_version));
        }
        if registry.registry_id.trim().is_empty() || registry.products.is_empty() {
            return Err(CapabilityError::InvalidRegistry(
                "registry id and at least one product are required".to_owned(),
            ));
        }
        Ok(registry)
    }

    /// Find the exact declared product. No venue or segment inference.
    pub fn product(
        &self,
        provider: &str,
        product: &str,
        venue: &str,
        segment: &str,
    ) -> Result<&SourceProduct, CapabilityError> {
        self.products
            .iter()
            .find(|candidate| {
                candidate.provider == provider
                    && candidate.product == product
                    && candidate.venue == venue
                    && candidate.segment == segment
            })
            .ok_or_else(|| {
                CapabilityError::UnknownProduct(
                    provider.to_owned(),
                    product.to_owned(),
                    venue.to_owned(),
                    segment.to_owned(),
                )
            })
    }

    /// The single fail-closed authorization helper. `UNKNOWN`, `PROHIBITED`
    /// and undeclared products are all denials; no caller may infer a right
    /// from a successful connection or from a partial product identity.
    pub fn authorize(
        &self,
        product_key: &ProductKey,
        operation: DataOperation,
    ) -> Result<(), CapabilityError> {
        let declared = self.product(
            &product_key.provider,
            &product_key.product,
            &product_key.venue,
            &product_key.segment,
        )?;
        let right = declared.rights.right_for(operation);
        if !right.state.is_permitted() {
            return Err(CapabilityError::MissingRight(operation.as_str().to_owned()));
        }
        if operation.is_persistence() && !declared.entitlement.verified {
            return Err(CapabilityError::EntitlementUnverified(
                declared.entitlement.reference.clone(),
            ));
        }
        Ok(())
    }

    /// Mirror the schema's cross-field coherence rules at runtime. A hand-edited
    /// registry must not become an internally inconsistent contract.
    fn check_coherence(&self, declared: &SourceProduct) -> Result<(), CapabilityError> {
        let incoherent = (declared.incremental_updates == UpdateMode::SnapshotOnly
            && !declared.snapshot)
            || (declared.recovery.contains(&RecoveryMode::FreshSnapshot) && !declared.snapshot)
            || (declared.depth_model == DepthModel::Mbo && !declared.order_level_identity);
        if incoherent {
            return Err(CapabilityError::InvalidRegistry(format!(
                "product `{}/{}` violates the schema coherence rules",
                declared.provider, declared.product
            )));
        }
        Ok(())
    }

    /// Reject the requirement unless every field is satisfied by the declared
    /// product. A five-level feed never satisfies a twenty-level requirement.
    pub fn evaluate(&self, requirement: &MarketDataRequirement) -> Result<(), CapabilityError> {
        let declared = self.product(
            &requirement.provider,
            &requirement.product,
            &requirement.venue,
            &requirement.segment,
        )?;
        self.check_coherence(declared)?;
        if declared.depth_model != requirement.depth_model {
            return Err(CapabilityError::DepthModelMismatch {
                declared: declared.depth_model,
                required: requirement.depth_model,
            });
        }
        if declared.depth_levels < requirement.min_depth {
            return Err(CapabilityError::InsufficientDepth {
                declared: declared.depth_levels,
                required: requirement.min_depth,
            });
        }
        if requirement.requires_trade_prints && !declared.trade_prints {
            return Err(CapabilityError::NoTradePrints);
        }
        if requirement.requires_clean_snapshot && !declared.snapshot {
            return Err(CapabilityError::NoSnapshot);
        }
        if requirement.requires_source_continuity
            && declared.source_sequence == SourceSequence::Unavailable
        {
            return Err(CapabilityError::NoSourceContinuity);
        }
        let key = declared.key();
        let required_operations = [
            (requirement.requires_capture, DataOperation::Capture),
            (requirement.requires_store, DataOperation::Store),
            (requirement.requires_replay, DataOperation::Replay),
            (requirement.requires_backup, DataOperation::Backup),
            (
                requirement.requires_internal_analytics,
                DataOperation::InternalAnalytics,
            ),
            (
                requirement.requires_external_redistribution,
                DataOperation::ExternalRedistribution,
            ),
            (requirement.requires_retention, DataOperation::Retention),
        ];
        for (needed, operation) in required_operations {
            if needed {
                self.authorize(&key, operation)?;
            }
        }
        Ok(())
    }
}
