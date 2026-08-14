use crate::images::{
    EditImageInput, EditImageRequest, GenerateImageRequest, ImageApiImage, ImageApiStreamEvent,
    OpenAiImageClient,
};
use async_trait::async_trait;
use base64::Engine;
use everruns_core::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, IntegrationPlugin,
};
use everruns_core::session_file::SessionFile;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::{image_services::CreateStoredImage, tool_context::ToolContext};
use everruns_provider::ToolResultImage;
use everruns_provider::tool_types::{DeferrablePolicy, ToolDefinition, ToolHints};
use everruns_provider::typed_id::ImageId;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::LazyLock;

const GPT_IMAGE_GEN_CAPABILITY_ID: &str = "gpt_image_gen";
// Default to OpenAI's ChatGPT Images 2.0 API model (`gpt-image-2`), while
// keeping the legacy GPT Image 1 path available as an explicit opt-in.
const DEFAULT_OPENAI_IMAGE_MODEL: ImageGenerationModel = ImageGenerationModel::GptImage2;
const DEFAULT_OPENAI_IMAGE_QUALITY: ImageGenerationQuality = ImageGenerationQuality::Medium;
const DEFAULT_OPENAI_IMAGE_PARTIAL_IMAGES: u8 = 1;
const DEFAULT_OUTPUT_DIR: &str = "/workspace/.outputs/images";
const DEFAULT_GENERATE_PREFIX: &str = "generated-image";
const DEFAULT_EDIT_PREFIX: &str = "edited-image";
const SESSION_API_KEY_SECRET_NAMES: &[&str] = &["OPENAI_API_KEY", "openai_api_key"];
const SESSION_BASE_URL_SECRET_NAMES: &[&str] = &["OPENAI_BASE_URL", "openai_base_url"];
const MAX_EDIT_SOURCE_BYTES: usize = 50 * 1024 * 1024;

inventory::submit! {
    IntegrationPlugin {
        experimental_only: false,
        feature_flag: None,
        factory: || Box::new(GptImageGenCapability),
    }
}

static SYSTEM_PROMPT: LazyLock<String> = LazyLock::new(|| {
    r#"When image tools are listed, call them directly for generation/editing requests; do not claim they are unavailable or stop at writing prompts unless a tool call fails. Avoid unrelated bookkeeping before straightforward image calls.

For multiple new images, make one generation call per requested concept unless the user asks for batching. Save session files under `/workspace/.outputs/images` by default, rely on the configured quality unless the user requests another, and wait for the completed result before describing a single-image output.

Per-session OpenAI overrides belong in `secret_store` as `OPENAI_API_KEY` and optionally `OPENAI_BASE_URL`."#
        .to_string()
});

pub struct GptImageGenCapability;

impl Capability for GptImageGenCapability {
    fn id(&self) -> &str {
        GPT_IMAGE_GEN_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "OpenAI Image Generation"
    }

    fn description(&self) -> &str {
        "Generate and edit raster images with OpenAI's GPT Image API."
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
        Some(&SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        self.tools_with_config(&json!({}))
    }

    fn tools_with_config(&self, config: &Value) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(GenerateImageTool {
                config: config.clone(),
            }),
            Box::new(EditImageTool {
                config: config.clone(),
            }),
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

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_file_system", "session_storage"]
    }

    fn config_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "title": "Image Generation Settings",
            "properties": {
                "model": {
                    "type": "string",
                    "title": "Image Model",
                    "description": "Default model used by image generation and edit tools. With an Azure OpenAI provider, this must match the name of an image model deployment on the Azure resource.",
                    "default": DEFAULT_OPENAI_IMAGE_MODEL.as_str(),
                    "oneOf": [
                        {
                            "const": "gpt-image-2",
                            "title": "ChatGPT Images 2.0"
                        },
                        {
                            "const": "gpt-image-1",
                            "title": "GPT Image 1"
                        }
                    ]
                },
                "default_quality": {
                    "type": "string",
                    "title": "Default Quality",
                    "description": "Default quality used when a tool call does not specify quality.",
                    "default": DEFAULT_OPENAI_IMAGE_QUALITY.as_str(),
                    "oneOf": [
                        { "const": "medium", "title": "Medium" },
                        { "const": "low", "title": "Low" },
                        { "const": "high", "title": "High" },
                        { "const": "auto", "title": "Auto" }
                    ]
                },
                "partial_images": {
                    "type": "integer",
                    "title": "Progress Updates",
                    "description": "Number of streamed in-progress image updates for single-image requests.",
                    "default": DEFAULT_OPENAI_IMAGE_PARTIAL_IMAGES,
                    "minimum": 0,
                    "maximum": 3,
                    "oneOf": [
                        { "const": 1, "title": "1 Progress Update" },
                        { "const": 0, "title": "Off" },
                        { "const": 2, "title": "2 Progress Updates" },
                        { "const": 3, "title": "3 Progress Updates" }
                    ]
                }
            }
        }))
    }

    fn config_ui_schema(&self) -> Option<Value> {
        Some(json!({
            "ui:submitButtonOptions": { "norender": true },
            "ui:order": ["model", "default_quality", "partial_images"],
            "model": {
                "ui:placeholder": DEFAULT_OPENAI_IMAGE_MODEL.as_str()
            },
            "default_quality": {
                "ui:placeholder": DEFAULT_OPENAI_IMAGE_QUALITY.as_str()
            },
            "partial_images": {
                "ui:widget": "select"
            }
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
                    "Choose the default image model, the default quality, and how many \
                     streamed progress updates single-image requests emit.",
                ),
                config_overlay: None,
            },
            CapabilityLocalization {
                locale: "uk",
                name: Some("Генерація зображень OpenAI"),
                description: Some(
                    "Генеруйте та редагуйте растрові зображення через GPT Image API від OpenAI.",
                ),
                config_description: Some(
                    "Визначає типову модель зображень, типову якість і кількість потокових \
                     оновлень прогресу для запитів з одним зображенням.",
                ),
                config_overlay: Some(json!({
                    "properties": {
                        "model": {
                            "title": "Модель зображень",
                            "description": "Типова модель для інструментів генерації та редагування зображень. Для провайдера Azure OpenAI назва має збігатися з іменем розгортання моделі зображень на ресурсі Azure.",
                            "enum_labels": {
                                "gpt-image-2": "ChatGPT Images 2.0",
                                "gpt-image-1": "GPT Image 1"
                            }
                        },
                        "default_quality": {
                            "title": "Типова якість",
                            "description": "Якість за замовчуванням, коли виклик інструмента не вказує якість.",
                            "enum_labels": {
                                "medium": "Середня",
                                "low": "Низька",
                                "high": "Висока",
                                "auto": "Авто"
                            }
                        },
                        "partial_images": {
                            "title": "Оновлення прогресу",
                            "description": "Кількість потокових проміжних оновлень зображення для запитів з одним зображенням.",
                            "enum_labels": {
                                "1": "1 оновлення прогресу",
                                "0": "Вимкнено",
                                "2": "2 оновлення прогресу",
                                "3": "3 оновлення прогресу"
                            }
                        }
                    }
                })),
            },
        ]
    }
}

pub struct GenerateImageTool {
    config: Value,
}

#[async_trait]
impl Tool for GenerateImageTool {
    fn name(&self) -> &str {
        "generate_image"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Generate Image")
    }

    fn description(&self) -> &str {
        "Server-side image generation tool. Call this directly for user image requests; it generates the image in this session and can persist results as artifacts or session files."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": { "type": "string", "description": "Image generation prompt." },
                "size": {
                    "type": "string",
                    "enum": ["1024x1024", "1536x1024", "1024x1536", "auto"],
                    "description": "Output size. Defaults to auto."
                },
                "quality": {
                    "type": "string",
                    "enum": ["low", "medium", "high", "auto"],
                    "description": "Generation quality. Defaults to the capability default quality (medium unless overridden)."
                },
                "background": {
                    "type": "string",
                    "enum": ["transparent", "opaque", "auto"],
                    "description": "Background treatment. Transparent requires png or webp."
                },
                "format": {
                    "type": "string",
                    "enum": ["png", "jpeg", "webp"],
                    "description": "Output image format. Defaults to png."
                },
                "count": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 10,
                    "description": "Number of images to generate. Defaults to 1."
                },
                "save_to_session_fs": {
                    "type": "boolean",
                    "description": "When true, save each generated image into the session filesystem."
                },
                "output_dir": {
                    "type": "string",
                    "description": "Session filesystem directory used when save_to_session_fs is true. Defaults to /workspace/.outputs/images."
                },
                "filename_prefix": {
                    "type": "string",
                    "description": "Filename prefix used for saved/generated artifacts."
                },
                "persist_artifact": {
                    "type": "boolean",
                    "description": "When true, persist each image in the durable image artifact store. Defaults to true."
                }
            },
            "required": ["prompt"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("generate_image requires session context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let args = match parse_arguments::<GenerateImageArgs>(arguments) {
            Ok(args) => args,
            Err(result) => return result,
        };
        if let Err(message) = validate_output_options(&args.common) {
            return ToolExecutionResult::tool_error(message);
        }
        let capability_config = match parse_capability_config(&self.config) {
            Ok(config) => config,
            Err(result) => return result,
        };
        let model = capability_config.model.as_str();

        let client = match build_client(context).await {
            Ok(client) => client,
            Err(result) => return result,
        };
        let partial_images = resolve_partial_images(args.common.count(), &capability_config);
        let request = GenerateImageRequest {
            model: model.to_string(),
            prompt: args.prompt.clone(),
            size: args.common.size.clone(),
            quality: Some(resolve_quality(
                args.common.quality.as_deref(),
                capability_config.default_quality,
            )),
            background: args.common.background.clone(),
            output_format: args.common.format.clone(),
            stream: partial_images.map(|_| true),
            partial_images,
            count: args.common.count(),
        };

        context
            .emit_progress(
                self.name(),
                &initial_progress_message("Generating", args.common.count(), partial_images),
            )
            .await;

        let response = if let Some(expected_previews) = partial_images {
            match client
                .generate_with_events(request, |event| async {
                    emit_stream_progress(
                        context,
                        self.name(),
                        "Generating",
                        expected_previews,
                        event,
                    )
                    .await;
                })
                .await
            {
                Ok(response) => response,
                Err(error) => return ToolExecutionResult::internal_error_msg(format!("{error:#}")),
            }
        } else {
            match client.generate(request).await {
                Ok(response) => response,
                Err(error) => return ToolExecutionResult::internal_error_msg(format!("{error:#}")),
            }
        };

        materialize_outputs(
            context,
            &args.prompt,
            None,
            &args.common,
            model,
            response.data,
            args.common
                .filename_prefix
                .clone()
                .unwrap_or_else(|| DEFAULT_GENERATE_PREFIX.to_string()),
        )
        .await
    }
}

pub struct EditImageTool {
    config: Value,
}

#[async_trait]
impl Tool for EditImageTool {
    fn name(&self) -> &str {
        "edit_image"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Edit Image")
    }

    fn description(&self) -> &str {
        "Server-side image editing tool. Call this directly when the user wants an existing image revised using a stored artifact and/or a session file path."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": { "type": "string", "description": "Editing prompt." },
                "image_id": {
                    "type": "string",
                    "description": "Optional stored image artifact ID to use as an edit source."
                },
                "path": {
                    "type": "string",
                    "description": "Optional session file path to use as an edit source."
                },
                "size": {
                    "type": "string",
                    "enum": ["1024x1024", "1536x1024", "1024x1536", "auto"],
                    "description": "Output size. Defaults to auto."
                },
                "quality": {
                    "type": "string",
                    "enum": ["low", "medium", "high", "auto"],
                    "description": "Generation quality. Defaults to the capability default quality (medium unless overridden)."
                },
                "background": {
                    "type": "string",
                    "enum": ["transparent", "opaque", "auto"],
                    "description": "Background treatment. Transparent requires png or webp."
                },
                "format": {
                    "type": "string",
                    "enum": ["png", "jpeg", "webp"],
                    "description": "Output image format. Defaults to png."
                },
                "count": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 10,
                    "description": "Number of images to generate. Defaults to 1."
                },
                "save_to_session_fs": { "type": "boolean" },
                "output_dir": { "type": "string" },
                "filename_prefix": { "type": "string" },
                "persist_artifact": { "type": "boolean" }
            },
            "required": ["prompt"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("edit_image requires session context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let args = match parse_arguments::<EditImageArgs>(arguments) {
            Ok(args) => args,
            Err(result) => return result,
        };
        if args.image_id.is_none() && args.path.is_none() {
            return ToolExecutionResult::tool_error(
                "edit_image requires at least one source: image_id and/or path",
            );
        }
        if let Err(message) = validate_output_options(&args.common) {
            return ToolExecutionResult::tool_error(message);
        }
        let capability_config = match parse_capability_config(&self.config) {
            Ok(config) => config,
            Err(result) => return result,
        };
        let model = capability_config.model.as_str();

        let sources = match collect_edit_sources(context, &args).await {
            Ok(sources) => sources,
            Err(result) => return result,
        };
        let client = match build_client(context).await {
            Ok(client) => client,
            Err(result) => return result,
        };
        let partial_images = resolve_partial_images(args.common.count(), &capability_config);
        let request = EditImageRequest {
            model: model.to_string(),
            prompt: args.prompt.clone(),
            images: sources,
            size: args.common.size.clone(),
            quality: Some(resolve_quality(
                args.common.quality.as_deref(),
                capability_config.default_quality,
            )),
            background: args.common.background.clone(),
            output_format: args.common.format.clone(),
            stream: partial_images.map(|_| true),
            partial_images,
            count: args.common.count(),
        };

        context
            .emit_progress(
                self.name(),
                &initial_progress_message("Editing", args.common.count(), partial_images),
            )
            .await;

        let response = if let Some(expected_previews) = partial_images {
            match client
                .edit_with_events(request, |event| async {
                    emit_stream_progress(context, self.name(), "Editing", expected_previews, event)
                        .await;
                })
                .await
            {
                Ok(response) => response,
                Err(error) => return ToolExecutionResult::internal_error_msg(format!("{error:#}")),
            }
        } else {
            match client.edit(request).await {
                Ok(response) => response,
                Err(error) => return ToolExecutionResult::internal_error_msg(format!("{error:#}")),
            }
        };

        materialize_outputs(
            context,
            &args.prompt,
            Some(json!({
                "image_id": args.image_id.map(|id| id.to_string()),
                "path": args.path,
            })),
            &args.common,
            model,
            response.data,
            args.common
                .filename_prefix
                .clone()
                .unwrap_or_else(|| DEFAULT_EDIT_PREFIX.to_string()),
        )
        .await
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct GenerateImageArgs {
    prompt: String,
    #[serde(flatten)]
    common: CommonImageArgs,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct EditImageArgs {
    prompt: String,
    image_id: Option<ImageId>,
    path: Option<String>,
    #[serde(flatten)]
    common: CommonImageArgs,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommonImageArgs {
    size: Option<String>,
    quality: Option<String>,
    background: Option<String>,
    format: Option<String>,
    count: Option<usize>,
    save_to_session_fs: Option<bool>,
    output_dir: Option<String>,
    filename_prefix: Option<String>,
    persist_artifact: Option<bool>,
}

impl CommonImageArgs {
    fn count(&self) -> usize {
        self.count.unwrap_or(1)
    }

    fn save_to_session_fs(&self) -> bool {
        self.save_to_session_fs.unwrap_or(false)
    }

    fn persist_artifact(&self) -> bool {
        self.persist_artifact.unwrap_or(true)
    }

    fn output_dir(&self) -> &str {
        self.output_dir.as_deref().unwrap_or(DEFAULT_OUTPUT_DIR)
    }

    fn format(&self) -> &str {
        self.format.as_deref().unwrap_or("png")
    }
}

impl CommonImageArgs {
    fn cloned_with_defaults(&self) -> CommonImageArgsWithDefaults {
        CommonImageArgsWithDefaults {
            format: Some(self.format().to_string()),
            count: self.count(),
            save_to_session_fs: self.save_to_session_fs(),
            output_dir: self.output_dir().to_string(),
            persist_artifact: self.persist_artifact(),
        }
    }
}

#[derive(Debug, Clone)]
struct CommonImageArgsWithDefaults {
    format: Option<String>,
    count: usize,
    save_to_session_fs: bool,
    output_dir: String,
    persist_artifact: bool,
}

#[derive(Debug, Clone)]
struct ResolvedClientConfig {
    api_key: String,
    base_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
enum ImageGenerationModel {
    #[serde(rename = "gpt-image-2")]
    GptImage2,
    #[serde(rename = "gpt-image-1")]
    GptImage1,
}

impl ImageGenerationModel {
    fn as_str(self) -> &'static str {
        match self {
            Self::GptImage2 => "gpt-image-2",
            Self::GptImage1 => "gpt-image-1",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
enum ImageGenerationQuality {
    #[serde(rename = "low")]
    Low,
    #[serde(rename = "medium")]
    Medium,
    #[serde(rename = "high")]
    High,
    #[serde(rename = "auto")]
    Auto,
}

impl ImageGenerationQuality {
    fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Auto => "auto",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct GptImageGenCapabilityConfig {
    #[serde(default = "default_openai_image_model")]
    model: ImageGenerationModel,
    #[serde(default = "default_openai_image_quality")]
    default_quality: ImageGenerationQuality,
    #[serde(default = "default_openai_image_partial_images")]
    partial_images: u8,
}

impl Default for GptImageGenCapabilityConfig {
    fn default() -> Self {
        Self {
            model: default_openai_image_model(),
            default_quality: default_openai_image_quality(),
            partial_images: default_openai_image_partial_images(),
        }
    }
}

fn default_openai_image_model() -> ImageGenerationModel {
    DEFAULT_OPENAI_IMAGE_MODEL
}

fn default_openai_image_quality() -> ImageGenerationQuality {
    DEFAULT_OPENAI_IMAGE_QUALITY
}

fn default_openai_image_partial_images() -> u8 {
    DEFAULT_OPENAI_IMAGE_PARTIAL_IMAGES
}

fn resolve_quality(
    explicit_quality: Option<&str>,
    default_quality: ImageGenerationQuality,
) -> String {
    explicit_quality
        .unwrap_or(default_quality.as_str())
        .to_string()
}

fn parse_arguments<T: for<'de> Deserialize<'de>>(
    arguments: Value,
) -> Result<T, ToolExecutionResult> {
    serde_json::from_value(arguments)
        .map_err(|error| ToolExecutionResult::tool_error(format!("Invalid arguments: {error}")))
}

fn parse_capability_config(
    config: &Value,
) -> Result<GptImageGenCapabilityConfig, ToolExecutionResult> {
    if config.is_null() {
        return Ok(GptImageGenCapabilityConfig::default());
    }

    let parsed =
        serde_json::from_value::<GptImageGenCapabilityConfig>(config.clone()).map_err(|error| {
            ToolExecutionResult::tool_error(format!("Invalid gpt_image_gen config: {error}"))
        })?;

    if parsed.partial_images > 3 {
        return Err(ToolExecutionResult::tool_error(
            "Invalid gpt_image_gen config: partial_images must be between 0 and 3",
        ));
    }

    Ok(parsed)
}

fn tool_error_to_string(error: ToolExecutionResult) -> String {
    match error {
        ToolExecutionResult::ToolError(message) => message,
        other => format!("{other:?}"),
    }
}

fn resolve_partial_images(count: usize, config: &GptImageGenCapabilityConfig) -> Option<u8> {
    if count == 1 && config.partial_images > 0 {
        Some(config.partial_images)
    } else {
        None
    }
}

fn initial_progress_message(action: &str, count: usize, partial_images: Option<u8>) -> String {
    if count > 1 {
        format!("{action} {count} images...")
    } else if partial_images.unwrap_or(0) > 0 {
        format!("{action} image and waiting for progress updates...")
    } else {
        format!("{action} image...")
    }
}

async fn emit_stream_progress(
    context: &ToolContext,
    tool_name: &str,
    action: &str,
    expected_previews: u8,
    event: ImageApiStreamEvent,
) {
    let message = match event {
        ImageApiStreamEvent::PartialImage {
            partial_image_index,
            ..
        } => format!(
            "{action} progress update {} of {}...",
            partial_image_index + 1,
            expected_previews
        ),
        ImageApiStreamEvent::Completed { .. } => format!("{action} final image..."),
    };
    context.emit_progress(tool_name, &message).await;
}

async fn build_client(context: &ToolContext) -> Result<OpenAiImageClient, ToolExecutionResult> {
    let config = match resolve_client_config(context).await {
        Ok(config) => config,
        Err(result) => return Err(result),
    };

    OpenAiImageClient::new(config.api_key, config.base_url)
        .map_err(|error| ToolExecutionResult::internal_error_msg(format!("{error:#}")))
}

async fn resolve_client_config(
    context: &ToolContext,
) -> Result<ResolvedClientConfig, ToolExecutionResult> {
    if let Some(storage_store) = &context.storage_store {
        let api_key = match get_first_secret(
            storage_store.as_ref(),
            context,
            SESSION_API_KEY_SECRET_NAMES,
        )
        .await
        {
            Ok(api_key) => api_key,
            Err(error) => {
                return Err(ToolExecutionResult::internal_error_msg(format!(
                    "{error:#}"
                )));
            }
        };
        let base_url = match get_first_secret(
            storage_store.as_ref(),
            context,
            SESSION_BASE_URL_SECRET_NAMES,
        )
        .await
        {
            Ok(base_url) => base_url,
            Err(error) => {
                return Err(ToolExecutionResult::internal_error_msg(format!(
                    "{error:#}"
                )));
            }
        };
        if let Some(api_key) = api_key {
            return Ok(ResolvedClientConfig { api_key, base_url });
        }
    }

    let Some(provider_store) = &context.provider_credential_store else {
        return Err(ToolExecutionResult::tool_error(
            "OpenAI credentials are not configured. Store OPENAI_API_KEY via secret_store or configure an OpenAI or Azure OpenAI provider.",
        ));
    };

    // Prefer a native OpenAI provider; fall back to Azure OpenAI, whose base
    // URL is validated at provider creation to the OpenAI-compatible
    // `/openai/v1` surface that serves the Images API (the client applies
    // Azure's `api-key` auth header based on the URL).
    for provider_type in ["openai", "azure_openai"] {
        match provider_store
            .get_default_provider_credentials(provider_type)
            .await
        {
            Ok(Some(credentials)) => {
                return Ok(ResolvedClientConfig {
                    api_key: credentials.api_key,
                    base_url: credentials.base_url,
                });
            }
            Ok(None) => {}
            Err(error) => return Err(ToolExecutionResult::internal_error(error)),
        }
    }

    Err(ToolExecutionResult::tool_error(
        "OpenAI credentials are not configured. Store OPENAI_API_KEY via secret_store or configure an OpenAI or Azure OpenAI provider.",
    ))
}

async fn get_first_secret(
    store: &dyn everruns_core::session_services::SessionStorageStore,
    context: &ToolContext,
    names: &[&str],
) -> anyhow::Result<Option<String>> {
    for name in names {
        if let Some(value) = store.get_secret(context.session_id, name).await? {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

fn validate_output_options(common: &CommonImageArgs) -> Result<(), String> {
    if !(1..=10).contains(&common.count()) {
        return Err("count must be between 1 and 10".to_string());
    }
    if matches!(common.background.as_deref(), Some("transparent"))
        && matches!(common.format(), "jpeg")
    {
        return Err("transparent background requires png or webp output".to_string());
    }
    Ok(())
}

async fn collect_edit_sources(
    context: &ToolContext,
    args: &EditImageArgs,
) -> Result<Vec<EditImageInput>, ToolExecutionResult> {
    let mut sources = Vec::new();

    if let Some(image_id) = args.image_id {
        let Some(image_store) = &context.image_store else {
            return Err(ToolExecutionResult::internal_error_msg(
                "Image artifact store not available in this context",
            ));
        };
        let image = match image_store.get_image(image_id).await {
            Ok(Some(image)) => image,
            Ok(None) => {
                return Err(ToolExecutionResult::tool_error(format!(
                    "Image artifact not found: {image_id}"
                )));
            }
            Err(error) => return Err(ToolExecutionResult::internal_error(error)),
        };
        if image.data.len() > MAX_EDIT_SOURCE_BYTES {
            return Err(ToolExecutionResult::tool_error(format!(
                "Image artifact {image_id} exceeds the 50MB edit limit"
            )));
        }
        sources.push(EditImageInput {
            filename: image.info.filename,
            content_type: image.info.content_type,
            data: image.data,
        });
    }

    if let Some(path) = args.path.as_deref() {
        let Some(file_store) = &context.file_store else {
            return Err(ToolExecutionResult::internal_error_msg(
                "Session file store not available in this context",
            ));
        };
        let normalized_path = normalize_workspace_path(path);
        let display_path = add_workspace_prefix(&normalized_path);
        let file = match file_store
            .read_file(context.session_id, &normalized_path)
            .await
        {
            Ok(Some(file)) => file,
            Ok(None) => {
                return Err(ToolExecutionResult::tool_error(format!(
                    "Session file not found: {display_path}"
                )));
            }
            Err(error) => return Err(ToolExecutionResult::internal_error(error)),
        };
        if file.is_directory {
            return Err(ToolExecutionResult::tool_error(format!(
                "Path is a directory, not an image file: {display_path}"
            )));
        }
        let Some(content) = file.content.as_deref() else {
            return Err(ToolExecutionResult::tool_error(format!(
                "Session file has no content: {display_path}"
            )));
        };
        let bytes = match SessionFile::decode_content(content, &file.encoding) {
            Ok(bytes) => bytes,
            Err(error) => {
                return Err(ToolExecutionResult::tool_error(format!(
                    "Failed to decode session file content for {display_path}: {error}"
                )));
            }
        };
        if bytes.len() > MAX_EDIT_SOURCE_BYTES {
            return Err(ToolExecutionResult::tool_error(format!(
                "Session file exceeds the 50MB edit limit: {display_path}"
            )));
        }
        let Some(content_type) = infer_image_content_type(&display_path) else {
            return Err(ToolExecutionResult::tool_error(format!(
                "Unsupported image file type for {display_path}. Use png, jpg, jpeg, or webp."
            )));
        };
        sources.push(EditImageInput {
            filename: file.name,
            content_type: content_type.to_string(),
            data: bytes,
        });
    }

    Ok(sources)
}

async fn materialize_outputs(
    context: &ToolContext,
    prompt: &str,
    source: Option<Value>,
    common: &CommonImageArgs,
    model: &str,
    images: Vec<ImageApiImage>,
    filename_prefix: String,
) -> ToolExecutionResult {
    let options = common.cloned_with_defaults();
    let Some(output_format) = options.format.clone() else {
        return ToolExecutionResult::internal_error_msg("missing output format");
    };
    let mut rendered_images = Vec::with_capacity(images.len());
    let mut rendered_results = Vec::with_capacity(images.len());

    for (index, image) in images.into_iter().enumerate() {
        let bytes = match base64::engine::general_purpose::STANDARD.decode(&image.b64_json) {
            Ok(bytes) => bytes,
            Err(error) => {
                return ToolExecutionResult::internal_error_msg(format!(
                    "invalid base64 image data from OpenAI: {error}"
                ));
            }
        };
        let filename = output_filename(&filename_prefix, index, options.count, &output_format);
        let media_type = format_media_type(&output_format);

        let artifact_id = if options.persist_artifact {
            let Some(image_store) = &context.image_store else {
                return ToolExecutionResult::internal_error_msg(
                    "Image artifact store not available in this context",
                );
            };
            match image_store
                .create_image(CreateStoredImage {
                    filename: filename.clone(),
                    content_type: media_type.to_string(),
                    data: bytes.clone(),
                    metadata: json!({
                        "provider": "openai",
                        "model": model,
                        "prompt": prompt,
                        "revised_prompt": image.revised_prompt,
                    }),
                })
                .await
            {
                Ok(info) => Some(info.id),
                Err(error) => return ToolExecutionResult::internal_error(error),
            }
        } else {
            None
        };

        let session_path = if options.save_to_session_fs {
            let Some(file_store) = &context.file_store else {
                return ToolExecutionResult::internal_error_msg(
                    "Session file store not available in this context",
                );
            };
            let normalized_output_dir = normalize_workspace_path(&options.output_dir);
            let normalized_path = join_workspace_path(&normalized_output_dir, &filename);
            match file_store
                .write_file(
                    context.session_id,
                    &normalized_path,
                    &image.b64_json,
                    "base64",
                )
                .await
            {
                Ok(_) => Some(add_workspace_prefix(&normalized_path)),
                Err(error) => return ToolExecutionResult::internal_error(error),
            }
        } else {
            None
        };

        rendered_images.push(ToolResultImage {
            base64: image.b64_json.clone(),
            media_type: media_type.to_string(),
        });
        rendered_results.push(json!({
            "index": index + 1,
            "artifact_id": artifact_id.map(|id| id.to_string()),
            "session_file": session_path,
            "filename": filename,
            "media_type": media_type,
            "size_bytes": bytes.len(),
            "revised_prompt": image.revised_prompt,
        }));
    }

    ToolExecutionResult::success_with_images(
        json!({
            "provider": "openai",
            "model": model,
            "prompt": prompt,
            "count": rendered_results.len(),
            "source": source,
            "images": rendered_results,
        }),
        rendered_images,
    )
}

fn output_filename(prefix: &str, index: usize, count: usize, format: &str) -> String {
    let extension = match format {
        "jpeg" => "jpg",
        other => other,
    };
    if count == 1 {
        format!("{prefix}.{extension}")
    } else {
        format!("{prefix}-{}.{extension}", index + 1)
    }
}

fn format_media_type(format: &str) -> &'static str {
    match format {
        "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => "image/png",
    }
}

fn normalize_workspace_path(path: &str) -> String {
    let path = if path == "/workspace" {
        "/".to_string()
    } else if let Some(stripped) = path.strip_prefix("/workspace") {
        if stripped.starts_with('/') {
            stripped.to_string()
        } else {
            path.to_string()
        }
    } else if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    };

    if path.is_empty() {
        "/".to_string()
    } else {
        path
    }
}

fn add_workspace_prefix(path: &str) -> String {
    if path == "/" {
        "/workspace".to_string()
    } else {
        format!("/workspace{path}")
    }
}

fn join_workspace_path(dir: &str, filename: &str) -> String {
    if dir == "/" {
        format!("/{filename}")
    } else {
        format!("{}/{}", dir.trim_end_matches('/'), filename)
    }
}

fn infer_image_content_type(path: &str) -> Option<&'static str> {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".png") {
        Some("image/png")
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        Some("image/jpeg")
    } else if lower.ends_with(".webp") {
        Some("image/webp")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use everruns_core::connection_services::ProviderCredentials;
    use everruns_core::{
        connection_services::ProviderCredentialStore, session_services::KeyInfo,
        session_services::SecretInfo, session_services::SessionStorageStore,
    };
    use everruns_provider::error::Result;
    use everruns_provider::typed_id::SessionId;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    struct MockSessionStorageStore {
        secrets: Mutex<HashMap<String, String>>,
    }

    impl MockSessionStorageStore {
        fn with_secrets(secrets: &[(&str, &str)]) -> Self {
            let mut map = HashMap::new();
            for (key, value) in secrets {
                map.insert((*key).to_string(), (*value).to_string());
            }
            Self {
                secrets: Mutex::new(map),
            }
        }
    }

    #[async_trait]
    impl SessionStorageStore for MockSessionStorageStore {
        async fn set_value(&self, _session_id: SessionId, _key: &str, _value: &str) -> Result<()> {
            unreachable!("unused in tests")
        }

        async fn get_value(&self, _session_id: SessionId, _key: &str) -> Result<Option<String>> {
            unreachable!("unused in tests")
        }

        async fn delete_value(&self, _session_id: SessionId, _key: &str) -> Result<bool> {
            unreachable!("unused in tests")
        }

        async fn list_keys(&self, _session_id: SessionId) -> Result<Vec<KeyInfo>> {
            unreachable!("unused in tests")
        }

        async fn set_secret(
            &self,
            _session_id: SessionId,
            _name: &str,
            _value: &str,
        ) -> Result<()> {
            unreachable!("unused in tests")
        }

        async fn get_secret(&self, _session_id: SessionId, name: &str) -> Result<Option<String>> {
            Ok(self.secrets.lock().expect("poisoned").get(name).cloned())
        }

        async fn delete_secret(&self, _session_id: SessionId, _name: &str) -> Result<bool> {
            unreachable!("unused in tests")
        }

        async fn list_secrets(&self, _session_id: SessionId) -> Result<Vec<SecretInfo>> {
            unreachable!("unused in tests")
        }
    }

    struct MockProviderCredentialStore {
        providers: Vec<(&'static str, ProviderCredentials)>,
    }

    impl MockProviderCredentialStore {
        fn openai(credentials: ProviderCredentials) -> Self {
            Self {
                providers: vec![("openai", credentials)],
            }
        }
    }

    #[async_trait]
    impl ProviderCredentialStore for MockProviderCredentialStore {
        async fn get_default_provider_credentials(
            &self,
            provider_type: &str,
        ) -> Result<Option<ProviderCredentials>> {
            Ok(self
                .providers
                .iter()
                .find(|(name, _)| *name == provider_type)
                .map(|(_, credentials)| credentials.clone()))
        }
    }

    #[test]
    fn normalize_workspace_paths() {
        assert_eq!(normalize_workspace_path("/workspace/out"), "/out");
        assert_eq!(normalize_workspace_path("out/file.png"), "/out/file.png");
    }

    #[test]
    fn output_filename_indexes_multiple_images() {
        assert_eq!(output_filename("image", 0, 1, "png"), "image.png");
        assert_eq!(output_filename("image", 1, 3, "jpeg"), "image-2.jpg");
    }

    #[test]
    fn parse_image_id_strings() {
        let value = json!({
            "prompt": "edit",
            "image_id": ImageId::new().to_string()
        });
        let args: EditImageArgs = serde_json::from_value(value).unwrap();
        assert!(args.image_id.is_some());
    }

    #[test]
    fn capability_declares_session_file_system_dependency() {
        let capability = GptImageGenCapability;
        assert_eq!(
            capability.dependencies(),
            vec!["session_file_system", "session_storage"]
        );
    }

    #[tokio::test]
    async fn uses_session_base_url_when_session_api_key_present() {
        let session_id = SessionId::new();
        let storage = Arc::new(MockSessionStorageStore::with_secrets(&[
            ("OPENAI_API_KEY", "session-key"),
            ("OPENAI_BASE_URL", "https://session.example/v1"),
        ]));
        let provider = Arc::new(MockProviderCredentialStore::openai(ProviderCredentials {
            api_key: "provider-key".to_string(),
            base_url: Some("https://provider.example/v1".to_string()),
        }));
        let context = ToolContext {
            session_id,
            storage_store: Some(storage),
            provider_credential_store: Some(provider),
            ..ToolContext::new(session_id)
        };

        let resolved = resolve_client_config(&context)
            .await
            .expect("resolve config");

        assert_eq!(resolved.api_key, "session-key");
        assert_eq!(
            resolved.base_url,
            Some("https://session.example/v1".to_string())
        );
    }

    #[tokio::test]
    async fn ignores_session_base_url_when_falling_back_to_provider_credentials() {
        let session_id = SessionId::new();
        let storage = Arc::new(MockSessionStorageStore::with_secrets(&[(
            "OPENAI_BASE_URL",
            "https://attacker.example/v1",
        )]));
        let provider = Arc::new(MockProviderCredentialStore::openai(ProviderCredentials {
            api_key: "provider-key".to_string(),
            base_url: Some("https://provider.example/v1".to_string()),
        }));
        let context = ToolContext {
            session_id,
            storage_store: Some(storage),
            provider_credential_store: Some(provider),
            ..ToolContext::new(session_id)
        };

        let resolved = resolve_client_config(&context)
            .await
            .expect("resolve config");

        assert_eq!(resolved.api_key, "provider-key");
        assert_eq!(
            resolved.base_url,
            Some("https://provider.example/v1".to_string())
        );
    }

    // With no OpenAI provider configured, credentials must fall back to the
    // org's default Azure OpenAI provider: its base URL is validated to the
    // OpenAI-compatible `/openai/v1` surface, which serves the Images API.
    #[tokio::test]
    async fn falls_back_to_azure_openai_provider_credentials() {
        let session_id = SessionId::new();
        let provider = Arc::new(MockProviderCredentialStore {
            providers: vec![(
                "azure_openai",
                ProviderCredentials {
                    api_key: "azure-key".to_string(),
                    base_url: Some("https://res.openai.azure.com/openai/v1".to_string()),
                },
            )],
        });
        let context = ToolContext {
            session_id,
            storage_store: None,
            provider_credential_store: Some(provider),
            ..ToolContext::new(session_id)
        };

        let resolved = resolve_client_config(&context)
            .await
            .expect("resolve config");

        assert_eq!(resolved.api_key, "azure-key");
        assert_eq!(
            resolved.base_url,
            Some("https://res.openai.azure.com/openai/v1".to_string())
        );
    }

    #[tokio::test]
    async fn openai_provider_wins_over_azure_openai_provider() {
        let session_id = SessionId::new();
        let provider = Arc::new(MockProviderCredentialStore {
            providers: vec![
                (
                    "openai",
                    ProviderCredentials {
                        api_key: "openai-key".to_string(),
                        base_url: None,
                    },
                ),
                (
                    "azure_openai",
                    ProviderCredentials {
                        api_key: "azure-key".to_string(),
                        base_url: Some("https://res.openai.azure.com/openai/v1".to_string()),
                    },
                ),
            ],
        });
        let context = ToolContext {
            session_id,
            storage_store: None,
            provider_credential_store: Some(provider),
            ..ToolContext::new(session_id)
        };

        let resolved = resolve_client_config(&context)
            .await
            .expect("resolve config");

        assert_eq!(resolved.api_key, "openai-key");
    }

    #[test]
    fn capability_config_defaults_to_gpt_image_2() {
        let config = parse_capability_config(&json!({})).unwrap();
        assert_eq!(config.model, ImageGenerationModel::GptImage2);
        assert_eq!(config.default_quality, ImageGenerationQuality::Medium);
        assert_eq!(config.partial_images, 1);
    }

    #[test]
    fn capability_config_accepts_legacy_model_override() {
        let config = parse_capability_config(&json!({
            "model": "gpt-image-1",
            "default_quality": "low",
            "partial_images": 2
        }))
        .unwrap();
        assert_eq!(config.model, ImageGenerationModel::GptImage1);
        assert_eq!(config.default_quality, ImageGenerationQuality::Low);
        assert_eq!(config.partial_images, 2);
    }

    #[test]
    fn capability_config_ignores_unknown_fields() {
        let config = parse_capability_config(&json!({
            "model": "gpt-image-1",
            "default_quality": "high",
            "partial_images": 3,
            "future_field": true
        }))
        .unwrap();
        assert_eq!(config.model, ImageGenerationModel::GptImage1);
        assert_eq!(config.default_quality, ImageGenerationQuality::High);
        assert_eq!(config.partial_images, 3);
    }

    #[test]
    fn capability_config_rejects_unknown_model_override() {
        let result = parse_capability_config(&json!({
            "model": "chatgpt-image-latest"
        }));
        let error = result.unwrap_err();
        match error {
            ToolExecutionResult::ToolError(message) => {
                assert!(message.contains("Invalid gpt_image_gen config"));
            }
            other => panic!("expected tool error, got {other:?}"),
        }
    }

    #[test]
    fn capability_config_rejects_invalid_partial_image_count() {
        let result = parse_capability_config(&json!({
            "partial_images": 4
        }));
        let error = result.unwrap_err();
        match error {
            ToolExecutionResult::ToolError(message) => {
                assert!(message.contains("partial_images must be between 0 and 3"));
            }
            other => panic!("expected tool error, got {other:?}"),
        }
    }

    #[test]
    fn resolve_quality_prefers_explicit_argument() {
        assert_eq!(
            resolve_quality(Some("high"), ImageGenerationQuality::Medium),
            "high"
        );
    }

    #[test]
    fn resolve_quality_uses_capability_default_when_omitted() {
        assert_eq!(
            resolve_quality(None, ImageGenerationQuality::Medium),
            "medium"
        );
    }

    #[test]
    fn resolve_partial_images_defaults_single_image_requests() {
        let config = GptImageGenCapabilityConfig::default();
        assert_eq!(resolve_partial_images(1, &config), Some(1));
    }

    #[test]
    fn resolve_partial_images_disables_preview_streaming_for_batches() {
        let config = GptImageGenCapabilityConfig::default();
        assert_eq!(resolve_partial_images(3, &config), None);
    }

    #[test]
    fn system_prompt_marks_image_tools_as_directly_invokable() {
        let prompt = GptImageGenCapability.system_prompt_addition().unwrap();
        assert!(prompt.contains("call them directly"));
        assert!(prompt.contains("do not claim they are unavailable"));
        assert!(prompt.contains("stop at writing prompts"));
    }

    #[tokio::test]
    async fn image_system_prompt_within_budget() {
        let ctx = everruns_core::capabilities::SystemPromptContext::without_file_store(
            everruns_provider::typed_id::SessionId::new(),
        );
        let prompt = GptImageGenCapability
            .system_prompt_contribution(&ctx)
            .await
            .unwrap();
        assert!(prompt.len() <= 850, "prompt is {} bytes", prompt.len());
    }

    #[test]
    fn localizations_cover_schema_summary_and_uk_name() {
        let cap = GptImageGenCapability;
        assert!(cap.describe_schema(None).is_some());
        assert_ne!(cap.localized_name(Some("uk-UA")), cap.name());
    }

    #[test]
    fn image_tools_are_never_deferred_for_tool_search() {
        for definition in GptImageGenCapability.tool_definitions() {
            let ToolDefinition::Builtin(builtin) = definition else {
                panic!("expected builtin tool definition");
            };
            assert_eq!(builtin.deferrable, DeferrablePolicy::Never);
        }
    }
}
