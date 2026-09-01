use anyhow::Result;
use clap::Subcommand;
use serde_json::{Value, json};

use crate::commands::api::ApiClient;
use crate::commands::discovery::{DiscoveryCommands, Resource};
use crate::output::OutputFormat;

#[derive(Subcommand, Debug)]
pub enum KnowledgeBasesCommands {
    /// List knowledge bases
    List,
    /// Get a knowledge base by ID
    Get { id: String },
    /// Create a knowledge base
    Create {
        name: String,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        embedding_model_id: Option<String>,
    },
    /// Delete a knowledge base by ID
    Delete { id: String },
}

const KNOWLEDGE_BASES: Resource = Resource {
    path: "/v1/knowledge-bases",
    collection_key: "knowledge_bases",
    empty_message: "No knowledge bases found.",
    columns: &[
        ("ID", "id"),
        ("NAME", "name"),
        ("DESCRIPTION", "description"),
    ],
    detail_fields: &[
        ("ID", "id"),
        ("Name", "name"),
        ("Description", "description"),
    ],
};

pub async fn run(
    command: KnowledgeBasesCommands,
    api_url: &str,
    api_key: &str,
    org_id: Option<&str>,
    format: OutputFormat,
) -> Result<()> {
    match command {
        KnowledgeBasesCommands::List => {
            run_discovery(DiscoveryCommands::List, api_url, api_key, org_id, format).await
        }
        KnowledgeBasesCommands::Get { id } => {
            run_discovery(
                DiscoveryCommands::Get { id },
                api_url,
                api_key,
                org_id,
                format,
            )
            .await
        }
        KnowledgeBasesCommands::Create {
            name,
            description,
            embedding_model_id,
        } => {
            let body = create_request(name, description, embedding_model_id);
            let response = ApiClient::new(api_url, api_key, org_id)
                .post(KNOWLEDGE_BASES.path, Some(&body))
                .await?;
            format.print_value(&response);
            Ok(())
        }
        KnowledgeBasesCommands::Delete { id } => {
            let response = ApiClient::new(api_url, api_key, org_id)
                .delete(&super::discovery::resource_path(KNOWLEDGE_BASES.path, &id))
                .await?;
            format.print_value(&response);
            Ok(())
        }
    }
}

async fn run_discovery(
    command: DiscoveryCommands,
    api_url: &str,
    api_key: &str,
    org_id: Option<&str>,
    format: OutputFormat,
) -> Result<()> {
    super::discovery::run(command, api_url, api_key, org_id, format, KNOWLEDGE_BASES).await
}

fn create_request(
    name: String,
    description: Option<String>,
    embedding_model_id: Option<String>,
) -> Value {
    json!({
        "name": name,
        "description": description,
        "embedding_model_id": embedding_model_id,
    })
}

#[cfg(test)]
mod tests {
    use super::{KNOWLEDGE_BASES, create_request};
    use crate::commands::discovery::resource_path;
    use serde_json::json;

    #[test]
    fn create_request_matches_server_contract_with_optional_fields() {
        assert_eq!(
            create_request(
                "Runbooks".into(),
                Some("Operations".into()),
                Some("model-id".into())
            ),
            json!({
                "name": "Runbooks",
                "description": "Operations",
                "embedding_model_id": "model-id"
            })
        );
        assert_eq!(
            create_request("Runbooks".into(), None, None),
            json!({
                "name": "Runbooks",
                "description": null,
                "embedding_model_id": null
            })
        );
    }

    #[test]
    fn request_paths_match_versioned_server_routes() {
        assert_eq!(KNOWLEDGE_BASES.path, "/v1/knowledge-bases");
        assert_eq!(
            resource_path(KNOWLEDGE_BASES.path, "kb/id"),
            "/v1/knowledge-bases/kb%2Fid"
        );
    }
}
