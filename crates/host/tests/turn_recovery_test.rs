use everruns_core::CapabilityRegistry;
use everruns_core::events::EventData;
use everruns_host::HostComposition;
use everruns_host::{HostBackends, InProcessRuntimeBuilder};
use everruns_provider::driver_registry::{DriverRegistry, LlmMessage};
use everruns_provider::llm_retry::LlmRetryConfig;
use everruns_test_support::LlmSimRuntimeExt;
use everruns_test_support::TestMathCapability;
use everruns_test_support::llmsim_driver::{LlmSimConfig, SimError, SimToolCall, SimTurn};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn fast_retry(max_retries: u32) -> LlmRetryConfig {
    LlmRetryConfig {
        max_retries,
        initial_backoff: Duration::from_millis(10),
        max_backoff: Duration::from_millis(40),
        backoff_multiplier: 2.0,
        jitter_factor: 0.0,
        max_retry_elapsed: Duration::from_millis(100),
    }
}

async fn runtime(
    turns: Vec<SimTurn>,
    retry: LlmRetryConfig,
    capture: Arc<Mutex<Vec<Vec<LlmMessage>>>>,
) -> everruns_host::InProcessRuntime {
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(TestMathCapability);
    let platform = HostComposition::new(capabilities, DriverRegistry::new());

    InProcessRuntimeBuilder::new()
        .host_composition(platform)
        .llm_sim_as_default(LlmSimConfig::scripted(turns).with_message_capture(capture))
        .provider_retry_config(retry)
        .provider_stall_timeout(Duration::from_millis(5))
        .single_session(|session| {
            session
                .harness("recovery", "Recover transient provider failures.")
                .with_capability("test_math")
                .agent("recovery-agent", "Complete the user's ask.")
        })
        .build()
        .await
        .expect("runtime")
}

fn provider_view(messages: &[LlmMessage]) -> Vec<String> {
    messages
        .iter()
        .map(|message| format!("{message:?}"))
        .collect()
}

#[tokio::test(start_paused = true)]
async fn transient_failure_classes_recover_through_run_turn() {
    for transient in [
        SimError::RateLimit,
        SimError::Timeout,
        SimError::Transport,
        SimError::Overloaded,
        SimError::Other("service unavailable".to_string()),
    ] {
        let capture = Arc::new(Mutex::new(Vec::new()));
        let runtime = runtime(
            vec![
                SimTurn::Error(transient.clone()),
                SimTurn::Assistant("recovered".to_string()),
            ],
            fast_retry(2),
            capture.clone(),
        )
        .await;
        let session_id = runtime.default_session_id().expect("session");

        let result = runtime
            .run_text_turn(session_id, "keep this ask intact")
            .await
            .expect("run turn");

        assert!(result.success, "{transient:?}: {result:?}");
        assert_eq!(result.response, "recovered");
        let attempts = capture.lock().expect("capture");
        assert_eq!(attempts.len(), 2, "{transient:?}");
        assert_eq!(
            provider_view(&attempts[0]),
            provider_view(&attempts[1]),
            "retry must preserve the ask"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn provider_stream_stall_recovers_through_run_turn() {
    let capture = Arc::new(Mutex::new(Vec::new()));
    let runtime = runtime(
        vec![
            SimTurn::StreamStall,
            SimTurn::Assistant("recovered after stall".to_string()),
        ],
        fast_retry(2),
        capture.clone(),
    )
    .await;
    let result = runtime
        .run_text_turn(
            runtime.default_session_id().expect("session"),
            "survive the stall",
        )
        .await
        .expect("run turn");

    assert!(result.success, "{result:?}");
    assert_eq!(result.response, "recovered after stall");
    assert_eq!(capture.lock().expect("capture").len(), 2);
}

#[tokio::test(start_paused = true)]
async fn permanent_failure_classes_fail_fast_through_run_turn() {
    for permanent in [
        SimError::Authentication,
        SimError::QuotaExhausted,
        SimError::UnsupportedModel("missing-model".to_string()),
        SimError::InvalidResponse("unsafe request".to_string()),
    ] {
        let capture = Arc::new(Mutex::new(Vec::new()));
        let runtime = runtime(
            vec![
                SimTurn::Error(permanent.clone()),
                SimTurn::Assistant("must not run".to_string()),
            ],
            fast_retry(2),
            capture.clone(),
        )
        .await;
        let result = runtime
            .run_text_turn(
                runtime.default_session_id().expect("session"),
                "fail precisely",
            )
            .await
            .expect("run turn");

        assert!(!result.success, "{permanent:?}");
        assert_eq!(capture.lock().expect("capture").len(), 1, "{permanent:?}");
        assert!(result.error.is_some(), "{permanent:?}");
    }
}

#[tokio::test(start_paused = true)]
async fn recovery_preserves_completed_tool_output_without_duplicate_side_effects() {
    let capture = Arc::new(Mutex::new(Vec::new()));
    let runtime = runtime(
        vec![
            SimTurn::ToolCalls(vec![SimToolCall {
                name: "multiply".to_string(),
                arguments: serde_json::json!({"a": 6, "b": 7}),
                id: Some("call_once".to_string()),
            }]),
            SimTurn::Error(SimError::Transport),
            SimTurn::Assistant("The result is 42.".to_string()),
        ],
        fast_retry(2),
        capture.clone(),
    )
    .await;
    let session_id = runtime.default_session_id().expect("session");

    let result = runtime
        .run_text_turn(session_id, "multiply 6 by 7")
        .await
        .expect("run turn");

    assert!(result.success, "{result:?}");
    assert_eq!(result.response, "The result is 42.");
    let completed_tools = runtime
        .events()
        .await
        .expect("events")
        .iter()
        .filter(|event| matches!(event.data, EventData::ToolCompleted(_)))
        .count();
    assert_eq!(
        completed_tools, 1,
        "completed mutation must not be replayed"
    );

    let attempts = capture.lock().expect("capture");
    assert_eq!(attempts.len(), 3);
    assert_eq!(
        provider_view(&attempts[1]),
        provider_view(&attempts[2]),
        "retry must preserve tool output"
    );
    assert!(
        attempts[2]
            .iter()
            .any(|message| message.content_as_text().contains("42")),
        "completed tool output must remain in provider continuation"
    );
}

#[tokio::test(start_paused = true)]
async fn retry_policy_beats_no_retry_baseline_and_obeys_backoff_and_time_budget() {
    let no_retry_capture = Arc::new(Mutex::new(Vec::new()));
    let baseline = runtime(
        vec![
            SimTurn::Error(SimError::Transport),
            SimTurn::Assistant("unreachable".to_string()),
        ],
        fast_retry(0),
        no_retry_capture.clone(),
    )
    .await;
    let baseline_result = baseline
        .run_text_turn(baseline.default_session_id().expect("session"), "baseline")
        .await
        .expect("baseline turn");
    assert!(!baseline_result.success);
    assert_eq!(no_retry_capture.lock().expect("capture").len(), 1);

    let recovered_capture = Arc::new(Mutex::new(Vec::new()));
    let recovered = runtime(
        vec![
            SimTurn::Error(SimError::Transport),
            SimTurn::Error(SimError::Overloaded),
            SimTurn::Assistant("completed".to_string()),
        ],
        fast_retry(2),
        recovered_capture.clone(),
    )
    .await;
    let started = tokio::time::Instant::now();
    let recovered_result = recovered
        .run_text_turn(recovered.default_session_id().expect("session"), "recover")
        .await
        .expect("recovered turn");
    assert!(recovered_result.success);
    assert_eq!(started.elapsed(), Duration::from_millis(30));
    assert_eq!(recovered_capture.lock().expect("capture").len(), 3);

    let budget_capture = Arc::new(Mutex::new(Vec::new()));
    let mut budget = fast_retry(5);
    budget.max_retry_elapsed = Duration::from_millis(5);
    let exhausted = runtime(
        vec![
            SimTurn::Error(SimError::Transport),
            SimTurn::Assistant("must not run".to_string()),
        ],
        budget,
        budget_capture.clone(),
    )
    .await;
    let exhausted_result = exhausted
        .run_text_turn(exhausted.default_session_id().expect("session"), "bounded")
        .await
        .expect("bounded turn");
    assert!(!exhausted_result.success);
    assert!(
        exhausted_result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("time budget exhausted")),
        "{exhausted_result:?}"
    );
    assert_eq!(budget_capture.lock().expect("capture").len(), 1);
}

#[tokio::test(start_paused = true)]
async fn interrupted_turn_history_survives_runtime_reconstruction() {
    let backends = HostBackends::in_memory();
    let stable_session_id = everruns_provider::typed_id::SessionId::from_seed(9191);
    let first_capture = Arc::new(Mutex::new(Vec::new()));
    let first = InProcessRuntimeBuilder::new()
        .backends(backends.clone())
        .llm_sim_as_default(
            LlmSimConfig::scripted(vec![SimTurn::Error(SimError::Transport)])
                .with_message_capture(first_capture),
        )
        .provider_retry_config(fast_retry(0))
        .single_session(|session| {
            session
                .session_id(stable_session_id)
                .harness("persistent-recovery", "Preserve session history.")
                .agent("persistent-recovery-agent", "Resume safely.")
        })
        .build()
        .await
        .expect("first runtime");
    let session_id = first.default_session_id().expect("session");
    let interrupted = first
        .run_text_turn(session_id, "original active ask")
        .await
        .expect("interrupted turn");
    assert!(!interrupted.success);
    drop(first);

    let resumed_capture = Arc::new(Mutex::new(Vec::new()));
    let resumed = InProcessRuntimeBuilder::new()
        .backends(backends)
        .llm_sim_as_default(
            LlmSimConfig::fixed("resumed completion").with_message_capture(resumed_capture.clone()),
        )
        .provider_retry_config(fast_retry(2))
        .single_session(|session| {
            session
                .session_id(stable_session_id)
                .harness("persistent-recovery", "Preserve session history.")
                .agent("persistent-recovery-agent", "Resume safely.")
        })
        .build()
        .await
        .expect("resumed runtime");
    assert_eq!(resumed.default_session_id(), Some(session_id));
    let result = resumed
        .run_text_turn(session_id, "resume the interrupted ask")
        .await
        .expect("resumed turn");
    assert!(result.success, "{result:?}");

    let calls = resumed_capture.lock().expect("capture");
    assert_eq!(calls.len(), 1);
    let visible = provider_view(&calls[0]).join("\n");
    assert!(visible.contains("original active ask"), "{visible}");
    assert!(visible.contains("resume the interrupted ask"), "{visible}");
}
