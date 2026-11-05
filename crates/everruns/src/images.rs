//! Image generation for Framework applications.
//!
//! The `images` feature registers the `image_generation` capability: agents
//! that list it get `generate_image` and `edit_image`, making stills,
//! animated GIFs and SVGs. A Framework app has no organization catalog, so
//! the capability finds provider accounts in the environment through
//! [`EnvImageCredentials`]:
//!
//! | Provider | Variables |
//! |---|---|
//! | OpenAI | `OPENAI_API_KEY`, optional `OPENAI_BASE_URL` |
//! | Azure OpenAI | `AZURE_OPENAI_API_KEY`, `AZURE_OPENAI_BASE_URL` |
//! | OpenRouter | `OPENROUTER_API_KEY` |
//! | Meta | `META_API_KEY` or `LLAMA_API_KEY` |
//!
//! Choose the model in the capability settings; without one, agents use
//! GPT Image 2.6 Flare:
//!
//! ```rust
//! use everruns::{Agent, CapabilityRef};
//!
//! let agent = Agent::builder("animator")
//!     .model("gpt-6.1-sol")
//!     .instructions("Make short looping GIFs.")
//!     .capability(CapabilityRef::with_config(
//!         "image_generation",
//!         serde_json::json!({ "model": "gpt-image-2.6-sunburst", "default_format": "gif" }),
//!     ))
//!     .build()
//!     .unwrap();
//! # let _ = agent;
//! ```
//!
//! Without an image store, generated images are written to the session
//! filesystem under `/workspace/.outputs/images`.

use async_trait::async_trait;
use everruns_contracts::runtime::connection_services::{
    ProviderCredentialStore, ProviderCredentials,
};

pub use everruns_integrations::image_generation::{
    ImageGenerationCapability as ImageGeneration, PLATFORM_DEFAULT_IMAGE_MODEL,
};

/// Image provider accounts read from environment variables, one per
/// provider type. See the [module docs](self) for the variables.
#[derive(Debug, Clone, Copy, Default)]
pub struct EnvImageCredentials;

impl EnvImageCredentials {
    fn lookup(provider_type: &str) -> Option<ProviderCredentials> {
        let var = |name: &str| {
            std::env::var(name)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        let (key, base_url) = match provider_type {
            "openai" => (var("OPENAI_API_KEY")?, var("OPENAI_BASE_URL")),
            "azure_openai" => (
                var("AZURE_OPENAI_API_KEY")?,
                Some(var("AZURE_OPENAI_BASE_URL")?),
            ),
            "openrouter" => (var("OPENROUTER_API_KEY")?, None),
            "meta" => (var("META_API_KEY").or_else(|| var("LLAMA_API_KEY"))?, None),
            _ => return None,
        };
        Some(ProviderCredentials {
            api_key: key,
            base_url,
        })
    }
}

#[async_trait]
impl ProviderCredentialStore for EnvImageCredentials {
    async fn get_default_provider_credentials(
        &self,
        provider_type: &str,
    ) -> everruns_contracts::error::Result<Option<ProviderCredentials>> {
        Ok(Self::lookup(provider_type))
    }
}
