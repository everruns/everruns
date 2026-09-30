use super::{ModelDescriptor, REGISTRY, get_model_profile, profile_data};
use crate::{ModelProfile, ModelVendor, ServiceKind};

/// A canonical registry identity and its capability/pricing profile.
///
/// `model_id` is the canonical lookup id, not necessarily a provider's request
/// id. `aliases` contains the other accepted lookup ids (including gateway
/// spellings); live discovery remains authoritative for request ids/availability.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelProfileEntry {
    pub model_id: String,
    pub aliases: Vec<String>,
    pub key: String,
    pub vendor: ModelVendor,
    pub service: ServiceKind,
    pub profile: ModelProfile,
}

fn profile_entry(descriptor: &ModelDescriptor, profile: ModelProfile) -> ModelProfileEntry {
    ModelProfileEntry {
        model_id: descriptor.ids[0].into(),
        aliases: descriptor.ids[1..].iter().map(|id| (*id).into()).collect(),
        key: format!("{}/{}", descriptor.vendor.slug(), descriptor.ids[0]),
        vendor: descriptor.vendor,
        service: descriptor.service,
        profile,
    }
}

/// Enumerate canonical registry entries once each, in registry order.
/// Profiles are provider-independent, as in [`super::get_model_profile_by_key`].
/// This is the curated registry, not the entire models.dev catalog; aliases and
/// dated ids do not produce extra entries. Order is not a recommendation ranking.
pub fn all_profile_entries() -> Vec<ModelProfileEntry> {
    REGISTRY
        .iter()
        .filter_map(|descriptor| {
            profile_data(descriptor.ids[0]).map(|profile| profile_entry(descriptor, profile))
        })
        .collect()
}

/// Enumerate canonical entries offered under a provider wire id, in registry
/// order, with the same capability masking as [`get_model_profile`]. Unknown
/// provider ids return an empty list. Registry coverage is not live availability.
pub fn profile_entries_for_provider(provider_type: &str) -> Vec<ModelProfileEntry> {
    REGISTRY
        .iter()
        .filter(|descriptor| descriptor.surfaces.contains(&provider_type))
        .filter_map(|descriptor| {
            get_model_profile(provider_type, descriptor.ids[0])
                .map(|profile| profile_entry(descriptor, profile))
        })
        .collect()
}

/// Enumerate every canonical profile in registry order, without provider masking.
/// Use [`all_profile_entries`] when model ids or service kinds are needed.
pub fn all_profiles() -> Vec<ModelProfile> {
    REGISTRY
        .iter()
        .filter_map(|descriptor| profile_data(descriptor.ids[0]))
        .collect()
}

/// Enumerate profiles for a provider wire id, applying lookup's capability masks.
/// Unknown provider ids return an empty list. Use [`profile_entries_for_provider`]
/// when model ids or service kinds are needed.
pub fn profiles_for_provider(provider_type: &str) -> Vec<ModelProfile> {
    REGISTRY
        .iter()
        .filter(|descriptor| descriptor.surfaces.contains(&provider_type))
        .filter_map(|descriptor| get_model_profile(provider_type, descriptor.ids[0]))
        .collect()
}

/// Enumerate the curated selection. Today the registry is itself the selection,
/// so this is identical to [`all_profiles`]; no independent preset list exists.
pub fn selected_profiles() -> Vec<ModelProfile> {
    all_profiles()
}

/// Enumerate the curated selection for a provider. Today this is identical to
/// [`profiles_for_provider`], including its provider capability masking.
pub fn selected_profiles_for_provider(provider_type: &str) -> Vec<ModelProfile> {
    profiles_for_provider(provider_type)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_enumeration_covers_every_descriptor_and_surface() {
        assert_eq!(all_profile_entries().len(), REGISTRY.len());
        for descriptor in REGISTRY {
            assert!(
                profile_data(descriptor.ids[0]).is_some(),
                "missing {}",
                descriptor.ids[0]
            );
            for provider in descriptor.surfaces {
                let entries = profile_entries_for_provider(provider);
                assert_eq!(
                    entries.len(),
                    REGISTRY
                        .iter()
                        .filter(|entry| entry.surfaces.contains(provider))
                        .count()
                );
                assert!(
                    entries
                        .iter()
                        .any(|entry| entry.model_id == descriptor.ids[0]),
                    "missing {provider}/{}",
                    descriptor.ids[0]
                );
            }
        }
    }
}
