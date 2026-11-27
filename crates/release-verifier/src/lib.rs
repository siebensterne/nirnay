// SPDX-License-Identifier: Apache-2.0
#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]
#![deny(rustdoc::private_intra_doc_links)]
//! Independent Nirnay verifier for immutable NDC v2 release packages.
//!
//! This crate is a separate implementation. It imports no Fieldbook code, owns
//! no signing key and creates no shared validator. It contacts no provider and
//! contains no market observation. Test data is abstract structured JSON with
//! official JCS and Ed25519 vectors only.
//!
//! Verification is twelve ordered stages. The first refusal stops processing;
//! no later stage softens an earlier failure and a rejected release is never
//! partially consumed. Published files are opened with `O_NOFOLLOW` below a
//! configured trusted root, hashed from the opened descriptor, copied into a
//! caller-provided private staging directory and sealed before any downstream
//! stage runs. A receipt records checks performed and is not a right.
//!
//! Entry-point classification: `verify_release_streaming` is the bounded
//! streaming path. `verify_release` and `verify_release_with_diagnostics` are
//! LEGACY_BUFFERED, keep their caller-owned policy and remain for existing
//! callers. A streaming refusal returns to its caller; the buffered verifiers
//! are not fallback paths and a rejection is not retried through them.
//!
//! The independent source-event validator reports normative v1 rejection codes
//! through [`SourceEventError::normative_code`]. Conformance evidence compares
//! those codes, never a display string, and never turns a valid envelope into
//! a rights grant or a dataset acceptance.

pub mod admission;
pub mod canonical_wire;
pub mod error;
pub mod files;
pub mod jcs;
pub mod payload_registry;
pub mod profile_resolver;
pub mod receipt;
pub mod resource_profile;
pub mod schema;
pub mod source_event;
pub mod streaming;
pub mod trust;
pub mod verifier;

#[cfg(test)]
extern crate self as release_verifier;
#[cfg(test)]
#[path = "../tests/admission.rs"]
mod admission_tests;
#[cfg(test)]
#[path = "../tests/b16_streaming.rs"]
mod b16_streaming_tests;
#[cfg(test)]
#[path = "../tests/nz_api001_cross_execution.rs"]
mod nz_api001_cross_execution_tests;
#[cfg(test)]
#[path = "../tests/source_event.rs"]
mod source_event_tests;

pub use admission::AdmissionError;
pub use admission::AdmissionInput;
pub use admission::DatasetAcceptance;
pub use admission::RightsVerdict;
pub use admission::SourceEventSink;
pub use admission::admit;
pub use canonical_wire::CanonicalError;
pub use canonical_wire::CanonicalManifest;
pub use canonical_wire::CanonicalRecord;
pub use canonical_wire::DerivedOrderKey;
pub use canonical_wire::ParsedManifest;
pub use error::CleanupPending;
pub use error::VerifierError;
pub use files::CleanupReport;
pub use files::EntryProof;
pub use files::PrivateSnapshot;
pub use files::SnapshotBuilder;
pub use files::SnapshotEntry;
pub use files::StagingLimits;
pub use payload_registry::DeclaredPayloadSchema;
pub use payload_registry::InstalledPayloadSchema;
pub use payload_registry::LocalSchemaInput;
pub use payload_registry::PayloadRegistryError;
pub use payload_registry::PayloadRegistryInput;
pub use payload_registry::PayloadSchemaRegistry;
pub use profile_resolver::ApprovedProfile;
pub use profile_resolver::ApprovedProfileRegistry;
pub use profile_resolver::ProfileResolver;
pub use receipt::AdmissionReceipt;
pub use receipt::AdmissionResult;
pub use receipt::ArtifactBinding;
pub use receipt::ReceiptInput;
pub use resource_profile::EffectiveLimits;
pub use resource_profile::LimitsInput;
pub use resource_profile::ProcessingRole;
pub use resource_profile::ProfileError;
pub use resource_profile::ProfileLimits;
pub use resource_profile::ResourceProfile;
pub use resource_profile::ResourceProfileId;
pub use resource_profile::StreamingResourcePolicy;
pub use schema::AdmissionRequest;
pub use schema::CoverageDecision;
pub use schema::CoverageEvaluator;
pub use schema::Decision;
pub use schema::LineageResolver;
pub use schema::PredecessorEvidence;
pub use schema::ReleaseManifest;
pub use schema::RightsDecision;
pub use schema::RightsEvaluator;
pub use source_event::ContinuityState;
pub use source_event::QualityState;
pub use source_event::SequenceLedger;
pub use source_event::SourceEvent;
pub use source_event::SourceEventError;
pub use source_event::V1_SCHEMA_VERSION;
pub use trust::KeyState;
pub use trust::TrustedKeyEntry;
pub use trust::TrustedKeyRegistry;
pub use trust::VerifierResourcePolicy;
pub use verifier::CleanupDisposition;
pub use verifier::InFlightArtifacts;
pub use verifier::InFlightSlotError;
pub use verifier::RefusalDiagnostic;
pub use verifier::ReleaseLocator;
pub use verifier::ReleaseVerifier;
pub use verifier::VerifierConfig;
