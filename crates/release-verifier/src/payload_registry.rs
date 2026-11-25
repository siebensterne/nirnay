// SPDX-License-Identifier: Apache-2.0
//! Locally trusted, immutable per-event payload schema registry for D6.
//!
//! The ratified contract declares per-event payload schemas in the release
//! manifest. Those declarations are untrusted bytes. This module keeps the
//! authoritative registry outside the release: the caller installs local
//! material, stage five verifies its digest, and every canonical record is then
//! checked against the installed binding before it is admitted.
//!
//! The Workspace ratification names no schema language. Adopting JSON Schema
//! would add a dependency and a new contract, so this implementation selects an
//! explicit local structural validator driven by registry metadata:
//! `nirnay-structural-payload-v1` with validator identity `structural-v1`.
//! That validator checks object members, scalar kinds, required fields,
//! duplicates and scalar grammar for the canonical record payload. JSON Schema
//! adoption requires a separate Workspace amendment; no such dependency is
//! added here and D6 is not reduced to event-type membership.

use crate::canonical_wire::Payload;
use crate::canonical_wire::PayloadFieldKind;
use crate::error::VerifierError;
use crate::files::hex_lower;
use crate::files::parse_hex_lower;
use crate::jcs::canonical_bytes;
use crate::jcs::parse_strict;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use sha2::Digest;
use sha2::Sha256;
use std::collections::HashSet;

/// Domain prefix bound into every canonical local payload registry digest.
const REGISTRY_DIGEST_DOMAIN: &[u8] = b"NDC-V2-PAYLOAD-REGISTRY-V1\x00";

/// The one registry version this verifier understands.
const SUPPORTED_REGISTRY_VERSION: &str = "nirnay-payload-registry-v1";

/// The one built-in validator identity this verifier understands.
const STRUCTURAL_VALIDATOR_V1: &str = "structural-v1";

/// Format token required inside installed structural schema bytes.
const STRUCTURAL_SCHEMA_FORMAT: &str = "nirnay-structural-payload-v1";

/// Fail-closed failure class for local registry load and per-record enforcement.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PayloadRegistryError {
    /// The installed registry version is not understood by this verifier.
    #[error("payload registry version is unknown: {version}")]
    UnknownRegistryVersion {
        /// Installed version token.
        version: String,
    },
    /// The installed registry digest does not cover its own bindings.
    #[error("payload registry digest mismatch")]
    RegistryDigestMismatch,
    /// The local schema bytes are malformed or do not match their event type.
    #[error("payload registry schema bytes are corrupted: {detail}")]
    CorruptSchema {
        /// Field-level detail without untrusted record bytes.
        detail: String,
    },
    /// The installed schema bytes do not hash to their declared digest.
    #[error("payload registry schema digest mismatch for event {event_type}")]
    SchemaDigestMismatch {
        /// Event type whose installed schema bytes disagree.
        event_type: String,
    },
    /// The installed validator identity has no local implementation.
    #[error("payload registry validator identity is unknown: {identity}")]
    UnknownValidator {
        /// Installed validator identity token.
        identity: String,
    },
    /// Two installed entries bind the same event type.
    #[error("payload registry duplicates event type {event_type}")]
    DuplicateEventType {
        /// Duplicated event type.
        event_type: String,
    },
    /// Two installed entries bind the same schema identity.
    #[error("payload registry duplicates schema identity {schema_id}")]
    DuplicateSchemaId {
        /// Duplicated schema identity.
        schema_id: String,
    },
    /// No installed entry resolves the record event type.
    #[error("payload registry has no entry for event {event_type}")]
    UnknownEvent {
        /// Record event type with no installed binding.
        event_type: String,
    },
    /// The manifest declaration names a different schema identity.
    #[error("payload schema identity differs from the installed entry for event {event_type}")]
    SchemaIdMismatch {
        /// Event type whose declaration disagrees.
        event_type: String,
    },
    /// The manifest declaration names a different schema version.
    #[error("payload schema version differs from the installed entry for event {event_type}")]
    SchemaVersionMismatch {
        /// Event type whose declaration disagrees.
        event_type: String,
    },
    /// The manifest declaration carries a different schema bytes digest.
    #[error("payload schema digest differs from the installed entry for event {event_type}")]
    DeclaredDigestMismatch {
        /// Event type whose declaration disagrees.
        event_type: String,
    },
    /// The payload does not match the installed structural schema.
    #[error("payload does not match the installed schema for event {event_type}: {detail}")]
    PayloadInvalid {
        /// Event type whose payload is refused.
        event_type: String,
        /// Structural detail without raw record bytes.
        detail: String,
    },
    /// Canonical mode was requested without an installed local registry.
    #[error("no locally installed payload schema registry is available")]
    NotInstalled,
}

impl PayloadRegistryError {
    /// Map a registry failure to the stage-five typed verifier refusal.
    pub fn into_verifier_error(self) -> VerifierError {
        VerifierError::PayloadRegistry {
            detail: self.to_string(),
        }
    }
}

/// One locally supplied payload schema binding before digest verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalSchemaInput {
    /// Event type bound by this entry.
    pub event_type: String,
    /// Schema identity bound by this entry.
    pub schema_id: String,
    /// Schema version bound by this entry.
    pub schema_version: String,
    /// Declared digest of the exact local schema bytes.
    pub schema_bytes_digest: String,
    /// Local validator identity selected for this entry.
    pub validator_identity: String,
    /// Exact local schema bytes, supplied outside the release package.
    pub schema_bytes: Vec<u8>,
}

impl LocalSchemaInput {
    /// Bind exact local schema bytes and compute their lowercase SHA-256 digest.
    pub fn seal(
        event_type: String,
        schema_id: String,
        schema_version: String,
        validator_identity: String,
        schema_bytes: Vec<u8>,
    ) -> Self {
        let digest = sha256_bytes(&schema_bytes);
        Self {
            event_type,
            schema_id,
            schema_version,
            schema_bytes_digest: hex_lower(&digest),
            validator_identity,
            schema_bytes,
        }
    }
}

/// Locally supplied material for the installed payload schema registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadRegistryInput {
    /// Local registry identity.
    pub registry_id: String,
    /// Local registry version understood by this verifier.
    pub registry_version: String,
    /// Declared digest over identity, version and every installed binding.
    pub registry_digest: String,
    /// Installed schema bindings in canonical order.
    pub schemas: Vec<LocalSchemaInput>,
}

impl PayloadRegistryInput {
    /// Compute the registry digest over identity, version and all bindings.
    pub fn seal(
        registry_id: String,
        registry_version: String,
        schemas: Vec<LocalSchemaInput>,
    ) -> Self {
        let digest = registry_digest(&registry_id, &registry_version, &schemas);
        Self {
            registry_id,
            registry_version,
            registry_digest: hex_lower(&digest),
            schemas,
        }
    }
}

/// One publisher-declared payload schema entry from an untrusted manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredPayloadSchema {
    /// Declared event type.
    pub event_type: String,
    /// Declared schema identity.
    pub schema_id: String,
    /// Declared schema version.
    pub schema_version: String,
    /// Declared digest of the schema bytes.
    pub schema_bytes_digest: String,
}

impl DeclaredPayloadSchema {
    /// Copy a ratified canonical-wire declaration into a comparison value.
    pub fn from_wire(entry: &crate::canonical_wire::PayloadSchemaEntry) -> Self {
        Self {
            event_type: entry.event_type.as_str().to_owned(),
            schema_id: entry.schema_id.as_str().to_owned(),
            schema_version: entry.schema_version.as_str().to_owned(),
            schema_bytes_digest: entry.digest_sha256.as_str().to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequiredFieldWire {
    field_name: String,
    kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct StructuralSchemaWire {
    format: String,
    event_type: String,
    required_fields: Vec<RequiredFieldWire>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RequiredField {
    field_name: String,
    kind: &'static str,
}

/// One installed payload schema binding inside the trusted registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPayloadSchema {
    event_type: String,
    schema_id: String,
    schema_version: String,
    schema_bytes_digest: String,
    validator_identity: String,
    required_fields: Vec<RequiredField>,
}

impl InstalledPayloadSchema {
    /// Bound event type.
    pub fn event_type(&self) -> &str {
        &self.event_type
    }

    /// Bound schema identity.
    pub fn schema_id(&self) -> &str {
        &self.schema_id
    }

    /// Bound schema version.
    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    /// Bound digest of the exact local schema bytes.
    pub fn schema_bytes_digest(&self) -> &str {
        &self.schema_bytes_digest
    }

    /// Bound local validator identity.
    pub fn validator_identity(&self) -> &str {
        &self.validator_identity
    }
}

/// Immutable locally trusted per-event payload schema registry.
///
/// A registry exists only after `load` has verified its version, digest, schema
/// bytes and validator identity. Callers cannot mutate a loaded registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadSchemaRegistry {
    registry_id: String,
    registry_version: String,
    registry_digest: String,
    schemas: Vec<InstalledPayloadSchema>,
}

impl PayloadSchemaRegistry {
    /// Verify and load locally supplied registry material at stage five.
    pub fn load(input: &PayloadRegistryInput) -> Result<Self, PayloadRegistryError> {
        if input.registry_id.is_empty() {
            return Err(PayloadRegistryError::CorruptSchema {
                detail: "registry identity is empty".to_owned(),
            });
        }
        if input.registry_version != SUPPORTED_REGISTRY_VERSION {
            return Err(PayloadRegistryError::UnknownRegistryVersion {
                version: input.registry_version.clone(),
            });
        }
        let expected = registry_digest(&input.registry_id, &input.registry_version, &input.schemas);
        let observed =
            parse_hex_lower(&input.registry_digest, "registry_digest").map_err(|_| {
                PayloadRegistryError::CorruptSchema {
                    detail: "registry digest is not lowercase hex".to_owned(),
                }
            })?;
        if observed != expected {
            return Err(PayloadRegistryError::RegistryDigestMismatch);
        }
        let mut schemas = Vec::with_capacity(input.schemas.len());
        let mut event_types = HashSet::new();
        let mut schema_ids = HashSet::new();
        for candidate in &input.schemas {
            if !event_types.insert(candidate.event_type.as_str()) {
                return Err(PayloadRegistryError::DuplicateEventType {
                    event_type: candidate.event_type.clone(),
                });
            }
            if !schema_ids.insert(candidate.schema_id.as_str()) {
                return Err(PayloadRegistryError::DuplicateSchemaId {
                    schema_id: candidate.schema_id.clone(),
                });
            }
            if candidate.event_type.is_empty() || candidate.schema_id.is_empty() {
                return Err(PayloadRegistryError::CorruptSchema {
                    detail: "schema binding identity is empty".to_owned(),
                });
            }
            if candidate.validator_identity != STRUCTURAL_VALIDATOR_V1 {
                return Err(PayloadRegistryError::UnknownValidator {
                    identity: candidate.validator_identity.clone(),
                });
            }
            let declared = parse_hex_lower(&candidate.schema_bytes_digest, "schema_bytes_digest")
                .map_err(|_| PayloadRegistryError::CorruptSchema {
                detail: "schema bytes digest is not lowercase hex".to_owned(),
            })?;
            if declared != sha256_bytes(&candidate.schema_bytes) {
                return Err(PayloadRegistryError::SchemaDigestMismatch {
                    event_type: candidate.event_type.clone(),
                });
            }
            let required_fields = parse_structural_schema(candidate)?;
            schemas.push(InstalledPayloadSchema {
                event_type: candidate.event_type.clone(),
                schema_id: candidate.schema_id.clone(),
                schema_version: candidate.schema_version.clone(),
                schema_bytes_digest: candidate.schema_bytes_digest.clone(),
                validator_identity: candidate.validator_identity.clone(),
                required_fields,
            });
        }
        Ok(Self {
            registry_id: input.registry_id.clone(),
            registry_version: input.registry_version.clone(),
            registry_digest: input.registry_digest.clone(),
            schemas,
        })
    }

    /// Local registry identity verified at load.
    pub fn registry_id(&self) -> &str {
        &self.registry_id
    }

    /// Registry version verified at load.
    pub fn registry_version(&self) -> &str {
        &self.registry_version
    }

    /// Digest verified at load.
    pub fn registry_digest(&self) -> &str {
        &self.registry_digest
    }

    /// Installed bindings in registry order.
    pub fn schemas(&self) -> &[InstalledPayloadSchema] {
        &self.schemas
    }

    fn entry(&self, event_type: &str) -> Option<&InstalledPayloadSchema> {
        self.schemas
            .iter()
            .find(|schema| schema.event_type == event_type)
    }
}

/// Check one canonical record against its installed schema binding.
///
/// The declaration comes from the untrusted manifest; the schema, validator and
/// metadata come only from the installed local registry.
pub fn validate_record(
    registry: &PayloadSchemaRegistry,
    event_type: &str,
    declared: &DeclaredPayloadSchema,
    payload: &Payload,
) -> Result<(), PayloadRegistryError> {
    let installed =
        registry
            .entry(event_type)
            .ok_or_else(|| PayloadRegistryError::UnknownEvent {
                event_type: event_type.to_owned(),
            })?;
    if declared.schema_id != installed.schema_id {
        return Err(PayloadRegistryError::SchemaIdMismatch {
            event_type: event_type.to_owned(),
        });
    }
    if declared.schema_version != installed.schema_version {
        return Err(PayloadRegistryError::SchemaVersionMismatch {
            event_type: event_type.to_owned(),
        });
    }
    if declared.schema_bytes_digest != installed.schema_bytes_digest {
        return Err(PayloadRegistryError::DeclaredDigestMismatch {
            event_type: event_type.to_owned(),
        });
    }
    validate_structural(installed, payload)
}

/// Build canonical bytes for the local structural schema format.
pub fn structural_schema_bytes(
    event_type: &str,
    fields: &[(&str, &str)],
) -> Result<Vec<u8>, PayloadRegistryError> {
    let required = fields
        .iter()
        .map(|(name, kind)| json!({"field_name": name.to_owned(), "kind": kind.to_owned()}))
        .collect::<Vec<Value>>();
    let value = json!({
        "format": STRUCTURAL_SCHEMA_FORMAT,
        "event_type": event_type,
        "required_fields": required,
    });
    Ok(canonical_bytes(&value))
}

fn sha256_bytes(bytes: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(bytes);
    let mut out = [0_u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn registry_digest(
    registry_id: &str,
    registry_version: &str,
    schemas: &[LocalSchemaInput],
) -> [u8; 32] {
    let bindings = schemas
        .iter()
        .map(|schema| {
            json!({
                "event_type": schema.event_type,
                "schema_id": schema.schema_id,
                "schema_version": schema.schema_version,
                "schema_bytes_digest": schema.schema_bytes_digest,
                "validator_identity": schema.validator_identity,
            })
        })
        .collect::<Vec<Value>>();
    let value = json!({
        "registry_id": registry_id,
        "registry_version": registry_version,
        "schemas": bindings,
    });
    let mut canonical = Vec::with_capacity(REGISTRY_DIGEST_DOMAIN.len() + 256);
    canonical.extend_from_slice(REGISTRY_DIGEST_DOMAIN);
    canonical.extend_from_slice(&canonical_bytes(&value));
    sha256_bytes(&canonical)
}

fn parse_structural_schema(
    candidate: &LocalSchemaInput,
) -> Result<Vec<RequiredField>, PayloadRegistryError> {
    let value =
        parse_strict(&candidate.schema_bytes).map_err(|_| PayloadRegistryError::CorruptSchema {
            detail: "installed schema bytes are not strict JSON".to_owned(),
        })?;
    if canonical_bytes(&value) != candidate.schema_bytes {
        return Err(PayloadRegistryError::CorruptSchema {
            detail: "installed schema bytes are not canonical".to_owned(),
        });
    }
    let wire: StructuralSchemaWire =
        serde_json::from_value(value).map_err(|_| PayloadRegistryError::CorruptSchema {
            detail: "installed schema members do not match the structural format".to_owned(),
        })?;
    if wire.format != STRUCTURAL_SCHEMA_FORMAT {
        return Err(PayloadRegistryError::CorruptSchema {
            detail: "installed schema format is unknown".to_owned(),
        });
    }
    if wire.event_type != candidate.event_type {
        return Err(PayloadRegistryError::CorruptSchema {
            detail: "installed schema event type differs from its binding".to_owned(),
        });
    }
    let mut names = HashSet::new();
    let mut required = Vec::with_capacity(wire.required_fields.len());
    for field in wire.required_fields {
        if field.field_name.is_empty() || !names.insert(field.field_name.clone()) {
            return Err(PayloadRegistryError::CorruptSchema {
                detail: "installed schema field name is empty or duplicated".to_owned(),
            });
        }
        let kind = field_kind(&field.kind).ok_or_else(|| PayloadRegistryError::CorruptSchema {
            detail: "installed schema field kind is unknown".to_owned(),
        })?;
        required.push(RequiredField {
            field_name: field.field_name,
            kind,
        });
    }
    Ok(required)
}

fn field_kind(token: &str) -> Option<&'static str> {
    match token {
        "string" => Some(PayloadFieldKind::String.as_str()),
        "decimal_string" => Some(PayloadFieldKind::DecimalString.as_str()),
        "boolean" => Some(PayloadFieldKind::Boolean.as_str()),
        "bytes_hex" => Some(PayloadFieldKind::BytesHex.as_str()),
        _ => None,
    }
}

fn validate_structural(
    installed: &InstalledPayloadSchema,
    payload: &Payload,
) -> Result<(), PayloadRegistryError> {
    let mut observed = HashSet::new();
    for field in &payload.fields {
        let name = field.field_name.as_str();
        if !observed.insert(name) {
            return Err(PayloadRegistryError::PayloadInvalid {
                event_type: installed.event_type.clone(),
                detail: format!("duplicate payload field {}", name),
            });
        }
        let required = installed
            .required_fields
            .iter()
            .find(|entry| entry.field_name == name)
            .ok_or_else(|| PayloadRegistryError::PayloadInvalid {
                event_type: installed.event_type.clone(),
                detail: format!("payload field {} is not registered", name),
            })?;
        if field.kind.as_str() != required.kind {
            return Err(PayloadRegistryError::PayloadInvalid {
                event_type: installed.event_type.clone(),
                detail: format!("payload field {} has the wrong scalar kind", name),
            });
        }
        if !value_matches_kind(required.kind, &field.value) {
            return Err(PayloadRegistryError::PayloadInvalid {
                event_type: installed.event_type.clone(),
                detail: format!("payload field {} has an invalid value", name),
            });
        }
    }
    for required in &installed.required_fields {
        if !observed.contains(required.field_name.as_str()) {
            return Err(PayloadRegistryError::PayloadInvalid {
                event_type: installed.event_type.clone(),
                detail: format!("required payload field {} is absent", required.field_name),
            });
        }
    }
    Ok(())
}

fn value_matches_kind(kind: &str, value: &str) -> bool {
    match kind {
        "string" => true,
        "decimal_string" => !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()),
        "boolean" => value == "true" || value == "false",
        "bytes_hex" => {
            !value.is_empty()
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVENT: &str = "registered-event";
    const SCHEMA_ID: &str = "payload-example";
    const VERSION: &str = "1";
    const FIELD: &str = "abstract-field";

    fn schema_input(bytes: Vec<u8>) -> LocalSchemaInput {
        LocalSchemaInput::seal(
            EVENT.to_owned(),
            SCHEMA_ID.to_owned(),
            VERSION.to_owned(),
            STRUCTURAL_VALIDATOR_V1.to_owned(),
            bytes,
        )
    }

    fn input() -> PayloadRegistryInput {
        let bytes = structural_schema_bytes(EVENT, &[(FIELD, "string")]).unwrap_or_default();
        PayloadRegistryInput::seal(
            "test-only-payload-registry".to_owned(),
            SUPPORTED_REGISTRY_VERSION.to_owned(),
            vec![schema_input(bytes)],
        )
    }

    fn declared() -> DeclaredPayloadSchema {
        DeclaredPayloadSchema {
            event_type: EVENT.to_owned(),
            schema_id: SCHEMA_ID.to_owned(),
            schema_version: VERSION.to_owned(),
            schema_bytes_digest: input().schemas[0].schema_bytes_digest.clone(),
        }
    }

    fn payload(fields: Vec<Value>) -> Payload {
        serde_json::from_value(json!({"fields": fields}))
            .unwrap_or_else(|_| Payload { fields: Vec::new() })
    }

    fn valid_payload() -> Payload {
        payload(vec![json!({
            "field_name": FIELD,
            "kind": "string",
            "value": "abstract-value",
        })])
    }

    #[test]
    fn valid_local_registry_loads_and_exposes_bindings() -> Result<(), PayloadRegistryError> {
        let registry = PayloadSchemaRegistry::load(&input())?;
        assert_eq!(
            registry.registry_id(),
            "test-only-payload-registry",
            "the local binding is preserved"
        );
        assert_eq!(
            registry.registry_version(),
            SUPPORTED_REGISTRY_VERSION,
            "the local binding is preserved"
        );
        assert_eq!(
            registry.registry_digest(),
            input().registry_digest,
            "the local binding is preserved"
        );
        let schema = &registry.schemas()[0];
        assert_eq!(schema.event_type(), EVENT, "the local binding is preserved");
        assert_eq!(
            schema.schema_id(),
            SCHEMA_ID,
            "the local binding is preserved"
        );
        assert_eq!(
            schema.schema_version(),
            VERSION,
            "the local binding is preserved"
        );
        assert_eq!(
            schema.schema_bytes_digest(),
            input().schemas[0].schema_bytes_digest,
            "the local binding is preserved"
        );
        assert_eq!(
            schema.validator_identity(),
            STRUCTURAL_VALIDATOR_V1,
            "the local binding is preserved"
        );
        Ok(())
    }

    #[test]
    fn unknown_registry_version_is_refused() {
        let mut candidate = input();
        candidate.registry_version = "nirnay-payload-registry-v2".to_owned();
        let refusal = PayloadSchemaRegistry::load(&candidate);
        assert!(
            matches!(
                refusal,
                Err(PayloadRegistryError::UnknownRegistryVersion { .. })
            ),
            "the typed refusal class is specific"
        );
    }

    #[test]
    fn registry_digest_mismatch_is_refused() {
        let mut candidate = input();
        candidate.registry_digest = "0".repeat(64);
        let refusal = PayloadSchemaRegistry::load(&candidate);
        assert!(
            matches!(refusal, Err(PayloadRegistryError::RegistryDigestMismatch)),
            "the typed refusal class is specific"
        );
    }

    #[test]
    fn corrupted_schema_bytes_are_refused() {
        let mut candidate = input();
        candidate.schemas = vec![schema_input(b"{invalid".to_vec())];
        candidate = PayloadRegistryInput::seal(
            candidate.registry_id,
            candidate.registry_version,
            candidate.schemas,
        );
        let refusal = PayloadSchemaRegistry::load(&candidate);
        assert!(
            matches!(refusal, Err(PayloadRegistryError::CorruptSchema { .. })),
            "the typed refusal class is specific"
        );
    }

    #[test]
    fn noncanonical_schema_bytes_are_refused() {
        let bytes = br#"{"format":"nirnay-structural-payload-v1","event_type":"registered-event", "required_fields":[{"field_name":"abstract-field","kind":"string"}]}"#.to_vec();
        let mut candidate = input();
        candidate.schemas = vec![schema_input(bytes)];
        candidate = PayloadRegistryInput::seal(
            candidate.registry_id,
            candidate.registry_version,
            candidate.schemas,
        );
        let refusal = PayloadSchemaRegistry::load(&candidate);
        assert!(
            matches!(refusal, Err(PayloadRegistryError::CorruptSchema { .. })),
            "the typed refusal class is specific"
        );
    }

    #[test]
    fn schema_bytes_digest_mismatch_is_refused() {
        let mut candidate = input();
        candidate.schemas[0].schema_bytes_digest = "0".repeat(64);
        candidate = PayloadRegistryInput::seal(
            candidate.registry_id,
            candidate.registry_version,
            candidate.schemas,
        );
        let refusal = PayloadSchemaRegistry::load(&candidate);
        assert!(
            matches!(
                refusal,
                Err(PayloadRegistryError::SchemaDigestMismatch { .. })
            ),
            "the typed refusal class is specific"
        );
    }

    #[test]
    fn unknown_validator_identity_is_refused() {
        let mut candidate = input();
        candidate.schemas[0].validator_identity = "unregistered-validator".to_owned();
        candidate = PayloadRegistryInput::seal(
            candidate.registry_id,
            candidate.registry_version,
            candidate.schemas,
        );
        let refusal = PayloadSchemaRegistry::load(&candidate);
        assert!(
            matches!(refusal, Err(PayloadRegistryError::UnknownValidator { .. })),
            "the typed refusal class is specific"
        );
    }

    #[test]
    fn duplicate_event_type_is_refused() {
        let mut candidate = input();
        let bytes = structural_schema_bytes(EVENT, &[(FIELD, "string")]).unwrap_or_default();
        let second = LocalSchemaInput::seal(
            EVENT.to_owned(),
            "other-schema".to_owned(),
            VERSION.to_owned(),
            STRUCTURAL_VALIDATOR_V1.to_owned(),
            bytes,
        );
        candidate.schemas.push(second);
        candidate = PayloadRegistryInput::seal(
            candidate.registry_id,
            candidate.registry_version,
            candidate.schemas,
        );
        let refusal = PayloadSchemaRegistry::load(&candidate);
        assert!(
            matches!(
                refusal,
                Err(PayloadRegistryError::DuplicateEventType { .. })
            ),
            "the typed refusal class is specific"
        );
    }

    #[test]
    fn duplicate_schema_identity_is_refused() {
        let mut candidate = input();
        let other_event = "other-event";
        let bytes = structural_schema_bytes(other_event, &[(FIELD, "string")]).unwrap_or_default();
        let second = LocalSchemaInput::seal(
            other_event.to_owned(),
            SCHEMA_ID.to_owned(),
            VERSION.to_owned(),
            STRUCTURAL_VALIDATOR_V1.to_owned(),
            bytes,
        );
        candidate.schemas.push(second);
        candidate = PayloadRegistryInput::seal(
            candidate.registry_id,
            candidate.registry_version,
            candidate.schemas,
        );
        let refusal = PayloadSchemaRegistry::load(&candidate);
        assert!(
            matches!(refusal, Err(PayloadRegistryError::DuplicateSchemaId { .. })),
            "the typed refusal class is specific"
        );
    }

    #[test]
    fn registry_digest_binds_each_schema_bytes_digest() {
        let first = input();
        let mut changed = first.clone();
        changed.schemas[0].schema_bytes_digest = "0".repeat(64);
        assert_ne!(
            first.registry_digest,
            PayloadRegistryInput::seal(
                changed.registry_id,
                changed.registry_version,
                changed.schemas,
            )
            .registry_digest,
            "a schema digest change must change the registry digest"
        );
    }

    #[test]
    fn valid_payload_passes_structural_validation() -> Result<(), PayloadRegistryError> {
        let registry = PayloadSchemaRegistry::load(&input())?;
        validate_record(&registry, EVENT, &declared(), &valid_payload())
    }

    #[test]
    fn wrong_declared_schema_id_is_refused() -> Result<(), PayloadRegistryError> {
        let registry = PayloadSchemaRegistry::load(&input())?;
        let mut declaration = declared();
        declaration.schema_id = "not-installed".to_owned();
        let refusal = validate_record(&registry, EVENT, &declaration, &valid_payload());
        assert!(
            matches!(refusal, Err(PayloadRegistryError::SchemaIdMismatch { .. })),
            "the typed refusal class is specific"
        );
        Ok(())
    }

    #[test]
    fn wrong_declared_schema_version_is_refused() -> Result<(), PayloadRegistryError> {
        let registry = PayloadSchemaRegistry::load(&input())?;
        let mut declaration = declared();
        declaration.schema_version = "2".to_owned();
        let refusal = validate_record(&registry, EVENT, &declaration, &valid_payload());
        assert!(
            matches!(
                refusal,
                Err(PayloadRegistryError::SchemaVersionMismatch { .. })
            ),
            "the typed refusal class is specific"
        );
        Ok(())
    }

    #[test]
    fn wrong_declared_digest_is_refused() -> Result<(), PayloadRegistryError> {
        let registry = PayloadSchemaRegistry::load(&input())?;
        let mut declaration = declared();
        declaration.schema_bytes_digest = "0".repeat(64);
        let refusal = validate_record(&registry, EVENT, &declaration, &valid_payload());
        assert!(
            matches!(
                refusal,
                Err(PayloadRegistryError::DeclaredDigestMismatch { .. })
            ),
            "the typed refusal class is specific"
        );
        Ok(())
    }

    #[test]
    fn unknown_event_is_refused() -> Result<(), PayloadRegistryError> {
        let registry = PayloadSchemaRegistry::load(&input())?;
        let refusal = validate_record(&registry, "other-event", &declared(), &valid_payload());
        assert!(
            matches!(refusal, Err(PayloadRegistryError::UnknownEvent { .. })),
            "the typed refusal class is specific"
        );
        Ok(())
    }

    #[test]
    fn wrong_kind_payload_is_refused() -> Result<(), PayloadRegistryError> {
        let registry = PayloadSchemaRegistry::load(&input())?;
        let invalid = payload(vec![json!({
            "field_name": FIELD,
            "kind": "boolean",
            "value": "true",
        })]);
        let refusal = validate_record(&registry, EVENT, &declared(), &invalid);
        assert!(
            matches!(refusal, Err(PayloadRegistryError::PayloadInvalid { .. })),
            "the typed refusal class is specific"
        );
        Ok(())
    }

    #[test]
    fn undeclared_payload_field_is_refused() -> Result<(), PayloadRegistryError> {
        let registry = PayloadSchemaRegistry::load(&input())?;
        let invalid = payload(vec![json!({
            "field_name": "unregistered-field",
            "kind": "string",
            "value": "abstract-value",
        })]);
        let refusal = validate_record(&registry, EVENT, &declared(), &invalid);
        assert!(
            matches!(refusal, Err(PayloadRegistryError::PayloadInvalid { .. })),
            "the typed refusal class is specific"
        );
        Ok(())
    }

    #[test]
    fn missing_required_payload_field_is_refused() -> Result<(), PayloadRegistryError> {
        let registry = PayloadSchemaRegistry::load(&input())?;
        let refusal = validate_record(&registry, EVENT, &declared(), &payload(Vec::new()));
        assert!(
            matches!(refusal, Err(PayloadRegistryError::PayloadInvalid { .. })),
            "the typed refusal class is specific"
        );
        Ok(())
    }

    #[test]
    fn duplicate_payload_field_is_refused() -> Result<(), PayloadRegistryError> {
        let registry = PayloadSchemaRegistry::load(&input())?;
        let invalid = payload(vec![
            json!({"field_name": FIELD, "kind": "string", "value": "one"}),
            json!({"field_name": FIELD, "kind": "string", "value": "two"}),
        ]);
        let refusal = validate_record(&registry, EVENT, &declared(), &invalid);
        assert!(
            matches!(refusal, Err(PayloadRegistryError::PayloadInvalid { .. })),
            "the typed refusal class is specific"
        );
        Ok(())
    }

    #[test]
    fn registry_error_maps_to_stage_five_typed_refusal() {
        let refusal = PayloadRegistryError::NotInstalled.into_verifier_error();
        assert_eq!(
            refusal.code(),
            "REJECT_PAYLOAD_REGISTRY",
            "typed refusal code is stable"
        );
        assert_eq!(refusal.stage(), 5, "refusal is raised at stage five");
    }
}
