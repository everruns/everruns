use super::*;
use crate::EventListener;
use crate::events::{Event, EventContext, LlmGenerationData};
use crate::host::observability::{OtelEventListener, TraceConventions};
use everruns_contracts::typed_id::SessionId;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn event_spans_reach_an_otlp_http_collector_and_flush() {
    everruns_contracts::install_default_crypto_provider();
    let collector = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/traces"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&collector)
        .await;
    let (provider, _) = build_otlp_tracer(&collector.uri(), Resource::builder().build()).unwrap();
    let listener =
        OtelEventListener::with_tracer_provider(provider.clone(), false, TraceConventions::ALL);
    let event = Event::new(
        SessionId::new(),
        EventContext::empty(),
        LlmGenerationData::failure(
            Vec::new(),
            Vec::new(),
            "synthetic-model".into(),
            Some("openai".into()),
            "SYNTHETIC_PRIVATE_ERROR".into(),
            Some(5),
            None,
        ),
    );
    listener.on_event(&event).await;
    listener.flush().await.unwrap();
    let requests = collector.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let body = &requests[0].body;
    assert!(
        body.windows(b"chat synthetic-model".len())
            .any(|w| w == b"chat synthetic-model")
    );
    assert!(
        !body
            .windows(b"SYNTHETIC_PRIVATE_ERROR".len())
            .any(|w| w == b"SYNTHETIC_PRIVATE_ERROR")
    );
    assert_eq!(
        requests[0].headers["content-type"],
        "application/x-protobuf"
    );
    let guard = TelemetryGuard {
        _provider: Some(provider),
    };
    assert!(guard.tracer_provider().is_some());
    guard.shutdown().await.unwrap();
}
