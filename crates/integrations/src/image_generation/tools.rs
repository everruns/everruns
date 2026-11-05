//! `generate_image` and `edit_image`.

use super::capability::{
    COMPLEXITIES, FORMATS, ImageGenerationConfig, QUALITIES, check_one_of, parse_capability_config,
};
use super::outputs::{
    MaterializeInput, add_workspace_prefix, infer_image_content_type, materialize_outputs,
    normalize_workspace_path,
};
use super::resolve::{ResolvedImageModel, build_driver, resolve_image_model};
use async_trait::async_trait;
use everruns_contracts::images::{ImageInput, ImageProgress, ImageRequest, ImagesDriver};
use everruns_contracts::runtime::connection_services::ProviderCredentialStore;
use everruns_contracts::runtime::session_file::SessionFile;
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tool_narration::{
    ToolNarrationContext, ToolNarrationPhase, narrate_labeled_action,
};
use everruns_contracts::runtime::tools::{Tool, ToolExecutionResult};
use everruns_contracts::tool_types::{ToolCall, ToolHints};
use everruns_contracts::typed_id::ImageId;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

const DEFAULT_GENERATE_PREFIX: &str = "generated-image";
const DEFAULT_EDIT_PREFIX: &str = "edited-image";
const MAX_EDIT_SOURCE_BYTES: usize = 50 * 1024 * 1024;

pub struct GenerateImageTool {
    config: Value,
    credentials: Option<Arc<dyn ProviderCredentialStore>>,
}

impl GenerateImageTool {
    pub(crate) fn new(config: Value) -> Self {
        Self {
            config,
            credentials: None,
        }
    }

    pub(crate) fn with_credentials(
        mut self,
        credentials: Option<Arc<dyn ProviderCredentialStore>>,
    ) -> Self {
        self.credentials = credentials;
        self
    }
}

pub struct EditImageTool {
    config: Value,
    credentials: Option<Arc<dyn ProviderCredentialStore>>,
}

impl EditImageTool {
    pub(crate) fn new(config: Value) -> Self {
        Self {
            config,
            credentials: None,
        }
    }

    pub(crate) fn with_credentials(
        mut self,
        credentials: Option<Arc<dyn ProviderCredentialStore>>,
    ) -> Self {
        self.credentials = credentials;
        self
    }
}

/// Arguments both tools share. Model-specific options (`complexity`,
/// `frames`, the higher qualities) are checked against the model profile.
fn common_properties() -> Value {
    json!({
        "size": {
            "type": "string",
            "enum": ["1024x1024", "1536x1024", "1024x1536", "auto"],
            "description": "Output size. Defaults to auto. Ignored for GIF, where the frame count sets the size."
        },
        "quality": {
            "type": "string",
            "enum": QUALITIES,
            "description": "Quality. Defaults to the configured quality. xhigh and max need GPT Image 2.5 or newer."
        },
        "background": {
            "type": "string",
            "enum": ["transparent", "opaque", "auto"],
            "description": "Background. Transparent needs png, webp or gif."
        },
        "format": {
            "type": "string",
            "enum": FORMATS,
            "description": "Output format. Defaults to the configured format (png). gif makes an animation, svg a vector image."
        },
        "complexity": {
            "type": "string",
            "enum": COMPLEXITIES,
            "description": "Scene detail for GPT Image 2.5+. Lower is faster."
        },
        "count": {
            "type": "integer",
            "minimum": 1,
            "maximum": 10,
            "description": "Number of images. Defaults to 1. GIF and SVG make one."
        },
        "save_to_session_fs": {
            "type": "boolean",
            "description": "Also save each image into the session filesystem. SVGs are always saved there."
        },
        "output_dir": {
            "type": "string",
            "description": "Session directory for saved images. Defaults to the configured folder (/workspace/.outputs/images)."
        },
        "filename_prefix": {
            "type": "string",
            "description": "Filename prefix for saved images."
        },
        "persist_artifact": {
            "type": "boolean",
            "description": "Keep each image in the durable image store. Defaults to true."
        }
    })
}

fn merge(mut base: Value, extra: Value) -> Value {
    if let (Some(base), Some(extra)) = (base.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            base.insert(key.clone(), value.clone());
        }
    }
    base
}

#[async_trait]
impl Tool for GenerateImageTool {
    fn narrate(
        &self,
        call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        let gif = call.arguments.get("format").and_then(Value::as_str) == Some("gif");
        let english = if gif {
            ("Animating GIF", "Made GIF", "Could not make GIF")
        } else {
            (
                "Generating image",
                "Generated image",
                "Could not generate image",
            )
        };
        let ukrainian = if gif {
            ("Створюю GIF", "Створив GIF", "Не вдалося створити GIF")
        } else {
            (
                "Генерую зображення",
                "Згенерував зображення",
                "Не вдалося згенерувати зображення",
            )
        };
        Some(narrate_labeled_action(
            &call.arguments,
            phase,
            locale,
            english,
            ukrainian,
            &[],
        ))
    }

    fn name(&self) -> &str {
        "generate_image"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Generate Image")
    }

    fn description(&self) -> &str {
        "Server-side image generation. Call this directly for image requests, including animated GIFs (format gif) and vector art (format svg). Results are kept as image artifacts and can be saved as session files."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": merge(
                json!({
                    "prompt": { "type": "string", "description": "What to draw. For a GIF, describe the motion." },
                    "frames": {
                        "type": "integer",
                        "minimum": 2,
                        "maximum": 64,
                        "description": "GIF only: number of frames. Defaults to the configured value (9)."
                    },
                    "frame_duration_ms": {
                        "type": "integer",
                        "minimum": 20,
                        "maximum": 2000,
                        "description": "GIF only: milliseconds per frame. Defaults to the configured value (200)."
                    }
                }),
                common_properties(),
            ),
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
        let config = match parse_capability_config(&self.config) {
            Ok(config) => config,
            Err(result) => return result,
        };
        let format = args.common.format(&config).to_string();
        let resolved =
            match resolve_image_model(context, self.credentials.as_ref(), &config, &format).await {
                Ok(resolved) => resolved,
                Err(result) => return result,
            };
        let request = match build_request(
            &args.prompt,
            &args.common,
            &config,
            &resolved,
            Animation {
                frames: args.frames,
                frame_duration_ms: args.frame_duration_ms,
            },
        ) {
            Ok(request) => request,
            Err(message) => return ToolExecutionResult::tool_error(message),
        };
        run(
            self.name(),
            "Generating",
            context,
            &resolved,
            request,
            &args.prompt,
            None,
            &args.common,
            config.output_dir.as_deref(),
            args.common
                .filename_prefix
                .clone()
                .unwrap_or_else(|| DEFAULT_GENERATE_PREFIX.to_string()),
        )
        .await
    }
}

#[async_trait]
impl Tool for EditImageTool {
    fn narrate(
        &self,
        call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(narrate_labeled_action(
            &call.arguments,
            phase,
            locale,
            ("Editing image", "Edited image", "Could not edit image"),
            (
                "Редагую зображення",
                "Відредагував зображення",
                "Не вдалося відредагувати зображення",
            ),
            &["path", "image_id"],
        ))
    }

    fn name(&self) -> &str {
        "edit_image"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Edit Image")
    }

    fn description(&self) -> &str {
        "Server-side image editing. Call this directly when the user wants an existing image revised, from a stored image artifact and/or a session file. GIF output is not available for edits."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": merge(
                json!({
                    "prompt": { "type": "string", "description": "The edit to make." },
                    "image_id": {
                        "type": "string",
                        "description": "Stored image artifact ID to edit."
                    },
                    "path": {
                        "type": "string",
                        "description": "Session file path of an image to edit."
                    },
                    "markup_path": {
                        "type": "string",
                        "description": "GPT Image 2.5+: session path of a transparent PNG with arrows, circles or notes drawn over the source, marking what to change."
                    }
                }),
                common_properties(),
            ),
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
        let config = match parse_capability_config(&self.config) {
            Ok(config) => config,
            Err(result) => return result,
        };
        let format = args.common.format(&config).to_string();
        if format == "gif" {
            return ToolExecutionResult::tool_error(
                "GIF output is only available for new images. Use generate_image with format gif.",
            );
        }
        let resolved =
            match resolve_image_model(context, self.credentials.as_ref(), &config, &format).await {
                Ok(resolved) => resolved,
                Err(result) => return result,
            };
        if let Some(profile) = &resolved.profile {
            if !profile.edits {
                return ToolExecutionResult::tool_error(format!(
                    "The image model {} does not edit images.",
                    resolved.api_model
                ));
            }
            if args.markup_path.is_some() && !profile.markup {
                return ToolExecutionResult::tool_error(format!(
                    "The image model {} does not take markup. Use GPT Image 2.5 or newer on OpenAI or Azure OpenAI.",
                    resolved.api_model
                ));
            }
        }
        let mut request = match build_request(
            &args.prompt,
            &args.common,
            &config,
            &resolved,
            Animation::default(),
        ) {
            Ok(request) => request,
            Err(message) => return ToolExecutionResult::tool_error(message),
        };
        request.sources = match collect_edit_sources(context, &args).await {
            Ok(sources) => sources,
            Err(result) => return result,
        };
        if let Some(path) = args.markup_path.as_deref() {
            request.markup = match read_session_image(context, path).await {
                Ok(markup) => Some(markup),
                Err(result) => return result,
            };
        }
        run(
            self.name(),
            "Editing",
            context,
            &resolved,
            request,
            &args.prompt,
            Some(json!({
                "image_id": args.image_id.map(|id| id.to_string()),
                "path": args.path,
                "markup_path": args.markup_path,
            })),
            &args.common,
            config.output_dir.as_deref(),
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
pub(crate) struct GenerateImageArgs {
    pub prompt: String,
    pub frames: Option<u8>,
    pub frame_duration_ms: Option<u16>,
    #[serde(flatten)]
    pub common: CommonImageArgs,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EditImageArgs {
    pub prompt: String,
    pub image_id: Option<ImageId>,
    pub path: Option<String>,
    pub markup_path: Option<String>,
    #[serde(flatten)]
    pub common: CommonImageArgs,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CommonImageArgs {
    pub size: Option<String>,
    pub quality: Option<String>,
    pub background: Option<String>,
    pub format: Option<String>,
    pub complexity: Option<String>,
    pub count: Option<usize>,
    pub save_to_session_fs: Option<bool>,
    pub output_dir: Option<String>,
    pub filename_prefix: Option<String>,
    pub persist_artifact: Option<bool>,
}

impl CommonImageArgs {
    pub(crate) fn count(&self) -> usize {
        self.count.unwrap_or(1)
    }

    pub(crate) fn format<'a>(&'a self, config: &'a ImageGenerationConfig) -> &'a str {
        self.format.as_deref().unwrap_or_else(|| config.format())
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Animation {
    pub frames: Option<u8>,
    pub frame_duration_ms: Option<u16>,
}

/// Turn tool arguments and settings into a driver request, checking every
/// option against what the resolved model supports.
pub(crate) fn build_request(
    prompt: &str,
    args: &CommonImageArgs,
    config: &ImageGenerationConfig,
    resolved: &ResolvedImageModel,
    animation: Animation,
) -> Result<ImageRequest, String> {
    let format = args.format(config);
    check_one_of("format", Some(format), FORMATS)?;
    check_one_of("quality", args.quality.as_deref(), QUALITIES)?;
    check_one_of("complexity", args.complexity.as_deref(), COMPLEXITIES)?;
    let count = args.count();
    let single_output = matches!(format, "gif" | "svg");
    if single_output && count != 1 {
        return Err(format!(
            "{format} output makes one image per call; set count to 1"
        ));
    }
    let max_count = resolved
        .profile
        .as_ref()
        .map_or(10, |profile| usize::from(profile.max_count.max(1)));
    if !(1..=max_count).contains(&count) {
        return Err(format!("count must be between 1 and {max_count}"));
    }
    if args.background.as_deref() == Some("transparent") && matches!(format, "jpeg" | "svg") {
        return Err("transparent background requires png, webp or gif output".to_string());
    }
    if format != "gif" && (animation.frames.is_some() || animation.frame_duration_ms.is_some()) {
        return Err("frames and frame_duration_ms only apply to format gif".to_string());
    }

    let profile = resolved.profile.as_ref();
    let quality = match profile {
        // Models without quality levels (Muse, Gemini) take none.
        Some(profile) if profile.qualities.is_empty() => None,
        Some(profile) => {
            let quality = args.quality.as_deref().unwrap_or_else(|| config.quality());
            if !profile.qualities.iter().any(|value| value == quality) {
                if args.quality.is_some() {
                    return Err(format!(
                        "quality {quality} is not available on {}; use one of {}",
                        resolved.api_model,
                        profile.qualities.join(", ")
                    ));
                }
                // A configured quality the model lacks falls back to the
                // model default rather than failing every call.
                None
            } else {
                Some(quality.to_string())
            }
        }
        None => Some(
            args.quality
                .as_deref()
                .unwrap_or_else(|| config.quality())
                .to_string(),
        ),
    };
    let complexity = args
        .complexity
        .clone()
        .or_else(|| config.complexity.clone());
    let complexity = match profile {
        Some(profile) if !profile.complexity => {
            if args.complexity.is_some() {
                return Err(format!(
                    "complexity is not available on {}",
                    resolved.api_model
                ));
            }
            None
        }
        _ => complexity,
    };

    let (frame_count, frame_duration_ms) = if format == "gif" {
        let limits = profile.and_then(|profile| profile.gif);
        let frames = animation
            .frames
            .or(config.gif_frames)
            .or(limits.map(|limits| limits.default_frames));
        let duration = animation
            .frame_duration_ms
            .or(config.gif_frame_ms)
            .or(limits.map(|limits| limits.default_frame_ms));
        if let Some(limits) = limits {
            if let Some(frames) = frames
                && !(limits.min_frames..=limits.max_frames).contains(&frames)
            {
                return Err(format!(
                    "frames must be between {} and {}",
                    limits.min_frames, limits.max_frames
                ));
            }
            if let Some(duration) = duration
                && !(limits.min_frame_ms..=limits.max_frame_ms).contains(&duration)
            {
                return Err(format!(
                    "frame_duration_ms must be between {} and {}",
                    limits.min_frame_ms, limits.max_frame_ms
                ));
            }
        }
        (frames, duration)
    } else {
        (None, None)
    };

    // Previews stream only for single still images; GIF and SVG never stream.
    let partial_images = (count == 1 && !single_output && config.partial_images > 0)
        .then_some(config.partial_images);

    // A GIF is drawn as one sheet of frames, so the frame size depends on the
    // frame count and the API rejects most explicit sizes. Let it choose.
    let size = if format == "gif" {
        None
    } else {
        args.size.clone()
    };

    Ok(ImageRequest {
        model: resolved.api_model.clone(),
        prompt: prompt.to_string(),
        size,
        quality,
        background: args.background.clone(),
        output_format: Some(format.to_string()),
        count: u8::try_from(count).unwrap_or(1),
        complexity,
        frame_count,
        frame_duration_ms,
        partial_images,
        sources: Vec::new(),
        markup: None,
    })
}

#[allow(clippy::too_many_arguments)]
async fn run(
    tool_name: &str,
    action: &str,
    context: &ToolContext,
    resolved: &ResolvedImageModel,
    request: ImageRequest,
    prompt: &str,
    source: Option<Value>,
    common: &CommonImageArgs,
    output_dir: Option<&str>,
    filename_prefix: String,
) -> ToolExecutionResult {
    let driver = match build_driver(context, resolved) {
        Ok(driver) => driver,
        Err(result) => return result,
    };
    let count = usize::from(request.count);
    let expected_previews = request.partial_images;
    let format = request.output_format.clone().unwrap_or_default();
    let frames = request.frame_count;
    let frame_duration_ms = request.frame_duration_ms;
    context
        .emit_progress(
            tool_name,
            &initial_progress_message(action, &format, count, expected_previews),
        )
        .await;

    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let progress = async {
        while let Some(event) = receiver.recv().await {
            emit_progress(context, tool_name, action, expected_previews, event).await;
        }
    };
    let (response, ()) = futures::join!(
        driver.create_images(request, expected_previews.map(|_| sender)),
        progress
    );
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            return ToolExecutionResult::tool_error(format!("Image API error: {}", error.0));
        }
    };

    materialize_outputs(
        context,
        MaterializeInput {
            prompt,
            source,
            provider_type: &resolved.provider_type,
            model: &resolved.api_model,
            format: &format,
            frames,
            frame_duration_ms,
            save_to_session_fs: common.save_to_session_fs.unwrap_or(false),
            persist_artifact: common.persist_artifact.unwrap_or(true),
            output_dir: common.output_dir.as_deref().or(output_dir),
            filename_prefix,
        },
        response,
    )
    .await
}

fn parse_arguments<T: for<'de> Deserialize<'de>>(
    arguments: Value,
) -> Result<T, ToolExecutionResult> {
    serde_json::from_value(arguments)
        .map_err(|error| ToolExecutionResult::tool_error(format!("Invalid arguments: {error}")))
}

pub(crate) fn initial_progress_message(
    action: &str,
    format: &str,
    count: usize,
    partial_images: Option<u8>,
) -> String {
    if format == "gif" {
        "Animating GIF frames...".to_string()
    } else if count > 1 {
        format!("{action} {count} images...")
    } else if partial_images.unwrap_or(0) > 0 {
        format!("{action} image and waiting for progress updates...")
    } else {
        format!("{action} image...")
    }
}

async fn emit_progress(
    context: &ToolContext,
    tool_name: &str,
    action: &str,
    expected_previews: Option<u8>,
    event: ImageProgress,
) {
    let (message, count) = match event {
        ImageProgress::Partial { index } => {
            let current = u64::try_from(index).unwrap_or(u64::MAX).saturating_add(1);
            let total = u64::from(expected_previews.unwrap_or(1)).max(current);
            (
                format!("{action} progress update {current} of {total}..."),
                Some((current, total)),
            )
        }
        ImageProgress::Completed { .. } => (format!("{action} final image..."), None),
    };
    context
        .emit_progress_counted(tool_name, &message, count)
        .await;
}

async fn collect_edit_sources(
    context: &ToolContext,
    args: &EditImageArgs,
) -> Result<Vec<ImageInput>, ToolExecutionResult> {
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
        sources.push(ImageInput {
            filename: image.info.filename,
            media_type: image.info.content_type,
            data: image.data,
        });
    }

    if let Some(path) = args.path.as_deref() {
        sources.push(read_session_image(context, path).await?);
    }

    Ok(sources)
}

async fn read_session_image(
    context: &ToolContext,
    path: &str,
) -> Result<ImageInput, ToolExecutionResult> {
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
    let bytes = SessionFile::decode_content(content, &file.encoding).map_err(|error| {
        ToolExecutionResult::tool_error(format!(
            "Failed to decode session file content for {display_path}: {error}"
        ))
    })?;
    if bytes.len() > MAX_EDIT_SOURCE_BYTES {
        return Err(ToolExecutionResult::tool_error(format!(
            "Session file exceeds the 50MB edit limit: {display_path}"
        )));
    }
    let Some(media_type) = infer_image_content_type(&display_path) else {
        return Err(ToolExecutionResult::tool_error(format!(
            "Unsupported image file type for {display_path}. Use png, jpg, jpeg, or webp."
        )));
    };
    Ok(ImageInput {
        filename: file.name,
        media_type: media_type.to_string(),
        data: bytes,
    })
}
