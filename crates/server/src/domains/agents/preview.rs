// Agent preview resolves the same harness and agent overlays as execution.

use super::queries as q;
use crate::domains::common::*;
use crate::kernel_imports::{
    AgentCapabilityConfig, InitialFile, ScopedMcpServers, contracts::tool_types::ToolDefinition,
};
use everruns_contracts::typed_id::{AgentId, HarnessId};
use serde::Deserialize;
use utoipa::ToSchema;

// ============================================================================
// PreviewAgent
// ============================================================================

/// Preview the final agent shape with capabilities applied.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct PreviewAgent {
    /// Harness to layer beneath this draft. Omit to preview the agent layer alone.
    #[serde(default)]
    pub harness_id: Option<HarnessId>,
    #[serde(default)]
    pub initial_files: Vec<InitialFile>,
    pub system_prompt: Option<String>,
    #[serde(default)]
    #[schema(value_type = Vec<crate::records::CapabilityRefSchema>)]
    pub capabilities: Vec<AgentCapabilityConfig>,
    #[serde(default)]
    pub tools: Vec<ToolDefinition>,
    #[serde(default)]
    pub mcp_servers: ScopedMcpServers,
    /// Saved agent this draft edits. Enables checks that depend on the agent's
    /// channels and harness, which the draft itself does not carry.
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub agent_id: Option<AgentId>,
}

#[derive(Debug, serde::Serialize)]
pub struct AgentPreview {
    pub features: Vec<String>,
    pub initial_files: Vec<InitialFile>,
    pub system_prompt: String,
    pub tools: Vec<ToolDefinition>,
    /// Advisory tier-1 findings about the previewed config.
    pub findings: Vec<super::checks::Finding>,
}

#[command(
    name = "preview_agent",
    category = "agents",
    description = "Preview the final agent shape with capabilities applied.",
    method = "POST",
    path = "/v1/agents/preview",
    policy = crate::domains::agents::AGENT_VIEW,
    read_only = true,
    cli = CliRoute::new(&["agents"], "preview").with_examples(&[CliExample::new("See the prompt a draft configuration would produce, without creating it", "everruns agents preview --system-prompt 'Triage incoming issues'",)]),
)]
impl Command for PreviewAgent {
    type Output = AgentPreview;

    async fn execute(self, ctx: &Ctx) -> Result<AgentPreview, CommandError> {
        let authored_prompt = self.system_prompt.clone().unwrap_or_default();
        let draft = everruns_core::AgentConfigOverlay {
            system_prompt: self.system_prompt,
            capabilities: self.capabilities,
            initial_files: self.initial_files,
            mcp_servers: self.mcp_servers,
            ..Default::default()
        };
        // Use execution's merge semantics, including live parent inheritance and
        // agent config overrides, rather than reconstructing inheritance in the UI.
        let mut effective = if let Some(harness_id) = self.harness_id {
            // THREAT[TM-AUTHZ-008]: Inherited config has its own read permission.
            crate::domains::harnesses::HARNESS_VIEW
                .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
                .map_err(|error| CommandError::forbidden(error.message))?;
            let harness = crate::domains::harnesses::queries::resolve_effective(
                &ctx.db,
                ctx.org_id(),
                harness_id,
            )
            .await?
            .ok_or_else(|| CommandError::not_found("Harness"))?;
            everruns_core::AgentConfigOverlay::from(&harness).merge(draft)
        } else {
            draft
        };
        effective.capabilities = q::ensure_file_system_capability(
            effective.capabilities,
            !effective.initial_files.is_empty(),
        );
        crate::domains::mcp_servers::scoped_mcp::validate_scoped_mcp_servers_for_org(
            &ctx.db,
            ctx.org_id(),
            &effective.mcp_servers,
        )
        .await?;
        let preview = ctx
            .capability_service
            .preview_with_features(
                ctx.org_id(),
                &effective.system_prompt.unwrap_or_default(),
                &effective.capabilities,
            )
            .await?;
        let prompt = preview.system_prompt;
        let mut tools = preview.tools;
        // Tools that no capability contributes: the duplicate web search check
        // matches these by name, capabilities declare their own search.
        let mut external_tools =
            crate::domains::mcp_servers::scoped_mcp::build_materialized_scoped_mcp_tool_definitions(
                &ctx.db,
                ctx.org_id(),
                &effective.mcp_servers,
                None,
                None,
                ctx.capability_service.egress_service().as_ref(),
            )
            .await?;
        external_tools.extend(self.tools);
        tools.extend(external_tools.iter().cloned());
        // Apply org rule config (phase 4): override built-in severities/enabled
        // and run custom declarative rules. Defaults to no-op when unconfigured.
        let rule_config = super::check_rules::load_effective_config(&ctx.db, ctx.org_id()).await;
        let mut builtin = super::checks::run_builtin_checks(
            &authored_prompt,
            &prompt,
            &effective.capabilities,
            &tools,
        );
        builtin.extend(super::checks::check_superseded_capabilities(
            &effective.capabilities,
            ctx.capability_service.registry(),
        ));
        builtin.extend(super::checks::check_duplicate_web_search(
            &effective.capabilities,
            ctx.capability_service.registry(),
            &external_tools,
        ));
        if let Some(agent_id) = self.agent_id {
            builtin.extend(
                saved_agent_checks(ctx, agent_id, self.harness_id, &effective.capabilities).await?,
            );
        }
        let mut findings = super::checks::apply_rule_overrides(builtin, &rule_config.overrides);
        findings.extend(super::checks::run_declarative_rules(
            &rule_config.declarative,
            &prompt,
        ));
        Ok(AgentPreview {
            features: preview.features,
            initial_files: effective.initial_files,
            system_prompt: prompt,
            tools,
            findings,
        })
    }
}

/// Checks that need the saved agent: its channels and, when the draft did not
/// name a harness, the harness the agent runs on.
async fn saved_agent_checks(
    ctx: &Ctx,
    agent_id: AgentId,
    draft_harness: Option<HarnessId>,
    draft_capabilities: &[AgentCapabilityConfig],
) -> Result<Vec<super::checks::Finding>, CommandError> {
    let Some(agent) = ctx
        .db
        .get_agent_by_public_id(ctx.org_id(), &agent_id.to_string())
        .await?
    else {
        return Ok(Vec::new());
    };
    let has_slack_channel = ctx
        .db
        .list_agent_channels(ctx.org_id(), agent.id.uuid())
        .await?
        .iter()
        .any(|channel| channel.channel_type == "slack");
    if !has_slack_channel {
        return Ok(Vec::new());
    }
    let mut capabilities = draft_capabilities.to_vec();
    if draft_harness.is_none() {
        capabilities.extend(
            crate::domains::harnesses::queries::get_capabilities(
                &ctx.db,
                ctx.org_id(),
                agent.harness_id.uuid(),
            )
            .await?,
        );
    }
    Ok(super::checks::check_slack_without_reactions(
        has_slack_channel,
        &capabilities,
    ))
}
