//! Parallel web search and fetch for Everruns agents.
//!
//! `everruns-integrations-parallel` is part of the
//! [Everruns](https://everruns.com) ecosystem. It contributes
//! [Parallel](https://parallel.ai)'s hosted MCP server so agents get
//! provider-owned `web_search` and `web_fetch` tools — free by default, with an
//! optional Parallel API-key connection for authenticated usage.
//!
//! # Example
//!
//! ```
//! use everruns_core::capabilities::Capability;
//! use everruns_integrations_parallel::ParallelCapability;
//!
//! let capability = ParallelCapability;
//! assert_eq!(capability.id(), "parallel_search");
//! ```
//!
//! # Design notes
//!
//! - Uses Parallel's hosted MCP server directly, so Everruns gets the
//!   provider-owned behavior without proxying or reimplementing it.
//! - Defaults to the free unauthenticated endpoint. Per-capability config can
//!   require a user-scoped Parallel API-key connection and switch to the
//!   OAuth-compatible MCP endpoint.

pub mod connection;
pub mod payments;

use everruns_core::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, IntegrationPlugin,
};
use everruns_core::{McpServerAuthMode, ScopedMcpServer, ScopedMcpServers};
use everruns_platform::connector::ConnectorPlugin;
use serde_json::{Value, json};

use connection::ParallelConnector;
pub use payments::ParallelPaymentsCapability;

inventory::submit! {
    IntegrationPlugin {
        experimental_only: true,
        feature_flag: None,
        factory: || Box::new(ParallelCapability),
    }
}

// Paid Parallel tools route spend through the core `PaymentAuthority`. Gated by
// the deployment-controlled `machine_payments` feature flag (off by default on all envs,
// including dev, because spend is irreversible) rather than the experimental
// grade gate, so it can be enabled deliberately in any environment.
inventory::submit! {
    IntegrationPlugin {
        experimental_only: false,
        feature_flag: Some("machine_payments"),
        factory: || Box::new(ParallelPaymentsCapability),
    }
}

inventory::submit! {
    ConnectorPlugin {
        experimental_only: true,
        factory: || Box::new(ParallelConnector),
    }
}

pub const PARALLEL_CAPABILITY_ID: &str = "parallel_search";
pub const PARALLEL_PROVIDER_ID: &str = "parallel";
pub const PARALLEL_SERVER_NAME: &str = "Parallel";
pub const PARALLEL_MCP_URL: &str = "https://search.parallel.ai/mcp";
pub const PARALLEL_MCP_OAUTH_URL: &str = "https://search.parallel.ai/mcp-oauth";

const CONFIG_AUTH_CONNECTION: &str = "connection";
const CONFIG_ENDPOINT_OAUTH: &str = "oauth";

pub struct ParallelCapability;

impl ParallelCapability {
    fn use_connection(config: &serde_json::Value) -> bool {
        config
            .get("auth")
            .and_then(|value| value.as_str())
            .is_some_and(|auth| auth == CONFIG_AUTH_CONNECTION)
            || Self::use_oauth_endpoint(config)
    }

    fn use_oauth_endpoint(config: &serde_json::Value) -> bool {
        config
            .get("endpoint")
            .and_then(|value| value.as_str())
            .is_some_and(|endpoint| endpoint == CONFIG_ENDPOINT_OAUTH)
    }

    fn mcp_url(config: &serde_json::Value) -> &'static str {
        if Self::use_oauth_endpoint(config) {
            PARALLEL_MCP_OAUTH_URL
        } else {
            PARALLEL_MCP_URL
        }
    }
}

impl Capability for ParallelCapability {
    fn id(&self) -> &str {
        PARALLEL_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "[Experimental] Parallel"
    }

    fn description(&self) -> &str {
        "Search and fetch web content through Parallel MCP. Free by default, with optional Parallel API-key connection for authenticated usage."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("search")
    }

    fn category(&self) -> Option<&str> {
        Some("Network")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            r#"## Parallel

Use Parallel for current web search and focused URL extraction.

For Parallel tool calls, generate one stable `session_id` for the conversation and reuse it for both search and fetch calls."#,
        )
    }

    fn config_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "title": "Parallel Settings",
            "properties": {
                "auth": {
                    "type": "string",
                    "title": "Authentication",
                    "description": "Free works without setup. API key uses Settings > Connections > Parallel.",
                    "default": "free",
                    "oneOf": [
                        { "const": "free", "title": "Free" },
                        { "const": "connection", "title": "Parallel API Key" }
                    ]
                },
                "endpoint": {
                    "type": "string",
                    "title": "MCP Endpoint",
                    "description": "OAuth MCP uses https://search.parallel.ai/mcp-oauth and requires a Parallel API key.",
                    "default": "free",
                    "oneOf": [
                        { "const": "free", "title": "Free MCP" },
                        { "const": "oauth", "title": "OAuth MCP" }
                    ]
                }
            },
            "allOf": [
                {
                    "if": {
                        "properties": {
                            "endpoint": { "const": "oauth" }
                        },
                        "required": ["endpoint"]
                    },
                    "then": {
                        "properties": {
                            "auth": { "const": "connection" }
                        },
                        "required": ["auth"]
                    }
                }
            ]
        }))
    }

    fn config_ui_schema(&self) -> Option<Value> {
        Some(json!({
            "ui:submitButtonOptions": { "norender": true },
            "ui:order": ["auth", "endpoint"],
            "auth": { "ui:widget": "select" },
            "endpoint": { "ui:widget": "select" }
        }))
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        if config.is_null() {
            return Ok(());
        }

        let object = config
            .as_object()
            .ok_or_else(|| "Invalid parallel_search config: expected object".to_string())?;

        for field in object.keys() {
            if field != "auth" && field != "endpoint" {
                return Err(format!(
                    "Invalid parallel_search config: unknown field `{field}`"
                ));
            }
        }

        if let Some(auth) = object.get("auth") {
            match auth.as_str() {
                Some("free" | CONFIG_AUTH_CONNECTION) => {}
                Some(other) => {
                    return Err(format!(
                        "Invalid parallel_search config: unsupported auth `{other}`"
                    ));
                }
                None => return Err("Invalid parallel_search config: auth must be a string".into()),
            }
        }

        if let Some(endpoint) = object.get("endpoint") {
            match endpoint.as_str() {
                Some("free" | CONFIG_ENDPOINT_OAUTH) => {}
                Some(other) => {
                    return Err(format!(
                        "Invalid parallel_search config: unsupported endpoint `{other}`"
                    ));
                }
                None => {
                    return Err("Invalid parallel_search config: endpoint must be a string".into());
                }
            }
        }

        let auth = object.get("auth").and_then(Value::as_str).unwrap_or("free");
        let endpoint = object
            .get("endpoint")
            .and_then(Value::as_str)
            .unwrap_or("free");
        if endpoint == CONFIG_ENDPOINT_OAUTH && auth != CONFIG_AUTH_CONNECTION {
            return Err(
                "Invalid parallel_search config: endpoint `oauth` requires auth `connection`"
                    .into(),
            );
        }

        Ok(())
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![
            CapabilityLocalization {
                locale: "en",
                name: None,
                description: None,
                config_description: Some(
                    "Choose whether Parallel uses the free endpoint or a Parallel API-key \
                     connection, and which MCP endpoint it talks to.",
                ),
                config_overlay: None,
            },
            CapabilityLocalization {
                locale: "uk",
                name: Some("[Експериментально] Parallel"),
                description: Some(
                    "Шукайте та отримуйте вебвміст через Parallel MCP. Безкоштовно за \
                     замовчуванням, з опціональним підключенням за API-ключем Parallel для \
                     автентифікованого використання.",
                ),
                config_description: Some(
                    "Визначає, чи використовує Parallel безкоштовний доступ або підключення з \
                     API-ключем Parallel, і який ендпоінт MCP застосовується.",
                ),
                config_overlay: Some(json!({
                    "properties": {
                        "auth": {
                            "title": "Автентифікація",
                            "description": "Безкоштовний режим працює без налаштувань. API-ключ використовує Налаштування > Підключення > Parallel.",
                            "enum_labels": {
                                "free": "Безкоштовно",
                                "connection": "API-ключ Parallel"
                            }
                        },
                        "endpoint": {
                            "title": "Ендпоінт MCP",
                            "description": "OAuth MCP використовує https://search.parallel.ai/mcp-oauth і потребує API-ключа Parallel.",
                            "enum_labels": {
                                "free": "Безкоштовний MCP",
                                "oauth": "OAuth MCP"
                            }
                        }
                    }
                })),
            },
        ]
    }

    fn mcp_servers_with_config(&self, config: &serde_json::Value) -> ScopedMcpServers {
        let mut servers = ScopedMcpServers::default();
        servers.insert(
            PARALLEL_SERVER_NAME.to_string(),
            ScopedMcpServer {
                url: Self::mcp_url(config).to_string(),
                auth_mode: if Self::use_connection(config) {
                    McpServerAuthMode::OAuth
                } else {
                    McpServerAuthMode::None
                },
                oauth_provider_id: Self::use_connection(config)
                    .then(|| PARALLEL_PROVIDER_ID.to_string()),
                ..Default::default()
            },
        );
        servers
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::capabilities::{Capability, collect_capability_mcp_servers};
    use everruns_core::capability_types::AgentCapabilityConfig;
    use serde_json::json;

    #[test]
    fn metadata() {
        let cap = ParallelCapability;
        assert_eq!(cap.id(), "parallel_search");
        assert_eq!(cap.icon(), Some("search"));
        assert_eq!(cap.category(), Some("Network"));
        assert!(cap.tools().is_empty());
        assert!(cap.tool_definitions().is_empty());
    }

    #[test]
    fn contributes_free_mcp_server_by_default() {
        let cap = ParallelCapability;
        let servers = cap.mcp_servers_with_config(&json!({}));
        let server = servers.get(PARALLEL_SERVER_NAME).unwrap();
        assert_eq!(server.url, PARALLEL_MCP_URL);
        assert_eq!(server.auth_mode, McpServerAuthMode::None);
        assert_eq!(server.oauth_provider_id, None);
    }

    #[test]
    fn config_can_use_connection_on_default_endpoint() {
        let cap = ParallelCapability;
        let servers = cap.mcp_servers_with_config(&json!({ "auth": "connection" }));
        let server = servers.get(PARALLEL_SERVER_NAME).unwrap();
        assert_eq!(server.url, PARALLEL_MCP_URL);
        assert_eq!(server.auth_mode, McpServerAuthMode::OAuth);
        assert_eq!(
            server.oauth_provider_id.as_deref(),
            Some(PARALLEL_PROVIDER_ID)
        );
    }

    #[test]
    fn config_can_use_oauth_endpoint() {
        let cap = ParallelCapability;
        let servers =
            cap.mcp_servers_with_config(&json!({ "auth": "connection", "endpoint": "oauth" }));
        let server = servers.get(PARALLEL_SERVER_NAME).unwrap();
        assert_eq!(server.url, PARALLEL_MCP_OAUTH_URL);
        assert_eq!(server.auth_mode, McpServerAuthMode::OAuth);
        assert_eq!(
            server.oauth_provider_id.as_deref(),
            Some(PARALLEL_PROVIDER_ID)
        );
    }

    #[test]
    fn validates_config_values() {
        let cap = ParallelCapability;

        assert!(
            cap.validate_config(&json!({ "auth": "connection", "endpoint": "oauth" }))
                .is_ok()
        );
        assert!(
            cap.validate_config(&json!({ "endpoint": "oauth" }))
                .is_err()
        );
        assert!(
            cap.validate_config(&json!({ "auth": "free", "endpoint": "oauth" }))
                .is_err()
        );
        assert!(cap.validate_config(&json!({ "auth": "invalid" })).is_err());
        assert!(cap.validate_config(&json!({ "extra": true })).is_err());
    }

    #[test]
    fn localizations_cover_schema_summary_and_uk_name() {
        let cap = ParallelCapability;
        assert!(cap.describe_schema(None).is_some());
        assert_ne!(cap.localized_name(Some("uk-UA")), cap.name());
    }

    #[test]
    fn collection_merges_contributed_mcp_servers() {
        let mut registry = everruns_core::CapabilityRegistry::new();
        registry.register(ParallelCapability);
        let configs = vec![AgentCapabilityConfig::new(PARALLEL_CAPABILITY_ID)];

        let servers = collect_capability_mcp_servers(&configs, &registry);
        assert!(servers.contains_key(PARALLEL_SERVER_NAME));
    }
}
