// Session command service.
//
// Decisions:
// - `/btw` reuses the session's merged harness/agent/session context so the side
//   answer sees the same conversation and prompt state as the main turn.
// - `/btw` never persists messages or events and always disables tools so the
//   side question behaves like Claude Code's ephemeral overlay answer.

use crate::direct_worker_adapters::DirectWorkerAdapters;
use crate::domains::mcp_servers::McpServerService;
use crate::domains::sessions::SESSION_VIEW;
use crate::errors::{BadRequestError, ResourceNotFoundError};
use crate::services::{EventService, LlmResolverService};
use crate::storage::StorageBackend;
use anyhow::{Result, anyhow};
use everruns_core::command::{CommandDescriptor, CommandResult, ExecuteCommandRequest};
use everruns_core::llm_driver_registry::{
    LlmCallConfigBuilder, LlmMessage, LlmMessageContent, LlmMessageRole, ProviderConfig,
    ProviderType, ToolSearchConfig,
};
use everruns_core::llm_models::LlmProviderType;
use everruns_core::message::{Controls, Message, MessageRole};
use everruns_core::runtime_context::{inspect_turn_context, resolve_runtime_capabilities};
use everruns_core::traits::{AgentStore, HarnessStore, ImageResolver, SessionStore};
use everruns_core::typed_id::{ModelId, SessionId};
use everruns_core::{Agent, Caller, CapabilityRegistry, DriverRegistry, Harness, ResolvedImage};
use everruns_macros::policy;
use everruns_worker::worker_adapters::{
    AdapterAgentStore, AdapterHarnessStore, AdapterImageResolver, AdapterLlmProviderStore,
    AdapterMessageRetriever, AdapterSessionFileStore, AdapterSessionStore,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

const BTW_COMMAND_NAME: &str = "btw";
const BTW_SYSTEM_PROMPT: &str = "You are answering an ephemeral side question about the current session. Use the existing conversation as context, answer exactly once, and do not call tools or ask follow-up questions.";

pub struct SessionCommandService {
    db: Arc<StorageBackend>,
    event_service: Arc<EventService>,
    llm_resolver: Arc<LlmResolverService>,
    mcp_server_service: Arc<McpServerService>,
    capability_registry: CapabilityRegistry,
    driver_registry: DriverRegistry,
    sqldb_store: everruns_core::traits::SessionSqlDbStoreRef,
    virtual_registry: Option<Arc<crate::services::virtual_mount_registry::VirtualMountRegistry>>,
}

impl SessionCommandService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: Arc<StorageBackend>,
        event_service: Arc<EventService>,
        llm_resolver: Arc<LlmResolverService>,
        mcp_server_service: Arc<McpServerService>,
        capability_registry: CapabilityRegistry,
        driver_registry: DriverRegistry,
        sqldb_store: everruns_core::traits::SessionSqlDbStoreRef,
    ) -> Self {
        Self {
            db,
            event_service,
            llm_resolver,
            mcp_server_service,
            capability_registry,
            driver_registry,
            sqldb_store,
            virtual_registry: None,
        }
    }

    pub fn with_virtual_registry(
        mut self,
        registry: Arc<crate::services::virtual_mount_registry::VirtualMountRegistry>,
    ) -> Self {
        self.virtual_registry = Some(registry);
        self
    }

    #[policy(SESSION_VIEW)]
    pub async fn list_system_commands(
        &self,
        caller: &Caller,
        session_id: SessionId,
    ) -> Result<Vec<CommandDescriptor>> {
        let (harness_chain, agent, session) = self
            .load_session_components(caller.org_id, session_id)
            .await?;
        let resolved = resolve_runtime_capabilities(
            &harness_chain,
            agent.as_ref(),
            &session,
            &self.capability_registry,
        );

        let mut seen = HashSet::new();
        let mut commands = Vec::new();
        for config in resolved.resolved_capability_configs {
            let Some(capability) = self.capability_registry.get(config.capability_id()) else {
                continue;
            };
            for command in capability.commands() {
                if seen.insert(command.name.clone()) {
                    commands.push(command);
                }
            }
        }

        Ok(commands)
    }

    #[policy(SESSION_VIEW)]
    pub async fn execute(
        &self,
        caller: &Caller,
        session_id: SessionId,
        req: ExecuteCommandRequest,
    ) -> Result<CommandResult> {
        let commands = self.list_system_commands(caller, session_id).await?;
        let command = commands
            .iter()
            .find(|command| command.name == req.name)
            .ok_or_else(|| {
                BadRequestError::new(format!(
                    "Unknown or unavailable system command: /{}",
                    req.name
                ))
            })?;

        match command.name.as_str() {
            BTW_COMMAND_NAME => self.execute_btw(caller.org_id, session_id, req).await,
            _ => Err(BadRequestError::new(format!(
                "Unsupported system command: /{}",
                command.name
            ))
            .into()),
        }
    }

    fn adapters(&self) -> DirectWorkerAdapters {
        let mut adapters = DirectWorkerAdapters::new(
            self.db.clone(),
            self.event_service.clone(),
            self.llm_resolver.clone(),
            self.mcp_server_service.clone(),
            self.capability_registry.clone(),
            self.driver_registry.clone(),
            self.sqldb_store.clone(),
        );
        if let Some(registry) = &self.virtual_registry {
            adapters = adapters.with_virtual_registry(registry.clone());
        }
        adapters
    }

    async fn load_session_components(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Result<(Vec<Harness>, Option<Agent>, everruns_core::Session)> {
        let adapters = self.adapters();
        let session_store = AdapterSessionStore::new(adapters.clone(), org_id);
        let harness_store = AdapterHarnessStore::new(adapters.clone(), org_id);
        let agent_store = AdapterAgentStore::new(adapters, org_id);

        let session = session_store
            .get_session(session_id)
            .await?
            .ok_or(ResourceNotFoundError::new("Session"))?;
        let harness_chain = harness_store.get_harness_chain(session.harness_id).await?;
        if harness_chain.is_empty() {
            return Err(ResourceNotFoundError::new("Harness").into());
        }
        let agent = match session.agent_id {
            Some(agent_id) => agent_store.get_agent(agent_id).await?,
            None => None,
        };

        Ok((harness_chain, agent, session))
    }

    async fn execute_btw(
        &self,
        org_id: i64,
        session_id: SessionId,
        req: ExecuteCommandRequest,
    ) -> Result<CommandResult> {
        let question = req
            .arguments
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| BadRequestError::new("/btw requires a question"))?;

        let adapters = self.adapters();
        let harness_store = AdapterHarnessStore::new(adapters.clone(), org_id);
        let agent_store = AdapterAgentStore::new(adapters.clone(), org_id);
        let session_store = AdapterSessionStore::new(adapters.clone(), org_id);
        let message_retriever = AdapterMessageRetriever::new(adapters.clone());
        let provider_store = AdapterLlmProviderStore::new(adapters.clone(), org_id);
        let file_store = Arc::new(AdapterSessionFileStore::new(adapters.clone()));
        let image_resolver = AdapterImageResolver::new(adapters.clone(), org_id);

        let session = session_store
            .get_session(session_id)
            .await?
            .ok_or(ResourceNotFoundError::new("Session"))?;

        let mut turn_context = inspect_turn_context(
            &harness_store,
            &agent_store,
            &session_store,
            &message_retriever,
            &provider_store,
            &self.capability_registry,
            session_id,
            session.harness_id,
            session.agent_id,
            &[],
            Some(file_store),
        )
        .await?;

        let model = self
            .resolve_model(
                org_id,
                req.controls.as_ref(),
                turn_context.resolved_model_id,
            )
            .await?
            .unwrap_or_else(|| turn_context.model_with_provider.clone());

        turn_context.runtime_agent.model = model.model.clone();

        let mut messages = patch_dangling_tool_calls(&turn_context.messages);
        let mut side_question = Message::user(question.to_string());
        side_question.controls = req.controls.clone();
        messages.push(side_question);

        let resolved_images = resolve_images(&image_resolver, &messages).await;

        let mut llm_messages = Vec::new();
        if !turn_context.runtime_agent.system_prompt.is_empty() {
            llm_messages.push(LlmMessage {
                role: LlmMessageRole::System,
                content: LlmMessageContent::Text(turn_context.runtime_agent.system_prompt.clone()),
                tool_calls: None,
                tool_call_id: None,
                phase: None,
                thinking: None,
                thinking_signature: None,
            });
        }
        llm_messages.push(LlmMessage {
            role: LlmMessageRole::System,
            content: LlmMessageContent::Text(BTW_SYSTEM_PROMPT.to_string()),
            tool_calls: None,
            tool_call_id: None,
            phase: None,
            thinking: None,
            thinking_signature: None,
        });

        for msg in &messages {
            let mut llm_msg = LlmMessage::from_message_with_images(msg, &resolved_images);
            if msg.role == MessageRole::User
                && let Some(actor) = &msg.external_actor
            {
                llm_msg.prepend_text_prefix(&format!("[{}] ", actor.display_label()));
            }
            llm_messages.push(llm_msg);
        }

        let provider = ProviderConfig {
            provider_type: provider_type_from_llm(model.provider_type)?,
            api_key: model.api_key.clone(),
            base_url: model.base_url.clone(),
        };
        let driver = self.driver_registry.create_driver(&provider)?;

        let mut llm_config_builder = LlmCallConfigBuilder::from(&turn_context.runtime_agent)
            .tools(vec![])
            .tool_search(ToolSearchConfig {
                enabled: false,
                threshold: usize::MAX,
            })
            .previous_response_id(None);

        if let Some(effort) = reasoning_effort(req.controls.as_ref()) {
            llm_config_builder = llm_config_builder.reasoning_effort(effort);
        }

        let llm_config = llm_config_builder
            .with_metadata("session_id", session_id.to_string())
            .with_metadata("command", BTW_COMMAND_NAME)
            .build();

        let response = driver.chat_completion(llm_messages, &llm_config).await?;
        let message = response.text.trim().to_string();
        if message.is_empty() {
            return Err(anyhow!("System command /btw returned an empty response"));
        }

        Ok(CommandResult {
            success: true,
            message,
        })
    }

    async fn resolve_model(
        &self,
        org_id: i64,
        controls: Option<&Controls>,
        fallback_model_id: Option<ModelId>,
    ) -> Result<Option<everruns_core::traits::ModelWithProvider>> {
        let requested_model_id = controls
            .and_then(|controls| controls.model_id)
            .or(fallback_model_id);
        let Some(model_id) = requested_model_id else {
            return Ok(None);
        };

        let resolved = self
            .llm_resolver
            .resolve_model(org_id, model_id.uuid())
            .await?
            .ok_or_else(|| BadRequestError::new(format!("Model not found: {}", model_id)))?;

        Ok(Some(everruns_core::traits::ModelWithProvider {
            model: resolved.model_id,
            provider_type: parse_llm_provider_type(&resolved.provider_type)?,
            api_key: resolved.api_key,
            base_url: resolved.base_url,
        }))
    }
}

fn reasoning_effort(controls: Option<&Controls>) -> Option<String> {
    controls
        .and_then(|controls| controls.reasoning.as_ref())
        .and_then(|reasoning| reasoning.effort.clone())
        .filter(|value| !value.is_empty())
}

fn provider_type_from_llm(provider_type: LlmProviderType) -> Result<ProviderType> {
    Ok(match provider_type {
        LlmProviderType::Openai => ProviderType::OpenAI,
        LlmProviderType::AzureOpenai => ProviderType::AzureOpenAI,
        LlmProviderType::OpenaiCompletions => ProviderType::OpenAICompletions,
        LlmProviderType::Anthropic => ProviderType::Anthropic,
        LlmProviderType::Gemini => ProviderType::Gemini,
        LlmProviderType::LlmSim => ProviderType::LlmSim,
    })
}

fn parse_llm_provider_type(provider_type: &str) -> Result<LlmProviderType> {
    provider_type
        .parse::<LlmProviderType>()
        .map_err(|error| anyhow!(error))
}

fn patch_dangling_tool_calls(messages: &[Message]) -> Vec<Message> {
    let mut result = Vec::new();

    for (index, msg) in messages.iter().enumerate() {
        result.push(msg.clone());
        if msg.role != MessageRole::Agent || !msg.has_tool_calls() {
            continue;
        }

        for tool_call in msg.tool_calls() {
            let has_result = messages[(index + 1)..].iter().any(|message| {
                message.role == MessageRole::ToolResult
                    && message.tool_call_id() == Some(tool_call.id.as_str())
            });

            if !has_result {
                result.push(Message::tool_result(
                    &tool_call.id,
                    None,
                    Some(
                        "cancelled - another message came in before it could be completed"
                            .to_string(),
                    ),
                ));
            }
        }
    }

    result
}

async fn resolve_images(
    image_resolver: &AdapterImageResolver<DirectWorkerAdapters>,
    messages: &[Message],
) -> HashMap<Uuid, ResolvedImage> {
    let image_ids: HashSet<Uuid> = messages
        .iter()
        .flat_map(LlmMessage::extract_image_file_ids)
        .collect();

    let mut resolved = HashMap::new();
    for image_id in image_ids {
        if let Ok(Some(image)) = image_resolver.resolve_image(image_id).await {
            resolved.insert(image_id, image);
        }
    }
    resolved
}
