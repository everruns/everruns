//! Standalone generation: model resolution, the square size, egress, and the
//! returned bytes.

use super::standalone::{StandaloneImageError, StandaloneImageRequest, generate_standalone_image};
use super::test_egress::LoopbackTestEgress;
use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_contracts::runtime::connection_services::{
    ImageModelBinding, ProviderCredentialStore, ProviderCredentials,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct OpenAiOnly {
    base_url: Option<String>,
}

#[async_trait]
impl ProviderCredentialStore for OpenAiOnly {
    async fn get_default_provider_credentials(
        &self,
        provider_type: &str,
    ) -> Result<Option<ProviderCredentials>> {
        Ok(
            (provider_type == "openai" && self.base_url.is_some()).then(|| ProviderCredentials {
                api_key: "sk-default".to_string(),
                base_url: self.base_url.clone(),
            }),
        )
    }
}

// 1x1 transparent PNG.
const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

async fn image_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/images/generations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{ "b64_json": PNG_B64, "revised_prompt": "a fox, flat icon" }]
        })))
        .expect(1)
        .mount(&server)
        .await;
    server
}

fn request() -> StandaloneImageRequest {
    StandaloneImageRequest {
        prompt: "a fox".to_string(),
        square: true,
        ..StandaloneImageRequest::default()
    }
}

async fn sent_body(server: &MockServer) -> Value {
    let requests = server.received_requests().await.unwrap();
    serde_json::from_slice(&requests[0].body).unwrap()
}

#[tokio::test]
async fn falls_back_to_platform_default_on_the_default_openai_account() {
    let server = image_server().await;
    let store: Arc<dyn ProviderCredentialStore> = Arc::new(OpenAiOnly {
        base_url: Some(format!("{}/v1", server.uri())),
    });
    let egress = LoopbackTestEgress::new();

    let image = generate_standalone_image(None, &store, egress.clone(), request())
        .await
        .unwrap();

    assert_eq!(image.provider_type, "openai");
    assert_eq!(image.model, "gpt-image-2.6-flare");
    assert_eq!(image.media_type, "image/png");
    assert!(image.data.starts_with(b"\x89PNG"));
    assert_eq!(image.revised_prompt.as_deref(), Some("a fox, flat icon"));
    let body = sent_body(&server).await;
    assert_eq!(body["size"], "1024x1024");
    assert_eq!(body["output_format"], "png");
    assert_eq!(body["n"], 1);
    assert!(body.get("partial_images").is_none());
    assert!(body.get("stream").is_none());
    // Through the host egress, never a direct dial.
    assert_eq!(egress.seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn the_org_default_binding_wins() {
    let server = image_server().await;
    // No default accounts: only the binding can serve.
    let store: Arc<dyn ProviderCredentialStore> = Arc::new(OpenAiOnly { base_url: None });
    let binding = ImageModelBinding {
        model_id: "model_01".to_string(),
        provider_id: "provider_01".to_string(),
        provider_type: "openai".to_string(),
        model: "gpt-image-2.6-sunburst".to_string(),
        profile_key: "gpt-image-2.6-sunburst".to_string(),
        api_key: "sk-bound".to_string(),
        base_url: Some(format!("{}/v1", server.uri())),
        headers: BTreeMap::new(),
    };

    let image =
        generate_standalone_image(Some(binding), &store, LoopbackTestEgress::new(), request())
            .await
            .unwrap();

    assert_eq!(image.model, "gpt-image-2.6-sunburst");
    assert_eq!(sent_body(&server).await["model"], "gpt-image-2.6-sunburst");
    let requests = server.received_requests().await.unwrap();
    let auth = requests[0].headers.get("authorization").unwrap();
    assert_eq!(auth.to_str().unwrap(), "Bearer sk-bound");
}

#[tokio::test]
async fn no_account_is_a_clear_error() {
    let store: Arc<dyn ProviderCredentialStore> = Arc::new(OpenAiOnly { base_url: None });
    let error = generate_standalone_image(None, &store, LoopbackTestEgress::new(), request())
        .await
        .unwrap_err();
    assert!(matches!(error, StandaloneImageError::Setup(_)), "{error:?}");
    assert!(
        error
            .to_string()
            .contains("Image credentials are not configured"),
        "{error}"
    );
}

#[tokio::test]
async fn an_unknown_quality_is_refused_before_any_call() {
    let server = MockServer::start().await;
    let store: Arc<dyn ProviderCredentialStore> = Arc::new(OpenAiOnly {
        base_url: Some(format!("{}/v1", server.uri())),
    });
    let error = generate_standalone_image(
        None,
        &store,
        LoopbackTestEgress::new(),
        StandaloneImageRequest {
            quality: Some("ultra".to_string()),
            ..request()
        },
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&error, StandaloneImageError::Setup(message) if message.contains("quality")),
        "{error:?}"
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_provider_failure_is_a_provider_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;
    let store: Arc<dyn ProviderCredentialStore> = Arc::new(OpenAiOnly {
        base_url: Some(format!("{}/v1", server.uri())),
    });
    let error = generate_standalone_image(None, &store, LoopbackTestEgress::new(), request())
        .await
        .unwrap_err();
    assert!(
        matches!(error, StandaloneImageError::Provider(_)),
        "{error:?}"
    );
}
