// Agent preview resolves the same harness and agent overlays as execution.

use super::queries as q;
use crate::domains::common::*;
use crate::kernel_imports::{
    AgentCapabilityConfig, InitialFile, ScopedMcpServers, contracts::tool_types::ToolDefinition,
};
use everruns_contracts::typed_id::HarnessId;
use serde::Deserialize;
use utoipa::ToSchema;

// ============================================================================
// PreviewAgent
// ============================================================================

/// Preview the final agent shape with capabilities applied.
#[derive(Debug, Deserialize, ToSchema)]
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

impl Command for PreviewAgent {
    type Output = AgentPreview;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "preview_agent",
            category: "agents",
            description: "Preview the final agent shape with capabilities applied.",
            method: "POST",
            path: "/v1/agents/preview",
        }
    }

    fn cli() -> Option<CliRoute> {
        // A const so the declared slices get 'static promotion:
        // `CliArg::new(..).short(..)` is a const fn, but an array of them
        // is only promoted inside a const initializer.
        const ROUTE: CliRoute =
            CliRoute::new(&["agents"], "preview").with_examples(&[CliExample::new(
                "See the prompt a draft configuration would produce, without creating it",
                "everruns agents preview --system-prompt 'Triage incoming issues'",
            )]);
        Some(ROUTE)
    }

    fn read_only() -> bool {
        true
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::agents::AGENT_VIEW)
    }

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
            .await
            .map_err(classify_anyhow)?
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
        .await
        .map_err(classify_anyhow)?;
        let preview = ctx
            .capability_service
            .preview_with_features(
                ctx.org_id(),
                &effective.system_prompt.unwrap_or_default(),
                &effective.capabilities,
            )
            .await
            .map_err(classify_anyhow)?;
        let prompt = preview.system_prompt;
        let mut tools = preview.tools;
        tools.extend(
            crate::domains::mcp_servers::scoped_mcp::build_materialized_scoped_mcp_tool_definitions(
                &ctx.db,
                ctx.org_id(),
                &effective.mcp_servers,
                None,
                None,
                ctx.capability_service.egress_service().as_ref(),
            )
            .await
            .map_err(classify_anyhow)?,
        );
        tools.extend(self.tools);
        // Apply org rule config (phase 4): override built-in severities/enabled
        // and run custom declarative rules. Defaults to no-op when unconfigured.
        let rule_config = super::check_rules::load_effective_config(&ctx.db, ctx.org_id()).await;
        let builtin = super::checks::run_builtin_checks(
            &authored_prompt,
            &prompt,
            &effective.capabilities,
            &tools,
        );
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

inventory::submit! { CommandDescriptor::of::<PreviewAgent>() }
