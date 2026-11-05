//! OpenAI-compatible Images API as an [`ImagesDriver`].
//!
//! One driver serves every account that speaks `/v1/images/generations` and
//! `/v1/images/edits`: OpenAI, Azure OpenAI's `/openai/v1` surface (api-key
//! header), Meta's Model API (`muse-image-1.0`) and OpenRouter, which serves
//! generations at `/api/v1/images`. GPT Image 2.6 adds `complexity`, `markup`
//! edits and `gif`/`svg` output; GIF takes `frame_count` and
//! `frame_duration_ms` and is never streamed.
//!
//! THREAT[TM-LLM-047]: the driver never dials. It builds the request, adds
//! the key for the configured origin, and hands it to the host's
//! [`ImagesTransport`], which owns ACLs, DNS pinning and the no-redirect rule.

use std::sync::Arc;

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use everruns_contracts::images::{
    GeneratedImage, ImageInput, ImageProgress, ImageProgressSender, ImageRequest, ImageResponse,
    ImageUsage, ImagesDriver, ImagesDriverError, ImagesHttpBody, ImagesHttpRequest,
    ImagesTransport,
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// OpenAI API root.
pub const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
/// Meta Model API root (serves `muse-image-1.0`).
pub const DEFAULT_META_BASE_URL: &str = "https://api.meta.ai/v1";
/// OpenRouter API root.
pub const DEFAULT_OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
const DEFAULT_TIMEOUT_MS: u64 = 300_000;
/// Error bodies are echoed into tool errors; cap them so a hostile endpoint
/// cannot flood the transcript.
const MAX_ERROR_BODY_CHARS: usize = 2048;

/// The OpenAI-compatible Images API for one account.
#[derive(Clone)]
pub struct OpenAIImagesDriver {
    transport: Arc<dyn ImagesTransport>,
    provider_type: String,
    api_key: String,
    base_url: String,
    headers: Vec<(String, String)>,
    timeout_ms: u64,
}

impl std::fmt::Debug for OpenAIImagesDriver {
    /// Never renders the key.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAIImagesDriver")
            .field("provider_type", &self.provider_type)
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl OpenAIImagesDriver {
    /// A driver for `provider_type` (`openai`, `azure_openai`, `meta`,
    /// `openrouter`) at `base_url`, or that provider's default root.
    pub fn new(
        transport: Arc<dyn ImagesTransport>,
        provider_type: impl Into<String>,
        api_key: impl Into<String>,
        base_url: Option<String>,
    ) -> Self {
        let provider_type = provider_type.into();
        let base_url = base_url
            .filter(|url| !url.trim().is_empty())
            .unwrap_or_else(|| default_base_url(&provider_type).to_string());
        Self {
            transport,
            provider_type,
            api_key: api_key.into(),
            base_url,
            headers: Vec::new(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
        }
    }

    /// Extra provider request headers (provider settings), sent on every call.
    pub fn with_headers(mut self, headers: impl IntoIterator<Item = (String, String)>) -> Self {
        self.headers = headers.into_iter().collect();
        self
    }

    /// Per-request timeout. GIFs and high quality stills can take minutes.
    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms.max(1);
        self
    }

    fn generations_path(&self) -> &'static str {
        // OpenRouter serves text-to-image at `/v1/images` rather than the
        // OpenAI-compatible `/v1/images/generations`.
        if self.provider_type == "openrouter" {
            "images"
        } else {
            "images/generations"
        }
    }

    fn http_request(
        &self,
        endpoint: &str,
        content_type: String,
        body: Vec<u8>,
        stream: bool,
    ) -> Result<ImagesHttpRequest, ImagesDriverError> {
        let url = endpoint_url(&self.base_url, endpoint)?;
        let mut headers = vec![("content-type".to_string(), content_type)];
        // Azure OpenAI authenticates with `api-key`, recognized by URL as the
        // chat driver does; everything else takes a Bearer token.
        if everruns_contracts::openai_protocol::is_azure_openai_api_url(&url) {
            headers.push(("api-key".to_string(), self.api_key.clone()));
        } else {
            headers.push((
                "authorization".to_string(),
                format!("Bearer {}", self.api_key),
            ));
        }
        if stream {
            headers.push(("accept".to_string(), "text/event-stream".to_string()));
        }
        headers.extend(self.headers.iter().cloned());
        Ok(ImagesHttpRequest {
            url,
            headers,
            body,
            stream,
            timeout_ms: self.timeout_ms,
        })
    }
}

fn default_base_url(provider_type: &str) -> &'static str {
    match provider_type {
        "meta" => DEFAULT_META_BASE_URL,
        "openrouter" => DEFAULT_OPENROUTER_BASE_URL,
        _ => DEFAULT_OPENAI_BASE_URL,
    }
}

/// Join `endpoint` onto the base URL, dropping an endpoint suffix someone
/// pasted into the provider's base URL.
fn endpoint_url(base_url: &str, endpoint: &str) -> Result<String, ImagesDriverError> {
    let mut normalized = base_url.trim_end_matches('/');
    for suffix in [
        "/responses",
        "/chat/completions",
        "/images/generations",
        "/images/edits",
        "/images",
    ] {
        if let Some(prefix) = normalized.strip_suffix(suffix) {
            normalized = prefix;
            break;
        }
    }
    let url = format!("{normalized}/{}", endpoint.trim_start_matches('/'));
    reqwest::Url::parse(&url)
        .map(|_| url)
        .map_err(|error| ImagesDriverError(format!("invalid image API URL: {error}")))
}

#[derive(Serialize)]
struct GenerationBody<'a> {
    model: &'a str,
    prompt: &'a str,
    #[serde(rename = "n")]
    count: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    quality: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    background: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_format: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    complexity: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame_count: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame_duration_ms: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    partial_images: Option<u8>,
}

#[derive(Deserialize)]
struct ApiResponse {
    data: Vec<ApiImage>,
    #[serde(default)]
    usage: Option<ApiUsage>,
}

#[derive(Deserialize)]
struct ApiImage {
    b64_json: String,
    #[serde(default)]
    revised_prompt: Option<String>,
}

#[derive(Deserialize, Default)]
struct ApiUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
    #[serde(default)]
    input_tokens_details: Option<ApiInputDetails>,
}

#[derive(Deserialize, Default)]
struct ApiInputDetails {
    #[serde(default)]
    text_tokens: u32,
    #[serde(default)]
    image_tokens: u32,
}

impl ApiUsage {
    fn into_usage(self) -> ImageUsage {
        let (text, image) = match self.input_tokens_details {
            Some(details) => (details.text_tokens, details.image_tokens),
            None => (self.input_tokens, 0),
        };
        ImageUsage {
            input_text_tokens: text,
            input_image_tokens: image,
            output_tokens: self.output_tokens,
        }
    }
}

/// Whether this request should stream: a single still with previews asked
/// for. GIF and SVG never stream.
fn streams(request: &ImageRequest) -> Option<u8> {
    let animated_or_vector = matches!(request.output_format.as_deref(), Some("gif" | "svg"));
    request
        .partial_images
        .filter(|previews| *previews > 0 && request.count == 1 && !animated_or_vector)
}

fn generation_body(request: &ImageRequest, partial_images: Option<u8>) -> GenerationBody<'_> {
    let gif = request.output_format.as_deref() == Some("gif");
    GenerationBody {
        model: &request.model,
        prompt: &request.prompt,
        count: request.count.max(1),
        size: request.size.as_deref(),
        quality: request.quality.as_deref(),
        background: request.background.as_deref(),
        output_format: request.output_format.as_deref(),
        complexity: request.complexity.as_deref(),
        // OpenAI rejects GIF fields on other formats.
        frame_count: request.frame_count.filter(|_| gif),
        frame_duration_ms: request.frame_duration_ms.filter(|_| gif),
        stream: partial_images.map(|_| true),
        partial_images,
    }
}

/// Encode an edit as `multipart/form-data` in memory, because the transport
/// carries a byte body. Returns `(content_type, body)`.
fn edit_form(
    request: &ImageRequest,
    partial_images: Option<u8>,
) -> Result<(String, Vec<u8>), ImagesDriverError> {
    let image_field = if request.sources.len() > 1 {
        "image[]"
    } else {
        "image"
    };
    let boundary = multipart_boundary();
    let mut body = Vec::new();
    let mut text = |name: &str, value: &str| {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    };
    text("model", &request.model);
    text("prompt", &request.prompt);
    text("n", &request.count.max(1).to_string());
    for (name, value) in [
        ("size", &request.size),
        ("quality", &request.quality),
        ("background", &request.background),
        ("output_format", &request.output_format),
        ("complexity", &request.complexity),
    ] {
        if let Some(value) = value {
            text(name, value);
        }
    }
    if let Some(previews) = partial_images {
        text("stream", "true");
        text("partial_images", &previews.to_string());
    }
    let mut file = |name: &str, image: &ImageInput| -> Result<(), ImagesDriverError> {
        let media_type = image.media_type.trim();
        if !media_type.contains('/')
            || media_type
                .chars()
                .any(|c| c.is_control() || c == '"' || c == ';')
        {
            return Err(ImagesDriverError(
                "invalid source image content type".into(),
            ));
        }
        let filename = sanitize_filename(&image.filename);
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\nContent-Type: {media_type}\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(&image.data);
        body.extend_from_slice(b"\r\n");
        Ok(())
    };
    for image in &request.sources {
        file(image_field, image)?;
    }
    if let Some(markup) = &request.markup {
        file("markup", markup)?;
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    Ok((format!("multipart/form-data; boundary={boundary}"), body))
}

/// Random boundary; collisions with binary image bytes are negligible at
/// 128 bits of entropy.
fn multipart_boundary() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::from("everruns-image-");
    for _ in 0..2 {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u128(
            web_time::SystemTime::now()
                .duration_since(web_time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default(),
        );
        out.push_str(&format!("{:016x}", hasher.finish()));
    }
    out
}

/// Keep header-breaking characters out of the `filename` parameter.
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '"' | '\\' | '\r' | '\n' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect()
}

fn status_error(status: u16, body: &[u8]) -> ImagesDriverError {
    if (300..400).contains(&status) {
        return ImagesDriverError(format!(
            "image API returned redirect {status}; redirects are not followed for image providers"
        ));
    }
    let body: String = String::from_utf8_lossy(body)
        .chars()
        .take(MAX_ERROR_BODY_CHARS)
        .collect();
    ImagesDriverError(format!("image API returned {status}: {body}"))
}

fn decode_response(body: &[u8]) -> Result<ImageResponse, ImagesDriverError> {
    let response: ApiResponse = serde_json::from_slice(body).map_err(|error| {
        ImagesDriverError(format!("failed to decode image API response: {error}"))
    })?;
    Ok(ImageResponse {
        images: response
            .data
            .into_iter()
            .map(|image| GeneratedImage {
                b64_json: image.b64_json,
                revised_prompt: image.revised_prompt,
            })
            .collect(),
        usage: response.usage.map(ApiUsage::into_usage),
    })
}

async fn collect_body(body: ImagesHttpBody) -> Result<Vec<u8>, ImagesDriverError> {
    match body {
        ImagesHttpBody::Full(bytes) => Ok(bytes),
        ImagesHttpBody::Stream(mut stream) => {
            let mut bytes = Vec::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(ImagesDriverError)?;
                if bytes.len() < 64 * 1024 {
                    bytes.extend_from_slice(&chunk);
                }
            }
            Ok(bytes)
        }
    }
}

async fn parse_stream(
    body: ImagesHttpBody,
    progress: Option<ImageProgressSender>,
) -> Result<ImageResponse, ImagesDriverError> {
    let stream = match body {
        ImagesHttpBody::Full(bytes) => {
            futures::stream::once(async move { Ok::<_, String>(bytes) }).boxed()
        }
        ImagesHttpBody::Stream(stream) => stream,
    };
    let mut events = stream.eventsource();
    let mut images = Vec::new();
    let mut usage = None;
    let send = |event: ImageProgress| {
        if let Some(sender) = &progress {
            let _ = sender.send(event);
        }
    };
    while let Some(event) = events.next().await {
        let event = event
            .map_err(|error| ImagesDriverError(format!("failed to read image stream: {error}")))?;
        if event.data == "[DONE]" {
            break;
        }
        let payload: Value = serde_json::from_str(&event.data).map_err(|error| {
            ImagesDriverError(format!("failed to decode image stream event: {error}"))
        })?;
        match payload.get("type").and_then(Value::as_str) {
            Some("image_generation.partial_image" | "image_edit.partial_image") => {
                let index = payload
                    .get("partial_image_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                send(ImageProgress::Partial { index });
            }
            Some("image_generation.completed" | "image_edit.completed") => {
                if let Some(event_usage) = payload.get("usage") {
                    usage = serde_json::from_value::<ApiUsage>(event_usage.clone())
                        .ok()
                        .map(ApiUsage::into_usage);
                }
                let image: ApiImage = serde_json::from_value(payload).map_err(|error| {
                    ImagesDriverError(format!("failed to decode completed image event: {error}"))
                })?;
                images.push(GeneratedImage {
                    b64_json: image.b64_json,
                    revised_prompt: image.revised_prompt,
                });
                send(ImageProgress::Completed {
                    count: images.len(),
                });
            }
            _ => {}
        }
    }
    if images.is_empty() {
        return Err(ImagesDriverError(
            "image stream ended before any completed image was received".into(),
        ));
    }
    Ok(ImageResponse { images, usage })
}

#[async_trait]
impl ImagesDriver for OpenAIImagesDriver {
    async fn create_images(
        &self,
        request: ImageRequest,
        progress: Option<ImageProgressSender>,
    ) -> Result<ImageResponse, ImagesDriverError> {
        let partial_images = streams(&request);
        let http = if request.is_edit() {
            let (content_type, body) = edit_form(&request, partial_images)?;
            self.http_request("images/edits", content_type, body, partial_images.is_some())?
        } else {
            let body = serde_json::to_vec(&generation_body(&request, partial_images)).map_err(
                |error| ImagesDriverError(format!("failed to encode image request: {error}")),
            )?;
            self.http_request(
                self.generations_path(),
                "application/json".into(),
                body,
                partial_images.is_some(),
            )?
        };
        let response = self.transport.send(http).await?;
        if !(200..300).contains(&response.status) {
            let body = collect_body(response.body).await?;
            return Err(status_error(response.status, &body));
        }
        if partial_images.is_some() {
            parse_stream(response.body, progress).await
        } else {
            decode_response(&collect_body(response.body).await?)
        }
    }
}

#[cfg(test)]
#[path = "images_tests.rs"]
mod tests;
