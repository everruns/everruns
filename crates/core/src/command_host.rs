// Command host facilities (see knowledge/project/commands.md, EVE-543).
//
// Decisions:
// - /btw-style commands (out-of-band LLM call over the session's assembled
//   context) are implemented inside capabilities. The host hands
//   `Capability::execute_command` a `CommandHost` with two composable
//   facilities: turn-context assembly and a tool-less session completion.
// - Credentials never cross the trait boundary: `CommandTurnContext` exposes
//   the model name and provider type for error classification but never API
//   keys; driver creation stays inside the store-backed implementation.
// - Completions are out-of-band by construction: nothing is persisted to
//   messages or events.

use crate::capabilities::CapabilityRegistry;
use crate::command::CommandResult;
use crate::driver_registry::{
    BoxedChatDriver, DriverRegistry, LlmCallConfig, LlmMessage, LlmMessageRole, LlmResponseStream,
    ToolSearchConfig,
};
use crate::error::{AgentLoopError, Result};
use crate::message::{Controls, Message, MessageRole, patch_dangling_tool_calls};
use crate::message_retriever::MessageRetriever;
use crate::runtime_context::{AssembledTurnContext, inspect_turn_context};
use crate::session::ExecutionSession;
use crate::traits::{
    AgentStore, HarnessStore, ImageResolver, ProviderStore, ResolvedImage, ResolvedModel,
    SessionFileSystem, SessionStore,
};
use crate::typed_id::SessionId;
use crate::user_facing_error::{UserFacingErrorContext, classify_runtime_error_message};
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

/// Credential-free snapshot of the session's assembled turn context for
/// command execution. Mirrors what a main turn sees (capability message
/// filters applied, merged harness/agent/session system prompt) without
/// exposing provider credentials.
///
/// THREAT[TM-LLM-023]: this view must stay credential-free. Provider keys and
/// base URLs live only inside the host implementation; do not add
/// `ResolvedModel` or other key-bearing types here.
#[derive(Debug, Clone)]
pub struct CommandTurnContext {
    /// Session the command is executing against.
    pub session: ExecutionSession,
    /// Conversation messages after capability message filters.
    pub messages: Vec<Message>,
    /// Merged system prompt including capability contributions.
    pub system_prompt: String,
    /// Resolved model name (no credentials).
    pub model: String,
    /// Resolved provider type string, for user-facing error classification.
    pub provider_type: String,
    /// Locale resolved from message controls or session defaults.
    pub resolved_locale: Option<String>,
}

/// Request for a tool-less, out-of-band completion against the session's
/// resolved model. The capability supplies core messages; the host owns
/// provider conversion (image resolution, external-actor prefixes,
/// dangling-tool-call patching) and credentials.
#[derive(Debug, Clone, Default)]
pub struct SessionCompletionRequest {
    /// System prompt stack, sent as system messages in order. Empty entries
    /// are skipped.
    pub system_prompts: Vec<String>,
    /// Conversation messages to complete against.
    pub messages: Vec<Message>,
    /// Per-invocation controls: model override (resolved through the same
    /// org-scoped store as a main turn) and reasoning effort.
    pub controls: Option<Controls>,
    /// Extra metadata forwarded to the provider call (e.g. `command: btw`).
    /// The host adds `session_id` itself.
    pub metadata: HashMap<String, String>,
}

/// Successful completion result.
#[derive(Debug, Clone)]
pub struct SessionCompletion {
    /// Trimmed, non-empty completion text.
    pub text: String,
}

/// Streaming variant of [`SessionCompletion`]: provider stream events for
/// progressive output, aligned with [`crate::utility_llm::UtilityLlmService`]'s
/// streaming shape.
pub struct SessionCompletionStream {
    /// Provider stream events (`TextDelta`, thinking deltas, `Done` metadata).
    pub events: LlmResponseStream,
    /// Classification context (provider, model) for mid-stream errors — pair
    /// with [`classify_runtime_error_message`] to produce stable user-facing
    /// error codes, mirroring [`SessionCompletionError::into_command_result`].
    pub context: UserFacingErrorContext,
}

impl std::fmt::Debug for SessionCompletionStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionCompletionStream")
            .field("context", &self.context)
            .finish()
    }
}

/// Completion failure.
#[derive(Debug)]
pub enum SessionCompletionError {
    /// Request-level failure (e.g. unknown model override). Capabilities
    /// should surface these as hard errors, not classified command results.
    InvalidRequest(AgentLoopError),
    /// The host does not implement [`CommandHost::completion_stream`].
    /// A stable, matchable signal so commands can fall back to
    /// [`CommandHost::completion`] without string matching.
    StreamingUnsupported,
    /// Provider/runtime failure carrying the resolved provider/model identity
    /// so callers can classify it into stable user-facing error codes.
    Completion {
        /// Formatted error chain.
        error: String,
        /// Classification context (provider, model).
        context: UserFacingErrorContext,
    },
}

impl SessionCompletionError {
    /// Convert into the result a command should return: provider failures
    /// become a classified `success: false` `CommandResult` (mirroring chat
    /// error messages so the UI can localize), request-level failures bubble.
    pub fn into_command_result(self) -> Result<CommandResult> {
        match self {
            Self::InvalidRequest(error) => Err(error),
            Self::StreamingUnsupported => Err(AgentLoopError::config(
                "command host does not support streaming completions",
            )),
            Self::Completion { error, context } => {
                let classified = classify_runtime_error_message(&error, &context);
                Ok(CommandResult {
                    success: false,
                    message: classified.fallback_message(),
                    error_code: Some(classified.code.clone()),
                    error_fields: classified.error_fields(),
                })
            }
        }
    }
}

/// Host facilities available to [`crate::capabilities::Capability::execute_command`].
///
/// Hosts that cannot provide these facilities use [`DisabledCommandHost`] so
/// misconfiguration surfaces at invocation time with a clear error.
#[async_trait]
pub trait CommandHost: Send + Sync {
    /// Assemble the same merged context a main turn would see.
    async fn turn_context(&self) -> Result<CommandTurnContext>;

    /// Run a tool-less completion against the session's resolved model (or a
    /// `Controls` model override). Persists nothing.
    async fn completion(
        &self,
        request: SessionCompletionRequest,
    ) -> std::result::Result<SessionCompletion, SessionCompletionError>;

    /// Streaming variant of [`Self::completion`] for commands that surface
    /// progressive output. Same request semantics, same out-of-band guarantee:
    /// nothing is persisted.
    ///
    /// Default: [`SessionCompletionError::StreamingUnsupported`]. Hosts that
    /// can stream override this; commands match on that variant to fall back
    /// to [`Self::completion`].
    async fn completion_stream(
        &self,
        _request: SessionCompletionRequest,
    ) -> std::result::Result<SessionCompletionStream, SessionCompletionError> {
        Err(SessionCompletionError::StreamingUnsupported)
    }
}

/// Host stub for embedders that dispatch commands without turn-context/LLM
/// facilities. Both methods fail with a clear error.
pub struct DisabledCommandHost;

#[async_trait]
impl CommandHost for DisabledCommandHost {
    async fn turn_context(&self) -> Result<CommandTurnContext> {
        Err(AgentLoopError::config(
            "command host does not provide turn-context access",
        ))
    }

    async fn completion(
        &self,
        _request: SessionCompletionRequest,
    ) -> std::result::Result<SessionCompletion, SessionCompletionError> {
        Err(SessionCompletionError::InvalidRequest(
            AgentLoopError::config("command host does not provide session completions"),
        ))
    }
}

/// Store-backed [`CommandHost`] shared by all hosts (server full/dev mode via
/// worker adapters, in-process runtime via its runtime stores). Reuses
/// `inspect_turn_context` and the reason-path message-building conventions so
/// command completions see exactly what a main turn would.
///
/// One instance is built per command dispatch; the assembled context is
/// memoized so `turn_context()` + `completion()` assemble once.
pub struct StoreCommandHost {
    session_id: SessionId,
    harness_store: Arc<dyn HarnessStore>,
    agent_store: Arc<dyn AgentStore>,
    session_store: Arc<dyn SessionStore>,
    message_retriever: Arc<dyn MessageRetriever>,
    provider_store: Arc<dyn ProviderStore>,
    capability_registry: CapabilityRegistry,
    driver_registry: DriverRegistry,
    image_resolver: Option<Arc<dyn ImageResolver>>,
    file_store: Option<Arc<dyn SessionFileSystem>>,
    assembled: tokio::sync::OnceCell<AssembledTurnContext>,
}

impl StoreCommandHost {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: SessionId,
        harness_store: Arc<dyn HarnessStore>,
        agent_store: Arc<dyn AgentStore>,
        session_store: Arc<dyn SessionStore>,
        message_retriever: Arc<dyn MessageRetriever>,
        provider_store: Arc<dyn ProviderStore>,
        capability_registry: CapabilityRegistry,
        driver_registry: DriverRegistry,
    ) -> Self {
        Self {
            session_id,
            harness_store,
            agent_store,
            session_store,
            message_retriever,
            provider_store,
            capability_registry,
            driver_registry,
            image_resolver: None,
            file_store: None,
            assembled: tokio::sync::OnceCell::new(),
        }
    }

    /// Resolve `image_file` references in messages to inline image data.
    /// Without a resolver, images degrade to placeholder text.
    pub fn with_image_resolver(mut self, image_resolver: Arc<dyn ImageResolver>) -> Self {
        self.image_resolver = Some(image_resolver);
        self
    }

    /// Session filesystem for dynamic system prompt contributions
    /// (AGENTS.md, skills discovery).
    pub fn with_file_store(mut self, file_store: Arc<dyn SessionFileSystem>) -> Self {
        self.file_store = Some(file_store);
        self
    }

    /// Seed the memoized turn context when the host already assembled one
    /// (e.g. the in-process runtime assembles it to resolve command dispatch).
    pub fn with_assembled_context(mut self, assembled: AssembledTurnContext) -> Self {
        self.assembled = tokio::sync::OnceCell::new_with(Some(assembled));
        self
    }

    async fn assembled(&self) -> Result<&AssembledTurnContext> {
        self.assembled
            .get_or_try_init(|| async {
                let session = self
                    .session_store
                    .get_session(self.session_id)
                    .await?
                    .ok_or_else(|| AgentLoopError::session_not_found(self.session_id))?;
                inspect_turn_context(
                    self.harness_store.as_ref(),
                    self.agent_store.as_ref(),
                    self.session_store.as_ref(),
                    self.message_retriever.as_ref(),
                    self.provider_store.as_ref(),
                    &self.capability_registry,
                    self.session_id,
                    session.harness_id,
                    session.agent_id,
                    &[],
                    self.file_store.clone(),
                )
                .await
            })
            .await
    }

    async fn resolve_images(&self, messages: &[Message]) -> HashMap<Uuid, ResolvedImage> {
        let Some(resolver) = &self.image_resolver else {
            return HashMap::new();
        };
        let image_ids: HashSet<Uuid> = messages
            .iter()
            .flat_map(crate::llm_conversions::extract_image_file_ids)
            .collect();
        let mut resolved = HashMap::new();
        for image_id in image_ids {
            if let Ok(Some(image)) = resolver.resolve_image(image_id).await {
                resolved.insert(image_id, image);
            }
        }
        resolved
    }

    /// Resolve the model for a completion: `Controls` override first (looked
    /// up through the same org-scoped store as a main turn), otherwise the
    /// model the assembled context already resolved.
    async fn resolve_completion_model(
        &self,
        controls: Option<&Controls>,
        assembled: &AssembledTurnContext,
    ) -> std::result::Result<ResolvedModel, SessionCompletionError> {
        let requested = controls.and_then(|controls| controls.model_id);
        match requested {
            Some(model_id) if Some(model_id) != assembled.resolved_model_id => self
                .provider_store
                .get_resolved_model(model_id)
                .await
                .map_err(SessionCompletionError::InvalidRequest)?
                .ok_or_else(|| {
                    SessionCompletionError::InvalidRequest(AgentLoopError::config(format!(
                        "Model not found: {model_id}"
                    )))
                }),
            _ => Ok(assembled.model_with_provider.clone()),
        }
    }

    /// Shared preparation for [`CommandHost::completion`] and
    /// [`CommandHost::completion_stream`]: resolve the model, convert
    /// messages, build the tool-less call config, and create the driver.
    async fn prepare_completion(
        &self,
        request: SessionCompletionRequest,
    ) -> std::result::Result<PreparedCompletion, SessionCompletionError> {
        let assembled = self
            .assembled()
            .await
            .map_err(SessionCompletionError::InvalidRequest)?;
        let model = self
            .resolve_completion_model(request.controls.as_ref(), assembled)
            .await?;

        let context = UserFacingErrorContext::default()
            .with_provider(model.provider_type.to_string())
            .with_model_id(model.model.clone());

        let messages = patch_dangling_tool_calls(&request.messages);
        let resolved_images = self.resolve_images(&messages).await;

        let mut llm_messages: Vec<LlmMessage> = request
            .system_prompts
            .iter()
            .filter(|prompt| !prompt.is_empty())
            .map(|prompt| LlmMessage::text(LlmMessageRole::System, prompt.clone()))
            .collect();
        for msg in &messages {
            let mut llm_msg =
                crate::llm_conversions::llm_message_from_message_with_images(msg, &resolved_images);
            if msg.role == MessageRole::User
                && let Some(actor) = &msg.external_actor
            {
                llm_msg.prepend_text_prefix(&format!("[{}] ", actor.display_label()));
            }
            llm_messages.push(llm_msg);
        }

        let mut llm_config_builder =
            crate::llm_conversions::llm_call_config_builder_from_agent(&assembled.runtime_agent)
                .model(&model.model)
                .tools(vec![])
                .tool_search(ToolSearchConfig {
                    enabled: false,
                    threshold: usize::MAX,
                })
                .previous_response_id(None)
                .with_metadata("session_id", self.session_id.to_string());
        if let Some(effort) = request
            .controls
            .as_ref()
            .and_then(|controls| controls.reasoning.as_ref())
            .and_then(|reasoning| reasoning.effort.clone())
            .filter(|value| !value.is_empty())
        {
            llm_config_builder = llm_config_builder.reasoning_effort(effort);
        }
        for (key, value) in &request.metadata {
            llm_config_builder = llm_config_builder.with_metadata(key, value);
        }
        let llm_config = llm_config_builder.build();

        let (_, compatibility_config) = model.canonical_parts();
        let provider_config = self
            .provider_store
            .get_provider_config(&compatibility_config.provider)
            .await
            .map_err(|error| SessionCompletionError::Completion {
                error: error.to_string(),
                context: context.clone(),
            })?
            .unwrap_or(compatibility_config);
        let driver = self
            .driver_registry
            .create_chat_driver(&provider_config)
            .map_err(|error| SessionCompletionError::Completion {
                error: error.to_string(),
                context: context.clone(),
            })?;

        Ok(PreparedCompletion {
            llm_messages,
            llm_config,
            driver,
            context,
        })
    }
}

/// Output of [`StoreCommandHost::prepare_completion`]: everything needed to
/// run the provider call, with credentials confined to the driver.
struct PreparedCompletion {
    llm_messages: Vec<LlmMessage>,
    llm_config: LlmCallConfig,
    driver: BoxedChatDriver,
    context: UserFacingErrorContext,
}

#[async_trait]
impl CommandHost for StoreCommandHost {
    async fn turn_context(&self) -> Result<CommandTurnContext> {
        let assembled = self.assembled().await?;
        Ok(CommandTurnContext {
            session: assembled.session.clone(),
            messages: assembled.messages.clone(),
            system_prompt: assembled.runtime_agent.system_prompt.clone(),
            model: assembled.model_with_provider.model.clone(),
            provider_type: assembled.model_with_provider.provider_type.to_string(),
            resolved_locale: assembled.resolved_locale.clone(),
        })
    }

    async fn completion(
        &self,
        request: SessionCompletionRequest,
    ) -> std::result::Result<SessionCompletion, SessionCompletionError> {
        let prepared = self.prepare_completion(request).await?;
        let completion_error = |error: String| SessionCompletionError::Completion {
            error,
            context: prepared.context.clone(),
        };

        let response = prepared
            .driver
            .chat_completion(
                &crate::ProviderEndpoint::default(),
                prepared.llm_messages,
                &prepared.llm_config,
            )
            .await
            .map_err(|error| completion_error(error.to_string()))?;

        let text = response.text.trim().to_string();
        if text.is_empty() {
            return Err(completion_error(
                "session completion returned an empty response".to_string(),
            ));
        }
        Ok(SessionCompletion { text })
    }

    async fn completion_stream(
        &self,
        request: SessionCompletionRequest,
    ) -> std::result::Result<SessionCompletionStream, SessionCompletionError> {
        let prepared = self.prepare_completion(request).await?;
        let events = prepared
            .driver
            .chat_completion_stream(
                &crate::ProviderEndpoint::default(),
                prepared.llm_messages,
                &prepared.llm_config,
            )
            .await
            .map_err(|error| SessionCompletionError::Completion {
                error: error.to_string(),
                context: prepared.context.clone(),
            })?;
        Ok(SessionCompletionStream {
            events,
            context: prepared.context,
        })
    }
}

// Tests for the command host live in the `everruns-test-support` crate
// (tests/command_host_test.rs): they drive the host through the llmsim
// simulator, which no longer ships with core (EVE-875).
