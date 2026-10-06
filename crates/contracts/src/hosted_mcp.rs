//! Credentials for remote MCP servers OpenAI calls on the agent's behalf
//! (EVE-1115).
//!
//! An agent's hosted MCP entry may name a registered Everruns MCP server
//! (`mcp_server`) instead of carrying a URL. Its URL and request headers, API
//! keys and OAuth tokens included, then come from the host's MCP connection
//! and secret stores, never from agent-writable config.
//!
//! [`HostedMcpDriver`] wraps the turn's model driver and resolves those
//! entries on every call, so a refreshed OAuth token is picked up by the next
//! request. The resolved headers exist only in that call's cloned
//! [`LlmCallConfig`] on the way into the wire driver: the engine never sees
//! them, so they cannot reach events, and [`McpServerTool`](crate::openai_hosted_tools::McpServerTool)'s `Debug` redacts
//! them (THREAT TM-AGENT-029).

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::compact::{CompactRequest, CompactResponse};
use crate::driver_registry::{
    ChatDriver, DiscoveredModel, LlmCallConfig, LlmResponse, LlmResponseStream, Message,
    ProviderOpaqueContext,
};
use crate::error::{AgentLoopError, Result};
use crate::openai_hosted_tools::{OPENAI_HOSTED_TOOLS_OPTION, OpenAiHostedTools};
use crate::runtime_provider::ProviderEndpoint;

/// A registered MCP server's endpoint, credentials included.
#[derive(Clone, PartialEq, Eq)]
pub struct ResolvedHostedMcp {
    pub url: String,
    pub headers: BTreeMap<String, String>,
}

impl std::fmt::Debug for ResolvedHostedMcp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedHostedMcp")
            .field("url", &self.url)
            .field("headers", &self.headers.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// Resolves a registered MCP server, by name, for the current session.
///
/// Implementations apply the same scoping and credential lookup as the
/// host's own MCP tool execution. An error fails the turn: the agent was
/// configured with the server, so dropping it silently would change behavior.
#[async_trait]
pub trait HostedMcpResolver: Send + Sync {
    async fn resolve(&self, server: &str) -> Result<ResolvedHostedMcp>;
}

/// Fill every registered-server entry in `config`'s hosted tools with its
/// resolved URL and headers. Returns `None` when there is nothing to resolve.
pub async fn resolve_hosted_mcp(
    config: &LlmCallConfig,
    resolver: &dyn HostedMcpResolver,
) -> Result<Option<LlmCallConfig>> {
    let Some(raw) = config.driver_options.get(OPENAI_HOSTED_TOOLS_OPTION) else {
        return Ok(None);
    };
    let Ok(mut tools) = serde_json::from_value::<OpenAiHostedTools>(raw.clone()) else {
        // The wire driver reports a malformed option with its own error.
        return Ok(None);
    };
    if tools.mcp_servers.iter().all(|s| s.mcp_server.is_none()) {
        return Ok(None);
    }
    for server in &mut tools.mcp_servers {
        let Some(name) = server.mcp_server.as_deref() else {
            continue;
        };
        let resolved = resolver.resolve(name).await?;
        if !resolved.url.starts_with("https://") {
            return Err(AgentLoopError::Configuration(format!(
                "MCP server {name} must use https to be called by OpenAI"
            )));
        }
        server.server_url = resolved.url;
        server.headers = resolved.headers;
    }
    let mut config = config.clone();
    let value = serde_json::to_value(&tools)
        .map_err(|error| AgentLoopError::Configuration(error.to_string()))?;
    config
        .driver_options
        .insert(OPENAI_HOSTED_TOOLS_OPTION.to_string(), value);
    Ok(Some(config))
}

/// Model driver that resolves registered hosted MCP servers before each call.
pub struct HostedMcpDriver {
    inner: Arc<dyn ChatDriver>,
    resolver: Arc<dyn HostedMcpResolver>,
}

impl HostedMcpDriver {
    pub fn new(inner: Arc<dyn ChatDriver>, resolver: Arc<dyn HostedMcpResolver>) -> Self {
        Self { inner, resolver }
    }

    async fn apply(&self, config: &LlmCallConfig) -> Result<Option<LlmCallConfig>> {
        resolve_hosted_mcp(config, self.resolver.as_ref()).await
    }
}

#[async_trait]
impl ChatDriver for HostedMcpDriver {
    fn native_async_driver(
        &self,
        model: &str,
        tools: BTreeMap<String, Option<serde_json::Value>>,
        continuation: Option<crate::native_async::Delivery>,
    ) -> Option<Arc<dyn ChatDriver>> {
        Some(Arc::new(Self {
            inner: self.inner.native_async_driver(model, tools, continuation)?,
            resolver: self.resolver.clone(),
        }))
    }

    async fn chat_completion_stream(
        &self,
        endpoint: &ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        let resolved = self.apply(config).await?;
        let config = resolved.as_ref().unwrap_or(config);
        self.inner
            .chat_completion_stream(endpoint, messages, config)
            .await
    }

    async fn chat_completion(
        &self,
        endpoint: &ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponse> {
        let resolved = self.apply(config).await?;
        let config = resolved.as_ref().unwrap_or(config);
        self.inner.chat_completion(endpoint, messages, config).await
    }

    fn supports_native_non_streaming(&self) -> bool {
        self.inner.supports_native_non_streaming()
    }

    async fn chat_completion_non_streaming(
        &self,
        endpoint: &ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponse> {
        let resolved = self.apply(config).await?;
        let config = resolved.as_ref().unwrap_or(config);
        self.inner
            .chat_completion_non_streaming(endpoint, messages, config)
            .await
    }

    async fn list_models(
        &self,
        endpoint: &ProviderEndpoint,
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

    fn supports_parallel_tool_calls(&self, model: &str) -> bool {
        self.inner.supports_parallel_tool_calls(model)
    }

    fn provider_managed_reduction_option(
        &self,
        endpoint: &ProviderEndpoint,
        model: &str,
        budget_tokens: usize,
    ) -> Option<(String, serde_json::Value)> {
        self.inner
            .provider_managed_reduction_option(endpoint, model, budget_tokens)
    }

    fn provider_managed_reduction_fallback_reason(
        &self,
        endpoint: &ProviderEndpoint,
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
        endpoint: &ProviderEndpoint,
        request: CompactRequest,
    ) -> Result<Option<CompactResponse>> {
        self.inner.compact(endpoint, request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai_hosted_tools::McpServerTool;
    use serde_json::json;

    struct Registered;

    #[async_trait]
    impl HostedMcpResolver for Registered {
        async fn resolve(&self, server: &str) -> Result<ResolvedHostedMcp> {
            match server {
                "github" => Ok(ResolvedHostedMcp {
                    url: "https://mcp.github.example/mcp".into(),
                    headers: BTreeMap::from([("Authorization".into(), "Bearer t0k".into())]),
                }),
                "plain" => Ok(ResolvedHostedMcp {
                    url: "http://plain.example/mcp".into(),
                    headers: BTreeMap::new(),
                }),
                other => Err(AgentLoopError::Configuration(format!("no {other}"))),
            }
        }
    }

    fn config_with(servers: Vec<McpServerTool>) -> LlmCallConfig {
        let mut config = LlmCallConfig::new("gpt-5");
        let (key, value) = OpenAiHostedTools {
            mcp_servers: servers,
            ..Default::default()
        }
        .to_driver_option()
        .unwrap();
        config.driver_options.insert(key, value);
        config
    }

    fn registered(label: &str, name: &str) -> McpServerTool {
        McpServerTool {
            server_label: label.into(),
            mcp_server: Some(name.into()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn registered_server_gets_url_and_credentials_per_call() {
        let public = McpServerTool {
            server_label: "deepwiki".into(),
            server_url: "https://mcp.deepwiki.com/mcp".into(),
            ..Default::default()
        };
        let config = config_with(vec![public, registered("gh", "github")]);
        let resolved = resolve_hosted_mcp(&config, &Registered)
            .await
            .unwrap()
            .unwrap();
        let tools = OpenAiHostedTools::from_driver_options(&resolved.driver_options)
            .unwrap()
            .unwrap();
        let wire = tools.wire_tools();
        assert_eq!(wire[0]["server_url"], "https://mcp.deepwiki.com/mcp");
        assert!(wire[0].get("headers").is_none());
        assert_eq!(wire[1]["server_url"], "https://mcp.github.example/mcp");
        assert_eq!(wire[1]["headers"], json!({ "Authorization": "Bearer t0k" }));
        // The engine's own config is untouched.
        assert!(!format!("{:?}", config.driver_options).contains("t0k"));
        // Debug output of the resolved tool never shows the credential.
        assert!(!format!("{tools:?}").contains("t0k"));
    }

    #[tokio::test]
    async fn nothing_to_resolve_passes_the_config_through() {
        let plain = LlmCallConfig::new("gpt-5");
        assert!(
            resolve_hosted_mcp(&plain, &Registered)
                .await
                .unwrap()
                .is_none()
        );
        let public = config_with(vec![McpServerTool {
            server_label: "deepwiki".into(),
            server_url: "https://mcp.deepwiki.com/mcp".into(),
            ..Default::default()
        }]);
        assert!(
            resolve_hosted_mcp(&public, &Registered)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn unresolvable_or_insecure_servers_fail_the_call() {
        for name in ["missing", "plain"] {
            let config = config_with(vec![registered("x", name)]);
            let error = resolve_hosted_mcp(&config, &Registered).await.unwrap_err();
            assert!(matches!(error, AgentLoopError::Configuration(_)), "{error}");
        }
    }
}
