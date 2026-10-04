use async_trait::async_trait;
use everruns_contracts::{
    AgentLoopError, BearerAuth, ChatDriver, DiscoveredModel, DriverId, DriverRegistry,
    LlmCallConfig, LlmResponseStream, Message, OpenResponsesProtocolChatDriver, Provider,
    ProviderEndpoint,
};
use everruns_contracts::{
    credential_schema::CredentialFormSchema,
    driver_registry::{DriverDescriptor, DriverOAuthConfig, DriverOAuthFlow},
    error::Result,
    openresponses_protocol::OpenResponsesRequestExtension,
};
use serde_json::Value;
use std::sync::Arc;
pub const REJECTED_FIELDS: &[&str] = &[
    "background",
    "conversation",
    "max_output_tokens",
    "max_tool_calls",
    "metadata",
    "moderation",
    "multi_agent",
    "prompt",
    "prompt_cache_retention",
    "safety_identifier",
    "temperature",
    "top_logprobs",
    "top_p",
    "truncation",
    "user",
    "previous_response_id",
    "service_tier",
];
struct PlanRequest;
impl OpenResponsesRequestExtension for PlanRequest {
    fn decorate(&self, body: &mut Value, _config: &LlmCallConfig) -> Result<()> {
        let Some(body) = body.as_object_mut() else {
            return Err(AgentLoopError::llm("Invalid ChatGPT request"));
        };
        for field in REJECTED_FIELDS {
            body.remove(*field);
        }
        body.insert("store".into(), false.into());
        body.insert("stream".into(), true.into());
        if let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) {
            for item in input {
                if item["role"] == "system" {
                    item["role"] = "developer".into();
                }
            }
        }
        if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
            if tools.iter().any(|t| {
                !matches!(
                    t["type"].as_str(),
                    Some("function" | "custom" | "namespace" | "web_search")
                )
            }) {
                return Err(AgentLoopError::llm(
                    "This tool is unavailable with the ChatGPT plan. Use a local tool or choose an API provider.",
                ));
            }
            // The plan route accepts client-executed tools inside namespaces.
            let mut local = Vec::new();
            tools.retain(|tool| {
                if matches!(tool["type"].as_str(), Some("function" | "custom")) {
                    local.push(tool.clone());
                    false
                } else {
                    true
                }
            });
            if !local.is_empty() {
                tools
                    .push(serde_json::json!({"type":"namespace","name":"functions","tools":local}));
            }
        }
        Ok(())
    }
    fn classify_error(
        &self,
        _status: u16,
        _headers: &reqwest::header::HeaderMap,
        body: &str,
    ) -> Option<everruns_contracts::LlmErrorKind> {
        let error: Value = serde_json::from_str(body).ok()?;
        match error.pointer("/error/code").and_then(Value::as_str) {
            Some("subscription_sharing_usage_limit_exceeded") => {
                Some(everruns_contracts::LlmErrorKind::QuotaExhausted)
            }
            Some(
                "subscription_sharing_user_not_eligible"
                | "subscription_sharing_invalid_user"
                | "chatpass_v2_scope_not_authorized"
                | "chatpass_v2_invalid_authorization_context",
            ) => Some(everruns_contracts::LlmErrorKind::Authentication),
            Some(
                "subscription_sharing_unsupported_capability"
                | "subscription_sharing_route_not_supported",
            ) => Some(everruns_contracts::LlmErrorKind::InvalidRequest),
            Some(
                "subscription_sharing_usage_unavailable" | "subscription_sharing_user_unavailable",
            ) => Some(everruns_contracts::LlmErrorKind::Unavailable),
            _ => None,
        }
    }
}
#[derive(Clone)]
pub struct ChatGptChatDriver {
    inner: OpenResponsesProtocolChatDriver,
}
impl Default for ChatGptChatDriver {
    fn default() -> Self {
        Self::new()
    }
}
impl ChatGptChatDriver {
    pub fn new() -> Self {
        Self {
            inner: OpenResponsesProtocolChatDriver::new()
                .with_native_features(true, false)
                .with_hosted_tools(true)
                .with_stateful_responses(false)
                .with_prompt_cache_options(false)
                .with_required_completion(true)
                .with_request_extension(Arc::new(PlanRequest)),
        }
    }
}
#[async_trait]
impl ChatDriver for ChatGptChatDriver {
    async fn chat_completion_stream(
        &self,
        endpoint: &ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        let hosted =
            everruns_contracts::openai_hosted_tools::OpenAiHostedTools::from_driver_options(
                &config.driver_options,
            )
            .map_err(|_| {
                AgentLoopError::llm_kind(
                    everruns_contracts::LlmErrorKind::InvalidRequest,
                    "Invalid ChatGPT hosted tool configuration",
                )
            })?;
        if hosted.is_some_and(|tools| {
            tools.code_interpreter.is_some()
                || tools.shell.is_some()
                || tools.file_search.is_some()
                || !tools.mcp_servers.is_empty()
        }) {
            return Err(AgentLoopError::llm_kind(
                everruns_contracts::LlmErrorKind::InvalidRequest,
                "This hosted tool is unavailable with the ChatGPT plan. Use a local tool or choose an API provider.",
            ));
        }
        let mut config = config.clone();
        // Native computer use is a preference; preserve its local-tool fallback.
        config
            .driver_options
            .remove(everruns_contracts::native_computer::NATIVE_COMPUTER_USE_OPTION);
        self.inner
            .chat_completion_stream(endpoint, messages, &config)
            .await
    }
    fn supports_parallel_tool_calls(&self, _model: &str) -> bool {
        true
    }
    async fn list_models(
        &self,
        endpoint: &ProviderEndpoint,
    ) -> Result<Option<Vec<DiscoveredModel>>> {
        let url = endpoint
            .url("models")
            .ok_or_else(|| AgentLoopError::llm("ChatGPT endpoint is not configured"))?;
        let resolved = endpoint.resolve("GET", url, &[]).await?;
        let mut request = self.inner.client().get(&resolved.url);
        for (name, value) in resolved.headers {
            request = request.header(name, value);
        }
        let response = request
            .send()
            .await
            .map_err(|_| AgentLoopError::llm("ChatGPT model discovery failed"))?;
        if !response.status().is_success() {
            return Err(AgentLoopError::llm_http(
                response.status().as_u16(),
                "",
                "ChatGPT model discovery failed. Reconnect the account or manage its usage.",
            ));
        }
        let body: Value = response
            .json()
            .await
            .map_err(|_| AgentLoopError::llm("Invalid ChatGPT model catalog"))?;
        let models = body["models"]
            .as_array()
            .ok_or_else(|| AgentLoopError::llm("ChatGPT returned no model catalog"))?;
        Ok(Some(
            models
                .iter()
                .filter(|m| m["visibility"] == "list")
                .filter_map(|m| {
                    Some(DiscoveredModel {
                        model_id: m["slug"].as_str()?.into(),
                        display_name: m["display_name"].as_str().map(str::to_owned),
                        capabilities: vec!["chat".into()],
                        created_at: None,
                        owned_by: None,
                        discovered_profile: None,
                    })
                })
                .collect(),
        ))
    }
}
pub fn provider(
    id: impl Into<everruns_contracts::ProviderKey>,
    auth: impl everruns_contracts::ProviderAuth + 'static,
) -> Provider {
    Provider::new(id, ChatGptChatDriver::new())
        .base_url(super::oauth::RESOURCE)
        .auth(auth)
}
pub fn descriptor() -> DriverDescriptor {
    let mut descriptor = DriverDescriptor::chat_only(DriverId::external("chatgpt"), |config| {
        Provider::new(config.provider.clone(), ChatGptChatDriver::new())
            .base_url(config.base_url.as_deref().unwrap_or(super::oauth::RESOURCE))
            .auth(BearerAuth::new(config.api_key.clone().unwrap_or_default()))
            .into_boxed_driver()
    });
    descriptor.display_name = "ChatGPT plan".into();
    descriptor.credential_schema = CredentialFormSchema::api_key(
        "",
        "Connect with your ChatGPT account. Credentials are managed by the host.",
    );
    descriptor.credential_schema.fields[0].env.clear();
    descriptor.oauth = Some(DriverOAuthConfig {
        authorize_url: super::oauth::Endpoints::production().authorize,
        token_url: super::oauth::Endpoints::production().token,
        flow: DriverOAuthFlow::ChatGptPlan,
    });
    descriptor
}
pub fn register_driver(registry: &mut DriverRegistry) {
    registry.register_descriptor(descriptor());
}
