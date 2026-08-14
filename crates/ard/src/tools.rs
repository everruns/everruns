//! `resource_discovery` tools: `discover_resources`, `attach_resource`,
//! `list_attached_resources`.
//!
//! Security posture (see SPEC.md / threat-model):
//! - Registry allowlist: the model selects a configured `registry_id`; raw URLs
//!   are never accepted from the model.
//! - Trust gate: `trustManifest` domain↔URN binding + `require_trust`
//!   attestations are enforced before any attach.
//! - SSRF: the resolved artifact URL is DNS-pinned validated before it is
//!   fetched (`validate_url_dns_pinned`); the live MCP/A2A endpoint URL is
//!   statically validated at attach time (`validate_safe_url`) and then
//!   DNS-pinned re-validated on every subsequent call by the scoped-server / A2A
//!   path. `allow_local_urls` (tests/dev only) relaxes both.
//! - `max_attachments` bounds prompt-injection-driven attach storms.
//! - All registry-returned text is treated as untrusted external data.

use async_trait::async_trait;
use serde_json::{Value, json};

use everruns_core::ard_attachment::{
    ARD_ATTACHMENT_KV_PREFIX, ARD_ATTACHMENT_RESOURCE_KIND, ARD_DISCOVERY_KV_PREFIX, ArdAttachment,
    ArdAttachmentTarget, attachment_kv_key, urn_slug,
};
use everruns_core::mcp_server::{McpServerTransportType, ScopedMcpServer};
use everruns_core::session_resource::{RegisterSessionResource, SessionResourceStatus};
use everruns_core::tool_context::ToolContext;
use everruns_core::tool_types::ToolHints;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::{validate_safe_url, validate_url_dns_pinned};
use everruns_http::DirectEgressService;

use crate::client::{
    ArdRegistryClient, CatalogEntry, MEDIA_TYPE_A2A_AGENT_CARD, MEDIA_TYPE_MCP_SERVER, SearchQuery,
    SearchRequest, TrustOutcome, a2a_target_from_artifact, mcp_endpoint_from_artifact, parse_urn,
    verify_trust,
};
use crate::config::ArdConfig;
use crate::connection::{ARD_API_KEY_SECRET, ARD_CONNECTION_PROVIDER};

/// Resolve the registry bearer token (connection provider → session secret).
/// Returns `None` when no token is configured — anonymous-read registries are
/// allowed, so this is not an error.
async fn resolve_token(context: &ToolContext) -> Option<String> {
    if let Some(resolver) = &context.connection_resolver
        && let Ok(Some(tok)) = resolver
            .get_connection_token(context.session_id, ARD_CONNECTION_PROVIDER)
            .await
        && !tok.is_empty()
    {
        return Some(tok);
    }
    if let Some(storage) = &context.storage_store
        && let Ok(Some(tok)) = storage
            .get_secret(context.session_id, ARD_API_KEY_SECRET)
            .await
        && !tok.is_empty()
    {
        return Some(tok);
    }
    None
}

/// Pick the registry the model named, or the sole configured registry.
fn resolve_registry<'a>(
    config: &'a ArdConfig,
    registry_id: Option<&str>,
) -> Result<&'a crate::config::RegistryConfig, ToolExecutionResult> {
    match registry_id {
        Some(id) => config.registry(id).ok_or_else(|| {
            ToolExecutionResult::tool_error(format!(
                "Unknown registry_id '{id}'. Configured registries: [{}]",
                config
                    .registries
                    .iter()
                    .map(|r| r.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        }),
        None => match config.registries.as_slice() {
            [single] => Ok(single),
            [] => Err(ToolExecutionResult::tool_error(
                "No ARD registries are configured for this agent.",
            )),
            _ => Err(ToolExecutionResult::tool_error(
                "Multiple registries configured; specify registry_id.",
            )),
        },
    }
}

// ============================================================================
// discover_resources
// ============================================================================

pub struct DiscoverResourcesTool {
    config: ArdConfig,
}

impl DiscoverResourcesTool {
    pub fn new(config: ArdConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for DiscoverResourcesTool {
    fn name(&self) -> &str {
        "discover_resources"
    }

    fn description(&self) -> &str {
        "Search configured Agentic Resource Discovery (ARD) registries for external capabilities \
         (MCP servers, A2A agents) you are not yet provisioned with. Returns ranked entries with a \
         URN you can pass to attach_resource. Search runs outside the model context."
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
                    "description": "Optional ARD structured filter (e.g. {\"type\": [\"application/mcp-server+json\"]})."
                },
                "registry_id": {
                    "type": "string",
                    "description": "Which configured registry to query. Optional when only one is configured."
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
        ToolExecutionResult::tool_error("discover_resources requires session context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let text = match arguments.get("text").and_then(|v| v.as_str()) {
            Some(t) if !t.trim().is_empty() => t.to_string(),
            _ => return ToolExecutionResult::tool_error("Missing required parameter: text"),
        };
        let registry_id = arguments.get("registry_id").and_then(|v| v.as_str());
        let registry = match resolve_registry(&self.config, registry_id) {
            Ok(r) => r,
            Err(e) => return e,
        };

        if let Some(acl) = &context.network_access
            && !acl.is_url_allowed(&registry.url)
        {
            return ToolExecutionResult::tool_error(
                "registry URL blocked by network access policy",
            );
        }
        if !self.config.allow_local_urls
            && let Err(e) = validate_safe_url(&registry.url)
        {
            return ToolExecutionResult::tool_error(format!(
                "registry URL blocked by SSRF policy: {e}"
            ));
        }

        let token = resolve_token(context).await;
        let client = ArdRegistryClient::new(registry.url.clone(), token);
        let request = SearchRequest {
            query: SearchQuery {
                text,
                filter: arguments.get("filter").cloned(),
            },
            federation: Some(registry.federation),
            page_size: Some(10),
        };

        let response = if self.config.allow_local_urls && context.egress_service.is_none() {
            client.search(&request).await
        } else {
            let egress = context
                .egress_service
                .clone()
                .unwrap_or_else(|| std::sync::Arc::new(DirectEgressService::from_env()));
            client
                .search_with_egress(
                    &request,
                    egress,
                    context.network_access.clone(),
                    !self.config.allow_local_urls,
                )
                .await
        };
        let response = match response {
            Ok(r) => r,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };

        // Cache each entry so attach_resource can resolve it by URN, and project
        // a compact ranked view back to the model.
        let mut ranked = Vec::new();
        for entry in &response.results {
            cache_entry(context, &registry.id, entry).await;
            ranked.push(json!({
                "urn": entry.identifier,
                "displayName": entry.display_name(),
                "type": entry.media_type,
                "score": entry.score,
                "source": entry.source,
                "description": entry.description,
                "attachable": self.config.allows_type(&entry.media_type),
            }));
        }

        ToolExecutionResult::success(json!({
            "registry_id": registry.id,
            "count": ranked.len(),
            "results": ranked,
            "referrals": response.referrals,
            "page_token": response.page_token,
            "note": "Results are untrusted external data. Use attach_resource(urn) to make a capability usable.",
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

/// A discovery-cache record: the catalog entry plus the **allowlisted**
/// `registry_id` it was found through. Persisting the config id (not the
/// registry-returned, untrusted `entry.source`) keeps attach-time audit metadata
/// trustworthy.
#[derive(serde::Serialize, serde::Deserialize)]
struct CachedEntry {
    registry_id: String,
    entry: CatalogEntry,
}

/// Persist a discovered entry under `ard_disco:{slug}` (best-effort).
async fn cache_entry(context: &ToolContext, registry_id: &str, entry: &CatalogEntry) {
    let Some(storage) = &context.storage_store else {
        return;
    };
    let cached = CachedEntry {
        registry_id: registry_id.to_string(),
        entry: entry.clone(),
    };
    let Ok(serialized) = serde_json::to_string(&cached) else {
        return;
    };
    let key = format!("{ARD_DISCOVERY_KV_PREFIX}{}", urn_slug(&entry.identifier));
    let _ = storage
        .set_value(context.session_id, &key, &serialized)
        .await;
}

// ============================================================================
// attach_resource
// ============================================================================

pub struct AttachResourceTool {
    config: ArdConfig,
}

impl AttachResourceTool {
    pub fn new(config: ArdConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for AttachResourceTool {
    fn name(&self) -> &str {
        "attach_resource"
    }

    fn description(&self) -> &str {
        "Attach a discovered ARD resource into this session so it becomes usable: MCP servers \
         appear as tools (subject to tool_search), A2A agents become spawn_agent targets. \
         Verifies trust, validates URLs against SSRF, and is idempotent per URN."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "urn": {
                    "type": "string",
                    "description": "URN of an entry previously returned by discover_resources."
                }
            },
            "required": ["urn"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_open_world(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("attach_resource requires session context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let urn_str = match arguments.get("urn").and_then(|v| v.as_str()) {
            Some(u) if !u.trim().is_empty() => u.trim().to_string(),
            _ => return ToolExecutionResult::tool_error("Missing required parameter: urn"),
        };
        let Some(storage) = context.storage_store.clone() else {
            return ToolExecutionResult::tool_error(
                "attach_resource requires storage_store context",
            );
        };
        let slug = urn_slug(&urn_str);

        // Idempotency: already attached?
        let attach_key = attachment_kv_key(&slug);
        if let Ok(Some(existing)) = storage.get_value(context.session_id, &attach_key).await
            && let Ok(att) = serde_json::from_str::<ArdAttachment>(&existing)
        {
            return ToolExecutionResult::success(json!({
                "urn": att.urn,
                "status": "already_attached",
                "display_name": att.display_name,
            }));
        }

        // Resolve the cached catalog entry plus the allowlisted registry id it
        // was discovered through.
        let disco_key = format!("{ARD_DISCOVERY_KV_PREFIX}{slug}");
        let cached: CachedEntry = match storage.get_value(context.session_id, &disco_key).await {
            Ok(Some(raw)) => match serde_json::from_str(&raw) {
                Ok(e) => e,
                Err(e) => {
                    return ToolExecutionResult::internal_error_msg(format!(
                        "cached entry for {urn_str} is corrupt: {e}"
                    ));
                }
            },
            _ => {
                return ToolExecutionResult::tool_error(format!(
                    "URN {urn_str} was not found in this session's discovery cache. \
                     Run discover_resources first."
                ));
            }
        };
        let registry_id = cached.registry_id;
        let entry = cached.entry;

        if let Err(e) = entry.validate_envelope() {
            return ToolExecutionResult::tool_error(e);
        }
        if !self.config.allows_type(&entry.media_type) {
            return ToolExecutionResult::tool_error(format!(
                "Media type '{}' is not permitted by allow_attach_types.",
                entry.media_type
            ));
        }

        // Trust gate.
        let urn = match parse_urn(&entry.identifier) {
            Ok(u) => u,
            Err(e) => return ToolExecutionResult::tool_error(format!("Invalid URN: {e}")),
        };
        if let TrustOutcome::Rejected(reason) = verify_trust(
            &urn,
            entry.trust_manifest.as_ref(),
            &self.config.require_trust,
        ) {
            return ToolExecutionResult::tool_error(format!("Trust verification failed: {reason}"));
        }

        // Enforce attachment cap (count distinct existing attachments).
        // Fail closed: if the count can't be determined, refuse rather than
        // bypass the threat-model bound.
        match count_attachments(context).await {
            Ok(n) if n >= self.config.max_attachments => {
                return ToolExecutionResult::tool_error(format!(
                    "max_attachments ({}) reached for this session.",
                    self.config.max_attachments
                ));
            }
            Ok(_) => {}
            Err(()) => {
                return ToolExecutionResult::tool_error(
                    "Could not verify the session's attachment count; refusing to attach \
                     (fail-closed on the max_attachments bound).",
                );
            }
        }

        // Resolve the artifact document (embedded `data` or fetched `url`).
        let artifact = match self.resolve_artifact(context, &entry).await {
            Ok(a) => a,
            Err(e) => return e,
        };

        // Map media type → attachment target, SSRF-validating the live endpoint.
        let target = match self.build_target(&entry, &artifact).await {
            Ok(t) => t,
            Err(e) => return e,
        };

        let attachment = ArdAttachment {
            urn: entry.identifier.clone(),
            display_name: entry.display_name(),
            media_type: entry.media_type.clone(),
            registry_id,
            target,
        };

        // Persist the attachment; turn-context assembly folds it into the
        // session config layer on the next turn.
        let serialized = match serde_json::to_string(&attachment) {
            Ok(s) => s,
            Err(e) => return ToolExecutionResult::internal_error_msg(e.to_string()),
        };
        if let Err(e) = storage
            .set_value(context.session_id, &attach_key, &serialized)
            .await
        {
            return ToolExecutionResult::internal_error(e);
        }

        // Register for visibility/audit (best-effort).
        register_resource(context, &attachment).await;

        let (kind, hint) = match &attachment.target {
            ArdAttachmentTarget::McpServer { name, .. } => (
                "mcp_server",
                format!(
                    "MCP tools will appear prefixed `mcp_{name}__*` on the next turn (subject to tool_search)."
                ),
            ),
            ArdAttachmentTarget::ExternalAgent { agent } => (
                "a2a_agent",
                format!(
                    "Use spawn_agent with target.external_agent_id = '{}' on the next turn.",
                    agent.get("id").and_then(|v| v.as_str()).unwrap_or("")
                ),
            ),
        };

        ToolExecutionResult::success(json!({
            "urn": attachment.urn,
            "status": "attached",
            "kind": kind,
            "display_name": attachment.display_name,
            "next_step": hint,
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

impl AttachResourceTool {
    /// Return the artifact document: the embedded `data`, or fetch the `url`
    /// after SSRF validation.
    async fn resolve_artifact(
        &self,
        context: &ToolContext,
        entry: &CatalogEntry,
    ) -> Result<Value, ToolExecutionResult> {
        if let Some(data) = &entry.data {
            return Ok(data.clone());
        }
        let url = entry
            .url
            .as_deref()
            .ok_or_else(|| ToolExecutionResult::tool_error("entry has no url or data"))?;
        let addrs = self.validate_url(context, url).await?;
        let token = resolve_token(context).await;
        let client = ArdRegistryClient::new(entry.source.clone().unwrap_or_default(), token);
        if self.config.allow_local_urls && context.egress_service.is_none() {
            return client
                .fetch_artifact(url, &addrs)
                .await
                .map_err(ToolExecutionResult::tool_error);
        }
        let egress = context
            .egress_service
            .clone()
            .unwrap_or_else(|| std::sync::Arc::new(DirectEgressService::from_env()));
        client
            .fetch_artifact_with_egress(url, &addrs, egress, context.network_access.clone())
            .await
            .map_err(ToolExecutionResult::tool_error)
    }

    /// Build the attachment target from the resolved artifact, SSRF-validating
    /// the live endpoint URL.
    async fn build_target(
        &self,
        entry: &CatalogEntry,
        artifact: &Value,
    ) -> Result<ArdAttachmentTarget, ToolExecutionResult> {
        match entry.media_type.as_str() {
            MEDIA_TYPE_MCP_SERVER => {
                let (url, headers) = mcp_endpoint_from_artifact(artifact)
                    .map_err(ToolExecutionResult::tool_error)?;
                // SSRF-validate the live MCP endpoint (re-validated by the
                // scoped-server path on each call too).
                self.validate_url_static(&url)?;
                let name = sanitize_server_name(&entry.identifier);
                Ok(ArdAttachmentTarget::McpServer {
                    name,
                    server: ScopedMcpServer {
                        transport_type: McpServerTransportType::Http,
                        url,
                        headers,
                        ..Default::default()
                    },
                })
            }
            MEDIA_TYPE_A2A_AGENT_CARD => {
                let (base_url, agent_card) =
                    a2a_target_from_artifact(artifact).map_err(ToolExecutionResult::tool_error)?;
                if let Some(base) = &base_url {
                    self.validate_url_static(base)?;
                }
                let mut agent = json!({
                    "id": sanitize_server_name(&entry.identifier),
                    "name": entry.display_name(),
                    "allow_local_urls": self.config.allow_local_urls,
                });
                if let Some(desc) = &entry.description {
                    agent["description"] = json!(desc);
                }
                if let Some(base) = base_url {
                    agent["base_url"] = json!(base);
                }
                if let Some(card) = agent_card {
                    agent["agent_card"] = card;
                }
                Ok(ArdAttachmentTarget::ExternalAgent { agent })
            }
            other => Err(ToolExecutionResult::tool_error(format!(
                "unsupported attachment media type '{other}'"
            ))),
        }
    }

    /// DNS-pinned SSRF validation; bypassed (parse-only) when allow_local_urls.
    async fn validate_url(
        &self,
        context: &ToolContext,
        url: &str,
    ) -> Result<Vec<std::net::SocketAddr>, ToolExecutionResult> {
        if let Some(acl) = &context.network_access
            && !acl.is_url_allowed(url)
        {
            return Err(ToolExecutionResult::tool_error(
                "URL blocked by network access policy",
            ));
        }
        if self.config.allow_local_urls {
            // Still require a syntactically valid http(s) URL.
            url::Url::parse(url)
                .map_err(|e| ToolExecutionResult::tool_error(format!("invalid url: {e}")))?;
            return Ok(Vec::new());
        }
        match validate_url_dns_pinned(url).await {
            Ok((_u, addrs)) => Ok(addrs),
            Err(e) => Err(ToolExecutionResult::tool_error(format!(
                "URL blocked by SSRF policy: {e}"
            ))),
        }
    }

    /// Static SSRF validation for endpoint URLs recorded into config.
    fn validate_url_static(&self, url: &str) -> Result<(), ToolExecutionResult> {
        if self.config.allow_local_urls {
            url::Url::parse(url)
                .map_err(|e| ToolExecutionResult::tool_error(format!("invalid url: {e}")))?;
            return Ok(());
        }
        validate_safe_url(url).map(|_| ()).map_err(|e| {
            ToolExecutionResult::tool_error(format!("URL blocked by SSRF policy: {e}"))
        })
    }
}

/// Count existing attachments in this session (distinct `ard_attach:` keys).
async fn count_attachments(context: &ToolContext) -> Result<usize, ()> {
    let Some(storage) = &context.storage_store else {
        return Ok(0);
    };
    let keys = storage
        .list_keys(context.session_id)
        .await
        .map_err(|_| ())?;
    Ok(keys
        .into_iter()
        .filter(|k| k.key.starts_with(ARD_ATTACHMENT_KV_PREFIX))
        .count())
}

/// Register an attachment in the session resource registry (best-effort).
async fn register_resource(context: &ToolContext, attachment: &ArdAttachment) {
    let Some(registry) = &context.session_resource_registry else {
        return;
    };
    let _ = registry
        .register(RegisterSessionResource {
            session_id: context.session_id,
            resource_id: format!("ard_{}", attachment.slug()),
            kind: ARD_ATTACHMENT_RESOURCE_KIND.to_string(),
            display_name: attachment.display_name.clone(),
            status: SessionResourceStatus::Active,
            metadata: json!({
                "urn": attachment.urn,
                "media_type": attachment.media_type,
                "registry_id": attachment.registry_id,
            }),
        })
        .await;
}

/// Derive a stable MCP/agent logical name from a URN's terminal segments.
///
/// Takes the last **non-empty** colon segment and collapses every run of
/// non-alphanumerics into a single `_`. Collapsing matters: the scoped-MCP
/// layer uses `__` as the `mcp_<server>__<tool>` separator and rejects server
/// names containing `__`, which would skip all scoped MCP tools for the turn.
fn sanitize_server_name(urn: &str) -> String {
    let tail = urn.rsplit(':').find(|s| !s.is_empty()).unwrap_or(urn);
    let mut name = String::with_capacity(tail.len());
    let mut prev_underscore = false;
    for c in tail.chars() {
        if c.is_ascii_alphanumeric() {
            name.push(c.to_ascii_lowercase());
            prev_underscore = false;
        } else if !prev_underscore {
            name.push('_');
            prev_underscore = true;
        }
    }
    let trimmed = name.trim_matches('_');
    if trimmed.is_empty() {
        "ard_resource".to_string()
    } else {
        trimmed.to_string()
    }
}

// ============================================================================
// list_attached_resources
// ============================================================================

pub struct ListAttachedResourcesTool;

#[async_trait]
impl Tool for ListAttachedResourcesTool {
    fn name(&self) -> &str {
        "list_attached_resources"
    }

    fn description(&self) -> &str {
        "List capabilities attached to this session via attach_resource (for visibility/audit). \
         Attachments are torn down with the session."
    }

    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("list_attached_resources requires session context")
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let Some(storage) = &context.storage_store else {
            return ToolExecutionResult::tool_error(
                "list_attached_resources requires storage_store context",
            );
        };
        let keys = match storage.list_keys(context.session_id).await {
            Ok(k) => k,
            Err(e) => return ToolExecutionResult::internal_error(e),
        };
        let mut items = Vec::new();
        for info in keys {
            if !info.key.starts_with(ARD_ATTACHMENT_KV_PREFIX) {
                continue;
            }
            if let Ok(Some(raw)) = storage.get_value(context.session_id, &info.key).await
                && let Ok(att) = serde_json::from_str::<ArdAttachment>(&raw)
            {
                let kind = match &att.target {
                    ArdAttachmentTarget::McpServer { name, .. } => {
                        json!({ "type": "mcp_server", "server_name": name })
                    }
                    ArdAttachmentTarget::ExternalAgent { agent } => json!({
                        "type": "a2a_agent",
                        "external_agent_id": agent.get("id"),
                    }),
                };
                items.push(json!({
                    "urn": att.urn,
                    "display_name": att.display_name,
                    "media_type": att.media_type,
                    "registry_id": att.registry_id,
                    "target": kind,
                }));
            }
        }
        ToolExecutionResult::success(json!({ "count": items.len(), "attachments": items }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_server_name_from_urn() {
        assert_eq!(
            sanitize_server_name("urn:ai:acme.com:agent:Weather-Bot"),
            "weather_bot"
        );
        // Runs of non-alphanumerics collapse to a single `_` so the name can
        // never contain the `__` scoped-MCP separator.
        assert_eq!(
            sanitize_server_name("urn:ai:acme.com:agent:Weather--Bot"),
            "weather_bot"
        );
        assert!(!sanitize_server_name("urn:ai:acme.com:agent:a..b__c").contains("__"));
        // Trailing empty segments are skipped; the last non-empty wins.
        assert_eq!(sanitize_server_name("urn:ai:x.com:agent:"), "agent");
    }

    #[test]
    fn discover_schema_requires_text() {
        let tool = DiscoverResourcesTool::new(ArdConfig::default());
        let schema = tool.parameters_schema();
        assert_eq!(schema["required"][0], "text");
    }

    #[test]
    fn resolve_registry_single_default() {
        let cfg = ArdConfig::from_value(&json!({
            "registries": [{"id": "only", "url": "https://r.example.com"}]
        }))
        .unwrap();
        assert_eq!(resolve_registry(&cfg, None).unwrap().id, "only");
    }

    #[test]
    fn resolve_registry_requires_id_when_many() {
        let cfg = ArdConfig::from_value(&json!({
            "registries": [
                {"id": "a", "url": "https://a.example.com"},
                {"id": "b", "url": "https://b.example.com"}
            ]
        }))
        .unwrap();
        assert!(resolve_registry(&cfg, None).is_err());
        assert_eq!(resolve_registry(&cfg, Some("b")).unwrap().id, "b");
    }

    #[test]
    fn federation_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&crate::client::Federation::Referrals).unwrap(),
            "\"referrals\""
        );
    }

    #[test]
    fn unknown_media_type_constants() {
        assert_eq!(MEDIA_TYPE_MCP_SERVER, "application/mcp-server+json");
        assert_eq!(MEDIA_TYPE_A2A_AGENT_CARD, "application/a2a-agent-card+json");
    }
}
