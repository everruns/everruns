//! Store generated images and shape the tool result.
//!
//! - Stills and GIFs become image artifacts (unless `persist_artifact` is
//!   false). The chat model sees a GIF as its first frame in PNG, since model
//!   APIs take still images; people see the animation through the artifact.
//! - Without an image store (an embedded Framework app), images are written
//!   to the session filesystem instead.
//! - SVGs are never stored as image artifacts: an SVG served from the image
//!   endpoints could carry script. They are written to the session
//!   filesystem only and returned as a path.

use base64::Engine;
use everruns_contracts::ToolResultImage;
use everruns_contracts::images::{ImageResponse, ImageUsage};
use everruns_contracts::runtime::image_services::CreateStoredImage;
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tools::ToolExecutionResult;
use serde_json::{Value, json};

const DEFAULT_OUTPUT_DIR: &str = "/workspace/.outputs/images";

pub(crate) struct MaterializeInput<'a> {
    pub prompt: &'a str,
    pub source: Option<Value>,
    pub provider_type: &'a str,
    pub model: &'a str,
    pub format: &'a str,
    pub frames: Option<u8>,
    pub frame_duration_ms: Option<u16>,
    pub save_to_session_fs: bool,
    pub persist_artifact: bool,
    pub output_dir: Option<&'a str>,
    pub filename_prefix: String,
}

pub(crate) async fn materialize_outputs(
    context: &ToolContext,
    input: MaterializeInput<'_>,
    response: ImageResponse,
) -> ToolExecutionResult {
    let svg = input.format == "svg";
    let gif = input.format == "gif";
    let count = response.images.len();
    let mut rendered_images = Vec::with_capacity(count);
    let mut rendered_results = Vec::with_capacity(count);

    for (index, image) in response.images.into_iter().enumerate() {
        let bytes = match base64::engine::general_purpose::STANDARD.decode(&image.b64_json) {
            Ok(bytes) => bytes,
            Err(error) => {
                return ToolExecutionResult::internal_error_msg(format!(
                    "invalid base64 image data from the image API: {error}"
                ));
            }
        };
        let filename = output_filename(&input.filename_prefix, index, count, input.format);
        let media_type = format_media_type(input.format);

        // A host without an image store (an embedded Framework app) keeps
        // the image as a session file instead.
        let image_store = context
            .image_store
            .as_ref()
            .filter(|_| input.persist_artifact && !svg);
        let artifact_id = if let Some(image_store) = image_store {
            match image_store
                .create_image(CreateStoredImage {
                    filename: filename.clone(),
                    content_type: media_type.to_string(),
                    data: bytes.clone(),
                    metadata: json!({
                        "provider": input.provider_type,
                        "model": input.model,
                        "prompt": input.prompt,
                        "revised_prompt": image.revised_prompt,
                        "frames": input.frames,
                        "frame_duration_ms": input.frame_duration_ms,
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

        let save_file = input.save_to_session_fs
            || svg
            || (input.persist_artifact && context.image_store.is_none());
        let session_path = if save_file {
            let Some(file_store) = &context.file_store else {
                return ToolExecutionResult::internal_error_msg(
                    "Session file store not available in this context",
                );
            };
            let output_dir =
                normalize_workspace_path(input.output_dir.unwrap_or(DEFAULT_OUTPUT_DIR));
            let normalized_path = join_workspace_path(&output_dir, &filename);
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
                // Policy denials and quota errors are the model's to work
                // around (another directory, no file), so it sees them.
                Err(error) => {
                    return ToolExecutionResult::tool_error(format!(
                        "Could not save {}: {error}",
                        add_workspace_prefix(&normalized_path)
                    ));
                }
            }
        } else {
            None
        };

        let mut frames = None;
        if gif {
            match gif_preview(&bytes) {
                Some((png, frame_count)) => {
                    frames = Some(frame_count);
                    rendered_images.push(ToolResultImage {
                        base64: base64::engine::general_purpose::STANDARD.encode(png),
                        media_type: "image/png".to_string(),
                    });
                }
                None => {
                    return ToolExecutionResult::internal_error_msg(
                        "the image API returned a GIF that could not be decoded",
                    );
                }
            }
        } else if !svg {
            rendered_images.push(ToolResultImage {
                base64: image.b64_json.clone(),
                media_type: media_type.to_string(),
            });
        }

        rendered_results.push(json!({
            "index": index + 1,
            "artifact_id": artifact_id.map(|id| id.to_string()),
            "session_file": session_path,
            "filename": filename,
            "media_type": media_type,
            "size_bytes": bytes.len(),
            "frames": frames,
            "frame_duration_ms": gif.then_some(input.frame_duration_ms).flatten(),
            "revised_prompt": image.revised_prompt,
        }));
    }

    let result = json!({
        "provider": input.provider_type,
        "model": input.model,
        "prompt": input.prompt,
        "format": input.format,
        "count": rendered_results.len(),
        "source": input.source,
        "images": rendered_results,
        "usage": response.usage.map(usage_json),
    });
    if rendered_images.is_empty() {
        ToolExecutionResult::success(result)
    } else {
        ToolExecutionResult::success_with_images(result, rendered_images)
    }
}

fn usage_json(usage: ImageUsage) -> Value {
    json!({
        "input_text_tokens": usage.input_text_tokens,
        "input_image_tokens": usage.input_image_tokens,
        "output_tokens": usage.output_tokens,
    })
}

/// First frame as PNG and the frame count, or `None` when the bytes are not
/// a decodable GIF.
pub(crate) fn gif_preview(bytes: &[u8]) -> Option<(Vec<u8>, usize)> {
    use image::AnimationDecoder;
    let decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).ok()?;
    let mut frames = decoder.into_frames();
    let first = frames.next()?.ok()?.into_buffer();
    let count = 1 + frames.filter(Result::is_ok).count();
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(first)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some((png, count))
}

pub(crate) fn output_filename(prefix: &str, index: usize, count: usize, format: &str) -> String {
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

pub(crate) fn format_media_type(format: &str) -> &'static str {
    match format {
        "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        _ => "image/png",
    }
}

pub(crate) fn normalize_workspace_path(path: &str) -> String {
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
        format!("/{path}")
    };
    if path.is_empty() {
        "/".to_string()
    } else {
        path
    }
}

pub(crate) fn add_workspace_prefix(path: &str) -> String {
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

pub(crate) fn infer_image_content_type(path: &str) -> Option<&'static str> {
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
