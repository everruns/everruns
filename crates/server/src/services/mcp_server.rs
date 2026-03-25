// MCP Server service for business logic
// Handles MCP server CRUD and tool discovery/caching
//
// Spec: specs/mcp.md (umbrella), specs/mcp-servers.md (detail)

use crate::storage::{
    EncryptionService, McpServerRow, StorageBackend,
    models::{CreateMcpServerRow, UpdateMcpServer, UpdateMcpServerTools},
};
use anyhow::{Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use chrono::{DateTime, Utc};
use everruns_core::{
    Caller, McpServer, McpServerAuthMode, McpServerStatus, McpServerTransportType,
    McpToolDefinition, McpToolsListRequest, McpToolsListResponse, Permission, Policy, Rule,
    mcp_oauth_provider_id_for_uuid,
};
use everruns_macros::policy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use everruns_core::validate_safe_url;

use crate::api::mcp_servers::{CreateMcpServerRequest, UpdateMcpServerRequest};

/// How long cached tools are considered fresh (1 hour)
const TOOL_CACHE_TTL: Duration = Duration::from_secs(3600);

/// HTTP client timeout for MCP server calls
const MCP_CLIENT_TIMEOUT: Duration = Duration::from_secs(30);

pub const MCP_SERVER_VIEW: Policy = Policy {
    id: "mcp_server.view",
    rules: &[Rule::UserHasPermission(Permission::OrgAgentsManage)],
};
pub const MCP_SERVER_MANAGE: Policy = Policy {
    id: "mcp_server.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgAgentsManage)],
};
pub const MCP_SERVER_DANGEROUS: Policy = Policy {
    id: "mcp_server.dangerous",
    rules: &[
        Rule::UserHasPermission(Permission::OrgAgentsManage),
        Rule::UserHasPermission(Permission::OrgMcpServersDangerous),
    ],
};

pub struct McpServerService {
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpServerSettings {
    #[serde(default)]
    pub auth_mode: McpServerAuthMode,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret_encrypted: Option<String>,
}

impl McpServerService {
    pub fn new(db: Arc<StorageBackend>, encryption: Option<Arc<EncryptionService>>) -> Self {
        Self { db, encryption }
    }

    pub fn oauth_provider_id(server_id: Uuid) -> String {
        mcp_oauth_provider_id_for_uuid(server_id)
    }

    pub fn settings_from_row(row: &McpServerRow) -> McpServerSettings {
        let mut settings =
            serde_json::from_value(row.settings.clone()).unwrap_or(McpServerSettings {
                auth_mode: McpServerAuthMode::None,
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

    #[policy(MCP_SERVER_MANAGE)]
    pub async fn create(&self, caller: &Caller, req: CreateMcpServerRequest) -> Result<McpServer> {
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

    #[policy(MCP_SERVER_VIEW)]
    pub async fn get(&self, caller: &Caller, id: Uuid) -> Result<Option<McpServer>> {
        let row = self.db.get_mcp_server(caller.org_id, id).await?;
        Ok(row
            .as_ref()
            .filter(|row| row.status != "deleted")
            .map(Self::row_to_mcp_server))
    }

    /// Batch fetch multiple MCP servers with their cached tools in a single query.
    /// Returns a map of server_id -> (McpServer, `Vec<McpToolDefinition>`).
    #[policy(MCP_SERVER_VIEW)]
    pub async fn get_batch_with_tools(
        &self,
        caller: &Caller,
        ids: &[Uuid],
    ) -> Result<HashMap<Uuid, (McpServer, Vec<McpToolDefinition>)>> {
        let rows = self.db.get_mcp_servers_batch(caller.org_id, ids).await?;
        Ok(rows
            .iter()
            .map(|row| {
                let server = Self::row_to_mcp_server(row);
                let tools: Vec<McpToolDefinition> =
                    serde_json::from_value(row.cached_tools.clone()).unwrap_or_default();
                (row.id.uuid(), (server, tools))
            })
            .collect())
    }

    #[policy(MCP_SERVER_VIEW)]
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

    #[policy(MCP_SERVER_MANAGE)]
    pub async fn update(
        &self,
        caller: &Caller,
        id: Uuid,
        req: UpdateMcpServerRequest,
    ) -> Result<Option<McpServer>> {
        if let Some(existing) = self.db.get_mcp_server(caller.org_id, id).await?
            && !matches!(existing.status.as_str(), "active" | "disabled")
        {
            anyhow::bail!("Archived or deleted MCP servers cannot be edited");
        }
        let existing_row = self.db.get_mcp_server(caller.org_id, id).await?;
        let existing_row = existing_row.ok_or_else(|| anyhow!("MCP server not found"))?;
        let mut settings = Self::settings_from_row(&existing_row);
        if let Some(auth_mode) = req.auth_mode.clone() {
            settings.auth_mode = auth_mode;
            if settings.auth_mode != McpServerAuthMode::OAuth {
                settings.oauth = None;
            }
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

    #[policy(MCP_SERVER_MANAGE)]
    pub async fn delete(&self, caller: &Caller, id: Uuid) -> Result<bool> {
        self.db.delete_mcp_server(caller.org_id, id).await
    }

    #[policy(MCP_SERVER_DANGEROUS)]
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
    #[policy(MCP_SERVER_VIEW)]
    pub async fn list_active(&self, caller: &Caller) -> Result<Vec<McpServer>> {
        let rows = self.db.list_active_mcp_servers(caller.org_id).await?;
        Ok(rows.iter().map(Self::row_to_mcp_server).collect())
    }

    /// List active MCP servers with their cached tools
    #[policy(MCP_SERVER_VIEW)]
    pub async fn list_active_with_tools(&self, caller: &Caller) -> Result<Vec<McpServerWithTools>> {
        let rows = self.db.list_active_mcp_servers(caller.org_id).await?;
        Ok(rows
            .iter()
            .map(Self::row_to_mcp_server_with_tools)
            .collect())
    }

    /// Refresh cached tools for an MCP server by calling tools/list
    #[policy(MCP_SERVER_MANAGE)]
    pub async fn refresh_tools(&self, caller: &Caller, id: Uuid) -> Result<Vec<McpToolDefinition>> {
        // Get the MCP server
        let row = self
            .db
            .get_mcp_server(caller.org_id, id)
            .await?
            .ok_or_else(|| anyhow!("MCP server not found"))?;
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
            let tools: Vec<McpToolDefinition> =
                serde_json::from_value(row.cached_tools.clone()).unwrap_or_default();
            if !tools.is_empty() {
                return Ok(tools);
            }
            anyhow::bail!(
                "OAuth MCP servers require a user connection before tools can be refreshed"
            );
        }

        // Fetch tools from MCP server
        let tools = fetch_mcp_tools(&row.url, api_key.as_deref(), &headers).await?;

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
            .ok_or_else(|| anyhow!("MCP server not found"))?;
        let headers: HashMap<String, String> =
            serde_json::from_value(row.headers.clone()).unwrap_or_default();
        let tools = fetch_mcp_tools(&row.url, Some(token), &headers).await?;
        let cached_tools = serde_json::to_value(&tools)?;
        self.db
            .update_mcp_server_tools(caller.org_id, id, UpdateMcpServerTools { cached_tools })
            .await?;
        Ok(tools)
    }

    /// Get cached tools for an MCP server, refreshing if stale
    #[policy(MCP_SERVER_VIEW)]
    pub async fn get_tools(
        &self,
        caller: &Caller,
        id: Uuid,
        force_refresh: bool,
    ) -> Result<Vec<McpToolDefinition>> {
        let row = self
            .db
            .get_mcp_server(caller.org_id, id)
            .await?
            .ok_or_else(|| anyhow!("MCP server not found"))?;

        // Check if cache is fresh
        let cache_fresh = if let Some(cached_at) = row.tools_cached_at {
            let age = Utc::now().signed_duration_since(cached_at);
            age < chrono::Duration::from_std(TOOL_CACHE_TTL).unwrap_or(chrono::Duration::hours(1))
        } else {
            false
        };

        if !force_refresh && cache_fresh {
            // Return cached tools
            let tools: Vec<McpToolDefinition> =
                serde_json::from_value(row.cached_tools.clone()).unwrap_or_default();
            return Ok(tools);
        }

        // Refresh tools
        self.refresh_tools(caller, id).await
    }

    /// Get cached tools for an MCP server without refreshing (for preview)
    /// Returns empty vec if server not found or no cached tools
    #[policy(MCP_SERVER_VIEW)]
    pub async fn get_cached_tools(
        &self,
        caller: &Caller,
        id: Uuid,
    ) -> Result<Vec<McpToolDefinition>> {
        match self.db.get_mcp_server(caller.org_id, id).await {
            Ok(Some(row)) => {
                Ok(serde_json::from_value(row.cached_tools.clone()).unwrap_or_default())
            }
            _ => Ok(Vec::new()),
        }
    }

    /// Decrypt API key for an MCP server by ID.
    /// Returns None if server has no API key set.
    #[policy(MCP_SERVER_MANAGE)]
    pub async fn decrypt_api_key(&self, caller: &Caller, id: Uuid) -> Result<Option<String>> {
        let row = self
            .db
            .get_mcp_server(caller.org_id, id)
            .await?
            .ok_or_else(|| anyhow!("MCP server not found"))?;

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
    #[policy(MCP_SERVER_VIEW)]
    pub async fn resolve_by_prefix(
        &self,
        caller: &Caller,
        server_prefix: &str,
    ) -> Result<Option<McpServerResolved>> {
        let servers = self.list(caller, None, false).await?;
        let server_prefix_lower = server_prefix.to_lowercase();

        let server = servers.into_iter().find(|s| {
            let sanitized = s
                .name
                .to_lowercase()
                .chars()
                .map(|c| if c.is_alphanumeric() { c } else { '_' })
                .collect::<String>();
            sanitized == server_prefix_lower
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

        Ok(Some(McpServerResolved {
            id: server.id.uuid(),
            name: server.name,
            url: server.url,
            auth_mode: server.auth_mode,
            oauth_provider_id: server.oauth_provider_id,
            api_key,
            headers: server.headers,
        }))
    }

    fn row_to_mcp_server(row: &McpServerRow) -> McpServer {
        // Parse headers from JSON
        let headers: HashMap<String, String> =
            serde_json::from_value(row.headers.clone()).unwrap_or_default();
        let settings = Self::settings_from_row(row);
        let oauth_provider_id = (settings.auth_mode == McpServerAuthMode::OAuth)
            .then(|| Self::oauth_provider_id(row.id.uuid()));

        McpServer {
            id: row.id,
            name: row.name.clone(),
            description: row.description.clone(),
            url: row.url.clone(),
            transport_type: McpServerTransportType::from(row.transport_type.as_str()),
            status: McpServerStatus::from(row.status.as_str()),
            auth_mode: settings.auth_mode,
            oauth_provider_id,
            api_key_set: row.api_key_set,
            headers,
            created_at: row.created_at,
            updated_at: row.updated_at,
            archived_at: row.archived_at,
            deleted_at: row.deleted_at,
        }
    }

    fn row_to_mcp_server_with_tools(row: &McpServerRow) -> McpServerWithTools {
        let server = Self::row_to_mcp_server(row);
        let cached_tools: Vec<McpToolDefinition> =
            serde_json::from_value(row.cached_tools.clone()).unwrap_or_default();

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

/// Resolved MCP server with decrypted credentials, ready for tool execution.
///
/// Produced by `McpServerService::resolve_by_prefix` and consumed by both
/// the gRPC service and direct worker adapters.
#[derive(Debug, Clone)]
pub struct McpServerResolved {
    pub id: Uuid,
    pub name: String,
    pub url: String,
    pub auth_mode: McpServerAuthMode,
    pub oauth_provider_id: Option<String>,
    pub api_key: Option<String>,
    pub headers: HashMap<String, String>,
}

/// Fetch tools from an MCP server using JSON-RPC over HTTP
async fn fetch_mcp_tools(
    url: &str,
    api_key: Option<&str>,
    headers: &HashMap<String, String>,
) -> Result<Vec<McpToolDefinition>> {
    // Re-validate URL at fetch time (SSRF defense-in-depth)
    validate_safe_url(url).map_err(|e| anyhow!("MCP server URL blocked: {}", e))?;

    let client = reqwest::Client::builder()
        .timeout(MCP_CLIENT_TIMEOUT)
        .build()?;

    let request = McpToolsListRequest::default();

    let mut req_builder = client.post(url).json(&request);

    // Add API key if provided
    if let Some(key) = api_key {
        req_builder = req_builder.header("Authorization", format!("Bearer {}", key));
    }

    // Add custom headers
    for (name, value) in headers {
        req_builder = req_builder.header(name, value);
    }

    let response = req_builder.send().await?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "MCP server returned error status: {}",
            response.status()
        ));
    }

    let mcp_response: McpToolsListResponse = response.json().await?;

    if let Some(error) = mcp_response.error {
        return Err(anyhow!(
            "MCP server error: {} ({})",
            error.message,
            error.code
        ));
    }

    let result = mcp_response
        .result
        .ok_or_else(|| anyhow!("MCP server returned empty result"))?;

    Ok(result.tools)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{EncryptionService, StorageBackend, models::CreateMcpServerRow};
    use everruns_core::OrgRole;

    fn test_caller(org_id: i64) -> Caller {
        Caller {
            org_id,
            org_public_id: format!("org_{org_id:032}"),
            user_id: None,
            role: OrgRole::Owner,
            is_platform_user: false,
        }
    }

    fn test_encryption() -> Arc<EncryptionService> {
        Arc::new(
            EncryptionService::new("kek-v1:8B3uCQ4Znx45hl5nB+PKVriRrj/KtEVM+wBZ2VGa9vY=", &[])
                .unwrap(),
        )
    }

    #[tokio::test]
    async fn decrypt_api_key_returns_none_when_no_key_set() {
        let db = Arc::new(StorageBackend::in_memory());
        let svc = McpServerService::new(db.clone(), Some(test_encryption()));

        let row = db
            .create_mcp_server(
                1,
                CreateMcpServerRow {
                    name: "test-server".into(),
                    description: None,
                    url: "https://example.com".into(),
                    transport_type: "streamable_http".into(),
                    api_key_encrypted: None,
                    headers: None,
                    settings: None,
                },
            )
            .await
            .unwrap();

        let result = svc
            .decrypt_api_key(&test_caller(1), row.id.uuid())
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn decrypt_api_key_returns_decrypted_key() {
        let encryption = test_encryption();
        let db = Arc::new(StorageBackend::in_memory());
        let svc = McpServerService::new(db.clone(), Some(encryption.clone()));

        let encrypted = encryption.encrypt_string("sk-secret-123").unwrap();

        let row = db
            .create_mcp_server(
                1,
                CreateMcpServerRow {
                    name: "authed-server".into(),
                    description: None,
                    url: "https://example.com".into(),
                    transport_type: "streamable_http".into(),
                    api_key_encrypted: Some(encrypted),
                    headers: None,
                    settings: None,
                },
            )
            .await
            .unwrap();

        let result = svc
            .decrypt_api_key(&test_caller(1), row.id.uuid())
            .await
            .unwrap();
        assert_eq!(result.as_deref(), Some("sk-secret-123"));
    }

    #[tokio::test]
    async fn decrypt_api_key_errors_without_encryption_service() {
        let encryption = test_encryption();
        let db = Arc::new(StorageBackend::in_memory());

        // Create server WITH encrypted key
        let encrypted = encryption.encrypt_string("sk-secret").unwrap();
        let row = db
            .create_mcp_server(
                1,
                CreateMcpServerRow {
                    name: "no-enc-server".into(),
                    description: None,
                    url: "https://example.com".into(),
                    transport_type: "streamable_http".into(),
                    api_key_encrypted: Some(encrypted),
                    headers: None,
                    settings: None,
                },
            )
            .await
            .unwrap();

        // Service WITHOUT encryption configured
        let svc = McpServerService::new(db, None);
        let result = svc.decrypt_api_key(&test_caller(1), row.id.uuid()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn decrypt_api_key_errors_for_missing_server() {
        let db = Arc::new(StorageBackend::in_memory());
        let svc = McpServerService::new(db, Some(test_encryption()));

        let result = svc.decrypt_api_key(&test_caller(1), Uuid::new_v4()).await;
        assert!(result.is_err());
    }

    // --- resolve_by_prefix tests ---

    #[tokio::test]
    async fn resolve_by_prefix_finds_matching_server() {
        let db = Arc::new(StorageBackend::in_memory());
        let svc = McpServerService::new(db.clone(), Some(test_encryption()));

        db.create_mcp_server(
            1,
            CreateMcpServerRow {
                name: "My Cool Server".into(),
                description: None,
                url: "https://example.com/mcp".into(),
                transport_type: "streamable_http".into(),
                api_key_encrypted: None,
                headers: None,
                settings: None,
            },
        )
        .await
        .unwrap();

        let resolved = svc
            .resolve_by_prefix(&test_caller(1), "my_cool_server")
            .await
            .unwrap();
        assert!(resolved.is_some());
        let r = resolved.unwrap();
        assert_eq!(r.name, "My Cool Server");
        assert_eq!(r.url, "https://example.com/mcp");
        assert!(r.api_key.is_none());
    }

    #[tokio::test]
    async fn resolve_by_prefix_returns_none_for_no_match() {
        let db = Arc::new(StorageBackend::in_memory());
        let svc = McpServerService::new(db, Some(test_encryption()));

        let resolved = svc
            .resolve_by_prefix(&test_caller(1), "nonexistent")
            .await
            .unwrap();
        assert!(resolved.is_none());
    }

    #[tokio::test]
    async fn resolve_by_prefix_decrypts_api_key() {
        let encryption = test_encryption();
        let db = Arc::new(StorageBackend::in_memory());
        let svc = McpServerService::new(db.clone(), Some(encryption.clone()));

        let encrypted = encryption.encrypt_string("sk-mcp-secret").unwrap();
        db.create_mcp_server(
            1,
            CreateMcpServerRow {
                name: "Auth Server".into(),
                description: None,
                url: "https://example.com/mcp".into(),
                transport_type: "streamable_http".into(),
                api_key_encrypted: Some(encrypted),
                headers: None,
                settings: None,
            },
        )
        .await
        .unwrap();

        let resolved = svc
            .resolve_by_prefix(&test_caller(1), "auth_server")
            .await
            .unwrap();
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().api_key.as_deref(), Some("sk-mcp-secret"));
    }

    // --- SSRF: fetch_mcp_tools blocks unsafe URLs ---

    #[tokio::test]
    async fn fetch_tools_blocks_localhost() {
        let result =
            super::fetch_mcp_tools("http://localhost:9999/mcp", None, &HashMap::new()).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("blocked"));
    }

    #[tokio::test]
    async fn fetch_tools_blocks_private_ip() {
        let result = super::fetch_mcp_tools("http://10.0.0.1/mcp", None, &HashMap::new()).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("blocked"));
    }

    #[tokio::test]
    async fn fetch_tools_blocks_metadata_endpoint() {
        let result = super::fetch_mcp_tools(
            "http://169.254.169.254/latest/meta-data/",
            None,
            &HashMap::new(),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("blocked"));
    }

    #[tokio::test]
    async fn resolve_by_prefix_errors_when_encryption_missing_for_key() {
        let encryption = test_encryption();
        let db = Arc::new(StorageBackend::in_memory());

        let encrypted = encryption.encrypt_string("sk-secret").unwrap();
        db.create_mcp_server(
            1,
            CreateMcpServerRow {
                name: "No Enc".into(),
                description: None,
                url: "https://example.com".into(),
                transport_type: "streamable_http".into(),
                api_key_encrypted: Some(encrypted),
                headers: None,
                settings: None,
            },
        )
        .await
        .unwrap();

        // Service without encryption
        let svc = McpServerService::new(db, None);
        let result = svc.resolve_by_prefix(&test_caller(1), "no_enc").await;
        assert!(result.is_err());
    }

    #[test]
    fn settings_from_row_defaults_auth_mode_for_legacy_api_key_servers() {
        let row = McpServerRow {
            id: everruns_core::typed_id::McpServerId::new(),
            org_id: 1,
            name: "legacy".into(),
            description: None,
            url: "https://example.com/mcp".into(),
            transport_type: "http".into(),
            status: "active".into(),
            api_key_encrypted: Some(vec![1, 2, 3]),
            api_key_set: true,
            headers: serde_json::json!({}),
            settings: serde_json::json!({}),
            cached_tools: serde_json::json!([]),
            tools_cached_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            archived_at: None,
            deleted_at: None,
        };

        let settings = McpServerService::settings_from_row(&row);
        assert_eq!(settings.auth_mode, McpServerAuthMode::ApiKey);
    }

    #[tokio::test]
    async fn create_rejects_api_key_when_auth_mode_is_not_api_key() {
        let db = Arc::new(StorageBackend::in_memory());
        let svc = McpServerService::new(db, Some(test_encryption()));

        let result = svc
            .create(
                &test_caller(1),
                CreateMcpServerRequest {
                    name: "bad-server".into(),
                    description: None,
                    url: "https://example.com/mcp".into(),
                    transport_type: McpServerTransportType::Http,
                    auth_mode: Some(McpServerAuthMode::None),
                    api_key: Some("secret".into()),
                    headers: None,
                },
            )
            .await;

        assert!(result.is_err());
    }
}
