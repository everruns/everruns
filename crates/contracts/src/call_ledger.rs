//! Privacy-minimal logical provider-call attribution, including auxiliary calls.
//!
//! Install at the registry boundary so a completion review cannot bypass accounting
//! merely because it does not emit an assistant message. This observes driver calls,
//! not physical HTTP attempts: internal retries and native batch requests require
//! separate transport records. Missing usage is unknown, never zero cost.

use crate::compact::{CompactRequest, CompactResponse};
use crate::driver_registry::*;
use crate::error::Result;
use async_trait::async_trait;
use futures::StreamExt;
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Arc;

/// An append-only start/terminal pair sharing one locally generated call ID.
/// No prompts, tool arguments, endpoint URLs, headers or error bodies are copied.
#[derive(Debug, Clone, Serialize)]
pub struct CallRecord {
    pub schema_version: u32,
    pub call_id: String,
    pub phase: String,
    pub recorded_at: chrono::DateTime<chrono::Utc>,
    /// Driver-reported retry count, not separately attributed HTTP attempts.
    pub retries: Option<u32>,
    pub provider: String,
    pub model: String,
    pub metadata: BTreeMap<String, String>,
    pub elapsed_ms: u64,
    /// HTTP request ID, distinct from response_id; absent when unavailable.
    pub provider_request_id: Option<String>,
    pub response_id: Option<String>,
    pub response_model: Option<String>,
    pub total_tokens: Option<u32>,
    pub service_tier: Option<String>,
    /// Non-cached input; cache buckets are additive.
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub cache_read_tokens: Option<u32>,
    pub cache_creation_tokens: Option<u32>,
    pub cache_creation_1h_tokens: Option<u32>,
    pub cache_creation_12h_tokens: Option<u32>,
    /// Subset of completion tokens, not an extra billable bucket.
    pub reasoning_tokens: Option<u32>,
    pub provider_cost_usd: Option<f64>,
}

/// Hosts choose durable storage and report their own write failures. Implementations
/// must not panic or block on network I/O. This hook is not a spending-limit system.
pub trait CallObserver: Send + Sync {
    fn record(&self, record: CallRecord);
}

/// Wraps all three chat entry points without changing the driver's wire protocol.
/// Native batch, model discovery, credit checks and compaction are delegated, not
/// attributed as chat. Hosts must not call this a complete billing reconciliation.
pub struct ObservedChatDriver {
    inner: Arc<dyn ChatDriver>,
    provider: String,
    observer: Arc<dyn CallObserver>,
}

impl ObservedChatDriver {
    pub fn new(
        provider: impl Into<String>,
        inner: Arc<dyn ChatDriver>,
        observer: Arc<dyn CallObserver>,
    ) -> Self {
        Self {
            inner,
            provider: provider.into(),
            observer,
        }
    }

    fn start(&self, config: &LlmCallConfig) -> CallGuard {
        let metadata = config
            .metadata
            .iter()
            .filter(|(key, value)| {
                matches!(
                    key.as_str(),
                    "session_id"
                        | "turn_id"
                        | "agent_id"
                        | "org_id"
                        | "exec_id"
                        | "purpose"
                        | "eval_run_id"
                        | "eval_case_id"
                        | "eval_trial"
                        | "eval_binary"
                        | "github_run_id"
                        | "github_run_attempt"
                ) && safe_identifier(value)
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        let record = CallRecord {
            schema_version: 1,
            call_id: uuid::Uuid::new_v4().to_string(),
            phase: "started".into(),
            recorded_at: chrono::Utc::now(),
            retries: None,
            provider: self.provider.clone(),
            model: config.model.clone(),
            metadata,
            elapsed_ms: 0,
            provider_request_id: None,
            response_id: None,
            response_model: None,
            total_tokens: None,
            service_tier: None,
            prompt_tokens: None,
            completion_tokens: None,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            cache_creation_1h_tokens: None,
            cache_creation_12h_tokens: None,
            reasoning_tokens: None,
            provider_cost_usd: None,
        };
        self.observer.record(record.clone());
        CallGuard {
            record,
            observer: self.observer.clone(),
            started: crate::rt::Instant::now(),
            finished: false,
        }
    }
}

fn safe_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-:/".contains(&b))
}

struct CallGuard {
    record: CallRecord,
    observer: Arc<dyn CallObserver>,
    started: crate::rt::Instant,
    finished: bool,
}

impl CallGuard {
    fn finish_result(&mut self, result: &Result<LlmResponse>) {
        match result {
            Ok(response) => self.finish("completed", Some(&response.metadata)),
            Err(_) => self.finish("failed", None),
        }
    }

    fn finish(&mut self, phase: &str, metadata: Option<&LlmCompletionMetadata>) {
        if self.finished {
            return;
        }
        self.finished = true;
        self.record.phase = phase.into();
        self.record.recorded_at = chrono::Utc::now();
        self.record.elapsed_ms = self.started.elapsed().as_millis().min(u64::MAX as u128) as u64;
        if let Some(m) = metadata {
            self.record.retries = m.retry_metadata.as_ref().map(|r| r.attempts);
            self.record.provider_request_id = m.provider_request_id.clone();
            self.record.response_id = m.response_id.clone();
            self.record.response_model = m.response_model.clone();
            self.record.total_tokens = m.total_tokens;
            self.record.service_tier = m.service_tier.clone();
            self.record.prompt_tokens = m.prompt_tokens;
            self.record.completion_tokens = m.completion_tokens;
            self.record.cache_read_tokens = m.cache_read_tokens;
            self.record.cache_creation_tokens = m.cache_creation_tokens;
            self.record.cache_creation_1h_tokens = m.cache_creation_1h_tokens;
            self.record.cache_creation_12h_tokens = m.cache_creation_12h_tokens;
            self.record.reasoning_tokens = m.reasoning_tokens;
            self.record.provider_cost_usd = m
                .provider_cost_usd
                .filter(|cost| cost.is_finite() && *cost >= 0.0);
        }
        self.observer.record(self.record.clone());
    }
}

impl Drop for CallGuard {
    fn drop(&mut self) {
        self.finish("incomplete", None);
    }
}

#[async_trait]
impl ChatDriver for ObservedChatDriver {
    fn batch_driver(&self) -> Option<&dyn crate::batch::BatchDriver> {
        self.inner.batch_driver()
    }

    fn native_async_driver(
        &self,
        model: &str,
        tools: std::collections::BTreeMap<String, Option<serde_json::Value>>,
        continuation: Option<crate::native_async::Delivery>,
    ) -> Option<Arc<dyn ChatDriver>> {
        Some(Arc::new(Self {
            inner: self.inner.native_async_driver(model, tools, continuation)?,
            provider: self.provider.clone(),
            observer: self.observer.clone(),
        }))
    }
    async fn chat_completion_stream(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        let mut call = self.start(config);
        match self
            .inner
            .chat_completion_stream(endpoint, messages, config)
            .await
        {
            Ok(stream) => Ok(Box::pin(stream.map(move |item| {
                match &item {
                    Ok(LlmStreamEvent::Done(metadata)) => call.finish("completed", Some(metadata)),
                    Err(_) => call.finish("failed", None),
                    _ => {}
                }
                item
            }))),
            Err(error) => {
                call.finish("failed", None);
                Err(error)
            }
        }
    }

    async fn chat_completion(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponse> {
        let mut call = self.start(config);
        let result = self.inner.chat_completion(endpoint, messages, config).await;
        call.finish_result(&result);
        result
    }

    fn supports_native_non_streaming(&self) -> bool {
        self.inner.supports_native_non_streaming()
    }

    async fn chat_completion_non_streaming(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponse> {
        let mut call = self.start(config);
        let result = self
            .inner
            .chat_completion_non_streaming(endpoint, messages, config)
            .await;
        call.finish_result(&result);
        result
    }

    async fn list_models(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
    ) -> Result<Option<Vec<DiscoveredModel>>> {
        self.inner.list_models(endpoint).await
    }

    fn supports_compact(&self) -> bool {
        self.inner.supports_compact()
    }

    fn supports_stateful_responses(&self) -> bool {
        self.inner.supports_stateful_responses()
    }

    fn effective_context_window(&self, model: &str) -> Option<usize> {
        self.inner.effective_context_window(model)
    }

    fn stream_stall_timeout(&self, config: &LlmCallConfig) -> Option<std::time::Duration> {
        self.inner.stream_stall_timeout(config)
    }

    fn supports_parallel_tool_calls(&self, model: &str) -> bool {
        self.inner.supports_parallel_tool_calls(model)
    }

    fn supports_response_format(&self, model: &str) -> bool {
        self.inner.supports_response_format(model)
    }

    fn provider_managed_reduction_option(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        model: &str,
        budget_tokens: usize,
    ) -> Option<(String, serde_json::Value)> {
        self.inner
            .provider_managed_reduction_option(endpoint, model, budget_tokens)
    }

    async fn credit_status(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
    ) -> Result<Option<crate::provider_limit::CreditStatus>> {
        self.inner.credit_status(endpoint).await
    }

    fn provider_managed_reduction_fallback_reason(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        config: &LlmCallConfig,
    ) -> Option<&'static str> {
        self.inner
            .provider_managed_reduction_fallback_reason(endpoint, config)
    }

    fn validate_provider_opaque_context(&self, context: &ProviderOpaqueContext) -> bool {
        self.inner.validate_provider_opaque_context(context)
    }

    async fn compact(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        request: CompactRequest,
    ) -> Result<Option<CompactResponse>> {
        self.inner.compact(endpoint, request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_provider::ProviderEndpoint;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemoryObserver(Mutex<Vec<CallRecord>>);
    impl CallObserver for MemoryObserver {
        fn record(&self, record: CallRecord) {
            self.0.lock().unwrap().push(record);
        }
    }

    struct Stub;
    #[async_trait]
    impl ChatDriver for Stub {
        async fn chat_completion_stream(
            &self,
            _: &ProviderEndpoint,
            _: Vec<Message>,
            _: &LlmCallConfig,
        ) -> Result<LlmResponseStream> {
            let mut metadata = LlmCompletionMetadata::default();
            metadata.prompt_tokens = Some(12);
            metadata.completion_tokens = Some(7);
            metadata.response_id = Some("resp_test".into());
            metadata.provider_request_id = Some("req_test".into());
            metadata.response_model = Some("served-model".into());
            metadata.request_body = Some(serde_json::json!({"secret": "must-not-record"}));
            Ok(Box::pin(futures::stream::iter([
                Ok(LlmStreamEvent::TextDelta("private answer".into())),
                Ok(LlmStreamEvent::Done(Box::new(metadata))),
            ])))
        }
        fn supports_response_format(&self, _: &str) -> bool {
            true
        }
        fn supports_parallel_tool_calls(&self, _: &str) -> bool {
            false
        }
    }

    fn setup() -> (BoxedChatDriver, Arc<MemoryObserver>, LlmCallConfig) {
        let observer = Arc::new(MemoryObserver::default());
        let mut registry = DriverRegistry::new().with_call_observer(observer.clone());
        registry.register_external("ledger-test", |_| Box::new(Stub));
        let driver = registry
            .create_chat_driver(&ProviderConfig::new(DriverId::external("ledger-test")))
            .unwrap();
        let mut config = LlmCallConfig::new("test-model");
        config
            .metadata
            .insert("purpose".into(), "completion_review".into());
        config
            .metadata
            .insert("session_id".into(), "session_123".into());
        config
            .metadata
            .insert("api_key".into(), "secret-token".into());
        config
            .metadata
            .insert("eval_case_id".into(), "invalid\nvalue".into());
        (driver, observer, config)
    }

    #[tokio::test]
    async fn registry_observes_auxiliary_completion_once_without_content_or_credentials() {
        let (driver, observer, config) = setup();
        let response = driver
            .chat_completion(&ProviderEndpoint::default(), vec![], &config)
            .await
            .unwrap();
        assert_eq!(response.text, "private answer");
        let records = observer.0.lock().unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].phase, "started");
        assert_eq!(records[1].phase, "completed");
        assert_eq!(records[0].call_id, records[1].call_id);
        assert_eq!(records[1].metadata["purpose"], "completion_review");
        assert_eq!(records[1].prompt_tokens, Some(12));
        assert_eq!(records[1].response_id.as_deref(), Some("resp_test"));
        assert_eq!(records[1].provider_request_id.as_deref(), Some("req_test"));
        assert_eq!(records[1].provider_cost_usd, None);
        let json = serde_json::to_string(&*records).unwrap();
        for forbidden in [
            "private answer",
            "secret-token",
            "must-not-record",
            "invalid",
            "api_key",
            "request_body",
        ] {
            assert!(!json.contains(forbidden), "{forbidden}");
        }
    }

    #[tokio::test]
    async fn stream_and_nonstream_entrypoints_do_not_double_count() {
        let (driver, observer, config) = setup();
        let mut stream = driver
            .chat_completion_stream(&ProviderEndpoint::default(), vec![], &config)
            .await
            .unwrap();
        while stream.next().await.is_some() {}
        drop(stream);
        driver
            .chat_completion_non_streaming(&ProviderEndpoint::default(), vec![], &config)
            .await
            .unwrap();
        let records = observer.0.lock().unwrap();
        assert_eq!(records.len(), 4);
        assert_eq!(records.iter().filter(|r| r.phase == "completed").count(), 2);
        assert_ne!(records[0].call_id, records[2].call_id);
        assert!(driver.supports_response_format("model"));
        assert!(!driver.supports_parallel_tool_calls("model"));
    }

    #[tokio::test]
    async fn dropped_stream_records_incomplete_not_zero_cost_success() {
        let (driver, observer, config) = setup();
        let stream = driver
            .chat_completion_stream(&ProviderEndpoint::default(), vec![], &config)
            .await
            .unwrap();
        drop(stream);
        let records = observer.0.lock().unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[1].phase, "incomplete");
        assert_eq!(records[1].prompt_tokens, None);
        assert_eq!(records[1].provider_cost_usd, None);
    }

    struct Failure;
    #[async_trait]
    impl ChatDriver for Failure {
        async fn chat_completion_stream(
            &self,
            _: &ProviderEndpoint,
            _: Vec<Message>,
            _: &LlmCallConfig,
        ) -> Result<LlmResponseStream> {
            Err(crate::error::AgentLoopError::llm(
                "private provider payload with a credential",
            ))
        }
    }

    #[tokio::test]
    async fn failed_connections_are_recorded_without_raw_error_bodies() {
        let observer = Arc::new(MemoryObserver::default());
        let driver = ObservedChatDriver::new("test", Arc::new(Failure), observer.clone());
        assert!(
            driver
                .chat_completion_stream(
                    &ProviderEndpoint::default(),
                    vec![],
                    &LlmCallConfig::new("model")
                )
                .await
                .is_err()
        );
        let records = observer.0.lock().unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[1].phase, "failed");
        assert!(
            !serde_json::to_string(&*records)
                .unwrap()
                .contains("credential")
        );
    }

    #[tokio::test]
    async fn directly_registered_runtime_provider_is_observed_too() {
        let observer = Arc::new(MemoryObserver::default());
        let mut registry = DriverRegistry::new().with_call_observer(observer.clone());
        registry
            .register_provider(crate::runtime_provider::RuntimeProvider::new(
                "custom", Stub,
            ))
            .unwrap();
        let driver = registry
            .create_chat_driver(&ProviderConfig::for_provider(
                "custom",
                DriverId::external("unused"),
            ))
            .unwrap();
        driver
            .chat_completion(
                &ProviderEndpoint::default(),
                vec![],
                &LlmCallConfig::new("model"),
            )
            .await
            .unwrap();
        let records = observer.0.lock().unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[1].provider, "custom");
        assert_eq!(records[1].phase, "completed");
    }

    struct Stalled;
    #[async_trait]
    impl ChatDriver for Stalled {
        async fn chat_completion_stream(
            &self,
            _: &ProviderEndpoint,
            _: Vec<Message>,
            _: &LlmCallConfig,
        ) -> Result<LlmResponseStream> {
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn cancelled_connection_is_incomplete_not_a_missing_terminal() {
        let observer = Arc::new(MemoryObserver::default());
        let driver = ObservedChatDriver::new("test", Arc::new(Stalled), observer.clone());
        let config = LlmCallConfig::new("model");
        let endpoint = ProviderEndpoint::default();
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(1),
                driver.chat_completion_stream(&endpoint, vec![], &config)
            )
            .await
            .is_err()
        );
        let records = observer.0.lock().unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[1].phase, "incomplete");
    }
}
