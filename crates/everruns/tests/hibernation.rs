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
    assert_eq!(
        status.phase(),
        "active",
        "parking compute does not finish the conversation's turn"
    );
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
                    Some("server_error"),
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
        .register(build(), agent.clone())
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
    let at = status.wake_at.expect("a replay-safe stream error parks");
    assert!(!status.executing);
    // Reconstruct Engine to prove the reset boundary survives executor loss.
    drop(session);
    drop(host);
    tokio::time::sleep(Duration::from_millis(
        (at - status.now_ms).max(0) as u64 + 20,
    ))
    .await;
    let host = Engine::builder()
        .hosting(External(profile.clone()))
        .register(build(), agent)
        .build();
    let session = host.resume(id).await.unwrap();
    Box::pin(host.activate(id)).await.unwrap();
    let turn = session
        .admission(sent.message_id.parse().unwrap())
        .await
        .unwrap()
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert!(turn.success);
    assert_eq!(turn.response, "recovered");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let events = session.events_after(0).await.unwrap();
    let starts: Vec<_> = events
        .iter()
        .filter_map(|event| match &event.kind {
            everruns::SessionEventKind::OutputStarted { message_id } => Some(message_id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(starts.len(), if partial { 2 } else { 1 });
    if partial {
        assert_eq!(starts[0], starts[1], "a parked retry keeps its message ID");
    }
}

#[tokio::test]
async fn a_stream_error_before_output_parks_for_recovery() {
    Box::pin(streaming_boundary(false)).await;
}

#[tokio::test]
async fn partial_text_restarts_under_the_same_message_after_executor_loss() {
    Box::pin(streaming_boundary(true)).await;
}

#[tokio::test]
async fn input_during_backoff_keeps_its_admission_and_the_original_deadline() {
    let profile = MemoryHosting::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let host = engine(External(profile.clone()), calls.clone(), 1);
    let id = SessionId::new();
    let session = Box::pin(host.create_registered(id, &build()))
        .await
        .unwrap();
    let first = session.send("first").await.unwrap();
    Box::pin(host.activate(id)).await.unwrap();
    let at = profile.status(id).await.unwrap().wake_at.unwrap();
    let second = session.send("also include this").await.unwrap();
    tokio::time::timeout(Duration::from_millis(500), Box::pin(host.activate(id)))
        .await
        .unwrap()
        .unwrap();
    let status = profile.status(id).await.unwrap();
    assert_eq!(status.wake_at, Some(at));
    assert!(!status.runnable);
    assert!(!status.executing);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    tokio::time::sleep(Duration::from_millis(
        (at - status.now_ms).max(0) as u64 + 20,
    ))
    .await;
    Box::pin(host.activate(id)).await.unwrap();
    for sent in [first, second] {
        let admission = session
            .admission(sent.message_id.parse().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(500), admission.wait())
                .await
                .unwrap()
                .unwrap()
                .success
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
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

/// First call: a classified provider refusal. Every later call never answers,
/// so the retained retry window expires on the reopened executor (EVE-1318).
#[derive(Clone)]
struct RefusedThenSilent {
    calls: Arc<AtomicUsize>,
    status: u16,
}
#[async_trait]
impl ChatDriver for RefusedThenSilent {
    async fn chat_completion_stream(
        &self,
        _: &ProviderEndpoint,
        _: Vec<everruns::llm::Message>,
        _: &LlmCallConfig,
    ) -> Result<LlmResponseStream, AgentLoopError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) > 0 {
            return futures::future::pending().await;
        }
        let body = if self.status == 429 {
            r#"{"error":{"code":"engine_overloaded","message":"Authorization: Bearer sk-live-secret","metadata":{"limit_source":"upstream_provider_shared_pool"}}}"#
        } else {
            r#"{"error":{"code":"service_unavailable","message":"Authorization: Bearer sk-live-secret"}}"#
        };
        Err(AgentLoopError::llm_http(
            self.status,
            body,
            format!("provider error ({}): {body}", self.status),
        ))
    }
}

/// `late`: reopen only after the retained window already expired, so the
/// Engine rejects the turn before provider I/O. Otherwise the reopened
/// attempt itself runs out of the remaining window.
async fn exhausted_window(status: u16, late: bool) -> (String, String) {
    let profile = MemoryHosting::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let agent = Agent::builder()
        .instructions("Answer briefly.")
        .provider(Provider::new(
            "refusing",
            RefusedThenSilent {
                calls: calls.clone(),
                status,
            },
        ))
        .model("test")
        .host_services(
            everruns::hosting::services::HostServices::new().provider_retry(
                everruns_contracts::LlmRetryConfig {
                    max_retries: 3,
                    initial_backoff: Duration::from_millis(50),
                    max_backoff: Duration::from_millis(50),
                    jitter_factor: 0.0,
                    max_retry_elapsed: Duration::from_millis(400),
                    ..Default::default()
                },
            ),
        )
        .build()
        .unwrap();
    let engine = || {
        Engine::builder()
            .hosting(External(profile.clone()))
            .register(build(), agent.clone())
            .build()
    };
    let first = engine();
    let id = SessionId::new();
    let session = Box::pin(first.create_registered(id, &build()))
        .await
        .unwrap();
    let sent = session.send("hello").await.unwrap();
    Box::pin(first.activate(id)).await.unwrap();
    let parked = profile.status(id).await.unwrap();
    assert!(parked.wake_at.is_some(), "a classified refusal parks");
    // Executor loss between attempts: the cause must survive in the checkpoint.
    drop(session);
    drop(first);
    let wait = if late {
        600
    } else {
        (parked.wake_at.unwrap() - parked.now_ms).max(0) as u64 + 20
    };
    tokio::time::sleep(Duration::from_millis(wait)).await;
    let replacement = engine();
    let session = replacement.resume(id).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), Box::pin(replacement.activate(id)))
        .await
        .unwrap()
        .unwrap();
    let turn = tokio::time::timeout(
        Duration::from_secs(2),
        session
            .admission(sent.message_id.parse().unwrap())
            .await
            .unwrap()
            .unwrap()
            .wait(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!turn.success);
    assert_eq!(calls.load(Ordering::SeqCst), if late { 1 } else { 2 });
    let events = session.events_after(0).await.unwrap();
    assert!(
        !format!("{events:?}").contains("sk-live-secret"),
        "{events:?}"
    );
    let reason = events
        .iter()
        .find_map(|event| match &event.kind {
            everruns::SessionEventKind::ReasonCompleted {
                success: false,
                error: Some(error),
            } => Some(error.clone()),
            _ => None,
        })
        .expect("the failed step records its terminal error");
    assert!(
        reason.contains("budget exhausted after 1 retries"),
        "{reason}"
    );
    (reason, turn.error.expect("a failed turn names its error"))
}

#[tokio::test]
async fn expired_retry_window_keeps_the_classified_429_after_reopen() {
    for late in [false, true] {
        let (error, shown) = Box::pin(exhausted_window(429, late)).await;
        assert!(shown.starts_with("Rate limited"), "{shown}");
        assert!(
            error.contains("last provider failure: rate_limited"),
            "{error}"
        );
        assert!(error.contains("HTTP 429"), "{error}");
        assert!(error.contains("engine_overloaded"), "{error}");
        assert!(error.contains("upstream_provider_shared_pool"), "{error}");
    }
}

#[tokio::test]
async fn expired_retry_window_keeps_a_provider_outage_distinct() {
    for late in [false, true] {
        let (error, shown) = Box::pin(exhausted_window(503, late)).await;
        assert!(shown.contains("experiencing issues"), "{shown}");
        assert!(
            error.contains("last provider failure: unavailable"),
            "{error}"
        );
        assert!(error.contains("HTTP 503"), "{error}");
        assert!(error.contains("service_unavailable"), "{error}");
        assert!(!error.contains("rate_limited"), "{error}");
    }
}
