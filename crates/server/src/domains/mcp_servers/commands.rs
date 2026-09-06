// MCP Server commands — user-facing operations.
//
// Each struct is the request type, catalog entry, and execution logic.
// inventory::submit! auto-registers for MCP catalog.

use super::McpServerSettings;
use super::queries as q;
use super::types::{
    CreateMcpServerRequest, CreateMcpServerRow, UpdateMcpServer, UpdateMcpServerRequest,
};
use super::{MCP_SERVER_DANGEROUS, MCP_SERVER_MANAGE, MCP_SERVER_VIEW};
use crate::domains::common::*;
use crate::kernel_imports::{
    McpServer, McpServerAuthMode, McpServerStatus, Policy,
    everruns_provider::url_validation::validate_safe_url,
};
use everruns_provider::typed_id::McpServerId;
use serde::Deserialize;
use utoipa::ToSchema;

// ============================================================================
// Input validation
// ============================================================================

/// Validate URL is non-empty and passes SSRF safety checks.
fn validate_url(url: &str) -> Result<(), CommandError> {
    if url.trim().is_empty() {
        return Err(CommandError::bad_request("URL cannot be empty"));
    }
    validate_safe_url(url)
        .map_err(|e| CommandError::bad_request(format!("Invalid MCP server URL: {e}")))?;
    Ok(())
}

/// Validate the name can be used as an unambiguous MCP tool prefix.
fn validate_mcp_name(name: &str) -> Result<(), CommandError> {
    if name.trim().is_empty() {
        return Err(CommandError::bad_request("Name cannot be empty"));
    }
    if !everruns_core::mcp_server::is_valid_mcp_server_name(name) {
        return Err(CommandError::bad_request(
            "MCP server name cannot contain consecutive underscores or end in an underscore after sanitization",
        ));
    }
    Ok(())
}

// ============================================================================
// Encryption helpers
// ============================================================================

fn encrypt_api_key(ctx: &Ctx, api_key: &str) -> Result<Vec<u8>, CommandError> {
    let encryption = ctx.encryption.as_ref().ok_or_else(|| {
        CommandError::internal(anyhow::anyhow!(
            "Encryption not configured. Cannot store API key."
        ))
    })?;
    encryption
        .encrypt_string(api_key)
        .map_err(CommandError::internal)
}

// ============================================================================
// CreateMcpServer
// ============================================================================

/// Create a new MCP server with a name, URL, and optional authentication.
#[derive(Debug, Deserialize)]
pub struct CreateMcpServer(pub CreateMcpServerRequest);

impl CommandSchema for CreateMcpServer {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<CreateMcpServerRequest>()
    }
}

impl Command for CreateMcpServer {
    type Output = McpServer;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_mcp_server",
            category: "mcp_servers",
            description: "Create a new MCP server with a name, URL, and optional authentication.",
            method: "POST",
            path: "/v1/mcp-servers",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&MCP_SERVER_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<McpServer, CommandError> {
        let req = self.0;

        // Validate
        validate_mcp_name(&req.name)?;
        validate_url(&req.url)?;

        // Determine auth mode
        let auth_mode = req.auth_mode.clone().unwrap_or_else(|| {
            if req.api_key.is_some() {
                McpServerAuthMode::ApiKey
            } else {
                McpServerAuthMode::None
            }
        });
        if req.api_key.is_some() && auth_mode != McpServerAuthMode::ApiKey {
            return Err(CommandError::bad_request(
                "Only API key MCP servers can store an API key",
            ));
        }

        // Encrypt API key if provided
        let api_key_encrypted = if auth_mode == McpServerAuthMode::ApiKey {
            let api_key = req
                .api_key
                .as_ref()
                .filter(|v| !v.is_empty())
                .ok_or_else(|| {
                    CommandError::bad_request("API key auth mode requires an API key")
                })?;
            Some(encrypt_api_key(ctx, api_key)?)
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
            settings: Some(
                serde_json::to_value(&settings).unwrap_or_else(|_| serde_json::json!({})),
            ),
        };

        let row = ctx
            .db
            .create_mcp_server(ctx.org_id(), input)
            .await
            .map_err(classify_anyhow)?;

        Ok(q::row_to_mcp_server(&row))
    }
}

inventory::submit! { CommandDescriptor::of::<CreateMcpServer>() }

// ============================================================================
// ListMcpServers
// ============================================================================

/// List MCP servers. Supports search and include_archived.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ListMcpServers {
    pub search: Option<String>,
    #[serde(default, deserialize_with = "deserialize_bool_lenient")]
    pub include_archived: bool,
}

impl Command for ListMcpServers {
    type Output = Vec<McpServer>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_mcp_servers",
            category: "mcp_servers",
            description: "List all active MCP servers. Use search for name/description search, include_archived=true to include archived.",
            method: "GET",
            path: "/v1/mcp-servers",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&MCP_SERVER_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<McpServer>, CommandError> {
        let rows = ctx
            .db
            .list_mcp_servers(ctx.org_id(), self.search.as_deref(), self.include_archived)
            .await
            .map_err(classify_anyhow)?;

        Ok(rows.iter().map(q::row_to_mcp_server).collect())
    }
}

inventory::submit! { CommandDescriptor::of::<ListMcpServers>() }

// ============================================================================
// GetMcpServer
// ============================================================================

/// Get a single MCP server by ID.
#[derive(Debug, Deserialize, ToSchema)]
pub struct GetMcpServer {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

impl Command for GetMcpServer {
    type Output = McpServer;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_mcp_server",
            category: "mcp_servers",
            description: "Get a single MCP server by ID.",
            method: "GET",
            path: "/v1/mcp-servers/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&MCP_SERVER_VIEW)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<McpServer, CommandError> {
        let server_id: McpServerId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid MCP server ID: {e}")))?;

        q::get_by_id(&ctx.db, ctx.org_id(), server_id.uuid())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("MCP server"))
    }
}

inventory::submit! { CommandDescriptor::of::<GetMcpServer>() }

// ============================================================================
// UpdateMcpServer
// ============================================================================

/// Update an MCP server. Only provided fields are changed.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateMcpServerCmd {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
    #[serde(flatten)]
    pub req: UpdateMcpServerRequest,
}

/// OAuth authority is immutable. The MCP server row id doubles as the OAuth
/// provider id, so changing an OAuth server's URL — or toggling it out of OAuth
/// mode — would let a stored refresh token be replayed against a newly
/// discovered token endpoint. Returns true when an update tries to retarget an
/// OAuth server's authority and must therefore be rejected.
fn oauth_authority_retargeted(
    existing_auth_mode: &McpServerAuthMode,
    existing_url: &str,
    req_url: Option<&str>,
    req_auth_mode: Option<&McpServerAuthMode>,
) -> bool {
    *existing_auth_mode == McpServerAuthMode::OAuth
        && (req_url.is_some_and(|url| url != existing_url)
            || req_auth_mode.is_some_and(|mode| *mode != McpServerAuthMode::OAuth))
}

impl Command for UpdateMcpServerCmd {
    type Output = McpServer;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_mcp_server",
            category: "mcp_servers",
            description: "Update an MCP server. Only provided fields are changed.",
            method: "PATCH",
            path: "/v1/mcp-servers/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&MCP_SERVER_MANAGE)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<McpServer, CommandError> {
        let server_id: McpServerId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid MCP server ID: {e}")))?;

        let req = self.req;
        if matches!(req.status, Some(McpServerStatus::Deleted)) {
            return Err(CommandError::forbidden(
                "Setting status=deleted requires dangerous delete permission".to_string(),
            ));
        }

        // Validate name if provided
        if let Some(ref name) = req.name {
            validate_mcp_name(name)?;
        }

        // Validate URL if provided (SSRF protection)
        if let Some(ref url) = req.url {
            validate_url(url)?;
        }

        // Resolve existing row
        let existing_row = q::get_row(&ctx.db, ctx.org_id(), server_id.uuid())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("MCP server"))?;

        if !matches!(existing_row.status.as_str(), "active" | "disabled") {
            return Err(CommandError::bad_request(
                "Archived or deleted MCP servers cannot be edited",
            ));
        }

        // Build settings
        let mut settings = q::settings_from_row(&existing_row);
        // OAuth authority is immutable: reject retargeting the server URL or
        // toggling it out of OAuth so a stored refresh token cannot flow to a
        // newly discovered token endpoint. (This is the live PATCH path.)
        if oauth_authority_retargeted(
            &settings.auth_mode,
            &existing_row.url,
            req.url.as_deref(),
            req.auth_mode.as_ref(),
        ) {
            return Err(CommandError::bad_request(
                "OAuth MCP server authority cannot be changed; create a new server instead",
            ));
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
            return Err(CommandError::bad_request(
                "Only API key MCP servers can store an API key",
            ));
        }
        if settings.auth_mode == McpServerAuthMode::ApiKey
            && !existing_row.api_key_set
            && req.api_key.as_deref().filter(|v| !v.is_empty()).is_none()
        {
            return Err(CommandError::bad_request(
                "API key auth mode requires an API key",
            ));
        }

        // Encrypt API key if provided
        let api_key_encrypted = if let Some(ref api_key) = req.api_key {
            if api_key.is_empty() {
                return Err(CommandError::bad_request(
                    "API key auth mode requires a non-empty API key",
                ));
            }
            Some(encrypt_api_key(ctx, api_key)?)
        } else if req.auth_mode == Some(McpServerAuthMode::None)
            || req.auth_mode == Some(McpServerAuthMode::OAuth)
        {
            // Clear API key when switching away from ApiKey auth mode
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
            settings: Some(
                serde_json::to_value(&settings).unwrap_or_else(|_| serde_json::json!({})),
            ),
        };

        let row = ctx
            .db
            .update_mcp_server(ctx.org_id(), server_id.uuid(), input)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("MCP server"))?;

        Ok(q::row_to_mcp_server(&row))
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateMcpServerCmd>() }

// ============================================================================
// DeleteMcpServer
// ============================================================================

/// Archive an MCP server (soft delete).
#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteMcpServer {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

impl Command for DeleteMcpServer {
    type Output = serde_json::Value;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_mcp_server",
            category: "mcp_servers",
            description: "Archive an MCP server (soft delete). Can be restored.",
            method: "DELETE",
            path: "/v1/mcp-servers/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&MCP_SERVER_MANAGE)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let server_id: McpServerId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid MCP server ID: {e}")))?;

        let deleted = ctx
            .db
            .delete_mcp_server(ctx.org_id(), server_id.uuid())
            .await
            .map_err(classify_anyhow)?;

        if deleted {
            Ok(serde_json::json!({"deleted": true}))
        } else {
            Err(CommandError::not_found("MCP server"))
        }
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteMcpServer>() }

// ============================================================================
// DestroyMcpServer (hard delete)
// ============================================================================

/// Permanently delete an archived MCP server.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DestroyMcpServer {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

impl Command for DestroyMcpServer {
    type Output = serde_json::Value;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "destroy_mcp_server",
            category: "mcp_servers",
            description: "Permanently delete an archived MCP server.",
            method: "POST",
            path: "/v1/mcp-servers/{id}/delete",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&MCP_SERVER_DANGEROUS)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let server_id: McpServerId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid MCP server ID: {e}")))?;

        let existing = q::get_row(&ctx.db, ctx.org_id(), server_id.uuid())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("MCP server"))?;

        if existing.status != "archived" {
            return Err(CommandError::bad_request(
                "MCP server must be archived before deletion",
            ));
        }

        let destroyed = ctx
            .db
            .destroy_mcp_server(ctx.org_id(), server_id.uuid())
            .await
            .map_err(classify_anyhow)?;

        if destroyed {
            Ok(serde_json::json!({"destroyed": true}))
        } else {
            Err(CommandError::not_found("MCP server"))
        }
    }
}

inventory::submit! { CommandDescriptor::of::<DestroyMcpServer>() }

#[cfg(test)]
mod oauth_authority_tests {
    use super::*;

    #[test]
    fn rejects_url_change_on_oauth_server() {
        assert!(oauth_authority_retargeted(
            &McpServerAuthMode::OAuth,
            "https://original.example/mcp",
            Some("https://attacker.example/mcp"),
            None,
        ));
    }

    #[test]
    fn rejects_toggling_oauth_server_out_of_oauth() {
        assert!(oauth_authority_retargeted(
            &McpServerAuthMode::OAuth,
            "https://original.example/mcp",
            None,
            Some(&McpServerAuthMode::ApiKey),
        ));
    }

    #[test]
    fn allows_unrelated_update_on_oauth_server() {
        // Same URL, no auth-mode change (e.g. name/description/header edit).
        assert!(!oauth_authority_retargeted(
            &McpServerAuthMode::OAuth,
            "https://original.example/mcp",
            Some("https://original.example/mcp"),
            None,
        ));
    }

    #[test]
    fn allows_url_change_on_non_oauth_server() {
        assert!(!oauth_authority_retargeted(
            &McpServerAuthMode::ApiKey,
            "https://original.example/mcp",
            Some("https://new.example/mcp"),
            None,
        ));
    }
    #[test]
    fn server_names_reject_ambiguous_sanitized_prefixes() {
        for name in [
            "",
            " ",
            "_",
            "docs_",
            "docs-",
            "docs__private",
            "docs..private",
        ] {
            assert!(validate_mcp_name(name).is_err(), "{name:?}");
        }
        for name in ["docs", "Docs API", "docs-api", "_docs"] {
            validate_mcp_name(name).unwrap();
        }
    }
}
