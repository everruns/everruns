//! Host wiring: run the Reason activity through the Agents API when the
//! session selected the backend, and the native runtime otherwise.
//!
//! The native worker path is untouched unless every condition holds: the
//! resolved capabilities include `openai_agents_api_runtime` (which the
//! platform strips unless the org has the `openai_agents_api` flag), the
//! turn's model is bound to the official OpenAI API, and the host supplies a
//! durable [`AgentsApiStore`]. A selected session whose model or host cannot
//! serve the backend falls back to the native loop.
//!
//! Everruns policy holds at the remote loop's boundaries (EVE-1124): every
//! tool call crosses the Act pipeline (approval gate, hooks and guardrails,
//! network access, the durable per-call claim) as one batch per required
//! action, a budget that ran out stops the turn before more tools run, a
//! pause parks the Everruns turn while the provider holds its required action
//! open, output guardrails judge every remote assistant message, and a
//! configuration with provider-run tools is refused.
//!
//! Accounting (EVE-1125) belongs to the durable driver: every provider turn
//! that ends is billed once as an `llm.generation`, so a replayed activity
//! cannot bill it twice.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use everruns_core::agents_api_store::ParkReason;
use everruns_core::capabilities::CapabilityRegistry;
use everruns_core::events::{
    CapabilityUsageData, EventContext, EventRequest, ReasonCompletedData, ReasonStartedData,
    ToolCompletedData,
};
use everruns_core::output_guardrail::{
    ArmedGuardrail, OutputGuardrail, OutputGuardrailContext, PostGenerationOutputContext,
    PostGenerationProvider, TrippedGuardrail, evaluate_guardrails,
    evaluate_post_generation_guardrails,
};
use everruns_core::{
    AssembledTurnContext, ContentPart, DecisionsService, EventEmitter, MessageRetriever,
    RuntimeAgent, RuntimeMessageRole, UtilityLlmService,
};
use everruns_engine::{ActOutcome, ActResult, NativeExecutionCounts, ReasonInput, ReasonResult};
use everruns_provider::BearerAuth;
use everruns_provider::driver_registry::{DriverId, ProviderConfig};
use everruns_provider::error::{AgentLoopError, Result};
use everruns_provider::openai_hosted_tools::OPENAI_HOSTED_TOOLS_OPTION;
use everruns_provider::runtime_provider::ProviderEndpoint;
use everruns_provider::tool_types::ToolCall;
use everruns_provider::typed_id::{MessageId, SessionId};
use everruns_provider::user_facing_error::{
    ErrorDisclosure, UserFacingError, UserFacingErrorContext, classify_runtime_error_message, codes,
};
use everruns_provider::{
    ASK_USER_TOOL_NAME, FormElicitationRequired, ToolApprovalRequired, UrlElicitationRequired,
};

use super::durable::{
    AgentsApiFunctionExecutor, AgentsApiLedger, AgentsApiOutputPolicy, AgentsApiTurnDriver,
    AgentsApiTurnOutcome, AgentsApiTurnRequest, FunctionBatch, FunctionOutcome,
};
use super::{AgentsApiClient, AgentsApiError, RUNTIME_CAPABILITY_ID, build_session_config};

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
    let emitter = adapter.event_emitter();
    let event_context = EventContext::from_execution_context(&input.context);
    let started = std::time::Instant::now();
    emit_best_effort(
        &emitter,
        EventRequest::new(
            input.context.session_id,
            event_context.clone(),
            ReasonStartedData {
                harness_id: input.harness_id,
                agent_id: input.agent_id,
                metadata: None,
            },
        ),
        "reason.started",
    )
    .await;
    let registry = adapter.capability_registry();
    // Capability attribution, as the native reason reports it every iteration.
    let records = everruns_engine::capability_usage_records(
        &registry,
        &assembled.resolved_capability_configs,
        &assembled.runtime_agent.tools,
    );
    if !records.is_empty() {
        emit_best_effort(
            &emitter,
            EventRequest::new(
                input.context.session_id,
                event_context.clone(),
                CapabilityUsageData { records },
            ),
            "capability.usage",
        )
        .await;
    }

    let outcome = match prepare_request(input, assembled, org_id, input_text, event_context.clone())
    {
        Ok(request) => {
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
                org_id,
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
            if let Some(policy) = HostOutputPolicy::new(
                &registry,
                assembled,
                adapter.utility_llm_service(),
                adapter.decisions(),
            ) {
                driver = driver.with_output_policy(Arc::new(policy));
            }
            if let Some(cancellation) = adapter.turn_cancellation() {
                driver = driver.with_cancellation(cancellation);
            }
            driver.run(&request).await
        }
        Err(violation) => {
            tracing::warn!(
                session_id = %input.context.session_id,
                %violation,
                "Agents API backend refused a configuration it cannot police"
            );
            Ok(AgentsApiTurnOutcome::Failed {
                code: Some("policy_unenforceable".to_string()),
                message: violation.to_string(),
                policy: true,
            })
        }
    };
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
        // Counts and usage are reported once, by the reason that completes.
        Ok(AgentsApiTurnOutcome::Paused) => ReasonResult {
            native_counts: Some(NativeExecutionCounts {
                llm_calls: 0,
                tool_calls: 0,
            }),
            success: true,
            waiting_for_tool_results: true,
            tool_definitions: assembled.runtime_agent.tools.clone(),
            max_iterations: assembled.runtime_agent.max_iterations,
            locale: assembled.resolved_locale.clone(),
            network_access: assembled.runtime_agent.network_access.clone(),
            parallel_tool_calls: assembled.runtime_agent.parallel_tool_calls,
            ..ReasonResult::default()
        },
        Ok(AgentsApiTurnOutcome::Failed {
            code,
            message,
            policy,
        }) => {
            // An Everruns-authored failure (budget, policy, expired pause) is
            // user-facing as is, so the canonical error classification
            // (e.g. budget exhausted) still recognizes it.
            let user_facing = (!policy)
                .then(|| {
                    lifecycle_user_error(
                        code.as_deref(),
                        &message,
                        &assembled.model.provider_type.to_string(),
                        &assembled.model.model,
                    )
                })
                .flatten();
            let error = match (policy, code) {
                (true, _) => message,
                (false, Some(code)) => {
                    format!("OpenAI Agents API turn failed ({code}): {message}")
                }
                (false, None) => format!("OpenAI Agents API turn failed: {message}"),
            };
            match user_facing {
                // A lifecycle failure carries its stable code, filtered through
                // the session's error-disclosure ceiling like a native failure.
                Some(source) => {
                    let disclosure = error_disclosure(&registry, assembled);
                    let user_error = source.apply_disclosure(disclosure, Some(&error));
                    ReasonResult {
                        success: false,
                        text: user_error.fallback_message(),
                        error: Some(error),
                        user_facing_error: Some(user_error),
                        error_disclosure: Some(disclosure),
                        max_iterations: assembled.runtime_agent.max_iterations,
                        ..ReasonResult::default()
                    }
                }
                None => ReasonResult {
                    success: false,
                    text: error.clone(),
                    error: Some(error),
                    max_iterations: assembled.runtime_agent.max_iterations,
                    ..ReasonResult::default()
                },
            }
        }
        Ok(AgentsApiTurnOutcome::Cancelled) | Err(AgentsApiError::Cancelled) => {
            return Err(AgentLoopError::Cancelled);
        }
        Err(error) => return Err(to_loop_error(error)),
    };
    let completed = if result.success {
        ReasonCompletedData::success(
            &result.text,
            result.waiting_for_tool_results,
            0,
            duration_ms,
            result.usage.clone(),
        )
    } else {
        ReasonCompletedData::failure(result.error.clone().unwrap_or_default(), duration_ms)
    };
    emit_best_effort(
        &emitter,
        EventRequest::new(input.context.session_id, event_context, completed),
        "reason.completed",
    )
    .await;
    Ok(Some(result))
}

async fn emit_best_effort(emitter: &Arc<dyn EventEmitter>, event: EventRequest, kind: &str) {
    if let Err(error) = emitter.emit(event).await {
        tracing::warn!(%error, kind, "Agents API backend: failed to emit event");
    }
}

/// Build the provider request, refusing configurations whose policy the
/// backend cannot enforce.
fn prepare_request(
    input: &ReasonInput,
    assembled: &AssembledTurnContext,
    org_id: i64,
    input_text: String,
    event_context: EventContext,
) -> std::result::Result<AgentsApiTurnRequest, AgentsApiError> {
    ensure_runtime_policy(&assembled.runtime_agent)?;
    let config = build_session_config(&assembled.runtime_agent, "", None)?;
    config.ensure_enforceable()?;
    Ok(AgentsApiTurnRequest {
        org_id,
        session_id: input.context.session_id,
        turn_id: input.context.turn_id,
        input_message_id: input.context.input_message_id,
        iteration: input.iteration,
        input_text,
        config,
        event_context,
        provider: Some(assembled.model.provider_type.to_string()),
        provider_key: Some(assembled.model.provider.to_string()),
        tools: assembled
            .runtime_agent
            .tools
            .iter()
            .map(Into::into)
            .collect(),
    })
}

/// Refuse provider-executed tools the capability set asked for. OpenAI-hosted
/// tools (web search, code interpreter, hosted MCP) would run inside the
/// managed harness with no approval gate, pre-tool guardrail, network policy,
/// or Everruns credentials boundary, so the turn fails instead of silently
/// dropping or forwarding them.
// THREAT[TM-LLM-043]: no provider-run tool on the managed harness.
fn ensure_runtime_policy(agent: &RuntimeAgent) -> std::result::Result<(), AgentsApiError> {
    if agent
        .driver_options
        .contains_key(OPENAI_HOSTED_TOOLS_OPTION)
    {
        return Err(AgentsApiError::PolicyViolation(
            "OpenAI-hosted tools (web search, code interpreter, hosted MCP) run outside Everruns' tool pipeline; remove those capabilities or use the native runtime".to_string(),
        ));
    }
    Ok(())
}

/// The Agents API endpoint for the turn's provider, with that provider's own
/// key. Only the OpenAI Responses driver on the official API qualifies; a
/// gateway, Azure, or another driver keeps the native loop.
// THREAT[TM-LLM-043]: the loop leaves the platform only through the official
// OpenAI API, billed to the session's own provider credentials.
pub fn official_endpoint(config: &ProviderConfig) -> Option<ProviderEndpoint> {
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

/// The stable user-facing error of a provider failure (EVE-1126). Codes the
/// driver assigned ([`super::lifecycle`]) are reported as is; a failure the
/// provider reported on the turn is mapped by its code, then by its text.
fn lifecycle_user_error(
    code: Option<&str>,
    message: &str,
    provider: &str,
    model: &str,
) -> Option<UserFacingError> {
    let code = code?;
    let context = UserFacingErrorContext::default()
        .with_provider(provider)
        .with_model_id(model);
    let user_error = if super::lifecycle::LIFECYCLE_CODES.contains(&code) {
        UserFacingError::new(code)
    } else if code.contains("model") {
        UserFacingError::new(codes::MODEL_UNAVAILABLE)
    } else {
        classify_runtime_error_message(&format!("{code}: {message}"), &context)
    };
    Some(match user_error.code.as_str() {
        codes::MODEL_UNAVAILABLE => user_error.with_field("model_id", model),
        codes::PROVIDER_MISCONFIGURED | codes::PROVIDER_QUOTA_EXHAUSTED => {
            user_error.with_field("provider", provider)
        }
        _ => user_error,
    })
}

/// The session's error-disclosure mode: the capability ceiling, narrowed by
/// the turn input's own control, as the native reason resolves it.
// THREAT[TM-LLM-024]: a client control may narrow disclosure, never widen it.
fn error_disclosure(
    registry: &CapabilityRegistry,
    assembled: &AssembledTurnContext,
) -> ErrorDisclosure {
    let ceiling = assembled
        .resolved_capability_configs
        .iter()
        .find_map(|config| {
            registry
                .get(config.capability_id())?
                .error_disclosure(config.config_value())
        })
        .unwrap_or_default();
    assembled
        .messages
        .iter()
        .rev()
        .find(|message| message.role == RuntimeMessageRole::User)
        .and_then(|message| message.controls.as_ref()?.error_disclosure.as_deref())
        .and_then(ErrorDisclosure::parse)
        .map_or(ceiling, |requested| requested.min(ceiling))
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

fn tool_result_value(
    result: &everruns_provider::tool_types::ToolResult,
) -> std::result::Result<String, String> {
    match &result.error {
        Some(error) => Err(error.clone()),
        None => Ok(result_text(result.result.as_ref())),
    }
}

/// Executes client functions through the ordinary Act pipeline: permission
/// checks, the approval gate, pre/post tool hooks and guardrails, network
/// access, the durable per-call claim, and the canonical `tool.completed`.
struct HostFunctionExecutor<A: crate::RuntimeHostAdapter> {
    adapter: A,
    org_id: i64,
    template: everruns_engine::ActInput,
    emitter: Arc<dyn EventEmitter>,
    event_context: EventContext,
}

/// Budget code for an Everruns budget the session ran out of mid-turn.
const BUDGET_EXHAUSTED_STOP: &str = "budget_exhausted";
/// Budget code for a budget paused at its soft limit mid-turn.
const BUDGET_PAUSED_STOP: &str = "budget_paused";
/// The agent or harness was archived or deleted mid-turn.
const DEPENDENCY_STOP: &str = "dependency_unavailable";

impl<A: crate::RuntimeHostAdapter> HostFunctionExecutor<A> {
    /// The budget gate the native loop applies between atoms, applied before
    /// every tool batch the managed harness asks for. Budget checks are
    /// post-hoc everywhere in Everruns, so a checker error fails open.
    async fn budget_stop(&self) -> Option<(String, String)> {
        let checker = self
            .adapter
            .budget_checker(self.org_id, self.template.agent_id)?;
        let session_id = self.template.context.session_id.to_string();
        match checker.check_budgets(&session_id).await {
            Ok(response) => budget_stop_for_status(&response.status),
            Err(error) => {
                tracing::warn!(%error, %session_id, "Agents API backend: budget check failed; continuing");
                None
            }
        }
    }

    /// Record a failed result for each call the turn stops before running.
    async fn fail_calls(
        &self,
        calls: &[ToolCall],
        status: &str,
        message: &str,
    ) -> std::result::Result<(), AgentsApiError> {
        for call in calls {
            self.emitter
                .emit(EventRequest::new(
                    self.template.context.session_id,
                    self.event_context.clone(),
                    ToolCompletedData::failure(
                        call.id.clone(),
                        call.name.clone(),
                        status.to_string(),
                        message.to_string(),
                        None,
                    ),
                ))
                .await
                .map_err(ledger_error)?;
        }
        Ok(())
    }
}

fn budget_stop_for_status(status: &str) -> Option<(String, String)> {
    match status {
        "exhausted" => Some((
            BUDGET_EXHAUSTED_STOP.to_string(),
            "Budget exhausted. Increase the budget to continue.".to_string(),
        )),
        "paused" => Some((
            BUDGET_PAUSED_STOP.to_string(),
            "Budget paused. Increase or resume the budget to continue.".to_string(),
        )),
        _ => None,
    }
}

#[async_trait]
impl<A: crate::RuntimeHostAdapter> AgentsApiFunctionExecutor for HostFunctionExecutor<A> {
    async fn execute(
        &self,
        calls: &[ToolCall],
    ) -> std::result::Result<FunctionBatch, AgentsApiError> {
        if let Some((code, message)) = self.budget_stop().await {
            self.fail_calls(calls, "blocked", &message).await?;
            return Ok(FunctionBatch::Halt { code, message });
        }
        let mut input = self.template.clone();
        input.context.exec_id = everruns_provider::typed_id::ExecId::new();
        input.tool_calls = calls.to_vec();
        let result = crate::execute_act_activity(&self.adapter, input)
            .await
            .map_err(|error| AgentsApiError::Store(error.user_facing_message()))?;
        if result.blocked {
            // The agent or harness went away mid-turn; the act already
            // recorded the dependency failure on the turn.
            let message = "This agent is no longer available, so its tools cannot run.";
            self.fail_calls(calls, "blocked", message).await?;
            return Ok(FunctionBatch::Halt {
                code: DEPENDENCY_STOP.to_string(),
                message: message.to_string(),
            });
        }
        let session_id = self.template.context.session_id;
        let outcome = act_outcome(&result);
        let pauses = if outcome.waiting_for_tool_results {
            let hints = crate::turn_strategy::resolve_pause_hints(
                &self.adapter,
                self.org_id,
                session_id,
                outcome,
            )
            .await;
            everruns_engine::act_pauses_turn(
                outcome,
                hints.setup_connection,
                hints.url_elicitation,
                hints.ask_user,
            )
        } else {
            false
        };
        if outcome.waiting_for_ask_user && !pauses {
            // No client can draw the question: answer it with its declared
            // defaults, exactly as the native planner does (EVE-1057).
            let calls =
                crate::turn_strategy::pending_ask_user_calls_from_slice(&result.client_tool_calls);
            crate::turn_strategy::perform_effects(
                &self.adapter,
                self.org_id,
                session_id,
                vec![
                    everruns_engine::TurnLifecycleEffect::ResolveAskUserUnattended {
                        turn_id: Some(self.template.context.turn_id),
                        input_message_id: self.template.context.input_message_id,
                        calls,
                    },
                ],
            )
            .await
            .map_err(|error| AgentsApiError::Store(error.user_facing_message()))?;
        }
        let mut outcomes = Vec::with_capacity(calls.len());
        for (call, classified) in calls.iter().zip(classify_act(calls, &result, pauses)) {
            outcomes.push(match classified {
                Classified::Outcome(outcome) => outcome,
                Classified::Unattended => {
                    FunctionOutcome::Done(self.ledger_result(&call.id).await?.unwrap_or_else(
                        || Err("Nobody could answer this question in this session.".to_string()),
                    ))
                }
            });
        }
        Ok(FunctionBatch::Outcomes(outcomes))
    }
}

impl<A: crate::RuntimeHostAdapter> HostFunctionExecutor<A> {
    async fn ledger_result(
        &self,
        call_id: &str,
    ) -> std::result::Result<Option<std::result::Result<String, String>>, AgentsApiError> {
        HostLedger {
            emitter: self.emitter.clone(),
            messages: self.adapter.message_store(),
        }
        .tool_result(self.template.context.session_id, call_id)
        .await
    }
}

/// The act outcome the native planner decides its pause from.
fn act_outcome(result: &ActResult) -> ActOutcome {
    ActOutcome {
        blocked: result.blocked,
        waiting_for_tool_results: result.waiting_for_tool_results,
        waiting_for_url_elicitation: result.waiting_for_url_elicitation,
        waiting_for_ask_user: result
            .client_tool_calls
            .iter()
            .any(|call| call.name == ASK_USER_TOOL_NAME),
        waiting_for_tool_approval: everruns_engine::has_pending_tool_approval(
            &result.client_tool_calls,
        ),
    }
}

/// How one call left a batch, before ledger lookups.
#[derive(Debug, PartialEq)]
enum Classified {
    Outcome(FunctionOutcome),
    /// An `ask_user` call answered unattended; its result is in the log.
    Unattended,
}

/// Map an act's results to per-call outcomes. When the act pauses the turn,
/// a call the approval gate deferred, a call that needs a connection or an
/// MCP elicitation, and a client-side call are parked; everything else has a
/// result to submit. A gated call never yields its `tool_approval_required`
/// result to the provider while the pause can still be answered.
fn classify_act(calls: &[ToolCall], result: &ActResult, pauses: bool) -> Vec<Classified> {
    calls
        .iter()
        .map(|call| {
            let executed = result
                .results
                .iter()
                .find(|executed| executed.tool_call.id == call.id);
            let is_client = result
                .client_tool_calls
                .iter()
                .any(|client| client.id == call.id);
            match executed {
                Some(executed) if pauses => {
                    if let Some(request) = ToolApprovalRequired::from_tool_result(&executed.result)
                    {
                        Classified::Outcome(FunctionOutcome::Parked(ParkReason::Approval {
                            request_call_id: request.request_call().id,
                        }))
                    } else if executed.connection_required.is_some()
                        || UrlElicitationRequired::from_tool_result(&executed.result)
                            .is_some_and(|elicitation| !elicitation.declined)
                        || FormElicitationRequired::from_tool_result(&executed.result).is_some()
                    {
                        Classified::Outcome(FunctionOutcome::Parked(ParkReason::Retry))
                    } else {
                        Classified::Outcome(FunctionOutcome::Done(tool_result_value(
                            &executed.result,
                        )))
                    }
                }
                Some(executed) => {
                    Classified::Outcome(FunctionOutcome::Done(tool_result_value(&executed.result)))
                }
                None if is_client && pauses => {
                    Classified::Outcome(FunctionOutcome::Parked(ParkReason::ClientResult))
                }
                None if is_client && call.name == ASK_USER_TOOL_NAME => Classified::Unattended,
                None if is_client => Classified::Outcome(FunctionOutcome::Done(Err(
                    "This tool runs on the client, and no client in this session can answer it."
                        .to_string(),
                ))),
                None => Classified::Outcome(FunctionOutcome::Done(Err(
                    "Tool execution returned no result.".to_string(),
                ))),
            }
        })
        .collect()
}

/// The session's output guardrails, applied to remote assistant messages the
/// way the native reason applies them to its own stream.
struct HostOutputPolicy {
    system_prompt: String,
    streaming: Vec<(String, serde_json::Value, Arc<dyn OutputGuardrail>)>,
    post_generation: Vec<PostGenerationProvider>,
    armed: Mutex<HashMap<String, Vec<ArmedGuardrail>>>,
    utility_llm_service: Option<Arc<dyn UtilityLlmService>>,
    decisions: Option<Arc<dyn DecisionsService>>,
}

impl HostOutputPolicy {
    /// `None` when no resolved capability contributes an output guardrail.
    fn new(
        registry: &CapabilityRegistry,
        assembled: &AssembledTurnContext,
        utility_llm_service: Option<Arc<dyn UtilityLlmService>>,
        decisions: Option<Arc<dyn DecisionsService>>,
    ) -> Option<Self> {
        let mut streaming = Vec::new();
        let mut post_generation = Vec::new();
        for config in &assembled.resolved_capability_configs {
            let capability_id = config.capability_id();
            let Some(capability) = registry.get(capability_id) else {
                continue;
            };
            for guardrail in capability.output_guardrails() {
                streaming.push((
                    capability_id.to_string(),
                    config.config_value().clone(),
                    guardrail,
                ));
            }
            for provider in capability.post_output_guardrails_with_config(config.config_value()) {
                post_generation.push(PostGenerationProvider {
                    capability_id: capability_id.to_string(),
                    provider,
                });
            }
        }
        if streaming.is_empty() && post_generation.is_empty() {
            return None;
        }
        Some(Self {
            system_prompt: assembled.runtime_agent.system_prompt.clone(),
            streaming,
            post_generation,
            armed: Mutex::new(HashMap::new()),
            utility_llm_service,
            decisions,
        })
    }

    fn arm(&self) -> Vec<ArmedGuardrail> {
        self.streaming
            .iter()
            .filter_map(|(capability_id, config, guardrail)| {
                let run = guardrail.arm(&OutputGuardrailContext {
                    system_prompt: &self.system_prompt,
                    config,
                })?;
                Some(ArmedGuardrail {
                    capability_id: capability_id.clone(),
                    guardrail_id: guardrail.id().to_string(),
                    run,
                })
            })
            .collect()
    }
}

#[async_trait]
impl AgentsApiOutputPolicy for HostOutputPolicy {
    fn withholds_deltas(&self) -> bool {
        !self.post_generation.is_empty()
    }

    fn check_delta(
        &self,
        item_id: &str,
        accumulated: &str,
        delta: &str,
    ) -> Option<TrippedGuardrail> {
        if self.streaming.is_empty() {
            return None;
        }
        let mut armed = self.armed.lock().ok()?;
        let runs = armed
            .entry(item_id.to_string())
            .or_insert_with(|| self.arm());
        evaluate_guardrails(runs, accumulated, delta)
    }

    async fn check_message(&self, item_id: &str, text: &str) -> Option<TrippedGuardrail> {
        if let Ok(mut armed) = self.armed.lock() {
            armed.remove(item_id);
        }
        // Fresh streaming runs over the whole text: the live stream may have
        // been missed (a reconnect, a restart), the saved item never is.
        let mut runs = self.arm();
        if let Some(trip) = evaluate_guardrails(&mut runs, text, text) {
            return Some(trip);
        }
        if self.post_generation.is_empty() || text.is_empty() {
            return None;
        }
        evaluate_post_generation_guardrails(
            &self.post_generation,
            &PostGenerationOutputContext {
                system_prompt: &self.system_prompt,
                message_text: text,
                utility_llm_service: self.utility_llm_service.as_ref(),
                decisions: self.decisions.as_ref(),
            },
        )
        .await
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
    fn provider_failures_map_to_stable_user_facing_codes() {
        let map = |code: Option<&str>, message: &str| {
            lifecycle_user_error(code, message, "openai", "gpt-6-astra")
        };
        assert_eq!(
            map(None, "no code"),
            None,
            "text classification stays as before"
        );
        let lost = map(Some(codes::PROVIDER_SESSION_UNAVAILABLE), "gone").unwrap();
        assert_eq!(lost.code, codes::PROVIDER_SESSION_UNAVAILABLE);
        assert!(lost.fallback_message().contains("new provider session"));
        let model = map(Some("model_not_found"), "retired").unwrap();
        assert_eq!(model.code, codes::MODEL_UNAVAILABLE);
        assert_eq!(model.fields["model_id"], "gpt-6-astra");
        let credentials = map(Some(codes::PROVIDER_MISCONFIGURED), "HTTP 401").unwrap();
        assert_eq!(credentials.fields["provider"], "openai");
        let quota = map(
            Some("insufficient_quota"),
            "You exceeded your current quota",
        )
        .unwrap();
        assert_eq!(quota.code, codes::PROVIDER_QUOTA_EXHAUSTED);
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

    fn call(id: &str, name: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: serde_json::json!({"customer_id": "123"}),
        }
    }

    fn executed(
        call: &ToolCall,
        result: Option<serde_json::Value>,
        error: Option<&str>,
    ) -> everruns_engine::ToolCallResult {
        everruns_engine::ToolCallResult {
            tool_call: call.clone(),
            result: everruns_provider::tool_types::ToolResult {
                tool_call_id: call.id.clone(),
                result,
                images: None,
                error: error.map(str::to_string),
                connection_required: None,
                raw_output: None,
            },
            success: error.is_none(),
            status: if error.is_none() { "success" } else { "error" }.into(),
            connection_required: None,
            determinism_fatal: None,
        }
    }

    fn act(results: Vec<everruns_engine::ToolCallResult>, client: Vec<ToolCall>) -> ActResult {
        ActResult {
            results,
            completed: true,
            success_count: 0,
            error_count: 0,
            waiting_for_tool_results: !client.is_empty(),
            waiting_for_url_elicitation: false,
            blocked: false,
            client_tool_calls: client,
            client_tool_definitions: vec![],
        }
    }

    fn approval_required(call: &ToolCall) -> serde_json::Value {
        serde_json::json!({
            "code": everruns_provider::TOOL_APPROVAL_REQUIRED_CODE,
            "error": "waiting for approval",
            "tool_call_id": call.id,
            "tool": call.name,
            "arguments": call.arguments,
            "fingerprint": "fp",
            "risk": "destructive",
            "mode": "always_ask",
            "asked_at": "2026-10-01T00:00:00Z",
            "expires_at": "2026-10-01T00:15:00Z",
        })
    }

    #[test]
    fn a_gated_call_parks_on_its_request_and_never_yields_its_placeholder() {
        let gated = call("call_1", "delete_customer");
        let plain = call("call_2", "lookup_customer");
        let request = ToolApprovalRequired::from_tool_result(
            &executed(
                &gated,
                Some(approval_required(&gated)),
                Some("approval required"),
            )
            .result,
        )
        .unwrap()
        .request_call();
        let result = act(
            vec![
                executed(
                    &gated,
                    Some(approval_required(&gated)),
                    Some("approval required"),
                ),
                executed(&plain, Some(serde_json::json!("Ada")), None),
            ],
            vec![request.clone()],
        );
        assert!(act_outcome(&result).waiting_for_tool_approval);
        let calls = [gated, plain];
        assert_eq!(
            classify_act(&calls, &result, true),
            vec![
                Classified::Outcome(FunctionOutcome::Parked(ParkReason::Approval {
                    request_call_id: request.id,
                })),
                Classified::Outcome(FunctionOutcome::Done(Ok("Ada".into()))),
            ]
        );
    }

    #[test]
    fn client_side_calls_park_only_when_the_turn_pauses() {
        let client = call("call_1", "pick_color");
        let ask = call("call_2", ASK_USER_TOOL_NAME);
        let result = act(vec![], vec![client.clone(), ask.clone()]);
        let calls = [client, ask];
        assert_eq!(
            classify_act(&calls, &result, true),
            vec![
                Classified::Outcome(FunctionOutcome::Parked(ParkReason::ClientResult)),
                Classified::Outcome(FunctionOutcome::Parked(ParkReason::ClientResult)),
            ]
        );
        // No client can answer: the call fails visibly, and a question gets
        // the unattended answer from the log.
        let unanswered = classify_act(&calls, &result, false);
        assert!(matches!(
            &unanswered[0],
            Classified::Outcome(FunctionOutcome::Done(Err(_)))
        ));
        assert_eq!(unanswered[1], Classified::Unattended);
    }

    #[test]
    fn a_connection_prompt_parks_for_a_retry() {
        let needs = call("call_1", "send_email");
        let mut needs_connection = executed(&needs, None, Some("connect gmail"));
        needs_connection.connection_required = Some(
            everruns_provider::ConnectionRequired::provider_only("gmail"),
        );
        let result = act(vec![needs_connection.clone()], vec![]);
        assert_eq!(
            classify_act(std::slice::from_ref(&needs), &result, true),
            vec![Classified::Outcome(FunctionOutcome::Parked(
                ParkReason::Retry
            ))]
        );
        // Without a pause the failure is the result, as in the native loop.
        assert_eq!(
            classify_act(&[needs], &result, false),
            vec![Classified::Outcome(FunctionOutcome::Done(Err(
                "connect gmail".into()
            )))]
        );
    }

    #[test]
    fn budget_statuses_that_stop_the_turn_use_canonical_copy() {
        let (code, message) = budget_stop_for_status("exhausted").unwrap();
        assert_eq!(code, BUDGET_EXHAUSTED_STOP);
        assert_eq!(
            everruns_provider::classify_runtime_error_message(
                &message,
                &everruns_provider::UserFacingErrorContext::default()
            )
            .code,
            everruns_provider::user_facing_error_codes::BUDGET_EXHAUSTED
        );
        let (code, message) = budget_stop_for_status("paused").unwrap();
        assert_eq!(code, BUDGET_PAUSED_STOP);
        assert_eq!(
            everruns_provider::classify_runtime_error_message(
                &message,
                &everruns_provider::UserFacingErrorContext::default()
            )
            .code,
            everruns_provider::user_facing_error_codes::BUDGET_PAUSED
        );
        for status in ["active", "warning", "no_budgets"] {
            assert!(budget_stop_for_status(status).is_none());
        }
    }

    #[test]
    fn openai_hosted_tools_are_refused_not_dropped() {
        let mut agent = RuntimeAgent::new("Test.", "gpt-6-astra");
        assert!(ensure_runtime_policy(&agent).is_ok());
        agent.driver_options.insert(
            OPENAI_HOSTED_TOOLS_OPTION.to_string(),
            serde_json::json!({"web_search": {}}),
        );
        assert!(matches!(
            ensure_runtime_policy(&agent),
            Err(AgentsApiError::PolicyViolation(_))
        ));
    }
}
