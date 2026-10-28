//! Customer HTTP endpoints use the custom-worker trust boundary. The adapter
//! credential never leaves the CP; response events carry no management authority.
use crate::domains::{
    agents::{AGENT_DANGEROUS, AGENT_VIEW},
    common::*,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

#[derive(Debug, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct ExternalAgent {
    pub endpoint_url: String,
    pub enabled: bool,
    pub last_error: Option<String>,
    pub last_invoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Deserialize, Serialize, ToSchema)]
pub struct GetExternalAgent {
    pub agent_id: String,
}

#[command(name = "get_external_agent", category = "agents", description = "Get an Agent's external execution endpoint.", method = "GET", path = "/v1/agents/{agent_id}/external-worker", policy = AGENT_VIEW)]
impl Command for GetExternalAgent {
    type Output = Option<ExternalAgent>;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let agent = super::agents::queries::resolve(&ctx.db, ctx.org_id(), &self.agent_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Agent"))?;
        Ok(sqlx::query_as("SELECT endpoint_url, enabled, last_error, last_invoked_at FROM custom_workers WHERE org_id = $1 AND agent_id = $2 AND endpoint_url IS NOT NULL")
            .bind(ctx.org_id()).bind(agent.internal_id).fetch_optional(ctx.db.pool()).await.map_err(anyhow::Error::from)?)
    }
}

#[derive(Deserialize, Serialize, ToSchema)]
pub struct RegisterExternalAgent {
    #[serde(default)]
    pub agent_id: String,
    pub endpoint_url: String,
    /// Preinstall this random, 32+ character secret in the endpoint. Never a user PAT.
    pub endpoint_secret: String,
}

#[command(name = "register_external_agent", category = "agents", description = "Verify and register a customer-hosted HTTPS Agent endpoint.", method = "POST", path = "/v1/agents/{agent_id}/external-worker", policy = AGENT_DANGEROUS)]
impl Command for RegisterExternalAgent {
    type Output = ExternalAgent;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        if !enabled() {
            return Err(CommandError::unavailable(
                "External agents are not enabled on this installation",
            ));
        }
        self.register(ctx).await
    }
}

impl RegisterExternalAgent {
    async fn register(mut self, ctx: &Ctx) -> Result<ExternalAgent, CommandError> {
        if !(32..=256).contains(&self.endpoint_secret.len()) {
            return Err(CommandError::bad_request(
                "Endpoint secret must contain 32-256 characters",
            ));
        }
        crate::worker_link::custom_workers::http::validate_endpoint(&self.endpoint_url).map_err(
            |_| {
                CommandError::bad_request(
                    "A public HTTPS endpoint without credentials, query or fragment is required",
                )
            },
        )?;
        self.endpoint_url = url::Url::parse(&self.endpoint_url)
            .map_err(anyhow::Error::from)?
            .to_string();
        let agent = super::agents::queries::resolve(&ctx.db, ctx.org_id(), &self.agent_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Agent"))?;
        if agent.is_built_in || agent.status != super::agents::record::AgentStatus::Active {
            return Err(CommandError::bad_request(
                "Only an active, customer-created Agent can use an external worker",
            ));
        }
        let encryption = ctx.encryption.as_ref().ok_or_else(|| {
            CommandError::unavailable("Encryption is required for external agents")
        })?;
        let egress = ctx
            .egress_service
            .as_ref()
            .ok_or_else(|| CommandError::unavailable("Outbound transport is required"))?;
        let org = ctx
            .db
            .get_organization(ctx.org_id())
            .await?
            .ok_or_else(|| CommandError::not_found("Organization"))?;
        let scoped = everruns_core::ScopedEgressService::new(
            egress.clone(),
            everruns_core::EgressScope {
                org_id: Some(org.public_id.parse().map_err(anyhow::Error::from)?),
                session_id: None,
            },
        );
        crate::worker_link::custom_workers::http::verify_endpoint(
            &scoped,
            &self.endpoint_url,
            &self.endpoint_secret,
        )
        .await
        .map_err(|error| {
            if matches!(
                error.downcast_ref::<everruns_core::EgressError>(),
                Some(everruns_core::EgressError::NetworkAccessDenied { .. })
            ) {
                CommandError::bad_request(
                    "The installation's outbound network policy blocks this endpoint",
                )
            } else {
                CommandError::bad_request("Endpoint ownership verification failed")
            }
        })?;
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let encrypted_secret = encryption.encrypt_string(&self.endpoint_secret)?;
        let encrypted_token = encryption.encrypt_string(&token)?;
        let mut tx = ctx.db.pool().begin().await.map_err(anyhow::Error::from)?;
        // Registration is a routing change. Briefly exclude actor writes while
        // checking for held leases, including actors created during the check.
        // Network verification has already finished, outside this transaction.
        sqlx::query("LOCK TABLE session_actors IN SHARE ROW EXCLUSIVE MODE")
            .execute(&mut *tx)
            .await
            .map_err(anyhow::Error::from)?;
        let held: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM session_actors a JOIN sessions s ON s.id = a.session_id WHERE s.agent_id = $1 AND a.lease_expires_at > NOW())").bind(agent.internal_id).fetch_one(&mut *tx).await.map_err(anyhow::Error::from)?;
        if held {
            return Err(CommandError::conflict(
                "Stop active turns before changing the external worker",
            ));
        }
        // An operator-created pull worker is never silently converted.
        let pull: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM custom_workers WHERE agent_id = $1 AND endpoint_url IS NULL)").bind(agent.internal_id).fetch_one(&mut *tx).await.map_err(anyhow::Error::from)?;
        if pull {
            return Err(CommandError::conflict(
                "Agent already has an operator-managed worker",
            ));
        }
        sqlx::query("INSERT INTO custom_workers(org_id, agent_id, name, token_hash, endpoint_url, endpoint_secret_encrypted, adapter_token_encrypted) VALUES ($1,$2,'external-http',$3,$4,$5,$6) ON CONFLICT(agent_id) DO UPDATE SET token_hash = EXCLUDED.token_hash, endpoint_url = EXCLUDED.endpoint_url, endpoint_secret_encrypted = EXCLUDED.endpoint_secret_encrypted, adapter_token_encrypted = EXCLUDED.adapter_token_encrypted, enabled = TRUE, last_error = NULL")
            .bind(ctx.org_id()).bind(agent.internal_id).bind(Sha256::digest(token.as_bytes()).as_slice()).bind(&self.endpoint_url).bind(encrypted_secret).bind(encrypted_token).execute(&mut *tx).await.map_err(anyhow::Error::from)?;
        tx.commit().await.map_err(anyhow::Error::from)?;
        audit(ctx, &self.agent_id, "external_worker_registered");
        Ok(ExternalAgent {
            endpoint_url: self.endpoint_url,
            enabled: true,
            last_error: None,
            last_invoked_at: None,
        })
    }
}

#[derive(Deserialize, Serialize, ToSchema)]
pub struct DisableExternalAgent {
    pub agent_id: String,
}

#[command(name = "disable_external_agent", category = "agents", description = "Revoke external execution; queued sessions stay bound without native fallback.", method = "DELETE", path = "/v1/agents/{agent_id}/external-worker", policy = AGENT_DANGEROUS)]
impl Command for DisableExternalAgent {
    type Output = ();
    async fn execute(self, ctx: &Ctx) -> Result<(), CommandError> {
        let agent = super::agents::queries::resolve(&ctx.db, ctx.org_id(), &self.agent_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Agent"))?;
        sqlx::query("UPDATE custom_workers SET enabled = FALSE WHERE org_id = $1 AND agent_id = $2 AND endpoint_url IS NOT NULL").bind(ctx.org_id()).bind(agent.internal_id).execute(ctx.db.pool()).await.map_err(anyhow::Error::from)?;
        audit(ctx, &self.agent_id, "external_worker_disabled");
        Ok(())
    }
}

fn audit(ctx: &Ctx, agent: &str, change: &str) {
    use crate::domains::audit_logs::record::{AuditEvent, ManagementAction};
    crate::auth::audit::emit_event(
        ctx.db.clone(),
        AuditEvent::management(
            ManagementAction::AgentUpdated,
            ctx.org_id(),
            ctx.caller.user_id,
        )
        .target("agent", agent.to_string())
        .detail("change", change)
        .build(),
    );
}

pub fn enabled() -> bool {
    std::env::var("EXTERNAL_AGENTS_ENABLED").is_ok_and(|v| v == "true")
}

#[cfg(test)]
mod tests;
