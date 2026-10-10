// User MCP servers: MCP servers a person adds for themselves.
//
// Spec: knowledge/integrations/user-mcp-servers.md (D1).
//
// A user server is an `mcp_servers` row owned by a virtual user. It is either
// added from the org catalog (it points at the preset and copies nothing, so
// the preset's OAuth client and the person's existing grant are reused) or
// custom (its own URL, and its own OAuth client registered on first connect).
// It always acts as its owner: there is no service login for it, and no other
// person's session can resolve it.
//
// Decision (D8): a personal sign-in to a catalog preset puts the preset on the
// person's list (`list_signed_in_catalog_server`), and removing a server from
// the list signs the person out of it. The list is the one place a person sees
// the MCP servers they connected.
//
// Decision: updates cannot change a server's URL. Stored credentials are bound
// to the origin they were issued for (EVE-1192); removing and re-adding the
// server is the honest way to point it somewhere else.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use everruns_contracts::url_validation::validate_safe_url;
use everruns_core::{McpServerAuthMode, mcp_oauth_provider_id_for_uuid};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::McpServerService;
use crate::storage::{
    CatalogListing, CreateMcpServerRow, EncryptionService, StorageBackend, UpdateMcpServer,
    UserMcpServerRow,
};

/// Most servers one person can own in one organization.
pub const MAX_USER_MCP_SERVERS: usize = 50;

/// Where a user MCP server came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UserMcpServerSource {
    /// Added from the organization's MCP catalog.
    Catalog,
    /// Added by URL.
    Custom,
}

/// Whether the person has signed in to a user MCP server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UserMcpConnectionStatus {
    /// Signed in, or an API key is stored.
    Connected,
    /// Needs a sign-in before its tools can be used.
    NotConnected,
    /// The server needs no sign-in.
    NotNeeded,
}

/// Sign-in state of a user MCP server.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UserMcpServerConnection {
    pub status: UserMcpConnectionStatus,
    /// Connection provider to pass to the authorize endpoint
    /// (`/v1/virtual-users/{id}/connections/{provider}/authorize`) when the
    /// server signs in with OAuth.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connected_at: Option<DateTime<Utc>>,
}

/// An MCP server a person added for themselves.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UserMcpServer {
    #[schema(value_type = String, example = "mcp_01933b5a00007000800000000000001")]
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub url: String,
    pub source: UserMcpServerSource,
    /// Catalog preset name, for servers added from the catalog.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_name: Option<String>,
    pub auth_mode: McpServerAuthMode,
    /// Disabled servers are kept but never offered to agents.
    pub enabled: bool,
    /// Whether the server loads on demand: agents see one line for it and
    /// list its tools only once they search for them. Off lists its tools
    /// from the start of every turn.
    pub deferred: bool,
    pub connection: UserMcpServerConnection,
    /// Names of the literal headers sent with each request. Values are
    /// write-only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub header_names: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Add a user MCP server. Give `catalog` to add a catalog preset, or `name`
/// and `url` to add a custom server.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct AddUserMcpServerRequest {
    /// Catalog preset name to add.
    #[serde(default)]
    #[schema(example = "linear")]
    pub catalog: Option<String>,
    /// Name for a custom server; also the tool prefix agents see.
    #[serde(default)]
    #[schema(example = "my-notes")]
    pub name: Option<String>,
    /// HTTPS endpoint of a custom server.
    #[serde(default)]
    #[schema(example = "https://mcp.example.com/mcp")]
    pub url: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// Sign-in for a custom server. Defaults to `api_key` when `api_key` is
    /// given, otherwise `none`.
    #[serde(default)]
    pub auth_mode: Option<McpServerAuthMode>,
    /// API key for a custom `api_key` server. Never returned.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Literal headers for a custom server. Never returned.
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    /// Defaults to true.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Load the server's tools on demand. Defaults to true.
    #[serde(default)]
    pub deferred: Option<bool>,
}

/// Change a user MCP server. The URL cannot change; remove and re-add the
/// server instead.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct UpdateUserMcpServerRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Load the server's tools on demand (true) or list them from the start
    /// of every turn (false).
    #[serde(default)]
    pub deferred: Option<bool>,
    /// Replace the API key of an `api_key` server.
    #[serde(default)]
    pub api_key: Option<String>,
}

/// Why a user MCP server request was refused.
#[derive(Debug, thiserror::Error)]
pub enum UserMcpServerError {
    #[error("{0}")]
    Invalid(String),
    #[error("MCP server not found")]
    NotFound,
    #[error("{0}")]
    Conflict(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

type Result<T> = std::result::Result<T, UserMcpServerError>;

fn invalid(message: impl Into<String>) -> UserMcpServerError {
    UserMcpServerError::Invalid(message.into())
}

fn validate_name(name: &str) -> Result<()> {
    if name.trim().is_empty() || name.len() > 64 {
        return Err(invalid("Name must be 1 to 64 characters"));
    }
    if !everruns_core::mcp_server::is_valid_mcp_server_name(name) {
        return Err(invalid(
            "MCP server name cannot contain consecutive underscores or end in an underscore after sanitization",
        ));
    }
    Ok(())
}

fn is_unique_violation(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<sqlx::Error>()
        .and_then(|e| e.as_database_error())
        .is_some_and(|e| e.is_unique_violation())
}

/// Lists, adds, changes and removes the servers one virtual user owns.
pub struct UserMcpServers<'a> {
    pub db: &'a StorageBackend,
    pub encryption: Option<&'a EncryptionService>,
    pub org_id: i64,
    pub owner: Uuid,
}

impl UserMcpServers<'_> {
    pub async fn list(&self) -> Result<Vec<UserMcpServer>> {
        let rows = self
            .db
            .list_user_mcp_servers(self.org_id, self.owner)
            .await?;
        let connections = self.connected_providers().await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(self.to_response(row, &connections).await?);
        }
        Ok(out)
    }

    pub async fn get(&self, id: Uuid) -> Result<UserMcpServer> {
        let row = self
            .db
            .get_user_mcp_server(self.org_id, self.owner, id)
            .await?
            .ok_or(UserMcpServerError::NotFound)?;
        let connections = self.connected_providers().await?;
        self.to_response(row, &connections).await
    }

    pub async fn add(&self, req: AddUserMcpServerRequest) -> Result<UserMcpServer> {
        let existing = self
            .db
            .list_user_mcp_servers(self.org_id, self.owner)
            .await?;
        if existing.len() >= MAX_USER_MCP_SERVERS {
            return Err(invalid(format!(
                "You can have at most {MAX_USER_MCP_SERVERS} MCP servers"
            )));
        }
        let enabled = req.enabled.unwrap_or(true);
        let deferred = req.deferred.unwrap_or(true);
        let (catalog_id, input) = match req.catalog.as_deref() {
            Some(catalog) => {
                if req.url.is_some() || req.api_key.is_some() || req.headers.is_some() {
                    return Err(invalid(
                        "A catalog server takes its URL and sign-in from the catalog",
                    ));
                }
                let preset = self
                    .db
                    .get_mcp_server_by_name(self.org_id, catalog)
                    .await?
                    .filter(|row| row.status == "active")
                    .ok_or_else(|| invalid(format!("No catalog MCP server named '{catalog}'")))?;
                let name = req.name.unwrap_or_else(|| preset.name.clone());
                validate_name(&name)?;
                (
                    Some(preset.id.uuid()),
                    CreateMcpServerRow {
                        name,
                        description: req.description.or(preset.description.clone()),
                        url: preset.url.clone(),
                        transport_type: preset.transport_type.clone(),
                        api_key_encrypted: None,
                        headers: None,
                        // The preset's settings decide sign-in; this row has none.
                        settings: Some(serde_json::json!({"auth_mode": "none"})),
                    },
                )
            }
            None => (None, self.custom_input(req)?),
        };
        let row = self
            .db
            .create_user_mcp_server(self.org_id, self.owner, catalog_id, deferred, input)
            .await
            .map_err(|e| {
                if is_unique_violation(&e) {
                    UserMcpServerError::Conflict(
                        "You already have an MCP server with this name".into(),
                    )
                } else {
                    e.into()
                }
            })?;
        let id = row.row.id.uuid();
        // Rows are created active; a disabled add is a second write.
        if !enabled {
            self.set_status(id, "disabled").await?;
        }
        self.get(id).await
    }

    fn custom_input(&self, req: AddUserMcpServerRequest) -> Result<CreateMcpServerRow> {
        let name = req
            .name
            .ok_or_else(|| invalid("Give a catalog name, or a name and URL"))?;
        validate_name(&name)?;
        let url = req
            .url
            .filter(|url| !url.trim().is_empty())
            .ok_or_else(|| invalid("A custom MCP server needs a URL"))?;
        validate_safe_url(&url).map_err(|e| invalid(format!("Invalid MCP server URL: {e}")))?;
        let auth_mode = req.auth_mode.unwrap_or(if req.api_key.is_some() {
            McpServerAuthMode::ApiKey
        } else {
            McpServerAuthMode::None
        });
        if req.api_key.is_some() && auth_mode != McpServerAuthMode::ApiKey {
            return Err(invalid("Only API key MCP servers can store an API key"));
        }
        let api_key_encrypted = if auth_mode == McpServerAuthMode::ApiKey {
            let key = req
                .api_key
                .filter(|key| !key.is_empty())
                .ok_or_else(|| invalid("API key sign-in needs an API key"))?;
            Some(self.encrypt(&key)?)
        } else {
            None
        };
        Ok(CreateMcpServerRow {
            name,
            description: req.description,
            url,
            transport_type: "http".to_string(),
            api_key_encrypted,
            headers: req
                .headers
                .map(|h| serde_json::to_value(h).unwrap_or_default()),
            settings: Some(serde_json::json!({ "auth_mode": auth_mode })),
        })
    }

    pub async fn update(&self, id: Uuid, req: UpdateUserMcpServerRequest) -> Result<UserMcpServer> {
        let current = self
            .db
            .get_user_mcp_server(self.org_id, self.owner, id)
            .await?
            .ok_or(UserMcpServerError::NotFound)?;
        if let Some(name) = req.name.as_deref() {
            validate_name(name)?;
        }
        let api_key_encrypted = match req.api_key.as_deref() {
            Some(key) => {
                let settings = McpServerService::settings_from_row(&current.row);
                if current.catalog_mcp_server_id.is_some()
                    || settings.auth_mode != McpServerAuthMode::ApiKey
                {
                    return Err(invalid("Only API key MCP servers can store an API key"));
                }
                if key.is_empty() {
                    return Err(invalid("API key cannot be empty"));
                }
                Some(self.encrypt(key)?)
            }
            None => None,
        };
        let status = req
            .enabled
            .map(|enabled| if enabled { "active" } else { "disabled" }.to_string());
        self.db
            .update_user_mcp_server(
                self.org_id,
                self.owner,
                id,
                UpdateMcpServer {
                    name: req.name,
                    description: req.description,
                    status,
                    api_key_encrypted,
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| {
                if is_unique_violation(&e) {
                    UserMcpServerError::Conflict(
                        "You already have an MCP server with this name".into(),
                    )
                } else {
                    e.into()
                }
            })?
            .ok_or(UserMcpServerError::NotFound)?;
        if let Some(deferred) = req.deferred {
            self.db
                .set_user_mcp_server_deferred(self.org_id, self.owner, id, deferred)
                .await?;
        }
        self.get(id).await
    }

    pub async fn remove(&self, id: Uuid) -> Result<()> {
        if self
            .db
            .delete_user_mcp_server(self.org_id, self.owner, id)
            .await?
        {
            Ok(())
        } else {
            Err(UserMcpServerError::NotFound)
        }
    }

    /// Put a catalog preset the person just signed in to on their list, unless
    /// it is already there. See `Database::list_catalog_server_for_owner`.
    pub async fn list_signed_in_catalog_server(&self, preset_id: Uuid) -> Result<CatalogListing> {
        Ok(self
            .db
            .list_catalog_server_for_owner(self.org_id, self.owner, preset_id, MAX_USER_MCP_SERVERS)
            .await?)
    }

    async fn set_status(&self, id: Uuid, status: &str) -> Result<()> {
        self.db
            .update_user_mcp_server(
                self.org_id,
                self.owner,
                id,
                UpdateMcpServer {
                    status: Some(status.to_string()),
                    ..Default::default()
                },
            )
            .await?;
        Ok(())
    }

    fn encrypt(&self, value: &str) -> Result<Vec<u8>> {
        let encryption = self
            .encryption
            .ok_or_else(|| anyhow::anyhow!("Encryption not configured. Cannot store API key."))?;
        Ok(encryption.encrypt_string(value)?)
    }

    async fn connected_providers(&self) -> Result<HashMap<String, DateTime<Utc>>> {
        Ok(self
            .db
            .list_user_connections(self.owner)
            .await?
            .into_iter()
            .filter(|c| c.provider.starts_with("mcp_oauth_"))
            .map(|c| (c.provider, c.created_at))
            .collect())
    }

    async fn to_response(
        &self,
        user: UserMcpServerRow,
        connections: &HashMap<String, DateTime<Utc>>,
    ) -> Result<UserMcpServer> {
        let deferred = user.deferred;
        let row = user.row;
        // A catalog server signs in exactly as its preset does.
        let (source, catalog_name, auth_mode, oauth_server) = match user.catalog_mcp_server_id {
            Some(catalog_id) => {
                let preset = self
                    .db
                    .get_mcp_server(self.org_id, catalog_id)
                    .await?
                    .filter(|preset| preset.status == "active");
                let auth_mode = preset
                    .as_ref()
                    .map(|p| McpServerService::settings_from_row(p).auth_mode)
                    .unwrap_or(McpServerAuthMode::None);
                (
                    UserMcpServerSource::Catalog,
                    preset.map(|p| p.name),
                    auth_mode,
                    catalog_id,
                )
            }
            None => (
                UserMcpServerSource::Custom,
                None,
                McpServerService::settings_from_row(&row).auth_mode,
                row.id.uuid(),
            ),
        };
        let connection = match auth_mode {
            McpServerAuthMode::OAuth => {
                let provider = mcp_oauth_provider_id_for_uuid(oauth_server);
                let connected_at = connections.get(&provider).copied();
                UserMcpServerConnection {
                    status: if connected_at.is_some() {
                        UserMcpConnectionStatus::Connected
                    } else {
                        UserMcpConnectionStatus::NotConnected
                    },
                    provider: Some(provider),
                    connected_at,
                }
            }
            McpServerAuthMode::ApiKey if row.api_key_set => UserMcpServerConnection {
                status: UserMcpConnectionStatus::Connected,
                provider: None,
                connected_at: None,
            },
            McpServerAuthMode::ApiKey => UserMcpServerConnection {
                status: UserMcpConnectionStatus::NotConnected,
                provider: None,
                connected_at: None,
            },
            _ => UserMcpServerConnection {
                status: UserMcpConnectionStatus::NotNeeded,
                provider: None,
                connected_at: None,
            },
        };
        let mut header_names: Vec<String> = row
            .headers
            .as_object()
            .map(|headers| headers.keys().cloned().collect())
            .unwrap_or_default();
        header_names.sort();
        Ok(UserMcpServer {
            id: row.id.to_string(),
            name: row.name,
            description: row.description,
            url: row.url,
            source,
            catalog_name,
            auth_mode,
            enabled: row.status == "active",
            deferred,
            connection,
            header_names,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

#[cfg(test)]
mod listing_tests;
#[cfg(test)]
mod tests;
