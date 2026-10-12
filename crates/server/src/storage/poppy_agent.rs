//! Storage for Everruns agents acting as Poppy personal agents: the
//! deployment's signing key, each person's User ID and DPoP key per company,
//! and each session's live conversation with a company. See migration 208
//! and `knowledge/integrations/poppy-personal-agent.md`.
use super::StorageBackend;
use super::pact_delegation::PactSigningKeyRow;
use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// One person's identity at one company: the pairwise User ID and the DPoP
/// key their Session Tokens are bound to.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PoppyAgentAccountRow {
    pub user_id: String,
    pub dpop_jwk: serde_json::Value,
    pub dpop_key: Vec<u8>,
    pub encrypted: bool,
}

/// One session's conversation with one company.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PoppyAgentConversationRow {
    pub session_id: Uuid,
    pub domain: String,
    pub issuer: String,
    pub state: serde_json::Value,
    pub access_token: Option<Vec<u8>>,
    pub token_sealed: bool,
    pub token_expires_at: Option<DateTime<Utc>>,
}

impl StorageBackend {
    pub async fn poppy_agent_key(&self) -> Result<Option<PactSigningKeyRow>> {
        let db = self.database();
        Ok(sqlx::query_as::<_, PactSigningKeyRow>(
            "SELECT kid,private_key,encrypted,public_jwk FROM poppy_agent_keys WHERE id=1",
        )
        .fetch_optional(db.pool())
        .await?)
    }

    /// Store `key` unless the deployment already has one, and return the one
    /// stored, so two first uses racing agree on one key.
    pub async fn insert_poppy_agent_key(
        &self,
        key: &PactSigningKeyRow,
    ) -> Result<PactSigningKeyRow> {
        let db = self.database();
        sqlx::query("INSERT INTO poppy_agent_keys(id,kid,private_key,encrypted,public_jwk) VALUES(1,$1,$2,$3,$4) ON CONFLICT (id) DO NOTHING")
            .bind(&key.kid).bind(&key.private_key).bind(key.encrypted).bind(&key.public_jwk)
            .execute(db.pool()).await?;
        self.poppy_agent_key()
            .await?
            .ok_or_else(|| anyhow::anyhow!("Poppy agent key missing after insert"))
    }

    pub async fn poppy_agent_account(
        &self,
        org_id: i64,
        owner: Uuid,
        issuer: &str,
    ) -> Result<Option<PoppyAgentAccountRow>> {
        let db = self.database();
        Ok(sqlx::query_as::<_, PoppyAgentAccountRow>(
            "SELECT user_id,dpop_jwk,dpop_key,encrypted FROM poppy_agent_accounts WHERE org_id=$1 AND owner_principal_id=$2 AND issuer=$3",
        )
        .bind(org_id)
        .bind(owner)
        .bind(issuer)
        .fetch_optional(db.pool())
        .await?)
    }

    /// Store `row` unless the person already has an identity at `issuer`,
    /// and return the one stored.
    pub async fn insert_poppy_agent_account(
        &self,
        org_id: i64,
        owner: Uuid,
        issuer: &str,
        row: &PoppyAgentAccountRow,
    ) -> Result<PoppyAgentAccountRow> {
        let db = self.database();
        sqlx::query("INSERT INTO poppy_agent_accounts(org_id,owner_principal_id,issuer,user_id,dpop_jwk,dpop_key,encrypted) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT DO NOTHING")
            .bind(org_id).bind(owner).bind(issuer).bind(&row.user_id).bind(&row.dpop_jwk)
            .bind(&row.dpop_key).bind(row.encrypted)
            .execute(db.pool()).await?;
        self.poppy_agent_account(org_id, owner, issuer)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Poppy agent account missing after insert"))
    }

    pub async fn poppy_agent_conversation(
        &self,
        session_id: Uuid,
        domain: &str,
    ) -> Result<Option<PoppyAgentConversationRow>> {
        let db = self.database();
        Ok(sqlx::query_as::<_, PoppyAgentConversationRow>(
            "SELECT session_id,domain,issuer,state,access_token,token_sealed,token_expires_at FROM poppy_agent_conversations WHERE session_id=$1 AND domain=$2",
        )
        .bind(session_id)
        .bind(domain)
        .fetch_optional(db.pool())
        .await?)
    }

    pub async fn upsert_poppy_agent_conversation(
        &self,
        org_id: i64,
        row: &PoppyAgentConversationRow,
    ) -> Result<()> {
        let db = self.database();
        sqlx::query("INSERT INTO poppy_agent_conversations(session_id,domain,org_id,issuer,state,access_token,token_sealed,token_expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT (session_id,domain) DO UPDATE SET issuer=EXCLUDED.issuer,state=EXCLUDED.state,access_token=EXCLUDED.access_token,token_sealed=EXCLUDED.token_sealed,token_expires_at=EXCLUDED.token_expires_at")
            .bind(row.session_id).bind(&row.domain).bind(org_id).bind(&row.issuer).bind(&row.state)
            .bind(&row.access_token).bind(row.token_sealed).bind(row.token_expires_at)
            .execute(db.pool()).await?;
        Ok(())
    }
}
