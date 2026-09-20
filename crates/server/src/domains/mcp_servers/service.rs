// MCP Server service for business logic
// Handles MCP server CRUD and tool discovery/caching
//
// Spec: knowledge/integrations/mcp.md (umbrella), knowledge/integrations/mcp-servers.md (detail)
//
// Note (EVE-316): this module was moved from `crate::services::mcp_server` to
// `crate::domains::mcp_servers::service`. The `McpServerService` type, the
// settings structs (`McpServerSettings`, `McpServerOAuthSettings`), and the
// resolved tool-cache structs (`McpServerResolved`, `McpServerWithTools`) all
// live here together because they share internal helpers (encryption,
// settings mapping, tool fetching).

use crate::storage::{
    EncryptionService, McpServerRow, StorageBackend,
    models::{CreateMcpServerRow, UpdateMcpServer, UpdateMcpServerTools},
};
use anyhow::{Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use chrono::{DateTime, Utc};
use everruns_core::{
    Caller, EgressService, McpProtocolMode, McpServer, McpServerActsAs, McpServerAuthMode,
    McpServerStatus, McpToolDefinition, mcp_oauth_provider_id_for_uuid,
};
use everruns_host::DirectEgressService;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

use crate::domains::mcp_servers::types::{CreateMcpServerRequest, UpdateMcpServerRequest};

/// How long cached tools are considered fresh (1 hour).
const TOOL_CACHE_TTL: Duration = Duration::from_secs(3600);

/// Maximum age for serving stale MCP tool definitions while revalidating.
///
/// MCP servers are untrusted inputs to agent tool resolution. Stale caches make
/// upstream outages less disruptive, but an unbounded stale window would let a
/// poisoned tool list persist indefinitely if future `tools/list` calls fail.
const TOOL_CACHE_MAX_STALE: Duration = Duration::from_secs(24 * 3600);

/// Identifies a server's tool cache for single-flight coordination.
type RefreshKey = (i64, Uuid);

/// Process-wide single-flight coordinator for MCP tool refreshes.
///
/// Resolving an agent's MCP tools is on the hot path: many concurrent turns and
/// sessions can ask for the same server's tools at once. Without coordination a
/// cold cache or a stale-cache read would fan every concurrent caller out into
/// its own upstream `tools/list` fetch (a thundering herd). This holds one async
/// lock per `(org, server)` so:
///   - blocking (cold-cache / forced) refreshes serialize and the late callers
///     pick up the freshly written cache instead of re-fetching, and
///   - background (stale-while-revalidate) refreshes use `try_lock`, so only one
///     runs at a time and extra triggers are dropped.
static REFRESH_LOCKS: LazyLock<KeyedLocks> = LazyLock::new(KeyedLocks::default);

#[derive(Default)]
struct KeyedLocks {
    map: Mutex<HashMap<RefreshKey, Arc<AsyncMutex<()>>>>,
}

impl KeyedLocks {
    /// Return the shared async lock for `key`, creating it on first use. Idle
    /// locks (held only by this map) are pruned to keep the map bounded by the
    /// number of servers currently refreshing rather than ever seen.
    fn lock_for(&self, key: RefreshKey) -> Arc<AsyncMutex<()>> {
        // Best-effort coordinator: recover from a poisoned mutex (a thread
        // panicked while holding it) rather than propagating that panic into
        // every subsequent tool resolution. The guarded map is plain data, so
        // continuing with the recovered value is safe.
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|k, lock| *k == key || Arc::strong_count(lock) > 1);
        map.entry(key).or_default().clone()
    }
}

#[derive(Clone)]
pub struct McpServerService {
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
    egress_service: Arc<dyn EgressService>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpServerSettings {
    #[serde(default)]
    pub auth_mode: McpServerAuthMode,
    /// Protocol-era adoption policy. Persisted in the `settings` JSON so no
    /// schema migration is needed; absent (legacy rows) deserializes to `auto`.
    #[serde(default, skip_serializing_if = "McpProtocolMode::is_auto")]
    pub protocol_mode: McpProtocolMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth: Option<McpServerOAuthSettings>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpServerOAuthSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registration_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes_supported: Vec<String>,
    /// Exact scope value requested by this preset. Providers do not all use
    /// the same delimiter, so this is not derived from `scopes_supported`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// RFC 8707 protected-resource identifier discovered for the MCP server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    /// Extra authorization parameters used only for service-owned grants.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub service_authorization_params: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret_encrypted: Option<String>,
}

impl McpServerService {
    pub fn new(db: Arc<StorageBackend>, encryption: Option<Arc<EncryptionService>>) -> Self {
        Self::with_egress_service(
            db,
            encryption,
            Arc::new(DirectEgressService::for_runtime_traffic_from_env()),
        )
    }

    pub fn with_egress_service(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        egress_service: Arc<dyn EgressService>,
    ) -> Self {
        Self {
            db,
            encryption,
            egress_service,
        }
    }

    pub fn egress_service(&self) -> Arc<dyn EgressService> {
        self.egress_service.clone()
    }

    pub fn encryption(&self) -> Option<Arc<EncryptionService>> {
        self.encryption.clone()
    }

    pub fn oauth_provider_id(server_id: Uuid) -> String {
        mcp_oauth_provider_id_for_uuid(server_id)
    }

    pub fn settings_from_row(row: &McpServerRow) -> McpServerSettings {
        let mut settings =
            serde_json::from_value(row.settings.clone()).unwrap_or(McpServerSettings {
                auth_mode: McpServerAuthMode::None,
                protocol_mode: McpProtocolMode::Auto,
                oauth: None,
            });

        if row.settings.get("auth_mode").is_none() {
            settings.auth_mode = if settings.oauth.is_some() {
                McpServerAuthMode::OAuth
            } else if row.api_key_set {
                McpServerAuthMode::ApiKey
            } else {
                McpServerAuthMode::None
            };
        }

        settings
    }

    fn settings_to_value(settings: &McpServerSettings) -> serde_json::Value {
        serde_json::to_value(settings).unwrap_or_else(|_| serde_json::json!({}))
    }

    pub fn encrypt_string_to_b64(&self, value: &str) -> Result<String> {
        let encryption = self
            .encryption
            .as_ref()
            .ok_or_else(|| anyhow!("Encryption not configured"))?;
        Ok(BASE64_STANDARD.encode(encryption.encrypt_string(value)?))
    }

    pub fn decrypt_string_from_b64(&self, value: &str) -> Result<String> {
        let encryption = self
            .encryption
            .as_ref()
            .ok_or_else(|| anyhow!("Encryption not configured"))?;
        let bytes = BASE64_STANDARD
            .decode(value)
            .map_err(|e| anyhow!("Invalid encrypted value: {e}"))?;
        encryption.decrypt_to_string(&bytes)
    }

    pub async fn create(&self, caller: &Caller, req: CreateMcpServerRequest) -> Result<McpServer> {
        if !everruns_core::mcp_server::is_valid_mcp_server_name(&req.name) {
            anyhow::bail!("MCP server name has an ambiguous tool prefix");
        }
        // Org-managed MCP servers are always remote; stdio is reserved for
        // single-tenant runtime/CLI hosts (knowledge/integrations/runtime-mcp.md D2).
        if req.transport_type.is_local() {
            anyhow::bail!("stdio MCP servers are not supported for organization MCP servers");
        }
        let auth_mode = req.auth_mode.clone().unwrap_or_else(|| {
            if req.api_key.is_some() {
                McpServerAuthMode::ApiKey
            } else {
                McpServerAuthMode::None
            }
        });
        if req.api_key.is_some() && auth_mode != McpServerAuthMode::ApiKey {
            anyhow::bail!("Only API key MCP servers can store an API key");
        }
        // Encrypt API key if provided
        let api_key_encrypted = if auth_mode == McpServerAuthMode::ApiKey {
            let api_key = req
                .api_key
                .as_ref()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow!("API key auth mode requires an API key"))?;
            let encryption = self
                .encryption
                .as_ref()
                .ok_or_else(|| anyhow!("Encryption not configured. Cannot store API key."))?;
            Some(encryption.encrypt_string(api_key)?)
        } else {
            None
        };

        let settings = McpServerSettings {
            auth_mode,
            protocol_mode: req.protocol_mode.unwrap_or_default(),
            oauth: None,
        };

        let input = CreateMcpServerRow {
            name: req.name,
            description: req.description,
            url: req.url,
            transport_type: req.transport_type.to_string(),
            api_key_encrypted,
            headers: req
                .headers
                .map(|h| serde_json::to_value(h).unwrap_or_default()),
            settings: Some(Self::settings_to_value(&settings)),
        };

        let row = self.db.create_mcp_server(caller.org_id, input).await?;
        Ok(Self::row_to_mcp_server(&row))
    }

    pub async fn get(&self, caller: &Caller, id: Uuid) -> Result<Option<McpServer>> {
        let row = self.db.get_mcp_server(caller.org_id, id).await?;
        Ok(row
            .as_ref()
            .filter(|row| row.status != "deleted")
            .map(Self::row_to_mcp_server))
    }

    /// Batch fetch multiple MCP servers with their cached tools in a single query.
    /// Returns a map of server_id -> (McpServer, `Vec<McpToolDefinition>`).
    pub async fn get_batch_with_tools(
        &self,
        caller: &Caller,
        ids: &[Uuid],
    ) -> Result<HashMap<Uuid, (McpServer, Vec<McpToolDefinition>)>> {
        let rows = self.db.get_mcp_servers_batch(caller.org_id, ids).await?;
        let mut servers = HashMap::with_capacity(rows.len());

        for row in rows {
            let server = Self::row_to_mcp_server(&row);
            let server_id = row.id.uuid();
            let tools = self.tools_for_row(caller.org_id, &row).await;
            servers.insert(server_id, (server, tools));
        }

        Ok(servers)
    }

    pub async fn list(
        &self,
        caller: &Caller,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<McpServer>> {
        let rows = self
            .db
            .list_mcp_servers(caller.org_id, search, include_archived)
            .await?;
        Ok(rows.iter().map(Self::row_to_mcp_server).collect())
    }

    pub async fn update(
        &self,
        caller: &Caller,
        id: Uuid,
        req: UpdateMcpServerRequest,
    ) -> Result<Option<McpServer>> {
        if req
            .name
            .as_deref()
            .is_some_and(|name| !everruns_core::mcp_server::is_valid_mcp_server_name(name))
        {
            anyhow::bail!("MCP server name has an ambiguous tool prefix");
        }
        if let Some(existing) = self.db.get_mcp_server(caller.org_id, id).await?
            && !matches!(existing.status.as_str(), "active" | "disabled")
        {
            anyhow::bail!("Archived or deleted MCP servers cannot be edited");
        }
        if req.transport_type.as_ref().is_some_and(|t| t.is_local()) {
            anyhow::bail!("stdio MCP servers are not supported for organization MCP servers");
        }
        let existing_row = self.db.get_mcp_server(caller.org_id, id).await?;
        let existing_row =
            existing_row.ok_or_else(|| crate::errors::ResourceNotFoundError::new("MCP server"))?;
        let mut settings = Self::settings_from_row(&existing_row);
        if settings.auth_mode == McpServerAuthMode::OAuth
            && (req
                .url
                .as_deref()
                .is_some_and(|url| url != existing_row.url)
                || req
                    .auth_mode
                    .as_ref()
                    .is_some_and(|mode| *mode != McpServerAuthMode::OAuth))
        {
            // The row id is the OAuth provider id. Retargeting it would make
            // existing refresh tokens flow to newly discovered metadata. Keep
            // the identity immutable, including against a mode-toggle bypass.
            anyhow::bail!(
                "OAuth MCP server authority cannot be changed; create a new server instead"
            );
        }
        if let Some(auth_mode) = req.auth_mode.clone() {
            settings.auth_mode = auth_mode;
            if settings.auth_mode != McpServerAuthMode::OAuth {
                settings.oauth = None;
            }
        }
        if let Some(protocol_mode) = req.protocol_mode {
            settings.protocol_mode = protocol_mode;
        }
        if req.api_key.is_some() && settings.auth_mode != McpServerAuthMode::ApiKey {
            anyhow::bail!("Only API key MCP servers can store an API key");
        }
        if settings.auth_mode == McpServerAuthMode::ApiKey
            && !existing_row.api_key_set
            && req
                .api_key
                .as_deref()
                .filter(|value| !value.is_empty())
                .is_none()
        {
            anyhow::bail!("API key auth mode requires an API key");
        }

        // Encrypt API key if provided
        let api_key_encrypted = if let Some(api_key) = &req.api_key {
            let encryption = self
                .encryption
                .as_ref()
                .ok_or_else(|| anyhow!("Encryption not configured. Cannot store API key."))?;
            if api_key.is_empty() {
                anyhow::bail!("API key auth mode requires a non-empty API key");
            }
            Some(encryption.encrypt_string(api_key)?)
        } else if req.auth_mode == Some(McpServerAuthMode::None)
            || req.auth_mode == Some(McpServerAuthMode::OAuth)
        {
            Some(Vec::new())
        } else {
            None
        };

        let input = UpdateMcpServer {
            name: req.name,
            description: req.description,
            url: req.url,
            transport_type: req.transport_type.map(|t| t.to_string()),
            status: req.status.map(|s| s.to_string()),
            api_key_encrypted,
            headers: req
                .headers
                .map(|h| serde_json::to_value(h).unwrap_or_default()),
            settings: Some(Self::settings_to_value(&settings)),
        };

        let row = self.db.update_mcp_server(caller.org_id, id, input).await?;
        Ok(row.as_ref().map(Self::row_to_mcp_server))
    }

    pub async fn delete(&self, caller: &Caller, id: Uuid) -> Result<bool> {
        self.db.delete_mcp_server(caller.org_id, id).await
    }

    pub async fn destroy(&self, caller: &Caller, id: Uuid) -> Result<bool> {
        let Some(existing) = self.db.get_mcp_server(caller.org_id, id).await? else {
            return Ok(false);
        };
        if existing.status != "archived" {
            anyhow::bail!("MCP server must be archived before deletion");
        }
        self.db.destroy_mcp_server(caller.org_id, id).await
    }

    /// List active MCP servers (for capability listing)
    pub async fn list_active(&self, caller: &Caller) -> Result<Vec<McpServer>> {
        let rows = self.db.list_active_mcp_servers(caller.org_id).await?;
        Ok(rows.iter().map(Self::row_to_mcp_server).collect())
    }

    /// List active MCP servers with their cached tools
    pub async fn list_active_with_tools(&self, caller: &Caller) -> Result<Vec<McpServerWithTools>> {
        let rows = self.db.list_active_mcp_servers(caller.org_id).await?;
        let mut servers = Vec::with_capacity(rows.len());
        for row in &rows {
            self.clear_legacy_oauth_tools(caller.org_id, row).await?;
            servers.push(Self::row_to_mcp_server_with_tools(row));
        }
        Ok(servers)
    }

    /// Refresh cached tools for an MCP server by calling tools/list
    pub async fn refresh_tools(&self, caller: &Caller, id: Uuid) -> Result<Vec<McpToolDefinition>> {
        // Get the MCP server
        let row = self
            .db
            .get_mcp_server(caller.org_id, id)
            .await?
            .ok_or_else(|| crate::errors::ResourceNotFoundError::new("MCP server"))?;
        let settings = Self::settings_from_row(&row);

        // Get decrypted API key if set
        let api_key = if settings.auth_mode == McpServerAuthMode::ApiKey && row.api_key_set {
            if let Some(encrypted) = &row.api_key_encrypted {
                let encryption = self
                    .encryption
                    .as_ref()
                    .ok_or_else(|| anyhow!("Encryption not configured"))?;
                Some(encryption.decrypt_to_string(encrypted)?)
            } else {
                None
            }
        } else {
            None
        };

        // Parse headers
        let headers: HashMap<String, String> =
            serde_json::from_value(row.headers.clone()).unwrap_or_default();

        if settings.auth_mode == McpServerAuthMode::OAuth {
            self.clear_legacy_oauth_tools(caller.org_id, &row).await?;
            anyhow::bail!(
                "OAuth MCP servers require an identity-scoped attachment and connection grant"
            );
        }

        // Fetch tools from MCP server
        let tools = fetch_mcp_tools(
            self.egress_service.as_ref(),
            &row.url,
            api_key.as_deref(),
            &headers,
        )
        .await?;

        // Cache tools in database
        let cached_tools = serde_json::to_value(&tools)?;
        self.db
            .update_mcp_server_tools(caller.org_id, id, UpdateMcpServerTools { cached_tools })
            .await?;

        Ok(tools)
    }

    pub async fn cache_tools_for_bearer_token(
        &self,
        caller: &Caller,
        id: Uuid,
        token: &str,
    ) -> Result<Vec<McpToolDefinition>> {
        let row = self
            .db
            .get_mcp_server(caller.org_id, id)
            .await?
            .ok_or_else(|| crate::errors::ResourceNotFoundError::new("MCP server"))?;
        let headers: HashMap<String, String> =
            serde_json::from_value(row.headers.clone()).unwrap_or_default();
        let tools = fetch_mcp_tools(
            self.egress_service.as_ref(),
            &row.url,
            Some(token),
            &headers,
        )
        .await?;
        Ok(tools)
    }

    /// Age of a cached tool list, if one exists. A small future timestamp
    /// (within `CACHE_FUTURE_SKEW`) is tolerated and treated as age zero so
    /// minor clock skew does not force needless refreshes. A timestamp further
    /// in the future is treated as invalid (`None`) so a corrupt/skewed
    /// "future" `cached_at` can't make a cache look perpetually fresh and bypass
    /// the bounded-stale window.
    fn cache_age(row: &McpServerRow) -> Option<chrono::Duration> {
        const CACHE_FUTURE_SKEW_SECS: i64 = 300;
        let cached_at = row.tools_cached_at?;
        let age = Utc::now().signed_duration_since(cached_at);
        if age < chrono::Duration::seconds(-CACHE_FUTURE_SKEW_SECS) {
            return None;
        }
        Some(age.max(chrono::Duration::zero()))
    }

    /// Whether a server row's cached tools are still within the freshness TTL.
    fn cache_fresh(row: &McpServerRow) -> bool {
        Self::cache_age(row).is_some_and(|age| {
            age < chrono::Duration::from_std(TOOL_CACHE_TTL)
                .unwrap_or_else(|_| chrono::Duration::hours(1))
        })
    }

    /// Whether a stale cache is still young enough to serve while revalidating.
    fn cache_within_max_stale(row: &McpServerRow) -> bool {
        Self::cache_age(row).is_some_and(|age| {
            age < chrono::Duration::from_std(TOOL_CACHE_MAX_STALE)
                .unwrap_or_else(|_| chrono::Duration::hours(24))
        })
    }

    fn cached_tools(row: &McpServerRow) -> Vec<McpToolDefinition> {
        if Self::settings_from_row(row).auth_mode == McpServerAuthMode::OAuth {
            return Vec::new();
        }
        serde_json::from_value(row.cached_tools.clone()).unwrap_or_default()
    }

    async fn clear_legacy_oauth_tools(&self, org_id: i64, row: &McpServerRow) -> Result<()> {
        if Self::settings_from_row(row).auth_mode == McpServerAuthMode::OAuth
            && (row.cached_tools != serde_json::json!([]) || row.tools_cached_at.is_some())
        {
            self.db
                .clear_mcp_server_tools(org_id, row.id.uuid())
                .await?;
        }
        Ok(())
    }

    /// Get cached tools for an MCP server, refreshing if stale.
    ///
    /// When the cache is stale but a previous successful fetch exists, the
    /// cached tools are returned immediately and a refresh is kicked off in the
    /// background (stale-while-revalidate), so agent resolution never blocks on
    /// an upstream `tools/list`. A cold cache (or `force_refresh`) blocks on a
    /// single-flight refresh shared across concurrent callers. OAuth servers
    /// are rejected here because their tool lists use the identity-scoped
    /// attachment cache.
    pub async fn get_tools(
        &self,
        caller: &Caller,
        id: Uuid,
        force_refresh: bool,
    ) -> Result<Vec<McpToolDefinition>> {
        if force_refresh {
            return self.refresh_tools_coalesced(caller, id, false).await;
        }

        let row = self
            .db
            .get_mcp_server(caller.org_id, id)
            .await?
            .ok_or_else(|| crate::errors::ResourceNotFoundError::new("MCP server"))?;
        if Self::settings_from_row(&row).auth_mode == McpServerAuthMode::OAuth {
            anyhow::bail!(
                "OAuth MCP tools require an identity-scoped attachment and connection grant"
            );
        }

        if Self::cache_fresh(&row) {
            return Ok(Self::cached_tools(&row));
        }

        if self.can_revalidate_in_background(&row) {
            self.spawn_background_refresh(caller.org_id, id);
            return Ok(Self::cached_tools(&row));
        }

        // Cold cache: block on a single-flight refresh.
        self.refresh_tools_coalesced(caller, id, true).await
    }

    /// Resolve tools for an already-loaded row, applying stale-while-revalidate
    /// and single-flight refresh. Never errors: a hard refresh failure degrades
    /// to cached tools only while the cache is inside the maximum stale window,
    /// matching the batch-load contract without registering indefinitely stale
    /// tool definitions.
    async fn tools_for_row(&self, org_id: i64, row: &McpServerRow) -> Vec<McpToolDefinition> {
        if Self::settings_from_row(row).auth_mode == McpServerAuthMode::OAuth {
            return Vec::new();
        }
        if Self::cache_fresh(row) {
            return Self::cached_tools(row);
        }

        if self.can_revalidate_in_background(row) {
            self.spawn_background_refresh(org_id, row.id.uuid());
            return Self::cached_tools(row);
        }

        match self
            .refresh_tools_coalesced(&Caller::internal(org_id), row.id.uuid(), true)
            .await
        {
            Ok(tools) => tools,
            Err(err) => {
                if Self::cache_within_max_stale(row) {
                    tracing::warn!(
                        server_id = %row.id.uuid(),
                        error = %err,
                        "Failed to refresh MCP tool cache; serving cached tools within max stale window"
                    );
                    Self::cached_tools(row)
                } else {
                    tracing::warn!(
                        server_id = %row.id.uuid(),
                        error = %err,
                        "Failed to refresh expired MCP tool cache; omitting stale tools"
                    );
                    Vec::new()
                }
            }
        }
    }

    /// Stale-while-revalidate is only safe when a real upstream refresh can
    /// succeed: there must be a recent prior successful fetch to serve, and the
    /// server must be self-refreshable.
    fn can_revalidate_in_background(&self, row: &McpServerRow) -> bool {
        Self::cache_within_max_stale(row)
            && Self::settings_from_row(row).auth_mode != McpServerAuthMode::OAuth
    }

    /// Refresh tools under the per-server single-flight lock. Concurrent callers
    /// serialize; when `allow_cached` is set, a caller that finds the cache made
    /// fresh while it waited returns that result instead of re-fetching. Forced
    /// refreshes pass `allow_cached = false` so they always delegate to
    /// `refresh_tools` rather than returning a still-fresh cache hit.
    async fn refresh_tools_coalesced(
        &self,
        caller: &Caller,
        id: Uuid,
        allow_cached: bool,
    ) -> Result<Vec<McpToolDefinition>> {
        let lock = REFRESH_LOCKS.lock_for((caller.org_id, id));
        let _guard = lock.lock_owned().await;

        if allow_cached
            && let Some(row) = self.db.get_mcp_server(caller.org_id, id).await?
            && Self::cache_fresh(&row)
        {
            return Ok(Self::cached_tools(&row));
        }

        self.refresh_tools(caller, id).await
    }

    /// Trigger a background refresh for a server, deduplicated so at most one
    /// runs per server at a time. Failures are logged, not surfaced: the caller
    /// has already been served a cache still inside the maximum stale window.
    fn spawn_background_refresh(&self, org_id: i64, id: Uuid) {
        let lock = REFRESH_LOCKS.lock_for((org_id, id));
        let Ok(guard) = lock.try_lock_owned() else {
            // A refresh is already in flight for this server; nothing to do.
            return;
        };

        let service = self.clone();
        tokio::spawn(async move {
            let _guard = guard; // held for the refresh duration -> single-flight
            if let Err(err) = service.refresh_tools(&Caller::internal(org_id), id).await {
                tracing::warn!(
                    server_id = %id,
                    error = %err,
                    "Background MCP tool refresh failed; cache remains stale until max stale age"
                );
            }
        });
    }

    /// Get cached tools for an MCP server without refreshing (for preview)
    /// Returns empty vec if server not found or no cached tools
    pub async fn get_cached_tools(
        &self,
        caller: &Caller,
        id: Uuid,
    ) -> Result<Vec<McpToolDefinition>> {
        match self.db.get_mcp_server(caller.org_id, id).await {
            Ok(Some(row)) => {
                self.clear_legacy_oauth_tools(caller.org_id, &row).await?;
                Ok(Self::cached_tools(&row))
            }
            _ => Ok(Vec::new()),
        }
    }

    /// Decrypt API key for an MCP server by ID.
    /// Returns None if server has no API key set.
    pub async fn decrypt_api_key(&self, caller: &Caller, id: Uuid) -> Result<Option<String>> {
        let row = self
            .db
            .get_mcp_server(caller.org_id, id)
            .await?
            .ok_or_else(|| crate::errors::ResourceNotFoundError::new("MCP server"))?;

        if !row.api_key_set {
            return Ok(None);
        }

        match &row.api_key_encrypted {
            Some(encrypted) => {
                let encryption = self
                    .encryption
                    .as_ref()
                    .ok_or_else(|| anyhow!("Encryption not configured"))?;
                Ok(Some(encryption.decrypt_to_string(encrypted)?))
            }
            None => Ok(None),
        }
    }

    /// Resolve an MCP server by sanitized name prefix, decrypting credentials.
    ///
    /// Used by both gRPC service and direct worker adapters to look up an MCP
    /// server by its sanitized name (lowercase, non-alphanumeric chars -> '_').
    pub async fn resolve_by_prefix(
        &self,
        caller: &Caller,
        server_prefix: &str,
    ) -> Result<Option<McpServerResolved>> {
        let servers = self.list(caller, None, false).await?;
        let server_prefix_lower = server_prefix.to_lowercase();

        let server = servers.into_iter().find(|s| {
            everruns_core::mcp_server::is_valid_mcp_server_name(&s.name)
                && everruns_core::sanitize_mcp_server_name(&s.name) == server_prefix_lower
                && s.status == McpServerStatus::Active
        });

        let server = match server {
            Some(s) => s,
            None => return Ok(None),
        };

        let api_key = if server.auth_mode == McpServerAuthMode::ApiKey && server.api_key_set {
            self.decrypt_api_key(caller, server.id.uuid()).await?
        } else {
            None
        };

        // Fetch raw headers from DB — server.headers has values redacted for API safety.
        let raw_row = self
            .db
            .get_mcp_server(caller.org_id, server.id.uuid())
            .await?;
        let Some(raw_row) = raw_row else {
            // Server removed between list and resolve — treat as not found.
            return Ok(None);
        };
        let headers =
            serde_json::from_value::<HashMap<String, String>>(raw_row.headers).unwrap_or_default();

        Ok(Some(McpServerResolved {
            id: server.id.uuid(),
            name: server.name,
            url: server.url,
            auth_mode: server.auth_mode,
            protocol_mode: server.protocol_mode,
            oauth_provider_id: server.oauth_provider_id,
            acts_as: McpServerActsAs::None,
            api_key,
            headers,
        }))
    }

    /// Resolve transport metadata for an active organization MCP server by name.
    pub async fn resolve_transport_by_name(
        &self,
        caller: &Caller,
        name: &str,
    ) -> Result<Option<McpServerResolved>> {
        let Some(row) = self
            .db
            .get_mcp_server_by_name(caller.org_id, name)
            .await?
            .filter(|row| row.status == "active")
        else {
            return Ok(None);
        };
        let settings = Self::settings_from_row(&row);
        let headers =
            serde_json::from_value::<HashMap<String, String>>(row.headers).unwrap_or_default();

        Ok(Some(McpServerResolved {
            id: row.id.uuid(),
            name: row.name,
            url: row.url,
            auth_mode: McpServerAuthMode::None,
            protocol_mode: settings.protocol_mode,
            oauth_provider_id: None,
            acts_as: McpServerActsAs::None,
            api_key: None,
            headers,
        }))
    }

    fn row_to_mcp_server(row: &McpServerRow) -> McpServer {
        super::queries::row_to_mcp_server(row)
    }

    fn row_to_mcp_server_with_tools(row: &McpServerRow) -> McpServerWithTools {
        let server = Self::row_to_mcp_server(row);
        let cached_tools = Self::cached_tools(row);

        McpServerWithTools {
            server,
            cached_tools,
            tools_cached_at: row.tools_cached_at,
        }
    }
}

/// MCP Server with cached tools
#[derive(Debug, Clone)]
pub struct McpServerWithTools {
    pub server: McpServer,
    pub cached_tools: Vec<McpToolDefinition>,
    pub tools_cached_at: Option<DateTime<Utc>>,
}

/// Resolved MCP server descriptor for worker transport.
///
/// Organization-level resolution can include credentials. Scoped catalog
/// resolution includes transport metadata only.
#[derive(Debug, Clone)]
pub struct McpServerResolved {
    pub id: Uuid,
    pub name: String,
    pub url: String,
    pub auth_mode: McpServerAuthMode,
    /// Protocol-era adoption policy (`auto` negotiates every protocol era).
    pub protocol_mode: McpProtocolMode,
    pub oauth_provider_id: Option<String>,
    pub acts_as: McpServerActsAs,
    pub api_key: Option<String>,
    pub headers: HashMap<String, String>,
}

/// Fetch tools from an MCP server using JSON-RPC over HTTP. Delegates to the
/// shared `everruns-mcp` client so SSRF validation and SSE/JSON handling are
/// not duplicated (knowledge/integrations/runtime-mcp.md D5).
pub(crate) async fn fetch_mcp_tools(
    egress_service: &dyn EgressService,
    url: &str,
    api_key: Option<&str>,
    headers: &HashMap<String, String>,
) -> Result<Vec<McpToolDefinition>> {
    let credential = api_key.map(everruns_mcp::McpCredential::bearer);
    everruns_mcp::http_list_tools(egress_service, url, headers, credential.as_ref()).await
}

pub(crate) async fn fetch_mcp_tools_with_cache_hints(
    egress_service: &dyn EgressService,
    url: &str,
    api_key: Option<&str>,
    headers: &HashMap<String, String>,
) -> Result<everruns_mcp::HttpToolsList> {
    let credential = api_key.map(everruns_mcp::McpCredential::bearer);
    everruns_mcp::http_list_tools_with_cache_hints(
        egress_service,
        url,
        headers,
        credential.as_ref(),
    )
    .await
}

#[cfg(test)]
#[path = "service/tests.rs"]
mod tests;
