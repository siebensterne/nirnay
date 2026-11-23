use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ArchiveError;

/// Maximum serialized journal record size in bytes, excluding the newline.
///
/// This is an application-level contract for the v1 line-oriented journal, not
/// a claim about any provider's maximum message size. Larger payloads require
/// a chunked/object-payload path. The Upstox recorder uses the shared
/// [`MAX_PAYLOAD_BYTES`] limit, so a frame rejected here is a terminal capture
/// outcome rather than a retryable network error.
pub const MAX_RAW_RECORD_BYTES: usize = 1024 * 1024;

/// Maximum decoded payload bytes. Base64 expands this to at most
/// `MAX_PAYLOAD_BYTES.div_ceil(3) * 4` bytes before serialization.
pub const MAX_PAYLOAD_BYTES: usize = 512 * 1024;

/// Maximum length of each textual metadata field. Derived from the record
/// bound minus the encoded payload and a fixed allowance for numeric fields,
/// schema overhead and JSON escaping. The 1 KiB allowance is a heuristic, not
/// a proof: the authoritative aggregate bound is the serialized-length check
/// in `to_json_line`, `from_json_line` and the journal scanner.
pub const MAX_FIELD_BYTES: usize =
    MAX_RAW_RECORD_BYTES - (MAX_PAYLOAD_BYTES.div_ceil(3) * 4) - 1024;

const fn encoded_payload_limit() -> usize {
    MAX_PAYLOAD_BYTES.div_ceil(3) * 4
}

/// An original provider frame, before normalization.
///
/// `entitlement_ref` is a non-secret reference to an internal entitlement
/// record. Never store API keys, access tokens, account numbers or signed URLs
/// in this structure. Payload bytes are preserved byte-for-byte in Base64.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawFrame {
    pub schema_version: u32,
    pub source: String,
    pub provider_instrument_id: String,
    pub received_utc_nanos: i64,
    pub received_monotonic_nanos: u64,
    pub exchange_utc_nanos: Option<i64>,
    pub capture_sequence: Option<u64>,
    pub entitlement_ref: String,
    pub payload_base64: String,
    pub payload_sha256: String,
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn nonempty(value: &str, field: &'static str) -> Result<(), ArchiveError> {
    if value.trim().is_empty() {
        return Err(ArchiveError::EmptyField(field));
    }
    if value.len() > MAX_FIELD_BYTES {
        return Err(ArchiveError::FieldTooLarge {
            field,
            size: value.len(),
            limit: MAX_FIELD_BYTES,
        });
    }
    Ok(())
}

impl RawFrame {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source: &str,
        provider_instrument_id: &str,
        received_utc_nanos: i64,
        received_monotonic_nanos: u64,
        exchange_utc_nanos: Option<i64>,
        capture_sequence: Option<u64>,
        entitlement_ref: &str,
        payload: &[u8],
    ) -> Result<Self, ArchiveError> {
        nonempty(source, "source")?;
        nonempty(provider_instrument_id, "provider_instrument_id")?;
        nonempty(entitlement_ref, "entitlement_ref")?;
        if payload.len() > MAX_PAYLOAD_BYTES {
            return Err(ArchiveError::RecordTooLarge {
                size: payload.len(),
                limit: MAX_PAYLOAD_BYTES,
            });
        }
        Ok(Self {
            schema_version: 1,
            source: source.to_owned(),
            provider_instrument_id: provider_instrument_id.to_owned(),
            received_utc_nanos,
            received_monotonic_nanos,
            exchange_utc_nanos,
            capture_sequence,
            entitlement_ref: entitlement_ref.to_owned(),
            payload_base64: STANDARD.encode(payload),
            payload_sha256: sha256_hex(payload),
        })
    }

    pub fn payload(&self) -> Result<Vec<u8>, ArchiveError> {
        if self.payload_base64.len() > encoded_payload_limit() {
            return Err(ArchiveError::RecordTooLarge {
                size: self.payload_base64.len(),
                limit: encoded_payload_limit(),
            });
        }
        let bytes = STANDARD
            .decode(&self.payload_base64)
            .map_err(|_| ArchiveError::PayloadEncoding)?;
        if bytes.len() > MAX_PAYLOAD_BYTES {
            return Err(ArchiveError::RecordTooLarge {
                size: bytes.len(),
                limit: MAX_PAYLOAD_BYTES,
            });
        }
        if sha256_hex(&bytes) != self.payload_sha256 {
            return Err(ArchiveError::PayloadHashMismatch);
        }
        Ok(bytes)
    }

    pub fn validate(&self) -> Result<(), ArchiveError> {
        nonempty(&self.source, "source")?;
        nonempty(&self.provider_instrument_id, "provider_instrument_id")?;
        nonempty(&self.entitlement_ref, "entitlement_ref")?;
        if self.schema_version != 1 {
            return Err(ArchiveError::MalformedRecord(format!(
                "unsupported schema_version {}",
                self.schema_version
            )));
        }
        let _ = self.payload()?;
        Ok(())
    }

    pub fn to_json_line(&self) -> Result<String, ArchiveError> {
        self.validate()?;
        let line = serde_json::to_string(self)
            .map_err(|error| ArchiveError::MalformedRecord(error.to_string()))?;
        if line.len() > MAX_RAW_RECORD_BYTES {
            return Err(ArchiveError::RecordTooLarge {
                size: line.len(),
                limit: MAX_RAW_RECORD_BYTES,
            });
        }
        Ok(line)
    }

    pub fn from_json_line(line: &str) -> Result<Self, ArchiveError> {
        if line.len() > MAX_RAW_RECORD_BYTES {
            return Err(ArchiveError::RecordTooLarge {
                size: line.len(),
                limit: MAX_RAW_RECORD_BYTES,
            });
        }
        let frame: Self = serde_json::from_str(line)
            .map_err(|error| ArchiveError::MalformedRecord(error.to_string()))?;
        frame.validate()?;
        Ok(frame)
    }

    #[must_use]
    pub fn stream_key(&self) -> String {
        format!("{}/{}", self.source, self.provider_instrument_id)
    }
}

#[cfg(all(test, feature = "market-shaped-tests"))]
mod tests {
    use super::*;

    fn sample(payload: &[u8]) -> RawFrame {
        RawFrame::new(
            "fyers",
            "NSE:SBIN-EQ",
            1_700_000_000_000_000_000,
            12,
            Some(1_699_999_999_000_000_000),
            Some(5),
            "entitlement-001",
            payload,
        )
        .unwrap()
    }

    #[test]
    fn preserves_arbitrary_payload_bytes() {
        let frame = sample(&[0, 1, 127, 128, 255]);
        let line = frame.to_json_line().unwrap();
        let restored = RawFrame::from_json_line(&line).unwrap();
        assert_eq!(restored, frame);
        assert_eq!(restored.payload().unwrap(), vec![0, 1, 127, 128, 255]);
    }

    #[test]
    fn sha256_matches_known_vector() {
        assert_eq!(
            sample(b"abc").payload_sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn tampering_is_detected() {
        let mut frame = sample(b"abc");
        frame.payload_base64 = STANDARD.encode(b"abd");
        assert!(matches!(
            frame.validate(),
            Err(ArchiveError::PayloadHashMismatch)
        ));
    }

    #[test]
    fn malformed_base64_is_rejected() {
        let mut frame = sample(b"abc");
        frame.payload_base64 = "not base64!".to_owned();
        assert!(matches!(
            frame.validate(),
            Err(ArchiveError::PayloadEncoding)
        ));
    }

    #[test]
    fn identity_fields_are_required() {
        assert!(matches!(
            RawFrame::new("", "X", 0, 0, None, None, "e", b"x"),
            Err(ArchiveError::EmptyField("source"))
        ));
        assert!(matches!(
            RawFrame::new("f", "X", 0, 0, None, None, "", b"x"),
            Err(ArchiveError::EmptyField("entitlement_ref"))
        ));
    }
}
