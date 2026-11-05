//! Image generation service contract (knowledge/execution/image-generation.md).
//!
//! An [`ImagesDriver`] makes images for one bound account: the OpenAI-compatible
//! Images API driver in `everruns-drivers` (OpenAI, Azure OpenAI, OpenRouter,
//! Meta) is the shipped implementation. The `image_generation` capability
//! resolves a model (capability setting, then the org's default image model),
//! asks the host for a binding, and calls the driver.
//!
//! Drivers never dial the network themselves: they hand an
//! [`ImagesHttpRequest`] to an [`ImagesTransport`]. Hosts supply one over
//! their egress boundary (ACL, allowlist, DNS pinning, no redirects;
//! THREAT[TM-LLM-047]), so a provider key only reaches its configured origin.

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};

/// A source image for an edit, or a markup layer.
#[derive(Debug, Clone)]
pub struct ImageInput {
    /// Filename sent in the multipart form.
    pub filename: String,
    /// Media type, for example `image/png`.
    pub media_type: String,
    /// Raw bytes.
    pub data: Vec<u8>,
}

/// One image request. Non-empty `sources` makes it an edit.
#[derive(Debug, Clone, Default)]
pub struct ImageRequest {
    /// Model id as the provider expects it.
    pub model: String,
    /// What to draw, or how to change the sources.
    pub prompt: String,
    /// `WIDTHxHEIGHT` or `auto`.
    pub size: Option<String>,
    /// `low`, `medium`, `high`, `xhigh`, `max` or `auto`.
    pub quality: Option<String>,
    /// `transparent`, `opaque` or `auto`.
    pub background: Option<String>,
    /// `png`, `jpeg`, `webp`, `gif` or `svg`.
    pub output_format: Option<String>,
    /// Images to return.
    pub count: u8,
    /// How much the model plans first: `low`, `medium` or `high`.
    pub complexity: Option<String>,
    /// GIF frames.
    pub frame_count: Option<u8>,
    /// GIF frame duration in milliseconds.
    pub frame_duration_ms: Option<u16>,
    /// Streamed previews for a single still image (0-3); `None` disables streaming.
    pub partial_images: Option<u8>,
    /// Edit sources. Empty for a generation.
    pub sources: Vec<ImageInput>,
    /// Transparent PNG with drawn comments on the first source (edits only).
    pub markup: Option<ImageInput>,
}

impl ImageRequest {
    /// Whether this request edits existing images.
    pub fn is_edit(&self) -> bool {
        !self.sources.is_empty()
    }
}

/// One returned image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedImage {
    /// Base64 image bytes as the API returned them.
    pub b64_json: String,
    /// The prompt the model rewrote, when it reports one.
    pub revised_prompt: Option<String>,
}

/// Token usage the API reported.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageUsage {
    /// Text prompt tokens.
    pub input_text_tokens: u32,
    /// Image prompt tokens (edit sources, markup).
    pub input_image_tokens: u32,
    /// Generated image tokens.
    pub output_tokens: u32,
}

/// Result of one image request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImageResponse {
    /// Returned images, in order.
    pub images: Vec<GeneratedImage>,
    /// Reported usage, when the provider includes it.
    pub usage: Option<ImageUsage>,
}

/// Progress while a streamed still image renders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageProgress {
    /// A preview arrived (`index` counts from 0).
    Partial { index: usize },
    /// A finished image arrived; `count` finished so far.
    Completed { count: usize },
}

/// Sender for [`ImageProgress`]; drivers ignore a closed receiver.
pub type ImageProgressSender = tokio::sync::mpsc::UnboundedSender<ImageProgress>;

/// Error from an images driver. The message is shown to the agent, so it
/// never carries credentials.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ImagesDriverError(pub String);

/// Makes images for one bound account.
#[async_trait]
pub trait ImagesDriver: Send + Sync {
    /// Generate (no sources) or edit (sources) images. Progress is sent only
    /// for streamed single still images.
    async fn create_images(
        &self,
        request: ImageRequest,
        progress: Option<ImageProgressSender>,
    ) -> Result<ImageResponse, ImagesDriverError>;
}

/// An outgoing image API call. The driver has already applied auth headers.
#[derive(Clone)]
pub struct ImagesHttpRequest {
    /// Absolute URL.
    pub url: String,
    /// Headers, auth included.
    pub headers: Vec<(String, String)>,
    /// Request body (JSON or multipart).
    pub body: Vec<u8>,
    /// Whether the response is a server-sent event stream.
    pub stream: bool,
    /// Request timeout in milliseconds.
    pub timeout_ms: u64,
}

impl std::fmt::Debug for ImagesHttpRequest {
    /// Never renders header values: they carry the provider key.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImagesHttpRequest")
            .field("url", &self.url)
            .field(
                "headers",
                &self.headers.iter().map(|(name, _)| name).collect::<Vec<_>>(),
            )
            .field("body_len", &self.body.len())
            .field("stream", &self.stream)
            .finish()
    }
}

/// Response body: whole for JSON, chunked for event streams.
pub enum ImagesHttpBody {
    /// Buffered body.
    Full(Vec<u8>),
    /// Streamed body chunks.
    Stream(BoxStream<'static, Result<Vec<u8>, String>>),
}

/// An image API response.
pub struct ImagesHttpResponse {
    /// HTTP status. A 3xx is never followed.
    pub status: u16,
    /// Body.
    pub body: ImagesHttpBody,
}

/// Sends image API calls for a driver.
#[async_trait]
pub trait ImagesTransport: Send + Sync {
    /// Send one request. Transport failures (blocked host, DNS, timeout) are
    /// errors; HTTP error statuses are responses.
    async fn send(&self, request: ImagesHttpRequest)
    -> Result<ImagesHttpResponse, ImagesDriverError>;
}
