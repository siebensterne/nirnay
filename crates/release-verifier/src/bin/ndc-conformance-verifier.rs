// SPDX-License-Identifier: Apache-2.0
//! Conformance executable for the canonical NDC v2 Profile 1 wire mapping.
//!
//! The binary reads a release directory containing canonical wire documents,
//! runs the twelve-stage release verifier against that directory and writes a
//! stage-by-stage diagnostic. It uses an injected test-only trusted publisher
//! entry whose public key is derived from the fixed conformance seed
//! `[0x11; 32]`; no private key material, release key, provider contact or
//! market observation is present in this executable.
//!
//! Rights remain `UNKNOWN` under the current freeze. The injected rights
//! evaluator therefore returns `Unknown` unconditionally. The expected result
//! is a first refusal at the rights-admission stage after the structural,
//! manifest, signature, artifact and byte-derived-ordering stages pass. No
//! admission receipt is created and no private snapshot survives the refusal.

use release_verifier::AdmissionRequest;
use release_verifier::CleanupDisposition;
use release_verifier::Decision;
use release_verifier::KeyState;
use release_verifier::RefusalDiagnostic;
use release_verifier::ReleaseLocator;
use release_verifier::ReleaseManifest;
use release_verifier::ReleaseVerifier;
use release_verifier::TrustedKeyEntry;
use release_verifier::TrustedKeyRegistry;
use release_verifier::VerifierConfig;
use release_verifier::VerifierError;
use release_verifier::VerifierResourcePolicy;
use release_verifier::canonical_wire;
use release_verifier::files::hex_lower;
use release_verifier::files::sha256;
use release_verifier::jcs::canonical_bytes;
use release_verifier::jcs::parse_strict;
use release_verifier::schema::CoverageDecision;
use release_verifier::schema::CoverageEvaluator;
use release_verifier::schema::LineageResolver;
use release_verifier::schema::PredecessorEvidence;
use release_verifier::schema::RightsDecision;
use release_verifier::schema::RightsEvaluator;
use release_verifier::trust::KeyPurpose;
use serde_json::Value;
use serde_json::json;
use std::cell::RefCell;
use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

/// Test-only key identity bound to the fixed conformance public key.
const CONFORMANCE_KEY_ID: &str = "conformance-key-1";

/// Test-only registry revision injected by this executable.
const CONFORMANCE_REGISTRY_REVISION: &str = "conformance-registry-rev-1";

/// Public key bytes derived independently from the fixed seed `[0x11; 32]`.
///
/// The derivation is performed outside this source tree by RFC 8032 key
/// expansion. Only the resulting public bytes are carried here; the seed is a
/// published test value and no private key object is constructed at runtime.
const CONFORMANCE_PUBLIC_KEY: [u8; 32] = [
    0xd0, 0x4a, 0xb2, 0x32, 0x74, 0x2b, 0xb4, 0xab, 0x3a, 0x13, 0x68, 0xbd, 0x46, 0x15, 0xe4, 0xe6,
    0xd0, 0x22, 0x4a, 0xb7, 0x1a, 0x01, 0x6b, 0xaf, 0x85, 0x20, 0xa3, 0x32, 0xc9, 0x77, 0x87, 0x37,
];

/// Names of the twelve verifier stages in execution order.
const STAGE_NAMES: [&str; 12] = [
    "STRUCTURAL_SCHEMA",
    "MANIFEST_DIGEST",
    "SIGNATURE",
    "PUBLISHER_IDENTITY",
    "ARTIFACTS_AND_ORDERING",
    "RIGHTS_ADMISSION",
    "COVERAGE",
    "PRODUCER_ACTIVITY",
    "ENVELOPE_SCHEMA",
    "LINEAGE_PIT",
    "USE_SCOPE",
    "RECEIPT",
];

struct Arguments {
    release: PathBuf,
    out: PathBuf,
}

struct FreezeUnknownRights;

impl RightsEvaluator for FreezeUnknownRights {
    fn evaluate(&self, _manifest: &ReleaseManifest, _request: &AdmissionRequest) -> RightsDecision {
        RightsDecision {
            decision: Decision::Unknown,
            matching_grant_ids: Vec::new(),
        }
    }
}

struct NoPromotionCoverage;

impl CoverageEvaluator for NoPromotionCoverage {
    fn evaluate(
        &self,
        _manifest: &ReleaseManifest,
        _request: &AdmissionRequest,
    ) -> CoverageDecision {
        CoverageDecision {
            approved_dimensions: Vec::new(),
            used_implicit_all: false,
        }
    }
}

struct NoTrustedPredecessor;

impl LineageResolver for NoTrustedPredecessor {
    fn predecessor(&self, _release_id: &str) -> Option<PredecessorEvidence> {
        None
    }
}

struct PrivateStaging(PathBuf);

impl PrivateStaging {
    fn create() -> Result<Self, Box<dyn Error>> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "ndc-conformance-private-{}-{}",
            std::process::id(),
            nanos
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn has_entries(&self) -> Result<bool, Box<dyn Error>> {
        Ok(fs::read_dir(self.path())?.next().is_some())
    }
}

impl Drop for PrivateStaging {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).ok();
    }
}

fn parse_arguments() -> Result<Arguments, Box<dyn Error>> {
    let mut release = None;
    let mut out = None;
    let mut arguments = env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("--release") => {
                if release.is_some() {
                    return Err("--release may appear once".into());
                }
                release = Some(arguments.next().ok_or("--release needs a directory")?);
            }
            Some("--out") => {
                if out.is_some() {
                    return Err("--out may appear once".into());
                }
                out = Some(arguments.next().ok_or("--out needs a file path")?);
            }
            Some("--help") => {
                eprintln!(
                    "usage: ndc-conformance-verifier --release <dir> --out <diagnostic.json>"
                );
                return Err("--help".into());
            }
            _ => return Err(format!("unknown argument: {}", argument.to_string_lossy()).into()),
        }
    }
    Ok(Arguments {
        release: PathBuf::from(release.ok_or("--release is required")?),
        out: PathBuf::from(out.ok_or("--out is required")?),
    })
}

fn locate_release(path: &Path) -> Result<(PathBuf, ReleaseLocator), Box<dyn Error>> {
    let mut components: Vec<&OsStr> = path
        .components()
        .map(|component| component.as_os_str())
        .collect();
    while components
        .last()
        .is_some_and(|component| component.is_empty())
    {
        components.pop();
    }
    if components.len() < 4
        || components
            .iter()
            .any(|component| *component == OsStr::new("."))
    {
        return Err("release path must be <root>/<publisher>/<dataset>/<release>".into());
    }
    let release_component = components.pop().ok_or("missing release component")?;
    let dataset_component = components.pop().ok_or("missing dataset component")?;
    let publisher_component = components.pop().ok_or("missing publisher component")?;
    let to_text = |component: &OsStr| -> Result<String, Box<dyn Error>> {
        component
            .to_str()
            .map(str::to_owned)
            .ok_or_else(|| "release path component is not UTF-8".into())
    };
    let locator = ReleaseLocator {
        publisher_id: to_text(publisher_component)?,
        dataset_id: to_text(dataset_component)?,
        release_id: to_text(release_component)?,
    };
    let root = components
        .iter()
        .fold(PathBuf::new(), |mut path, component| {
            path.push(component);
            path
        });
    Ok((root, locator))
}

fn conformance_registry(publisher_id: &str) -> Result<TrustedKeyRegistry, Box<dyn Error>> {
    TrustedKeyRegistry::new(
        vec![TrustedKeyEntry {
            publisher_id: publisher_id.to_owned(),
            key_id: CONFORMANCE_KEY_ID.to_owned(),
            public_key_bytes: CONFORMANCE_PUBLIC_KEY,
            algorithm: "Ed25519".to_owned(),
            purpose: KeyPurpose::ReleaseSigning,
            valid_from_ns: 1,
            valid_until_ns: u64::MAX,
            state: KeyState::Active,
            evidence_reference: None,
        }],
        CONFORMANCE_REGISTRY_REVISION.to_owned(),
    )
    .map_err(|error| error.into())
}

fn conformance_policy() -> VerifierResourcePolicy {
    VerifierResourcePolicy {
        max_manifest_bytes: 1_000_000,
        max_sidecar_bytes: 4_096,
        max_artifact_count: 16,
        max_stored_bytes: 1_000_000,
        max_expanded_bytes: 1_000_000,
        max_total_expanded_bytes: 1_000_000,
        max_record_bytes: 4_096,
        max_record_count: 1_000,
        max_json_nesting: 16,
        max_decompression_budget: 1_000_000,
        max_temp_disk_bytes: 1_000_000,
        max_open_file_descriptors: 64,
    }
}

fn stage_diagnostic(passed: &[u8], refusal_stage: Option<u8>, code: Option<&str>) -> Vec<Value> {
    STAGE_NAMES
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let number = (index + 1) as u8;
            let status = if passed.contains(&number) {
                "PASS"
            } else if refusal_stage == Some(number) {
                "REFUSED"
            } else {
                "NOT_RUN"
            };
            let stage_code = if status == "PASS" {
                "OK"
            } else if status == "REFUSED" {
                code.unwrap_or("REJECT_UNKNOWN")
            } else {
                "NOT_APPLICABLE"
            };
            json!({
                "stage": number,
                "name": name,
                "status": status,
                "code": stage_code,
            })
        })
        .collect()
}

fn cleanup_value(cleanup: &Option<Box<CleanupDisposition>>) -> Value {
    match cleanup.as_deref() {
        None => Value::Null,
        Some(CleanupDisposition::Completed(report)) => json!({
            "cleanup_succeeded": report.is_complete(),
            "cleanup_pending": false,
            "code": "CLEANUP_COMPLETED",
            "files_expected": report.files_expected,
            "files_removed": report.files_removed,
            "files_already_absent": report.files_already_absent,
            "bytes_removed": report.bytes_removed,
            "directory_empty": report.directory_empty,
            "directory_sync_completed": report.directory_sync_completed,
        }),
        Some(CleanupDisposition::Pending(pending)) => json!({
            "cleanup_succeeded": false,
            "cleanup_pending": true,
            "code": "CLEANUP_PENDING",
            "staging_path": pending.staging_path.to_string_lossy(),
            "remaining_entries": pending.remaining_entries,
            "detail": pending.detail,
        }),
    }
}

fn diagnostic(
    locator: &ReleaseLocator,
    stages: Vec<Value>,
    refusal: Option<Value>,
    outcome: &str,
    snapshot_survives: bool,
    cleanup: Value,
) -> Value {
    let status_of = |stage: u8| -> &str {
        stages
            .iter()
            .find(|entry| entry["stage"] == stage)
            .and_then(|entry| entry["status"].as_str())
            .unwrap_or("NOT_RUN")
    };
    json!({
        "schema_version": 1,
        "record_date": "2026-11-24",
        "tool": "ndc-conformance-verifier",
        "release": {
            "publisher_id": locator.publisher_id,
            "dataset_id": locator.dataset_id,
            "release_id": locator.release_id,
        },
        "outcome": outcome,
        "stages": stages,
        "phase_summary": {
            "structural": status_of(1),
            "manifest": status_of(2),
            "signature": status_of(3),
            "artifact": status_of(5),
            "ordering": status_of(5),
            "rights_admission": status_of(6),
        },
        "refusal": refusal,
        "cleanup": cleanup,
        "admission_receipt": Value::Null,
        "receipt_written": false,
        "private_snapshot_survives": snapshot_survives,
        "rights_state": "UNKNOWN",
        "dataset_accepted": false,
        "collection_enabled": false,
        "live_execution_enabled": false,
    })
}

fn refusal_value(error: &VerifierError) -> Value {
    json!({
        "stage": error.stage(),
        "code": error.code(),
        "detail": error.to_string(),
    })
}

fn run(arguments: &Arguments) -> Result<Value, Box<dyn Error>> {
    let (root, locator) = locate_release(&arguments.release)?;
    let manifest_bytes = fs::read(arguments.release.join("manifest.json"))?;
    let manifest_value = parse_strict(&manifest_bytes).map_err(|error| error.to_string())?;
    let parsed =
        canonical_wire::parse_manifest(&manifest_value).map_err(|error| error.to_string())?;
    let digest = hex_lower(&sha256(&canonical_bytes(&manifest_value)));
    let request = canonical_wire::into_internal_request(&parsed, &digest)?;

    let passed = Rc::new(RefCell::new(Vec::new()));
    let staging = PrivateStaging::create()?;
    let verifier = ReleaseVerifier::new(VerifierConfig {
        root,
        private_staging: staging.path().to_owned(),
        registry: conformance_registry(&locator.publisher_id)?,
        rights: Box::new(FreezeUnknownRights),
        coverage: Box::new(NoPromotionCoverage),
        lineage: Box::new(NoTrustedPredecessor),
        policy: conformance_policy(),
        verifier_commit: "conformance-verifier-no-commit".to_owned(),
    })
    .with_canonical_wire()
    .with_stage_observer({
        let passed = Rc::clone(&passed);
        move |stage| passed.borrow_mut().push(stage)
    });

    let outcome: Result<_, RefusalDiagnostic> =
        verifier.verify_release_with_diagnostics(&locator, &request);
    drop(verifier);
    let observed = Rc::try_unwrap(passed)
        .map_err(|_| "stage observer outlived verification".to_owned())?
        .into_inner();
    let survives = staging.has_entries()?;
    match outcome {
        Ok(_) => Ok(diagnostic(
            &locator,
            stage_diagnostic(&observed, None, None),
            None,
            "UNEXPECTED_RIGHTS_ADMISSION",
            survives,
            Value::Null,
        )),
        Err(failure) => {
            let refusal = refusal_value(&failure.refusal);
            Ok(diagnostic(
                &locator,
                stage_diagnostic(
                    &observed,
                    Some(failure.refusal.stage()),
                    Some(failure.refusal.code()),
                ),
                Some(refusal),
                "RIGHTS_ADMISSION_REFUSED",
                survives,
                cleanup_value(&failure.cleanup),
            ))
        }
    }
}

fn main() -> ExitCode {
    let arguments = match parse_arguments() {
        Ok(arguments) => arguments,
        Err(error) => {
            eprintln!("conformance verifier arguments refused: {}", error);
            return ExitCode::FAILURE;
        }
    };
    let result = run(&arguments);
    let report = match result {
        Ok(report) => report,
        Err(error) => {
            let placeholder = ReleaseLocator {
                publisher_id: "unresolved".to_owned(),
                dataset_id: "unresolved".to_owned(),
                release_id: "unresolved".to_owned(),
            };
            diagnostic(
                &placeholder,
                stage_diagnostic(&[], Some(1), Some("REJECT_SCHEMA_ENVELOPE")),
                Some(json!({
                    "stage": 1,
                    "code": "REJECT_SCHEMA_ENVELOPE",
                    "detail": error.to_string(),
                })),
                "PREFLIGHT_REFUSED",
                false,
                Value::Null,
            )
        }
    };
    let conforms = report["outcome"] == "RIGHTS_ADMISSION_REFUSED"
        && report["refusal"]["stage"] == 6
        && report["refusal"]["code"] == "REJECT_RIGHTS_EVIDENCE"
        && report["admission_receipt"].is_null()
        && report["private_snapshot_survives"] == false
        && report["cleanup"]["cleanup_succeeded"] == true
        && report["cleanup"]["cleanup_pending"] == false;
    if let Some(parent) = arguments.out.parent()
        && !parent.as_os_str().is_empty()
        && let Err(error) = fs::create_dir_all(parent)
    {
        eprintln!("cannot create diagnostic parent directory: {}", error);
        return ExitCode::FAILURE;
    }
    let encoded = match serde_json::to_string_pretty(&report) {
        Ok(encoded) => encoded,
        Err(error) => {
            eprintln!("cannot encode diagnostic: {}", error);
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = fs::write(&arguments.out, encoded) {
        eprintln!("cannot write diagnostic: {}", error);
        return ExitCode::FAILURE;
    }
    if conforms {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
