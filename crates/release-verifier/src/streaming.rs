// SPDX-License-Identifier: Apache-2.0
//! Bounded, one-record-at-a-time streaming verification for one artifact.
//!
//! The pipeline holds a fixed stored chunk, a fixed decoder output chunk and a
//! reusable record line buffer. Stored, expanded and dataset SHA-256 values are
//! updated as bytes pass through. The only per-artifact state retained after
//! EOF is fixed-size: three counters, two digests, one predecessor ordering key
//! and an envelope-agreement flag. No whole-artifact byte vector and no record
//! collection is created anywhere in this module.

use crate::canonical_wire;
use crate::canonical_wire::ArtifactRecordOrderProfile;
use crate::canonical_wire::DerivedOrderKey;
use crate::error::VerifierError;
use crate::files::FileFacts;
use crate::files::SnapshotBuilder;
use crate::files::hex_lower;
use crate::files::parse_hex_lower;
use crate::jcs::canonical_bytes;
use crate::jcs::json_depth;
use crate::jcs::parse_strict;
use crate::resource_profile::EffectiveLimits;
use crate::schema::ArtifactDescriptor;
use crate::schema::ReleaseRecord;
use flate2::write::GzDecoder;
use sha2::Digest;
use sha2::Sha256;
use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::io::Read;
use std::io::Write;
use std::rc::Rc;

const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

type FailureSlot = Rc<RefCell<Option<VerifierError>>>;

fn new_failure_slot() -> FailureSlot {
    Rc::new(RefCell::new(None))
}

fn reject(slot: &FailureSlot, error: VerifierError) -> std::io::Error {
    let mut held = slot.borrow_mut();
    if held.is_none() {
        *held = Some(error);
    }
    std::io::Error::other("bounded streaming verification refused")
}

fn decode_error(slot: &FailureSlot, error: std::io::Error) -> VerifierError {
    let mut held = slot.borrow_mut();
    held.take().unwrap_or_else(|| VerifierError::Decompression {
        detail: error.to_string(),
    })
}

/// Constant-size verification state retained for one artifact.
#[derive(Debug, Clone)]
pub struct ArtifactState {
    stored_digest: [u8; 32],
    expanded_digest: [u8; 32],
    stored_bytes: u64,
    expanded_bytes: u64,
    record_count: u64,
    previous_key: Option<DerivedOrderKey>,
    envelope_agrees: bool,
}

impl ArtifactState {
    /// Return the incremental stored digest.
    pub fn stored_digest(&self) -> &[u8; 32] {
        &self.stored_digest
    }

    /// Return the incremental expanded digest.
    pub fn expanded_digest(&self) -> &[u8; 32] {
        &self.expanded_digest
    }

    /// Return the observed stored byte count.
    pub fn stored_bytes(&self) -> u64 {
        self.stored_bytes
    }

    /// Return the observed expanded byte count.
    pub fn expanded_bytes(&self) -> u64 {
        self.expanded_bytes
    }

    /// Return the observed record count.
    pub fn record_count(&self) -> u64 {
        self.record_count
    }

    /// Return the envelope agreement flag consumed by downstream evidence.
    pub fn envelope_agrees(&self) -> bool {
        self.envelope_agrees
    }
}

fn digest_of(hasher: Sha256) -> [u8; 32] {
    let bytes = hasher.finalize();
    let mut out = [0_u8; 32];
    out.copy_from_slice(&bytes);
    out
}

/// Reader that emits fixed stored chunks and updates the stored digest.
struct StoredStream<'a> {
    file: &'a std::fs::File,
    facts: &'a FileFacts,
    declared: u64,
    maximum: u64,
    chunk: Vec<u8>,
    total: u64,
    hasher: Sha256,
}

impl<'a> StoredStream<'a> {
    fn new(
        file: &'a std::fs::File,
        facts: &'a FileFacts,
        declared: u64,
        maximum: u64,
        chunk_bytes: u64,
    ) -> Result<Self, VerifierError> {
        if declared == 0 || declared > maximum {
            return Err(VerifierError::ResourceLimit {
                limit: "max_stored_bytes_per_artifact".to_owned(),
            });
        }
        if facts.size != declared {
            return Err(VerifierError::ByteCount {
                detail: "fstat size differs from the declared stored byte count".to_owned(),
            });
        }
        Ok(Self {
            file,
            facts,
            declared,
            maximum,
            chunk: vec![0_u8; chunk_bytes as usize],
            total: 0,
            hasher: Sha256::new(),
        })
    }

    fn consume<F>(&mut self, mut emit: F) -> Result<([u8; 32], u64), VerifierError>
    where
        F: FnMut(&[u8]) -> Result<(), VerifierError>,
    {
        let Self {
            file,
            chunk,
            total,
            hasher,
            declared,
            maximum,
            facts,
        } = self;
        while *total < *declared {
            let allowance = (*declared - *total).min(chunk.len() as u64) as usize;
            let read = file
                .read(&mut chunk[..allowance])
                .map_err(|error| VerifierError::Io {
                    detail: format!("read bounded stored chunk: {}", error),
                })?;
            if read == 0 {
                return Err(VerifierError::ByteCount {
                    detail: "stored stream ended before the declared byte count".to_owned(),
                });
            }
            if *total + read as u64 > *maximum {
                return Err(VerifierError::ResourceLimit {
                    limit: "max_stored_bytes_per_artifact".to_owned(),
                });
            }
            hasher.update(&chunk[..read]);
            emit(&chunk[..read])?;
            *total += read as u64;
        }
        let after = crate::files::file_facts(file)?;
        if &after != *facts {
            return Err(VerifierError::ByteCount {
                detail: "stored descriptor metadata changed during the read".to_owned(),
            });
        }
        Ok((digest_of(hasher.clone()), *total))
    }
}

/// Sink that enforces the emission cap on decoder output before further work.
struct ExpandedSink<'a, 'b> {
    expanded: &'a mut Sha256,
    dataset: &'a mut Sha256,
    framer: &'a mut RecordFramer<'b>,
    cap: u64,
    total: u64,
    failure: &'a FailureSlot,
}

impl Write for ExpandedSink<'_, '_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let added = buf.len() as u64;
        if self.total.saturating_add(added) > self.cap {
            return Err(reject(
                self.failure,
                VerifierError::ResourceLimit {
                    limit: "max_decompression_budget_bytes".to_owned(),
                },
            ));
        }
        self.expanded.update(buf);
        self.dataset.update(buf);
        let failure = self.failure;
        let outcome = self
            .framer
            .push(buf)
            .map_err(|error| reject(failure, error));
        outcome?;
        self.total += added;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

trait RecordSink {
    fn accept(&mut self, line: &[u8], ordinal: u64) -> Result<(), VerifierError>;
}

/// Reusable line framer that emits one LF-terminated record at a time.
struct RecordFramer<'a> {
    line: Vec<u8>,
    max_record_bytes: u64,
    count: u64,
    max_records: u64,
    declared: u64,
    first: bool,
    sink: &'a mut dyn RecordSink,
}

impl<'a> RecordFramer<'a> {
    fn new(limits: &EffectiveLimits, declared: u64, sink: &'a mut dyn RecordSink) -> Self {
        Self {
            line: Vec::new(),
            max_record_bytes: limits.max_record_bytes(),
            count: 0,
            max_records: limits.max_records_per_artifact(),
            declared,
            first: true,
            sink,
        }
    }

    fn push(&mut self, chunk: &[u8]) -> Result<(), VerifierError> {
        for &byte in chunk {
            if byte == b'\r' {
                return Err(VerifierError::RecordStream {
                    detail: "canonical record stream contains a CR byte".to_owned(),
                });
            }
            if byte == b'\n' {
                if self.line.is_empty() {
                    return Err(VerifierError::RecordStream {
                        detail: "canonical record stream contains a blank line".to_owned(),
                    });
                }
                if self.first && self.line.starts_with(&BOM) {
                    return Err(VerifierError::RecordStream {
                        detail: "canonical record stream starts with a BOM".to_owned(),
                    });
                }
                self.first = false;
                self.count += 1;
                if self.count > self.max_records {
                    return Err(VerifierError::ResourceLimit {
                        limit: "max_records_per_artifact".to_owned(),
                    });
                }
                self.sink.accept(&self.line, self.count)?;
                self.line.clear();
            } else {
                if self.line.len() as u64 >= self.max_record_bytes {
                    return Err(VerifierError::ResourceLimit {
                        limit: "max_record_bytes".to_owned(),
                    });
                }
                self.line.push(byte);
            }
        }
        Ok(())
    }

    fn finish(&self) -> Result<(), VerifierError> {
        if !self.line.is_empty() {
            return Err(VerifierError::RecordStream {
                detail: "canonical record stream lacks a trailing LF boundary".to_owned(),
            });
        }
        if self.count != self.declared {
            return Err(VerifierError::RecordCount);
        }
        Ok(())
    }
}

#[derive(Debug)]
struct BoundFrame {
    kind: u8,
    members: u32,
}

fn check_line_bounds(line: &[u8], limits: &EffectiveLimits) -> Result<(), VerifierError> {
    let mut stack: Vec<BoundFrame> = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    let mut string_bytes = 0_u64;
    let mut token_bytes = 0_u64;
    for &byte in line {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                if string_bytes > limits.max_string_bytes() {
                    return Err(VerifierError::ResourceLimit {
                        limit: "max_string_bytes".to_owned(),
                    });
                }
                in_string = false;
                string_bytes = 0;
                continue;
            }
            string_bytes += 1;
            continue;
        }
        match byte {
            b'"' => {
                in_string = true;
                string_bytes = 0;
            }
            b'{' | b'[' => {
                token_bytes = 0;
                if stack.len() as u32 >= limits.max_nesting_depth() {
                    return Err(VerifierError::ResourceLimit {
                        limit: "max_nesting_depth".to_owned(),
                    });
                }
                stack.push(BoundFrame {
                    kind: byte,
                    members: 0,
                });
            }
            b'}' | b']' => {
                token_bytes = 0;
                stack.pop();
            }
            b',' => {
                token_bytes = 0;
                if let Some(frame) = stack.last_mut() {
                    frame.members += 1;
                    let bound = if frame.kind == b'{' {
                        limits.max_object_members()
                    } else {
                        limits.max_array_elements()
                    };
                    if frame.members > bound {
                        return Err(VerifierError::ResourceLimit {
                            limit: if frame.kind == b'{' {
                                "max_object_members"
                            } else {
                                "max_array_elements"
                            }
                            .to_owned(),
                        });
                    }
                }
            }
            b' ' | b'\t' | b'\n' | b':' => token_bytes = 0,
            _ => {
                token_bytes += 1;
                if token_bytes > limits.max_token_bytes() {
                    return Err(VerifierError::ResourceLimit {
                        limit: "max_token_bytes".to_owned(),
                    });
                }
            }
        }
    }
    Ok(())
}

/// Parsing mode selected by the caller's canonical-wire configuration.
pub(crate) enum RecordMode<'a> {
    /// Canonical Profile 1 wire shape with a registered payload registry.
    Canonical {
        /// Artifact order profiles keyed by declared artifact path.
        profiles: &'a HashMap<String, ArtifactRecordOrderProfile>,
        /// Registered event type tokens.
        event_types: &'a HashSet<String>,
    },
    /// Internal record shape retained for the compatibility adapter.
    Internal,
}

struct OneRecordParser<'a> {
    mode: &'a RecordMode<'a>,
    limits: &'a EffectiveLimits,
    artifact_path: &'a str,
    order_profile: &'a str,
    expected_envelope: &'a str,
    state: &'a mut ArtifactState,
}

impl RecordSink for OneRecordParser<'_> {
    fn accept(&mut self, line: &[u8], _ordinal: u64) -> Result<(), VerifierError> {
        check_line_bounds(line, self.limits)?;
        let value = parse_strict(line).map_err(|error| VerifierError::RecordStream {
            detail: error.to_string(),
        })?;
        if json_depth(&value) > self.limits.max_nesting_depth() {
            return Err(VerifierError::ResourceLimit {
                limit: "max_nesting_depth".to_owned(),
            });
        }
        if canonical_bytes(&value) != line {
            return Err(VerifierError::RecordStream {
                detail: "record line is not RFC 8785 canonical bytes".to_owned(),
            });
        }
        let key = match self.mode {
            RecordMode::Canonical {
                profiles,
                event_types,
            } => {
                let profile =
                    profiles
                        .get(self.artifact_path)
                        .ok_or_else(|| VerifierError::RecordOrder {
                            detail: "artifact has no canonical order profile".to_owned(),
                        })?;
                let key = canonical_wire::derive_key_from_bytes(line, profile)
                    .map_err(canonical_wire::CanonicalError::into_record_error)?;
                let record = canonical_wire::parse_record(&value)
                    .map_err(canonical_wire::CanonicalError::into_record_error)?;
                if record.envelope_schema_version.as_str() != self.expected_envelope {
                    return Err(VerifierError::EnvelopeVersion {
                        detail: "record envelope version differs from the manifest".to_owned(),
                    });
                }
                if !event_types.contains(record.event_type.as_str()) {
                    return Err(VerifierError::PayloadRegistry {
                        detail: "record event type is absent from the payload registry".to_owned(),
                    });
                }
                key
            }
            RecordMode::Internal => {
                let record: ReleaseRecord =
                    serde_json::from_slice(line).map_err(|_| VerifierError::RecordStream {
                        detail: "record line does not match the record schema".to_owned(),
                    })?;
                if record.envelope_schema_version != self.expected_envelope {
                    return Err(VerifierError::EnvelopeVersion {
                        detail: "record envelope version differs from the manifest".to_owned(),
                    });
                }
                internal_key(&record, self.order_profile)?
            }
        };
        if let Some(earlier) = &self.state.previous_key
            && (earlier.kind() != key.kind() || *earlier >= key)
        {
            return Err(VerifierError::RecordOrder {
                detail: "byte-derived keys are not strictly ascending".to_owned(),
            });
        }
        self.state.previous_key = Some(key);
        self.state.record_count += 1;
        self.state.envelope_agrees = true;
        Ok(())
    }
}

fn internal_key(
    record: &ReleaseRecord,
    order_profile: &str,
) -> Result<DerivedOrderKey, VerifierError> {
    if order_profile == "reference-key" {
        if record.object_id.is_empty() || record.assertion_id.is_empty() {
            return Err(VerifierError::RecordOrder {
                detail: "reference key members are empty".to_owned(),
            });
        }
        return Ok(DerivedOrderKey::Reference(
            record.object_id.clone(),
            record.revision,
            record.knowledge_time_ns,
            record.assertion_id.clone(),
        ));
    }
    if record.sequence_domain_id.is_empty() || record.sequence_epoch.is_empty() {
        return Err(VerifierError::RecordOrder {
            detail: "sequence domain or epoch is empty".to_owned(),
        });
    }
    Ok(DerivedOrderKey::Sequenced(
        record.sequence_domain_id.clone(),
        record.sequence_epoch.clone(),
        record.sequence_number,
    ))
}

/// Inputs for one streamed artifact verification.
pub(crate) struct ArtifactStream<'a> {
    /// Opened published artifact descriptor.
    pub(crate) file: &'a std::fs::File,
    /// Pre-read descriptor facts for the same artifact.
    pub(crate) facts: &'a FileFacts,
    /// Declared artifact metadata.
    pub(crate) descriptor: &'a ArtifactDescriptor,
    /// Private staging entry name.
    pub(crate) component: &'a str,
    /// Effective stricter-of limits.
    pub(crate) limits: &'a EffectiveLimits,
    /// Dataset accumulator shared across artifacts.
    pub(crate) dataset_hasher: &'a mut Sha256,
    /// Private snapshot writer.
    pub(crate) builder: &'a mut SnapshotBuilder,
    /// Canonical or internal parsing mode.
    pub(crate) mode: RecordMode<'a>,
    /// Envelope version declared by the manifest.
    pub(crate) expected_envelope: &'a str,
}

impl ArtifactStream<'_> {
    /// Run the bounded stored, decode, frame and parse passes for one artifact.
    pub(crate) fn verify(&mut self) -> Result<ArtifactState, VerifierError> {
        let cap = self
            .limits
            .max_decompression_budget_bytes()
            .min(self.limits.max_expanded_bytes_per_artifact())
            .min(self.descriptor.uncompressed_bytes);
        let ratio = u128::from(self.limits.max_expansion_ratio())
            .saturating_mul(u128::from(self.descriptor.stored_bytes));
        if u128::from(self.descriptor.uncompressed_bytes) > ratio {
            return Err(VerifierError::ResourceLimit {
                limit: "max_expansion_ratio".to_owned(),
            });
        }
        if self.descriptor.compression != "none" && self.descriptor.compression != "gzip" {
            return Err(VerifierError::Decompression {
                detail: "compression declaration is unknown".to_owned(),
            });
        }

        let mut state = ArtifactState {
            stored_digest: [0_u8; 32],
            expanded_digest: [0_u8; 32],
            stored_bytes: 0,
            expanded_bytes: 0,
            record_count: 0,
            previous_key: None,
            envelope_agrees: false,
        };
        let failure_slot = new_failure_slot();
        let (stored_digest, stored_bytes, expanded_digest, expanded_bytes, record_count) = {
            let mut expanded_hasher = Sha256::new();
            let mut parser = OneRecordParser {
                mode: &self.mode,
                limits: self.limits,
                artifact_path: self.descriptor.path.as_str(),
                order_profile: self.descriptor.record_order_profile.as_str(),
                expected_envelope: self.expected_envelope,
                state: &mut state,
            };
            let mut framer =
                RecordFramer::new(self.limits, self.descriptor.record_count, &mut parser);
            let (stored_digest, stored_bytes, expanded_bytes) = {
                let mut sink = ExpandedSink {
                    expanded: &mut expanded_hasher,
                    dataset: self.dataset_hasher,
                    framer: &mut framer,
                    cap,
                    total: 0,
                    failure: &failure_slot,
                };
                let mut stored = StoredStream::new(
                    self.file,
                    self.facts,
                    self.descriptor.stored_bytes,
                    self.limits.max_stored_bytes_per_artifact(),
                    self.limits.io_chunk_bytes(),
                )?;
                self.builder.begin_entry(self.component)?;
                let counted = if self.descriptor.compression == "gzip" {
                    let mut decoder = GzDecoder::new(&mut sink);
                    let result = stored.consume(|chunk| {
                        self.builder.write_chunk(chunk)?;
                        decoder
                            .write_all(chunk)
                            .map_err(|error| decode_error(&failure_slot, error))
                    });
                    match result {
                        Ok(value) => decoder
                            .finish()
                            .map_err(|error| decode_error(&failure_slot, error))
                            .map(|_| value),
                        Err(error) => Err(error),
                    }
                } else {
                    stored.consume(|chunk| {
                        self.builder.write_chunk(chunk)?;
                        sink.write_all(chunk)
                            .map_err(|error| decode_error(&failure_slot, error))
                    })
                };
                let expanded_bytes = sink.total;
                let counted = counted?;
                if let Some(error) = failure_slot.borrow_mut().take() {
                    return Err(error);
                }
                (counted.0, counted.1, expanded_bytes)
            };
            framer.finish()?;
            (
                stored_digest,
                stored_bytes,
                digest_of(expanded_hasher),
                expanded_bytes,
                framer.count,
            )
        };
        if expanded_digest
            != parse_hex_lower(&self.descriptor.uncompressed_sha256, "uncompressed_sha256")?
        {
            return Err(VerifierError::DigestMismatch {
                detail: "uncompressed_sha256 differs from decoded bytes".to_owned(),
            });
        }
        if expanded_bytes != self.descriptor.uncompressed_bytes {
            return Err(VerifierError::ByteCount {
                detail: "uncompressed_bytes differs from decoded bytes".to_owned(),
            });
        }
        self.builder.finalize_entry(stored_digest, stored_bytes)?;
        if hex_lower(&stored_digest) != self.descriptor.stored_sha256 {
            return Err(VerifierError::DigestMismatch {
                detail: "stored_sha256 differs from streamed bytes".to_owned(),
            });
        }
        state.stored_digest = stored_digest;
        state.expanded_digest = expanded_digest;
        state.stored_bytes = stored_bytes;
        state.expanded_bytes = expanded_bytes;
        state.record_count = record_count;
        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical_wire::ArtifactRecordOrderProfile;
    use crate::canonical_wire::NonEmptyString;
    use crate::canonical_wire::SequencedArtifactProfile;
    use crate::canonical_wire::SequencedKind;
    use crate::resource_profile::LimitsInput;
    use crate::resource_profile::ProcessingRole;
    use crate::resource_profile::ProfileLimits;
    use crate::resource_profile::ResourceProfile;
    use crate::resource_profile::ResourceProfileId;
    use crate::resource_profile::StreamingResourcePolicy;
    use serde_json::json;
    use std::fs::File;
    use std::io::Write as IoWrite;

    struct CapturingSink {
        lines: Vec<Vec<u8>>,
    }

    impl RecordSink for CapturingSink {
        fn accept(&mut self, line: &[u8], _ordinal: u64) -> Result<(), VerifierError> {
            self.lines.push(line.to_vec());
            Ok(())
        }
    }

    fn test_input() -> LimitsInput {
        LimitsInput {
            max_record_bytes: 128,
            max_records_per_artifact: 16,
            max_records_per_release: 64,
            max_stored_bytes_per_artifact: 4_096,
            max_stored_bytes_per_release: 16_384,
            max_expanded_bytes_per_artifact: 4_096,
            max_expanded_bytes_per_release: 16_384,
            max_decompression_budget_bytes: 4_096,
            max_expansion_ratio: 16,
            codec_window_bytes: 2_048,
            io_chunk_bytes: 8,
            max_nesting_depth: 8,
            max_string_bytes: 32,
            max_token_bytes: 16,
            max_object_members: 8,
            max_array_elements: 8,
            max_concurrent_artifacts: 2,
            worker_count: 2,
            queue_capacity_records: 8,
            temp_disk_bytes: 16_384,
            max_processing_duration_ms: 10_000,
            max_journal_record_bytes: 1_024,
            max_preparation_intent_bytes: 8_192,
            max_sealed_intent_bytes: 1_024,
            max_open_file_descriptors: 16,
            memory_budget_bytes: 268_435_456,
            max_artifacts: 4,
        }
    }

    fn effective() -> Result<EffectiveLimits, crate::resource_profile::ProfileError> {
        let input = test_input();
        let verifier = ResourceProfile::new(
            ResourceProfileId::new("TEST_ONLY_VERIFIER_FRAMER_V1", 1)?,
            ProcessingRole::Verifier,
            input,
        )?;
        let snapshot = ResourceProfile::new(
            ResourceProfileId::new("TEST_ONLY_SNAPSHOT_FRAMER_V1", 1)?,
            ProcessingRole::Snapshot,
            input,
        )?;
        StreamingResourcePolicy::new(ProfileLimits::validate(input)?, verifier, snapshot)?
            .verifier_limits()
    }

    #[test]
    fn framer_emits_one_line_and_reuses_the_buffer() -> Result<(), Box<dyn std::error::Error>> {
        let limits = effective()?;
        let mut sink = CapturingSink { lines: Vec::new() };
        let mut framer = RecordFramer::new(&limits, 2, &mut sink);
        framer.push(b"alpha\nbeta\n")?;
        framer.finish()?;
        assert_eq!(
            sink.lines,
            vec![b"alpha".to_vec(), b"beta".to_vec()],
            "two LF-terminated records were emitted in order"
        );
        Ok(())
    }

    #[test]
    fn framer_refuses_oversized_record_before_growth() -> Result<(), Box<dyn std::error::Error>> {
        let limits = effective()?;
        let mut sink = CapturingSink { lines: Vec::new() };
        let mut framer = RecordFramer::new(&limits, 1, &mut sink);
        let long = vec![b'x'; limits.max_record_bytes() as usize + 1];
        assert!(
            matches!(framer.push(&long), Err(VerifierError::ResourceLimit { .. })),
            "the line buffer stops at the record bound"
        );
        Ok(())
    }

    #[test]
    fn framer_refuses_blank_cr_and_missing_lf() -> Result<(), Box<dyn std::error::Error>> {
        let limits = effective()?;
        let mut sink = CapturingSink { lines: Vec::new() };
        let mut framer = RecordFramer::new(&limits, 1, &mut sink);
        assert!(
            matches!(framer.push(b"\n"), Err(VerifierError::RecordStream { .. })),
            "blank lines are refused"
        );
        let mut framer = RecordFramer::new(&limits, 1, &mut sink);
        assert!(
            matches!(
                framer.push(b"a\r\n"),
                Err(VerifierError::RecordStream { .. })
            ),
            "CR bytes are refused"
        );
        let mut framer = RecordFramer::new(&limits, 1, &mut sink);
        framer.push(b"partial")?;
        assert!(
            matches!(framer.finish(), Err(VerifierError::RecordStream { .. })),
            "a missing trailing LF is refused"
        );
        Ok(())
    }

    fn canonical_line(event_type: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let value = json!({
            "envelope_schema_version": "envelope-1",
            "event_type": event_type,
            "ordering": {
                "kind": "sequenced",
                "sequence_domain_id": "domain-a",
                "sequence_epoch": "epoch-1",
                "sequence_number": "1",
            },
            "payload": {
                "fields": [
                    {"field_name": "abstract-field", "kind": "string", "value": "abstract-value"}
                ]
            }
        });
        Ok(canonical_bytes(&value))
    }

    fn canonical_profiles()
    -> Result<HashMap<String, ArtifactRecordOrderProfile>, Box<dyn std::error::Error>> {
        let mut profiles = HashMap::new();
        profiles.insert(
            "artifacts/records.jsonl".to_owned(),
            ArtifactRecordOrderProfile::Sequenced(SequencedArtifactProfile {
                kind: SequencedKind::Sequenced,
                sequence_domain_id: NonEmptyString::new("domain-a")?,
                sequence_epoch: NonEmptyString::new("epoch-1")?,
            }),
        );
        Ok(profiles)
    }

    #[test]
    fn canonical_registry_miss_is_a_stage_five_typed_refusal()
    -> Result<(), Box<dyn std::error::Error>> {
        let limits = effective()?;
        let profiles = canonical_profiles()?;
        let event_types = HashSet::new();
        let mode = RecordMode::Canonical {
            profiles: &profiles,
            event_types: &event_types,
        };
        let mut state = ArtifactState {
            stored_digest: [0_u8; 32],
            expanded_digest: [0_u8; 32],
            stored_bytes: 0,
            expanded_bytes: 0,
            record_count: 0,
            previous_key: None,
            envelope_agrees: false,
        };
        let mut parser = OneRecordParser {
            mode: &mode,
            limits: &limits,
            artifact_path: "artifacts/records.jsonl",
            order_profile: "sequence-domain-strict",
            expected_envelope: "envelope-1",
            state: &mut state,
        };
        let line = canonical_line("unregistered-event")?;
        let refusal = parser.accept(&line, 1).unwrap_err();
        assert_eq!(
            refusal.code(),
            "REJECT_PAYLOAD_REGISTRY",
            "registry miss has its own code"
        );
        assert_eq!(
            refusal.stage(),
            5,
            "registry miss is detected at stage five"
        );
        Ok(())
    }

    #[test]
    fn canonical_registry_hit_is_accepted() -> Result<(), Box<dyn std::error::Error>> {
        let limits = effective()?;
        let profiles = canonical_profiles()?;
        let event_types: HashSet<String> = ["registered-event".to_owned()].into_iter().collect();
        let mode = RecordMode::Canonical {
            profiles: &profiles,
            event_types: &event_types,
        };
        let mut state = ArtifactState {
            stored_digest: [0_u8; 32],
            expanded_digest: [0_u8; 32],
            stored_bytes: 0,
            expanded_bytes: 0,
            record_count: 0,
            previous_key: None,
            envelope_agrees: false,
        };
        let mut parser = OneRecordParser {
            mode: &mode,
            limits: &limits,
            artifact_path: "artifacts/records.jsonl",
            order_profile: "sequence-domain-strict",
            expected_envelope: "envelope-1",
            state: &mut state,
        };
        let line = canonical_line("registered-event")?;
        parser.accept(&line, 1)?;
        assert!(
            state.envelope_agrees,
            "a registry hit updates the aggregate flag"
        );
        Ok(())
    }

    #[test]
    fn bounds_scan_refuses_deep_nesting_and_long_tokens() -> Result<(), Box<dyn std::error::Error>>
    {
        let limits = effective()?;
        let deep = b"[[[[[[[[[1]]]]]]]]]";
        assert!(
            matches!(
                check_line_bounds(deep, &limits),
                Err(VerifierError::ResourceLimit { .. })
            ),
            "nesting past the profile depth is refused"
        );
        let long_token = [b'x'; 17];
        assert!(
            matches!(
                check_line_bounds(&long_token, &limits),
                Err(VerifierError::ResourceLimit { .. })
            ),
            "a token past the profile bound is refused"
        );
        Ok(())
    }

    #[test]
    fn bounds_scan_refuses_long_string_but_accepts_short_line()
    -> Result<(), Box<dyn std::error::Error>> {
        let limits = effective()?;
        let mut long = vec![b'"'; 34];
        long.extend_from_slice(&[b'a'; 33]);
        assert!(
            matches!(
                check_line_bounds(&long, &limits),
                Err(VerifierError::ResourceLimit { .. })
            ),
            "a string past the profile bound is refused"
        );
        assert!(
            check_line_bounds(br#"{"a":"b"}"#, &limits).is_ok(),
            "a short canonical object passes the structural scan"
        );
        Ok(())
    }

    #[test]
    fn stored_stream_updates_incremental_digest_and_chunk_count()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("stored.bin");
        let mut file = File::create(&path)?;
        file.write_all(b"abcdefghij")?;
        drop(file);
        let file = File::open(&path)?;
        let facts = crate::files::file_facts(&file)?;
        let mut stream = StoredStream::new(&file, &facts, 10, 64, 4)?;
        let mut seen = 0_u64;
        let (digest, total) = stream.consume(|chunk| {
            seen += 1;
            assert!(chunk.len() <= 4, "each emitted part fits the fixed chunk");
            Ok(())
        })?;
        assert_eq!(seen, 3, "three fixed chunks were emitted");
        assert_eq!(total, 10, "the declared byte count was consumed");
        assert_eq!(
            digest,
            crate::files::sha256(b"abcdefghij"),
            "stored digest is incremental"
        );
        Ok(())
    }

    #[test]
    fn expanded_sink_enforces_cap_during_decode() -> Result<(), Box<dyn std::error::Error>> {
        let limits = effective()?;
        let slot = new_failure_slot();
        let mut expanded_hasher = Sha256::new();
        let mut dataset_hasher = Sha256::new();
        let mut sink = CapturingSink { lines: Vec::new() };
        let mut framer = RecordFramer::new(&limits, 4, &mut sink);
        let mut sink_wrapper = ExpandedSink {
            expanded: &mut expanded_hasher,
            dataset: &mut dataset_hasher,
            framer: &mut framer,
            cap: 4,
            total: 0,
            failure: &slot,
        };
        let refused = sink_wrapper.write_all(b"abcdef").is_err();
        assert!(refused, "the emission cap is checked during decode");
        assert!(
            matches!(
                slot.borrow_mut().take(),
                Some(VerifierError::ResourceLimit { .. })
            ),
            "the typed resource refusal is retained"
        );
        Ok(())
    }
}
