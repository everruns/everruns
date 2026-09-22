use uuid::Uuid;

use super::{SeedResult, seed_ids};
use crate::storage::{StorageBackend, models::CreateMcpServerRow};
use everruns_core::DEFAULT_ORG_ID;

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
];

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
