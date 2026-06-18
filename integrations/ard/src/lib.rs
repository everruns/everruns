//! Agentic Resource Discovery (ARD) client integration (Experimental).
//!
//! Provides the `resource_discovery` capability: agents discover external
//! capabilities (MCP servers, A2A agents) via operator-configured ARD registries
//! ([spec](https://agenticresourcediscovery.org/spec/)) and attach them into the
//! session config layer.
//!
//! # Example
//!
//! ```
//! use everruns_core::capabilities::Capability;
//! use everruns_integrations_ard::ResourceDiscoveryCapability;
//!
//! let capability = ResourceDiscoveryCapability;
//! assert_eq!(capability.id(), "resource_discovery");
//! ```
//!
//! # Design notes
//!
//! - Registries are operator-configured. The model selects a `registry_id`; it
//!   can never supply a raw URL. This is the primary SSRF/exfiltration control.
//! - All registry responses are treated as untrusted external data.
//! - `attach_resource` runs a security gauntlet (trust gate, value-or-reference
//!   envelope check, SSRF validation, `max_attachments` cap) before recording a
//!   session-scoped attachment.

pub mod client;
pub mod config;
mod tools;

/// Test-support constructors for the tool implementations.
///
/// The tool structs are an internal detail; this module exposes builders so
/// integration tests can drive `execute_with_context` directly. Not part of the
/// stable public surface.
#[doc(hidden)]
pub mod test_support {
    use crate::config::ResourceDiscoveryConfig;
    use crate::tools::{AttachResourceTool, DiscoverResourcesTool, ListAttachedResourcesTool};
    use everruns_core::tools::Tool;

    /// Build a `discover_resources` tool with the given config.
    pub fn discover_tool(config: ResourceDiscoveryConfig) -> impl Tool {
        DiscoverResourcesTool::new(config)
    }

    /// Build an `attach_resource` tool with the given config.
    pub fn attach_tool(config: ResourceDiscoveryConfig) -> impl Tool {
        AttachResourceTool::new(config)
    }

    /// Build a `list_attached_resources` tool.
    pub fn list_tool() -> impl Tool {
        ListAttachedResourcesTool
    }
}

use everruns_core::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, IntegrationPlugin,
};
use everruns_core::tools::Tool;
use serde_json::{Value, json};

use config::ResourceDiscoveryConfig;
use tools::{AttachResourceTool, DiscoverResourcesTool, ListAttachedResourcesTool};

// ============================================================================
// Plugin Registration
// ============================================================================

inventory::submit! {
    IntegrationPlugin {
        experimental_only: true,
        feature_flag: None,
        factory: || Box::new(ResourceDiscoveryCapability),
    }
}

// ============================================================================
// Capability
// ============================================================================

/// The `resource_discovery` capability.
pub struct ResourceDiscoveryCapability;

impl Capability for ResourceDiscoveryCapability {
    fn id(&self) -> &str {
        "resource_discovery"
    }

    fn name(&self) -> &str {
        "[Experimental] Resource Discovery (ARD)"
    }

    fn description(&self) -> &str {
        "Discover and attach external capabilities (MCP servers, A2A agents) via \
         Agentic Resource Discovery (ARD) registries. EXPERIMENTAL: this capability \
         may change."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("compass")
    }

    fn category(&self) -> Option<&str> {
        Some("Network")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            "Use `discover_resources` to find external MCP servers / A2A agents in \
             configured ARD registries, then `attach_resource` to make one usable \
             in this session; `list_attached_resources` shows what is attached.",
        )
    }

    fn config_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "title": "Resource Discovery (ARD) Settings",
            "properties": {
                "registries": {
                    "type": "array",
                    "description": "Operator-configured ARD registries. The model selects one by id; it can never supply a raw URL.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string" },
                            "url": { "type": "string", "format": "uri" },
                            "federation": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "RESERVED / not yet enforced: intended allowlist of registry hostnames this registry may federate to. The client does not follow federated links today, so this has no effect yet."
                            }
                        },
                        "required": ["id", "url"],
                        "additionalProperties": false
                    }
                },
                "require_trust": {
                    "type": "boolean",
                    "default": true,
                    "description": "Require a verifiable trust manifest before any attach."
                },
                "allow_attach_types": {
                    "type": "array",
                    "items": { "type": "string", "enum": ["mcp_server", "a2a_agent"] },
                    "description": "Attachment kinds permitted. Empty means all supported kinds."
                },
                "max_attachments": {
                    "type": "integer",
                    "minimum": 0,
                    "default": 8,
                    "description": "Maximum resources attachable per session."
                },
                "allow_local_urls": {
                    "type": "boolean",
                    "default": false,
                    "description": "Permit resolved URLs pointing at local/private addresses (test/dev only)."
                }
            },
            "additionalProperties": false
        }))
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        let parsed: ResourceDiscoveryConfig = serde_json::from_value(config.clone())
            .map_err(|e| format!("Invalid resource_discovery config: {e}"))?;
        // Registry ids must be unique and URLs syntactically valid.
        let mut seen = std::collections::HashSet::new();
        for r in &parsed.registries {
            if r.id.trim().is_empty() {
                return Err("registry id must not be empty".to_string());
            }
            if !seen.insert(r.id.clone()) {
                return Err(format!("duplicate registry id `{}`", r.id));
            }
            if url::Url::parse(&r.url).is_err() {
                return Err(format!("registry `{}` has an invalid url", r.id));
            }
        }
        for t in &parsed.allow_attach_types {
            if t != "mcp_server" && t != "a2a_agent" {
                return Err(format!("unknown allow_attach_types value `{t}`"));
            }
        }
        Ok(())
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        // Default (no config): tools with an empty registry list.
        self.tools_with_config(&Value::Null)
    }

    fn tools_with_config(&self, config: &Value) -> Vec<Box<dyn Tool>> {
        let cfg = ResourceDiscoveryConfig::from_value(config);
        vec![
            Box::new(DiscoverResourcesTool::new(cfg.clone())),
            Box::new(AttachResourceTool::new(cfg)),
            Box::new(ListAttachedResourcesTool),
        ]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec![]
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "[Експериментально] Виявлення ресурсів (ARD)",
            "Знаходьте та підключайте зовнішні можливості (MCP-сервери, A2A-агенти) \
             через реєстри Agentic Resource Discovery (ARD). ЕКСПЕРИМЕНТАЛЬНО: ця \
             можливість може змінитися.",
        )]
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_metadata() {
        let cap = ResourceDiscoveryCapability;
        assert_eq!(cap.id(), "resource_discovery");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.category(), Some("Network"));
    }

    #[test]
    fn capability_has_three_tools() {
        let cap = ResourceDiscoveryCapability;
        let names: Vec<String> = cap.tools().iter().map(|t| t.name().to_string()).collect();
        assert_eq!(names.len(), 3);
        assert!(names.contains(&"discover_resources".to_string()));
        assert!(names.contains(&"attach_resource".to_string()));
        assert!(names.contains(&"list_attached_resources".to_string()));
    }

    #[test]
    fn all_tools_require_context() {
        let cap = ResourceDiscoveryCapability;
        for tool in cap.tools() {
            assert!(
                tool.requires_context(),
                "tool {} should require context",
                tool.name()
            );
        }
    }

    #[test]
    fn config_schema_present() {
        let cap = ResourceDiscoveryCapability;
        let schema = cap.config_schema().unwrap();
        assert!(schema["properties"]["registries"].is_object());
        assert!(schema["properties"]["require_trust"].is_object());
        assert!(schema["properties"]["max_attachments"].is_object());
        assert!(schema["properties"]["allow_local_urls"].is_object());
    }

    #[test]
    fn validate_config_accepts_valid() {
        let cap = ResourceDiscoveryCapability;
        let cfg = json!({
            "registries": [{ "id": "main", "url": "https://ard.example.com" }],
            "require_trust": true,
            "max_attachments": 4
        });
        assert!(cap.validate_config(&cfg).is_ok());
    }

    #[test]
    fn validate_config_rejects_duplicate_ids() {
        let cap = ResourceDiscoveryCapability;
        let cfg = json!({
            "registries": [
                { "id": "x", "url": "https://a.example.com" },
                { "id": "x", "url": "https://b.example.com" }
            ]
        });
        assert!(cap.validate_config(&cfg).is_err());
    }

    #[test]
    fn validate_config_rejects_bad_url() {
        let cap = ResourceDiscoveryCapability;
        let cfg = json!({ "registries": [{ "id": "x", "url": "not a url" }] });
        assert!(cap.validate_config(&cfg).is_err());
    }

    #[test]
    fn validate_config_rejects_unknown_attach_type() {
        let cap = ResourceDiscoveryCapability;
        let cfg = json!({ "allow_attach_types": ["skill"] });
        assert!(cap.validate_config(&cfg).is_err());
    }

    #[test]
    fn empty_config_is_valid() {
        let cap = ResourceDiscoveryCapability;
        assert!(cap.validate_config(&json!({})).is_ok());
    }
}
