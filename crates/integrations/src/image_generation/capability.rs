//! The `image_generation` capability: settings, schema and localization.
//!
//! The capability is provider-neutral. Requests go through the typed
//! [`ImagesDriver`](everruns_contracts::images::ImagesDriver) contract
//! (`everruns_drivers::openai::images`), which serves OpenAI, Azure OpenAI,
//! OpenRouter and Meta. What a model can do (formats, qualities, GIF limits)
//! comes from its model profile, so new image models need a profile entry,
//! not capability code.

use super::resolve::{PLATFORM_DEFAULT_IMAGE_MODEL, is_known_image_model};
use super::tools::{EditImageTool, GenerateImageTool};
use everruns_contracts::runtime::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, IntegrationPlugin,
};
use everruns_contracts::runtime::connection_services::ProviderCredentialStore;
use everruns_contracts::runtime::tools::{Tool, ToolExecutionResult};
use everruns_contracts::tool_types::{DeferrablePolicy, ToolDefinition};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

pub(crate) const IMAGE_GENERATION_CAPABILITY_ID: &str = "image_generation";
pub(crate) const DEFAULT_QUALITY: &str = "medium";
pub(crate) const DEFAULT_FORMAT: &str = "png";
const DEFAULT_PARTIAL_IMAGES: u8 = 1;

pub(crate) const FORMATS: &[&str] = &["png", "jpeg", "webp", "gif", "svg"];
pub(crate) const QUALITIES: &[&str] = &["low", "medium", "high", "xhigh", "max", "auto"];
pub(crate) const COMPLEXITIES: &[&str] = &["low", "medium", "high"];

/// Capability plugins this crate contributes to a hosted catalog.
pub const CAPABILITY_PLUGINS: &[IntegrationPlugin] = &[IntegrationPlugin {
    feature_flag: None,
    factory: || Box::new(ImageGenerationCapability::new()),
}];

// Budget: 850 bytes (capability_tests). Every agent with the capability pays
// for this prompt on every turn.
const SYSTEM_PROMPT: &str = "When image tools are listed, call them directly for image requests; do not claim they are unavailable or stop at writing prompts unless a call fails. Skip unrelated bookkeeping before simple image calls.

Make one generation call per requested concept unless asked to batch. Use format `gif` for animations (generation only, `frames` 2-64) and `svg` for vector art (one image, saved as a session file). Keep the configured quality unless the user asks for another. Session files go under `/workspace/.outputs/images` by default. Wait for the completed result before describing a single image.";

/// Generate and edit images with any image model the organization can reach.
///
/// Hosted deployments resolve provider accounts through the host's
/// credential store. An embedding application without one hands the
/// capability its own with [`ImageGenerationCapability::with_credentials`].
#[derive(Clone, Default)]
pub struct ImageGenerationCapability {
    credentials: Option<Arc<dyn ProviderCredentialStore>>,
}

impl ImageGenerationCapability {
    /// Uses the host's provider accounts.
    pub fn new() -> Self {
        Self::default()
    }

    /// Uses `credentials` when the host provides no credential store, as in
    /// an application embedding the Framework.
    pub fn with_credentials(credentials: Arc<dyn ProviderCredentialStore>) -> Self {
        Self {
            credentials: Some(credentials),
        }
    }
}

impl Capability for ImageGenerationCapability {
    fn id(&self) -> &str {
        IMAGE_GENERATION_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Image Generation"
    }

    fn description(&self) -> &str {
        "Generate and edit images, animated GIFs and SVGs with GPT Image, Muse Image or Gemini image models."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("image")
    }

    fn category(&self) -> Option<&str> {
        Some("Media")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        self.tools_with_config(&json!({}))
    }

    fn tools_with_config(&self, config: &Value) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(
                GenerateImageTool::new(config.clone()).with_credentials(self.credentials.clone()),
            ),
            Box::new(EditImageTool::new(config.clone()).with_credentials(self.credentials.clone())),
        ]
    }

    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tools()
            .into_iter()
            .map(|tool| {
                let mut definition = tool.to_definition();
                if let ToolDefinition::Builtin(builtin) = &mut definition {
                    builtin.deferrable = DeferrablePolicy::Never;
                }
                definition
            })
            .collect()
    }

    fn config_schema(&self) -> Option<Value> {
        let options = |values: &[&str], titles: &[&str]| -> Vec<Value> {
            values
                .iter()
                .zip(titles)
                .map(|(value, title)| json!({ "const": value, "title": title }))
                .collect()
        };
        Some(json!({
            "type": "object",
            "title": "Image Generation Settings",
            "properties": {
                "model": {
                    "type": "string",
                    "title": "Image Model",
                    "description": "Image model for this agent. Leave empty to use the organization's default image model, or GPT Image 2.6 Flare when none is chosen. Accepts a model from the organization's catalog or a model name such as gpt-image-2.6-sunburst. With an Azure OpenAI provider, the name must match an image model deployment."
                },
                "default_format": {
                    "type": "string",
                    "title": "Default Format",
                    "description": "Output format when a call does not choose one. GIF and SVG need GPT Image 2.6 on OpenAI or Azure OpenAI.",
                    "default": DEFAULT_FORMAT,
                    "oneOf": options(FORMATS, &["PNG", "JPEG", "WebP", "Animated GIF", "SVG"])
                },
                "default_quality": {
                    "type": "string",
                    "title": "Default Quality",
                    "description": "Quality when a call does not choose one. Extra high and Max need GPT Image 2.5 or newer.",
                    "default": DEFAULT_QUALITY,
                    "oneOf": options(
                        &["medium", "low", "high", "xhigh", "max", "auto"],
                        &["Medium", "Low", "High", "Extra high", "Max", "Auto"]
                    )
                },
                "complexity": {
                    "type": "string",
                    "title": "Complexity",
                    "description": "How much detail GPT Image 2.5 and newer put into a scene. Lower is faster. Unset keeps the model default (high).",
                    "oneOf": options(COMPLEXITIES, &["Low", "Medium", "High"])
                },
                "gif_frames": {
                    "type": "integer",
                    "title": "GIF Frames",
                    "description": "Frames in an animated GIF when a call does not choose. Unset keeps the model default (9).",
                    "minimum": 2,
                    "maximum": 64
                },
                "gif_frame_ms": {
                    "type": "integer",
                    "title": "GIF Frame Duration (ms)",
                    "description": "How long each GIF frame shows. Unset keeps the model default (200 ms).",
                    "minimum": 20,
                    "maximum": 2000
                },
                "partial_images": {
                    "type": "integer",
                    "title": "Progress Updates",
                    "description": "Streamed in-progress updates for single still images. GIF and SVG are never streamed.",
                    "default": DEFAULT_PARTIAL_IMAGES,
                    "minimum": 0,
                    "maximum": 3,
                    "oneOf": [
                        { "const": 1, "title": "1 Progress Update" },
                        { "const": 0, "title": "Off" },
                        { "const": 2, "title": "2 Progress Updates" },
                        { "const": 3, "title": "3 Progress Updates" }
                    ]
                },
                "output_dir": {
                    "type": "string",
                    "title": "Output Folder",
                    "description": "Session folder for saved images when a call names none. Defaults to /workspace/.outputs/images."
                },
                "fallback": {
                    "type": "string",
                    "title": "Provider Fallback",
                    "description": "When \"auto\" (default) and no provider serves the chosen GPT Image model, fall back to Muse Image on a Meta or OpenRouter provider. When \"off\", fail instead.",
                    "default": "auto",
                    "oneOf": [
                        { "const": "auto", "title": "Auto" },
                        { "const": "off", "title": "Off" }
                    ]
                }
            }
        }))
    }

    fn config_ui_schema(&self) -> Option<Value> {
        Some(json!({
            "ui:submitButtonOptions": { "norender": true },
            "ui:order": [
                "model", "default_format", "default_quality", "complexity",
                "gif_frames", "gif_frame_ms", "partial_images", "output_dir", "fallback"
            ],
            "model": {
                "ui:widget": "ImageModel",
                "ui:placeholder": "Organization default"
            },
            "default_format": { "ui:widget": "select" },
            "default_quality": { "ui:widget": "select" },
            "complexity": { "ui:widget": "select", "ui:placeholder": "Model default" },
            "gif_frames": { "ui:placeholder": "9" },
            "gif_frame_ms": { "ui:placeholder": "200" },
            "partial_images": { "ui:widget": "select" },
            "output_dir": { "ui:placeholder": "/workspace/.outputs/images" },
            "fallback": { "ui:widget": "select" }
        }))
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        parse_capability_config(config)
            .map(|_| ())
            .map_err(tool_error_to_string)
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![
            CapabilityLocalization {
                locale: "en",
                name: None,
                description: None,
                config_description: Some(
                    "Choose the image model, the default format and quality, GIF timing, \
                     and how many progress updates single images stream.",
                ),
                config_overlay: None,
            },
            CapabilityLocalization {
                locale: "uk",
                name: Some("Генерація зображень"),
                description: Some(
                    "Генеруйте та редагуйте зображення, анімовані GIF і SVG моделями GPT Image, Muse Image або Gemini.",
                ),
                config_description: Some(
                    "Визначає модель зображень, типові формат і якість, параметри GIF \
                     та кількість оновлень прогресу для одного зображення.",
                ),
                config_overlay: Some(json!({
                    "properties": {
                        "model": {
                            "title": "Модель зображень",
                            "description": "Модель зображень для цього агента. Порожнє значення використовує типову модель організації або GPT Image 2.6 Flare."
                        },
                        "default_format": {
                            "title": "Типовий формат",
                            "description": "Формат, коли виклик його не вказує. GIF і SVG потребують GPT Image 2.6 через OpenAI або Azure OpenAI.",
                            "enum_labels": {
                                "png": "PNG", "jpeg": "JPEG", "webp": "WebP",
                                "gif": "Анімований GIF", "svg": "SVG"
                            }
                        },
                        "default_quality": {
                            "title": "Типова якість",
                            "description": "Якість, коли виклик її не вказує.",
                            "enum_labels": {
                                "medium": "Середня", "low": "Низька", "high": "Висока",
                                "xhigh": "Дуже висока", "max": "Максимальна", "auto": "Авто"
                            }
                        },
                        "complexity": {
                            "title": "Складність",
                            "description": "Скільки деталей GPT Image 2.5 і новіші додають у сцену. Нижча складність працює швидше.",
                            "enum_labels": { "low": "Низька", "medium": "Середня", "high": "Висока" }
                        },
                        "gif_frames": {
                            "title": "Кадри GIF",
                            "description": "Кількість кадрів анімованого GIF, коли виклик її не вказує."
                        },
                        "gif_frame_ms": {
                            "title": "Тривалість кадру GIF (мс)",
                            "description": "Скільки показується кожен кадр GIF."
                        },
                        "partial_images": {
                            "title": "Оновлення прогресу",
                            "description": "Кількість потокових проміжних оновлень для одного статичного зображення.",
                            "enum_labels": {
                                "1": "1 оновлення прогресу",
                                "0": "Вимкнено",
                                "2": "2 оновлення прогресу",
                                "3": "3 оновлення прогресу"
                            }
                        },
                        "output_dir": {
                            "title": "Папка для зображень",
                            "description": "Папка сесії для збережених зображень, коли виклик її не вказує."
                        },
                        "fallback": {
                            "title": "Резервна модель",
                            "description": "Коли \"auto\" (типово) і жоден провайдер не обслуговує вибрану модель GPT Image, використовується Muse Image через Meta або OpenRouter. Коли \"off\", виклик завершується помилкою.",
                            "enum_labels": { "auto": "Авто", "off": "Вимкнено" }
                        }
                    }
                })),
            },
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ImageModelFallback {
    /// Fall back to Muse Image on Meta or OpenRouter when no provider serves
    /// the chosen GPT Image model.
    #[default]
    Auto,
    /// Never fall back.
    Off,
}

/// Capability settings. Unknown fields are ignored so older configs load.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ImageGenerationConfig {
    /// Catalog model id (`model_...`) or model name; `None` uses the org
    /// default, then [`PLATFORM_DEFAULT_IMAGE_MODEL`].
    #[serde(default, deserialize_with = "empty_as_none")]
    pub model: Option<String>,
    #[serde(default, deserialize_with = "empty_as_none")]
    pub default_format: Option<String>,
    #[serde(default, deserialize_with = "empty_as_none")]
    pub default_quality: Option<String>,
    #[serde(default, deserialize_with = "empty_as_none")]
    pub complexity: Option<String>,
    #[serde(default)]
    pub gif_frames: Option<u8>,
    #[serde(default)]
    pub gif_frame_ms: Option<u16>,
    #[serde(default = "default_partial_images")]
    pub partial_images: u8,
    #[serde(default)]
    pub fallback: ImageModelFallback,
    /// Session directory images are saved to when a call names none.
    #[serde(default, deserialize_with = "empty_as_none")]
    pub output_dir: Option<String>,
}

impl Default for ImageGenerationConfig {
    fn default() -> Self {
        Self {
            model: None,
            default_format: None,
            default_quality: None,
            complexity: None,
            gif_frames: None,
            gif_frame_ms: None,
            partial_images: DEFAULT_PARTIAL_IMAGES,
            fallback: ImageModelFallback::Auto,
            output_dir: None,
        }
    }
}

impl ImageGenerationConfig {
    pub(crate) fn format(&self) -> &str {
        self.default_format.as_deref().unwrap_or(DEFAULT_FORMAT)
    }

    pub(crate) fn quality(&self) -> &str {
        self.default_quality.as_deref().unwrap_or(DEFAULT_QUALITY)
    }
}

fn default_partial_images() -> u8 {
    DEFAULT_PARTIAL_IMAGES
}

fn empty_as_none<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    let value = Option::<String>::deserialize(deserializer)?;
    Ok(value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty()))
}

pub(crate) fn parse_capability_config(
    config: &Value,
) -> Result<ImageGenerationConfig, ToolExecutionResult> {
    if config.is_null() {
        return Ok(ImageGenerationConfig::default());
    }
    let invalid = |message: String| {
        ToolExecutionResult::tool_error(format!("Invalid image_generation config: {message}"))
    };
    let parsed = serde_json::from_value::<ImageGenerationConfig>(config.clone())
        .map_err(|error| invalid(error.to_string()))?;

    if let Some(model) = parsed.model.as_deref()
        && !model.starts_with("model_")
        && !is_known_image_model(model)
    {
        return Err(invalid(format!(
            "unknown image model {model}; pick one from the model catalog (for example {PLATFORM_DEFAULT_IMAGE_MODEL})"
        )));
    }
    check_one_of("default_format", parsed.default_format.as_deref(), FORMATS).map_err(invalid)?;
    check_one_of(
        "default_quality",
        parsed.default_quality.as_deref(),
        QUALITIES,
    )
    .map_err(invalid)?;
    check_one_of("complexity", parsed.complexity.as_deref(), COMPLEXITIES).map_err(invalid)?;
    if parsed.partial_images > 3 {
        return Err(invalid("partial_images must be between 0 and 3".into()));
    }
    if let Some(frames) = parsed.gif_frames
        && !(2..=64).contains(&frames)
    {
        return Err(invalid("gif_frames must be between 2 and 64".into()));
    }
    if let Some(ms) = parsed.gif_frame_ms
        && !(20..=2000).contains(&ms)
    {
        return Err(invalid("gif_frame_ms must be between 20 and 2000".into()));
    }
    Ok(parsed)
}

pub(crate) fn check_one_of(
    field: &str,
    value: Option<&str>,
    allowed: &[&str],
) -> Result<(), String> {
    match value {
        Some(value) if !allowed.contains(&value) => {
            Err(format!("{field} must be one of {}", allowed.join(", ")))
        }
        _ => Ok(()),
    }
}

pub(crate) fn tool_error_to_string(error: ToolExecutionResult) -> String {
    match error {
        ToolExecutionResult::ToolError(message) => message,
        other => format!("{other:?}"),
    }
}
