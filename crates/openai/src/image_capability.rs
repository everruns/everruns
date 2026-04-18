use crate::images::{
    EditImageInput, EditImageRequest, GenerateImageRequest, ImageApiImage, OpenAiImageClient,
};
use async_trait::async_trait;
use base64::Engine;
use everruns_core::ImageId;
use everruns_core::capabilities::{Capability, CapabilityStatus, IntegrationPlugin};
use everruns_core::session_file::SessionFile;
use everruns_core::tool_types::ToolHints;
use everruns_core::tools::{Tool, ToolExecutionResult, ToolResultImage};
use everruns_core::traits::{CreateStoredImage, ToolContext};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::LazyLock;

const GPT_IMAGE_GEN_CAPABILITY_ID: &str = "gpt_image_gen";
const OPENAI_IMAGE_MODEL: &str = "gpt-image-1";
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
    r#"Use `generate_image` for raster image generation and `edit_image` to transform existing images.

Store per-session OpenAI overrides in `secret_store` under `OPENAI_API_KEY` and optionally `OPENAI_BASE_URL`.
When saving files, write under `/workspace/.outputs/images` unless the user requests another directory."#
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
        vec![Box::new(GenerateImageTool), Box::new(EditImageTool)]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_file_system", "session_storage"]
    }
}

pub struct GenerateImageTool;

#[async_trait]
impl Tool for GenerateImageTool {
    fn name(&self) -> &str {
        "generate_image"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Generate Image")
    }

    fn description(&self) -> &str {
        "Generate raster images with OpenAI's GPT Image API and optionally persist them as artifacts or session files."
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
                    "description": "Generation quality. Defaults to auto."
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

        let client = match build_client(context).await {
            Ok(client) => client,
            Err(result) => return result,
        };

        let response = match client
            .generate(GenerateImageRequest {
                model: OPENAI_IMAGE_MODEL.to_string(),
                prompt: args.prompt.clone(),
                size: args.common.size.clone(),
                quality: args.common.quality.clone(),
                background: args.common.background.clone(),
                output_format: args.common.format.clone(),
                count: args.common.count(),
            })
            .await
        {
            Ok(response) => response,
            Err(error) => return ToolExecutionResult::internal_error_msg(error.to_string()),
        };

        materialize_outputs(
            context,
            &args.prompt,
            None,
            &args.common,
            response.data,
            args.common
                .filename_prefix
                .clone()
                .unwrap_or_else(|| DEFAULT_GENERATE_PREFIX.to_string()),
        )
        .await
    }
}

pub struct EditImageTool;

#[async_trait]
impl Tool for EditImageTool {
    fn name(&self) -> &str {
        "edit_image"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Edit Image")
    }

    fn description(&self) -> &str {
        "Edit one or more source images from a stored image artifact and/or a session file path."
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
                    "description": "Generation quality. Defaults to auto."
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

        let sources = match collect_edit_sources(context, &args).await {
            Ok(sources) => sources,
            Err(result) => return result,
        };
        let client = match build_client(context).await {
            Ok(client) => client,
            Err(result) => return result,
        };

        let response = match client
            .edit(EditImageRequest {
                model: OPENAI_IMAGE_MODEL.to_string(),
                prompt: args.prompt.clone(),
                images: sources,
                size: args.common.size.clone(),
                quality: args.common.quality.clone(),
                background: args.common.background.clone(),
                output_format: args.common.format.clone(),
                count: args.common.count(),
            })
            .await
        {
            Ok(response) => response,
            Err(error) => return ToolExecutionResult::internal_error_msg(error.to_string()),
        };

        materialize_outputs(
            context,
            &args.prompt,
            Some(json!({
                "image_id": args.image_id.map(|id| id.to_string()),
                "path": args.path,
            })),
            &args.common,
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

fn parse_arguments<T: for<'de> Deserialize<'de>>(
    arguments: Value,
) -> Result<T, ToolExecutionResult> {
    serde_json::from_value(arguments)
        .map_err(|error| ToolExecutionResult::tool_error(format!("Invalid arguments: {error}")))
}

async fn build_client(context: &ToolContext) -> Result<OpenAiImageClient, ToolExecutionResult> {
    let config = match resolve_client_config(context).await {
        Ok(config) => config,
        Err(result) => return Err(result),
    };

    OpenAiImageClient::new(config.api_key, config.base_url)
        .map_err(|error| ToolExecutionResult::internal_error_msg(error.to_string()))
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
            Err(error) => return Err(ToolExecutionResult::internal_error_msg(error.to_string())),
        };
        let base_url = match get_first_secret(
            storage_store.as_ref(),
            context,
            SESSION_BASE_URL_SECRET_NAMES,
        )
        .await
        {
            Ok(base_url) => base_url,
            Err(error) => return Err(ToolExecutionResult::internal_error_msg(error.to_string())),
        };
        if let Some(api_key) = api_key {
            return Ok(ResolvedClientConfig { api_key, base_url });
        }

        if let Some(base_url) = base_url {
            let Some(provider_store) = &context.provider_credential_store else {
                return Err(ToolExecutionResult::tool_error(
                    "OpenAI credentials are not configured. Store OPENAI_API_KEY via secret_store or configure an OpenAI provider.",
                ));
            };

            return match provider_store
                .get_default_provider_credentials("openai")
                .await
            {
                Ok(Some(credentials)) => Ok(ResolvedClientConfig {
                    api_key: credentials.api_key,
                    base_url: Some(base_url),
                }),
                Ok(None) => Err(ToolExecutionResult::tool_error(
                    "OpenAI credentials are not configured. Store OPENAI_API_KEY via secret_store or configure an OpenAI provider.",
                )),
                Err(error) => Err(ToolExecutionResult::internal_error(error)),
            };
        }
    }

    let Some(provider_store) = &context.provider_credential_store else {
        return Err(ToolExecutionResult::tool_error(
            "OpenAI credentials are not configured. Store OPENAI_API_KEY via secret_store or configure an OpenAI provider.",
        ));
    };

    match provider_store
        .get_default_provider_credentials("openai")
        .await
    {
        Ok(Some(credentials)) => Ok(ResolvedClientConfig {
            api_key: credentials.api_key,
            base_url: credentials.base_url,
        }),
        Ok(None) => Err(ToolExecutionResult::tool_error(
            "OpenAI credentials are not configured. Store OPENAI_API_KEY via secret_store or configure an OpenAI provider.",
        )),
        Err(error) => Err(ToolExecutionResult::internal_error(error)),
    }
}

async fn get_first_secret(
    store: &dyn everruns_core::traits::SessionStorageStore,
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
                        "model": OPENAI_IMAGE_MODEL,
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
            "model": OPENAI_IMAGE_MODEL,
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
}
