//! Stability: alpha — may change without a major bump; see [`stability`](crate::stability).
//!
//! What a provider offers, and what a model can do.
//!
//! Selecting a model needs an exact, provider-visible id. Applications that let
//! a person choose one — a picker, a `--model` flag, a settings page — need the
//! catalog behind it: which ids this provider serves, what they are called, and
//! what each one supports. That is the same provider edge an
//! [`Agent`](crate::Agent) or a [`Completion`](crate::Completion) uses, asked a
//! different question.
//!
//! [`list`] asks a provider for its catalog:
//!
//! ```no_run
//! use everruns::{Provider, models};
//!
//! # async fn run(provider: Provider) -> Result<(), Box<dyn std::error::Error>> {
//! for model in models::list(provider).await? {
//!     println!("{} — {}", model.id(), model.display_name().unwrap_or("?"));
//! }
//! # Ok(())
//! # }
//! ```
//!
//! With the `openai` feature that provider is `OpenAI::from_env()?`; see
//! [Supported providers](https://docs.everruns.com/framework/supported-providers/).
//!
//! Each entry carries the metadata a picker renders and converts back into a
//! [`Model`] the rest of the API takes, so a selection needs no string
//! handling. Catalogs are the provider's own answer: ids come back exactly as
//! chat calls expect them, merged with the profile registry for display names,
//! descriptions, limits, and prices the API does not report.

use everruns_provider::model::{ModelProfile, ModelVendor};
use everruns_provider::model_profiles::{get_model_profile, get_model_vendor};
use everruns_provider::provider::DriverId;
use everruns_provider::runtime_provider::Provider;

use crate::Model;
use std::fmt;

/// Why a provider's catalog could not be read.
///
/// [`NoCatalog`](Self::NoCatalog) is not a failure of the request: some
/// providers serve models without offering any way to enumerate them. Callers
/// that keep curated suggestions should fall back to them here rather than
/// reporting an error. [`Call`](Self::Call) carries the provider failure
/// verbatim, so the full [`LlmError`](crate::LlmError) classification survives.
#[derive(Debug)]
#[non_exhaustive]
pub enum CatalogError {
    /// The provider offers no model catalog.
    NoCatalog,
    /// The catalog request itself failed.
    Call(everruns_provider::error::AgentLoopError),
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CatalogError::NoCatalog => {
                write!(f, "provider offers no model catalog")
            }
            CatalogError::Call(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for CatalogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CatalogError::Call(error) => Some(error),
            CatalogError::NoCatalog => None,
        }
    }
}

impl From<everruns_provider::error::AgentLoopError> for CatalogError {
    fn from(error: everruns_provider::error::AgentLoopError) -> Self {
        CatalogError::Call(error)
    }
}

/// One model a provider offers.
///
/// Built by [`list`]. The id is the provider's own, ready to pass back
/// unchanged; everything else is display and capability metadata, absent when
/// neither the provider nor the profile registry knows it.
#[derive(Clone, Debug)]
pub struct ModelInfo {
    id: String,
    display_name: Option<String>,
    description: Option<String>,
    profile: Option<ModelProfile>,
    provider: Provider,
}

impl ModelInfo {
    /// The provider-visible model id, exactly as calls expect it.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// A human-readable name, when the provider or a profile supplies one.
    pub fn display_name(&self) -> Option<&str> {
        self.display_name.as_deref()
    }

    /// A short description of the model's strengths, when one is known.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The curated profile for this model: limits, prices, modalities, and
    /// capability flags. `None` for a model the registry does not carry.
    pub fn profile(&self) -> Option<&ModelProfile> {
        self.profile.as_ref()
    }

    /// The vendor that trained the model, which is not always the provider that
    /// serves it — an aggregator offers many vendors' models.
    pub fn vendor(&self) -> Option<ModelVendor> {
        get_model_vendor(&self.provider.driver_id(), &self.id)
    }

    /// Context window in tokens, when the profile registry knows it.
    pub fn context_window(&self) -> Option<u32> {
        self.profile
            .as_ref()
            .and_then(|profile| profile.limits.as_ref())
            .and_then(|limits| u32::try_from(limits.context).ok())
    }

    /// Whether the model supports tool calling, as far as the profile registry
    /// knows. `false` also covers "not in the registry", so treat it as a
    /// display hint rather than a guarantee.
    pub fn supports_tools(&self) -> bool {
        self.profile
            .as_ref()
            .is_some_and(|profile| profile.tool_call)
    }

    /// Whether the model reasons before answering, on the same best-effort
    /// basis as [`supports_tools`](Self::supports_tools).
    pub fn supports_reasoning(&self) -> bool {
        self.profile
            .as_ref()
            .is_some_and(|profile| profile.reasoning)
    }

    /// Turn the selection into a [`Model`], bundled with the provider it was
    /// discovered through and ready for an agent or a direct completion.
    pub fn model(&self) -> Model {
        Model::new(self.id.clone(), self.provider.clone())
    }
}

/// List the models `provider` offers, ready for display.
///
/// Ids come back exactly as the provider reports them, newest first, merged
/// with the profile registry. A provider that cannot enumerate its models
/// returns [`CatalogError::NoCatalog`]; the request itself failing returns
/// [`CatalogError::Call`].
///
/// This is a provider call: it costs a round trip and the catalog changes
/// rarely, so cache it rather than asking per keystroke.
pub async fn list(provider: impl Into<Provider>) -> Result<Vec<ModelInfo>, CatalogError> {
    let provider = provider.into();
    let models = provider.models().await?.ok_or(CatalogError::NoCatalog)?;
    Ok(models
        .into_iter()
        .map(|model| ModelInfo {
            // The catalog already merged the curated registry with what the
            // provider reported, so a model the registry has never heard of
            // still arrives with the limits its provider advertises. Looking
            // the id up again here would throw that half away.
            profile: model.profile,
            id: model.model_id,
            display_name: model.display_name,
            description: model.description,
            provider: provider.clone(),
        })
        .collect())
}

/// The curated profile for a model id offered under `driver`, without asking
/// the provider anything.
///
/// The registry is static data, so this is the offline answer to "what is this
/// model": limits, prices, modalities, and capability flags. `None` means the
/// id is not in the registry, which is not the same as the provider not
/// offering it.
pub fn profile(driver: &DriverId, model_id: &str) -> Option<ModelProfile> {
    get_model_profile(driver, model_id)
}

impl Model {
    /// The curated profile for this model, resolved through the provider it
    /// carries.
    ///
    /// `None` for a bare model id (nothing says which vendor's registry to
    /// consult) and for a model the registry does not carry. See
    /// [`models::profile`](profile) to look one up by driver and id.
    pub fn profile(&self) -> Option<ModelProfile> {
        let provider = self.bundled_provider()?;
        get_model_profile(&provider.driver_id(), self.id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LlmSimConfig;

    #[test]
    fn a_bare_model_id_has_no_profile() {
        // Nothing says which vendor's registry to consult.
        assert!(Model::from("gpt-5.6-terra").profile().is_none());
    }

    #[test]
    fn a_bundled_provider_resolves_the_profile() {
        let profile = Model::new(
            "gpt-5.6-terra",
            Provider::new(
                "openai",
                everruns_llmsim::LlmSimDriver::new(LlmSimConfig::fixed("unused")),
            )
            .with_driver_id(DriverId::OpenAI),
        )
        .profile()
        .expect("registry carries gpt-5.6-terra");
        assert_eq!(profile.family, "gpt-5.6-terra");
    }

    #[test]
    fn profile_lookup_is_offline_and_by_driver() {
        assert!(profile(&DriverId::OpenAI, "gpt-5.6-terra").is_some());
        assert!(profile(&DriverId::OpenAI, "not-a-model").is_none());
    }

    #[test]
    fn debug_output_carries_no_credential() {
        // `ModelInfo` holds the provider it was discovered through, so its
        // derived `Debug` inherits the provider's redaction. Assert that here:
        // catalogs are logged liberally by pickers.
        let info = ModelInfo {
            id: "gpt-5.6-terra".to_string(),
            display_name: None,
            description: None,
            profile: None,
            provider: Provider::new(
                "openai",
                everruns_llmsim::LlmSimDriver::new(LlmSimConfig::fixed("unused")),
            )
            .auth(everruns_provider::runtime_provider::BearerAuth::new(
                "sk-super-secret",
            )),
        };
        let rendered = format!("{info:?}");
        assert!(!rendered.contains("sk-super-secret"), "got {rendered}");
    }

    #[tokio::test]
    async fn a_provider_without_a_catalog_is_typed_not_an_error() {
        let provider = Provider::new(
            "llmsim",
            everruns_llmsim::LlmSimDriver::new(LlmSimConfig::fixed("unused")),
        );
        let error = list(provider).await.expect_err("simulator lists nothing");
        assert!(matches!(error, CatalogError::NoCatalog), "got {error:?}");
    }
}
