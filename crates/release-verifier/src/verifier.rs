// SPDX-License-Identifier: Apache-2.0
//! The twelve ordered admission stages.
//!
//! Processing stops at the first refusal. Byte reads come from descriptors
//! opened with `O_NOFOLLOW` below a configured trusted root. After stage five
//! copies verified bytes into private staging and seals the snapshot, no later
//! stage opens a published path.

use crate::canonical_wire;
use crate::canonical_wire::ArtifactRecordOrderProfile;
use crate::error::CleanupPending;
use crate::error::VerifierError;
use crate::files::CleanupReport;
use crate::files::FdBudget;
use crate::files::PrivateSnapshot;
use crate::files::ReleaseRoot;
use crate::files::SnapshotBuilder;
use crate::files::artifact_component;
use crate::files::enumerate_names;
use crate::files::hex_lower;
use crate::files::open_dir_at;
use crate::files::open_regular_file;
use crate::files::parse_hex_lower;
use crate::files::read_bounded;
use crate::files::sha256;
use crate::jcs::canonical_bytes;
use crate::jcs::json_depth;
use crate::jcs::parse_strict;
use crate::receipt::AdmissionReceipt;
use crate::receipt::ArtifactBinding;
use crate::schema::AdmissionRequest;
use crate::schema::ArtifactDescriptor;
use crate::schema::CoverageEvaluator;
use crate::schema::Decision;
use crate::schema::LineageResolver;
use crate::schema::ReleaseManifest;
use crate::schema::ReleaseRecord;
use crate::schema::RightsEvaluator;
use crate::schema::SignatureSidecar;
use crate::trust::TrustedKeyRegistry;
use crate::trust::VerifierResourcePolicy;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::Signature;
use ed25519_dalek::Verifier as SignatureVerifier;
use ed25519_dalek::VerifyingKey;
use flate2::read::GzDecoder;
use serde_json::Value;
use sha2::Digest;
use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::io::Read;
use std::path::PathBuf;

/// Domain prefix bound into every Ed25519 signature.
pub const SIGNING_DOMAIN: &[u8] = b"NDC-V2-RELEASE-MANIFEST-SHA256-V1\x00";

/// Version of the enforcement policy applied by this verifier.
pub const VERIFICATION_POLICY_VERSION: &str = "ndc-v2-release-verifier-policy/1";

/// Exact purpose declared for reviewed consumer use.
pub const DECLARED_PURPOSE: &str = "proprietary_own_account_trading_research";

/// Accepted record encoding for this profile.
pub const RECORD_ENCODING: &str = "application/x-ndjson; charset=utf-8";

/// The nine coverage dimensions required by the ratified profile.
pub const COVERAGE_DIMENSIONS: [&str; 9] = [
    "instrument",
    "listing_identity",
    "contract",
    "corporate_action",
    "calendar_session",
    "trades",
    "quotes_depth",
    "mbo",
    "sequence_continuity",
];

const SUPERSEDES_STRICT: &str = "sequence-domain-strict";
const SUPERSEDES_GAPS: &str = "sequence-domain-declared-gaps";
const REFERENCE_KEY: &str = "reference-key";

/// Cleanup outcome recorded next to a first refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupDisposition {
    /// Cleanup observed an empty directory after a post-removal sync.
    Completed(CleanupReport),
    /// Cleanup stopped before an empty, synced directory was observed.
    Pending(CleanupPending),
}

impl CleanupDisposition {
    /// Report success only for a completed report with an empty synced directory.
    pub fn is_complete(&self) -> bool {
        matches!(self, Self::Completed(report) if report.is_complete())
    }

    /// Report pending for every outcome that did not prove cleanup completion.
    pub fn is_pending(&self) -> bool {
        matches!(self, Self::Pending(_))
    }
}

/// A first refusal together with its independent cleanup outcome.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{refusal}")]
pub struct RefusalDiagnostic {
    /// The original first refusal, which keeps its own stage and code.
    pub refusal: VerifierError,
    /// Cleanup attempt made after this refusal, once staging had started.
    pub cleanup: Option<Box<CleanupDisposition>>,
}

type PipelineResult<T> = Result<T, Box<(VerifierError, Option<CleanupDisposition>)>>;

fn no_cleanup<T>(result: Result<T, VerifierError>) -> PipelineResult<T> {
    result.map_err(|error| Box::new((error, None)))
}

fn with_cleanup(
    refusal: VerifierError,
    cleanup: Result<CleanupReport, CleanupPending>,
) -> (VerifierError, Option<CleanupDisposition>) {
    let disposition = match cleanup {
        Ok(report) => CleanupDisposition::Completed(report),
        Err(pending) => CleanupDisposition::Pending(pending),
    };
    (refusal, Some(disposition))
}

/// Locator for one immutable release under the configured trusted root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseLocator {
    /// Publisher directory component.
    pub publisher_id: String,
    /// Dataset directory component.
    pub dataset_id: String,
    /// Release directory component.
    pub release_id: String,
}

/// Injected configuration for an independent release verifier.
pub struct VerifierConfig {
    /// Configured trusted release root path.
    pub root: PathBuf,
    /// Caller-provided private staging directory.
    pub private_staging: PathBuf,
    /// Out-of-band trusted publisher registry.
    pub registry: TrustedKeyRegistry,
    /// Injected rights evaluator.
    pub rights: Box<dyn RightsEvaluator>,
    /// Injected coverage evaluator.
    pub coverage: Box<dyn CoverageEvaluator>,
    /// Injected trusted lineage resolver.
    pub lineage: Box<dyn LineageResolver>,
    /// Caller-supplied resource maxima.
    pub policy: VerifierResourcePolicy,
    /// Commit string bound into the receipt.
    pub verifier_commit: String,
}

/// An independent verifier for NDC v2 release packages.
pub struct ReleaseVerifier {
    root: PathBuf,
    private_staging: PathBuf,
    registry: TrustedKeyRegistry,
    rights: Box<dyn RightsEvaluator>,
    coverage: Box<dyn CoverageEvaluator>,
    lineage: Box<dyn LineageResolver>,
    policy: VerifierResourcePolicy,
    verifier_commit: String,
    require_canonical_wire: bool,
    stage_observer: Option<Box<dyn Fn(u8)>>,
    canonical_profiles: RefCell<HashMap<String, ArtifactRecordOrderProfile>>,
    canonical_event_types: RefCell<HashSet<String>>,
}

impl ReleaseVerifier {
    /// Assemble a verifier from injected trust, policy and evaluation inputs.
    pub fn new(config: VerifierConfig) -> Self {
        Self {
            root: config.root,
            private_staging: config.private_staging,
            registry: config.registry,
            rights: config.rights,
            coverage: config.coverage,
            lineage: config.lineage,
            policy: config.policy,
            verifier_commit: config.verifier_commit,
            require_canonical_wire: false,
            stage_observer: None,
            canonical_profiles: RefCell::new(HashMap::new()),
            canonical_event_types: RefCell::new(HashSet::new()),
        }
    }

    /// Require the ratified canonical wire documents instead of the older internal shape.
    ///
    /// The canonical representation is not normative Rust code; this mode
    /// parses the canonical bytes independently and maps them into the internal
    /// types after validation.
    pub fn with_canonical_wire(mut self) -> Self {
        self.require_canonical_wire = true;
        self
    }

    /// Observe the number of each stage that completes successfully.
    ///
    /// The callback is used by the conformance executable to record stage
    /// status without relaxing first-refusal semantics.
    pub fn with_stage_observer<F>(mut self, observer: F) -> Self
    where
        F: Fn(u8) + 'static,
    {
        self.stage_observer = Some(Box::new(observer));
        self
    }

    fn note_stage(&self, stage: u8) {
        if let Some(observer) = &self.stage_observer {
            observer(stage);
        }
    }

    /// Verify a release and issue an immutable admission receipt.
    ///
    /// This wrapper exposes the first refusal. Callers that also need the
    /// cleanup result use `verify_release_with_diagnostics`.
    pub fn verify_release(
        &self,
        locator: &ReleaseLocator,
        request: &AdmissionRequest,
    ) -> Result<AdmissionReceipt, VerifierError> {
        self.verify_release_with_diagnostics(locator, request)
            .map_err(|diagnostic| diagnostic.refusal)
    }

    /// Verify a release and keep a cleanup failure beside the first refusal.
    pub fn verify_release_with_diagnostics(
        &self,
        locator: &ReleaseLocator,
        request: &AdmissionRequest,
    ) -> Result<AdmissionReceipt, RefusalDiagnostic> {
        self.canonical_profiles.borrow_mut().clear();
        self.canonical_event_types.borrow_mut().clear();
        self.run_stages(locator, request).map_err(|failure| {
            let (refusal, cleanup) = *failure;
            RefusalDiagnostic {
                refusal,
                cleanup: cleanup.map(Box::new),
            }
        })
    }

    fn run_stages(
        &self,
        locator: &ReleaseLocator,
        request: &AdmissionRequest,
    ) -> PipelineResult<AdmissionReceipt> {
        let mut budget = no_cleanup(FdBudget::new(&self.policy))?;
        let root = no_cleanup(ReleaseRoot::open(&self.root, &mut budget))?;
        let release_dir = no_cleanup(root.release_dir(
            &locator.publisher_id,
            &locator.dataset_id,
            &locator.release_id,
            &mut budget,
        ))?;
        let top_level = no_cleanup(enumerate_names(&release_dir))?;
        let expected_top: HashSet<&str> = ["manifest.json", "signature.json", "artifacts"]
            .into_iter()
            .collect();
        if top_level
            .iter()
            .map(String::as_str)
            .collect::<HashSet<&str>>()
            != expected_top
        {
            return Err(Box::new((
                VerifierError::FileSet {
                    detail: "release root file set differs from manifest, sidecar and artifacts"
                        .to_owned(),
                },
                None,
            )));
        }
        let (manifest_file, manifest_facts) = no_cleanup(open_regular_file(
            &release_dir,
            "manifest.json",
            &mut budget,
        ))?;
        let manifest_bytes = no_cleanup(read_bounded(
            &manifest_file,
            &manifest_facts,
            manifest_facts.size,
            self.policy.max_manifest_bytes,
            "max_manifest_bytes",
        ))?;
        let (sidecar_file, sidecar_facts) = no_cleanup(open_regular_file(
            &release_dir,
            "signature.json",
            &mut budget,
        ))?;
        let sidecar_bytes = no_cleanup(read_bounded(
            &sidecar_file,
            &sidecar_facts,
            sidecar_facts.size,
            self.policy.max_sidecar_bytes,
            "max_sidecar_bytes",
        ))?;
        drop(manifest_file);
        drop(sidecar_file);

        let mut builder = no_cleanup(SnapshotBuilder::new(
            &self.private_staging,
            &mut budget,
            &self.policy,
        ))?;
        for (name, bytes) in [
            ("manifest.json", manifest_bytes.as_slice()),
            ("signature.json", sidecar_bytes.as_slice()),
        ] {
            if let Err(error) = builder.add(name, bytes) {
                return Err(Box::new(with_cleanup(error, builder.discard_partial())));
            }
        }

        let parsed = match self.stages_one_to_four(&manifest_bytes, &sidecar_bytes, request) {
            Ok(value) => value,
            Err(error) => return Err(Box::new(with_cleanup(error, builder.discard_partial()))),
        };
        let stage_five =
            match self.stage_five(&parsed.manifest, &release_dir, &mut builder, &mut budget) {
                Ok(value) => value,
                Err(error) => return Err(Box::new(with_cleanup(error, builder.discard_partial()))),
            };
        if let Err(error) = builder.sync_directory() {
            return Err(Box::new(with_cleanup(error, builder.discard_partial())));
        }
        let snapshot = builder.seal();
        self.note_stage(5);
        drop(release_dir);
        drop(root);

        match self.stages_six_to_twelve(
            &parsed.manifest,
            &parsed.sidecar,
            &parsed.manifest_digest,
            &stage_five,
            &snapshot,
            request,
        ) {
            Ok(receipt) => Ok(receipt),
            Err(error) => Err(Box::new(with_cleanup(error, snapshot.discard()))),
        }
    }

    fn stages_one_to_four(
        &self,
        manifest_bytes: &[u8],
        sidecar_bytes: &[u8],
        request: &AdmissionRequest,
    ) -> Result<ParsedStages, VerifierError> {
        let manifest_value =
            parse_strict(manifest_bytes).map_err(|error| VerifierError::ManifestParse {
                detail: error.to_string(),
            })?;
        check_depth(&manifest_value, self.policy.max_json_nesting)?;
        let manifest = if self.require_canonical_wire {
            let parsed = canonical_wire::parse_manifest(&manifest_value)
                .map_err(canonical_wire::CanonicalError::into_manifest_error)?;
            *self.canonical_profiles.borrow_mut() = parsed.profiles.clone();
            *self.canonical_event_types.borrow_mut() = parsed.event_types.clone();
            canonical_wire::into_internal_manifest(&parsed)?
        } else {
            serde_json::from_value(manifest_value.clone()).map_err(classify_decode_error)?
        };
        self.check_manifest_schema(&manifest)?;

        let sidecar_value =
            parse_strict(sidecar_bytes).map_err(|error| VerifierError::SidecarSchema {
                detail: error.to_string(),
            })?;
        check_depth(&sidecar_value, self.policy.max_json_nesting)?;
        let sidecar: SignatureSidecar =
            serde_json::from_value(sidecar_value).map_err(classify_decode_error)?;
        self.note_stage(1);

        let canonical = canonical_bytes(&manifest_value);
        let manifest_digest = hex_lower(&sha256(&canonical));
        let sidecar_digest = parse_hex_lower(&sidecar.manifest_sha256, "manifest_sha256")
            .map_err(|_| VerifierError::MalformedManifestDigest)?;
        if sidecar_digest != sha256(&canonical) {
            return Err(VerifierError::ManifestDigestMismatch);
        }
        self.note_stage(2);

        if sidecar.schema_version != 1
            || sidecar.algorithm != "Ed25519"
            || sidecar.key_id.is_empty()
        {
            return Err(VerifierError::SidecarSchema {
                detail: "sidecar version, algorithm or key id is invalid".to_owned(),
            });
        }
        let signature_bytes = STANDARD
            .decode(&sidecar.signature_b64)
            .map_err(|_| VerifierError::MalformedSignature)?;
        if signature_bytes.len() != 64 {
            return Err(VerifierError::MalformedSignature);
        }
        let signature = Signature::from_slice(&signature_bytes)
            .map_err(|_| VerifierError::MalformedSignature)?;

        let entry = self.registry.lookup(
            &manifest.publication_identity.publisher_identity,
            &sidecar.key_id,
        )?;
        let verifying_key = VerifyingKey::from_bytes(&entry.public_key_bytes)
            .map_err(|_| VerifierError::KeyAlgorithm)?;
        let mut message = Vec::with_capacity(SIGNING_DOMAIN.len() + 32);
        message.extend_from_slice(SIGNING_DOMAIN);
        message.extend_from_slice(&sidecar_digest);
        verifying_key
            .verify(&message, &signature)
            .map_err(|_| VerifierError::SignatureInvalid)?;
        self.note_stage(3);

        self.registry
            .check_policy(entry, manifest.release_identity.as_of_ns)?;
        if sidecar.key_id != manifest.publication_identity.signing_key_id {
            return Err(VerifierError::KeyBinding);
        }
        if request.publisher_identity != manifest.publication_identity.publisher_identity {
            return Err(VerifierError::KeyBinding);
        }
        self.note_stage(4);
        Ok(ParsedStages {
            manifest,
            sidecar,
            manifest_digest,
        })
    }

    fn check_manifest_schema(&self, manifest: &ReleaseManifest) -> Result<(), VerifierError> {
        let identity = &manifest.release_identity;
        if manifest.schema_version != 1 {
            return Err(VerifierError::ManifestSchema {
                detail: "schema_version must be one".to_owned(),
            });
        }
        if manifest.contract_version != "v2" || manifest.contract_revision.is_empty() {
            return Err(VerifierError::ManifestSchema {
                detail: "contract version or revision is invalid".to_owned(),
            });
        }
        if identity.as_of_ns == 0
            || identity.created_at_utc_ns == 0
            || identity.release_id.is_empty()
            || identity.publication_state != "AVAILABLE"
        {
            return Err(VerifierError::ManifestSchema {
                detail: "release identity is incomplete or not AVAILABLE".to_owned(),
            });
        }
        if manifest.digests.digest_algorithm != "sha256"
            || parse_hex_lower(&manifest.digests.data_sha256, "data_sha256").is_err()
        {
            return Err(VerifierError::ManifestSchema {
                detail: "dataset digest declaration is invalid".to_owned(),
            });
        }
        if manifest.publication_identity.publisher_identity.is_empty()
            || manifest.publication_identity.signing_key_id.is_empty()
            || manifest.publication_identity.signature_reference != "signature.json"
            || manifest.publication_identity.archive_reference.is_empty()
        {
            return Err(VerifierError::ManifestSchema {
                detail: "publication identity is incomplete".to_owned(),
            });
        }
        Ok(())
    }

    fn stage_five(
        &self,
        manifest: &ReleaseManifest,
        release_dir: &std::fs::File,
        builder: &mut SnapshotBuilder,
        budget: &mut FdBudget,
    ) -> Result<StageFive, VerifierError> {
        let descriptors = self.check_artifact_declarations(manifest)?;
        let artifacts_dir = open_dir_at(release_dir, "artifacts", budget)?;
        let observed = enumerate_names(&artifacts_dir)?;
        let components: Vec<String> = descriptors
            .iter()
            .map(|descriptor| artifact_component(&descriptor.path))
            .collect::<Result<_, _>>()?;
        let expected: HashSet<&str> = components.iter().map(String::as_str).collect();
        if observed
            .iter()
            .map(String::as_str)
            .collect::<HashSet<&str>>()
            != expected
        {
            return Err(VerifierError::FileSet {
                detail: "artifact directory file set differs from artifacts[]".to_owned(),
            });
        }
        let mut dataset_hasher = sha2::Sha256::new();
        let mut all_records = Vec::new();
        let mut bindings = Vec::new();
        for descriptor in descriptors {
            let component = artifact_component(&descriptor.path)?;
            let (file, facts) = open_regular_file(&artifacts_dir, &component, budget)?;
            let stored = read_bounded(
                &file,
                &facts,
                descriptor.stored_bytes,
                self.policy.max_stored_bytes,
                "max_stored_bytes",
            )?;
            let stored_digest = sha256(&stored);
            if stored_digest != parse_hex_lower(&descriptor.stored_sha256, "stored_sha256")? {
                return Err(VerifierError::DigestMismatch {
                    detail: "stored_sha256 differs from opened bytes".to_owned(),
                });
            }
            let expanded = decompress(&stored, descriptor, &self.policy)?;
            if expanded.len() as u64 != descriptor.uncompressed_bytes {
                return Err(VerifierError::ByteCount {
                    detail: "uncompressed_bytes differs from decoded bytes".to_owned(),
                });
            }
            if expanded.len() as u64 > self.policy.max_expanded_bytes {
                return Err(VerifierError::ResourceLimit {
                    limit: "max_expanded_bytes".to_owned(),
                });
            }
            let expanded_digest = sha256(&expanded);
            if expanded_digest
                != parse_hex_lower(&descriptor.uncompressed_sha256, "uncompressed_sha256")?
            {
                return Err(VerifierError::DigestMismatch {
                    detail: "uncompressed_sha256 differs from decoded bytes".to_owned(),
                });
            }
            dataset_hasher.update(&expanded);
            let records = if self.require_canonical_wire {
                verify_canonical_record_stream(
                    &expanded,
                    descriptor,
                    &self.canonical_profiles.borrow(),
                    &self.canonical_event_types.borrow(),
                    &self.policy,
                )?
            } else {
                verify_record_stream(&expanded, descriptor, &self.policy)?
            };
            drop(file);
            builder.add(&component, &stored)?;
            all_records.push(records);
            bindings.push(ArtifactBinding {
                path: descriptor.path.clone(),
                logical_partition_id: descriptor.logical_partition_id.clone(),
                shard_ordinal: descriptor.shard_ordinal,
                stored_sha256: descriptor.stored_sha256.clone(),
                uncompressed_sha256: descriptor.uncompressed_sha256.clone(),
                stored_bytes: descriptor.stored_bytes,
                uncompressed_bytes: descriptor.uncompressed_bytes,
                record_count: descriptor.record_count,
            });
        }
        let mut total = 0_u64;
        for binding in &bindings {
            total = total.saturating_add(binding.uncompressed_bytes);
        }
        if total > self.policy.max_total_expanded_bytes {
            return Err(VerifierError::ResourceLimit {
                limit: "max_total_expanded_bytes".to_owned(),
            });
        }
        let dataset_bytes = dataset_hasher.finalize();
        let mut raw_digest = [0_u8; 32];
        raw_digest.copy_from_slice(&dataset_bytes);
        let data_digest = hex_lower(&raw_digest);
        if data_digest != manifest.digests.data_sha256 {
            return Err(VerifierError::DigestMismatch {
                detail: "data_sha256 differs from ordered uncompressed streams".to_owned(),
            });
        }
        Ok(StageFive {
            bindings,
            data_digest,
            records: all_records,
        })
    }

    fn check_artifact_declarations<'a>(
        &self,
        manifest: &'a ReleaseManifest,
    ) -> Result<Vec<&'a ArtifactDescriptor>, VerifierError> {
        if manifest.artifacts.is_empty()
            || manifest.artifacts.len() as u64 > self.policy.max_artifact_count
        {
            return Err(VerifierError::ResourceLimit {
                limit: "max_artifact_count".to_owned(),
            });
        }
        let mut descriptors: Vec<&ArtifactDescriptor> = manifest.artifacts.iter().collect();
        descriptors.sort_by(|left, right| {
            left.logical_partition_id
                .cmp(&right.logical_partition_id)
                .then(left.shard_ordinal.cmp(&right.shard_ordinal))
        });
        let mut paths = HashSet::new();
        let mut identities = HashSet::new();
        let mut previous: Option<(&str, u32)> = None;
        for descriptor in &descriptors {
            artifact_component(&descriptor.path)?;
            if !paths.insert(&descriptor.path) {
                return Err(VerifierError::FileSet {
                    detail: "duplicate normalized artifact path".to_owned(),
                });
            }
            let identity = (
                descriptor.logical_partition_id.as_str(),
                descriptor.shard_ordinal,
            );
            if !identities.insert(identity) {
                return Err(VerifierError::FileSet {
                    detail: "duplicate logical partition and shard ordinal".to_owned(),
                });
            }
            if let Some(earlier) = previous
                && earlier >= (identity.0, identity.1)
            {
                return Err(VerifierError::ManifestSchema {
                    detail: "artifacts are not in canonical ascending order".to_owned(),
                });
            }
            previous = Some(identity);
            if descriptor.encoding != RECORD_ENCODING
                || !matches!(
                    descriptor.record_order_profile.as_str(),
                    SUPERSEDES_STRICT | SUPERSEDES_GAPS | REFERENCE_KEY
                )
            {
                return Err(VerifierError::ManifestSchema {
                    detail: "artifact encoding or record order profile is unknown".to_owned(),
                });
            }
            let compression_ok = match descriptor.compression.as_str() {
                "none" | "gzip" => descriptor.compression_version == 1,
                _ => false,
            };
            if !compression_ok {
                return Err(VerifierError::Decompression {
                    detail: "compression declaration is unknown".to_owned(),
                });
            }
            if descriptor.stored_bytes == 0
                || descriptor.stored_bytes > self.policy.max_stored_bytes
                || descriptor.uncompressed_bytes == 0
                || descriptor.uncompressed_bytes > self.policy.max_expanded_bytes
                || descriptor.record_count == 0
                || descriptor.record_count > self.policy.max_record_count
            {
                return Err(VerifierError::ResourceLimit {
                    limit: "artifact declared size or record count exceeds policy".to_owned(),
                });
            }
            parse_hex_lower(&descriptor.stored_sha256, "stored_sha256")?;
            parse_hex_lower(&descriptor.uncompressed_sha256, "uncompressed_sha256")?;
        }
        Ok(descriptors)
    }

    fn stages_six_to_twelve(
        &self,
        manifest: &ReleaseManifest,
        sidecar: &SignatureSidecar,
        manifest_digest: &str,
        stage_five: &StageFive,
        snapshot: &PrivateSnapshot,
        request: &AdmissionRequest,
    ) -> Result<AdmissionReceipt, VerifierError> {
        verify_snapshot_binding(snapshot, stage_five)?;
        let rights = self.rights.evaluate(manifest, request);
        if rights.decision != Decision::Permit {
            return Err(VerifierError::RightsNotPermitted);
        }
        if rights.matching_grant_ids.len() != 1 {
            return Err(VerifierError::AmbiguousGrants);
        }
        self.note_stage(6);
        self.stage_seven(manifest, request, stage_five)?;
        self.note_stage(7);
        self.stage_eight(manifest)?;
        self.note_stage(8);
        self.stage_nine(manifest, stage_five)?;
        self.note_stage(9);
        self.stage_ten(manifest, request)?;
        self.note_stage(10);
        self.stage_eleven(manifest, sidecar, manifest_digest, stage_five, request)?;
        self.note_stage(11);
        let receipt = AdmissionReceipt::new(crate::receipt::ReceiptInput {
            release_id: manifest.release_identity.release_id.clone(),
            manifest_sha256: manifest_digest.to_owned(),
            data_sha256: stage_five.data_digest.clone(),
            publisher_id: manifest.publication_identity.publisher_identity.clone(),
            key_id: sidecar.key_id.clone(),
            trust_registry_revision: self.registry.revision().to_owned(),
            contract_version: manifest.contract_version.clone(),
            contract_revision: manifest.contract_revision.clone(),
            verification_policy_version: VERIFICATION_POLICY_VERSION.to_owned(),
            artifact_set: stage_five.bindings.clone(),
            rights_decision: rights,
            coverage_decision: self.coverage.evaluate(manifest, request),
            admission_request: request.clone(),
            consumer_use_profile: request.use_profile.clone(),
            verifier_commit: self.verifier_commit.clone(),
        })?;
        self.note_stage(12);
        Ok(receipt)
    }

    fn stage_seven(
        &self,
        manifest: &ReleaseManifest,
        request: &AdmissionRequest,
        stage_five: &StageFive,
    ) -> Result<(), VerifierError> {
        let coverage = self.coverage.evaluate(manifest, request);
        if coverage.used_implicit_all {
            return Err(VerifierError::Coverage {
                detail: "coverage evaluator relied on implicit ALL".to_owned(),
            });
        }
        let declared: HashSet<&str> = manifest
            .coverage
            .dimensions
            .iter()
            .map(|dimension| dimension.dimension_id.as_str())
            .collect();
        for required in COVERAGE_DIMENSIONS {
            if !declared.contains(required) {
                return Err(VerifierError::Coverage {
                    detail: format!("coverage dimension {} is absent", required),
                });
            }
        }
        for dimension in &manifest.coverage.dimensions {
            let missing_denominator = match dimension.denominator {
                Some(value) => value == 0,
                None => true,
            };
            if missing_denominator || dimension.numerator > dimension.denominator.unwrap_or(0) {
                return Err(VerifierError::Coverage {
                    detail: format!(
                        "dimension {} has no usable denominator",
                        dimension.dimension_id
                    ),
                });
            }
            if dimension.denominator_identity.is_empty()
                || dimension.universe_name.is_empty()
                || dimension.evidence_reference.is_empty()
                || dimension.as_of_ns != manifest.release_identity.as_of_ns
            {
                return Err(VerifierError::Coverage {
                    detail: format!("dimension {} is incomplete", dimension.dimension_id),
                });
            }
            if dimension.verdict == "PASS"
                && !coverage
                    .approved_dimensions
                    .contains(&dimension.dimension_id)
            {
                return Err(VerifierError::Coverage {
                    detail: format!(
                        "dimension {} PASS is not established",
                        dimension.dimension_id
                    ),
                });
            }
        }
        if stage_five.bindings.is_empty() {
            return Err(VerifierError::Coverage {
                detail: "no verified artifact supports coverage".to_owned(),
            });
        }
        Ok(())
    }

    fn stage_eight(&self, manifest: &ReleaseManifest) -> Result<(), VerifierError> {
        let activity = &manifest.producer_activity;
        if activity.performed_operations.is_empty() || activity.operation_evidence.is_empty() {
            return Err(VerifierError::ProducerActivity {
                detail: "operation or evidence list is empty".to_owned(),
            });
        }
        let operation_refs: HashSet<&str> = activity
            .performed_operations
            .iter()
            .map(|operation| operation.operation_ref.as_str())
            .collect();
        for evidence in &activity.operation_evidence {
            if !operation_refs.contains(evidence.operation_ref.as_str())
                || evidence.event_id.is_empty()
                || evidence.run_id.is_empty()
                || evidence.decision_key.is_empty()
                || evidence.recorded_at_ns == 0
                || evidence.recorded_at_ns > manifest.release_identity.as_of_ns
            {
                return Err(VerifierError::ProducerActivity {
                    detail: "evidence reference does not identify a recorded operation".to_owned(),
                });
            }
            parse_hex_lower(&evidence.digest_sha256, "operation evidence digest")?;
        }
        Ok(())
    }

    fn stage_nine(
        &self,
        manifest: &ReleaseManifest,
        stage_five: &StageFive,
    ) -> Result<(), VerifierError> {
        let versions = &manifest.schema_versions;
        if versions.contract_version != manifest.contract_version
            || versions.envelope_version.is_empty()
            || versions.payload_schema_version.is_empty()
            || versions.normalizer_version.is_empty()
        {
            return Err(VerifierError::Envelope {
                detail: "declared schema versions are missing or inconsistent".to_owned(),
            });
        }
        for records in &stage_five.records {
            for record in records {
                if record.envelope_schema_version != versions.envelope_version {
                    return Err(VerifierError::Envelope {
                        detail: "record envelope version differs from the manifest".to_owned(),
                    });
                }
            }
        }
        Ok(())
    }

    fn stage_ten(
        &self,
        manifest: &ReleaseManifest,
        request: &AdmissionRequest,
    ) -> Result<(), VerifierError> {
        let lineage = &manifest.lineage;
        if lineage.raw_archive_reference.is_empty()
            || lineage.transformation_record.is_empty()
            || lineage.was_derived_from.is_empty()
            || lineage.had_primary_source.is_empty()
            || lineage.knowledge_time_ns == 0
            || lineage.revision == 0
        {
            return Err(VerifierError::Lineage {
                detail: "source lineage declaration is incomplete".to_owned(),
            });
        }
        parse_hex_lower(&lineage.raw_archive_sha256, "raw_archive_sha256")?;
        if lineage.knowledge_time_ns > manifest.release_identity.as_of_ns {
            return Err(VerifierError::PointInTime {
                detail: "knowledge time is after release as_of_ns".to_owned(),
            });
        }
        if request.as_of_ns < lineage.knowledge_time_ns
            || request.as_of_ns > manifest.release_identity.as_of_ns
        {
            return Err(VerifierError::PointInTime {
                detail: "request as_of_ns is outside the knowledge basis".to_owned(),
            });
        }
        let supersedes = match &lineage.supersedes {
            Some(value) => value,
            None => {
                if lineage.invalidated_at_time_ns != 0
                    && lineage.invalidated_at_time_ns < lineage.knowledge_time_ns
                {
                    return Err(VerifierError::PointInTime {
                        detail: "invalidation precedes knowledge".to_owned(),
                    });
                }
                return Ok(());
            }
        };
        if supersedes.supersedes_release_id == manifest.release_identity.release_id
            || supersedes.supersedes_release_id.is_empty()
            || supersedes.revision == 0
            || supersedes.revision >= lineage.revision
            || supersedes.correction_reason.is_empty()
        {
            return Err(VerifierError::Lineage {
                detail: "supersedes relation is self-referential or regresses revision".to_owned(),
            });
        }
        if supersedes.knowledge_time_ns == 0
            || supersedes.knowledge_time_ns > lineage.knowledge_time_ns
            || supersedes.knowledge_time_ns > manifest.release_identity.as_of_ns
            || supersedes.invalidated_at_time_ns < supersedes.knowledge_time_ns
        {
            return Err(VerifierError::PointInTime {
                detail: "correction knowledge or invalidation time is inconsistent".to_owned(),
            });
        }
        parse_hex_lower(
            &supersedes.supersedes_manifest_sha256,
            "supersedes manifest digest",
        )?;
        parse_hex_lower(
            &supersedes.supersedes_data_sha256,
            "supersedes dataset digest",
        )?;
        self.walk_lineage(supersedes, manifest.release_identity.release_id.as_str())
    }

    fn walk_lineage(
        &self,
        relation: &crate::schema::SupersedesRelation,
        current_release_id: &str,
    ) -> Result<(), VerifierError> {
        let mut visited = HashSet::new();
        visited.insert(current_release_id.to_owned());
        let mut next = Some(relation.supersedes_release_id.clone());
        let mut expected_revision = relation.revision;
        let mut expected_manifest = relation.supersedes_manifest_sha256.clone();
        let mut expected_data = relation.supersedes_data_sha256.clone();
        while let Some(release_id) = next {
            if !visited.insert(release_id.clone()) {
                return Err(VerifierError::Lineage {
                    detail: "correction lineage contains a cycle".to_owned(),
                });
            }
            let evidence =
                self.lineage
                    .predecessor(&release_id)
                    .ok_or_else(|| VerifierError::Lineage {
                        detail: "predecessor is absent from the trusted archive".to_owned(),
                    })?;
            if evidence.release_id != release_id
                || evidence.manifest_sha256 != expected_manifest
                || evidence.data_sha256 != expected_data
                || evidence.revision != expected_revision
            {
                return Err(VerifierError::Lineage {
                    detail: "predecessor evidence disagrees with the declared relation".to_owned(),
                });
            }
            if evidence.knowledge_time_ns > relation.knowledge_time_ns {
                return Err(VerifierError::PointInTime {
                    detail: "predecessor knowledge time is after the correction".to_owned(),
                });
            }
            next = evidence.supersedes_release_id.clone();
            expected_revision = evidence.revision.saturating_sub(1);
            expected_manifest = evidence.manifest_sha256;
            expected_data = evidence.data_sha256;
        }
        Ok(())
    }

    fn stage_eleven(
        &self,
        manifest: &ReleaseManifest,
        sidecar: &SignatureSidecar,
        manifest_digest: &str,
        stage_five: &StageFive,
        request: &AdmissionRequest,
    ) -> Result<(), VerifierError> {
        if request.release_id != manifest.release_identity.release_id
            || request.manifest_sha256 != manifest_digest
            || request.data_sha256 != stage_five.data_digest
            || request.publisher_identity != manifest.publication_identity.publisher_identity
        {
            return Err(VerifierError::UseScope {
                detail: "request release identity or digests differ from verified values"
                    .to_owned(),
            });
        }
        let required: &[&str] = match request.use_profile.as_str() {
            "reference_lookup" => &["read_release"],
            "historical_research" | "intraday_replay_research" => &[
                "read_release",
                "store_permitted_research_copy",
                "replay",
                "internal_analytics",
            ],
            "derived_research_dataset" => &[
                "read_release",
                "store_permitted_research_copy",
                "internal_analytics",
                "retain_permitted_derived_data",
                "create_derived",
            ],
            "portfolio_simulation" => &["read_release", "replay", "internal_analytics"],
            _ => {
                return Err(VerifierError::UseScope {
                    detail: "use profile is not declared".to_owned(),
                });
            }
        };
        let requested: HashSet<&str> = request
            .requested_actions
            .iter()
            .map(String::as_str)
            .collect();
        let expected: HashSet<&str> = required.iter().copied().collect();
        if requested != expected {
            return Err(VerifierError::UseScope {
                detail: "requested actions are not exactly the profile actions".to_owned(),
            });
        }
        let needed_capability = if request.use_profile == "intraday_replay_research" {
            Some("intraday_capability")
        } else {
            None
        };
        if let Some(capability) = needed_capability {
            let present = request
                .declared_capabilities
                .iter()
                .any(|claim| claim.capability_id == capability);
            if !present {
                return Err(VerifierError::Capability {
                    detail: format!("capability {} is not declared", capability),
                });
            }
        }
        if request.purpose != DECLARED_PURPOSE
            || request.actor.is_empty()
            || request.licensee.is_empty()
            || request.recipient_identity.is_empty()
        {
            return Err(VerifierError::UseScope {
                detail: "request purpose or identity does not match the reviewed terms".to_owned(),
            });
        }
        if request.scope.subject_state != "NAMED" || request.scope.named_subjects.is_empty() {
            return Err(VerifierError::UseScope {
                detail: "runtime request must use a non-empty NAMED subject set".to_owned(),
            });
        }
        match (
            request.scope.data_time_from_ns,
            request.scope.data_time_to_ns,
        ) {
            (Some(from), Some(to)) if from <= to => {}
            _ => {
                return Err(VerifierError::UseScope {
                    detail: "request data-time window is unbounded or inverted".to_owned(),
                });
            }
        }
        if request.requested_retention.max_duration_ns == 0
            || request.requested_retention.purge_rule.is_empty()
        {
            return Err(VerifierError::Retention);
        }
        if sidecar.algorithm != "Ed25519" {
            return Err(VerifierError::SidecarSchema {
                detail: "sidecar algorithm is not Ed25519".to_owned(),
            });
        }
        Ok(())
    }
}

struct ParsedStages {
    manifest: ReleaseManifest,
    sidecar: SignatureSidecar,
    manifest_digest: String,
}

struct StageFive {
    bindings: Vec<ArtifactBinding>,
    data_digest: String,
    records: Vec<Vec<ReleaseRecord>>,
}

fn verify_canonical_record_stream(
    bytes: &[u8],
    descriptor: &ArtifactDescriptor,
    profiles: &HashMap<String, ArtifactRecordOrderProfile>,
    event_types: &HashSet<String>,
    policy: &VerifierResourcePolicy,
) -> Result<Vec<ReleaseRecord>, VerifierError> {
    if bytes.is_empty() || bytes.last() != Some(&b'\n') {
        return Err(VerifierError::RecordStream {
            detail: "canonical record stream lacks a trailing LF boundary".to_owned(),
        });
    }
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) || bytes.contains(&0x0D) {
        return Err(VerifierError::RecordStream {
            detail: "canonical record stream contains a BOM or CR byte".to_owned(),
        });
    }
    let profile = profiles
        .get(&descriptor.path)
        .ok_or_else(|| VerifierError::RecordOrder {
            detail: "artifact has no canonical order profile".to_owned(),
        })?;
    if descriptor.encoding != RECORD_ENCODING {
        return Err(VerifierError::ManifestSchema {
            detail: "canonical record encoding is not registered".to_owned(),
        });
    }
    let mut records = Vec::new();
    let mut keys = Vec::new();
    for line in bytes[..bytes.len() - 1].split(|byte| *byte == b'\n') {
        if line.is_empty() {
            return Err(VerifierError::RecordStream {
                detail: "canonical record stream contains a blank line".to_owned(),
            });
        }
        if line.len() as u64 > policy.max_record_bytes {
            return Err(VerifierError::ResourceLimit {
                limit: "max_record_bytes".to_owned(),
            });
        }
        let key = canonical_wire::derive_key_from_bytes(line, profile)
            .map_err(canonical_wire::CanonicalError::into_record_error)?;
        let value = parse_strict(line).map_err(|error| VerifierError::RecordStream {
            detail: error.to_string(),
        })?;
        check_depth(&value, policy.max_json_nesting)?;
        let record = canonical_wire::parse_record(&value)
            .map_err(canonical_wire::CanonicalError::into_record_error)?;
        if !event_types.contains(record.event_type.as_str()) {
            return Err(VerifierError::RecordStream {
                detail: "record event type is absent from the payload registry".to_owned(),
            });
        }
        keys.push(key);
        records.push(canonical_wire::into_internal_record(&record));
    }
    canonical_wire::validate_order_sequence(&keys)
        .map_err(canonical_wire::CanonicalError::into_record_error)?;
    if records.len() as u64 != descriptor.record_count {
        return Err(VerifierError::RecordCount);
    }
    Ok(records)
}

fn verify_snapshot_binding(
    snapshot: &PrivateSnapshot,
    stage_five: &StageFive,
) -> Result<(), VerifierError> {
    let expected_count = stage_five.bindings.len() + 2;
    if snapshot.entries().len() != expected_count {
        return Err(VerifierError::SnapshotMismatch);
    }
    for entry in snapshot.entries() {
        if sha256(&entry.bytes) != entry.sha256 {
            return Err(VerifierError::SnapshotMismatch);
        }
    }
    for binding in &stage_five.bindings {
        let component = artifact_component(&binding.path)?;
        let entry = snapshot.entry(&component)?;
        if hex_lower(&entry.sha256) != binding.stored_sha256
            || entry.bytes.len() as u64 != binding.stored_bytes
        {
            return Err(VerifierError::SnapshotMismatch);
        }
    }
    Ok(())
}

fn check_depth(value: &Value, maximum: u32) -> Result<(), VerifierError> {
    if json_depth(value) > maximum {
        return Err(VerifierError::ResourceLimit {
            limit: "max_json_nesting".to_owned(),
        });
    }
    Ok(())
}

fn classify_decode_error(error: serde_json::Error) -> VerifierError {
    let text = error.to_string();
    if text.contains("duplicate field") {
        VerifierError::DuplicateMember { member: text }
    } else if text.contains("unknown field") {
        VerifierError::UnknownMember { member: text }
    } else {
        VerifierError::ManifestSchema { detail: text }
    }
}

fn decompress(
    stored: &[u8],
    descriptor: &ArtifactDescriptor,
    policy: &VerifierResourcePolicy,
) -> Result<Vec<u8>, VerifierError> {
    if descriptor.compression == "none" {
        if stored.len() as u64 > policy.max_decompression_budget {
            return Err(VerifierError::ResourceLimit {
                limit: "max_decompression_budget".to_owned(),
            });
        }
        return Ok(stored.to_vec());
    }
    let budget = policy
        .max_decompression_budget
        .min(policy.max_expanded_bytes)
        .min(descriptor.uncompressed_bytes);
    let decoder = GzDecoder::new(stored);
    let mut expanded = Vec::new();
    let read = decoder
        .take(budget.saturating_add(1))
        .read_to_end(&mut expanded)
        .map_err(|error| VerifierError::Decompression {
            detail: error.to_string(),
        })?;
    if read as u64 > budget {
        return Err(VerifierError::ResourceLimit {
            limit: "max_decompression_budget".to_owned(),
        });
    }
    Ok(expanded)
}

fn verify_record_stream(
    bytes: &[u8],
    descriptor: &ArtifactDescriptor,
    policy: &VerifierResourcePolicy,
) -> Result<Vec<ReleaseRecord>, VerifierError> {
    if bytes.is_empty() || bytes.last() != Some(&b'\n') {
        return Err(VerifierError::RecordStream {
            detail: "record stream lacks a trailing LF boundary".to_owned(),
        });
    }
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) || bytes.contains(&0x0D) {
        return Err(VerifierError::RecordStream {
            detail: "record stream contains a BOM or CR byte".to_owned(),
        });
    }
    let profiles = [SUPERSEDES_STRICT, SUPERSEDES_GAPS, REFERENCE_KEY];
    if !profiles.contains(&descriptor.record_order_profile.as_str()) {
        return Err(VerifierError::ManifestSchema {
            detail: "record order profile is unknown".to_owned(),
        });
    }
    let mut records = Vec::new();
    let mut previous_sequence: Option<(String, String, u64)> = None;
    let mut previous_reference: Option<(String, u64, u64, String)> = None;
    for line in bytes[..bytes.len() - 1].split(|byte| *byte == b'\n') {
        if line.is_empty() {
            return Err(VerifierError::RecordStream {
                detail: "record stream contains a blank line".to_owned(),
            });
        }
        if line.len() as u64 > policy.max_record_bytes {
            return Err(VerifierError::ResourceLimit {
                limit: "max_record_bytes".to_owned(),
            });
        }
        let text = std::str::from_utf8(line).map_err(|_| VerifierError::RecordStream {
            detail: "record line is not UTF-8".to_owned(),
        })?;
        let value = parse_strict(line).map_err(|_| VerifierError::RecordStream {
            detail: "record line is not a strict JSON object".to_owned(),
        })?;
        check_depth(&value, policy.max_json_nesting)?;
        if canonical_bytes(&value) != line {
            return Err(VerifierError::RecordStream {
                detail: "record line is not RFC 8785 canonical bytes".to_owned(),
            });
        }
        let record: ReleaseRecord =
            serde_json::from_str(text).map_err(|_| VerifierError::RecordStream {
                detail: "record line does not match the record schema".to_owned(),
            })?;
        check_record_order(
            &record,
            descriptor.record_order_profile.as_str(),
            &mut previous_sequence,
            &mut previous_reference,
        )?;
        records.push(record);
    }
    if records.len() as u64 != descriptor.record_count {
        return Err(VerifierError::RecordCount);
    }
    Ok(records)
}

fn check_record_order(
    record: &ReleaseRecord,
    profile: &str,
    previous_sequence: &mut Option<(String, String, u64)>,
    previous_reference: &mut Option<(String, u64, u64, String)>,
) -> Result<(), VerifierError> {
    if profile == REFERENCE_KEY {
        let key = (
            record.object_id.clone(),
            record.revision,
            record.knowledge_time_ns,
            record.assertion_id.clone(),
        );
        if let Some(earlier) = previous_reference
            && *earlier >= key
        {
            return Err(VerifierError::RecordOrder {
                detail: "reference key ordering violation or duplicate position".to_owned(),
            });
        }
        *previous_reference = Some(key);
        return Ok(());
    }
    if record.sequence_domain_id.is_empty() || record.sequence_epoch.is_empty() {
        return Err(VerifierError::RecordOrder {
            detail: "sequence domain or epoch is empty".to_owned(),
        });
    }
    let key = (
        record.sequence_domain_id.clone(),
        record.sequence_epoch.clone(),
        record.sequence_number,
    );
    if let Some(earlier) = previous_sequence {
        if earlier.0 == key.0 && earlier.1 == key.1 {
            if earlier.2 == key.2 {
                return Err(VerifierError::RecordOrder {
                    detail: "duplicate sequence position".to_owned(),
                });
            }
            if profile == SUPERSEDES_STRICT && key.2 != earlier.2 + 1 {
                return Err(VerifierError::RecordOrder {
                    detail: "sequence gap under a strict gap policy".to_owned(),
                });
            }
        } else if *earlier >= key {
            return Err(VerifierError::RecordOrder {
                detail: "records are not in ascending sequence-domain order".to_owned(),
            });
        }
    }
    *previous_sequence = Some(key);
    Ok(())
}
