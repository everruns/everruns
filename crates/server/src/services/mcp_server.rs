// MCP Server service for business logic
// Handles MCP server CRUD and tool discovery/caching

use crate::storage::{
    EncryptionService, McpServerRow, StorageBackend,
    models::{CreateMcpServerRow, UpdateMcpServer, UpdateMcpServerTools},
};
use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use everruns_core::{
    McpServer, McpServerStatus, McpServerTransportType, McpToolDefinition, McpToolsListRequest,
    McpToolsListResponse,
};
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

pub struct McpServerService {
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
}

impl McpServerService {
    pub fn new(db: Arc<StorageBackend>, encryption: Option<Arc<EncryptionService>>) -> Self {
        Self { db, encryption }
    }

    pub async fn create(&self, org_id: i64, req: CreateMcpServerRequest) -> Result<McpServer> {
        // Encrypt API key if provided
        let api_key_encrypted = if let Some(api_key) = &req.api_key {
            let encryption = self
                .encryption
                .as_ref()
                .ok_or_else(|| anyhow!("Encryption not configured. Cannot store API key."))?;
            Some(encryption.encrypt_string(api_key)?)
        } else {
            None
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
            settings: None,
        };

        let row = self.db.create_mcp_server(org_id, input).await?;
        Ok(Self::row_to_mcp_server(&row))
    }

    pub async fn get(&self, org_id: i64, id: Uuid) -> Result<Option<McpServer>> {
        let row = self.db.get_mcp_server(org_id, id).await?;
        Ok(row.as_ref().map(Self::row_to_mcp_server))
    }

    /// Batch fetch multiple MCP servers with their cached tools in a single query.
    /// Returns a map of server_id -> (McpServer, `Vec<McpToolDefinition>`).
    pub async fn get_batch_with_tools(
        &self,
        org_id: i64,
        ids: &[Uuid],
    ) -> Result<HashMap<Uuid, (McpServer, Vec<McpToolDefinition>)>> {
        let rows = self.db.get_mcp_servers_batch(org_id, ids).await?;
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

    pub async fn list(&self, org_id: i64) -> Result<Vec<McpServer>> {
        let rows = self.db.list_mcp_servers(org_id).await?;
        Ok(rows.iter().map(Self::row_to_mcp_server).collect())
    }

    pub async fn update(
        &self,
        org_id: i64,
        id: Uuid,
        req: UpdateMcpServerRequest,
    ) -> Result<Option<McpServer>> {
        // Encrypt API key if provided
        let api_key_encrypted = if let Some(api_key) = &req.api_key {
            let encryption = self
                .encryption
                .as_ref()
                .ok_or_else(|| anyhow!("Encryption not configured. Cannot store API key."))?;
            Some(encryption.encrypt_string(api_key)?)
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
            settings: None,
        };

        let row = self.db.update_mcp_server(org_id, id, input).await?;
        Ok(row.as_ref().map(Self::row_to_mcp_server))
    }

    pub async fn delete(&self, org_id: i64, id: Uuid) -> Result<bool> {
        self.db.delete_mcp_server(org_id, id).await
    }

    /// List active MCP servers (for capability listing)
    pub async fn list_active(&self, org_id: i64) -> Result<Vec<McpServer>> {
        let rows = self.db.list_active_mcp_servers(org_id).await?;
        Ok(rows.iter().map(Self::row_to_mcp_server).collect())
    }

    /// List active MCP servers with their cached tools
    pub async fn list_active_with_tools(&self, org_id: i64) -> Result<Vec<McpServerWithTools>> {
        let rows = self.db.list_active_mcp_servers(org_id).await?;
        Ok(rows
            .iter()
            .map(Self::row_to_mcp_server_with_tools)
            .collect())
    }

    /// Refresh cached tools for an MCP server by calling tools/list
    pub async fn refresh_tools(&self, org_id: i64, id: Uuid) -> Result<Vec<McpToolDefinition>> {
        // Get the MCP server
        let row = self
            .db
            .get_mcp_server(org_id, id)
            .await?
            .ok_or_else(|| anyhow!("MCP server not found"))?;

        // Get decrypted API key if set
        let api_key = if row.api_key_set {
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

        // Fetch tools from MCP server
        let tools = fetch_mcp_tools(&row.url, api_key.as_deref(), &headers).await?;

        // Cache tools in database
        let cached_tools = serde_json::to_value(&tools)?;
        self.db
            .update_mcp_server_tools(org_id, id, UpdateMcpServerTools { cached_tools })
            .await?;

        Ok(tools)
    }

    /// Get cached tools for an MCP server, refreshing if stale
    pub async fn get_tools(
        &self,
        org_id: i64,
        id: Uuid,
        force_refresh: bool,
    ) -> Result<Vec<McpToolDefinition>> {
        let row = self
            .db
            .get_mcp_server(org_id, id)
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
        self.refresh_tools(org_id, id).await
    }

    /// Get cached tools for an MCP server without refreshing (for preview)
    /// Returns empty vec if server not found or no cached tools
    pub async fn get_cached_tools(&self, org_id: i64, id: Uuid) -> Vec<McpToolDefinition> {
        match self.db.get_mcp_server(org_id, id).await {
            Ok(Some(row)) => serde_json::from_value(row.cached_tools.clone()).unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// Decrypt API key for an MCP server by ID.
    /// Returns None if server has no API key set.
    pub async fn decrypt_api_key(&self, org_id: i64, id: Uuid) -> Result<Option<String>> {
        let row = self
            .db
            .get_mcp_server(org_id, id)
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
    pub async fn resolve_by_prefix(
        &self,
        org_id: i64,
        server_prefix: &str,
    ) -> Result<Option<McpServerResolved>> {
        let servers = self.list(org_id).await?;
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

        let api_key = if server.api_key_set {
            self.decrypt_api_key(org_id, server.id.uuid()).await?
        } else {
            None
        };

        Ok(Some(McpServerResolved {
            id: server.id.uuid(),
            name: server.name,
            url: server.url,
            api_key,
            headers: server.headers,
        }))
    }

    fn row_to_mcp_server(row: &McpServerRow) -> McpServer {
        // Parse headers from JSON
        let headers: HashMap<String, String> =
            serde_json::from_value(row.headers.clone()).unwrap_or_default();

        McpServer {
            id: row.id,
            name: row.name.clone(),
            description: row.description.clone(),
            url: row.url.clone(),
            transport_type: McpServerTransportType::from(row.transport_type.as_str()),
            status: McpServerStatus::from(row.status.as_str()),
            api_key_set: row.api_key_set,
            headers,
            created_at: row.created_at,
            updated_at: row.updated_at,
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

        let result = svc.decrypt_api_key(1, row.id.uuid()).await.unwrap();
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

        let result = svc.decrypt_api_key(1, row.id.uuid()).await.unwrap();
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
        let result = svc.decrypt_api_key(1, row.id.uuid()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn decrypt_api_key_errors_for_missing_server() {
        let db = Arc::new(StorageBackend::in_memory());
        let svc = McpServerService::new(db, Some(test_encryption()));

        let result = svc.decrypt_api_key(1, Uuid::new_v4()).await;
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

        let resolved = svc.resolve_by_prefix(1, "my_cool_server").await.unwrap();
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

        let resolved = svc.resolve_by_prefix(1, "nonexistent").await.unwrap();
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

        let resolved = svc.resolve_by_prefix(1, "auth_server").await.unwrap();
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
        let result = svc.resolve_by_prefix(1, "no_enc").await;
        assert!(result.is_err());
    }
}
