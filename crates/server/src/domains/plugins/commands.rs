// Plugin domain commands — marketplace CRUD + sync + install lifecycle.
//
// Policies:
//   PLUGIN_VIEW   — list/get (OrgPluginsView)
//   PLUGIN_MANAGE — write (OrgPluginsManage = Admin+ only)

use super::fetcher::{
    FetchedPluginFileSet, PluginSource, download_bytes, fetch_plugin, validate_github_repo_slug,
};
use super::queries as q;
use super::types::*;
use super::{PLUGIN_MANAGE, PLUGIN_VIEW};
use crate::domains::common::*;
use crate::kernel_imports::{
    DeploymentGrade, Policy, everruns_provider::typed_id::PluginInstallId,
    everruns_provider::typed_id::PluginMarketplaceId,
};
use everruns_core::plugins::compile_plugin;
use everruns_provider::url_validation::validate_safe_url;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

/// Response size cap for URL marketplace.json fetches: 1 MB.
const URL_MARKETPLACE_RESPONSE_CAP: usize = 1024 * 1024;

/// Timeout for GitHub API and raw-content requests.
const GITHUB_TIMEOUT_MS: u64 = 30_000;

// ============================================================================
// Input validation
// ============================================================================

fn validate_marketplace_name(name: &str) -> Result<(), CommandError> {
    if name.trim().is_empty() {
        return Err(CommandError::bad_request(
            "Marketplace name cannot be empty",
        ));
    }
    // Must match the DB constraint: ^[a-z][a-z0-9_-]*[a-z0-9]$
    let n = name.trim();
    if n.len() < 2 {
        return Err(CommandError::bad_request(
            "Marketplace name must be at least 2 characters",
        ));
    }
    let ok = n.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && n.chars()
            .last()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && n.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if !ok {
        return Err(CommandError::bad_request(
            "Marketplace name must be lowercase kebab-case (a-z, 0-9, -, _), \
             start with a letter, and not end with - or _",
        ));
    }
    if n.len() > 80 {
        return Err(CommandError::bad_request(
            "Marketplace name must not exceed 80 bytes",
        ));
    }
    Ok(())
}

fn validate_source_type(source_type: &str, _ctx: &Ctx) -> Result<(), CommandError> {
    match source_type {
        "github" | "url" => Ok(()),
        "local_path" => {
            // Gate local_path to dev-grade deployments only.
            if DeploymentGrade::from_env().is_dev() {
                Ok(())
            } else {
                Err(CommandError::bad_request(
                    "local_path source type is only allowed in development deployments",
                ))
            }
        }
        _ => Err(CommandError::bad_request(format!(
            "Invalid source_type '{source_type}'. Must be one of: github, url, local_path"
        ))),
    }
}

fn parse_marketplace_public_id(id: &str) -> Result<PluginMarketplaceId, CommandError> {
    id.parse::<PluginMarketplaceId>()
        .map_err(|e| CommandError::bad_request(format!("Invalid marketplace ID: {e}")))
}

fn parse_plugin_public_id(id: &str) -> Result<PluginInstallId, CommandError> {
    id.parse::<PluginInstallId>()
        .map_err(|e| CommandError::bad_request(format!("Invalid plugin ID: {e}")))
}

async fn get_marketplace_by_public_id(
    ctx: &Ctx,
    id: &str,
) -> Result<PluginMarketplaceRow, CommandError> {
    let public_id = parse_marketplace_public_id(id)?;
    ctx.db
        .get_plugin_marketplace_by_public_id(ctx.org_id(), &public_id.to_string())
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("Plugin marketplace"))
}

async fn get_install_by_public_id(ctx: &Ctx, id: &str) -> Result<PluginInstallRow, CommandError> {
    let public_id = parse_plugin_public_id(id)?;
    ctx.db
        .get_plugin_install_by_public_id(ctx.org_id(), &public_id.to_string())
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("Installed plugin"))
}

/// Parse the marketplace.json catalog (bytes or string) and validate it.
///
/// Returns the raw JSON value.
fn parse_marketplace_json(content: &str) -> Result<serde_json::Value, CommandError> {
    let value: serde_json::Value = serde_json::from_str(content)
        .map_err(|e| CommandError::bad_request(format!("Invalid marketplace.json: {e}")))?;
    // Must have `plugins` array.
    if value.get("plugins").and_then(|v| v.as_array()).is_none() {
        return Err(CommandError::bad_request(
            "marketplace.json must have a 'plugins' array",
        ));
    }
    Ok(value)
}

/// Read marketplace.json for a local_path marketplace.
fn read_local_marketplace_json(path: &str) -> Result<(String, Option<String>), CommandError> {
    let manifest_path = std::path::Path::new(path)
        .join(".claude-plugin")
        .join("marketplace.json");
    // Also try the root directly.
    let manifest_path = if manifest_path.exists() {
        manifest_path
    } else {
        std::path::Path::new(path).join("marketplace.json")
    };
    // Also try .claude-plugin/marketplace.json at root level.
    let content = std::fs::read_to_string(&manifest_path)
        .or_else(|_| {
            let alt = std::path::Path::new(path)
                .join(".claude-plugin")
                .join("marketplace.json");
            std::fs::read_to_string(alt)
        })
        .map_err(|e| {
            CommandError::bad_request(format!(
                "Cannot read marketplace.json from local_path '{}': {e}",
                path
            ))
        })?;
    Ok((content, None)) // no SHA for local_path
}

/// Sync a GitHub-hosted marketplace: resolve the HEAD SHA and fetch
/// `.claude-plugin/marketplace.json` at that commit.
async fn sync_github(
    source: &serde_json::Value,
    egress: &std::sync::Arc<dyn everruns_core::EgressService>,
) -> Result<(String, Option<String>), CommandError> {
    let repo = source
        .get("repo")
        .and_then(|v| v.as_str())
        .ok_or_else(|| CommandError::bad_request("github source requires a 'repo' field"))?;

    validate_github_repo_slug(repo).map_err(CommandError::bad_request)?;

    let parts: Vec<&str> = repo.splitn(2, '/').collect();
    let (owner, repo_name) = (parts[0], parts[1]);

    // Resolve the default-branch HEAD SHA via the GitHub REST API.
    let commits_url = format!("https://api.github.com/repos/{owner}/{repo_name}/commits/HEAD");
    let mut headers: Vec<(&str, String)> = vec![
        ("Accept", "application/vnd.github+json".to_string()),
        ("User-Agent", "everruns-plugin-sync/1.0".to_string()),
    ];

    // Attach GITHUB_TOKEN from environment if available (unauthenticated rate
    // limit is 60 req/hr; authenticated is 5000).
    if let Ok(token) = std::env::var("GITHUB_TOKEN")
        && !token.is_empty()
    {
        headers.push(("Authorization", format!("Bearer {token}")));
    }

    let api_body = download_bytes(
        &commits_url,
        egress,
        GITHUB_TIMEOUT_MS,
        URL_MARKETPLACE_RESPONSE_CAP,
        &headers,
    )
    .await
    .map_err(|e| CommandError::bad_request(format!("GitHub API request failed: {e}")))?;

    let api_json: serde_json::Value = serde_json::from_slice(&api_body).map_err(|e| {
        CommandError::bad_request(format!("GitHub API response is not valid JSON: {e}"))
    })?;

    let sha = api_json
        .get("sha")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            CommandError::bad_request("GitHub API response missing 'sha' field".to_string())
        })?
        .to_string();

    // Fetch marketplace.json at the resolved SHA.
    let raw_url = format!(
        "https://raw.githubusercontent.com/{owner}/{repo_name}/{sha}/.claude-plugin/marketplace.json"
    );

    // Size cap enforced while streaming inside download_bytes.
    let catalog_bytes = download_bytes(
        &raw_url,
        egress,
        GITHUB_TIMEOUT_MS,
        URL_MARKETPLACE_RESPONSE_CAP,
        &[],
    )
    .await
    .map_err(CommandError::bad_request)?;

    let catalog_str = String::from_utf8(catalog_bytes).map_err(|_| {
        CommandError::bad_request("marketplace.json from GitHub is not valid UTF-8")
    })?;

    Ok((catalog_str, Some(sha)))
}

/// Sync a URL-hosted marketplace: GET the URL and expect marketplace.json content.
async fn sync_url(
    source: &serde_json::Value,
    egress: &std::sync::Arc<dyn everruns_core::EgressService>,
) -> Result<(String, Option<String>), CommandError> {
    let url = source
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| CommandError::bad_request("url source requires a 'url' field"))?;

    // Static SSRF check at sync time (DNS-pinned check happens in download_bytes).
    validate_safe_url(url)
        .map_err(|e| CommandError::bad_request(format!("marketplace URL is not safe: {e}")))?;

    // Size cap enforced while streaming inside download_bytes.
    let content_bytes = download_bytes(
        url,
        egress,
        GITHUB_TIMEOUT_MS,
        URL_MARKETPLACE_RESPONSE_CAP,
        &[],
    )
    .await
    .map_err(CommandError::bad_request)?;

    let catalog_str = String::from_utf8(content_bytes)
        .map_err(|_| CommandError::bad_request("marketplace.json from URL is not valid UTF-8"))?;

    // last_synced_sha stays null for url sources.
    Ok((catalog_str, None))
}

/// Resolve the marketplace catalog for a sync operation.
fn sync_local_path(source: &serde_json::Value) -> Result<(String, Option<String>), CommandError> {
    let path = source
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| CommandError::bad_request("local_path source requires a 'path' field"))?;
    read_local_marketplace_json(path)
}

// ============================================================================
// ListPluginMarketplaces
// ============================================================================

/// List plugin marketplaces registered for this org.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ListPluginMarketplaces {
    pub search: Option<String>,
}

impl Command for ListPluginMarketplaces {
    type Output = Vec<PluginMarketplace>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_plugin_marketplaces",
            category: "plugins",
            description: "List plugin marketplaces registered for this organization.",
            method: "GET",
            path: "/v1/plugin_marketplaces",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&PLUGIN_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<PluginMarketplace>, CommandError> {
        let rows = ctx
            .db
            .list_plugin_marketplaces(ctx.org_id(), self.search.as_deref())
            .await
            .map_err(classify_anyhow)?;
        Ok(rows.iter().map(q::row_to_marketplace).collect())
    }
}

inventory::submit! { CommandDescriptor::of::<ListPluginMarketplaces>() }

// ============================================================================
// GetPluginMarketplace
// ============================================================================

/// Get a plugin marketplace by ID.
#[derive(Debug, Deserialize, ToSchema)]
pub struct GetPluginMarketplace {
    /// Public marketplace ID (`plgmkt_<32-hex>`).
    pub id: String,
}

impl Command for GetPluginMarketplace {
    type Output = PluginMarketplace;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_plugin_marketplace",
            category: "plugins",
            description: "Get a plugin marketplace by ID.",
            method: "GET",
            path: "/v1/plugin_marketplaces/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&PLUGIN_VIEW)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<PluginMarketplace, CommandError> {
        let row = get_marketplace_by_public_id(ctx, &self.id).await?;
        Ok(q::row_to_marketplace(&row))
    }
}

inventory::submit! { CommandDescriptor::of::<GetPluginMarketplace>() }

// ============================================================================
// CreatePluginMarketplace
// ============================================================================

/// Register a new plugin marketplace.
#[derive(Debug, Deserialize)]
pub struct CreatePluginMarketplaceCmd(pub CreatePluginMarketplaceRequest);

impl CommandSchema for CreatePluginMarketplaceCmd {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<CreatePluginMarketplaceRequest>()
    }
}

impl Command for CreatePluginMarketplaceCmd {
    type Output = PluginMarketplace;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_plugin_marketplace",
            category: "plugins",
            description: "Register a new plugin marketplace for this organization.",
            method: "POST",
            path: "/v1/plugin_marketplaces",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&PLUGIN_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<PluginMarketplace, CommandError> {
        let req = self.0;
        validate_marketplace_name(&req.name)?;
        validate_source_type(&req.source_type, ctx)?;

        // For url sources: apply static SSRF check at registration time.
        if req.source_type == "url" {
            validate_safe_url(&req.source)
                .map_err(|e| CommandError::bad_request(format!("url source is not safe: {e}")))?;
        }

        // For github sources: validate repo slug format.
        if req.source_type == "github" {
            validate_github_repo_slug(&req.source).map_err(CommandError::bad_request)?;
        }

        // Build source JSONB.
        let source = build_source_json(&req.source_type, &req.source);

        // Check name uniqueness.
        if ctx
            .db
            .list_plugin_marketplaces(ctx.org_id(), Some(&req.name))
            .await
            .map_err(classify_anyhow)?
            .iter()
            .any(|r| r.name == req.name)
        {
            return Err(CommandError::conflict(format!(
                "A marketplace with name '{}' already exists",
                req.name
            )));
        }

        let row = ctx
            .db
            .create_plugin_marketplace(
                ctx.org_id(),
                CreatePluginMarketplaceRow {
                    public_id: PluginMarketplaceId::new().to_string(),
                    name: req.name,
                    source_type: req.source_type,
                    source,
                },
            )
            .await
            .map_err(classify_anyhow)?;

        Ok(q::row_to_marketplace(&row))
    }
}

inventory::submit! { CommandDescriptor::of::<CreatePluginMarketplaceCmd>() }

/// Return the egress service from `ctx` for source types that need it
/// (`github`, `url`). For `local_path`, returns a no-op disabled service
/// because the fetcher never calls egress for local sources.
fn require_egress_for_source(
    ctx: &Ctx,
    source_type: &str,
) -> Result<std::sync::Arc<dyn everruns_core::EgressService>, CommandError> {
    match source_type {
        "local_path" => {
            // local_path fetch never makes outbound calls; return a disabled
            // service as a placeholder so fetch_plugin receives something valid.
            Ok(std::sync::Arc::new(everruns_core::DisabledEgressService))
        }
        _ => ctx.egress_service.clone().ok_or_else(|| {
            CommandError::bad_request("no egress service configured for remote fetch")
        }),
    }
}

/// Reject compiled definitions with unsafe MCP server URLs at install/update
/// time, mirroring declarative capability create/patch (TM-PLUGIN-003).
/// Without this gate an unsafe URL would only surface at runtime, where
/// scoped-MCP validation fails the whole server set for the turn.
fn validate_compiled_mcp_servers(
    definition: &everruns_core::DeclarativeCapabilityDefinition,
) -> Result<(), CommandError> {
    if let Some(servers) = &definition.mcp_servers {
        crate::domains::mcp_servers::scoped_mcp::validate_scoped_mcp_servers(servers)
            .map_err(|error| CommandError::bad_request(format!("Invalid MCP servers: {error}")))?;
    }
    Ok(())
}

/// The marketplace/install row name is the trusted plugin identity. Reject a
/// fetched package whose manifest compiles to a different definition name so
/// updates cannot rebind OAuth anchors owned by another installed plugin.
fn validate_compiled_plugin_identity(
    expected_name: &str,
    definition: &everruns_core::DeclarativeCapabilityDefinition,
) -> Result<(), CommandError> {
    if definition.name != expected_name {
        return Err(CommandError::bad_request(format!(
            "Plugin manifest name '{}' does not match expected plugin '{}'",
            definition.name, expected_name
        )));
    }
    Ok(())
}

fn build_source_json(source_type: &str, source_value: &str) -> serde_json::Value {
    match source_type {
        "github" => serde_json::json!({ "repo": source_value }),
        "url" => serde_json::json!({ "url": source_value }),
        "local_path" => serde_json::json!({ "path": source_value }),
        _ => serde_json::json!({ "value": source_value }),
    }
}

// ============================================================================
// UpdatePluginMarketplaceCmd
// ============================================================================

/// Update a plugin marketplace's name or status.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdatePluginMarketplaceCmd {
    /// Public marketplace ID.
    pub id: String,
    #[serde(flatten)]
    pub req: UpdatePluginMarketplaceRequest,
}

impl Command for UpdatePluginMarketplaceCmd {
    type Output = PluginMarketplace;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_plugin_marketplace",
            category: "plugins",
            description: "Update a plugin marketplace (name or status).",
            method: "PATCH",
            path: "/v1/plugin_marketplaces/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&PLUGIN_MANAGE)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<PluginMarketplace, CommandError> {
        let existing = get_marketplace_by_public_id(ctx, &self.id).await?;
        let req = self.req;
        if let Some(ref name) = req.name {
            validate_marketplace_name(name)?;
        }
        if req
            .status
            .as_deref()
            .is_some_and(|s| !matches!(s, "active" | "disabled"))
        {
            return Err(CommandError::bad_request(
                "status must be 'active' or 'disabled'",
            ));
        }
        let row = ctx
            .db
            .update_plugin_marketplace(
                ctx.org_id(),
                existing.id,
                UpdatePluginMarketplace {
                    name: req.name,
                    source_type: None,
                    source: None,
                    status: req.status,
                    catalog: None,
                    last_synced_at: None,
                    last_synced_sha: None,
                },
            )
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Plugin marketplace"))?;
        Ok(q::row_to_marketplace(&row))
    }
}

inventory::submit! { CommandDescriptor::of::<UpdatePluginMarketplaceCmd>() }

// ============================================================================
// DeletePluginMarketplace
// ============================================================================

/// Delete a plugin marketplace registration. Installed plugins from this
/// marketplace become unattached (marketplace_id is set to NULL).
#[derive(Debug, Deserialize, ToSchema)]
pub struct DeletePluginMarketplace {
    /// Public marketplace ID.
    pub id: String,
}

impl Command for DeletePluginMarketplace {
    type Output = serde_json::Value;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_plugin_marketplace",
            category: "plugins",
            description: "Delete a plugin marketplace. Installed plugins become unattached.",
            method: "DELETE",
            path: "/v1/plugin_marketplaces/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&PLUGIN_MANAGE)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let row = get_marketplace_by_public_id(ctx, &self.id).await?;
        let deleted = ctx
            .db
            .delete_plugin_marketplace(ctx.org_id(), row.id)
            .await
            .map_err(classify_anyhow)?;
        if deleted {
            Ok(serde_json::json!({ "deleted": true }))
        } else {
            Err(CommandError::not_found("Plugin marketplace"))
        }
    }
}

inventory::submit! { CommandDescriptor::of::<DeletePluginMarketplace>() }

// ============================================================================
// SyncPluginMarketplace
// ============================================================================

/// Re-sync a plugin marketplace catalog from its source.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SyncPluginMarketplace {
    /// Public marketplace ID.
    pub id: String,
}

impl Command for SyncPluginMarketplace {
    type Output = PluginMarketplace;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "sync_plugin_marketplace",
            category: "plugins",
            description: "Re-sync the plugin marketplace catalog from its source.",
            method: "POST",
            path: "/v1/plugin_marketplaces/{id}/sync",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&PLUGIN_MANAGE)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<PluginMarketplace, CommandError> {
        let marketplace_row = get_marketplace_by_public_id(ctx, &self.id).await?;

        let (catalog_str, resolved_sha) = match marketplace_row.source_type.as_str() {
            "local_path" => sync_local_path(&marketplace_row.source)?,
            "github" | "url" => {
                let egress = ctx.egress_service.as_ref().ok_or_else(|| {
                    CommandError::bad_request("no egress service configured for remote sync")
                })?;
                if marketplace_row.source_type == "github" {
                    sync_github(&marketplace_row.source, egress).await?
                } else {
                    sync_url(&marketplace_row.source, egress).await?
                }
            }
            other => {
                return Err(CommandError::bad_request(format!(
                    "Unknown source type: {other}"
                )));
            }
        };

        let catalog = parse_marketplace_json(&catalog_str)?;
        let now = chrono::Utc::now();

        let updated = ctx
            .db
            .update_plugin_marketplace(
                ctx.org_id(),
                marketplace_row.id,
                UpdatePluginMarketplace {
                    name: None,
                    source_type: None,
                    source: None,
                    status: None,
                    catalog: Some(catalog),
                    last_synced_at: Some(now),
                    last_synced_sha: Some(resolved_sha),
                },
            )
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Plugin marketplace"))?;

        Ok(q::row_to_marketplace(&updated))
    }
}

inventory::submit! { CommandDescriptor::of::<SyncPluginMarketplace>() }

// ============================================================================
// GetMarketplaceCatalog
// ============================================================================

/// List the synced catalog entries for a marketplace, with `installed` flag.
#[derive(Debug, Deserialize, ToSchema)]
pub struct GetMarketplaceCatalog {
    /// Public marketplace ID.
    pub id: String,
}

impl Command for GetMarketplaceCatalog {
    type Output = Vec<MarketplaceCatalogEntry>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_marketplace_catalog",
            category: "plugins",
            description: "List plugin catalog entries for a marketplace (with installed flag per entry).",
            method: "GET",
            path: "/v1/plugin_marketplaces/{id}/plugins",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&PLUGIN_VIEW)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<MarketplaceCatalogEntry>, CommandError> {
        let marketplace_row = get_marketplace_by_public_id(ctx, &self.id).await?;

        let catalog = marketplace_row.catalog.as_ref().ok_or_else(|| {
            CommandError::bad_request(
                "This marketplace has not been synced yet. Run POST .../sync first.",
            )
        })?;

        let plugins = catalog
            .get("plugins")
            .and_then(|v| v.as_array())
            .ok_or_else(|| CommandError::bad_request("Catalog is missing a 'plugins' array"))?;

        // Collect installed plugin names for the org to set the `installed` flag.
        let installed_rows = ctx
            .db
            .list_plugin_installs(ctx.org_id(), None)
            .await
            .map_err(classify_anyhow)?;
        let installed_names: std::collections::HashSet<_> =
            installed_rows.iter().map(|r| r.name.as_str()).collect();

        let entries = plugins
            .iter()
            .filter_map(|entry| {
                let name = entry.get("name")?.as_str()?.to_string();
                let display_name = entry
                    .get("display_name")
                    .or_else(|| entry.get("displayName"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let description = entry
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let version = entry
                    .get("version")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let author = entry
                    .get("author")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let category = entry
                    .get("category")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let installed = installed_names.contains(name.as_str());
                Some(MarketplaceCatalogEntry {
                    name,
                    display_name,
                    description,
                    version,
                    author,
                    category,
                    installed,
                })
            })
            .collect();

        Ok(entries)
    }
}

inventory::submit! { CommandDescriptor::of::<GetMarketplaceCatalog>() }

// ============================================================================
// ListPlugins
// ============================================================================

/// List installed plugins for this org.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ListPlugins {
    pub search: Option<String>,
}

impl Command for ListPlugins {
    type Output = Vec<InstalledPlugin>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_plugins",
            category: "plugins",
            description: "List installed plugins for this organization.",
            method: "GET",
            path: "/v1/plugins",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&PLUGIN_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<InstalledPlugin>, CommandError> {
        let installs = ctx
            .db
            .list_plugin_installs(ctx.org_id(), self.search.as_deref())
            .await
            .map_err(classify_anyhow)?;

        // Batch load marketplace rows to compute `update_available`.
        let marketplace_ids: Vec<Uuid> = installs
            .iter()
            .filter_map(|i| i.marketplace_id)
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        let mut marketplaces = std::collections::HashMap::new();
        for mid in marketplace_ids {
            if let Some(mkt) = ctx
                .db
                .get_plugin_marketplace(ctx.org_id(), mid)
                .await
                .map_err(classify_anyhow)?
            {
                marketplaces.insert(mid, mkt);
            }
        }

        Ok(installs
            .iter()
            .map(|install| {
                let mkt = install
                    .marketplace_id
                    .and_then(|mid| marketplaces.get(&mid));
                q::row_to_installed_plugin(install, mkt)
            })
            .collect())
    }
}

inventory::submit! { CommandDescriptor::of::<ListPlugins>() }

// ============================================================================
// GetPlugin
// ============================================================================

/// Get an installed plugin by ID.
#[derive(Debug, Deserialize, ToSchema)]
pub struct GetPlugin {
    /// Public plugin ID (`plugin_<32-hex>`).
    pub id: String,
}

impl Command for GetPlugin {
    type Output = InstalledPlugin;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_plugin",
            category: "plugins",
            description: "Get an installed plugin by ID.",
            method: "GET",
            path: "/v1/plugins/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&PLUGIN_VIEW)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<InstalledPlugin, CommandError> {
        let install = get_install_by_public_id(ctx, &self.id).await?;
        let mkt = if let Some(mid) = install.marketplace_id {
            ctx.db
                .get_plugin_marketplace(ctx.org_id(), mid)
                .await
                .map_err(classify_anyhow)?
        } else {
            None
        };
        Ok(q::row_to_installed_plugin(&install, mkt.as_ref()))
    }
}

inventory::submit! { CommandDescriptor::of::<GetPlugin>() }

// ============================================================================
// InstallPlugin
// ============================================================================

/// Install a plugin from a marketplace catalog entry.
#[derive(Debug, Deserialize)]
pub struct InstallPluginCmd(pub InstallPluginRequest);

impl CommandSchema for InstallPluginCmd {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<InstallPluginRequest>()
    }
}

impl Command for InstallPluginCmd {
    type Output = InstalledPlugin;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "install_plugin",
            category: "plugins",
            description: "Install a plugin from a marketplace catalog entry.",
            method: "POST",
            path: "/v1/plugins",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&PLUGIN_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<InstalledPlugin, CommandError> {
        let req = self.0;

        // Resolve marketplace.
        let marketplace_public_id = parse_marketplace_public_id(&req.marketplace_id)?;
        let marketplace_row = ctx
            .db
            .get_plugin_marketplace_by_public_id(ctx.org_id(), &marketplace_public_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Plugin marketplace"))?;

        // Find the plugin entry in the catalog.
        let catalog = marketplace_row.catalog.as_ref().ok_or_else(|| {
            CommandError::bad_request(
                "This marketplace has not been synced yet. Run POST .../sync first.",
            )
        })?;

        let entry = catalog
            .get("plugins")
            .and_then(|v| v.as_array())
            .and_then(|arr| {
                arr.iter().find(|e| {
                    e.get("name")
                        .and_then(|v| v.as_str())
                        .is_some_and(|n| n == req.plugin_name)
                })
            })
            .cloned()
            .ok_or_else(|| {
                CommandError::not_found(
                    format!(
                        "Plugin '{}' not found in marketplace catalog",
                        req.plugin_name
                    )
                    .as_str(),
                )
            })?;

        let source_value = entry
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or(&req.plugin_name)
            .to_string();

        // Check if already installed.
        if ctx
            .db
            .get_plugin_install_by_name(ctx.org_id(), &req.plugin_name)
            .await
            .map_err(classify_anyhow)?
            .is_some()
        {
            return Err(CommandError::conflict(format!(
                "Plugin '{}' is already installed",
                req.plugin_name
            )));
        }

        // Fetch and compile.
        let local_path = marketplace_row
            .source
            .get("path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let github_repo = marketplace_row
            .source
            .get("repo")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let plugin_source = PluginSource {
            source_value,
            marketplace_source_type: marketplace_row.source_type.clone(),
            marketplace_local_path: local_path,
            marketplace_github_repo: github_repo,
            marketplace_last_synced_sha: marketplace_row.last_synced_sha.clone(),
        };

        let egress = require_egress_for_source(ctx, &marketplace_row.source_type)?;

        let FetchedPluginFileSet {
            file_set,
            resolved_sha,
        } = fetch_plugin(&plugin_source, &egress)
            .await
            .map_err(CommandError::bad_request)?;

        let compiled = compile_plugin(&file_set)
            .map_err(|e| CommandError::bad_request(format!("Plugin compilation failed: {e}")))?;
        validate_compiled_plugin_identity(&req.plugin_name, &compiled.definition)?;
        validate_compiled_mcp_servers(&compiled.definition)?;

        // OAuth MCP servers get an anchor row and a host-assigned provider id
        // (see oauth_anchor.rs).
        let mut definition = compiled.definition.clone();
        super::oauth_anchor::sync_plugin_oauth_anchors(
            &ctx.db,
            ctx.org_id(),
            &req.plugin_name,
            &mut definition,
        )
        .await
        .map_err(CommandError::internal)?;

        // Persist.
        let manifest_json = serde_json::to_value(&compiled.manifest)
            .map_err(|e| CommandError::internal(e.into()))?;
        let definition_json =
            serde_json::to_value(&definition).map_err(|e| CommandError::internal(e.into()))?;
        let warnings_json = serde_json::to_value(&compiled.warnings)
            .map_err(|e| CommandError::internal(e.into()))?;
        let version = compiled.manifest.version.clone();

        let source_json = serde_json::json!({
            "marketplace_id": marketplace_row.public_id,
            "plugin_name": req.plugin_name,
        });

        let install_row = ctx
            .db
            .create_plugin_install(
                ctx.org_id(),
                CreatePluginInstallRow {
                    public_id: PluginInstallId::new().to_string(),
                    name: compiled.definition.name.clone(),
                    marketplace_id: Some(marketplace_row.id),
                    source: source_json,
                    version,
                    pinned_sha: resolved_sha,
                    manifest: manifest_json,
                    definition: definition_json,
                    warnings: warnings_json,
                },
            )
            .await
            .map_err(classify_anyhow)?;

        Ok(q::row_to_installed_plugin(
            &install_row,
            Some(&marketplace_row),
        ))
    }
}

inventory::submit! { CommandDescriptor::of::<InstallPluginCmd>() }

// ============================================================================
// PatchInstalledPlugin (enable / disable)
// ============================================================================

/// Update an installed plugin's status (enable/disable).
#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchInstalledPlugin {
    /// Public plugin ID.
    pub id: String,
    #[serde(flatten)]
    pub req: UpdateInstalledPluginRequest,
}

impl Command for PatchInstalledPlugin {
    type Output = InstalledPlugin;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "patch_installed_plugin",
            category: "plugins",
            description: "Update an installed plugin's status (active/disabled).",
            method: "PATCH",
            path: "/v1/plugins/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&PLUGIN_MANAGE)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<InstalledPlugin, CommandError> {
        let existing = get_install_by_public_id(ctx, &self.id).await?;
        if self
            .req
            .status
            .as_deref()
            .is_some_and(|s| !matches!(s, "active" | "disabled"))
        {
            return Err(CommandError::bad_request(
                "status must be 'active' or 'disabled'",
            ));
        }
        let updated = ctx
            .db
            .update_plugin_install(
                ctx.org_id(),
                existing.id,
                UpdatePluginInstall {
                    status: self.req.status,
                    ..Default::default()
                },
            )
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Installed plugin"))?;

        let mkt = if let Some(mid) = updated.marketplace_id {
            ctx.db
                .get_plugin_marketplace(ctx.org_id(), mid)
                .await
                .map_err(classify_anyhow)?
        } else {
            None
        };

        Ok(q::row_to_installed_plugin(&updated, mkt.as_ref()))
    }
}

inventory::submit! { CommandDescriptor::of::<PatchInstalledPlugin>() }

// ============================================================================
// UninstallPlugin (delete)
// ============================================================================

/// Uninstall a plugin. Agents/harnesses referencing `plugin:{install_id}` will
/// surface a dangling-ref error (same behaviour as deleted declarative caps).
#[derive(Debug, Deserialize, ToSchema)]
pub struct UninstallPlugin {
    /// Public plugin ID.
    pub id: String,
}

impl Command for UninstallPlugin {
    type Output = serde_json::Value;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "uninstall_plugin",
            category: "plugins",
            description: "Uninstall a plugin. Capability ref becomes dangling for assigned agents.",
            method: "DELETE",
            path: "/v1/plugins/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&PLUGIN_MANAGE)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let existing = get_install_by_public_id(ctx, &self.id).await?;
        let deleted = ctx
            .db
            .delete_plugin_install(ctx.org_id(), existing.id)
            .await
            .map_err(classify_anyhow)?;
        if deleted {
            super::oauth_anchor::delete_plugin_oauth_anchors(&ctx.db, ctx.org_id(), &existing.name)
                .await
                .map_err(CommandError::internal)?;
            Ok(serde_json::json!({ "deleted": true }))
        } else {
            Err(CommandError::not_found("Installed plugin"))
        }
    }
}

inventory::submit! { CommandDescriptor::of::<UninstallPlugin>() }

// ============================================================================
// UpdatePlugin (re-install at current catalog version)
// ============================================================================

/// Re-compile and update an installed plugin from the marketplace's current catalog.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdatePlugin {
    /// Public plugin ID.
    pub id: String,
}

impl Command for UpdatePlugin {
    type Output = InstalledPlugin;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_plugin",
            category: "plugins",
            description: "Re-install an installed plugin at the marketplace's current catalog version.",
            method: "POST",
            path: "/v1/plugins/{id}/update",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&PLUGIN_MANAGE)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<InstalledPlugin, CommandError> {
        let install = get_install_by_public_id(ctx, &self.id).await?;

        let Some(marketplace_id) = install.marketplace_id else {
            return Err(CommandError::bad_request(
                "Plugin was not installed from a marketplace and cannot be updated automatically.",
            ));
        };

        let marketplace_row = ctx
            .db
            .get_plugin_marketplace(ctx.org_id(), marketplace_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| {
                CommandError::bad_request(
                    "Installed plugin's marketplace no longer exists. \
                     Cannot update without a marketplace.",
                )
            })?;

        // Resolve source from catalog.
        let catalog = marketplace_row.catalog.as_ref().ok_or_else(|| {
            CommandError::bad_request("Marketplace has no synced catalog. Run POST .../sync first.")
        })?;

        let entry = catalog
            .get("plugins")
            .and_then(|v| v.as_array())
            .and_then(|arr| {
                arr.iter().find(|e| {
                    e.get("name")
                        .and_then(|v| v.as_str())
                        .is_some_and(|n| n == install.name)
                })
            })
            .cloned()
            .ok_or_else(|| {
                CommandError::bad_request(format!(
                    "Plugin '{}' not found in current marketplace catalog",
                    install.name
                ))
            })?;

        let source_value = entry
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or(install.name.as_str())
            .to_string();

        let local_path = marketplace_row
            .source
            .get("path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let github_repo = marketplace_row
            .source
            .get("repo")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let plugin_source = PluginSource {
            source_value,
            marketplace_source_type: marketplace_row.source_type.clone(),
            marketplace_local_path: local_path,
            marketplace_github_repo: github_repo,
            marketplace_last_synced_sha: marketplace_row.last_synced_sha.clone(),
        };

        let egress = require_egress_for_source(ctx, &marketplace_row.source_type)?;

        let FetchedPluginFileSet {
            file_set,
            resolved_sha,
        } = fetch_plugin(&plugin_source, &egress)
            .await
            .map_err(CommandError::bad_request)?;

        let compiled = compile_plugin(&file_set)
            .map_err(|e| CommandError::bad_request(format!("Plugin compilation failed: {e}")))?;
        validate_compiled_plugin_identity(&install.name, &compiled.definition)?;
        validate_compiled_mcp_servers(&compiled.definition)?;

        // Re-sync OAuth anchors: reuse existing anchors (provider ids and
        // user connections survive updates), drop anchors for removed servers.
        let mut definition = compiled.definition.clone();
        super::oauth_anchor::sync_plugin_oauth_anchors(
            &ctx.db,
            ctx.org_id(),
            &install.name,
            &mut definition,
        )
        .await
        .map_err(CommandError::internal)?;

        let manifest_json = serde_json::to_value(&compiled.manifest)
            .map_err(|e| CommandError::internal(e.into()))?;
        let definition_json =
            serde_json::to_value(&definition).map_err(|e| CommandError::internal(e.into()))?;
        let warnings_json = serde_json::to_value(&compiled.warnings)
            .map_err(|e| CommandError::internal(e.into()))?;
        let version = compiled.manifest.version.clone();

        let updated = ctx
            .db
            .update_plugin_install(
                ctx.org_id(),
                install.id,
                UpdatePluginInstall {
                    version,
                    pinned_sha: resolved_sha,
                    manifest: Some(manifest_json),
                    definition: Some(definition_json),
                    warnings: Some(warnings_json),
                    ..Default::default()
                },
            )
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Installed plugin"))?;

        Ok(q::row_to_installed_plugin(&updated, Some(&marketplace_row)))
    }
}

inventory::submit! { CommandDescriptor::of::<UpdatePlugin>() }

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::capabilities::DeclarativeCapabilityDefinition;

    #[test]
    fn compiled_plugin_identity_accepts_expected_name() {
        let definition = DeclarativeCapabilityDefinition {
            name: "resend".to_string(),
            ..DeclarativeCapabilityDefinition::default()
        };

        validate_compiled_plugin_identity("resend", &definition).unwrap();
    }

    #[test]
    fn compiled_plugin_identity_rejects_manifest_name_mismatch() {
        let definition = DeclarativeCapabilityDefinition {
            name: "resend".to_string(),
            ..DeclarativeCapabilityDefinition::default()
        };

        let err = validate_compiled_plugin_identity("evilmail", &definition).unwrap_err();

        assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST);
        assert!(
            err.message()
                .contains("does not match expected plugin 'evilmail'"),
            "{}",
            err.message()
        );
    }
}
