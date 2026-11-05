#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use std::sync::Mutex;

/// Records requests and answers with canned responses.
struct FakeTransport {
    seen: Mutex<Vec<ImagesHttpRequest>>,
    status: u16,
    body: Vec<u8>,
    stream: bool,
}

impl FakeTransport {
    fn json(status: u16, body: Value) -> Arc<Self> {
        Arc::new(Self {
            seen: Mutex::new(Vec::new()),
            status,
            body: serde_json::to_vec(&body).unwrap(),
            stream: false,
        })
    }

    fn sse(events: &[Value]) -> Arc<Self> {
        let mut body = String::new();
        for event in events {
            body.push_str(&format!("data: {event}\n\n"));
        }
        body.push_str("data: [DONE]\n\n");
        Arc::new(Self {
            seen: Mutex::new(Vec::new()),
            status: 200,
            body: body.into_bytes(),
            stream: true,
        })
    }

    fn last(&self) -> ImagesHttpRequest {
        self.seen.lock().unwrap().last().cloned().unwrap()
    }
}

#[async_trait]
impl ImagesTransport for FakeTransport {
    async fn send(
        &self,
        request: ImagesHttpRequest,
    ) -> Result<everruns_contracts::images::ImagesHttpResponse, ImagesDriverError> {
        self.seen.lock().unwrap().push(request);
        let body = if self.stream {
            // Split mid-event to prove the SSE parser reassembles chunks.
            let (a, b) = self.body.split_at(self.body.len() / 2);
            let chunks = vec![Ok(a.to_vec()), Ok(b.to_vec())];
            ImagesHttpBody::Stream(futures::stream::iter(chunks).boxed())
        } else {
            ImagesHttpBody::Full(self.body.clone())
        };
        Ok(everruns_contracts::images::ImagesHttpResponse {
            status: self.status,
            body,
        })
    }
}

fn header<'a>(request: &'a ImagesHttpRequest, name: &str) -> Option<&'a str> {
    request
        .headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn ok_body() -> Value {
    serde_json::json!({
        "data": [{"b64_json": "R0lGODlh", "revised_prompt": "a lantern"}],
        "usage": {
            "input_tokens": 37, "output_tokens": 1172,
            "input_tokens_details": {"text_tokens": 37, "image_tokens": 0}
        }
    })
}

#[tokio::test]
async fn gif_generation_sends_frames_and_reads_usage() {
    let transport = FakeTransport::json(200, ok_body());
    let driver = OpenAIImagesDriver::new(transport.clone(), "openai", "sk-test", None);
    let response = driver
        .create_images(
            ImageRequest {
                model: "gpt-image-2.6-flare".into(),
                prompt: "a lantern swaying".into(),
                output_format: Some("gif".into()),
                frame_count: Some(9),
                frame_duration_ms: Some(200),
                complexity: Some("low".into()),
                // GIFs are never streamed, even when previews are asked for.
                partial_images: Some(1),
                count: 1,
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();

    let request = transport.last();
    assert_eq!(request.url, "https://api.openai.com/v1/images/generations");
    assert_eq!(header(&request, "authorization"), Some("Bearer sk-test"));
    assert!(!request.stream);
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(
        body,
        serde_json::json!({
            "model": "gpt-image-2.6-flare", "prompt": "a lantern swaying", "n": 1,
            "output_format": "gif", "complexity": "low",
            "frame_count": 9, "frame_duration_ms": 200
        })
    );
    assert_eq!(
        response.images[0].revised_prompt.as_deref(),
        Some("a lantern")
    );
    assert_eq!(
        response.usage,
        Some(ImageUsage {
            input_text_tokens: 37,
            input_image_tokens: 0,
            output_tokens: 1172
        })
    );
}

#[tokio::test]
async fn frame_fields_are_dropped_for_stills() {
    let transport = FakeTransport::json(200, ok_body());
    let driver = OpenAIImagesDriver::new(transport.clone(), "openai", "sk", None);
    driver
        .create_images(
            ImageRequest {
                model: "gpt-image-2.6-flare".into(),
                prompt: "p".into(),
                output_format: Some("png".into()),
                frame_count: Some(9),
                count: 1,
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&transport.last().body).unwrap();
    assert!(body.get("frame_count").is_none());
}

#[tokio::test]
async fn openrouter_uses_its_images_path_and_azure_uses_api_key() {
    let transport = FakeTransport::json(200, ok_body());
    let request = || ImageRequest {
        model: "m".into(),
        prompt: "p".into(),
        count: 1,
        ..Default::default()
    };
    OpenAIImagesDriver::new(transport.clone(), "openrouter", "or-key", None)
        .create_images(request(), None)
        .await
        .unwrap();
    assert_eq!(transport.last().url, "https://openrouter.ai/api/v1/images");

    OpenAIImagesDriver::new(
        transport.clone(),
        "azure_openai",
        "az-key",
        Some("https://res.openai.azure.com/openai/v1/".into()),
    )
    .with_headers([("x-extra".to_string(), "1".to_string())])
    .create_images(request(), None)
    .await
    .unwrap();
    let sent = transport.last();
    assert_eq!(
        sent.url,
        "https://res.openai.azure.com/openai/v1/images/generations"
    );
    assert_eq!(header(&sent, "api-key"), Some("az-key"));
    assert_eq!(header(&sent, "authorization"), None);
    assert_eq!(header(&sent, "x-extra"), Some("1"));
}

#[tokio::test]
async fn edit_sends_sources_and_markup_as_multipart() {
    let transport = FakeTransport::json(200, ok_body());
    let driver = OpenAIImagesDriver::new(transport.clone(), "openai", "sk", None);
    driver
        .create_images(
            ImageRequest {
                model: "gpt-image-2.6-sunburst".into(),
                prompt: "recolor".into(),
                count: 1,
                sources: vec![ImageInput {
                    filename: "kitchen.webp".into(),
                    media_type: "image/webp".into(),
                    data: vec![1, 2, 3],
                }],
                markup: Some(ImageInput {
                    filename: "overlay.png".into(),
                    media_type: "image/png".into(),
                    data: vec![4, 5],
                }),
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();
    let sent = transport.last();
    assert_eq!(sent.url, "https://api.openai.com/v1/images/edits");
    assert!(
        header(&sent, "content-type")
            .unwrap()
            .starts_with("multipart/form-data")
    );
    let body = String::from_utf8_lossy(&sent.body);
    assert!(body.contains("name=\"image\"; filename=\"kitchen.webp\""));
    assert!(body.contains("name=\"markup\"; filename=\"overlay.png\""));
}

#[tokio::test]
async fn streamed_still_reports_progress() {
    let transport = FakeTransport::sse(&[
        serde_json::json!({"type": "image_generation.partial_image", "partial_image_index": 0, "b64_json": "AA=="}),
        serde_json::json!({"type": "image_generation.completed", "b64_json": "iVBORw0K", "usage": {"input_tokens": 5, "output_tokens": 9}}),
    ]);
    let driver = OpenAIImagesDriver::new(transport.clone(), "openai", "sk", None);
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let response = driver
        .create_images(
            ImageRequest {
                model: "gpt-image-2".into(),
                prompt: "p".into(),
                count: 1,
                partial_images: Some(1),
                ..Default::default()
            },
            Some(sender),
        )
        .await
        .unwrap();
    assert!(transport.last().stream);
    assert_eq!(response.images[0].b64_json, "iVBORw0K");
    assert_eq!(response.usage.unwrap().output_tokens, 9);
    assert_eq!(
        receiver.recv().await,
        Some(ImageProgress::Partial { index: 0 })
    );
    assert_eq!(
        receiver.recv().await,
        Some(ImageProgress::Completed { count: 1 })
    );
}

#[tokio::test]
async fn redirects_and_errors_surface_without_the_key() {
    let transport = FakeTransport::json(302, serde_json::json!({}));
    let error = OpenAIImagesDriver::new(transport, "openai", "sk-secret", None)
        .create_images(
            ImageRequest {
                model: "m".into(),
                prompt: "p".into(),
                count: 1,
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap_err();
    assert!(error.0.contains("redirect 302"));
    assert!(!error.0.contains("sk-secret"));

    let transport = FakeTransport::json(400, serde_json::json!({"error": {"message": "bad size"}}));
    let error = OpenAIImagesDriver::new(transport, "openai", "sk", None)
        .create_images(
            ImageRequest {
                model: "m".into(),
                prompt: "p".into(),
                count: 1,
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap_err();
    assert!(error.0.contains("400") && error.0.contains("bad size"));
}

#[test]
fn endpoint_url_strips_pasted_suffixes() {
    assert_eq!(
        endpoint_url(
            "https://api.openai.com/v1/images/generations",
            "images/edits"
        )
        .unwrap(),
        "https://api.openai.com/v1/images/edits"
    );
    assert!(endpoint_url("not a url", "images").is_err());
}
