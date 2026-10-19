//! AgentID consumer sign-in storage: in-flight sign-in state and the per-owner
//! agent cap. An AgentID subject only ever becomes an end-user virtual user;
//! nothing here touches `users`, personal access tokens or the email linker.
//! See knowledge/integrations/agentid.md.
use super::runtime_identity::VerifiedRuntimeIdentity;
use super::{StorageBackend, VirtualUserRow};
use crate::domains::agent_channels::record::AGENTID_ISSUER;
use crate::domains::agent_channels::record::AGENTID_PROVIDER;
use anyhow::{Result, bail};

/// Agents one AgentID owner may sign in to an org when the org sets no cap.
pub const DEFAULT_AGENTS_PER_OWNER: i32 = 5;
/// How long a browser sign-in may take, start to callback. Above AgentID's own
/// five-minute transaction so the agent's approval, not ours, runs out first.
pub const LOGIN_STATE_TTL_MINUTES: i64 = 10;

/// A sign-in started for a channel, waiting for AgentID's callback.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgentIdLoginState {
    pub org_id: i64,
    pub channel_id: String,
    pub code_verifier: String,
    pub nonce: String,
    pub login_hint: Option<String>,
}

/// A verified AgentID agent, as the callback hands it to storage.
#[derive(Debug, Clone)]
pub struct AgentIdAgent {
    pub subject: String,
    pub owner_sub: String,
    pub owner_email: Option<String>,
    pub display_name: String,
}

#[derive(Debug)]
pub enum AgentIdSignIn {
    SignedIn(Box<VirtualUserRow>),
    /// The owner already has the org's cap of active agents.
    OwnerCapReached,
}

impl StorageBackend {
    pub async fn create_agentid_login_state(
        &self,
        state_hash: &[u8],
        login: &AgentIdLoginState,
    ) -> Result<()> {
        if state_hash.len() != 32 {
            bail!("Invalid sign-in state");
        }
        let expires = chrono::Utc::now() + chrono::Duration::minutes(LOGIN_STATE_TTL_MINUTES);
        let db = self.database();
        sqlx::query("DELETE FROM agentid_login_states WHERE state_hash IN (SELECT state_hash FROM agentid_login_states WHERE expires_at<now() LIMIT 1000)").execute(db.pool()).await?;
        sqlx::query("INSERT INTO agentid_login_states(state_hash,org_id,channel_id,code_verifier,nonce,login_hint,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind(state_hash).bind(login.org_id).bind(&login.channel_id).bind(&login.code_verifier)
            .bind(&login.nonce).bind(&login.login_hint).bind(expires)
            .execute(db.pool()).await?;
        Ok(())
    }

    /// Consume a sign-in state once. An unknown, reused or expired state
    /// returns `None`, so the callback fails closed.
    pub async fn consume_agentid_login_state(
        &self,
        state_hash: &[u8],
    ) -> Result<Option<AgentIdLoginState>> {
        let db = self.database();
        Ok(sqlx::query_as::<_, AgentIdLoginState>("DELETE FROM agentid_login_states WHERE state_hash=$1 AND expires_at>now() RETURNING org_id,channel_id,code_verifier,nonce,login_hint")
            .bind(state_hash).fetch_optional(db.pool()).await?)
    }

    pub async fn agentid_agents_per_owner(&self, org_id: i64) -> Result<i32> {
        Ok(self
            .agentid_agents_per_owner_setting(org_id)
            .await?
            .unwrap_or(DEFAULT_AGENTS_PER_OWNER))
    }

    /// The org's own cap, `None` when it uses the platform default. Read
    /// apart from `OrganizationSettingsRow`, whose file may not grow.
    pub async fn agentid_agents_per_owner_setting(&self, org_id: i64) -> Result<Option<i32>> {
        let db = self.database();
        let cap: Option<Option<i32>> = sqlx::query_scalar(
            "SELECT agentid_agents_per_owner FROM organization_settings WHERE org_id=$1",
        )
        .bind(org_id)
        .fetch_optional(db.pool())
        .await?;
        Ok(cap.flatten())
    }

    pub async fn set_agentid_agents_per_owner(&self, org_id: i64, cap: Option<i32>) -> Result<()> {
        if cap.is_some_and(|cap| cap < 0) {
            bail!("The agent cap cannot be negative");
        }
        let db = self.database();
        sqlx::query("INSERT INTO organization_settings(org_id,agentid_agents_per_owner) VALUES($1,$2) ON CONFLICT(org_id) DO UPDATE SET agentid_agents_per_owner=EXCLUDED.agentid_agents_per_owner, updated_at=NOW()")
            .bind(org_id).bind(cap).execute(db.pool()).await?;
        Ok(())
    }

    /// Resolve the end-user virtual user for a verified AgentID agent, keyed by
    /// its `sub`. A returning agent always gets its account back; a new one is
    /// created only while its owner is under the org's cap of active agents.
    pub async fn agentid_sign_in(&self, org_id: i64, agent: AgentIdAgent) -> Result<AgentIdSignIn> {
        if agent.owner_sub.is_empty() || agent.owner_sub.len() > 256 {
            bail!("Invalid AgentID owner");
        }
        let db = self.database();
        // Serialize first sign-ins per owner so concurrent callbacks cannot
        // both pass the cap. The transaction only holds the lock.
        let mut lock = db.pool().begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, $2))")
            .bind(format!("agentid-owner:{}", agent.owner_sub))
            .bind(org_id)
            .execute(&mut *lock)
            .await?;
        let known: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM virtual_user_bindings WHERE org_id=$1 AND provider=$2 AND realm=$3 AND subject=$4)")
            .bind(org_id).bind(AGENTID_PROVIDER).bind(AGENTID_ISSUER).bind(&agent.subject)
            .fetch_one(db.pool()).await?;
        if !known {
            let active: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agentid_agent_owners o JOIN virtual_users v ON v.org_id=o.org_id AND v.id=o.virtual_user_id WHERE o.org_id=$1 AND o.owner_sub=$2 AND v.status='active'")
                .bind(org_id).bind(&agent.owner_sub).fetch_one(db.pool()).await?;
            if active >= i64::from(self.agentid_agents_per_owner(org_id).await?) {
                return Ok(AgentIdSignIn::OwnerCapReached);
            }
        }
        let user = self
            .resolve_runtime_identity(VerifiedRuntimeIdentity {
                org_id,
                provider: AGENTID_PROVIDER.into(),
                realm: AGENTID_ISSUER.into(),
                subject: agent.subject,
                name: agent.display_name,
                avatar_url: None,
                // Never linked to a management user, whatever the emails say.
                management_user_id: None,
            })
            .await?;
        sqlx::query("INSERT INTO agentid_agent_owners(org_id,virtual_user_id,owner_sub,owner_email) VALUES($1,$2,$3,$4) ON CONFLICT(org_id,virtual_user_id) DO UPDATE SET owner_sub=EXCLUDED.owner_sub, owner_email=EXCLUDED.owner_email, updated_at=NOW()")
            .bind(org_id).bind(user.id).bind(&agent.owner_sub).bind(&agent.owner_email)
            .execute(db.pool()).await?;
        lock.commit().await?;
        Ok(AgentIdSignIn::SignedIn(Box::new(user)))
    }
}

#[cfg(test)]
mod tests;
