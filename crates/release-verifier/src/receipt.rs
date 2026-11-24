// SPDX-License-Identifier: Apache-2.0
//! Immutable admission receipt issued after all twelve stages succeed.
//!
//! A receipt records which checks ran and what they bound. It is not a right,
//! a licence, a coverage certificate, a dataset entitlement or proof of any
//! observation.

use crate::error::VerifierError;
use crate::schema::AdmissionRequest;
use crate::schema::CoverageDecision;
use crate::schema::RightsDecision;
use sha2::Digest;
use sha2::Sha256;

/// One artifact binding carried in an admission receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactBinding {
    /// Declared repository-relative artifact path.
    pub path: String,
    /// Logical partition identity.
    pub logical_partition_id: String,
    /// Shard ordinal.
    pub shard_ordinal: u32,
    /// SHA-256 over exact stored bytes.
    pub stored_sha256: String,
    /// SHA-256 over decoded bytes.
    pub uncompressed_sha256: String,
    /// Exact stored byte count.
    pub stored_bytes: u64,
    /// Exact uncompressed byte count.
    pub uncompressed_bytes: u64,
    /// Verified record count.
    pub record_count: u64,
}

/// Terminal result recorded in an admission receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionResult {
    /// All twelve stages completed and the request was admitted.
    Admitted,
}

/// Bound fields supplied to receipt construction after all stages succeed.
pub struct ReceiptInput {
    /// Release identity from the verified manifest.
    pub release_id: String,
    /// Canonical manifest digest.
    pub manifest_sha256: String,
    /// Ordered dataset digest.
    pub data_sha256: String,
    /// Trusted publisher identity.
    pub publisher_id: String,
    /// Trusted key identity.
    pub key_id: String,
    /// Trust registry revision.
    pub trust_registry_revision: String,
    /// Contract version.
    pub contract_version: String,
    /// Contract revision.
    pub contract_revision: String,
    /// Verification policy version.
    pub verification_policy_version: String,
    /// Verified artifact bindings in canonical order.
    pub artifact_set: Vec<ArtifactBinding>,
    /// Injected rights decision.
    pub rights_decision: RightsDecision,
    /// Injected coverage decision.
    pub coverage_decision: CoverageDecision,
    /// Admission request submitted by the consumer.
    pub admission_request: AdmissionRequest,
    /// Consumer use profile.
    pub consumer_use_profile: String,
    /// Verifier commit string.
    pub verifier_commit: String,
}

/// An immutable receipt binding every verified decision.
#[derive(Debug, Clone)]
pub struct AdmissionReceipt {
    release_id: String,
    manifest_sha256: String,
    data_sha256: String,
    publisher_id: String,
    key_id: String,
    trust_registry_revision: String,
    contract_version: String,
    contract_revision: String,
    verification_policy_version: String,
    artifact_set: Vec<ArtifactBinding>,
    rights_decision: RightsDecision,
    coverage_decision: CoverageDecision,
    admission_request: AdmissionRequest,
    consumer_use_profile: String,
    verifier_commit: String,
    result: AdmissionResult,
    receipt_digest: [u8; 32],
}

impl AdmissionReceipt {
    /// Construct a receipt and compute its binding digest.
    pub fn new(input: ReceiptInput) -> Result<Self, VerifierError> {
        let ReceiptInput {
            release_id,
            manifest_sha256,
            data_sha256,
            publisher_id,
            key_id,
            trust_registry_revision,
            contract_version,
            contract_revision,
            verification_policy_version,
            artifact_set,
            rights_decision,
            coverage_decision,
            admission_request,
            consumer_use_profile,
            verifier_commit,
        } = input;
        if release_id.is_empty()
            || manifest_sha256.is_empty()
            || data_sha256.is_empty()
            || publisher_id.is_empty()
            || key_id.is_empty()
            || trust_registry_revision.is_empty()
            || verification_policy_version.is_empty()
        {
            return Err(VerifierError::Receipt {
                detail: "a required binding is empty".to_owned(),
            });
        }
        if artifact_set.is_empty() {
            return Err(VerifierError::Receipt {
                detail: "artifact set is empty".to_owned(),
            });
        }
        let digest = ReceiptDigest::new()
            .field("release_id", &release_id)
            .field("manifest_sha256", &manifest_sha256)
            .field("data_sha256", &data_sha256)
            .field("publisher_id", &publisher_id)
            .field("key_id", &key_id)
            .field("registry_revision", &trust_registry_revision)
            .field("contract_version", &contract_version)
            .field("contract_revision", &contract_revision)
            .field("policy_version", &verification_policy_version)
            .fields(&artifact_set, |artifact| {
                vec![
                    ("artifact_path".to_owned(), artifact.path.clone()),
                    ("artifact_stored".to_owned(), artifact.stored_sha256.clone()),
                    (
                        "artifact_uncompressed".to_owned(),
                        artifact.uncompressed_sha256.clone(),
                    ),
                ]
            })
            .field(
                "rights_decision",
                match rights_decision.decision {
                    crate::schema::Decision::Permit => "PERMIT",
                    crate::schema::Decision::Deny => "DENY",
                    crate::schema::Decision::Unknown => "UNKNOWN",
                },
            )
            .fields(&coverage_decision.approved_dimensions, |dimension| {
                vec![("coverage_dimension".to_owned(), dimension.clone())]
            })
            .field("use_profile", &consumer_use_profile)
            .field("request_release_id", &admission_request.release_id)
            .field("request_actor", &admission_request.actor)
            .field("verifier_commit", &verifier_commit)
            .finish();
        Ok(Self {
            release_id,
            manifest_sha256,
            data_sha256,
            publisher_id,
            key_id,
            trust_registry_revision,
            contract_version,
            contract_revision,
            verification_policy_version,
            artifact_set,
            rights_decision,
            coverage_decision,
            admission_request,
            consumer_use_profile,
            verifier_commit,
            result: AdmissionResult::Admitted,
            receipt_digest: digest,
        })
    }

    /// Return the release identity bound by this receipt.
    pub fn release_id(&self) -> &str {
        &self.release_id
    }

    /// Return the canonical manifest digest bound by this receipt.
    pub fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }

    /// Return the ordered dataset digest bound by this receipt.
    pub fn data_sha256(&self) -> &str {
        &self.data_sha256
    }

    /// Return the publisher identity bound by this receipt.
    pub fn publisher_id(&self) -> &str {
        &self.publisher_id
    }

    /// Return the trusted key identity bound by this receipt.
    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    /// Return the trusted registry revision bound by this receipt.
    pub fn trust_registry_revision(&self) -> &str {
        &self.trust_registry_revision
    }

    /// Return the contract version bound by this receipt.
    pub fn contract_version(&self) -> &str {
        &self.contract_version
    }

    /// Return the contract revision bound by this receipt.
    pub fn contract_revision(&self) -> &str {
        &self.contract_revision
    }

    /// Return the verifier policy version bound by this receipt.
    pub fn verification_policy_version(&self) -> &str {
        &self.verification_policy_version
    }

    /// Return the verified artifact set in canonical order.
    pub fn artifact_set(&self) -> &[ArtifactBinding] {
        &self.artifact_set
    }

    /// Return the rights decision recorded in this receipt.
    pub fn rights_decision(&self) -> &RightsDecision {
        &self.rights_decision
    }

    /// Return the coverage decision recorded in this receipt.
    pub fn coverage_decision(&self) -> &CoverageDecision {
        &self.coverage_decision
    }

    /// Return the admission request recorded in this receipt.
    pub fn admission_request(&self) -> &AdmissionRequest {
        &self.admission_request
    }

    /// Return the consumer use profile recorded in this receipt.
    pub fn consumer_use_profile(&self) -> &str {
        &self.consumer_use_profile
    }

    /// Return the verifier commit string recorded in this receipt.
    pub fn verifier_commit(&self) -> &str {
        &self.verifier_commit
    }

    /// Return the terminal receipt result.
    pub fn result(&self) -> AdmissionResult {
        self.result
    }

    /// Return the binding digest over the receipt fields.
    pub fn receipt_digest(&self) -> &[u8; 32] {
        &self.receipt_digest
    }
}

struct ReceiptDigest {
    buffer: Vec<u8>,
}

impl ReceiptDigest {
    fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    fn field(mut self, label: &str, value: &str) -> Self {
        self.append(label, value);
        self
    }

    fn fields<T, F>(mut self, values: &[T], map: F) -> Self
    where
        F: Fn(&T) -> Vec<(String, String)>,
    {
        for value in values {
            for (label, text) in map(value) {
                self.append(&label, &text);
            }
        }
        self
    }

    fn append(&mut self, label: &str, value: &str) {
        self.buffer.extend_from_slice(label.as_bytes());
        self.buffer.push(b'=');
        self.buffer.extend_from_slice(value.as_bytes());
        self.buffer.push(b'\n');
    }

    fn finish(self) -> [u8; 32] {
        let digest = Sha256::digest(&self.buffer);
        let mut out = [0_u8; 32];
        out.copy_from_slice(&digest);
        out
    }
}
