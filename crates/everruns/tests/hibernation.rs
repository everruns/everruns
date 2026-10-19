//! A parked retry must leave no live executor or process-local continuation.
use async_trait::async_trait;
use everruns::hosting::{AgentBuild, MemoryHosting, SessionHosting};
use everruns::{
    Agent, AgentLoopError, ChatDriver, Engine, LlmCallConfig, LlmResponseStream, LlmStreamEvent,
    MessageId, Provider, ProviderEndpoint, SessionId,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

#[path = "support/session_hosting.rs"]
mod session_hosting;
use session_hosting::External;

#[derive(Clone)]
struct Flaky {
    calls: Arc<AtomicUsize>,
    failures: usize,
}

#[async_trait]
impl ChatDriver for Flaky {
    async fn chat_completion_stream(
        &self,
        _: &ProviderEndpoint,
        _: Vec<everruns::llm::Message>,
        _: &LlmCallConfig,
    ) -> Result<LlmResponseStream, AgentLoopError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) < self.failures {
            return Err(AgentLoopError::llm_kind(
                everruns_contracts::error::LlmErrorKind::Unavailable,
                "temporary failure",
            )
            .with_retry_after_secs(1));
        }
        Ok(Box::pin(futures::stream::iter([
            Ok(LlmStreamEvent::TextDelta("recovered".into())),
            Ok(LlmStreamEvent::Done(Box::default())),
        ])))
    }
}

fn build() -> AgentBuild {
    AgentBuild::new("hibernation", "v1")
}
fn engine(profile: impl SessionHosting, calls: Arc<AtomicUsize>, failures: usize) -> Engine {
    Engine::builder()
        .hosting(profile)
        .register(
            build(),
            Agent::builder()
                .instructions("Answer briefly.")
                .provider(Provider::new("flaky", Flaky { calls, failures }))
                .model("test")
                .build()
                .unwrap(),
        )
        .build()
}

async fn exercise(profile: impl SessionHosting + Clone, failures: usize) {
    let calls = Arc::new(AtomicUsize::new(0));
    let first = engine(External(profile.clone()), calls.clone(), failures);
    let id = SessionId::new();
    let session = Box::pin(first.create_registered(id, &build()))
        .await
        .unwrap();
    let message = MessageId::new();
    session.send_with_id(message, "hello").await.unwrap();
    tokio::time::timeout(Duration::from_millis(500), Box::pin(first.activate(id)))
        .await
        .expect("a retry must park instead of holding the invocation")
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let status = profile.status(id).await.unwrap();
    assert!(!status.executing);
    assert!(!status.runnable);
    assert!(status.wake_at.unwrap() > status.now_ms);
    assert_eq!(
        status.wake(),
        everruns::hosting::WakeState::At(status.wake_at.unwrap())
    );
    assert!(status.open_turn.is_some());
    Box::pin(first.activate(id)).await.unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "early activation must not send"
    );
    drop(session);
    drop(first);

    for expected in 2..=3 {
        let status = profile.status(id).await.unwrap();
        let remaining = status.wake_at.unwrap() - status.now_ms;
        tokio::time::sleep(Duration::from_millis(remaining.max(0) as u64 + 20)).await;
        let replacement = engine(External(profile.clone()), calls.clone(), failures);
        tokio::time::timeout(
            Duration::from_millis(500),
            Box::pin(replacement.activate(id)),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), expected);
        if failures == 1 || expected == 3 {
            let session = replacement.resume(id).await.unwrap();
            let turn = session
                .admission(message)
                .await
                .unwrap()
                .unwrap()
                .wait()
                .await
                .unwrap();
            assert_eq!(turn.success, failures == 1);
            assert!(!profile.status(id).await.unwrap().executing);
            return;
        }
    }
    unreachable!();
}

#[tokio::test]
async fn cancellation_interrupts_a_parked_retry_without_another_model_call() {
    let profile = MemoryHosting::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let host = engine(External(profile.clone()), calls.clone(), usize::MAX);
    let id = SessionId::new();
    let session = Box::pin(host.create_registered(id, &build()))
        .await
        .unwrap();
    let admission = session.send("hello").await.unwrap();
    Box::pin(host.activate(id)).await.unwrap();
    session
        .admission(admission.message_id.parse().unwrap())
        .await
        .unwrap()
        .unwrap()
        .cancel(uuid::Uuid::new_v4())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_millis(500), Box::pin(host.activate(id)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let status = profile.status(id).await.unwrap();
    assert!(!status.executing);
    assert!(status.wake_at.is_none());
    assert_eq!(status.wake(), everruns::hosting::WakeState::Dormant);
    assert!(
        session
            .events_after(0)
            .await
            .unwrap()
            .iter()
            .any(|event| event.event_type() == "turn.cancelled")
    );
}

#[tokio::test]
async fn the_default_engine_retries_without_an_external_host() {
    let calls = Arc::new(AtomicUsize::new(0));
    let agent = Agent::builder()
        .instructions("Answer briefly.")
        .provider(Provider::new(
            "flaky",
            Flaky {
                calls: calls.clone(),
                failures: 1,
            },
        ))
        .model("test")
        .build()
        .unwrap();
    let turn = everruns::InMemoryEngine::new()
        .create(agent)
        .run("hello")
        .await
        .unwrap();
    assert!(turn.success);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[derive(Clone)]
struct LimitedHttp(Arc<AtomicUsize>);
#[async_trait]
impl everruns_contracts::http_transport::HttpTransport for LimitedHttp {
    async fn execute(
        &self,
        _: everruns_contracts::http_transport::HttpRequest,
    ) -> Result<
        everruns_contracts::http_transport::HttpResponse,
        everruns_contracts::http_transport::HttpError,
    > {
        use everruns_contracts::http_transport::*;
        self.0.fetch_add(1, Ordering::SeqCst);
        let mut headers = HeaderMap::new();
        headers.insert("retry-after", "1".parse().unwrap());
        Ok(HttpResponse::new(
            StatusCode::TOO_MANY_REQUESTS,
            headers,
            Box::pin(futures::stream::once(async {
                Ok(b"{\"error\":{\"message\":\"rate limit\"}}".to_vec().into())
            })),
        ))
    }
}

async fn driver_policy(max_retries: u32) {
    let profile = MemoryHosting::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let driver = everruns_contracts::OpenAIProtocolChatDriver::new()
        .with_transport(Arc::new(LimitedHttp(calls.clone())))
        .with_retry_config(everruns_contracts::LlmRetryConfig {
            max_retries,
            jitter_factor: 0.0,
            ..Default::default()
        });
    let agent = Agent::builder()
        .instructions("Answer briefly.")
        .provider(Provider::new("test", driver).base_url("https://test.invalid/v1"))
        .model("test")
        .build()
        .unwrap();
    let host = Engine::builder()
        .hosting(External(profile.clone()))
        .register(build(), agent)
        .build();
    let id = SessionId::new();
    let session = Box::pin(host.create_registered(id, &build()))
        .await
        .unwrap();
    let receipt = session.send("hello").await.unwrap();
    Box::pin(host.activate(id)).await.unwrap();
    if max_retries > 0 {
        let status = profile.status(id).await.unwrap();
        assert!(status.wake_at.is_some());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "HTTP driver must not sleep/retry underneath Engine"
        );
        tokio::time::sleep(Duration::from_millis(
            (status.wake_at.unwrap() - status.now_ms) as u64 + 20,
        ))
        .await;
        Box::pin(host.activate(id)).await.unwrap();
    }
    let turn = tokio::time::timeout(Duration::from_millis(500), receipt.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(!turn.success);
    assert_eq!(calls.load(Ordering::SeqCst), 1 + max_retries as usize);
    assert!(profile.status(id).await.unwrap().wake_at.is_none());
}

#[tokio::test]
async fn an_explicit_driver_no_retry_policy_is_preserved() {
    Box::pin(driver_policy(0)).await;
}

#[tokio::test]
async fn driver_and_engine_share_the_configured_retry_budget() {
    Box::pin(driver_policy(1)).await;
}

#[derive(Clone)]
struct StreamFault {
    calls: Arc<AtomicUsize>,
    partial: bool,
}
#[async_trait]
impl ChatDriver for StreamFault {
    async fn chat_completion_stream(
        &self,
        _: &ProviderEndpoint,
        _: Vec<everruns::llm::Message>,
        _: &LlmCallConfig,
    ) -> Result<LlmResponseStream, AgentLoopError> {
        let first = self.calls.fetch_add(1, Ordering::SeqCst) == 0;
        let events = if first {
            let mut events = vec![];
            if self.partial {
                events.push(Ok(LlmStreamEvent::TextDelta("already visible".into())));
            }
            events.push(Ok(LlmStreamEvent::Error(
                everruns_contracts::driver_registry::LlmStreamError::provider(
                    Some("server_error".into()),
                    Some(503),
                    "temporary provider failure",
                ),
            )));
            events
        } else {
            vec![
                Ok(LlmStreamEvent::TextDelta("recovered".into())),
                Ok(LlmStreamEvent::Done(Box::default())),
            ]
        };
        Ok(Box::pin(futures::stream::iter(events)))
    }
}

async fn streaming_boundary(partial: bool) {
    let profile = MemoryHosting::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let agent = Agent::builder()
        .instructions("Answer briefly.")
        .provider(Provider::new(
            "stream-fault",
            StreamFault {
                calls: calls.clone(),
                partial,
            },
        ))
        .model("test")
        .build()
        .unwrap();
    let host = Engine::builder()
        .hosting(External(profile.clone()))
        .register(build(), agent)
        .build();
    let id = SessionId::new();
    let session = Box::pin(host.create_registered(id, &build()))
        .await
        .unwrap();
    let sent = session.send("hello").await.unwrap();
    tokio::time::timeout(Duration::from_millis(500), Box::pin(host.activate(id)))
        .await
        .unwrap()
        .unwrap();
    let status = profile.status(id).await.unwrap();
    if partial {
        assert!(
            status.wake_at.is_none(),
            "visible partial output is not replayed as a fresh request"
        );
    } else {
        let at = status.wake_at.expect("a pre-output stream error parks");
        assert!(!status.executing);
        tokio::time::sleep(Duration::from_millis((at - status.now_ms) as u64 + 20)).await;
        Box::pin(host.activate(id)).await.unwrap();
    }
    let turn = session
        .admission(sent.message_id.parse().unwrap())
        .await
        .unwrap()
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert!(turn.success);
    assert_eq!(calls.load(Ordering::SeqCst), if partial { 1 } else { 2 });
}

#[tokio::test]
async fn a_stream_error_before_output_parks_for_recovery() {
    Box::pin(streaming_boundary(false)).await;
}

#[tokio::test]
async fn visible_partial_output_does_not_gain_automatic_replay() {
    Box::pin(streaming_boundary(true)).await;
}

#[tokio::test]
async fn retry_reopens_in_another_engine() {
    Box::pin(exercise(MemoryHosting::default(), 1)).await;
}

#[tokio::test]
async fn eviction_cannot_reset_the_attempt_budget() {
    Box::pin(exercise(MemoryHosting::default(), usize::MAX)).await;
}

#[cfg(feature = "local")]
#[tokio::test]
async fn sqlite_retry_reopens_in_another_engine() {
    let dir = tempfile::tempdir().unwrap();
    Box::pin(exercise(
        everruns::hosting::SqliteHosting::open(dir.path().join("sessions.sqlite")).unwrap(),
        1,
    ))
    .await;
}
