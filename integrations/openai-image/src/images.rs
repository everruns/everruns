use anyhow::{Context, Result, anyhow};
use eventsource_stream::Eventsource;
use everruns_core::network_access::NetworkAccessList;
use everruns_core::{
    EgressByteStream, EgressError, EgressRequest, EgressRequestKind, EgressService,
};
use futures::{StreamExt, future::Future};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_OPENAI_IMAGE_TIMEOUT_SECS: u64 = 300;
const OPENAI_IMAGE_TIMEOUT_ENV: &str = "OPENAI_IMAGE_TIMEOUT_SECS";
/// Error bodies are echoed into tool errors; cap them so a hostile endpoint
/// cannot flood the transcript.
const MAX_ERROR_BODY_CHARS: usize = 2048;

/// Client for OpenAI-compatible image APIs.
///
/// THREAT[TM-LLM-047]: the base URL is org-configurable and the request
/// carries the decrypted provider key, so every request goes through the
/// host [`EgressService`] (EVE-1174): the merged network ACL and system
/// allowlist apply, DNS is resolved and pinned inside the egress boundary
/// (`require_dns_pinning`), and redirects are never followed. A 3xx is a
/// hard error, so the key is only ever sent to the configured, validated
/// origin. There is no direct-dial fallback.
#[derive(Clone)]
pub struct OpenAiImageClient {
    egress: Arc<dyn EgressService>,
    network_access: Option<NetworkAccessList>,
    timeout_ms: u64,
    api_key: String,
    base_url: Option<String>,
    /// Path appended to the base URL for text-to-image generations.
    /// Defaults to `"images/generations"` (OpenAI and Meta Model API);
    /// providers with a different surface (e.g. OpenRouter's `"images"`)
    /// override it via [`OpenAiImageClient::with_images_path`].
    images_path: String,
}

impl std::fmt::Debug for OpenAiImageClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiImageClient")
            .field("egress", &self.egress.name())
            .field("base_url", &self.base_url)
            .field("images_path", &self.images_path)
            .finish_non_exhaustive()
    }
}

impl OpenAiImageClient {
    pub fn new(
        egress: Arc<dyn EgressService>,
        network_access: Option<NetworkAccessList>,
        api_key: impl Into<String>,
        base_url: Option<String>,
    ) -> Self {
        let timeout_secs = std::env::var(OPENAI_IMAGE_TIMEOUT_ENV)
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_OPENAI_IMAGE_TIMEOUT_SECS);
        Self {
            egress,
            network_access,
            timeout_ms: timeout_secs.saturating_mul(1000),
            api_key: api_key.into(),
            base_url,
            images_path: "images/generations".to_string(),
        }
    }

    /// Override the generations endpoint path for providers whose image API
    /// lives at a different path than OpenAI's `images/generations`.
    pub fn with_images_path(mut self, path: impl Into<String>) -> Self {
        self.images_path = path.into();
        self
    }

    pub async fn generate(&self, request: GenerateImageRequest) -> Result<ImageApiResponse> {
        let request = self.json_request(&self.images_path, &request)?;
        let response = self
            .egress
            .send(request)
            .await
            .map_err(|error| egress_error("image generation", error))?;
        ensure_success(response.status, &response.body)?;
        serde_json::from_slice::<ImageApiResponse>(&response.body)
            .context("failed to decode OpenAI image API response")
    }

    pub async fn generate_with_events<F, Fut>(
        &self,
        request: GenerateImageRequest,
        on_event: F,
    ) -> Result<ImageApiResponse>
    where
        F: FnMut(ImageApiStreamEvent) -> Fut + Send,
        Fut: Future<Output = ()> + Send,
    {
        let request = self.json_request(&self.images_path, &request)?;
        let body = self.send_stream(request, "image generation").await?;
        parse_image_stream(body, on_event).await
    }

    pub async fn edit(&self, request: EditImageRequest) -> Result<ImageApiResponse> {
        let request = self.multipart_request(request)?;
        let response = self
            .egress
            .send(request)
            .await
            .map_err(|error| egress_error("image edit", error))?;
        ensure_success(response.status, &response.body)?;
        serde_json::from_slice::<ImageApiResponse>(&response.body)
            .context("failed to decode OpenAI image API response")
    }

    pub async fn edit_with_events<F, Fut>(
        &self,
        request: EditImageRequest,
        on_event: F,
    ) -> Result<ImageApiResponse>
    where
        F: FnMut(ImageApiStreamEvent) -> Fut + Send,
        Fut: Future<Output = ()> + Send,
    {
        let request = self.multipart_request(request)?;
        let body = self.send_stream(request, "image edit").await?;
        parse_image_stream(body, on_event).await
    }

    fn json_request<T: Serialize>(&self, endpoint: &str, payload: &T) -> Result<EgressRequest> {
        let body = serde_json::to_vec(payload).context("failed to encode image request")?;
        Ok(self
            .egress_request(endpoint)?
            .header("content-type", "application/json")
            .body(body))
    }

    fn multipart_request(&self, request: EditImageRequest) -> Result<EgressRequest> {
        let (content_type, body) = build_edit_image_form(request)?;
        Ok(self
            .egress_request("images/edits")?
            .header("content-type", content_type)
            .body(body))
    }

    fn egress_request(&self, endpoint: &str) -> Result<EgressRequest> {
        let url = image_endpoint_url(self.base_url.as_deref(), endpoint)?;
        let (auth_header, auth_value) =
            if everruns_contracts::openai_protocol::is_azure_openai_api_url(url.as_str()) {
                ("api-key", self.api_key.clone())
            } else {
                ("authorization", format!("Bearer {}", self.api_key))
            };
        Ok(
            EgressRequest::new("POST", url.as_str(), EgressRequestKind::Capability)
                .network_access(self.network_access.clone())
                .require_dns_pinning()
                .timeout_ms(self.timeout_ms)
                .header(auth_header, auth_value),
        )
    }

    async fn send_stream(&self, request: EgressRequest, action: &str) -> Result<EgressByteStream> {
        let response = self
            .egress
            .send_stream(request)
            .await
            .map_err(|error| egress_error(action, error))?;
        if !(200..300).contains(&response.status) {
            let mut body = Vec::new();
            let mut stream = response.body;
            while let Some(chunk) = stream.next().await {
                let Ok(chunk) = chunk else { break };
                body.extend_from_slice(&chunk);
                if body.len() > MAX_ERROR_BODY_CHARS * 4 {
                    break;
                }
            }
            return Err(status_error(response.status, &body));
        }
        Ok(response.body)
    }
}

fn egress_error(action: &str, error: EgressError) -> anyhow::Error {
    match error {
        EgressError::NetworkAccessDenied { .. } => {
            anyhow!("OpenAI {action} request blocked by network access policy: {error}")
        }
        other => anyhow!("failed to call OpenAI {action} API: {other}"),
    }
}

/// Reject non-2xx responses. Redirects are never followed by the egress
/// boundary; surfacing them as errors keeps the provider key on the
/// configured origin (EVE-1174).
fn ensure_success(status: u16, body: &[u8]) -> Result<()> {
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(status_error(status, body))
    }
}

fn status_error(status: u16, body: &[u8]) -> anyhow::Error {
    if (300..400).contains(&status) {
        return anyhow!(
            "OpenAI image API returned redirect {status}; redirects are not followed for image providers"
        );
    }
    let body: String = String::from_utf8_lossy(body)
        .chars()
        .take(MAX_ERROR_BODY_CHARS)
        .collect();
    anyhow!("OpenAI image API returned {status}: {body}")
}

#[derive(Debug, Clone, Serialize)]
pub struct GenerateImageRequest {
    pub model: String,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "output_format")]
    pub output_format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partial_images: Option<u8>,
    #[serde(rename = "n")]
    pub count: usize,
}

#[derive(Debug, Clone)]
pub struct EditImageRequest {
    pub model: String,
    pub prompt: String,
    pub images: Vec<EditImageInput>,
    pub size: Option<String>,
    pub quality: Option<String>,
    pub background: Option<String>,
    pub output_format: Option<String>,
    pub stream: Option<bool>,
    pub partial_images: Option<u8>,
    pub count: usize,
}

#[derive(Debug, Clone)]
pub struct EditImageInput {
    pub filename: String,
    pub content_type: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImageApiResponse {
    pub data: Vec<ImageApiImage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImageApiImage {
    pub b64_json: String,
    #[serde(default)]
    pub revised_prompt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageApiStreamEvent {
    PartialImage {
        partial_image_index: usize,
        b64_json: Option<String>,
        revised_prompt: Option<String>,
    },
    Completed {
        completed_image_count: usize,
    },
}

/// Encode the edit request as `multipart/form-data`.
///
/// The body is built in memory (instead of `reqwest::multipart`) because the
/// egress boundary carries a byte body; returns `(content_type, body)`.
fn build_edit_image_form(request: EditImageRequest) -> Result<(String, Vec<u8>)> {
    let image_field_name = if request.images.len() > 1 {
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
    text("n", &request.count.to_string());
    if let Some(size) = &request.size {
        text("size", size);
    }
    if let Some(quality) = &request.quality {
        text("quality", quality);
    }
    if let Some(background) = &request.background {
        text("background", background);
    }
    if let Some(output_format) = &request.output_format {
        text("output_format", output_format);
    }
    if let Some(stream) = request.stream {
        text("stream", &stream.to_string());
    }
    if let Some(partial_images) = request.partial_images {
        text("partial_images", &partial_images.to_string());
    }

    for image in request.images {
        let content_type = image.content_type.trim();
        if !content_type.contains('/')
            || content_type
                .chars()
                .any(|c| c.is_control() || c == '"' || c == ';')
        {
            return Err(anyhow!("invalid source image content type"));
        }
        let filename = sanitize_multipart_filename(&image.filename);
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{image_field_name}\"; filename=\"{filename}\"\r\nContent-Type: {content_type}\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(&image.data);
        body.extend_from_slice(b"\r\n");
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
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default(),
        );
        out.push_str(&format!("{:016x}", hasher.finish()));
    }
    out
}

/// Keep header-breaking characters out of the `filename` parameter.
fn sanitize_multipart_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '"' | '\\' | '\r' | '\n' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect()
}

fn image_endpoint_url(base_url: Option<&str>, endpoint: &str) -> Result<Url> {
    let mut normalized = base_url
        .unwrap_or(DEFAULT_OPENAI_BASE_URL)
        .trim_end_matches('/');

    for suffix in [
        "/responses",
        "/chat/completions",
        "/images/generations",
        "/images/edits",
    ] {
        if let Some(prefix) = normalized.strip_suffix(suffix) {
            normalized = prefix;
            break;
        }
    }

    Url::parse(&format!(
        "{}/{}",
        normalized,
        endpoint.trim_start_matches('/')
    ))
    .map_err(|error| anyhow!("invalid OpenAI image API URL: {error}"))
}

async fn parse_image_stream<F, Fut>(
    body: EgressByteStream,
    mut on_event: F,
) -> Result<ImageApiResponse>
where
    F: FnMut(ImageApiStreamEvent) -> Fut + Send,
    Fut: Future<Output = ()> + Send,
{
    let mut images = Vec::new();
    let mut event_stream = body.eventsource();

    while let Some(event) = event_stream.next().await {
        let event = event.context("failed to read OpenAI image stream event")?;
        if event.data == "[DONE]" {
            break;
        }

        let payload: Value = serde_json::from_str(&event.data)
            .context("failed to decode OpenAI image stream event")?;
        let Some(event_type) = payload.get("type").and_then(Value::as_str) else {
            tracing::debug!("Ignoring OpenAI image stream payload without type");
            continue;
        };

        match event_type {
            "image_generation.partial_image" => {
                let partial_image_index = payload
                    .get("partial_image_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let b64_json = payload
                    .get("b64_json")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                let revised_prompt = payload
                    .get("revised_prompt")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                on_event(ImageApiStreamEvent::PartialImage {
                    partial_image_index,
                    b64_json,
                    revised_prompt,
                })
                .await;
            }
            "image_generation.completed" => {
                let image = serde_json::from_value::<ImageApiImage>(payload)
                    .context("failed to decode OpenAI completed image event")?;
                images.push(image);
                on_event(ImageApiStreamEvent::Completed {
                    completed_image_count: images.len(),
                })
                .await;
            }
            other => {
                tracing::debug!(
                    event_type = other,
                    "Ignoring unknown OpenAI image stream event"
                );
            }
        }
    }

    if images.is_empty() {
        return Err(anyhow!(
            "OpenAI image stream ended before any completed image events were received"
        ));
    }

    Ok(ImageApiResponse { data: images })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_egress::{BLOCKED_ANSWERS, LoopbackTestEgress, rebinding_egress};
    use everruns_core::network_access::NetworkAccessList;
    use wiremock::matchers::{any, body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_client(api_key: &str, base_url: Option<String>) -> OpenAiImageClient {
        OpenAiImageClient::new(LoopbackTestEgress::new(), None, api_key, base_url)
    }

    fn generate_request(stream: bool) -> GenerateImageRequest {
        GenerateImageRequest {
            model: "gpt-image-2".to_string(),
            prompt: "otter".to_string(),
            size: None,
            quality: None,
            background: None,
            output_format: None,
            stream: stream.then_some(true),
            partial_images: stream.then_some(1),
            count: 1,
        }
    }

    fn edit_request(stream: bool) -> EditImageRequest {
        EditImageRequest {
            model: "gpt-image-2".to_string(),
            prompt: "edit it".to_string(),
            images: vec![EditImageInput {
                filename: "source.png".to_string(),
                content_type: "image/png".to_string(),
                data: vec![1, 2, 3],
            }],
            size: None,
            quality: None,
            background: None,
            output_format: None,
            stream: stream.then_some(true),
            partial_images: stream.then_some(1),
            count: 1,
        }
    }

    /// Run all four request shapes (generate/edit x buffered/streamed).
    async fn run_all(client: &OpenAiImageClient) -> Vec<Result<ImageApiResponse>> {
        vec![
            client.generate(generate_request(false)).await,
            client
                .generate_with_events(generate_request(true), |_| async {})
                .await,
            client.edit(edit_request(false)).await,
            client
                .edit_with_events(edit_request(true), |_| async {})
                .await,
        ]
    }

    #[tokio::test]
    async fn generate_uses_bearer_auth_and_json_payload() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_json(serde_json::json!({
                "model": "gpt-image-1",
                "prompt": "otter",
                "size": "1024x1024",
                "quality": "high",
                "background": "transparent",
                "output_format": "png",
                "n": 1
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"b64_json": "aGVsbG8=", "revised_prompt": "otter"}]
            })))
            .mount(&server)
            .await;

        let client = test_client("sk-test", Some(format!("{}/v1", server.uri())));
        let response = client
            .generate(GenerateImageRequest {
                model: "gpt-image-1".to_string(),
                prompt: "otter".to_string(),
                size: Some("1024x1024".to_string()),
                quality: Some("high".to_string()),
                background: Some("transparent".to_string()),
                output_format: Some("png".to_string()),
                stream: None,
                partial_images: None,
                count: 1,
            })
            .await
            .unwrap();

        assert_eq!(response.data.len(), 1);
        assert_eq!(response.data[0].b64_json, "aGVsbG8=");
    }

    #[tokio::test]
    async fn generate_uses_custom_images_path() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images"))
            .and(header("authorization", "Bearer or-key"))
            .and(body_json(serde_json::json!({
                "model": "meta/muse-image",
                "prompt": "otter",
                "n": 1
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"b64_json": "aGVsbG8="}]
            })))
            .mount(&server)
            .await;

        let client =
            test_client("or-key", Some(format!("{}/v1", server.uri()))).with_images_path("images");
        let response = client
            .generate(GenerateImageRequest {
                model: "meta/muse-image".to_string(),
                prompt: "otter".to_string(),
                size: None,
                quality: None,
                background: None,
                output_format: None,
                stream: None,
                partial_images: None,
                count: 1,
            })
            .await
            .unwrap();

        assert_eq!(response.data.len(), 1);
        assert_eq!(response.data[0].b64_json, "aGVsbG8=");
    }

    #[tokio::test]
    async fn edit_uses_multipart_endpoint_with_custom_base_url() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/openai/v1/images/edits"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"b64_json": "aGVsbG8="}]
            })))
            .mount(&server)
            .await;

        let client = test_client("azure-key", Some(format!("{}/openai/v1", server.uri())));

        let response = client
            .edit(EditImageRequest {
                model: "gpt-image-1".to_string(),
                prompt: "edit it".to_string(),
                images: vec![EditImageInput {
                    filename: "source.png".to_string(),
                    content_type: "image/png".to_string(),
                    data: vec![1, 2, 3],
                }],
                size: Some("1024x1024".to_string()),
                quality: Some("medium".to_string()),
                background: Some("opaque".to_string()),
                output_format: Some("png".to_string()),
                stream: None,
                partial_images: None,
                count: 1,
            })
            .await
            .unwrap();

        assert_eq!(response.data.len(), 1);
        let received = server.received_requests().await.unwrap();
        let content_type = received[0]
            .headers
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap()
            .to_string();
        let boundary = content_type
            .strip_prefix("multipart/form-data; boundary=")
            .expect("multipart content type");
        let body = String::from_utf8_lossy(&received[0].body);
        assert!(body.contains("name=\"model\"\r\n\r\ngpt-image-1\r\n"));
        assert!(body.contains("name=\"quality\"\r\n\r\nmedium\r\n"));
        assert!(body.contains(
            "name=\"image\"; filename=\"source.png\"\r\nContent-Type: image/png\r\n\r\n\u{1}\u{2}\u{3}\r\n"
        ));
        assert!(body.ends_with(&format!("--{boundary}--\r\n")));
    }

    #[tokio::test]
    async fn generate_with_events_streams_partial_and_completed_images() {
        let server = MockServer::start().await;
        let stream_body = concat!(
            "data: {\"type\":\"image_generation.partial_image\",\"partial_image_index\":0,\"b64_json\":\"cHJldmlldw==\",\"revised_prompt\":\"otter sketch\"}\n\n",
            "data: {\"type\":\"image_generation.completed\",\"b64_json\":\"aGVsbG8=\",\"revised_prompt\":\"otter\"}\n\n",
            "data: [DONE]\n\n"
        );
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(stream_body),
            )
            .mount(&server)
            .await;

        let client = test_client("sk-test", Some(format!("{}/v1", server.uri())));
        let mut events = Vec::new();
        let response = client
            .generate_with_events(
                GenerateImageRequest {
                    model: "gpt-image-2".to_string(),
                    prompt: "otter".to_string(),
                    size: Some("1024x1024".to_string()),
                    quality: Some("medium".to_string()),
                    background: Some("opaque".to_string()),
                    output_format: Some("png".to_string()),
                    stream: Some(true),
                    partial_images: Some(1),
                    count: 1,
                },
                |event| {
                    events.push(event);
                    async {}
                },
            )
            .await
            .unwrap();

        assert_eq!(
            events,
            vec![
                ImageApiStreamEvent::PartialImage {
                    partial_image_index: 0,
                    b64_json: Some("cHJldmlldw==".to_string()),
                    revised_prompt: Some("otter sketch".to_string()),
                },
                ImageApiStreamEvent::Completed {
                    completed_image_count: 1
                },
            ]
        );
        assert_eq!(response.data.len(), 1);
        assert_eq!(response.data[0].revised_prompt.as_deref(), Some("otter"));
    }

    #[tokio::test]
    async fn edit_with_events_streams_partial_and_completed_images() {
        let server = MockServer::start().await;
        let stream_body = concat!(
            "data: {\"type\":\"image_generation.partial_image\",\"partial_image_index\":0,\"b64_json\":\"cHJldmlldw==\"}\n\n",
            "data: {\"type\":\"image_generation.completed\",\"b64_json\":\"ZmluYWw=\",\"revised_prompt\":\"edited otter\"}\n\n",
            "data: [DONE]\n\n"
        );
        Mock::given(method("POST"))
            .and(path("/v1/images/edits"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(stream_body),
            )
            .mount(&server)
            .await;

        let client = test_client("sk-test", Some(format!("{}/v1", server.uri())));
        let mut events = Vec::new();
        let response = client
            .edit_with_events(
                EditImageRequest {
                    model: "gpt-image-2".to_string(),
                    prompt: "edit otter".to_string(),
                    images: vec![EditImageInput {
                        filename: "source.png".to_string(),
                        content_type: "image/png".to_string(),
                        data: vec![1, 2, 3],
                    }],
                    size: Some("1024x1024".to_string()),
                    quality: Some("medium".to_string()),
                    background: Some("opaque".to_string()),
                    output_format: Some("png".to_string()),
                    stream: Some(true),
                    partial_images: Some(1),
                    count: 1,
                },
                |event| {
                    events.push(event);
                    async {}
                },
            )
            .await
            .unwrap();

        assert_eq!(
            events,
            vec![
                ImageApiStreamEvent::PartialImage {
                    partial_image_index: 0,
                    b64_json: Some("cHJldmlldw==".to_string()),
                    revised_prompt: None,
                },
                ImageApiStreamEvent::Completed {
                    completed_image_count: 1
                },
            ]
        );
        assert_eq!(response.data.len(), 1);
        assert_eq!(response.data[0].b64_json, "ZmluYWw=");
        assert_eq!(
            response.data[0].revised_prompt.as_deref(),
            Some("edited otter")
        );
    }

    /// EVE-1174: every image request asks the egress boundary for DNS pinning
    /// and carries the session's network ACL and the image timeout.
    #[tokio::test]
    async fn requests_require_dns_pinning_and_carry_network_policy() {
        let egress = LoopbackTestEgress::new();
        let acl = NetworkAccessList::allow_only(["images.example.com"]);
        let client = OpenAiImageClient::new(
            egress.clone(),
            Some(acl.clone()),
            "sk-test",
            Some("https://images.example.com/v1".to_string()),
        );
        let _ = run_all(&client).await;

        let seen = egress.seen.lock().unwrap();
        assert_eq!(seen.len(), 4);
        for request in seen.iter() {
            assert!(request.dns_pinning_required);
            assert_eq!(request.network_access.as_ref(), Some(&acl));
            assert_eq!(request.timeout_ms, Some(300_000));
            assert_eq!(request.kind, EgressRequestKind::Capability);
        }
    }

    /// EVE-1174: a base URL whose hostname resolves (or is rebound) to a
    /// private, loopback, link-local, or metadata address is refused before
    /// connect, for generations and edits, buffered and streamed. The mock
    /// stands in for the internal service and must see nothing.
    #[tokio::test]
    async fn rebound_dns_to_internal_address_is_refused_before_connect() {
        let internal = MockServer::start().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&internal)
            .await;
        let port = reqwest::Url::parse(&internal.uri())
            .unwrap()
            .port()
            .unwrap();

        for answer in BLOCKED_ANSWERS {
            let client = OpenAiImageClient::new(
                Arc::new(rebinding_egress(answer.parse().unwrap())),
                None,
                "sk-secret",
                Some(format!("http://images.rebind.test:{port}/v1")),
            );
            for result in run_all(&client).await {
                let error = format!("{:#}", result.unwrap_err());
                assert!(
                    error.contains("blocked by network access policy"),
                    "{answer} must be denied: {error}"
                );
                assert!(!error.contains("sk-secret"));
            }
        }
        assert!(internal.received_requests().await.unwrap().is_empty());
    }

    /// EVE-1174: an IP-literal internal base URL (e.g. cloud metadata) is
    /// refused by the egress boundary without any DNS involvement.
    #[tokio::test]
    async fn internal_ip_literal_base_url_is_refused() {
        for base_url in [
            "http://169.254.169.254/v1",
            "http://127.0.0.1:9/v1",
            "http://[::1]:9/v1",
            "http://10.1.2.3/v1",
        ] {
            let client = OpenAiImageClient::new(
                Arc::new(everruns_core::host::DirectEgressService::new()),
                None,
                "sk-secret",
                Some(base_url.to_string()),
            );
            for result in run_all(&client).await {
                let error = format!("{:#}", result.unwrap_err());
                assert!(
                    error.contains("blocked by network access policy"),
                    "{base_url}: {error}"
                );
            }
        }
    }

    /// EVE-1174: a provider that answers with a redirect never gets it
    /// followed, so the key is not forwarded to the redirect target.
    #[tokio::test]
    async fn redirects_are_not_followed_and_key_is_not_forwarded() {
        let target = MockServer::start().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"b64_json": "aGVsbG8="}]
            })))
            .expect(0)
            .mount(&target)
            .await;
        let provider = MockServer::start().await;
        for status in [301, 302, 307, 308] {
            provider.reset().await;
            Mock::given(any())
                .respond_with(
                    ResponseTemplate::new(status)
                        .insert_header("location", format!("{}/v1/steal", target.uri())),
                )
                .mount(&provider)
                .await;
            let client = test_client("sk-secret", Some(format!("{}/v1", provider.uri())));
            for result in run_all(&client).await {
                let error = format!("{:#}", result.unwrap_err());
                assert!(error.contains("redirect"), "{status}: {error}");
            }
            assert_eq!(provider.received_requests().await.unwrap().len(), 4);
        }
        assert!(target.received_requests().await.unwrap().is_empty());
    }

    /// The per-session network ACL applies to the configured base URL.
    #[tokio::test]
    async fn network_access_policy_blocks_unlisted_base_url() {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let client = OpenAiImageClient::new(
            LoopbackTestEgress::new(),
            Some(NetworkAccessList::allow_only(["api.openai.com"])),
            "sk-secret",
            Some(format!("{}/v1", server.uri())),
        );
        for result in run_all(&client).await {
            let error = format!("{:#}", result.unwrap_err());
            assert!(
                error.contains("blocked by network access policy"),
                "{error}"
            );
        }
    }

    #[test]
    fn multipart_rejects_header_injection() {
        let mut request = edit_request(false);
        request.images[0].content_type = "image/png\r\nX-Evil: 1".to_string();
        assert!(build_edit_image_form(request).is_err());

        let mut request = edit_request(false);
        request.images[0].filename = "a\"\r\nX: y.png".to_string();
        let (_, body) = build_edit_image_form(request).unwrap();
        assert!(String::from_utf8_lossy(&body).contains("filename=\"a___X: y.png\""));
    }
}
