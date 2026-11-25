// A person's own plugins: what they add or turn off for themselves.
//
// Design: knowledge/integrations/plugins.md ("Personal plugins and Work Chat").
//
// Decisions:
// - A person only ever picks from the plugins the organization approved
//   (installed and active). Approving stays an admin's decision.
// - Choices are keyed by the person's user principal, so Settings > Personal >
//   Plugins and Work Chat show the same switches. A Work Chat visitor who is
//   not an organization member is keyed by their own principal.
// - In Work Chat the channel decides what a choice may do
//   (`WorkChatPlugins`): add plugins the agent does not come with, turn off
//   ones it does, both, or neither. Choices apply to new conversations as
//   session-level refs: `plugin:{id}` adds, `plugin:{id}` with
//   `{"enabled": false}` turns the agent's off when the layers merge.
// - A plugin the agent cannot run (its environment) is listed with the
//   reason and never added.

use crate::domains::agent_channels::record::work_chat::WorkChatPlugins;
use crate::domains::common::CommandError;
use crate::storage::{PluginInstallRow, StorageBackend};
use everruns_contracts::CapabilityRef as AgentCapabilityConfig;
use everruns_contracts::runtime::capabilities::PluginEnvironment;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use utoipa::ToSchema;
use uuid::Uuid;

/// Where a plugin in a person's list comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PluginOrigin {
    /// The agent comes with it.
    Agent,
    /// The person added it.
    Added,
    /// Approved, not in use: the person could add it.
    Available,
}

/// One approved plugin as a person sees it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PluginOption {
    /// Installed plugin id (`plugin_…`).
    #[schema(example = "plugin_01933b5a00007000800000000000001")]
    pub id: String,
    /// Name from the plugin manifest.
    #[schema(example = "stend")]
    pub name: String,
    /// Name to show people, when the manifest gives one.
    #[schema(example = "Stend")]
    pub display_name: Option<String>,
    /// What the plugin does.
    #[schema(example = "Publish web pages from the conversation")]
    pub description: String,
    /// Installed version.
    #[schema(example = "1.2.0")]
    pub version: Option<String>,
    /// What the plugin needs from the agent.
    #[schema(value_type = String, example = "workspace")]
    pub environment: PluginEnvironment,
    pub origin: PluginOrigin,
    /// Whether it is in this person's new conversations.
    #[schema(example = true)]
    pub enabled: bool,
    /// Whether the agent provides what the plugin needs. Always true outside
    /// an agent (Settings > Personal > Plugins).
    #[schema(example = true)]
    pub fits: bool,
    /// An admin still has to choose how one of its MCP servers signs in.
    #[schema(example = false)]
    pub needs_admin_setup: bool,
}

/// A person's plugins in one Work Chat channel.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WorkChatPluginList {
    /// The channel lets people add approved plugins.
    #[schema(example = true)]
    pub allow_adding: bool,
    /// The channel lets people turn off the agent's plugins.
    #[schema(example = false)]
    pub allow_turning_off: bool,
    /// The agent's plugins, the ones this person added, and ones they could add.
    pub plugins: Vec<PluginOption>,
}

/// Request body for a person's choice about one plugin.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct SetPluginChoice {
    /// In this person's new conversations, or not.
    #[schema(example = true)]
    pub enabled: bool,
}

/// The principal a person's choices are stored under: their user principal
/// when the principal resolves to a user who has one, otherwise the principal
/// itself (a Work Chat visitor outside the organization).
pub async fn choice_owner(
    db: &StorageBackend,
    org_id: i64,
    principal_id: everruns_contracts::typed_id::PrincipalId,
) -> anyhow::Result<Uuid> {
    let resolved = db
        .get_principal(org_id, principal_id)
        .await?
        .and_then(|row| row.resolved_user_id);
    if let Some(user_id) = resolved
        && let Some(row) = db.get_principal_by_subject(org_id, "user", user_id).await?
    {
        return Ok(row.id.uuid());
    }
    Ok(principal_id.uuid())
}

/// The plugin-relevant refs of an agent: its effective harness's and its
/// own, unexpanded.
pub async fn agent_refs(
    db: &StorageBackend,
    org_id: i64,
    harness_id: everruns_contracts::typed_id::HarnessId,
    agent_internal_id: Uuid,
) -> anyhow::Result<Vec<AgentCapabilityConfig>> {
    let mut refs = crate::domains::harnesses::queries::resolve_effective(db, org_id, harness_id)
        .await?
        .map(|harness| harness.capabilities)
        .unwrap_or_default();
    refs.extend(
        db.get_agent_capabilities(agent_internal_id)
            .await?
            .into_iter()
            .map(|row| AgentCapabilityConfig::with_config(row.capability_id, row.config)),
    );
    Ok(refs)
}

/// What a person's choice about `plugin_id` may do in a channel with these
/// options, or why not.
pub fn check_channel_choice(
    list: &[PluginOption],
    settings: WorkChatPlugins,
    plugin_id: &str,
    enabled: bool,
) -> Result<(), CommandError> {
    let option = list
        .iter()
        .find(|option| option.id == plugin_id)
        .ok_or_else(|| CommandError::not_found("Plugin"))?;
    match option.origin {
        PluginOrigin::Agent if !settings.allow_turning_off => Err(CommandError::forbidden(
            "This chat does not let people turn off the agent's plugins",
        )),
        PluginOrigin::Agent => Ok(()),
        _ if !settings.allow_adding => Err(CommandError::forbidden(
            "This chat does not let people add plugins",
        )),
        _ if enabled && !option.fits => Err(CommandError::bad_request(format!(
            "{} needs {}, which this agent does not provide",
            option.name,
            option.environment.level_name()
        ))),
        _ if enabled && option.needs_admin_setup => Err(CommandError::bad_request(format!(
            "{} is waiting for an admin to choose how it signs in",
            option.name
        ))),
        _ => Ok(()),
    }
}

/// The principal an organization user's choices are stored under.
pub async fn user_owner(
    db: &std::sync::Arc<StorageBackend>,
    org_id: i64,
    user_id: Uuid,
) -> anyhow::Result<Uuid> {
    let row = crate::domains::users::principal::PrincipalService::new(db.clone())
        .ensure_user_principal(org_id, user_id)
        .await?;
    Ok(row.id.uuid())
}

/// The installs an agent with these (unhydrated, merged harness and agent)
/// refs comes with: the ones it names, or every approved one that needs no
/// admin setup when it takes "all approved".
fn agent_plugins(refs: &[AgentCapabilityConfig], installs: &[PluginInstallRow]) -> HashSet<String> {
    let mut ids: HashSet<String> = refs
        .iter()
        .filter(|cap| !everruns_contracts::is_all_approved_plugins(cap.capability_id()))
        .filter_map(|cap| everruns_contracts::parse_plugin_capability_id(cap.capability_id()))
        .map(str::to_string)
        .collect();
    if refs
        .iter()
        .any(|cap| everruns_contracts::is_all_approved_plugins(cap.capability_id()))
    {
        ids.extend(
            installs
                .iter()
                .filter(|install| !needs_admin_setup(install))
                .map(|install| install.public_id.clone()),
        );
    }
    ids
}

fn needs_admin_setup(install: &PluginInstallRow) -> bool {
    !crate::domains::capabilities::queries::legacy_mcp_servers_requiring_identity(
        &install.definition,
    )
    .is_empty()
}

fn text(definition: &serde_json::Value, key: &str) -> Option<String> {
    definition
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Every approved plugin with this person's choice. With `agent` (its
/// merged harness and agent refs, plus the channel's options), the agent's
/// plugins and what it can run are taken into account.
pub async fn options(
    db: &StorageBackend,
    org_id: i64,
    owner: Uuid,
    agent: Option<(&[AgentCapabilityConfig], WorkChatPlugins)>,
) -> anyhow::Result<Vec<PluginOption>> {
    let installs = db.list_active_plugin_installs(org_id).await?;
    let choices: HashMap<Uuid, bool> = db
        .list_plugin_choices(org_id, owner)
        .await?
        .into_iter()
        .map(|choice| (choice.plugin_install_id, choice.enabled))
        .collect();
    let (agent_ids, provided, settings) = match agent {
        Some((refs, settings)) => (
            agent_plugins(refs, &installs),
            Some(PluginEnvironment::provided_by(
                refs.iter().map(|cap| cap.capability_id()),
            )),
            settings,
        ),
        None => (
            HashSet::new(),
            None,
            WorkChatPlugins {
                allow_adding: true,
                allow_turning_off: true,
            },
        ),
    };
    let mut options: Vec<PluginOption> = installs
        .iter()
        .map(|install| {
            let requirement = super::queries::install_requirement(install);
            let choice = choices.get(&install.id).copied();
            let in_agent = agent_ids.contains(&install.public_id);
            let (origin, enabled) = if in_agent {
                let enabled = if settings.allow_turning_off {
                    choice.unwrap_or(true)
                } else {
                    true
                };
                (PluginOrigin::Agent, enabled)
            } else if settings.allow_adding && choice == Some(true) {
                (PluginOrigin::Added, true)
            } else {
                (PluginOrigin::Available, false)
            };
            PluginOption {
                id: install.public_id.clone(),
                name: install.name.clone(),
                display_name: text(&install.definition, "display_name"),
                description: text(&install.definition, "description").unwrap_or_default(),
                version: install.version.clone(),
                environment: requirement.environment,
                origin,
                enabled,
                fits: provided
                    .is_none_or(|provided| requirement.environment.satisfied_by(provided)),
                needs_admin_setup: needs_admin_setup(install),
            }
        })
        .collect();
    // The person's own first, then what they could add.
    options.sort_by_key(|option| match option.origin {
        PluginOrigin::Agent => 0,
        PluginOrigin::Added => 1,
        PluginOrigin::Available => 2,
    });
    Ok(options)
}

/// Record a person's choice about the approved plugin `plugin_id`.
pub async fn set_choice(
    db: &StorageBackend,
    org_id: i64,
    owner: Uuid,
    plugin_id: &str,
    enabled: bool,
) -> Result<(), CommandError> {
    let install = db
        .get_plugin_install_by_public_id(org_id, plugin_id)
        .await?
        .filter(|install| install.status == "active")
        .ok_or_else(|| CommandError::not_found("Plugin"))?;
    db.set_plugin_choice(org_id, owner, install.id, enabled)
        .await?;
    Ok(())
}

/// The session-level refs a person's choices add to a new conversation.
pub fn session_capabilities(options: &[PluginOption]) -> Vec<AgentCapabilityConfig> {
    options
        .iter()
        .filter_map(|option| {
            let id = everruns_contracts::plugin_capability_id(&option.id);
            match option.origin {
                PluginOrigin::Added if option.fits && !option.needs_admin_setup => {
                    Some(AgentCapabilityConfig::new(id))
                }
                PluginOrigin::Agent if !option.enabled => Some(AgentCapabilityConfig::with_config(
                    id,
                    serde_json::json!({"enabled": false}),
                )),
                _ => None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn option(id: &str, origin: PluginOrigin, enabled: bool, fits: bool) -> PluginOption {
        PluginOption {
            id: id.into(),
            name: id.into(),
            display_name: None,
            description: String::new(),
            version: None,
            environment: PluginEnvironment::Workspace,
            origin,
            enabled,
            fits,
            needs_admin_setup: false,
        }
    }

    #[test]
    fn choices_become_session_refs() {
        let refs = session_capabilities(&[
            option("plugin_a", PluginOrigin::Added, true, true),
            option("plugin_b", PluginOrigin::Added, true, false),
            option("plugin_c", PluginOrigin::Agent, false, true),
            option("plugin_d", PluginOrigin::Agent, true, true),
            option("plugin_e", PluginOrigin::Available, false, true),
        ]);
        let ids: Vec<_> = refs.iter().map(|cap| cap.capability_id()).collect();
        assert_eq!(ids, ["plugin:plugin_a", "plugin:plugin_c"]);
        assert!(everruns_core::capabilities::is_plugin_turned_off(
            refs[1].config_value()
        ));
    }
}

// ============================================================================
// Commands: Settings > Personal > Plugins
// ============================================================================

use crate::domains::common::*;
use everruns_core::{Policy, Rule};

/// Only the signed-in person's own choices are read or written; an
/// organization role never reaches another person's.
pub const OWN_PLUGINS: Policy = Policy {
    id: "plugins.self",
    rules: &[Rule::IsPlatformUser],
};

async fn caller_owner(ctx: &Ctx) -> Result<Uuid, CommandError> {
    let user_id = ctx
        .caller
        .user_id
        .ok_or_else(|| CommandError::forbidden("Personal plugins need a signed-in person"))?;
    user_owner(&ctx.db, ctx.org_id(), user_id)
        .await
        .map_err(CommandError::internal)
}

/// List the organization's approved plugins with your own choices.
#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct ListMyPlugins {}

#[command(
    name = "list_my_plugins",
    category = "plugins",
    description = "List the organization's approved plugins with your own choices: added for yourself, or not.",
    method = "GET",
    path = "/v1/platform/users/me/plugins",
    policy = OWN_PLUGINS,
    cli = CliRoute::new(&["plugins", "mine"], "list").with_examples(&[CliExample::new("See which approved plugins you added for yourself", "everruns plugins mine list",)]),
)]
impl Command for ListMyPlugins {
    type Output = Vec<PluginOption>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<PluginOption>, CommandError> {
        let owner = caller_owner(ctx).await?;
        Ok(options(&ctx.db, ctx.org_id(), owner, None).await?)
    }
}

/// Add an approved plugin for yourself, or remove it.
#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct SetMyPluginChoice {
    /// Installed plugin id (`plugin_…`).
    pub id: String,
    /// In your new conversations, or not.
    pub enabled: bool,
}

#[command(
    name = "set_my_plugin_choice",
    category = "plugins",
    description = "Add an approved plugin to your own conversations, or take it out.",
    method = "PUT",
    path = "/v1/platform/users/me/plugins/{id}",
    policy = OWN_PLUGINS,
    positional = "id",
    cli = CliRoute::new(&["plugins", "mine"], "set").with_args(&[CliArg::new("id").at(1)]).with_examples(&[CliExample::new("Add Visti to your own conversations", "everruns plugins mine set plugin_01h9 --enabled true",)]),
)]
impl Command for SetMyPluginChoice {
    type Output = Vec<PluginOption>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<PluginOption>, CommandError> {
        let owner = caller_owner(ctx).await?;
        set_choice(&ctx.db, ctx.org_id(), owner, &self.id, self.enabled).await?;
        Ok(options(&ctx.db, ctx.org_id(), owner, None).await?)
    }
}

#[cfg(test)]
mod channel_tests {
    use super::*;

    fn option(id: &str, origin: PluginOrigin, fits: bool, needs_admin_setup: bool) -> PluginOption {
        PluginOption {
            id: id.into(),
            name: id.into(),
            display_name: None,
            description: String::new(),
            version: None,
            environment: PluginEnvironment::Sandbox,
            origin,
            enabled: origin != PluginOrigin::Available,
            fits,
            needs_admin_setup,
        }
    }

    fn settings(allow_adding: bool, allow_turning_off: bool) -> WorkChatPlugins {
        WorkChatPlugins {
            allow_adding,
            allow_turning_off,
        }
    }

    #[test]
    fn a_locked_channel_refuses_every_change() {
        let list = [
            option("plugin_agent", PluginOrigin::Agent, true, false),
            option("plugin_free", PluginOrigin::Available, true, false),
        ];
        let locked = settings(false, false);
        assert!(check_channel_choice(&list, locked, "plugin_agent", false).is_err());
        assert!(check_channel_choice(&list, locked, "plugin_free", true).is_err());
    }

    #[test]
    fn each_option_opens_only_its_own_change() {
        let list = [
            option("plugin_agent", PluginOrigin::Agent, true, false),
            option("plugin_free", PluginOrigin::Available, true, false),
        ];
        assert!(check_channel_choice(&list, settings(false, true), "plugin_agent", false).is_ok());
        assert!(check_channel_choice(&list, settings(false, true), "plugin_free", true).is_err());
        assert!(check_channel_choice(&list, settings(true, false), "plugin_free", true).is_ok());
        assert!(check_channel_choice(&list, settings(true, false), "plugin_agent", false).is_err());
    }

    #[test]
    fn a_plugin_the_agent_cannot_run_cannot_be_added_but_can_be_removed() {
        let list = [
            option("plugin_big", PluginOrigin::Added, false, false),
            option("plugin_wait", PluginOrigin::Available, true, true),
        ];
        let open = settings(true, true);
        assert!(check_channel_choice(&list, open, "plugin_big", true).is_err());
        assert!(check_channel_choice(&list, open, "plugin_big", false).is_ok());
        assert!(check_channel_choice(&list, open, "plugin_wait", true).is_err());
        assert!(check_channel_choice(&list, open, "plugin_missing", true).is_err());
    }
}
