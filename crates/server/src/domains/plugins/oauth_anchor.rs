// Plugin MCP OAuth anchors.
//
// A plugin-declared MCP server with `auth_mode = oauth` needs (1) a stable
// per-org OAuth provider id (`mcp_oauth_{uuid}`) and (2) a place to persist
// discovered authorization-server metadata plus the dynamically registered
// OAuth client. Instead of a parallel store, each such server gets a linked
// org `mcp_servers` row created at install time — the *anchor* — kept in
// `status = disabled` so it never becomes a runtime capability and never
// resolves by tool prefix. The existing user-connections OAuth flow
// (authorize/callback, token storage, providers listing) works against the
// anchor unchanged; the compiled scoped server carries the anchor's provider
// id. Spec: knowledge/integrations/plugins.md.
//
// Security: the provider id is always assigned here, server-side, from a row
// this module created. Plugin content can only mark a server as OAuth — it
// can never bind to an arbitrary provider id (e.g. `github`) and read tokens
// connected for other providers (TM-PLUGIN-008).

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use everruns_core::capabilities::DeclarativeCapabilityDefinition;
use everruns_core::{McpServerAuthMode, mcp_oauth_provider_id_for_uuid};
use uuid::Uuid;

use crate::domains::mcp_servers::McpServerSettings;
use crate::storage::StorageBackend;
use crate::storage::models::{CreateMcpServerRow, McpServerRow, UpdateMcpServer};

/// Marker key inside `mcp_servers.settings` identifying a plugin OAuth anchor.
const ANCHOR_KEY: &str = "plugin_anchor";
pub(crate) const CONNECTION_DISPLAY_NAME_KEY: &str = "connection_display_name";

pub(crate) fn humanize_connection_name(name: &str) -> String {
    name.split(['-', '_', ' '])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            match characters.next() {
                Some(first) => first.to_uppercase().chain(characters).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Build the `settings` JSON for an anchor row. Auth mode is serialized through
/// the typed `McpServerSettings` (not a hand-written string) so it matches the
/// exact serde representation `settings_from_row` deserializes — otherwise the
/// connections API would read the anchor as `auth_mode = none`.
fn anchor_settings_value(
    plugin_name: &str,
    server_name: &str,
    connection_display_name: &str,
) -> serde_json::Value {
    let settings = McpServerSettings {
        auth_mode: McpServerAuthMode::OAuth,
        ..Default::default()
    };
    let mut value = serde_json::to_value(settings).unwrap_or_else(|_| serde_json::json!({}));
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            ANCHOR_KEY.to_string(),
            serde_json::json!({ "plugin": plugin_name, "server": server_name }),
        );
        obj.insert(
            CONNECTION_DISPLAY_NAME_KEY.to_string(),
            serde_json::json!(connection_display_name),
        );
    }
    value
}

/// If `row` is an OAuth anchor for `plugin_name`, return the plugin MCP
/// server name it anchors.
fn anchored_server_name(row: &McpServerRow, plugin_name: &str) -> Option<String> {
    let anchor = row.settings.get(ANCHOR_KEY)?;
    if anchor.get("plugin").and_then(|v| v.as_str()) != Some(plugin_name) {
        return None;
    }
    anchor
        .get("server")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// List the OAuth anchor rows for a plugin, keyed by plugin server name.
async fn list_plugin_anchors(
    db: &StorageBackend,
    org_id: i64,
    plugin_name: &str,
) -> Result<BTreeMap<String, McpServerRow>> {
    // include_archived=false: live (active/disabled) rows only, so archived
    // (deleted) anchors are not resurrected.
    let rows = db.list_mcp_servers(org_id, None, false).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| anchored_server_name(&row, plugin_name).map(|server| (server, row)))
        .collect())
}

/// Ensure every OAuth MCP server in `definition` has an anchor row and carries
/// its provider id; remove anchors for servers the plugin no longer declares.
///
/// Mutates `definition` in place (sets `oauth_provider_id` on OAuth servers).
/// Idempotent: re-running against unchanged anchors reuses them, preserving any
/// registered OAuth client and existing user connections across plugin
/// updates. If a server's URL changed, its anchor is replaced so credentials
/// issued by the previous OAuth authority cannot be refreshed against the new
/// server.
pub async fn sync_plugin_oauth_anchors(
    db: &StorageBackend,
    org_id: i64,
    plugin_name: &str,
    definition: &mut DeclarativeCapabilityDefinition,
) -> Result<()> {
    let mut existing = list_plugin_anchors(db, org_id, plugin_name).await?;
    let fallback_display_name = humanize_connection_name(plugin_name);
    let plugin_display_name = definition
        .display_name
        .as_deref()
        .unwrap_or(&fallback_display_name);

    if let Some(servers) = definition.mcp_servers.as_mut() {
        for (server_name, server) in servers.iter_mut() {
            if server.auth_mode != McpServerAuthMode::OAuth {
                continue;
            }
            let connection_display_name = if server_name == plugin_name {
                plugin_display_name.to_string()
            } else {
                format!("{plugin_display_name} — {server_name}")
            };
            let anchor_id = match existing.remove(server_name) {
                Some(row) => {
                    if row.url != server.url {
                        // The provider id is the credential's authority binding.
                        // Never reuse it after retargeting: the refresh path must
                        // not send old user secrets to newly discovered metadata.
                        db.delete_mcp_server(org_id, row.id.uuid()).await?;
                        create_anchor(
                            db,
                            org_id,
                            plugin_name,
                            server_name,
                            &connection_display_name,
                            &server.url,
                        )
                        .await?
                    } else {
                        let mut settings = row.settings.clone();
                        if let Some(object) = settings.as_object_mut() {
                            object.insert(
                                CONNECTION_DISPLAY_NAME_KEY.to_string(),
                                serde_json::json!(connection_display_name),
                            );
                        }
                        db.update_mcp_server(
                            org_id,
                            row.id.uuid(),
                            UpdateMcpServer {
                                settings: Some(settings),
                                ..Default::default()
                            },
                        )
                        .await?;
                        row.id.uuid()
                    }
                }
                None => {
                    create_anchor(
                        db,
                        org_id,
                        plugin_name,
                        server_name,
                        &connection_display_name,
                        &server.url,
                    )
                    .await?
                }
            };
            server.oauth_provider_id = Some(mcp_oauth_provider_id_for_uuid(anchor_id));
        }
    }

    // Anchors for servers the plugin no longer declares (or that are no
    // longer OAuth) are removed; their user connections become dangling and
    // are cleaned up by the normal connection lifecycle.
    for (_, row) in existing {
        db.delete_mcp_server(org_id, row.id.uuid()).await?;
    }

    Ok(())
}

/// Delete all OAuth anchor rows for a plugin (uninstall).
pub async fn delete_plugin_oauth_anchors(
    db: &StorageBackend,
    org_id: i64,
    plugin_name: &str,
) -> Result<()> {
    for (_, row) in list_plugin_anchors(db, org_id, plugin_name).await? {
        db.delete_mcp_server(org_id, row.id.uuid()).await?;
    }
    Ok(())
}

async fn create_anchor(
    db: &StorageBackend,
    org_id: i64,
    plugin_name: &str,
    server_name: &str,
    connection_display_name: &str,
    url: &str,
) -> Result<Uuid> {
    // Prefer the friendliest available name — it is what the Connections UI
    // shows as the provider display name.
    let mut candidates = vec![server_name.to_string()];
    if plugin_name != server_name {
        candidates.push(format!("{plugin_name}-{server_name}"));
    }
    candidates.push(format!("plugin-{plugin_name}-{server_name}"));

    let taken: std::collections::HashSet<String> = db
        .list_mcp_servers(org_id, None, true) // include archived: names may still collide
        .await?
        .into_iter()
        .map(|row| row.name)
        .collect();
    let name = candidates
        .into_iter()
        .find(|candidate| !taken.contains(candidate))
        .unwrap_or_else(|| format!("plugin-{plugin_name}-{server_name}-{}", Uuid::new_v4()));

    let row = db
        .create_mcp_server(
            org_id,
            CreateMcpServerRow {
                name,
                description: Some(format!(
                    "OAuth connection for MCP server '{server_name}' of plugin '{plugin_name}'. \
                     Managed by the plugin install; do not enable directly."
                )),
                url: url.to_string(),
                transport_type: "http".to_string(),
                api_key_encrypted: None,
                headers: None,
                settings: Some(anchor_settings_value(
                    plugin_name,
                    server_name,
                    connection_display_name,
                )),
            },
        )
        .await
        .context("failed to create plugin OAuth anchor")?;

    // Anchors are never runtime capabilities: disabled rows are skipped by
    // capability listing and tool-prefix resolution, but still surface as
    // OAuth providers in the connections API.
    db.update_mcp_server(
        org_id,
        row.id.uuid(),
        UpdateMcpServer {
            status: Some("disabled".to_string()),
            ..Default::default()
        },
    )
    .await
    .context("failed to disable plugin OAuth anchor")?;

    Ok(row.id.uuid())
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::capabilities::{CapabilityStatus, DeclarativeCapabilityDefinition};
    use everruns_core::{ScopedMcpServer, ScopedMcpServers};
    use std::sync::Arc;

    fn oauth_definition(server_name: &str, url: &str) -> DeclarativeCapabilityDefinition {
        let mut servers = ScopedMcpServers::new();
        servers.insert(
            server_name.to_string(),
            ScopedMcpServer {
                url: url.to_string(),
                auth_mode: McpServerAuthMode::OAuth,
                ..ScopedMcpServer::default()
            },
        );
        DeclarativeCapabilityDefinition {
            name: "resend".to_string(),
            display_name: None,
            description: "test".to_string(),
            status: CapabilityStatus::Available,
            icon: None,
            category: None,
            system_prompt: None,
            mcp_servers: Some(servers),
            skills: Vec::new(),
            files: Vec::new(),
            dependencies: Vec::new(),
            features: Vec::new(),
            risk_level: everruns_core::capabilities::RiskLevel::Low,
        }
    }

    #[tokio::test]
    async fn sync_creates_disabled_anchor_and_assigns_provider_id() {
        let db = Arc::new(StorageBackend::in_memory());
        let mut definition = oauth_definition("resend", "https://mcp.resend.com/mcp");
        sync_plugin_oauth_anchors(&db, 1, "resend", &mut definition)
            .await
            .unwrap();

        let server = definition
            .mcp_servers
            .as_ref()
            .unwrap()
            .get("resend")
            .unwrap();
        let provider = server.oauth_provider_id.as_deref().expect("provider id");
        assert!(provider.starts_with("mcp_oauth_"), "{provider}");

        let anchors = list_plugin_anchors(&db, 1, "resend").await.unwrap();
        let anchor = anchors.get("resend").expect("anchor row");
        assert_eq!(anchor.status, "disabled");
        assert_eq!(anchor.url, "https://mcp.resend.com/mcp");
        assert_eq!(
            provider,
            mcp_oauth_provider_id_for_uuid(anchor.id.uuid()).as_str()
        );
        // The anchor advertises OAuth (via the exact serde representation the
        // connections API reads) so it is listed as an OAuth provider.
        let settings: McpServerSettings =
            serde_json::from_value(anchor.settings.clone()).expect("settings round-trip");
        assert_eq!(settings.auth_mode, McpServerAuthMode::OAuth);
        assert!(anchor.settings.get(ANCHOR_KEY).is_some());
        assert_eq!(
            anchor.settings[CONNECTION_DISPLAY_NAME_KEY],
            serde_json::json!("Resend")
        );
    }

    #[tokio::test]
    async fn sync_is_idempotent_and_reuses_anchor() {
        let db = Arc::new(StorageBackend::in_memory());
        let mut first = oauth_definition("resend", "https://mcp.resend.com/mcp");
        sync_plugin_oauth_anchors(&db, 1, "resend", &mut first)
            .await
            .unwrap();
        let first_provider = first.mcp_servers.as_ref().unwrap()["resend"]
            .oauth_provider_id
            .clone();

        let mut second = oauth_definition("resend", "https://mcp.resend.com/mcp");
        sync_plugin_oauth_anchors(&db, 1, "resend", &mut second)
            .await
            .unwrap();
        let second_provider = second.mcp_servers.as_ref().unwrap()["resend"]
            .oauth_provider_id
            .clone();

        // Same provider id across updates → user connections survive.
        assert_eq!(first_provider, second_provider);
        assert_eq!(
            list_plugin_anchors(&db, 1, "resend").await.unwrap().len(),
            1
        );
    }

    #[tokio::test]
    async fn changed_url_rotates_provider_id() {
        let db = Arc::new(StorageBackend::in_memory());
        let mut first = oauth_definition("resend", "https://mcp.resend.com/mcp");
        sync_plugin_oauth_anchors(&db, 1, "resend", &mut first)
            .await
            .unwrap();
        let first_provider = first.mcp_servers.as_ref().unwrap()["resend"]
            .oauth_provider_id
            .clone();

        let mut retargeted = oauth_definition("resend", "https://attacker.example/mcp");
        sync_plugin_oauth_anchors(&db, 1, "resend", &mut retargeted)
            .await
            .unwrap();
        let retargeted_provider = retargeted.mcp_servers.as_ref().unwrap()["resend"]
            .oauth_provider_id
            .clone();

        assert_ne!(first_provider, retargeted_provider);
        assert_eq!(
            list_plugin_anchors(&db, 1, "resend").await.unwrap()["resend"].url,
            "https://attacker.example/mcp"
        );
    }

    #[tokio::test]
    async fn sync_removes_anchor_when_server_dropped() {
        let db = Arc::new(StorageBackend::in_memory());
        let mut definition = oauth_definition("resend", "https://mcp.resend.com/mcp");
        sync_plugin_oauth_anchors(&db, 1, "resend", &mut definition)
            .await
            .unwrap();

        let mut without_server = oauth_definition("resend", "https://mcp.resend.com/mcp");
        without_server.mcp_servers = None;
        sync_plugin_oauth_anchors(&db, 1, "resend", &mut without_server)
            .await
            .unwrap();

        assert!(
            list_plugin_anchors(&db, 1, "resend")
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn delete_removes_all_anchors() {
        let db = Arc::new(StorageBackend::in_memory());
        let mut definition = oauth_definition("resend", "https://mcp.resend.com/mcp");
        sync_plugin_oauth_anchors(&db, 1, "resend", &mut definition)
            .await
            .unwrap();

        delete_plugin_oauth_anchors(&db, 1, "resend").await.unwrap();
        assert!(
            list_plugin_anchors(&db, 1, "resend")
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn anchor_name_avoids_collision_with_existing_server() {
        let db = Arc::new(StorageBackend::in_memory());
        db.create_mcp_server(
            1,
            CreateMcpServerRow {
                name: "resend".to_string(),
                description: None,
                url: "https://other.example.com/mcp".to_string(),
                transport_type: "http".to_string(),
                api_key_encrypted: None,
                headers: None,
                settings: None,
            },
        )
        .await
        .unwrap();

        let mut definition = oauth_definition("resend", "https://mcp.resend.com/mcp");
        sync_plugin_oauth_anchors(&db, 1, "resend", &mut definition)
            .await
            .unwrap();

        let anchors = list_plugin_anchors(&db, 1, "resend").await.unwrap();
        let anchor = anchors.get("resend").expect("anchor row");
        assert_ne!(anchor.name, "resend");
    }
}
