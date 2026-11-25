// SPDX-License-Identifier: Apache-2.0
//! Trusted local resolution of approved resource profiles.
//!
//! This module is the only path from a profile declaration to an enforcement
//! limit. The approved registry is held here, not supplied by the caller of the
//! streaming verifier. No production profile is registered: with an empty
//! registry the resolver refuses with [`ProfileError::NoApprovedProfile`] before
//! a release directory or any source is opened. Finite `TEST_ONLY` bindings are
//! constructed only inside `cfg(test)`; they carry no production authority.
//!
//! Resolution rechecks the role binding, canonical digest and approved version
//! of each entry. No numeric limit is invented or registered by this module;
//! the local ceiling is derived from the approved role profiles themselves.

use crate::resource_profile::EffectiveLimits;
#[cfg(test)]
use crate::resource_profile::LimitsInput;
use crate::resource_profile::ProcessingRole;
use crate::resource_profile::ProfileError;
use crate::resource_profile::ProfileLimits;
use crate::resource_profile::ResourceProfile;
use crate::resource_profile::ResourceProfileId;
use crate::resource_profile::StreamingResourcePolicy;
use crate::resource_profile::profile_digest;

/// One locally approved role binding and the profile it names.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ApprovedEntry {
    binding_role: ProcessingRole,
    id: ResourceProfileId,
    declared_digest: [u8; 32],
    profile: ResourceProfile,
}

/// Approved registry held by the trusted local resolver.
///
/// A new registry is empty. No numeric maximum is invented or recorded, and no
/// production entry exists until a ratified profile is approved in reviewed
/// source.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApprovedProfileRegistry {
    entries: Vec<ApprovedEntry>,
}

impl ApprovedProfileRegistry {
    /// Construct an empty registry with no approved role binding.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Report whether no role binding is approved.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Return the number of approved role bindings.
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
impl ApprovedProfileRegistry {
    pub(crate) fn register_test_binding(
        &mut self,
        binding_role: ProcessingRole,
        id: ResourceProfileId,
        profile_role: ProcessingRole,
        input: LimitsInput,
    ) -> Result<[u8; 32], ProfileError> {
        if !id.is_test_only() {
            return Err(ProfileError::Identifier {
                detail: "a test binding requires a TEST_ONLY identifier".to_owned(),
            });
        }
        let profile = ResourceProfile::new(id.clone(), profile_role, input)?;
        let digest = *profile.digest();
        self.entries.push(ApprovedEntry {
            binding_role,
            id,
            declared_digest: digest,
            profile,
        });
        Ok(digest)
    }

    pub(crate) fn register_test_profile(
        &mut self,
        id: ResourceProfileId,
        role: ProcessingRole,
        input: LimitsInput,
    ) -> Result<[u8; 32], ProfileError> {
        self.register_test_binding(role, id, role, input)
    }
}

/// Opaque handle to one approved, role-bound profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedProfile {
    profile: ResourceProfile,
}

impl ApprovedProfile {
    fn from_profile(profile: ResourceProfile) -> Self {
        Self { profile }
    }

    /// Return the approved versioned identifier.
    pub fn id(&self) -> &ResourceProfileId {
        self.profile.id()
    }

    /// Return the canonical digest of the approved profile.
    pub fn digest(&self) -> &[u8; 32] {
        self.profile.digest()
    }

    /// Return the role bound to this approved profile.
    pub fn role(&self) -> ProcessingRole {
        self.profile.role()
    }

    fn limits(&self) -> &ProfileLimits {
        self.profile.limits()
    }

    fn into_profile(self) -> ResourceProfile {
        self.profile
    }
}

/// Trusted local resolver of approved resource profiles.
///
/// The resolver owns its registry. The streaming verifier calls it at the start
/// of the bounded path; no caller-supplied policy reaches this type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileResolver {
    registry: ApprovedProfileRegistry,
}

impl ProfileResolver {
    /// Bind a resolver to a locally held approved registry.
    pub fn new(registry: ApprovedProfileRegistry) -> Self {
        Self { registry }
    }

    /// Resolve the approved profile bound to one role.
    ///
    /// An empty registry returns [`ProfileError::NoApprovedProfile`]. A binding
    /// is returned only when its role, digest and approved version all agree.
    pub fn resolve(&self, role: ProcessingRole) -> Result<ApprovedProfile, ProfileError> {
        let mut candidates = self
            .registry
            .entries
            .iter()
            .filter(|entry| entry.binding_role == role);
        let entry = match candidates.next() {
            Some(entry) => entry,
            None => {
                return Err(ProfileError::NoApprovedProfile {
                    role: role.as_str().to_owned(),
                });
            }
        };
        if candidates.next().is_some() {
            return Err(ProfileError::Identifier {
                detail: "ambiguous approved role binding".to_owned(),
            });
        }
        if entry.profile.role() != role {
            return Err(ProfileError::Role {
                detail: "approved binding role differs from the profile role".to_owned(),
            });
        }
        if entry.id.version() == 0 || entry.profile.id() != &entry.id {
            return Err(ProfileError::Version {
                detail: "approved version differs from the bound profile".to_owned(),
            });
        }
        let recomputed = profile_digest(
            entry.profile.id(),
            entry.profile.role(),
            entry.profile.limits(),
        );
        if recomputed != entry.declared_digest || entry.profile.digest() != &recomputed {
            return Err(ProfileError::DigestMismatch);
        }
        if entry.profile.id().is_test_only() && !cfg!(test) {
            return Err(ProfileError::NoApprovedProfile {
                role: role.as_str().to_owned(),
            });
        }
        Ok(ApprovedProfile::from_profile(entry.profile.clone()))
    }

    /// Build the streaming selection from approved entries only.
    pub(crate) fn streaming_policy(&self) -> Result<StreamingResourcePolicy, ProfileError> {
        let verifier = self.resolve(ProcessingRole::Verifier)?;
        let snapshot = self.resolve(ProcessingRole::Snapshot)?;
        let ceiling =
            EffectiveLimits::combine(verifier.limits(), snapshot.limits(), None)?.into_limits();
        let mut policy = StreamingResourcePolicy::new(
            ceiling,
            verifier.into_profile(),
            snapshot.into_profile(),
        )?;
        match self.resolve(ProcessingRole::Producer) {
            Ok(producer) => {
                policy = policy.with_producer_declaration(producer.into_profile())?;
            }
            Err(ProfileError::NoApprovedProfile { .. }) => {}
            Err(error) => return Err(error),
        }
        Ok(policy)
    }
}

#[cfg(test)]
mod tests {
    use super::ApprovedEntry;
    use super::ApprovedProfileRegistry;
    use super::ProfileResolver;
    use crate::resource_profile::LimitsInput;
    use crate::resource_profile::ProcessingRole;
    use crate::resource_profile::ProfileError;
    use crate::resource_profile::ResourceProfile;
    use crate::resource_profile::ResourceProfileId;

    fn finite_limits() -> LimitsInput {
        LimitsInput {
            max_record_bytes: 512,
            max_records_per_artifact: 32,
            max_records_per_release: 128,
            max_stored_bytes_per_artifact: 16_384,
            max_stored_bytes_per_release: 65_536,
            max_expanded_bytes_per_artifact: 65_536,
            max_expanded_bytes_per_release: 262_144,
            max_decompression_budget_bytes: 65_536,
            max_expansion_ratio: 16,
            codec_window_bytes: 32_768,
            io_chunk_bytes: 64,
            max_nesting_depth: 8,
            max_string_bytes: 128,
            max_token_bytes: 64,
            max_object_members: 32,
            max_array_elements: 32,
            max_concurrent_artifacts: 4,
            worker_count: 4,
            queue_capacity_records: 16,
            temp_disk_bytes: 65_536,
            max_processing_duration_ms: 30_000,
            max_journal_record_bytes: 4_096,
            max_preparation_intent_bytes: 65_536,
            max_sealed_intent_bytes: 4_096,
            max_open_file_descriptors: 16,
            memory_budget_bytes: 268_435_456,
            max_artifacts: 4,
        }
    }

    fn verifier_registry() -> Result<ApprovedProfileRegistry, ProfileError> {
        let mut registry = ApprovedProfileRegistry::new();
        registry.register_test_profile(
            ResourceProfileId::new("TEST_ONLY_RESOLVER_VERIFIER_V1", 1)?,
            ProcessingRole::Verifier,
            finite_limits(),
        )?;
        Ok(registry)
    }

    fn streaming_registry() -> Result<ApprovedProfileRegistry, ProfileError> {
        let mut registry = verifier_registry()?;
        registry.register_test_profile(
            ResourceProfileId::new("TEST_ONLY_RESOLVER_SNAPSHOT_V1", 1)?,
            ProcessingRole::Snapshot,
            finite_limits(),
        )?;
        Ok(registry)
    }

    #[test]
    fn empty_registry_has_no_numeric_limits() {
        let registry = ApprovedProfileRegistry::new();
        assert!(registry.is_empty(), "a new registry is empty");
        assert_eq!(registry.len(), 0, "no role binding is pre-registered");
    }

    #[test]
    fn empty_registry_refuses_role_resolution() {
        let resolver = ProfileResolver::new(ApprovedProfileRegistry::new());
        let refusal = resolver.resolve(ProcessingRole::Verifier);
        assert!(
            matches!(
                refusal,
                Err(ProfileError::NoApprovedProfile { ref role }) if role == "VERIFIER"
            ),
            "an empty registry refuses with a typed role error"
        );
    }

    #[test]
    fn empty_registry_refuses_streaming_selection() {
        let resolver = ProfileResolver::new(ApprovedProfileRegistry::new());
        assert!(
            matches!(
                resolver.streaming_policy(),
                Err(ProfileError::NoApprovedProfile { .. })
            ),
            "selection fails closed before any filesystem access"
        );
    }

    #[test]
    fn approved_profile_is_a_role_bound_handle() -> Result<(), Box<dyn std::error::Error>> {
        let resolver = ProfileResolver::new(verifier_registry()?);
        let approved = resolver.resolve(ProcessingRole::Verifier)?;
        assert_eq!(approved.role(), ProcessingRole::Verifier, "role is bound");
        assert_eq!(approved.id().version(), 1, "approved version is retained");
        assert_eq!(approved.digest().len(), 32, "canonical digest is retained");
        Ok(())
    }

    #[test]
    fn wrong_role_binding_is_refused() -> Result<(), Box<dyn std::error::Error>> {
        let mut registry = ApprovedProfileRegistry::new();
        registry.register_test_binding(
            ProcessingRole::Producer,
            ResourceProfileId::new("TEST_ONLY_RESOLVER_WRONG_ROLE_V1", 1)?,
            ProcessingRole::Snapshot,
            finite_limits(),
        )?;
        let resolver = ProfileResolver::new(registry);
        assert!(
            matches!(
                resolver.resolve(ProcessingRole::Producer),
                Err(ProfileError::Role { .. })
            ),
            "a binding role that differs from the profile role is refused"
        );
        Ok(())
    }

    #[test]
    fn wrong_digest_is_refused() -> Result<(), Box<dyn std::error::Error>> {
        let profile = ResourceProfile::new(
            ResourceProfileId::new("TEST_ONLY_RESOLVER_BAD_DIGEST_V1", 1)?,
            ProcessingRole::Verifier,
            finite_limits(),
        )?;
        let mut registry = ApprovedProfileRegistry::new();
        registry.entries.push(ApprovedEntry {
            binding_role: ProcessingRole::Verifier,
            id: ResourceProfileId::new("TEST_ONLY_RESOLVER_BAD_DIGEST_V1", 1)?,
            declared_digest: [0_u8; 32],
            profile,
        });
        let resolver = ProfileResolver::new(registry);
        assert!(
            matches!(
                resolver.resolve(ProcessingRole::Verifier),
                Err(ProfileError::DigestMismatch)
            ),
            "a digest that differs from the approved declaration is refused"
        );
        Ok(())
    }

    #[test]
    fn wrong_version_is_refused() -> Result<(), Box<dyn std::error::Error>> {
        let profile = ResourceProfile::new(
            ResourceProfileId::new("TEST_ONLY_RESOLVER_BAD_VERSION_V2", 2)?,
            ProcessingRole::Verifier,
            finite_limits(),
        )?;
        let mut registry = ApprovedProfileRegistry::new();
        registry.entries.push(ApprovedEntry {
            binding_role: ProcessingRole::Verifier,
            id: ResourceProfileId::new("TEST_ONLY_RESOLVER_BAD_VERSION_V1", 1)?,
            declared_digest: *profile.digest(),
            profile,
        });
        let resolver = ProfileResolver::new(registry);
        assert!(
            matches!(
                resolver.resolve(ProcessingRole::Verifier),
                Err(ProfileError::Version { .. })
            ),
            "a version that differs from the approved binding is refused"
        );
        Ok(())
    }

    #[test]
    fn ambiguous_binding_is_refused() -> Result<(), Box<dyn std::error::Error>> {
        let mut registry = verifier_registry()?;
        registry.register_test_profile(
            ResourceProfileId::new("TEST_ONLY_RESOLVER_VERIFIER_ALT_V1", 1)?,
            ProcessingRole::Verifier,
            finite_limits(),
        )?;
        let resolver = ProfileResolver::new(registry);
        assert!(
            matches!(
                resolver.resolve(ProcessingRole::Verifier),
                Err(ProfileError::Identifier { .. })
            ),
            "two bindings for one role are ambiguous and refused"
        );
        Ok(())
    }

    #[test]
    fn missing_second_role_refuses_streaming_selection() -> Result<(), Box<dyn std::error::Error>> {
        let resolver = ProfileResolver::new(verifier_registry()?);
        assert!(
            matches!(
                resolver.streaming_policy(),
                Err(ProfileError::NoApprovedProfile { ref role }) if role == "SNAPSHOT"
            ),
            "streaming selection requires both approved consumer roles"
        );
        Ok(())
    }

    #[test]
    fn producer_declaration_cannot_raise_the_consumer_limit()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut registry = streaming_registry()?;
        let mut generous = finite_limits();
        generous.max_record_bytes = 4_096;
        registry.register_test_profile(
            ResourceProfileId::new("TEST_ONLY_RESOLVER_GENEROUS_PRODUCER_V1", 1)?,
            ProcessingRole::Producer,
            generous,
        )?;
        let policy = ProfileResolver::new(registry).streaming_policy()?;
        assert_eq!(
            policy.verifier_limits()?.max_record_bytes(),
            512,
            "the approved consumer ceiling governs the effective limit"
        );
        Ok(())
    }

    #[test]
    fn producer_declaration_can_lower_the_consumer_limit() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut registry = streaming_registry()?;
        let mut strict = finite_limits();
        strict.max_record_bytes = 256;
        registry.register_test_profile(
            ResourceProfileId::new("TEST_ONLY_RESOLVER_STRICT_PRODUCER_V1", 1)?,
            ProcessingRole::Producer,
            strict,
        )?;
        let policy = ProfileResolver::new(registry).streaming_policy()?;
        assert_eq!(
            policy.verifier_limits()?.max_record_bytes(),
            256,
            "the approved producer declaration lowers the effective limit"
        );
        Ok(())
    }

    #[test]
    fn test_registration_requires_a_test_only_identifier() {
        let mut registry = ApprovedProfileRegistry::new();
        assert!(
            matches!(
                registry.register_test_profile(
                    ResourceProfileId::new("PRODUCTION_VERIFIER_V1", 1)
                        .expect("identifier is valid"),
                    ProcessingRole::Verifier,
                    finite_limits(),
                ),
                Err(ProfileError::Identifier { .. })
            ),
            "a production-looking identifier cannot enter the test registry"
        );
    }
}
