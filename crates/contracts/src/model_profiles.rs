// Typed access to the model profile registry in this crate's model_profile_data.
// The data uses plain provider wire IDs; these helpers accept DriverId.
// See knowledge/foundations/providers.md and knowledge/foundations/models.md.

use crate::driver_registry::ServiceKind;
use crate::model::{ModelProfile, ModelVendor};
use crate::provider::DriverId;

pub use crate::model_profile_data::{
    ModelProfileEntry, all_profile_entries, all_profiles, selected_profiles,
};

/// Enumerate profiles for a driver with the same capability masks as lookup.
pub fn profiles_for_provider(provider_type: &DriverId) -> Vec<ModelProfile> {
    crate::model_profile_data::profiles_for_provider(provider_type.as_str())
}

/// Enumerate the curated selection for a driver (currently the full registry).
pub fn selected_profiles_for_provider(provider_type: &DriverId) -> Vec<ModelProfile> {
    crate::model_profile_data::selected_profiles_for_provider(provider_type.as_str())
}

/// Enumerate canonical model identities and masked profiles for a driver.
pub fn profile_entries_for_provider(provider_type: &DriverId) -> Vec<ModelProfileEntry> {
    crate::model_profile_data::profile_entries_for_provider(provider_type.as_str())
}

/// Get a model profile by matching provider_type and model_id.
/// Returns None if the id is not in the registry or is not offered under the
/// given provider type.
pub fn get_model_profile(provider_type: &DriverId, model_id: &str) -> Option<ModelProfile> {
    crate::model_profile_data::get_model_profile(provider_type.as_str(), model_id)
}

/// Estimate the USD cost of a generation from the model's static price-table
/// profile. See `crate::model_profile_data::estimate_cost_usd` for details.
pub fn estimate_cost_usd(
    provider_type: &DriverId,
    model_id: &str,
    input_tokens: u32,
    output_tokens: u32,
    cache_read_tokens: u32,
    cache_creation_tokens: u32,
) -> Option<f64> {
    crate::model_profile_data::estimate_cost_usd(
        provider_type.as_str(),
        model_id,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_creation_tokens,
    )
}

/// [`estimate_cost_usd`] priced at the service tier that served the request.
/// See `crate::model_profile_data::estimate_cost_usd_for_speed`.
pub fn estimate_cost_usd_for_speed(
    provider_type: &DriverId,
    model_id: &str,
    input_tokens: u32,
    output_tokens: u32,
    cache_read_tokens: u32,
    cache_creation_tokens: u32,
    served_tier: Option<&str>,
) -> Option<f64> {
    crate::model_profile_data::estimate_cost_usd_for_speed(
        provider_type.as_str(),
        model_id,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_creation_tokens,
        served_tier,
    )
}

/// Get the vendor/brand for a model id, or None if it is not in the registry
/// (or not offered under the given provider type).
pub fn get_model_vendor(provider_type: &DriverId, model_id: &str) -> Option<ModelVendor> {
    crate::model_profile_data::get_model_vendor(provider_type.as_str(), model_id)
}

/// Stable public profile key: `"{vendor}/{canonical_id}"`.
pub fn get_model_profile_key(provider_type: &DriverId, model_id: &str) -> Option<String> {
    crate::model_profile_data::get_model_profile_key(provider_type.as_str(), model_id)
}

/// Look up a profile by its stable key (`"{vendor}/{canonical_id}"`).
pub fn get_model_profile_by_key(key: &str) -> Option<ModelProfile> {
    crate::model_profile_data::get_model_profile_by_key(key)
}

/// Which provider service a model belongs to. Unknown models default to
/// [`ServiceKind::Chat`].
pub fn get_model_service_kind(provider_type: &DriverId, model_id: &str) -> ServiceKind {
    crate::model_profile_data::get_model_service_kind(provider_type.as_str(), model_id)
}
