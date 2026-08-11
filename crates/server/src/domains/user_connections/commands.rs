use super::types::{ConnectionProviderInfo, UserConnectionInfo};
use crate::domains::common::*;
use everruns_core::{McpServerAuthMode, mcp_oauth_provider_id_for_uuid};
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ListUserConnections {
    /// Optional provider ID filter.
    pub provider: Option<String>,
}

impl Command for ListUserConnections {
    type Output = Vec<UserConnectionInfo>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_user_connections",
            category: "connections",
            description: "List sanitized connection state for the current user. Returns provider identity and connection metadata, never credentials or tokens.",
            method: "GET",
            path: "/v1/user/connections",
        }
    }

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        // THREAT[TM-AGENT-017]: Connection state is resolved from the owning
        // Platform Chat caller and projected into a secret-free DTO. Never
        // accept a user ID parameter or serialize the storage row directly.
        let user_id = ctx.caller.user_id.ok_or_else(|| {
            CommandError::forbidden("A signed-in user is required to inspect user connections")
        })?;
        let rows = ctx
            .db
            .list_user_connections(user_id)
            .await
            .map_err(classify_anyhow)?;
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

inventory::submit! { CommandDescriptor::of::<ListUserConnections>() }

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ListConnectionProviders {
    /// Optional case-insensitive provider name/ID filter.
    pub search: Option<String>,
}

impl Command for ListConnectionProviders {
    type Output = Vec<ConnectionProviderInfo>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_connection_providers",
            category: "connections",
            description: "List connection providers available in the current organization. This reports provider availability, not whether the current user is connected.",
            method: "GET",
            path: "/v1/user/connections/providers",
        }
    }

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
                        everruns_platform::connector::ConnectorType::OAuth => "oauth",
                        everruns_platform::connector::ConnectorType::ApiKey => "api_key",
                    }
                    .to_string(),
                    ui_link: "/settings/connections".to_string(),
                }
            }));
        }

        let mcp_servers = ctx
            .db
            .list_mcp_servers(ctx.org_id(), None, false)
            .await
            .map_err(classify_anyhow)?;
        providers.extend(mcp_servers.into_iter().filter_map(|server| {
            let settings = crate::domains::mcp_servers::queries::settings_from_row(&server);
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

inventory::submit! { CommandDescriptor::of::<ListConnectionProviders>() }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageBackend;
    use crate::storage::models::CreateUserConnectionRow;
    use everruns_core::capabilities::{CapabilityStatus, DeclarativeCapabilityDefinition};
    use everruns_core::{Caller, McpServerAuthMode, OrgRole, ScopedMcpServer, ScopedMcpServers};
    use std::sync::Arc;
    use uuid::Uuid;

    fn caller(org_id: i64, user_id: Uuid) -> Caller {
        Caller {
            org_id,
            org_public_id: everruns_core::organization::org_public_id_from_internal(org_id),
            user_id: Some(user_id),
            role: OrgRole::Admin,
            is_platform_user: false,
            is_internal: false,
        }
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
        let db = Arc::new(StorageBackend::in_memory());
        let current_user = Uuid::new_v4();
        let other_user = Uuid::new_v4();
        seed_connection(&db, current_user, "resend").await;
        seed_connection(&db, other_user, "other-provider").await;
        let ctx = Ctx::minimal_for_test(caller(7, current_user), db, None);

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
    }

    #[tokio::test]
    async fn connection_listing_requires_a_user_principal() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = Ctx::minimal_for_test(Caller::internal(7), db, None);
        let error = ListUserConnections::default()
            .execute(&ctx)
            .await
            .expect_err("internal caller without user must be rejected");
        assert!(matches!(error.kind, CommandErrorKind::Forbidden(_)));
    }

    #[tokio::test]
    async fn plugin_oauth_provider_and_current_user_connection_are_independent() {
        let db = Arc::new(StorageBackend::in_memory());
        let user_id = Uuid::new_v4();
        let mut servers = ScopedMcpServers::new();
        servers.insert(
            "resend".to_string(),
            ScopedMcpServer {
                url: "https://mcp.resend.com/mcp".to_string(),
                auth_mode: McpServerAuthMode::OAuth,
                ..ScopedMcpServer::default()
            },
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
        let ctx = Ctx::minimal_for_test(caller(7, user_id), db.clone(), None);

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
