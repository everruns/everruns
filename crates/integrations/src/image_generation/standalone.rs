//! One still image made outside a session.
//!
//! Hosts use this where no agent turn is running, such as generating an
//! agent's avatar from the CLI or MCP. It resolves the model the same way the
//! `image_generation` capability does (the organization's default image
//! model, then GPT Image 2.6 Flare on the org's default provider, then Muse
//! Image) and sends through the host egress service, but writes no session
//! file or image artifact: the caller gets the bytes.

use super::capability::ImageGenerationConfig;
use super::resolve::{driver_over, resolve_from};
use super::tools::{Animation, CommonImageArgs, build_request};
use base64::Engine as _;
use everruns_contracts::images::ImagesDriver;
use everruns_contracts::runtime::EgressService;
use everruns_contracts::runtime::connection_services::{
    ImageModelBinding, ProviderCredentialStore,
};
use std::sync::Arc;

/// Size asked for when the model lists it or publishes no sizes at all.
const SQUARE_SIZE: &str = "1024x1024";

/// What to draw.
#[derive(Debug, Clone, Default)]
pub struct StandaloneImageRequest {
    /// The full prompt sent to the model.
    pub prompt: String,
    /// `low`, `medium`, `high`, ... when the model takes a quality.
    pub quality: Option<String>,
    /// `transparent`, `opaque` or `auto`.
    pub background: Option<String>,
    /// Ask for a square image when the model offers one.
    pub square: bool,
}

/// The generated image and where it came from.
#[derive(Debug, Clone)]
pub struct StandaloneImage {
    /// Decoded PNG bytes.
    pub data: Vec<u8>,
    /// Always `image/png`.
    pub media_type: &'static str,
    /// Provider type that drew it (`openai`, `openrouter`, ...).
    pub provider_type: String,
    /// Model id as sent to the provider.
    pub model: String,
    /// The prompt the model rewrote, when it reports one.
    pub revised_prompt: Option<String>,
}

/// Why no image came back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StandaloneImageError {
    /// No usable model or account, or an option the model does not take.
    /// Fixing it is the caller's or the org admin's job.
    Setup(String),
    /// The provider call failed or returned nothing usable.
    Provider(String),
}

impl std::fmt::Display for StandaloneImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StandaloneImageError::Setup(message) | StandaloneImageError::Provider(message) => {
                f.write_str(message)
            }
        }
    }
}

impl std::error::Error for StandaloneImageError {}

/// Generate one PNG.
///
/// `org_default` is the organization's default image model as the host bound
/// it, or `None` when the org has not chosen one. `credentials` supplies the
/// org's default account per provider type for the fallbacks. Error messages
/// are fit to show the caller.
pub async fn generate_standalone_image(
    org_default: Option<ImageModelBinding>,
    credentials: &Arc<dyn ProviderCredentialStore>,
    egress: Arc<dyn EgressService>,
    request: StandaloneImageRequest,
) -> Result<StandaloneImage, StandaloneImageError> {
    let config = ImageGenerationConfig::default();
    let resolved = resolve_from(org_default, credentials, &config, "png")
        .await
        .map_err(|error| StandaloneImageError::Setup(error.to_string()))?;
    let size = request.square.then(|| match &resolved.profile {
        None => Some(SQUARE_SIZE.to_string()),
        Some(profile) if profile.sizes.iter().any(|size| size == SQUARE_SIZE) => {
            Some(SQUARE_SIZE.to_string())
        }
        // A model without 1024x1024 gets its first square size, if any; the
        // caller crops whatever comes back.
        Some(profile) => profile
            .sizes
            .iter()
            .find(|size| {
                size.split_once('x')
                    .is_some_and(|(width, height)| width == height)
            })
            .cloned(),
    });
    let args = CommonImageArgs {
        size: size.flatten(),
        quality: request.quality,
        background: request.background,
        format: Some("png".to_string()),
        count: Some(1),
        ..CommonImageArgs::default()
    };
    let mut image_request = build_request(
        &request.prompt,
        &args,
        &config,
        &resolved,
        Animation::default(),
    )
    .map_err(StandaloneImageError::Setup)?;
    // Nobody watches progress here.
    image_request.partial_images = None;

    let driver = driver_over(egress, None, &resolved);
    let response = driver
        .create_images(image_request, None)
        .await
        .map_err(|error| StandaloneImageError::Provider(format!("Image API error: {}", error.0)))?;
    let image = response.images.into_iter().next().ok_or_else(|| {
        StandaloneImageError::Provider("The image API returned no image".to_string())
    })?;
    let data = base64::engine::general_purpose::STANDARD
        .decode(image.b64_json.as_bytes())
        .map_err(|_| {
            StandaloneImageError::Provider(
                "The image API returned an image that is not valid base64".to_string(),
            )
        })?;
    Ok(StandaloneImage {
        data,
        media_type: "image/png",
        provider_type: resolved.provider_type,
        model: resolved.api_model,
        revised_prompt: image.revised_prompt,
    })
}
