use super::PLUGIN_MANAGE;
use super::lookup::get_install_by_public_id;
use super::queries as q;
use super::types::{InstalledPlugin, UpdateInstalledPluginRequest, UpdatePluginInstall};
use crate::domains::common::*;
use crate::kernel_imports::{McpServerActsAs, Policy};
use serde::Deserialize;
use utoipa::ToSchema;

fn apply_mcp_identity_choices(
    definition: &serde_json::Value,
    choices: &std::collections::BTreeMap<String, McpServerActsAs>,
) -> Result<serde_json::Value, CommandError> {
    if choices.is_empty() {
        return Err(CommandError::bad_request(
            "mcp_server_identities must contain at least one choice",
        ));
    }

    let unresolved: std::collections::HashSet<_> =
        crate::domains::capabilities::queries::legacy_mcp_servers_requiring_identity(definition)
            .into_iter()
            .collect();
    let mut updated = definition.clone();
    let servers = updated
        .get_mut("mcp_servers")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| CommandError::bad_request("Plugin has no MCP servers"))?;

    for (name, acts_as) in choices {
        if acts_as.is_none() {
            return Err(CommandError::bad_request(format!(
                "MCP server '{name}' must act as user or service"
            )));
        }
        if !unresolved.contains(name) {
            return Err(CommandError::bad_request(format!(
                "MCP server '{name}' does not require an identity choice"
            )));
        }
        let server = servers
            .get_mut(name)
            .and_then(serde_json::Value::as_object_mut)
            .ok_or_else(|| {
                CommandError::bad_request(format!("MCP server '{name}' is not an object"))
            })?;
        server.insert(
            "actsAs".to_string(),
            serde_json::Value::String(acts_as.to_string()),
        );
    }

    Ok(updated)
}

/// Update an installed plugin's status or resolve legacy MCP identities.
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
            description: "Update an installed plugin's status or MCP acting identities.",
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
            .is_some_and(|status| !matches!(status, "active" | "disabled"))
        {
            return Err(CommandError::bad_request(
                "status must be 'active' or 'disabled'",
            ));
        }
        let definition = self
            .req
            .mcp_server_identities
            .as_ref()
            .map(|choices| apply_mcp_identity_choices(&existing.definition, choices))
            .transpose()?;
        let effective_definition = definition.as_ref().unwrap_or(&existing.definition);
        if self.req.status.as_deref() == Some("active") {
            let unresolved =
                crate::domains::capabilities::queries::legacy_mcp_servers_requiring_identity(
                    effective_definition,
                );
            if !unresolved.is_empty() {
                return Err(CommandError::bad_request(format!(
                    "Plugin needs an acting identity for MCP server(s): {}",
                    unresolved.join(", ")
                )));
            }
        }
        let updated = ctx
            .db
            .update_plugin_install(
                ctx.org_id(),
                existing.id,
                UpdatePluginInstall {
                    status: self.req.status,
                    definition,
                    ..Default::default()
                },
            )
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Installed plugin"))?;

        let marketplace = if let Some(marketplace_id) = updated.marketplace_id {
            ctx.db
                .get_plugin_marketplace(ctx.org_id(), marketplace_id)
                .await
                .map_err(classify_anyhow)?
        } else {
            None
        };

        Ok(q::row_to_installed_plugin(&updated, marketplace.as_ref()))
    }
}

inventory::submit! { CommandDescriptor::of::<PatchInstalledPlugin>() }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_plugin_identity_choices_are_explicit_and_targeted() {
        let stored = serde_json::json!({
            "name": "legacy",
            "description": "Legacy plugin",
            "mcp_servers": {
                "oauth": {
                    "url": "https://example.com/mcp",
                    "auth_mode": "oauth"
                },
                "service": {
                    "url": "https://example.com/service",
                    "auth_mode": "oauth"
                },
                "public": {
                    "url": "https://example.com/public"
                }
            }
        });
        let choices = std::collections::BTreeMap::from([
            ("oauth".to_string(), McpServerActsAs::User),
            ("service".to_string(), McpServerActsAs::Service),
        ]);

        let updated = apply_mcp_identity_choices(&stored, &choices).unwrap();

        assert_eq!(updated["mcp_servers"]["oauth"]["actsAs"], "user");
        assert_eq!(updated["mcp_servers"]["service"]["actsAs"], "service");
        assert!(
            updated["mcp_servers"]["public"].get("actsAs").is_none(),
            "the explicit management action must not rewrite unrelated legacy entries"
        );
        assert!(
            stored["mcp_servers"]["oauth"].get("actsAs").is_none(),
            "the caller retains the original row until persistence succeeds"
        );
    }

    #[test]
    fn legacy_plugin_identity_choices_reject_none_and_known_servers() {
        let stored = serde_json::json!({
            "mcp_servers": {
                "oauth": {"url": "https://example.com/mcp", "auth_mode": "oauth"},
                "known": {
                    "url": "https://example.com/known",
                    "auth_mode": "oauth",
                    "actsAs": "service"
                }
            }
        });

        for (name, acts_as, expected) in [
            (
                "oauth",
                McpServerActsAs::None,
                "must act as user or service",
            ),
            (
                "known",
                McpServerActsAs::Service,
                "does not require an identity choice",
            ),
        ] {
            let error = apply_mcp_identity_choices(
                &stored,
                &std::collections::BTreeMap::from([(name.to_string(), acts_as)]),
            )
            .unwrap_err();
            assert!(error.message().contains(expected), "{}", error.message());
        }
    }
}
