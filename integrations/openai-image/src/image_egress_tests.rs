//! EVE-1174: image generation/edit traffic must leave through the host egress
//! boundary (network ACL, system allowlist, DNS pinning, no redirects), so an
//! org-configured provider base URL cannot reach internal addresses or forward
//! the decrypted provider key to an unapproved origin.

use super::*;
use crate::test_egress::LoopbackTestEgress;
use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_contracts::typed_id::SessionId;
use everruns_core::EgressService;
use everruns_core::connection_services::{ProviderCredentialStore, ProviderCredentials};
use everruns_core::host::DirectEgressService;
use std::sync::{Arc, Mutex};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::any};

/// Provider store whose saved base URL can be edited between calls while the
/// API key stays the same (the "admin changes base URL, key retained" case).
struct MutableProviderStore {
    base_url: Mutex<Option<String>>,
}

#[async_trait]
impl ProviderCredentialStore for MutableProviderStore {
    async fn get_default_provider_credentials(
        &self,
        provider_type: &str,
    ) -> Result<Option<ProviderCredentials>> {
        Ok((provider_type == "openai").then(|| ProviderCredentials {
            api_key: "sk-retained-secret".to_string(),
            base_url: self.base_url.lock().unwrap().clone(),
        }))
    }
}

fn context_with(store: Arc<MutableProviderStore>, egress: Arc<dyn EgressService>) -> ToolContext {
    let session_id = SessionId::new();
    ToolContext {
        provider_credential_store: Some(store),
        ..ToolContext::new(session_id).with_egress_service(egress)
    }
}

fn store(base_url: String) -> Arc<MutableProviderStore> {
    Arc::new(MutableProviderStore {
        base_url: Mutex::new(Some(base_url)),
    })
}

async fn generate(context: &ToolContext, streaming: bool) -> ToolExecutionResult {
    let tool = GenerateImageTool {
        config: json!({ "partial_images": if streaming { 1 } else { 0 } }),
    };
    tool.execute_with_context(json!({ "prompt": "otter" }), context)
        .await
}

async fn silent_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{ "b64_json": "aGVsbG8=" }]
        })))
        .expect(0)
        .mount(&server)
        .await;
    server
}

/// A provider base URL that points at an internal address (here: loopback,
/// standing in for a public hostname that resolves internally) must never
/// receive the request, for both streamed and non-streamed generations.
#[tokio::test]
async fn internal_base_url_is_denied_by_host_egress() {
    let server = silent_server().await;
    let context = context_with(
        store(format!("{}/v1", server.uri())),
        Arc::new(DirectEgressService::new()),
    );
    for streaming in [false, true] {
        let result = generate(&context, streaming).await;
        assert!(result.is_error(), "internal base URL must be refused");
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

/// An admin edits the saved base URL while the stored key is retained. Each
/// call re-resolves credentials and re-validates the new origin at the egress
/// boundary, so the retained key never reaches the new, internally-resolving
/// host; a redirect from the approved origin is not followed either.
#[tokio::test]
async fn changed_base_url_with_retained_key_is_revalidated_per_call() {
    let approved = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{ "b64_json": "aGVsbG8=" }]
        })))
        .mount(&approved)
        .await;
    let internal = silent_server().await;
    let port = reqwest::Url::parse(&internal.uri())
        .unwrap()
        .port()
        .unwrap();

    let store = store(format!("{}/v1", approved.uri()));
    let egress = LoopbackTestEgress::new();
    let context = context_with(store.clone(), egress.clone());

    // Approved origin: the key goes where the provider is configured.
    let _ = generate(&context, false).await;
    let received = approved.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    assert_eq!(
        received[0].headers.get("authorization").unwrap(),
        "Bearer sk-retained-secret"
    );

    // Saved base URL changes to a public-looking hostname that resolves to
    // a private address (the test egress answers 10.0.0.1); key retained.
    *store.base_url.lock().unwrap() = Some(format!("http://images.attacker.test:{port}/v1"));
    for streaming in [false, true] {
        let result = generate(&context, streaming).await;
        assert!(result.is_error(), "rebound base URL must be refused");
    }
    assert!(internal.received_requests().await.unwrap().is_empty());

    // The approved origin now redirects to the internal service: not followed.
    approved.reset().await;
    Mock::given(any())
        .respond_with(
            ResponseTemplate::new(307).insert_header("location", format!("{}/v1", internal.uri())),
        )
        .mount(&approved)
        .await;
    *store.base_url.lock().unwrap() = Some(format!("{}/v1", approved.uri()));
    for streaming in [false, true] {
        let result = generate(&context, streaming).await;
        assert!(result.is_error(), "redirect must not be followed");
    }
    assert_eq!(approved.received_requests().await.unwrap().len(), 2);
    assert!(internal.received_requests().await.unwrap().is_empty());
    assert!(
        egress
            .seen
            .lock()
            .unwrap()
            .iter()
            .all(|request| request.dns_pinning_required)
    );
}

/// No egress service in the context means no image traffic: there is no
/// direct-dial fallback.
#[tokio::test]
async fn missing_egress_service_fails_closed() {
    let server = silent_server().await;
    let context = ToolContext {
        provider_credential_store: Some(store(format!("{}/v1", server.uri()))),
        ..ToolContext::new(SessionId::new())
    };
    let result = generate(&context, false).await;
    assert!(
        matches!(&result, ToolExecutionResult::ToolError(message) if message.contains("egress")),
        "{result:?}"
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}
