use uuid::Uuid;

use super::{SeedResult, seed_ids};
use crate::storage::{CreateMcpServerRow, StorageBackend};
use everruns_core::DEFAULT_ORG_ID;

/// GitHub's remote MCP server, backed by the agent's GitHub App.
const GITHUB_MCP: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000503);

struct SeedMcpServer {
    id: Uuid,
    name: &'static str,
    description: &'static str,
    url: &'static str,
    settings: Option<fn() -> serde_json::Value>,
}

const SEED_MCP_SERVERS: &[SeedMcpServer] = &[
    SeedMcpServer {
        id: seed_ids::MS_LEARN_MCP,
        name: "microsoft_learn",
        description: "Microsoft Learn documentation MCP server - search and retrieve Microsoft documentation",
        url: "https://learn.microsoft.com/api/mcp",
        settings: None,
    },
    SeedMcpServer {
        id: seed_ids::LINEAR_MCP,
        name: "linear",
        description: "Linear MCP server - search and update issues, projects, and comments",
        url: "https://mcp.linear.app/mcp",
        settings: Some(linear_mcp_settings),
    },
    SeedMcpServer {
        id: GITHUB_MCP,
        name: "github",
        description: "GitHub MCP server - read code, issues and pull requests, and review them. Acting as the agent, it uses the agent's GitHub App; acting as a person, their own GitHub login",
        url: "https://api.githubcopilot.com/mcp/",
        settings: Some(github_mcp_settings),
    },
];

/// User MCP servers D4: an agent attachment acting as `service` (or
/// `user_or_service` falling back to the agent) uses the agent's `github`
/// connection, its GitHub App installation, so the agent needs no second
/// GitHub login for MCP. A person acting as themselves still signs in through
/// this preset's OAuth client.
fn github_mcp_settings() -> serde_json::Value {
    serde_json::json!({
        "auth_mode": "oauth",
        "protocol_mode": "auto",
        "service_connection_provider": "github"
    })
}

fn linear_mcp_settings() -> serde_json::Value {
    serde_json::json!({
        "auth_mode": "oauth",
        "protocol_mode": "auto",
        "oauth": {
            "scope": "read,write",
            "service_authorization_params": {
                "actor": "app"
            }
        }
    })
}

pub(super) async fn seed_mcp_servers(db: &StorageBackend) -> anyhow::Result<SeedResult> {
    let mut result = SeedResult::default();

    for seed in SEED_MCP_SERVERS {
        let input = CreateMcpServerRow {
            name: seed.name.to_string(),
            description: Some(seed.description.to_string()),
            url: seed.url.to_string(),
            transport_type: "http".to_string(),
            api_key_encrypted: None,
            headers: None,
            settings: seed.settings.map(|settings| settings()),
        };

        match db
            .create_mcp_server_with_id(DEFAULT_ORG_ID, seed.id, input)
            .await?
        {
            Some(row) => {
                if row.created_at == row.updated_at {
                    tracing::info!(
                        name = seed.name,
                        id = %seed.id,
                        url = seed.url,
                        "Created seed MCP server"
                    );
                    result.created += 1;
                } else {
                    tracing::info!(
                        name = seed.name,
                        id = %seed.id,
                        url = seed.url,
                        "Updated seed MCP server"
                    );
                    result.updated += 1;
                }
            }
            None => {
                tracing::debug!(
                    name = seed.name,
                    id = %seed.id,
                    "MCP server up to date"
                );
                result.unchanged += 1;
            }
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::mcp_servers::McpServerService;

    #[tokio::test]
    async fn github_preset_is_backed_by_the_agents_github_app() {
        let db = StorageBackend::test_database();
        seed_mcp_servers(&db).await.unwrap();

        let row = db
            .get_mcp_server(DEFAULT_ORG_ID, GITHUB_MCP)
            .await
            .unwrap()
            .expect("GitHub MCP preset should be seeded");
        let settings = McpServerService::settings_from_row(&row);

        assert_eq!(row.name, "github");
        assert_eq!(
            settings.service_connection_provider.as_deref(),
            Some("github")
        );
        assert_eq!(settings.auth_mode, everruns_core::McpServerAuthMode::OAuth);
        // The pinned host is the one GitHub App installation tokens may reach.
        assert!(
            crate::domains::mcp_servers::connection_backed::token_may_reach("github", &row.url)
        );
    }
}
