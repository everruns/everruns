//! Store-backed command context and completion implementation.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use everruns_core::capabilities::CapabilityRegistry;
use everruns_core::command_host::{
    CommandHost, CommandTurnContext, SessionCompletion, SessionCompletionError,
    SessionCompletionRequest, SessionCompletionStream,
};
use everruns_core::driver_registry::{
    ChatDriver, DriverRegistry, LlmCallConfig, LlmMessage, LlmMessageRole, ToolSearchConfig,
};
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::execution_loading::{AgentStore, HarnessStore, SessionStore};
use everruns_core::image_services::{ImageResolver, ResolvedImage};
use everruns_core::message::{Controls, Message, MessageRole, patch_dangling_tool_calls};
use everruns_core::message_retriever::MessageRetriever;
use everruns_core::provider_resolution::ProviderStore;
use everruns_core::runtime_context::{AssembledTurnContext, ResolvedModelExecution};
use everruns_core::session_files::SessionFileSystem;
use everruns_core::user_facing_error::UserFacingErrorContext;
use everruns_core::{ProviderEndpoint, SessionId};

use crate::runtime_context::inspect_turn_context_for_session;

/// Store-backed [`CommandHost`] shared by embedded and durable hosts.
///
/// One instance is built per command dispatch. Its assembled context is
/// memoized so `turn_context()` and completion share one store resolution.
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
    /// Construct a command host from org-scoped runtime stores and registries.
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

    /// Resolve `image_file` references for provider conversion.
    pub fn with_image_resolver(mut self, image_resolver: Arc<dyn ImageResolver>) -> Self {
        self.image_resolver = Some(image_resolver);
        self
    }

    /// Supply the session filesystem used by dynamic prompt capabilities.
    pub fn with_file_store(mut self, file_store: Arc<dyn SessionFileSystem>) -> Self {
        self.file_store = Some(file_store);
        self
    }

    /// Seed a context already assembled by the dispatching host.
    pub fn with_assembled_context(mut self, assembled: AssembledTurnContext) -> Self {
        self.assembled = tokio::sync::OnceCell::new_with(Some(assembled));
        self
    }

    async fn assembled(&self) -> Result<&AssembledTurnContext> {
        self.assembled
            .get_or_try_init(|| async {
                inspect_turn_context_for_session(
                    self.harness_store.as_ref(),
                    self.agent_store.as_ref(),
                    self.session_store.as_ref(),
                    self.message_retriever.as_ref(),
                    self.provider_store.as_ref(),
                    &self.capability_registry,
                    &self.driver_registry,
                    self.session_id,
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
            .flat_map(everruns_core::llm_conversions::extract_image_file_ids)
            .collect();
        let mut resolved = HashMap::new();
        for image_id in image_ids {
            if let Ok(Some(image)) = resolver.resolve_image(image_id).await {
                resolved.insert(image_id, image);
            }
        }
        resolved
    }

    async fn resolve_completion_model(
        &self,
        controls: Option<&Controls>,
        assembled: &AssembledTurnContext,
    ) -> std::result::Result<ResolvedModelExecution, SessionCompletionError> {
        let requested = controls.and_then(|controls| controls.model_id);
        if requested.is_none() || requested == assembled.resolved_model_id {
            return Ok(assembled.model.clone());
        }
        let model_id = requested.expect("checked above");
        let resolved = self
            .provider_store
            .get_resolved_model(model_id)
            .await
            .map_err(SessionCompletionError::InvalidRequest)?
            .ok_or_else(|| {
                SessionCompletionError::InvalidRequest(AgentLoopError::config(format!(
                    "Model not found: {model_id}"
                )))
            })?;
        let (spec, compatibility_config) = resolved.canonical_parts();
        let error_context = UserFacingErrorContext::default()
            .with_provider(compatibility_config.provider_type.to_string())
            .with_model_id(spec.model.clone());
        let config = self
            .provider_store
            .get_provider_config(&spec.provider)
            .await
            .map_err(|error| SessionCompletionError::Completion {
                error: error.to_string(),
                context: error_context.clone(),
            })?
            .unwrap_or(compatibility_config);
        let provider_type = config.provider_type.clone();
        let driver: Arc<dyn ChatDriver> = Arc::from(
            self.driver_registry
                .create_chat_driver(&config)
                .map_err(|error| SessionCompletionError::Completion {
                    error: error.to_string(),
                    context: error_context,
                })?,
        );
        Ok(ResolvedModelExecution {
            model: spec.model,
            provider: spec.provider,
            provider_type,
            driver,
        })
    }

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
        for message in &messages {
            let mut llm_message =
                everruns_core::llm_conversions::llm_message_from_message_with_images(
                    message,
                    &resolved_images,
                );
            if message.role == MessageRole::User
                && let Some(actor) = &message.external_actor
            {
                llm_message.prepend_text_prefix(&format!("[{}] ", actor.display_label()));
            }
            llm_messages.push(llm_message);
        }
        let mut builder = everruns_core::llm_conversions::llm_call_config_builder_from_agent(
            &assembled.runtime_agent,
        )
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
            builder = builder.reasoning_effort(effort);
        }
        for (key, value) in &request.metadata {
            builder = builder.with_metadata(key, value);
        }
        Ok(PreparedCompletion {
            llm_messages,
            llm_config: builder.build(),
            driver: model.driver,
            context,
        })
    }
}

struct PreparedCompletion {
    llm_messages: Vec<LlmMessage>,
    llm_config: LlmCallConfig,
    driver: Arc<dyn ChatDriver>,
    context: UserFacingErrorContext,
}

#[async_trait]
impl CommandHost for StoreCommandHost {
    async fn turn_context(&self) -> Result<CommandTurnContext> {
        let assembled = self.assembled().await?;
        Ok(CommandTurnContext {
            session_id: assembled.snapshot.session_id,
            messages: assembled.messages.clone(),
            system_prompt: assembled.runtime_agent.system_prompt.clone(),
            model: assembled.model.model.clone(),
            provider_type: assembled.model.provider_type.to_string(),
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
                &ProviderEndpoint::default(),
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
                &ProviderEndpoint::default(),
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
