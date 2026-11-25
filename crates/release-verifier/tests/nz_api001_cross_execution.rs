// SPDX-License-Identifier: Apache-2.0
//! NZ-API-001 cross-execution harness for the B15 streaming producer.
//!
//! The harness invokes a Fieldbook B15 streaming producer binary when one is
//! supplied and feeds its release to the real `verify_release_streaming`
//! method. All profiles and keys here are finite `TEST_ONLY` fixtures; no
//! provider, rights grant, market observation, experiment, India artefact or
//! live route is created. When no producer binary is present the harness records
//! an explicit skip reason and produces no simulated producer output.

use ed25519_dalek::SigningKey;
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use release_verifier::AdmissionReceipt;
use release_verifier::AdmissionRequest;
use release_verifier::ApprovedProfileRegistry;
use release_verifier::CleanupDisposition;
use release_verifier::Decision;
use release_verifier::DeclaredPayloadSchema;
use release_verifier::KeyState;
use release_verifier::LimitsInput;
use release_verifier::LocalSchemaInput;
use release_verifier::PayloadRegistryError;
use release_verifier::PayloadRegistryInput;
use release_verifier::ProcessingRole;
use release_verifier::ProfileResolver;
use release_verifier::RefusalDiagnostic;
use release_verifier::ReleaseLocator;
use release_verifier::ReleaseVerifier;
use release_verifier::ResourceProfileId;
use release_verifier::TrustedKeyEntry;
use release_verifier::TrustedKeyRegistry;
use release_verifier::VerifierConfig;
use release_verifier::VerifierResourcePolicy;
use release_verifier::canonical_wire;
use release_verifier::canonical_wire::Payload;
use release_verifier::files;
use release_verifier::jcs;
use release_verifier::payload_registry;
use release_verifier::payload_registry::PayloadSchemaRegistry;
use release_verifier::schema::CoverageDecision;
use release_verifier::schema::CoverageEvaluator;
use release_verifier::schema::LineageResolver;
use release_verifier::schema::PredecessorEvidence;
use release_verifier::schema::ReleaseManifest;
use release_verifier::schema::RightsDecision;
use release_verifier::schema::RightsEvaluator;
use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::io::Read as IoRead;
use std::io::Write as IoWrite;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::Mutex;
use tempfile::TempDir;

/// Environment variable naming the Fieldbook B15 producer executable.
const ENV_BINARY: &str = "NIRNAY_B15_PRODUCER_BIN";
/// Environment variable carrying the Fieldbook commit for the evidence record.
const ENV_COMMIT: &str = "NIRNAY_B15_PRODUCER_COMMIT";
/// Published conformance seed used by the Fieldbook B15 test producer.
const PRODUCER_TEST_SEED: [u8; 32] = [0x51; 32];
/// Stable event type for the local TEST_ONLY payload schema fixtures.
const LOCAL_EVENT_TYPE: &str = "test-only-event";
/// Event type used to force a local registry miss.
const MISS_EVENT_TYPE: &str = "test-only-unregistered-event";

fn test_limits() -> LimitsInput {
    LimitsInput {
        max_record_bytes: 4_096,
        max_records_per_artifact: 128,
        max_records_per_release: 512,
        max_stored_bytes_per_artifact: 2_000_000,
        max_stored_bytes_per_release: 8_000_000,
        max_expanded_bytes_per_artifact: 2_000_000,
        max_expanded_bytes_per_release: 8_000_000,
        max_decompression_budget_bytes: 2_000_000,
        max_expansion_ratio: 64,
        codec_window_bytes: 32_768,
        io_chunk_bytes: 16,
        max_nesting_depth: 16,
        max_string_bytes: 1_024,
        max_token_bytes: 256,
        max_object_members: 64,
        max_array_elements: 256,
        max_concurrent_artifacts: 4,
        worker_count: 4,
        queue_capacity_records: 64,
        temp_disk_bytes: 8_000_000,
        max_processing_duration_ms: 120_000,
        max_journal_record_bytes: 65_536,
        max_preparation_intent_bytes: 262_144,
        max_sealed_intent_bytes: 262_144,
        max_open_file_descriptors: 16,
        memory_budget_bytes: 268_435_456,
        max_artifacts: 8,
    }
}

fn test_resolver() -> Result<ProfileResolver, Box<dyn Error>> {
    let mut registry = ApprovedProfileRegistry::new();
    registry.register_test_profile(
        ResourceProfileId::new("TEST_ONLY_NZ_API001_VERIFIER_V1", 1)?,
        ProcessingRole::Verifier,
        test_limits(),
    )?;
    registry.register_test_profile(
        ResourceProfileId::new("TEST_ONLY_NZ_API001_SNAPSHOT_V1", 1)?,
        ProcessingRole::Snapshot,
        test_limits(),
    )?;
    Ok(ProfileResolver::new(registry))
}

struct UnknownRights;

impl RightsEvaluator for UnknownRights {
    fn evaluate(&self, _manifest: &ReleaseManifest, _request: &AdmissionRequest) -> RightsDecision {
        RightsDecision {
            decision: Decision::Unknown,
            matching_grant_ids: Vec::new(),
        }
    }
}

struct PassCoverage;

impl CoverageEvaluator for PassCoverage {
    fn evaluate(
        &self,
        manifest: &ReleaseManifest,
        _request: &AdmissionRequest,
    ) -> CoverageDecision {
        CoverageDecision {
            approved_dimensions: manifest
                .coverage
                .dimensions
                .iter()
                .map(|item| item.dimension_id.clone())
                .collect(),
            used_implicit_all: false,
        }
    }
}

struct EmptyLineage;

impl LineageResolver for EmptyLineage {
    fn predecessor(&self, _release_id: &str) -> Option<PredecessorEvidence> {
        None
    }
}

/// One real producer invocation and the digests recorded for the evidence log.
struct ProducerArtifact {
    _temp: TempDir,
    root: PathBuf,
    release_dir: PathBuf,
    binary: PathBuf,
    command: String,
    binary_sha256: String,
    producer_commit: String,
    stdout: String,
}

fn skip_reason() -> &'static str {
    "NIRNAY_B15_PRODUCER_BIN is unset or does not name an executable Fieldbook B15 streaming producer; no producer output was fabricated"
}

fn producer_binary() -> Option<PathBuf> {
    let supplied = std::env::var_os(ENV_BINARY)?;
    let candidate = PathBuf::from(supplied);
    let metadata = candidate.metadata().ok()?;
    if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 {
        Some(candidate)
    } else {
        None
    }
}

fn find_release_dir(dir: &Path, depth: u8) -> Result<Option<PathBuf>, Box<dyn Error>> {
    if dir.join("manifest.json").is_file()
        && dir.join("signature.json").is_file()
        && dir.join("artifacts").is_dir()
    {
        return Ok(Some(dir.to_path_buf()));
    }
    if depth == 0 {
        return Ok(None);
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir()
            && let Some(found) = find_release_dir(&entry.path(), depth - 1)?
        {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

/// Invoke the real producer. `None` means the binary was absent; absent output
/// is reported as a skip and is never replaced with a local fixture.
fn run_producer() -> Result<Option<ProducerArtifact>, Box<dyn Error>> {
    let Some(binary) = producer_binary() else {
        eprintln!("SKIP NZ-API-001 cross-execution: {}", skip_reason());
        return Ok(None);
    };
    let temp = TempDir::new()?;
    let root = temp.path().join("produced-releases");
    fs::create_dir_all(&root)?;
    let command = format!("{} --out {}", binary.display(), root.display());
    let output = Command::new(&binary).arg("--out").arg(&root).output()?;
    if !output.status.success() {
        return Err(format!(
            "producer exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    let release_dir = find_release_dir(&root, 4)?
        .ok_or("producer supplied no directory with manifest, sidecar and artifacts")?;
    let binary_bytes = fs::read(&binary)?;
    let binary_sha256 = files::hex_lower(&files::sha256(&binary_bytes));
    let producer_commit = std::env::var(ENV_COMMIT).unwrap_or_else(|_| "not-supplied".to_owned());
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    eprintln!("NZ-API-001 producer method: Fieldbook publish_streaming");
    eprintln!("NZ-API-001 producer command: {}", command);
    eprintln!(
        "NZ-API-001 producer binary: {} sha256={}",
        binary.display(),
        binary_sha256
    );
    eprintln!("NZ-API-001 producer commit: {}", producer_commit);
    eprintln!("NZ-API-001 producer stdout: {}", stdout);
    Ok(Some(ProducerArtifact {
        _temp: temp,
        root,
        release_dir,
        binary,
        command,
        binary_sha256,
        producer_commit,
        stdout,
    }))
}

fn provenance(artifact: &ProducerArtifact) -> String {
    format!(
        "binary={} sha256={} commit={} command={} stdout={}",
        artifact.binary.display(),
        artifact.binary_sha256,
        artifact.producer_commit,
        artifact.command,
        artifact.stdout
    )
}

fn sha256_text(bytes: &[u8]) -> String {
    files::hex_lower(&files::sha256(bytes))
}

fn artifact_files(dir: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            files.push(entry.path());
        }
    }
    files.sort();
    Ok(files)
}

fn snapshot_release(dir: &Path) -> Result<HashMap<PathBuf, Vec<u8>>, Box<dyn Error>> {
    let mut saved = HashMap::new();
    for path in [
        dir.join("manifest.json"),
        dir.join("signature.json"),
        artifact_files(&dir.join("artifacts"))?
            .first()
            .ok_or("release has no artifact")?
            .clone(),
    ] {
        saved.insert(path.clone(), fs::read(&path)?);
    }
    Ok(saved)
}

fn restore_release(saved: &HashMap<PathBuf, Vec<u8>>) -> Result<(), Box<dyn Error>> {
    for (path, bytes) in saved {
        fs::write(path, bytes)?;
    }
    Ok(())
}

fn verify_context(
    artifact: &ProducerArtifact,
    staging: &Path,
    rights: Box<dyn RightsEvaluator>,
    resolver: ProfileResolver,
    canonical: bool,
    registry_input: Option<PayloadRegistryInput>,
    observed: Arc<Mutex<Vec<u8>>>,
) -> Result<(ReleaseLocator, AdmissionRequest, ReleaseVerifier), Box<dyn Error>> {
    let manifest_bytes = fs::read(artifact.release_dir.join("manifest.json"))?;
    let manifest_value = jcs::parse_strict(&manifest_bytes)?;
    let parsed = canonical_wire::parse_manifest(&manifest_value)
        .map_err(canonical_wire::CanonicalError::into_manifest_error)?;
    let digest = sha256_text(&jcs::canonical_bytes(&manifest_value));
    let request = canonical_wire::into_internal_request(&parsed, &digest)?;
    let locator = ReleaseLocator {
        publisher_id: parsed
            .manifest
            .publication_identity
            .publisher_identity
            .as_str()
            .to_owned(),
        dataset_id: parsed.manifest.source_scope.dataset_id.as_str().to_owned(),
        release_id: parsed
            .manifest
            .release_identity
            .release_id
            .as_str()
            .to_owned(),
    };
    let sidecar_value: Value =
        serde_json::from_slice(&fs::read(artifact.release_dir.join("signature.json"))?)?;
    let key_id = sidecar_value["key_id"]
        .as_str()
        .ok_or("producer sidecar has no key id")?
        .to_owned();
    let verifying = SigningKey::from_bytes(&PRODUCER_TEST_SEED).verifying_key();
    let registry = TrustedKeyRegistry::new(
        vec![TrustedKeyEntry {
            publisher_id: locator.publisher_id.clone(),
            key_id,
            public_key_bytes: verifying.to_bytes(),
            algorithm: "Ed25519".to_owned(),
            purpose: release_verifier::trust::KeyPurpose::ReleaseSigning,
            valid_from_ns: 1,
            valid_until_ns: u64::MAX,
            state: KeyState::Active,
            evidence_reference: None,
        }],
        "test-only-fieldbook-producer-keys".to_owned(),
    )?;
    let mut verifier = ReleaseVerifier::new(VerifierConfig {
        root: artifact.root.clone(),
        private_staging: staging.to_path_buf(),
        registry,
        rights,
        coverage: Box::new(PassCoverage),
        lineage: Box::new(EmptyLineage),
        policy: VerifierResourcePolicy {
            max_manifest_bytes: 1_000_000,
            max_sidecar_bytes: 4_096,
            max_artifact_count: 8,
            max_stored_bytes: 2_000_000,
            max_expanded_bytes: 2_000_000,
            max_total_expanded_bytes: 8_000_000,
            max_record_bytes: 4_096,
            max_record_count: 128,
            max_json_nesting: 16,
            max_decompression_budget: 2_000_000,
            max_temp_disk_bytes: 8_000_000,
            max_open_file_descriptors: 16,
        },
        verifier_commit: "nz-api001-cross-execution".to_owned(),
    })
    .with_test_profile_resolver(resolver)
    .with_stage_observer(move |stage| {
        if let Ok(mut seen) = observed.lock() {
            seen.push(stage);
        }
    });
    if canonical {
        verifier = verifier.with_canonical_wire();
    }
    if let Some(input) = registry_input {
        verifier = verifier.with_payload_schema_registry(input);
    }
    Ok((locator, request, verifier))
}

fn staging_dir(artifact: &ProducerArtifact) -> Result<PathBuf, Box<dyn Error>> {
    let staging = artifact._temp.path().join("private-staging");
    fs::create_dir_all(&staging)?;
    Ok(staging)
}

fn assert_typed_refusal(
    result: &Result<AdmissionReceipt, RefusalDiagnostic>,
    code: &str,
    stage: u8,
    detail: &str,
) -> Result<(), Box<dyn Error>> {
    let refusal = match result {
        Ok(_) => return Err(format!("{}: a refusal was required", detail).into()),
        Err(diagnostic) => &diagnostic.refusal,
    };
    assert_eq!(refusal.code(), code, "{}: typed code", detail);
    assert_eq!(refusal.stage(), stage, "{}: stage", detail);
    Ok(())
}

fn assert_cleanup(
    result: &Result<AdmissionReceipt, RefusalDiagnostic>,
) -> Result<(), Box<dyn Error>> {
    let diagnostic = match result {
        Ok(_) => return Err("cleanup follows a refusal only".into()),
        Err(diagnostic) => diagnostic,
    };
    let cleanup = diagnostic
        .cleanup
        .as_deref()
        .ok_or("a post-staging refusal records a cleanup outcome")?;
    assert!(
        matches!(cleanup, CleanupDisposition::Completed(report) if report.is_complete())
            || cleanup.is_pending(),
        "cleanup is complete or reports pending"
    );
    Ok(())
}

fn no_receipt(result: &Result<AdmissionReceipt, RefusalDiagnostic>) -> bool {
    result.is_err()
}

fn local_registry(event_type: &str) -> Result<PayloadRegistryInput, Box<dyn Error>> {
    let bytes =
        payload_registry::structural_schema_bytes(event_type, &[("abstract-field", "string")])?;
    let schema = LocalSchemaInput::seal(
        event_type.to_owned(),
        "nirnay-payload-example".to_owned(),
        "1".to_owned(),
        "structural-v1".to_owned(),
        bytes,
    );
    Ok(PayloadRegistryInput::seal(
        "test-only-nz-api001-registry".to_owned(),
        "nirnay-payload-registry-v1".to_owned(),
        vec![schema],
    ))
}

fn local_payload() -> Result<Payload, Box<dyn Error>> {
    Ok(serde_json::from_value(json!({
        "fields": [{"field_name": "abstract-field", "kind": "string", "value": "abstract-value"}]
    }))?)
}

fn flip_first_byte(bytes: &mut [u8]) -> Result<(), Box<dyn Error>> {
    let first = bytes.first_mut().ok_or("empty bytes cannot be mutated")?;
    *first ^= 0x01;
    Ok(())
}

#[test]
fn cross_execution_producer_release_reaches_stage_six() -> Result<(), Box<dyn Error>> {
    let Some(artifact) = run_producer()? else {
        eprintln!(
            "SKIP cross_execution_producer_release_reaches_stage_six: {}",
            skip_reason()
        );
        return Ok(());
    };
    let staging = staging_dir(&artifact)?;
    let observed = Arc::new(Mutex::new(Vec::new()));
    let (locator, request, verifier) = verify_context(
        &artifact,
        &staging,
        Box::new(UnknownRights),
        test_resolver()?,
        false,
        None,
        Arc::clone(&observed),
    )?;
    let result = verifier.verify_release_streaming(&locator, &request);
    assert_typed_refusal(&result, "REJECT_RIGHTS_EVIDENCE", 6, &provenance(&artifact))?;
    assert!(
        no_receipt(&result),
        "UNKNOWN rights yield no admission receipt: {}",
        provenance(&artifact)
    );
    assert_cleanup(&result)?;
    assert_eq!(fs::read_dir(&staging)?.count(), 0, "no snapshot survives");
    let guard = observed.lock().map_err(|error| error.to_string())?;
    let stages = guard.clone();
    assert_eq!(
        stages,
        vec![1_u8, 2, 3, 4, 5],
        "stages one through five completed under UNKNOWN rights"
    );
    Ok(())
}

#[test]
fn cross_execution_one_byte_mutations_are_typed() -> Result<(), Box<dyn Error>> {
    let Some(artifact) = run_producer()? else {
        eprintln!(
            "SKIP cross_execution_one_byte_mutations_are_typed: {}",
            skip_reason()
        );
        return Ok(());
    };
    let saved = snapshot_release(&artifact.release_dir)?;
    let manifest_path = artifact.release_dir.join("manifest.json");
    let sidecar_path = artifact.release_dir.join("signature.json");
    let artifact_path = artifact_files(&artifact.release_dir.join("artifacts"))?
        .first()
        .ok_or("producer release has no artifact")?
        .clone();

    let mut manifest = fs::read(&manifest_path)?;
    flip_first_byte(&mut manifest)?;
    fs::write(&manifest_path, &manifest)?;
    let staging = staging_dir(&artifact)?;
    let (locator, request, verifier) = verify_context(
        &artifact,
        &staging,
        Box::new(UnknownRights),
        test_resolver()?,
        false,
        None,
        Arc::new(Mutex::new(Vec::new())),
    )?;
    let result = verifier.verify_release_streaming(&locator, &request);
    assert_typed_refusal(&result, "REJECT_SCHEMA_ENVELOPE", 1, "manifest mutation")?;
    assert_cleanup(&result)?;
    restore_release(&saved)?;

    let mut sidecar = fs::read(&sidecar_path)?;
    flip_first_byte(&mut sidecar)?;
    fs::write(&sidecar_path, &sidecar)?;
    let (locator, request, verifier) = verify_context(
        &artifact,
        &staging,
        Box::new(UnknownRights),
        test_resolver()?,
        false,
        None,
        Arc::new(Mutex::new(Vec::new())),
    )?;
    let result = verifier.verify_release_streaming(&locator, &request);
    assert_typed_refusal(&result, "REJECT_SIGNATURE_INTEGRITY", 3, "sidecar mutation")?;
    assert_cleanup(&result)?;
    restore_release(&saved)?;

    let mut stored = fs::read(&artifact_path)?;
    flip_first_byte(&mut stored)?;
    fs::write(&artifact_path, &stored)?;
    let (locator, request, verifier) = verify_context(
        &artifact,
        &staging,
        Box::new(UnknownRights),
        test_resolver()?,
        false,
        None,
        Arc::new(Mutex::new(Vec::new())),
    )?;
    let result = verifier.verify_release_streaming(&locator, &request);
    assert_typed_refusal(
        &result,
        "REJECT_ARTIFACT_DIGESTS",
        5,
        "stored artifact mutation",
    )?;
    assert_cleanup(&result)?;
    restore_release(&saved)?;

    let stored = fs::read(&artifact_path)?;
    let mut expanded = Vec::new();
    if stored.starts_with(&[0x1f, 0x8b]) {
        let mut decoder = GzDecoder::new(&stored[..]);
        decoder.read_to_end(&mut expanded)?;
    } else {
        expanded = stored.clone();
    }
    flip_first_byte(&mut expanded)?;
    let mut mutated = Vec::new();
    if stored.starts_with(&[0x1f, 0x8b]) {
        let mut encoder = GzEncoder::new(&mut mutated, Compression::default());
        encoder.write_all(&expanded)?;
        encoder.finish()?;
    } else {
        mutated = expanded;
    }
    fs::write(&artifact_path, &mutated)?;
    let (locator, request, verifier) = verify_context(
        &artifact,
        &staging,
        Box::new(UnknownRights),
        test_resolver()?,
        false,
        None,
        Arc::new(Mutex::new(Vec::new())),
    )?;
    let result = verifier.verify_release_streaming(&locator, &request);
    assert_typed_refusal(
        &result,
        "REJECT_ARTIFACT_DIGESTS",
        5,
        "expanded artifact mutation",
    )?;
    assert_cleanup(&result)?;
    restore_release(&saved)?;
    assert_eq!(
        fs::read_dir(&staging)?.count(),
        0,
        "no snapshot survives any mutation"
    );
    Ok(())
}

#[test]
fn cross_execution_payload_registry_hit_and_miss() -> Result<(), Box<dyn Error>> {
    let Some(artifact) = run_producer()? else {
        eprintln!(
            "SKIP cross_execution_payload_registry_hit_and_miss: {}",
            skip_reason()
        );
        return Ok(());
    };
    let manifest_bytes = fs::read(artifact.release_dir.join("manifest.json"))?;
    let manifest_value = jcs::parse_strict(&manifest_bytes)?;
    let parsed = canonical_wire::parse_manifest(&manifest_value)?;
    let event_type = parsed
        .declared_payload_schemas
        .keys()
        .next()
        .ok_or("producer manifest declares no payload event type")?
        .clone();
    let staging = staging_dir(&artifact)?;
    let hit = local_registry(&event_type)?;
    let (locator, request, verifier) = verify_context(
        &artifact,
        &staging,
        Box::new(UnknownRights),
        test_resolver()?,
        true,
        Some(hit),
        Arc::new(Mutex::new(Vec::new())),
    )?;
    let result = verifier.verify_release_streaming(&locator, &request);
    assert_typed_refusal(&result, "REJECT_RIGHTS_EVIDENCE", 6, "registry hit")?;

    let miss = local_registry(MISS_EVENT_TYPE)?;
    let (locator, request, verifier) = verify_context(
        &artifact,
        &staging,
        Box::new(UnknownRights),
        test_resolver()?,
        true,
        Some(miss),
        Arc::new(Mutex::new(Vec::new())),
    )?;
    let result = verifier.verify_release_streaming(&locator, &request);
    assert_typed_refusal(&result, "REJECT_PAYLOAD_REGISTRY", 5, "registry miss")?;
    assert_cleanup(&result)?;
    Ok(())
}

#[test]
fn payload_registry_hit_accepts_the_declared_event() -> Result<(), Box<dyn Error>> {
    let input = local_registry(LOCAL_EVENT_TYPE)?;
    let registry = PayloadSchemaRegistry::load(&input)?;
    let installed = registry
        .schemas()
        .first()
        .ok_or("local registry has no schema")?;
    let declared = DeclaredPayloadSchema {
        event_type: LOCAL_EVENT_TYPE.to_owned(),
        schema_id: installed.schema_id().to_owned(),
        schema_version: installed.schema_version().to_owned(),
        schema_bytes_digest: installed.schema_bytes_digest().to_owned(),
    };
    payload_registry::validate_record(&registry, LOCAL_EVENT_TYPE, &declared, &local_payload()?)?;
    Ok(())
}

#[test]
fn payload_registry_miss_is_a_stage_five_typed_refusal() -> Result<(), Box<dyn Error>> {
    let input = local_registry(LOCAL_EVENT_TYPE)?;
    let registry = PayloadSchemaRegistry::load(&input)?;
    let installed = registry
        .schemas()
        .first()
        .ok_or("local registry has no schema")?;
    let declared = DeclaredPayloadSchema {
        event_type: MISS_EVENT_TYPE.to_owned(),
        schema_id: installed.schema_id().to_owned(),
        schema_version: installed.schema_version().to_owned(),
        schema_bytes_digest: installed.schema_bytes_digest().to_owned(),
    };
    let failure =
        payload_registry::validate_record(&registry, MISS_EVENT_TYPE, &declared, &local_payload()?)
            .unwrap_err();
    assert!(
        matches!(failure, PayloadRegistryError::UnknownEvent { .. }),
        "an unregistered event type is a registry miss"
    );
    let refusal = failure.into_verifier_error();
    assert_eq!(
        refusal.code(),
        "REJECT_PAYLOAD_REGISTRY",
        "registry miss keeps its typed code"
    );
    assert_eq!(refusal.stage(), 5, "registry miss is staged at five");
    Ok(())
}
