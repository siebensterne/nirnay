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

pub mod canonical_wire;
pub mod error;
pub mod files;
pub mod jcs;
pub mod receipt;
pub mod schema;
pub mod trust;
pub mod verifier;

pub use canonical_wire::CanonicalError;
pub use canonical_wire::CanonicalManifest;
pub use canonical_wire::CanonicalRecord;
pub use canonical_wire::DerivedOrderKey;
pub use canonical_wire::ParsedManifest;
pub use error::CleanupPending;
pub use error::VerifierError;
pub use files::CleanupReport;
pub use files::PrivateSnapshot;
pub use files::SnapshotBuilder;
pub use receipt::AdmissionReceipt;
pub use receipt::AdmissionResult;
pub use receipt::ArtifactBinding;
pub use receipt::ReceiptInput;
pub use schema::AdmissionRequest;
pub use schema::CoverageDecision;
pub use schema::CoverageEvaluator;
pub use schema::Decision;
pub use schema::LineageResolver;
pub use schema::PredecessorEvidence;
pub use schema::ReleaseManifest;
pub use schema::RightsDecision;
pub use schema::RightsEvaluator;
pub use trust::KeyState;
pub use trust::TrustedKeyEntry;
pub use trust::TrustedKeyRegistry;
pub use trust::VerifierResourcePolicy;
pub use verifier::CleanupDisposition;
pub use verifier::RefusalDiagnostic;
pub use verifier::ReleaseLocator;
pub use verifier::ReleaseVerifier;
pub use verifier::VerifierConfig;
