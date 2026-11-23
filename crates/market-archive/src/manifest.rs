use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{ArchiveError, StreamQuality};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamEntry {
    pub stream: String,
    pub quality: StreamQuality,
}

/// Content that defines one point-in-time dataset. `known_at_utc_nanos` is
/// distinct from `range_end_utc_nanos`: knowledge may arrive later than events.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestDraft {
    pub market: String,
    pub venue: String,
    pub universe_revision: String,
    pub instrument_revision: String,
    pub corporate_action_revision: String,
    pub calendar_revision: String,
    pub source_hashes: BTreeMap<String, String>,
    pub data_schema_version: u32,
    pub range_start_utc_nanos: i64,
    pub range_end_utc_nanos: i64,
    pub known_at_utc_nanos: i64,
    pub streams: Vec<StreamEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetManifest {
    pub dataset_id: String,
    pub created_at_utc_nanos: i64,
    #[serde(flatten)]
    pub content: ManifestDraft,
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

impl ManifestDraft {
    fn validate(&self) -> Result<(), ArchiveError> {
        let required_fields = [
            ("market", self.market.as_str()),
            ("venue", self.venue.as_str()),
            ("universe_revision", self.universe_revision.as_str()),
            ("instrument_revision", self.instrument_revision.as_str()),
            (
                "corporate_action_revision",
                self.corporate_action_revision.as_str(),
            ),
            ("calendar_revision", self.calendar_revision.as_str()),
        ];
        for (name, value) in required_fields {
            if value.trim().is_empty() {
                return Err(ArchiveError::InvalidManifest(format!("{name} is empty")));
            }
        }

        if self.range_start_utc_nanos >= self.range_end_utc_nanos {
            return Err(ArchiveError::InvalidManifest(
                "range_start must be before range_end".into(),
            ));
        }
        if self.known_at_utc_nanos < self.range_end_utc_nanos {
            return Err(ArchiveError::InvalidManifest(
                "known_at precedes the end of the covered range".into(),
            ));
        }
        if self.data_schema_version == 0 {
            return Err(ArchiveError::InvalidManifest(
                "data_schema_version must be positive".into(),
            ));
        }
        if self.source_hashes.is_empty() {
            return Err(ArchiveError::InvalidManifest(
                "source_hashes is empty".into(),
            ));
        }
        for (name, hash) in &self.source_hashes {
            let valid_hash = hash.len() == 64
                && hash
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
            if name.trim().is_empty() || !valid_hash {
                return Err(ArchiveError::InvalidManifest(format!(
                    "source `{name}` needs lowercase SHA-256"
                )));
            }
        }
        if self.streams.is_empty() {
            return Err(ArchiveError::InvalidManifest("streams is empty".into()));
        }

        let mut names: Vec<&str> = self
            .streams
            .iter()
            .map(|stream| stream.stream.as_str())
            .collect();
        names.sort_unstable();
        let has_empty_name = names.iter().any(|name| name.trim().is_empty());
        let has_duplicate_name = names.windows(2).any(|pair| pair[0] == pair[1]);
        if has_empty_name || has_duplicate_name {
            return Err(ArchiveError::InvalidManifest(
                "stream names must be non-empty and unique".into(),
            ));
        }
        if self
            .streams
            .iter()
            .any(|stream| !stream.quality.is_well_formed())
        {
            return Err(ArchiveError::InvalidManifest(
                "stream quality counters are inconsistent".into(),
            ));
        }
        Ok(())
    }

    fn canonical_id(&self) -> Result<String, ArchiveError> {
        let mut canonical = self.clone();
        canonical
            .streams
            .sort_by(|left, right| left.stream.cmp(&right.stream));
        let bytes = serde_json::to_vec(&canonical)
            .map_err(|error| ArchiveError::InvalidManifest(error.to_string()))?;
        Ok(sha256_hex(&bytes))
    }
}

impl DatasetManifest {
    pub fn seal(mut draft: ManifestDraft, created_at_utc_nanos: i64) -> Result<Self, ArchiveError> {
        draft.validate()?;
        if created_at_utc_nanos < draft.known_at_utc_nanos {
            return Err(ArchiveError::InvalidManifest(
                "created_at precedes known_at".into(),
            ));
        }
        draft
            .streams
            .sort_by(|left, right| left.stream.cmp(&right.stream));
        let dataset_id = draft.canonical_id()?;
        Ok(Self {
            dataset_id,
            created_at_utc_nanos,
            content: draft,
        })
    }

    pub fn verify(&self) -> Result<(), ArchiveError> {
        self.content.validate()?;
        if self.content.canonical_id()? == self.dataset_id {
            Ok(())
        } else {
            Err(ArchiveError::DatasetIdMismatch)
        }
    }

    pub fn require_research_grade(&self) -> Result<(), ArchiveError> {
        for stream in &self.content.streams {
            let quality = &stream.quality;
            if quality.observed == 0 || !quality.is_clean() {
                return Err(ArchiveError::NotResearchGrade(format!(
                    "{}: observed={}, gaps={}, missing={}, duplicates={}, out_of_order={}",
                    stream.stream,
                    quality.observed,
                    quality.gap_events,
                    quality.missing_total,
                    quality.duplicates,
                    quality.out_of_order
                )));
            }
        }
        Ok(())
    }
}

#[cfg(all(test, feature = "market-shaped-tests"))]
mod tests {
    use super::*;

    fn clean() -> StreamQuality {
        StreamQuality {
            observed: 10,
            first_sequence: Some(1),
            highest_sequence: Some(10),
            ..StreamQuality::default()
        }
    }

    fn draft() -> ManifestDraft {
        ManifestDraft {
            market: "india-cash".into(),
            venue: "NSE".into(),
            universe_revision: "universe-1".into(),
            instrument_revision: "instruments-1".into(),
            corporate_action_revision: "actions-1".into(),
            calendar_revision: "calendar-1".into(),
            source_hashes: BTreeMap::from([("session.jsonl".to_owned(), "a".repeat(64))]),
            data_schema_version: 1,
            range_start_utc_nanos: 100,
            range_end_utc_nanos: 200,
            known_at_utc_nanos: 250,
            streams: vec![StreamEntry {
                stream: "fyers/NSE:SBIN-EQ".into(),
                quality: clean(),
            }],
        }
    }

    #[test]
    fn content_hash_is_stable_and_verifiable() {
        let manifest = DatasetManifest::seal(draft(), 300).unwrap();
        manifest.verify().unwrap();
        assert_eq!(manifest.dataset_id.len(), 64);
        assert_eq!(
            manifest.dataset_id,
            DatasetManifest::seal(draft(), 999).unwrap().dataset_id
        );
    }

    #[test]
    fn content_mutation_is_detected() {
        let mut manifest = DatasetManifest::seal(draft(), 300).unwrap();
        manifest.content.venue = "BSE".into();
        assert!(matches!(
            manifest.verify(),
            Err(ArchiveError::DatasetIdMismatch)
        ));
    }

    #[test]
    fn knowledge_time_must_follow_covered_range() {
        let mut manifest_draft = draft();
        manifest_draft.known_at_utc_nanos = 199;
        assert!(DatasetManifest::seal(manifest_draft, 300).is_err());
    }

    #[test]
    fn schema_version_zero_is_rejected() {
        let mut manifest_draft = draft();
        manifest_draft.data_schema_version = 0;
        assert!(DatasetManifest::seal(manifest_draft, 300).is_err());
    }

    #[test]
    fn malformed_stream_quality_is_rejected() {
        let mut manifest_draft = draft();
        manifest_draft.streams[0].quality.first_sequence = None;
        assert!(DatasetManifest::seal(manifest_draft, 300).is_err());
    }

    #[test]
    fn research_grade_rejects_gap_streams() {
        let mut manifest_draft = draft();
        manifest_draft.streams[0].quality.gap_events = 1;
        manifest_draft.streams[0].quality.missing_total = 2;
        let manifest = DatasetManifest::seal(manifest_draft, 300).unwrap();
        assert!(matches!(
            manifest.require_research_grade(),
            Err(ArchiveError::NotResearchGrade(_))
        ));
    }
}
