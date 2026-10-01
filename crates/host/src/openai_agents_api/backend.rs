//! Host wiring: run the Reason activity through the Agents API when the
//! session selected the backend, and the native runtime otherwise.
//!
//! The native worker path is untouched unless every condition holds: the
//! resolved capabilities include `openai_agents_api_runtime` (which the
//! platform strips unless the org has the `openai_agents_api` flag), the
//! turn's model is bound to the official OpenAI API, and the host supplies a
//! durable [`AgentsApiStore`]. A selected session whose model or host cannot
//! serve the backend falls back to the native loop.

use std::sync::Arc;

use async_trait::async_trait;
use everruns_core::events::{
    EventContext, EventRequest, ReasonCompletedData, ReasonStartedData, ToolCompletedData,
};
use everruns_core::{
    AssembledTurnContext, ContentPart, EventEmitter, MessageRetriever, ScopedMcpServers,
};
use everruns_engine::{NativeExecutionCounts, ReasonInput, ReasonResult};
use everruns_provider::BearerAuth;
use everruns_provider::driver_registry::{DriverId, ProviderConfig};
use everruns_provider::error::{AgentLoopError, Result};
use everruns_provider::runtime_provider::ProviderEndpoint;
use everruns_provider::typed_id::{MessageId, SessionId};

use super::durable::{
    AgentsApiFunctionExecutor, AgentsApiLedger, AgentsApiTurnDriver, AgentsApiTurnOutcome,
    AgentsApiTurnRequest,
};
use super::{
    AgentsApiClient, AgentsApiError, FunctionCallAction, RUNTIME_CAPABILITY_ID,
    build_session_config,
};

/// Whether the resolved capabilities select the Agents API backend.
pub fn selects_backend(capabilities: &[everruns_capability::CapabilityRef]) -> bool {
    capabilities
        .iter()
        .any(|config| config.id() == RUNTIME_CAPABILITY_ID)
}

/// Run the turn through the Agents API when selected and available. `None`
/// means the native Reason path runs unchanged.
pub(crate) async fn try_execute_reason<A: crate::RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    input: &ReasonInput,
    assembled: &AssembledTurnContext,
) -> Result<Option<ReasonResult>> {
    if !selects_backend(&assembled.resolved_capability_configs) {
        return Ok(None);
    }
    let provider = if assembled.model.provider_type == DriverId::OpenAI {
        adapter
            .provider_store(org_id)
            .get_provider_config(&assembled.model.provider)
            .await?
    } else {
        None
    };
    let Some(endpoint) = provider.as_ref().and_then(official_endpoint) else {
        tracing::warn!(
            session_id = %input.context.session_id,
            model = %assembled.model.model,
            "openai_agents_api_runtime selected for a model not bound to the OpenAI API; using the native runtime"
        );
        return Ok(None);
    };
    let Some(store) = adapter.agents_api_store() else {
        tracing::warn!(
            session_id = %input.context.session_id,
            "openai_agents_api_runtime selected but the host has no durable Agents API store; using the native runtime"
        );
        return Ok(None);
    };
    let input_text = input_text(assembled, input.context.input_message_id)?;
    // Every Everruns tool crosses the client-function boundary; direct MCP
    // and OpenAI built-ins are not sent, so the provider cannot run a tool
    // outside Everruns' pipeline.
    let config = build_session_config(
        &assembled.runtime_agent,
        &ScopedMcpServers::default(),
        "",
        None,
    )
    .map_err(|error| AgentLoopError::config(error.to_string()))?;
    let event_context = EventContext::from_execution_context(&input.context);
    let request = AgentsApiTurnRequest {
        org_id,
        session_id: input.context.session_id,
        turn_id: input.context.turn_id,
        input_message_id: input.context.input_message_id,
        input_text,
        config,
        event_context: event_context.clone(),
    };
    let ledger = Arc::new(HostLedger {
        emitter: adapter.event_emitter(),
        messages: adapter.message_store(),
    });
    let template = everruns_engine::ActInput {
        org_id: Some(org_id),
        context: input.context.clone(),
        harness_id: input.harness_id,
        agent_id: input.agent_id,
        tool_calls: vec![],
        tool_definitions: assembled.runtime_agent.tools.clone(),
        locale: assembled.resolved_locale.clone(),
        blueprint_id: assembled.snapshot.blueprint_id.clone(),
        network_access: assembled.runtime_agent.network_access.clone(),
        parallel_tool_calls: assembled.runtime_agent.parallel_tool_calls,
    };
    let executor = Arc::new(HostFunctionExecutor {
        adapter: adapter.clone(),
        template,
        emitter: adapter.event_emitter(),
        event_context: event_context.clone(),
    });
    let mut driver = AgentsApiTurnDriver::new(
        AgentsApiClient::from_endpoint(endpoint),
        store,
        ledger,
        executor,
    );
    if let Some(cancellation) = adapter.turn_cancellation() {
        driver = driver.with_cancellation(cancellation);
    }

    let emitter = adapter.event_emitter();
    let started = std::time::Instant::now();
    if let Err(error) = emitter
        .emit(EventRequest::new(
            input.context.session_id,
            event_context.clone(),
            ReasonStartedData {
                harness_id: input.harness_id,
                agent_id: input.agent_id,
                metadata: None,
            },
        ))
        .await
    {
        tracing::warn!(%error, "Agents API backend: failed to emit reason.started");
    }
    let outcome = driver.run(&request).await;
    let duration_ms = Some(started.elapsed().as_millis().min(u64::MAX as u128) as u64);
    let result = match outcome {
        Ok(AgentsApiTurnOutcome::Completed {
            final_message_id,
            final_text,
            usage,
            tool_calls,
        }) => ReasonResult {
            native_counts: Some(NativeExecutionCounts {
                llm_calls: 1,
                tool_calls,
            }),
            success: true,
            text: final_text,
            tool_definitions: assembled.runtime_agent.tools.clone(),
            max_iterations: assembled.runtime_agent.max_iterations,
            usage,
            output_message_id: final_message_id,
            finish_reason: Some("stop".to_string()),
            locale: assembled.resolved_locale.clone(),
            network_access: assembled.runtime_agent.network_access.clone(),
            parallel_tool_calls: assembled.runtime_agent.parallel_tool_calls,
            ..ReasonResult::default()
        },
        Ok(AgentsApiTurnOutcome::Failed { code, message }) => {
            let error = match code {
                Some(code) => format!("OpenAI Agents API turn failed ({code}): {message}"),
                None => format!("OpenAI Agents API turn failed: {message}"),
            };
            ReasonResult {
                success: false,
                text: error.clone(),
                error: Some(error),
                max_iterations: assembled.runtime_agent.max_iterations,
                ..ReasonResult::default()
            }
        }
        Ok(AgentsApiTurnOutcome::Cancelled) | Err(AgentsApiError::Cancelled) => {
            return Err(AgentLoopError::Cancelled);
        }
        Err(error) => return Err(to_loop_error(error)),
    };
    let completed = if result.success {
        ReasonCompletedData::success(&result.text, false, 0, duration_ms, result.usage.clone())
    } else {
        ReasonCompletedData::failure(result.error.clone().unwrap_or_default(), duration_ms)
    };
    if let Err(error) = emitter
        .emit(EventRequest::new(
            input.context.session_id,
            event_context,
            completed,
        ))
        .await
    {
        tracing::warn!(%error, "Agents API backend: failed to emit reason.completed");
    }
    Ok(Some(result))
}

/// The Agents API endpoint for the turn's provider, with that provider's own
/// key. Only the OpenAI Responses driver on the official API qualifies; a
/// gateway, Azure, or another driver keeps the native loop.
// THREAT[TM-LLM-043]: the loop leaves the platform only through the official
// OpenAI API, billed to the session's own provider credentials.
fn official_endpoint(config: &ProviderConfig) -> Option<ProviderEndpoint> {
    if config.provider_type != DriverId::OpenAI {
        return None;
    }
    let base = config
        .base_url
        .as_deref()
        .unwrap_or(super::DEFAULT_BASE_URL);
    let lower = base.to_ascii_lowercase();
    let official =
        lower == "https://api.openai.com" || lower.starts_with("https://api.openai.com/");
    let key = config
        .api_key
        .as_deref()
        .filter(|key| !key.trim().is_empty())?;
    official.then(|| ProviderEndpoint::from_parts(base, BearerAuth::new(key)))
}

fn to_loop_error(error: AgentsApiError) -> AgentLoopError {
    match error {
        AgentsApiError::Store(message) => AgentLoopError::store(message),
        AgentsApiError::Cancelled => AgentLoopError::Cancelled,
        other => AgentLoopError::llm(other.to_string()),
    }
}

/// The text of the turn's input message. The provider receives text input
/// only; attachments fail closed rather than being silently dropped.
fn input_text(assembled: &AssembledTurnContext, input_message_id: MessageId) -> Result<String> {
    message_text(&assembled.messages, input_message_id)
}

fn message_text(
    messages: &[everruns_core::RuntimeMessage],
    input_message_id: MessageId,
) -> Result<String> {
    let message = messages
        .iter()
        .find(|message| message.id == input_message_id)
        .ok_or_else(|| {
            AgentLoopError::config("Agents API backend: turn input message is not in context")
        })?;
    let mut text = Vec::new();
    for part in &message.content {
        match part {
            ContentPart::Text(part) => text.push(part.text.as_str()),
            _ => {
                return Err(AgentLoopError::config(
                    "Agents API backend accepts text input only",
                ));
            }
        }
    }
    let text = text.join("\n");
    if text.trim().is_empty() {
        return Err(AgentLoopError::config(
            "Agents API backend: turn input has no text",
        ));
    }
    Ok(text)
}

/// The Everruns event log through the host's emitter and message store.
struct HostLedger {
    emitter: Arc<dyn EventEmitter>,
    messages: Arc<dyn MessageRetriever>,
}

fn ledger_error(error: impl std::fmt::Display) -> AgentsApiError {
    AgentsApiError::Store(error.to_string())
}

#[async_trait]
impl AgentsApiLedger for HostLedger {
    async fn emit(&self, event: EventRequest) -> std::result::Result<(), AgentsApiError> {
        self.emitter.emit(event).await.map_err(ledger_error)?;
        Ok(())
    }

    async fn has_message(
        &self,
        session_id: SessionId,
        message_id: MessageId,
    ) -> std::result::Result<bool, AgentsApiError> {
        Ok(self
            .messages
            .get(session_id, message_id)
            .await
            .map_err(ledger_error)?
            .is_some())
    }

    async fn tool_result(
        &self,
        session_id: SessionId,
        call_id: &str,
    ) -> std::result::Result<Option<std::result::Result<String, String>>, AgentsApiError> {
        let messages = self.messages.load(session_id).await.map_err(ledger_error)?;
        Ok(messages.iter().rev().find_map(|message| {
            message.content.iter().find_map(|part| match part {
                ContentPart::ToolResult(result) if result.tool_call_id == call_id => {
                    Some(match &result.error {
                        Some(error) => Err(error.clone()),
                        None => Ok(result_text(result.result.as_ref())),
                    })
                }
                _ => None,
            })
        }))
    }
}

fn result_text(result: Option<&serde_json::Value>) -> String {
    match result {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(serde_json::Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// Executes client functions through the ordinary Act pipeline: permission
/// checks, hooks, durable per-call claims, and the canonical `tool.completed`.
struct HostFunctionExecutor<A: crate::RuntimeHostAdapter> {
    adapter: A,
    template: everruns_engine::ActInput,
    emitter: Arc<dyn EventEmitter>,
    event_context: EventContext,
}

#[async_trait]
impl<A: crate::RuntimeHostAdapter> AgentsApiFunctionExecutor for HostFunctionExecutor<A> {
    async fn execute(
        &self,
        call: &FunctionCallAction,
    ) -> std::result::Result<std::result::Result<String, String>, AgentsApiError> {
        let mut input = self.template.clone();
        input.context.exec_id = everruns_provider::typed_id::ExecId::new();
        input.tool_calls = vec![everruns_provider::tool_types::ToolCall {
            id: call.call_id.clone(),
            name: call.name.clone(),
            arguments: call.arguments.clone(),
        }];
        let outcome = crate::execute_act_activity(&self.adapter, input)
            .await
            .map_err(|error| AgentsApiError::Store(error.user_facing_message()))?;
        if outcome.blocked || outcome.waiting_for_tool_results {
            // Approval gates and client-side tools pause a native turn. The
            // remote loop cannot pause on them yet (EVE-1124), so the call
            // fails visibly instead of running unapproved.
            let error = "this tool needs a user action that the OpenAI Agents API backend cannot wait for yet".to_string();
            self.emitter
                .emit(EventRequest::new(
                    self.template.context.session_id,
                    self.event_context.clone(),
                    ToolCompletedData::failure(
                        call.call_id.clone(),
                        call.name.clone(),
                        "blocked".to_string(),
                        error.clone(),
                        None,
                    ),
                ))
                .await
                .map_err(ledger_error)?;
            return Ok(Err(error));
        }
        let result = outcome
            .results
            .into_iter()
            .next()
            .ok_or_else(|| AgentsApiError::Store("tool execution returned no result".into()))?
            .result;
        Ok(match result.error {
            Some(error) => Err(error),
            None => Ok(result_text(result.result.as_ref())),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_capability::CapabilityRef;
    use everruns_core::RuntimeMessage;

    #[test]
    fn only_an_openai_provider_on_the_official_api_offers_an_endpoint() {
        let mut config = ProviderConfig::new(DriverId::OpenAI);
        assert!(official_endpoint(&config).is_none(), "no key, no endpoint");
        config.api_key = Some("sk-test".into());
        let endpoint = official_endpoint(&config).unwrap();
        assert_eq!(endpoint.base_url(), Some("https://api.openai.com/v1"));
        config.base_url = Some("https://llm-gateway.example.com/v1".into());
        assert!(
            official_endpoint(&config).is_none(),
            "gateways keep the native loop"
        );
        config.base_url = Some("http://api.openai.com/v1".into());
        assert!(official_endpoint(&config).is_none());
        config.base_url = Some("https://api.openai.com.evil.example/v1".into());
        assert!(official_endpoint(&config).is_none());
        let mut other = ProviderConfig::new(DriverId::OpenAICompletions);
        other.api_key = Some("sk-test".into());
        assert!(official_endpoint(&other).is_none());
    }

    #[test]
    fn only_the_runtime_capability_selects_the_backend() {
        assert!(!selects_backend(&[]));
        assert!(!selects_backend(&[CapabilityRef::new("current_time")]));
        assert!(selects_backend(&[
            CapabilityRef::new("current_time"),
            CapabilityRef::new("openai_agents_api_runtime"),
        ]));
    }

    #[test]
    fn input_is_the_turn_message_text_and_attachments_fail_closed() {
        let text = RuntimeMessage::user("Who is customer 123?");
        let mut image = RuntimeMessage::user("look");
        image.content.push(ContentPart::Image(
            everruns_core::ImageContentPart::from_base64("aGk=", "image/png"),
        ));
        let messages = vec![text.clone(), image.clone()];
        assert_eq!(
            message_text(&messages, text.id).unwrap(),
            "Who is customer 123?"
        );
        assert!(message_text(&messages, image.id).is_err());
        assert!(message_text(&messages, MessageId::new()).is_err());
    }

    #[test]
    fn recorded_tool_results_are_read_back_as_provider_output() {
        assert_eq!(result_text(Some(&serde_json::json!("plain"))), "plain");
        assert_eq!(
            result_text(Some(&serde_json::json!({"a": 1}))),
            r#"{"a":1}"#
        );
        assert_eq!(result_text(None), "");
    }
}
