//! Which image model a call uses, on which provider account, and the egress
//! transport the driver sends through.
//!
//! Resolution order:
//! 1. the capability's `model` setting: a catalog model id (`model_...`) is
//!    resolved by the host and never falls back to another account; a plain
//!    model name is looked up on the org's default provider of each type
//!    that serves it,
//! 2. the organization's default image model,
//! 3. the platform default, GPT Image 2.6 Flare,
//! 4. with `fallback: auto`, Muse Image on a Meta or OpenRouter account.
//!
//! A provider that cannot produce the requested format (GIF and SVG are
//! OpenAI and Azure OpenAI only) is skipped, so a GIF request with only an
//! OpenRouter account fails with a clear message instead of a provider 400.

use super::capability::{ImageGenerationConfig, ImageModelFallback};
use async_trait::async_trait;
use everruns_contracts::images::{
    ImagesDriverError, ImagesHttpBody, ImagesHttpRequest, ImagesHttpResponse, ImagesTransport,
};
use everruns_contracts::model_profile_data::{ImageModelProfile, get_model_profile};
use everruns_contracts::runtime::connection_services::ProviderCredentialStore;
use everruns_contracts::runtime::network_access::NetworkAccessList;
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tools::ToolExecutionResult;
use everruns_contracts::runtime::{EgressError, EgressRequest, EgressRequestKind, EgressService};
use everruns_drivers::openai::images::OpenAIImagesDriver;
use futures::StreamExt;
use std::sync::Arc;

/// Used when neither the capability nor the organization names a model.
pub const PLATFORM_DEFAULT_IMAGE_MODEL: &str = "gpt-image-2.6-flare";
/// Fallback when no provider serves the chosen GPT image model.
const MUSE_IMAGE_MODEL: &str = "muse-image-1.0";

const NOT_CONFIGURED: &str = "Image credentials are not configured. Configure an OpenAI, Azure OpenAI or OpenRouter provider (GPT Image models), a Meta provider (Muse Image), or choose a default image model for the organization.";

/// The model and account one image call uses.
#[derive(Debug, Clone)]
pub(crate) struct ResolvedImageModel {
    pub provider_type: String,
    pub api_key: String,
    pub base_url: Option<String>,
    pub headers: Vec<(String, String)>,
    /// Model id sent to the provider (`openai/gpt-image-2.6-flare` on
    /// OpenRouter).
    pub api_model: String,
    /// Image capabilities of the model on this provider, when known. Custom
    /// models have none and are passed through unchecked.
    pub profile: Option<ImageModelProfile>,
}

/// Provider types that serve a model, in preference order.
fn provider_candidates(model: &str) -> &'static [&'static str] {
    if model.starts_with("muse-image") {
        &["meta", "openrouter"]
    } else if model.starts_with("gemini-") {
        &["openrouter"]
    } else {
        &["openai", "azure_openai", "openrouter"]
    }
}

/// OpenRouter namespaces models by vendor; the first-party APIs do not.
pub(crate) fn api_model_id(provider_type: &str, model: &str) -> String {
    if provider_type != "openrouter" || model.contains('/') {
        return model.to_string();
    }
    if model.starts_with("muse-image") {
        "meta/muse-image".to_string()
    } else if model.starts_with("gemini-") {
        format!("google/{model}")
    } else {
        format!("openai/{model}")
    }
}

/// True when `model` is a model name this capability knows how to route.
pub(crate) fn is_known_image_model(model: &str) -> bool {
    provider_candidates(model).iter().any(|provider_type| {
        get_model_profile(provider_type, model).is_some_and(|profile| profile.images.is_some())
    })
}

fn image_profile(provider_type: &str, model: &str) -> Option<ImageModelProfile> {
    get_model_profile(provider_type, model).and_then(|profile| profile.images)
}

/// Resolve with the host's credential store, or `fallback` (credentials an
/// embedding application gave the capability) when the host has none.
pub(crate) async fn resolve_image_model(
    context: &ToolContext,
    fallback: Option<&Arc<dyn ProviderCredentialStore>>,
    config: &ImageGenerationConfig,
    format: &str,
) -> Result<ResolvedImageModel, ToolExecutionResult> {
    let Some(store) = context.provider_credential_store.as_ref().or(fallback) else {
        return Err(ToolExecutionResult::tool_error(NOT_CONFIGURED));
    };

    let catalog_id = config
        .model
        .as_deref()
        .filter(|model| model.starts_with("model_"));
    if config.model.is_none() || catalog_id.is_some() {
        match store.get_image_model(catalog_id, context.session_id).await {
            Ok(Some(binding)) => {
                let profile = image_profile(&binding.provider_type, &binding.model);
                if let Some(profile) = &profile
                    && !profile.supports_format(format)
                {
                    return Err(ToolExecutionResult::tool_error(format!(
                        "The image model {} cannot make {format} images on {}. Pick another format or choose a GPT Image 2.6 model on an OpenAI or Azure OpenAI provider.",
                        binding.model, binding.provider_type
                    )));
                }
                return Ok(ResolvedImageModel {
                    provider_type: binding.provider_type,
                    api_key: binding.api_key,
                    base_url: binding.base_url,
                    headers: binding.headers.into_iter().collect(),
                    api_model: binding.model,
                    profile,
                });
            }
            Ok(None) if catalog_id.is_some() => {
                return Err(ToolExecutionResult::tool_error(format!(
                    "The image model {} is not available in this organization.",
                    catalog_id.unwrap_or_default()
                )));
            }
            Ok(None) => {}
            Err(error) => return Err(ToolExecutionResult::internal_error(error)),
        }
    }

    let model = config
        .model
        .as_deref()
        .unwrap_or(PLATFORM_DEFAULT_IMAGE_MODEL);
    let mut skipped_for_format = false;
    if let Some(resolved) = by_provider_type(store, model, format, &mut skipped_for_format).await? {
        return Ok(resolved);
    }
    if config.fallback == ImageModelFallback::Auto
        && !model.starts_with("muse-image")
        && let Some(resolved) =
            by_provider_type(store, MUSE_IMAGE_MODEL, format, &mut skipped_for_format).await?
    {
        return Ok(resolved);
    }
    if skipped_for_format {
        return Err(ToolExecutionResult::tool_error(format!(
            "{format} output needs GPT Image 2.6 on an OpenAI or Azure OpenAI provider, and none is configured."
        )));
    }
    Err(ToolExecutionResult::tool_error(NOT_CONFIGURED))
}

async fn by_provider_type(
    store: &Arc<dyn ProviderCredentialStore>,
    model: &str,
    format: &str,
    skipped_for_format: &mut bool,
) -> Result<Option<ResolvedImageModel>, ToolExecutionResult> {
    for provider_type in provider_candidates(model) {
        let profile = image_profile(provider_type, model);
        if let Some(profile) = &profile
            && !profile.supports_format(format)
        {
            *skipped_for_format = true;
            continue;
        }
        match store.get_default_provider_credentials(provider_type).await {
            Ok(Some(credentials)) => {
                return Ok(Some(ResolvedImageModel {
                    provider_type: (*provider_type).to_string(),
                    api_key: credentials.api_key,
                    base_url: credentials.base_url,
                    headers: Vec::new(),
                    api_model: api_model_id(provider_type, model),
                    profile,
                }));
            }
            Ok(None) => {}
            Err(error) => return Err(ToolExecutionResult::internal_error(error)),
        }
    }
    Ok(None)
}

/// The driver for a resolved model, sending through the host egress service.
pub(crate) fn build_driver(
    context: &ToolContext,
    resolved: &ResolvedImageModel,
) -> Result<OpenAIImagesDriver, ToolExecutionResult> {
    // THREAT[TM-LLM-047]: the org-configured base URL and decrypted key only
    // leave through the host egress boundary (ACL, allowlist, DNS pinning, no
    // redirects). No egress service means no image traffic, never a direct
    // dial (EVE-1174).
    let Some(egress) = context.egress_service.clone() else {
        return Err(ToolExecutionResult::tool_error(
            "Image generation requires the host egress service, which is not available in this environment.",
        ));
    };
    let transport = Arc::new(EgressImagesTransport {
        egress,
        network_access: context.network_access.clone(),
    });
    Ok(OpenAIImagesDriver::new(
        transport,
        resolved.provider_type.clone(),
        resolved.api_key.clone(),
        resolved.base_url.clone(),
    )
    .with_headers(resolved.headers.clone()))
}

/// [`ImagesTransport`] over the host [`EgressService`]: the merged network
/// ACL and system allowlist apply, DNS is resolved and pinned inside the
/// egress boundary, and redirects come back as responses the driver rejects.
pub(crate) struct EgressImagesTransport {
    pub egress: Arc<dyn EgressService>,
    pub network_access: Option<NetworkAccessList>,
}

#[async_trait]
impl ImagesTransport for EgressImagesTransport {
    async fn send(
        &self,
        request: ImagesHttpRequest,
    ) -> Result<ImagesHttpResponse, ImagesDriverError> {
        let mut egress_request =
            EgressRequest::new("POST", request.url.as_str(), EgressRequestKind::Capability)
                .network_access(self.network_access.clone())
                .require_dns_pinning()
                .timeout_ms(request.timeout_ms);
        for (name, value) in request.headers {
            egress_request = egress_request.header(name, value);
        }
        let egress_request = egress_request.body(request.body);
        if request.stream {
            let response = self
                .egress
                .send_stream(egress_request)
                .await
                .map_err(egress_error)?;
            let body = response
                .body
                .map(|chunk| chunk.map_err(|error| error.to_string()))
                .boxed();
            Ok(ImagesHttpResponse {
                status: response.status,
                body: ImagesHttpBody::Stream(body),
            })
        } else {
            let response = self
                .egress
                .send(egress_request)
                .await
                .map_err(egress_error)?;
            Ok(ImagesHttpResponse {
                status: response.status,
                body: ImagesHttpBody::Full(response.body),
            })
        }
    }
}

fn egress_error(error: EgressError) -> ImagesDriverError {
    match error {
        EgressError::NetworkAccessDenied { .. } => ImagesDriverError(format!(
            "image request blocked by network access policy: {error}"
        )),
        other => ImagesDriverError(format!("failed to call the image API: {other}")),
    }
}
