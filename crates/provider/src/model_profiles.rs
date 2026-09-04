// Adapter over the `everruns-model-profiles` crate, which owns the model
// profile registry and lookup logic (knowledge/foundations/providers.md,
// knowledge/foundations/models.md). That crate does not depend on this one
// (it takes provider identity as a plain wire-id string rather than
// `DriverId`, to avoid a dependency cycle), so this module adapts the
// `DriverId`-typed signatures existing callers use.

use crate::driver_registry::ServiceKind;
use crate::model::{ModelProfile, ModelVendor};
use crate::provider::DriverId;

/// Get a model profile by matching provider_type and model_id.
/// Returns None if the id is not in the registry or is not offered under the
/// given provider type.
pub fn get_model_profile(provider_type: &DriverId, model_id: &str) -> Option<ModelProfile> {
    everruns_model_profiles::get_model_profile(provider_type.as_str(), model_id)
}

/// Estimate the USD cost of a generation from the model's static price-table
/// profile. See `everruns_model_profiles::estimate_cost_usd` for details.
pub fn estimate_cost_usd(
    provider_type: &DriverId,
    model_id: &str,
    input_tokens: u32,
    output_tokens: u32,
    cache_read_tokens: u32,
    cache_creation_tokens: u32,
) -> Option<f64> {
    everruns_model_profiles::estimate_cost_usd(
        provider_type.as_str(),
        model_id,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_creation_tokens,
    )
}

/// Get the vendor/brand for a model id, or None if it is not in the registry
/// (or not offered under the given provider type).
pub fn get_model_vendor(provider_type: &DriverId, model_id: &str) -> Option<ModelVendor> {
    everruns_model_profiles::get_model_vendor(provider_type.as_str(), model_id)
}

/// Stable public profile key: `"{vendor}/{canonical_id}"` (knowledge/foundations/providers.md).
pub fn get_model_profile_key(provider_type: &DriverId, model_id: &str) -> Option<String> {
    everruns_model_profiles::get_model_profile_key(provider_type.as_str(), model_id)
}

/// Look up a profile by its stable key (`"{vendor}/{canonical_id}"`).
pub fn get_model_profile_by_key(key: &str) -> Option<ModelProfile> {
    everruns_model_profiles::get_model_profile_by_key(key)
}

/// Which provider service a model belongs to. Unknown models default to
/// [`ServiceKind::Chat`].
pub fn get_model_service_kind(provider_type: &DriverId, model_id: &str) -> ServiceKind {
    everruns_model_profiles::get_model_service_kind(provider_type.as_str(), model_id)
}
