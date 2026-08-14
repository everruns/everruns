// Agent health check service + commands (knowledge/evaluation/agent-checks.md, tier-3).

use std::sync::Arc;

use everruns_core::Caller;
use everruns_provider::typed_id::HealthCheckRunId;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use utoipa::ToSchema;
use uuid::Uuid;

use super::runner::{HealthCheckRunContext, HealthCheckTarget, spawn_health_check_run};
use super::types::{HealthCheckRun, LatestHealthCheckRun};
use crate::domains::common::*;
use crate::services::CapabilityService;
use crate::storage::models::CreateAgentHealthCheckRunRow;

/// How many runs `list` returns.
const LIST_LIMIT: i64 = 20;

/// Service that triggers and reads agent health check runs. Constructed in
/// `app_builder` and attached to `Ctx`. Mirrors `EvalService`.
pub struct AgentHealthCheckService {
    run_ctx: Arc<HealthCheckRunContext>,
    capability_service: Arc<CapabilityService>,
}

impl AgentHealthCheckService {
    pub fn new(
        run_ctx: Arc<HealthCheckRunContext>,
        capability_service: Arc<CapabilityService>,
    ) -> Self {
        Self {
            run_ctx,
            capability_service,
        }
    }

    /// Resolve the agent's config, create a run row, and spawn the background
    /// execution. Returns the pending run immediately.
    pub async fn trigger(
        &self,
        caller: &Caller,
        agent_id: &str,
    ) -> Result<HealthCheckRun, CommandError> {
        let org_id = caller.org_id;
        let agent = crate::domains::agents::queries::resolve(&self.run_ctx.db, org_id, agent_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Agent"))?;

        let (resolved_prompt, tools) = self
            .capability_service
            .preview(org_id, &agent.system_prompt, &agent.capabilities)
            .await
            .map_err(classify_anyhow)?;
        let tool_listing = tools
            .iter()
            .map(|t| format!("- {}: {}", t.name(), t.description()))
            .collect::<Vec<_>>()
            .join("\n");

        let model_id = agent.default_model_id.map(|m| m.to_string());
        let config_hash = config_hash(&resolved_prompt, &tool_listing, model_id.as_deref());

        let public_id = format!("healthcheck_{:032x}", Uuid::now_v7().as_u128());
        let row = self
            .run_ctx
            .db
            .create_agent_health_check_run(
                org_id,
                CreateAgentHealthCheckRunRow {
                    public_id,
                    agent_id: Some(agent.internal_id),
                    config_hash,
                    model_id: model_id.clone(),
                },
            )
            .await
            .map_err(classify_anyhow)?;

        spawn_health_check_run(
            self.run_ctx.clone(),
            HealthCheckTarget {
                org_id,
                run_id: row.id,
                agent_internal_id: agent.internal_id,
                agent_public_id: agent.public_id,
                model_id,
                description: agent.description.clone(),
                resolved_system_prompt: resolved_prompt,
                tool_listing,
            },
        );

        Ok(HealthCheckRun::from(row))
    }

    pub async fn get(
        &self,
        caller: &Caller,
        agent_id: &str,
        run_id: &str,
    ) -> Result<HealthCheckRun, CommandError> {
        let run_id: HealthCheckRunId = run_id
            .parse()
            .map_err(|_| CommandError::bad_request("Invalid health check run id"))?;
        let agent =
            crate::domains::agents::queries::resolve(&self.run_ctx.db, caller.org_id, agent_id)
                .await
                .map_err(classify_anyhow)?
                .ok_or_else(|| CommandError::not_found("Agent"))?;
        let row = self
            .run_ctx
            .db
            .get_agent_health_check_run(caller.org_id, &run_id.to_string())
            .await
            .map_err(classify_anyhow)?
            // Scope the run to the agent in the path: a run for a different
            // agent is treated as not found rather than returned on a
            // mismatched URL.
            .filter(|row| row.agent_id == Some(agent.internal_id))
            .ok_or_else(|| CommandError::not_found("Health check run"))?;
        Ok(HealthCheckRun::from_row_at(row, chrono::Utc::now()))
    }

    pub async fn list(
        &self,
        caller: &Caller,
        agent_id: &str,
    ) -> Result<Vec<HealthCheckRun>, CommandError> {
        let agent =
            crate::domains::agents::queries::resolve(&self.run_ctx.db, caller.org_id, agent_id)
                .await
                .map_err(classify_anyhow)?
                .ok_or_else(|| CommandError::not_found("Agent"))?;
        let rows = self
            .run_ctx
            .db
            .list_agent_health_check_runs(caller.org_id, agent.internal_id, LIST_LIMIT)
            .await
            .map_err(classify_anyhow)?;
        let now = chrono::Utc::now();
        Ok(rows
            .into_iter()
            .map(|row| HealthCheckRun::from_row_at(row, now))
            .collect())
    }

    /// Latest run for the agent (any config), paired with whether the agent's
    /// current resolved config differs from that run's. Lets the editor show
    /// prior results on mount and hint when they're stale. Read-only: it never
    /// triggers a new LLM run.
    pub async fn latest(
        &self,
        caller: &Caller,
        agent_id: &str,
    ) -> Result<LatestHealthCheckRun, CommandError> {
        let org_id = caller.org_id;
        let agent = crate::domains::agents::queries::resolve(&self.run_ctx.db, org_id, agent_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Agent"))?;

        let rows = self
            .run_ctx
            .db
            .list_agent_health_check_runs(org_id, agent.internal_id, 1)
            .await
            .map_err(classify_anyhow)?;
        let Some(row) = rows.into_iter().next() else {
            return Ok(LatestHealthCheckRun {
                run: None,
                config_changed: false,
            });
        };

        // Recompute the current resolved-config hash the same way `trigger`
        // does; a mismatch means this run predates the agent's current config.
        let (resolved_prompt, tools) = self
            .capability_service
            .preview(org_id, &agent.system_prompt, &agent.capabilities)
            .await
            .map_err(classify_anyhow)?;
        let tool_listing = tools
            .iter()
            .map(|t| format!("- {}: {}", t.name(), t.description()))
            .collect::<Vec<_>>()
            .join("\n");
        let model_id = agent.default_model_id.map(|m| m.to_string());
        let current_hash = config_hash(&resolved_prompt, &tool_listing, model_id.as_deref());

        let config_changed = row.config_hash != current_hash;
        Ok(LatestHealthCheckRun {
            // Apply the same staleness guard as get/list so a crashed in-flight
            // latest run surfaces as failed rather than a perpetual spinner.
            run: Some(HealthCheckRun::from_row_at(row, chrono::Utc::now())),
            config_changed,
        })
    }
}

/// Stable hash of the resolved config a run targets.
fn config_hash(resolved_prompt: &str, tool_listing: &str, model_id: Option<&str>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(resolved_prompt.as_bytes());
    hasher.update([0u8]);
    hasher.update(tool_listing.as_bytes());
    hasher.update([0u8]);
    hasher.update(model_id.unwrap_or("").as_bytes());
    hex::encode(hasher.finalize())
}

fn service(ctx: &Ctx) -> Result<Arc<AgentHealthCheckService>, CommandError> {
    ctx.health_check_service.clone().ok_or_else(|| {
        // The service is wired only when the deployment has both the system
        // utility LLM (generates/judges cases) and a default harness (hosts
        // the sessions), so keep the message general rather than naming one.
        CommandError::bad_request("Agent health checks are not available on this deployment")
    })
}

// ============================================================================
// Commands
// ============================================================================

/// Trigger a behavioral health check for an agent.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TriggerAgentHealthCheck {
    pub agent_id: String,
}

impl Command for TriggerAgentHealthCheck {
    type Output = HealthCheckRun;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "trigger_agent_health_check",
            category: "agents",
            description: "Run a behavioral health check (generated smoke tests) against an agent.",
            method: "POST",
            path: "/v1/agents/{agent_id}/health-checks",
        }
    }

    fn read_only() -> bool {
        false
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::agents::AGENT_HEALTH_CHECK_RUN)
    }

    async fn execute(self, ctx: &Ctx) -> Result<HealthCheckRun, CommandError> {
        service(ctx)?.trigger(&ctx.caller, &self.agent_id).await
    }
}

inventory::submit! { CommandDescriptor::of::<TriggerAgentHealthCheck>() }

/// Get a health check run by id.
#[derive(Debug, Deserialize, ToSchema)]
pub struct GetAgentHealthCheckRun {
    pub agent_id: String,
    pub run_id: String,
}

impl Command for GetAgentHealthCheckRun {
    type Output = HealthCheckRun;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_agent_health_check_run",
            category: "agents",
            description: "Get a single agent health check run with its results.",
            method: "GET",
            path: "/v1/agents/{agent_id}/health-checks/{run_id}",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::agents::AGENT_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<HealthCheckRun, CommandError> {
        service(ctx)?
            .get(&ctx.caller, &self.agent_id, &self.run_id)
            .await
    }
}

inventory::submit! { CommandDescriptor::of::<GetAgentHealthCheckRun>() }

/// List recent health check runs for an agent.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ListAgentHealthCheckRuns {
    pub agent_id: String,
}

impl Command for ListAgentHealthCheckRuns {
    type Output = Vec<HealthCheckRun>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_agent_health_check_runs",
            category: "agents",
            description: "List recent health check runs for an agent.",
            method: "GET",
            path: "/v1/agents/{agent_id}/health-checks",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::agents::AGENT_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<HealthCheckRun>, CommandError> {
        service(ctx)?.list(&ctx.caller, &self.agent_id).await
    }
}

inventory::submit! { CommandDescriptor::of::<ListAgentHealthCheckRuns>() }

/// Get the latest health check run for an agent, without triggering a new one.
#[derive(Debug, Deserialize, ToSchema)]
pub struct GetLatestAgentHealthCheckRun {
    pub agent_id: String,
}

impl Command for GetLatestAgentHealthCheckRun {
    type Output = LatestHealthCheckRun;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_latest_agent_health_check_run",
            category: "agents",
            description: "Get the latest health check run for an agent, with a stale-config flag.",
            method: "GET",
            path: "/v1/agents/{agent_id}/health-checks/latest",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::agents::AGENT_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<LatestHealthCheckRun, CommandError> {
        service(ctx)?.latest(&ctx.caller, &self.agent_id).await
    }
}

inventory::submit! { CommandDescriptor::of::<GetLatestAgentHealthCheckRun>() }
