use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Subcommand;
use serde_json::{Value, json};

use crate::commands::api::ApiClient;
use crate::commands::discovery::{DiscoveryCommands, Resource};
use crate::output::OutputFormat;

#[derive(Subcommand, Debug)]
pub enum SkillsCommands {
    /// List skills
    List,
    /// Get a skill by ID
    Get { id: String },
    /// Create a skill from a local SKILL.md file
    Create { path: PathBuf },
    /// Delete a skill by ID
    Delete { id: String },
}

const SKILLS: Resource = Resource {
    path: "/v1/skills",
    collection_key: "skills",
    empty_message: "No skills found.",
    columns: &[
        ("ID", "id"),
        ("NAME", "name"),
        ("DESCRIPTION", "description"),
    ],
    detail_fields: &[
        ("ID", "id"),
        ("Name", "name"),
        ("Description", "description"),
        ("Version", "version"),
    ],
};

pub async fn run(
    command: SkillsCommands,
    api_url: &str,
    api_key: &str,
    org_id: Option<&str>,
    format: OutputFormat,
) -> Result<()> {
    match command {
        SkillsCommands::List => {
            run_discovery(DiscoveryCommands::List, api_url, api_key, org_id, format).await
        }
        SkillsCommands::Get { id } => {
            run_discovery(
                DiscoveryCommands::Get { id },
                api_url,
                api_key,
                org_id,
                format,
            )
            .await
        }
        SkillsCommands::Create { path } => {
            let skill_md = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read SKILL.md at {}", path.display()))?;
            let body = create_request(&skill_md);
            let response = ApiClient::new(api_url, api_key, org_id)
                .post(SKILLS.path, Some(&body))
                .await?;
            format.print_value(&response);
            Ok(())
        }
        SkillsCommands::Delete { id } => {
            let response = ApiClient::new(api_url, api_key, org_id)
                .delete(&super::discovery::resource_path(SKILLS.path, &id))
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
    super::discovery::run(command, api_url, api_key, org_id, format, SKILLS).await
}

fn create_request(skill_md: &str) -> Value {
    json!({ "skill_md": skill_md })
}

#[cfg(test)]
mod tests {
    use super::{SKILLS, create_request};
    use crate::commands::discovery::resource_path;
    use serde_json::json;

    #[test]
    fn create_request_preserves_exact_skill_md_content() {
        let content = "---\nname: example\n---\n\n# Example\n";
        assert_eq!(create_request(content), json!({ "skill_md": content }));
    }

    #[test]
    fn request_paths_match_versioned_server_routes() {
        assert_eq!(SKILLS.path, "/v1/skills");
        assert_eq!(
            resource_path(SKILLS.path, "skill/id"),
            "/v1/skills/skill%2Fid"
        );
    }
}
