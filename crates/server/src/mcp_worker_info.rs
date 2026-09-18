use crate::domains::mcp_servers::McpServerResolved;
use crate::storage::StorageBackend;
use everruns_core::{ConnectionRequiredSubject, ConnectionRequiredSubjectKind, McpServerActsAs};
use everruns_platform::Agent;
use everruns_worker::mcp_executor::McpServerInfo;
use std::collections::HashMap;
use uuid::Uuid;

pub(crate) fn resolved_mcp_server_to_worker_info(
    resolved: McpServerResolved,
    secret_bindings: HashMap<String, Vec<everruns_mcp::McpSecretBinding>>,
) -> McpServerInfo {
    McpServerInfo {
        id: resolved.id,
        name: resolved.name,
        url: resolved.url,
        api_key: resolved.api_key,
        headers: resolved.headers,
        auth_mode: resolved.auth_mode,
        protocol_mode: resolved.protocol_mode,
        oauth_provider_id: resolved.oauth_provider_id,
        acts_as: resolved.acts_as,
        connection_subject: None,
        connection_setup_url: None,
        secret_bindings,
    }
}

pub(crate) async fn connection_setup_details(
    db: &StorageBackend,
    acts_as: McpServerActsAs,
    agent: Option<&Agent>,
    resolved_owner_user_id: Option<Uuid>,
) -> anyhow::Result<Option<(ConnectionRequiredSubject, String)>> {
    match acts_as {
        McpServerActsAs::Service => Ok(agent.map(|agent| {
            (
                ConnectionRequiredSubject {
                    kind: ConnectionRequiredSubjectKind::Agent,
                    name: agent
                        .display_name
                        .clone()
                        .unwrap_or_else(|| agent.name.clone()),
                },
                format!("/agents/{}?tab=mcp", agent.public_id),
            )
        })),
        McpServerActsAs::User => {
            let Some(user_id) = resolved_owner_user_id else {
                return Ok(None);
            };
            Ok(db.get_user(user_id).await?.map(|user| {
                (
                    ConnectionRequiredSubject {
                        kind: ConnectionRequiredSubjectKind::User,
                        name: user.name,
                    },
                    "/settings/connections".to_string(),
                )
            }))
        }
        McpServerActsAs::None => Ok(None),
    }
}

pub(crate) async fn apply_connection_setup_details(
    db: &StorageBackend,
    acts_as: McpServerActsAs,
    agent: Option<&Agent>,
    resolved_owner_user_id: Option<Uuid>,
    info: &mut McpServerInfo,
) -> anyhow::Result<()> {
    if let Some((subject, setup_url)) =
        connection_setup_details(db, acts_as, agent, resolved_owner_user_id).await?
    {
        info.connection_subject = Some(subject);
        info.connection_setup_url = Some(setup_url);
    }
    Ok(())
}
pub(crate) fn name_from_path(path: &str) -> String {
    if path == "/" {
        return "/".to_string();
    }
    std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::models::CreateUserRow;

    #[tokio::test]
    async fn user_connection_setup_uses_the_resolved_owner_name() {
        let db = StorageBackend::in_memory();
        let user = db
            .create_user(CreateUserRow {
                email: "ada@example.com".to_string(),
                name: "Ada Lovelace".to_string(),
                avatar_url: None,
                roles: vec![],
                password_hash: None,
                email_verified: true,
                auth_provider: None,
                auth_provider_id: None,
                external_id: None,
            })
            .await
            .expect("create user");

        let (subject, setup_url) =
            connection_setup_details(&db, McpServerActsAs::User, None, Some(user.id))
                .await
                .expect("resolve setup details")
                .expect("user details");

        assert_eq!(subject.kind, ConnectionRequiredSubjectKind::User);
        assert_eq!(subject.name, "Ada Lovelace");
        assert_eq!(setup_url, "/settings/connections");
    }
}
