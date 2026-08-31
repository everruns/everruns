use anyhow::Result;
use clap::Subcommand;
use serde_json::{Value, json};

use crate::commands::api::ApiClient;
use crate::commands::discovery::{DiscoveryCommands, Resource};
use crate::output::OutputFormat;

#[derive(Subcommand, Debug)]
pub enum PluginsCommands {
    /// List installed plugins
    List,
    /// Get an installed plugin by ID
    Get { id: String },
    /// Install a plugin from a marketplace
    Install {
        marketplace_id: String,
        plugin_name: String,
    },
    /// Uninstall a plugin by ID
    Uninstall { id: String },
}

const PLUGINS: Resource = Resource {
    path: "/v1/plugins",
    collection_key: "plugins",
    empty_message: "No plugins found.",
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
    command: PluginsCommands,
    api_url: &str,
    api_key: &str,
    org_id: Option<&str>,
    format: OutputFormat,
) -> Result<()> {
    match command {
        PluginsCommands::List => {
            run_discovery(DiscoveryCommands::List, api_url, api_key, org_id, format).await
        }
        PluginsCommands::Get { id } => {
            run_discovery(
                DiscoveryCommands::Get { id },
                api_url,
                api_key,
                org_id,
                format,
            )
            .await
        }
        PluginsCommands::Install {
            marketplace_id,
            plugin_name,
        } => {
            let body = install_request(&marketplace_id, &plugin_name);
            let response = ApiClient::new(api_url, api_key, org_id)
                .post(PLUGINS.path, Some(&body))
                .await?;
            format.print_value(&response);
            Ok(())
        }
        PluginsCommands::Uninstall { id } => {
            let response = ApiClient::new(api_url, api_key, org_id)
                .delete(&super::discovery::resource_path(PLUGINS.path, &id))
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
    super::discovery::run(command, api_url, api_key, org_id, format, PLUGINS).await
}

fn install_request(marketplace_id: &str, plugin_name: &str) -> Value {
    json!({ "marketplace_id": marketplace_id, "plugin_name": plugin_name })
}

#[cfg(test)]
mod tests {
    use super::{PLUGINS, install_request};
    use crate::commands::discovery::resource_path;
    use serde_json::json;

    #[test]
    fn install_request_matches_server_contract() {
        assert_eq!(
            install_request("marketplace-id", "plugin-name"),
            json!({
                "marketplace_id": "marketplace-id",
                "plugin_name": "plugin-name"
            })
        );
    }

    #[test]
    fn request_paths_match_versioned_server_routes() {
        assert_eq!(PLUGINS.path, "/v1/plugins");
        assert_eq!(
            resource_path(PLUGINS.path, "plugin/id"),
            "/v1/plugins/plugin%2Fid"
        );
    }
}
