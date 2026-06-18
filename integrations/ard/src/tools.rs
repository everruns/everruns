//! ARD client tools: `discover_resources`, `attach_resource`, `list_attached_resources`.
//!
//! Security model (all enforced here):
//! - Registry allowlist: the model selects a configured `registry_id`; it can
//!   never supply a raw registry URL.
//! - `require_trust`: trust-manifest gate runs before any attach.
//! - SSRF: every resolved resource URL is validated with the shared
//!   `validate_safe_url` helper; `allow_local_urls` bypasses ONLY local-address
//!   blocking and defaults false.
//! - `max_attachments`: per-session cap, checked against the session resource
//!   registry.
//! - All registry-returned text/JSON is treated as untrusted external data.

use std::sync::Arc;

use async_trait::async_trait;
use everruns_core::ToolHints;
use everruns_core::session_resource::{
    RegisterSessionResource, SessionResourceFilter, SessionResourceStatus,
};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::{SessionResourceRegistry, ToolContext};
use everruns_core::typed_id::SessionId;
use everruns_core::url_validation::{UrlValidationError, validate_safe_url};
use serde_json::{Value, json};

use crate::client::RegistryClient;
use crate::config::{ResolvedEntry, ResourceDiscoveryConfig};

/// `kind` values this capability records into the session resource registry.
const RESOURCE_KINDS: [&str; 2] = ["mcp_server", "external_a2a_agent"];

// ============================================================================
// discover_resources
// ============================================================================

/// Proxies `POST /search` to a configured registry and returns ranked results.
pub struct DiscoverResourcesTool {
    config: ResourceDiscoveryConfig,
}

impl DiscoverResourcesTool {
    pub fn new(config: ResourceDiscoveryConfig) -> Self {
        Self { config }
    }
}

/// Pick the registry: explicit `registry_id` if given, else the sole configured
/// one. Returns an error result if ambiguous or unknown.
fn select_registry<'a>(
    config: &'a ResourceDiscoveryConfig,
    registry_id: Option<&str>,
) -> Result<&'a crate::config::RegistryConfig, ToolExecutionResult> {
    match registry_id {
        Some(id) => config.registry(id).ok_or_else(|| {
            ToolExecutionResult::tool_error(format!(
                "Unknown registry_id `{id}`. Configured registries: {}",
                configured_ids(config)
            ))
        }),
        None => match config.registries.as_slice() {
            [] => Err(ToolExecutionResult::tool_error(
                "No ARD registries are configured for resource_discovery.",
            )),
            [only] => Ok(only),
            _ => Err(ToolExecutionResult::tool_error(format!(
                "registry_id is required (multiple registries configured): {}",
                configured_ids(config)
            ))),
        },
    }
}

fn configured_ids(config: &ResourceDiscoveryConfig) -> String {
    config
        .registries
        .iter()
        .map(|r| r.id.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

#[async_trait]
impl Tool for DiscoverResourcesTool {
    fn name(&self) -> &str {
        "discover_resources"
    }

    fn description(&self) -> &str {
        "Discover external capabilities (MCP servers, A2A agents) via an Agentic \
         Resource Discovery (ARD) registry. Returns ranked candidates by URN; use \
         `attach_resource` to make one usable in this session."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "Natural-language description of the capability you need."
                },
                "filter": {
                    "type": "object",
                    "description": "Optional structured filter passed through to the registry.",
                    "additionalProperties": true
                },
                "registry_id": {
                    "type": "string",
                    "description": "ID of a configured registry to search. Required when more than one is configured."
                }
            },
            "required": ["text"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
            .with_open_world(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("discover_resources requires session context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        _context: &ToolContext,
    ) -> ToolExecutionResult {
        let text = match arguments.get("text").and_then(|v| v.as_str()) {
            Some(t) if !t.is_empty() => t,
            _ => return ToolExecutionResult::tool_error("Missing required parameter: text"),
        };
        let registry_id = arguments.get("registry_id").and_then(|v| v.as_str());
        let registry = match select_registry(&self.config, registry_id) {
            Ok(r) => r,
            Err(e) => return e,
        };

        let mut body = json!({ "text": text });
        if let Some(filter) = arguments.get("filter") {
            body["filter"] = filter.clone();
        }

        let client = RegistryClient::new(&registry.url, self.config.allow_local_urls);
        match client.search(body).await {
            Ok(resp) => {
                let results: Vec<Value> = resp
                    .results
                    .iter()
                    .map(|hit| {
                        json!({
                            "urn": hit.urn,
                            "displayName": hit.display_name,
                            "type": hit.media_type,
                            "score": hit.score,
                            "source": registry.id,
                            "description": hit.description,
                        })
                    })
                    .collect();
                ToolExecutionResult::success(json!({
                    "registry_id": registry.id,
                    "count": results.len(),
                    "results": results,
                }))
            }
            Err(e) => ToolExecutionResult::tool_error(e.to_string()),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// attach_resource
// ============================================================================

/// Resolves a URN, runs the full security gauntlet, and records a session-scoped
/// attachment (MCP server or external A2A agent) in the session resource registry.
pub struct AttachResourceTool {
    config: ResourceDiscoveryConfig,
}

impl AttachResourceTool {
    pub fn new(config: ResourceDiscoveryConfig) -> Self {
        Self { config }
    }
}

/// Count currently-attached ARD resources for a session.
async fn count_attachments(
    registry: &Arc<dyn SessionResourceRegistry>,
    session_id: SessionId,
) -> usize {
    let mut total = 0usize;
    for kind in RESOURCE_KINDS {
        let filter = SessionResourceFilter {
            kind: Some(kind.to_string()),
            status: None,
        };
        if let Ok(entries) = registry.list(session_id, Some(&filter)).await {
            total += entries
                .iter()
                .filter(|e| e.status != SessionResourceStatus::Released)
                .count();
        }
    }
    total
}

/// SSRF-validate a resolved resource URL, honoring `allow_local_urls`.
fn validate_resolved_url(url: &str, allow_local_urls: bool) -> Result<(), String> {
    match validate_safe_url(url) {
        Ok(_) => Ok(()),
        Err(UrlValidationError::BlockedHost(_)) if allow_local_urls => Ok(()),
        Err(e) => Err(format!("Resolved resource URL is not allowed: {e}")),
    }
}

#[async_trait]
impl Tool for AttachResourceTool {
    fn name(&self) -> &str {
        "attach_resource"
    }

    fn description(&self) -> &str {
        "Attach a discovered ARD resource (by URN) into this session: MCP servers \
         become session-scoped tool providers and A2A agent cards become external \
         delegation targets. Verifies trust and validates the resolved endpoint \
         before attaching. Idempotent per URN."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "urn": {
                    "type": "string",
                    "description": "URN of a resource returned by discover_resources."
                },
                "registry_id": {
                    "type": "string",
                    "description": "ID of the configured registry that surfaced the URN. Required when more than one is configured."
                }
            },
            "required": ["urn"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        // Attaching mutates session state; not read-only. Idempotent per URN.
        ToolHints::default()
            .with_idempotent(true)
            .with_open_world(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("attach_resource requires session context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let urn = match arguments.get("urn").and_then(|v| v.as_str()) {
            Some(u) if !u.is_empty() => u.to_string(),
            _ => return ToolExecutionResult::tool_error("Missing required parameter: urn"),
        };
        let registry_id = arguments.get("registry_id").and_then(|v| v.as_str());
        let registry = match select_registry(&self.config, registry_id) {
            Ok(r) => r,
            Err(e) => return e,
        };

        let Some(resource_registry) = context.session_resource_registry.clone() else {
            return ToolExecutionResult::internal_error_msg(
                "session resource registry unavailable; cannot attach resources",
            );
        };

        // Idempotency: a prior attach of this URN short-circuits.
        if let Ok(Some(existing)) = resource_registry.get(context.session_id, &urn).await {
            return ToolExecutionResult::success(json!({
                "urn": urn,
                "status": "already_attached",
                "kind": existing.kind,
                "display_name": existing.display_name,
            }));
        }

        // Enforce the per-session cap BEFORE doing remote work.
        let current = count_attachments(&resource_registry, context.session_id).await;
        if current >= self.config.max_attachments {
            return ToolExecutionResult::tool_error(format!(
                "Attachment limit reached ({current}/{}). Detach a resource before attaching another.",
                self.config.max_attachments
            ));
        }

        // Resolve from the registry (untrusted response).
        let client = RegistryClient::new(&registry.url, self.config.allow_local_urls);
        let entry: ResolvedEntry = match client.resolve(&urn).await {
            Ok(e) => e,
            Err(e) => return ToolExecutionResult::tool_error(e.to_string()),
        };

        // Envelope must be value-or-reference, never both.
        if let Err(e) = entry.validate_value_or_reference() {
            return ToolExecutionResult::tool_error(e.to_string());
        }

        // Media type -> attachment kind, and config must permit it.
        let kind = match entry.attachment_kind() {
            Ok(k) => k,
            Err(e) => return ToolExecutionResult::tool_error(e.to_string()),
        };
        if !self.config.allows_kind(kind) {
            return ToolExecutionResult::tool_error(format!(
                "Attachment type `{}` is not permitted by resource_discovery config.",
                kind.config_token()
            ));
        }

        // Trust gate before any attach.
        if let Err(e) = entry.verify_trust(self.config.require_trust) {
            return ToolExecutionResult::tool_error(e.to_string());
        }

        // SSRF-validate the resolved endpoint URL (reference form, or a `url`
        // field inside an inline descriptor).
        if let Some(url) = resolved_endpoint_url(&entry)
            && let Err(msg) = validate_resolved_url(&url, self.config.allow_local_urls)
        {
            return ToolExecutionResult::tool_error(msg);
        }

        let display_name = entry.display_name.clone().unwrap_or_else(|| urn.clone());

        // Materialize: record the session-scoped attachment in the session
        // resource registry. The runtime consumes session-scoped mcpServers /
        // external A2A agents from their config layer; recording here gives the
        // attachment durable, idempotent visibility. (See SPEC.md "Runtime
        // attach seam" — runtime consumption of tool-attached resources is a
        // documented follow-up gated on a `SessionMutator` overlay API.)
        let metadata = json!({
            "ard_urn": urn,
            "ard_registry_id": registry.id,
            "ard_media_type": entry.media_type,
            "attachment_kind": kind.config_token(),
            "endpoint_url": resolved_endpoint_url(&entry),
            "trusted": entry.trust_manifest.is_some(),
        });

        let register = RegisterSessionResource {
            session_id: context.session_id,
            resource_id: urn.clone(),
            kind: kind.resource_kind().to_string(),
            display_name: display_name.clone(),
            status: SessionResourceStatus::Active,
            metadata,
        };

        match resource_registry.register(register).await {
            Ok(_) => ToolExecutionResult::success(json!({
                "urn": urn,
                "status": "attached",
                "kind": kind.config_token(),
                "display_name": display_name,
                "registry_id": registry.id,
            })),
            Err(e) => {
                ToolExecutionResult::internal_error_msg(format!("Failed to record attachment: {e}"))
            }
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

/// Extract the endpoint URL to SSRF-validate: the reference `url` if present,
/// else a `url` / `base_url` field inside an inline descriptor.
fn resolved_endpoint_url(entry: &ResolvedEntry) -> Option<String> {
    if let Some(url) = &entry.url {
        return Some(url.clone());
    }
    let data = entry.data.as_ref()?;
    for key in ["url", "base_url", "baseUrl", "endpoint"] {
        if let Some(s) = data.get(key).and_then(|v| v.as_str()) {
            return Some(s.to_string());
        }
    }
    // For an A2A agent card, the endpoint may live under supported interfaces.
    if let Some(ifaces) = data.get("supported_interfaces").and_then(|v| v.as_array()) {
        for iface in ifaces {
            if let Some(s) = iface.get("url").and_then(|v| v.as_str()) {
                return Some(s.to_string());
            }
        }
    }
    None
}

// ============================================================================
// list_attached_resources
// ============================================================================

/// Lists ARD attachments recorded for the session.
pub struct ListAttachedResourcesTool;

#[async_trait]
impl Tool for ListAttachedResourcesTool {
    fn name(&self) -> &str {
        "list_attached_resources"
    }

    fn description(&self) -> &str {
        "List external resources (MCP servers, A2A agents) attached to this \
         session via ARD discovery."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("list_attached_resources requires session context.")
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let Some(registry) = context.session_resource_registry.clone() else {
            return ToolExecutionResult::success(json!({ "count": 0, "resources": [] }));
        };

        let mut resources: Vec<Value> = Vec::new();
        for kind in RESOURCE_KINDS {
            let filter = SessionResourceFilter {
                kind: Some(kind.to_string()),
                status: None,
            };
            if let Ok(entries) = registry.list(context.session_id, Some(&filter)).await {
                for e in entries {
                    resources.push(json!({
                        "urn": e.resource_id,
                        "kind": e.kind,
                        "display_name": e.display_name,
                        "status": e.status.to_string(),
                        "metadata": e.metadata,
                    }));
                }
            }
        }

        ToolExecutionResult::success(json!({
            "count": resources.len(),
            "resources": resources,
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RegistryConfig;

    fn cfg(registries: Vec<RegistryConfig>) -> ResourceDiscoveryConfig {
        ResourceDiscoveryConfig {
            registries,
            ..Default::default()
        }
    }

    #[test]
    fn select_registry_requires_id_when_ambiguous() {
        let c = cfg(vec![
            RegistryConfig {
                id: "a".into(),
                url: "https://a.example".into(),
                federation: vec![],
            },
            RegistryConfig {
                id: "b".into(),
                url: "https://b.example".into(),
                federation: vec![],
            },
        ]);
        assert!(select_registry(&c, None).is_err());
        assert!(select_registry(&c, Some("a")).is_ok());
        assert!(select_registry(&c, Some("missing")).is_err());
    }

    #[test]
    fn select_registry_defaults_to_sole() {
        let c = cfg(vec![RegistryConfig {
            id: "only".into(),
            url: "https://only.example".into(),
            federation: vec![],
        }]);
        assert_eq!(select_registry(&c, None).unwrap().id, "only");
    }

    #[test]
    fn select_registry_errors_when_none_configured() {
        let c = cfg(vec![]);
        assert!(select_registry(&c, None).is_err());
    }

    #[test]
    fn validate_resolved_url_blocks_local_by_default() {
        assert!(validate_resolved_url("http://127.0.0.1/mcp", false).is_err());
        assert!(validate_resolved_url("http://localhost/mcp", false).is_err());
    }

    #[test]
    fn validate_resolved_url_allows_local_when_opted_in() {
        assert!(validate_resolved_url("http://127.0.0.1/mcp", true).is_ok());
    }

    #[test]
    fn validate_resolved_url_never_bypasses_scheme() {
        assert!(validate_resolved_url("file:///etc/passwd", true).is_err());
    }

    #[test]
    fn validate_resolved_url_allows_public() {
        assert!(validate_resolved_url("https://mcp.example.com/v1/mcp", false).is_ok());
    }

    #[test]
    fn resolved_endpoint_url_prefers_reference() {
        let entry = ResolvedEntry {
            urn: "urn:ard:example.com:foo".into(),
            display_name: None,
            media_type: "application/mcp-server+json".into(),
            url: Some("https://mcp.example.com".into()),
            data: None,
            trust_manifest: None,
        };
        assert_eq!(
            resolved_endpoint_url(&entry).as_deref(),
            Some("https://mcp.example.com")
        );
    }

    #[test]
    fn resolved_endpoint_url_reads_inline_agent_card_interface() {
        let entry = ResolvedEntry {
            urn: "urn:ard:example.com:agent".into(),
            display_name: None,
            media_type: "application/a2a-agent-card+json".into(),
            url: None,
            data: Some(json!({
                "name": "Agent",
                "supported_interfaces": [{ "url": "https://agent.example.com/jsonrpc", "type": "JSONRPC" }]
            })),
            trust_manifest: None,
        };
        assert_eq!(
            resolved_endpoint_url(&entry).as_deref(),
            Some("https://agent.example.com/jsonrpc")
        );
    }

    #[tokio::test]
    async fn discover_without_context_errors() {
        let tool = DiscoverResourcesTool::new(ResourceDiscoveryConfig::default());
        let r = tool.execute(json!({ "text": "x" })).await;
        assert!(matches!(r, ToolExecutionResult::ToolError(_)));
    }

    #[test]
    fn tools_have_stable_names() {
        assert_eq!(
            DiscoverResourcesTool::new(ResourceDiscoveryConfig::default()).name(),
            "discover_resources"
        );
        assert_eq!(
            AttachResourceTool::new(ResourceDiscoveryConfig::default()).name(),
            "attach_resource"
        );
        assert_eq!(ListAttachedResourcesTool.name(), "list_attached_resources");
    }

    #[test]
    fn attach_is_not_readonly() {
        let tool = AttachResourceTool::new(ResourceDiscoveryConfig::default());
        assert!(!tool.hints().readonly.unwrap_or(false));
    }
}
