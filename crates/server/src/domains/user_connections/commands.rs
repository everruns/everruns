use super::types::{ConnectionProviderInfo, UserConnectionInfo};
use crate::domains::common::*;
use everruns_core::{McpServerAuthMode, mcp_oauth_provider_id_for_uuid};
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Default, Deserialize, ToSchema, serde::Serialize)]
pub struct ListUserConnections {
    /// Optional provider ID filter.
    pub provider: Option<String>,
}

#[command(
    name = "list_user_connections",
    category = "connections",
    description = "List sanitized connection state for the current user. Returns provider identity and connection metadata, never credentials or tokens.",
    method = "GET",
    path = "/v1/user/connections"
)]
impl Command for ListUserConnections {
    type Output = Vec<UserConnectionInfo>;

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        // THREAT[TM-AGENT-017]: The caller carries management authority, not
        // credential ownership. Resolve the same org-scoped console self binding
        // as Settings; never accept a user ID or serialize credential rows.
        let management_user_id = ctx.caller.user_id.ok_or_else(|| {
            CommandError::forbidden("A signed-in user is required to inspect user connections")
        })?;
        let runtime_user = ctx
            .db
            .default_virtual_user(ctx.org_id(), management_user_id)
            .await?;
        if runtime_user.status != "active" {
            return Err(CommandError::forbidden("Runtime account is not active"));
        }
        let rows = ctx
            .db
            .list_virtual_user_connections(runtime_user.id)
            .await?;
        Ok(rows
            .into_iter()
            .filter(|row| {
                self.provider
                    .as_deref()
                    .is_none_or(|provider| row.provider.eq_ignore_ascii_case(provider))
            })
            .map(|row| UserConnectionInfo {
                provider: row.provider,
                connection_type: row.connection_type,
                provider_username: row.provider_username,
                connected_at: row.created_at,
                ui_link: "/settings/connections".to_string(),
            })
            .collect())
    }
}

#[derive(Debug, Default, Deserialize, ToSchema, serde::Serialize)]
pub struct ListConnectionProviders {
    /// Optional case-insensitive provider name/ID filter.
    pub search: Option<String>,
}

#[command(
    name = "list_connection_providers",
    category = "connections",
    description = "List connection providers available in the current organization. This reports provider availability, not whether the current user is connected.",
    method = "GET",
    path = "/v1/user/connections/providers"
)]
impl Command for ListConnectionProviders {
    type Output = Vec<ConnectionProviderInfo>;

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        // THREAT[TM-AGENT-017]: Provider discovery is org-scoped. OAuth anchor
        // rows expose only provider identity and display metadata, never their
        // stored OAuth registration details or user credentials.
        let mut providers = Vec::new();
        if let Some(registry) = &ctx.connector_registry {
            providers.extend(registry.list().into_iter().map(|provider| {
                ConnectionProviderInfo {
                    provider_id: provider.provider_id().to_string(),
                    display_name: provider.display_name().to_string(),
                    description: provider.description().to_string(),
                    connection_type: match provider.connection_type() {
                        everruns_contracts::connector::ConnectorType::OAuth => "oauth",
                        everruns_contracts::connector::ConnectorType::ApiKey => "api_key",
                    }
                    .to_string(),
                    ui_link: "/settings/connections".to_string(),
                }
            }));
        }

        let mcp_servers = ctx.db.list_mcp_servers(ctx.org_id(), None, false).await?;
        providers.extend(mcp_servers.into_iter().filter_map(|server| {
            let settings =
                crate::domains::mcp_servers::McpServerService::settings_from_row(&server);
            (settings.auth_mode == McpServerAuthMode::OAuth).then(|| ConnectionProviderInfo {
                provider_id: mcp_oauth_provider_id_for_uuid(server.id.uuid()),
                display_name: server.name,
                description: server.description.unwrap_or_else(|| {
                    "Authenticate this MCP provider for agent tool access".to_string()
                }),
                connection_type: "oauth".to_string(),
                ui_link: "/settings/connections".to_string(),
            })
        }));

        if let Some(search) = self.search.as_deref().map(str::to_ascii_lowercase) {
            providers.retain(|provider| {
                provider.provider_id.to_ascii_lowercase().contains(&search)
                    || provider.display_name.to_ascii_lowercase().contains(&search)
                    || provider.description.to_ascii_lowercase().contains(&search)
            });
        }
        providers.sort_by(|left, right| {
            left.display_name
                .to_ascii_lowercase()
                .cmp(&right.display_name.to_ascii_lowercase())
                .then_with(|| left.provider_id.cmp(&right.provider_id))
        });
        providers.dedup_by(|left, right| left.provider_id == right.provider_id);
        Ok(providers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageBackend;
    use crate::storage::models::CreateUserConnectionRow;
    use everruns_core::capabilities::{CapabilityStatus, DeclarativeCapabilityDefinition};
    use everruns_core::{
        Caller, CapabilityMcpServer, CapabilityMcpServers, McpServerActsAs, McpServerAuthMode,
        OrgRole, ScopedMcpServer,
    };
    use std::sync::Arc;
    use uuid::Uuid;

    fn caller(org_id: i64, user_id: Uuid) -> Caller {
        Caller {
            org_id,
            org_public_id: everruns_core::organization::org_public_id_from_internal(org_id),
            user_id: Some(user_id),
            role: OrgRole::Member,
            is_platform_user: false,
            is_internal: false,
        }
    }

    async fn management_user(db: &StorageBackend, org_id: i64) -> Uuid {
        let id = db.create_test_user(Uuid::now_v7()).await;
        db.add_organization_member(org_id, id, "member")
            .await
            .expect("add management account to org");
        id
    }

    async fn seed_connection(db: &StorageBackend, user_id: Uuid, provider: &str) {
        db.upsert_user_connection(CreateUserConnectionRow {
            user_id,
            provider: provider.to_string(),
            connection_type: "oauth".to_string(),
            provider_user_id: Some("external-secret-id".to_string()),
            provider_username: Some("visible-user".to_string()),
            access_token_encrypted: Some(vec![1, 2, 3]),
            refresh_token_encrypted: Some(vec![4, 5, 6]),
            scopes: Some("send:email".to_string()),
            expires_at: None,
            installation_id: None,
            provider_metadata: Some(serde_json::json!({"private": "metadata"})),
        })
        .await
        .expect("seed connection");
    }

    #[tokio::test]
    async fn user_connections_are_current_user_scoped_and_secret_free() {
        let db = Arc::new(StorageBackend::test_database());
        let management = management_user(&db, 7).await;
        let current_user = db
            .default_virtual_user(7, management)
            .await
            .unwrap()
            .id
            .uuid();
        let other_user = db.create_test_virtual_user(7).await.uuid();
        db.add_organization_member(8, management, "member")
            .await
            .unwrap();
        let other_org_user = db
            .default_virtual_user(8, management)
            .await
            .unwrap()
            .id
            .uuid();
        assert_ne!(current_user, management);
        seed_connection(&db, current_user, "resend").await;
        seed_connection(&db, other_user, "other-provider").await;
        seed_connection(&db, other_org_user, "other-org-provider").await;
        let ctx = Ctx::minimal_for_test(caller(7, management), db.clone(), None);

        let output = ListUserConnections::default()
            .execute(&ctx)
            .await
            .expect("list connections");
        let json = serde_json::to_value(&output).expect("serialize output");

        assert_eq!(output.len(), 1);
        assert_eq!(output[0].provider, "resend");
        let encoded = json.to_string();
        for forbidden in ["token", "external-secret-id", "send:email", "metadata"] {
            assert!(
                !encoded.contains(forbidden),
                "leaked {forbidden}: {encoded}"
            );
        }
        let other_org_ctx = Ctx::minimal_for_test(caller(8, management), db, None);
        let other_org_output = ListUserConnections::default()
            .execute(&other_org_ctx)
            .await
            .expect("list connections in second org");
        assert_eq!(other_org_output.len(), 1);
        assert_eq!(other_org_output[0].provider, "other-org-provider");
    }

    #[tokio::test]
    async fn connection_listing_rejects_an_archived_runtime_account() {
        let db = Arc::new(StorageBackend::test_database());
        let management = management_user(&db, 7).await;
        let runtime_user = db.default_virtual_user(7, management).await.unwrap();
        seed_connection(&db, runtime_user.id.uuid(), "github").await;
        db.delete_virtual_user(7, runtime_user.id).await.unwrap();
        let ctx = Ctx::minimal_for_test(caller(7, management), db, None);
        let error = ListUserConnections::default()
            .execute(&ctx)
            .await
            .expect_err("archived runtime account must not expose grants");
        assert!(matches!(error.kind, CommandErrorKind::Forbidden(_)));
    }

    #[tokio::test]
    async fn connection_listing_requires_a_user_principal() {
        let db = Arc::new(StorageBackend::test_database());
        let ctx = Ctx::minimal_for_test(Caller::internal(7), db, None);
        let error = ListUserConnections::default()
            .execute(&ctx)
            .await
            .expect_err("internal caller without user must be rejected");
        assert!(matches!(error.kind, CommandErrorKind::Forbidden(_)));
    }

    #[tokio::test]
    async fn plugin_oauth_provider_and_current_user_connection_are_independent() {
        let db = Arc::new(StorageBackend::test_database());
        let management = management_user(&db, 7).await;
        let user_id = db
            .default_virtual_user(7, management)
            .await
            .unwrap()
            .id
            .uuid();
        let mut servers = CapabilityMcpServers::new();
        servers.insert(
            "resend".to_string(),
            CapabilityMcpServer::new(
                ScopedMcpServer {
                    url: "https://mcp.resend.com/mcp".to_string(),
                    auth_mode: McpServerAuthMode::OAuth,
                    ..ScopedMcpServer::default()
                },
                McpServerActsAs::User,
            ),
        );
        let mut definition = DeclarativeCapabilityDefinition {
            name: "resend".to_string(),
            display_name: Some("Resend".to_string()),
            description: "Email via Resend".to_string(),
            status: CapabilityStatus::Available,
            mcp_servers: Some(servers),
            ..DeclarativeCapabilityDefinition::default()
        };
        crate::domains::plugins::oauth_anchor::sync_plugin_oauth_anchors(
            &db,
            7,
            "resend",
            &mut definition,
        )
        .await
        .expect("create plugin OAuth anchor");
        let provider_id = definition.mcp_servers.as_ref().unwrap()["resend"]
            .oauth_provider_id
            .clone()
            .expect("provider id");
        let ctx = Ctx::minimal_for_test(caller(7, management), db.clone(), None);

        let providers = ListConnectionProviders {
            search: Some("resend".to_string()),
        }
        .execute(&ctx)
        .await
        .expect("list providers");
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].provider_id, provider_id);
        assert!(
            ListUserConnections::default()
                .execute(&ctx)
                .await
                .expect("list disconnected state")
                .is_empty(),
            "provider availability must not imply a user connection"
        );

        seed_connection(&db, user_id, &provider_id).await;
        let connections = ListUserConnections {
            provider: Some(provider_id.clone()),
        }
        .execute(&ctx)
        .await
        .expect("list connected state");
        assert_eq!(connections.len(), 1);
        assert_eq!(connections[0].provider, provider_id);
    }
}
