//! Content-addressed identity for capture frames, observations and business
//! events.
//!
//! Identity is derived only from stable inputs. Locally assigned, reset-prone
//! metadata (capture counters, connection epochs, receive times and sequence
//! counters) is excluded by design, so a reconnect or replay cannot change
//! an identity. This implements ADR-M1-005 D-005.1 and D-005.2.

use std::fmt;

use serde::{
    Deserialize, //
    Serialize,
};
use sha2::{
    Digest, //
    Sha256,
};

use crate::error::ContractError;

/// Lowercase SHA-256 hex of a canonical identity preimage.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ContentId(String);

impl ContentId {
    pub const HEX_LEN: usize = 64;

    /// Hash the exact preimage bytes. No fallback on empty input.
    pub fn from_preimage(preimage: &[u8]) -> Result<Self, ContractError> {
        if preimage.is_empty() {
            return Err(ContractError::EmptyIdentityPreimage);
        }
        let digest = Sha256::digest(preimage);
        let mut text = String::with_capacity(Self::HEX_LEN);
        for byte in digest {
            use std::fmt::Write as _;
            let _ = write!(text, "{byte:02x}");
        }
        Ok(Self(text))
    }

    pub fn parse(value: impl AsRef<str>) -> Result<Self, ContractError> {
        let value = value.as_ref();
        if value.len() != Self::HEX_LEN
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(ContractError::InvalidContentId(value.to_owned()));
        }
        Ok(Self(value.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ContentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl TryFrom<String> for ContentId {
    type Error = ContractError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<ContentId> for String {
    fn from(value: ContentId) -> Self {
        value.0
    }
}

/// Kind of producer that originated an observation (ADR-M1-005 D-005.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OriginClass {
    /// Exchange-native event. No currently connected source qualifies.
    ExchangeEvent,
    /// Provider SDK-decoded callback.
    SdkCallback,
    /// Original raw wire frame at the capture layer.
    RawWireFrame,
    /// Local receive act, represented only by receive timestamps.
    LocalReceive,
    /// Derived observation inferred from another observation.
    Reconstructed,
}

impl OriginClass {
    /// Exchange events require a source that can establish exchange identity.
    #[must_use]
    pub const fn can_be_exchange_event(&self) -> bool {
        matches!(self, Self::ExchangeEvent)
    }
}

/// Where a sequence value is meaningful (ADR-M1-006 ordering scopes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SequenceScope {
    /// No source sequence exists; only local receive order is available.
    None,
    /// Sequence resets per connection generation.
    Connection,
    /// Sequence is provider-per-instrument and connection scoped.
    ProviderInstrument,
    /// Sequence is meaningful across a whole venue channel.
    Channel,
}

/// Compute a capture-frame identity from the exact original payload bytes.
pub fn capture_frame_id(payload: &[u8]) -> Result<ContentId, ContractError> {
    ContentId::from_preimage(&canonical_bytes("capture_frame_v1", &[], payload))
}

/// Compute an observation identity. Excludes every local, reset-prone field.
pub fn observation_id(
    provider: &str,
    source_scope: &str,
    provider_instrument_id: &str,
    canonical_payload: &[u8],
) -> Result<ContentId, ContractError> {
    let parts = [provider, source_scope, provider_instrument_id];
    ContentId::from_preimage(&canonical_bytes(
        "observation_v1",
        &parts,
        canonical_payload,
    ))
}

/// Compute a business-event identity from an observation and a domain
/// discriminator. Callers must not invent one for a bare state observation;
/// use `KernelEvent::observation_only` from the event module.
pub fn business_event_id(
    observation: &ContentId,
    event_kind: &str,
    domain_discriminator: &str,
) -> Result<ContentId, ContractError> {
    let parts = [observation.as_str(), event_kind, domain_discriminator];
    ContentId::from_preimage(&canonical_bytes("business_event_v1", &parts, &[]))
}

/// Length-prefixed canonical preimage: no ambiguity between adjacent fields.
fn canonical_bytes(label: &str, parts: &[&str], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    push_part(&mut out, label.as_bytes());
    out.push(u8::try_from(parts.len()).unwrap_or(u8::MAX));
    for part in parts {
        push_part(&mut out, part.as_bytes());
    }
    push_part(&mut out, payload);
    out
}

fn push_part(out: &mut Vec<u8>, bytes: &[u8]) {
    let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(bytes);
}
