//! Storage for the PACT Delegated profile's authorization server (PACT 1.0
//! §5.3): per-endpoint signing keys, device-code requests, grants, refresh
//! tokens and redeemed company sign-in assertions. Device codes and refresh
//! tokens are stored only as SHA-256 hashes. See migration 188.
use super::StorageBackend;
use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// An endpoint's delegation signing key as stored.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PactSigningKeyRow {
    pub kid: String,
    pub private_key: Vec<u8>,
    pub encrypted: bool,
    pub public_jwk: serde_json::Value,
}

/// A device-code request (RFC 8628 §3.1).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PactDeviceAuthorizationRow {
    pub device_code_hash: Vec<u8>,
    pub client_id: String,
    pub user_code: String,
    pub requested_scope: String,
    pub status: String,
    pub grant_id: Option<String>,
    pub interval_secs: i32,
    pub last_polled_at: Option<DateTime<Utc>>,
    pub expires_at: DateTime<Utc>,
}

/// What a user approved for one personal agent.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PactGrantRow {
    pub id: String,
    pub channel_id: Uuid,
    pub client_id: String,
    pub brand_user_id: String,
    pub scope: String,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl PactGrantRow {
    /// Not revoked and not expired at `now`.
    pub fn is_live(&self, now: DateTime<Utc>) -> bool {
        self.revoked_at.is_none() && self.expires_at > now
    }
}

impl StorageBackend {
    pub async fn pact_signing_key(&self, channel_id: Uuid) -> Result<Option<PactSigningKeyRow>> {
        let db = self.database();
        Ok(sqlx::query_as::<_, PactSigningKeyRow>(
            "SELECT kid,private_key,encrypted,public_jwk FROM pact_signing_keys WHERE channel_id=$1",
        )
        .bind(channel_id)
        .fetch_optional(db.pool())
        .await?)
    }

    /// Store `key` unless the endpoint already has one, and return the key
    /// that is stored, so two first requests racing agree on one key.
    pub async fn insert_pact_signing_key(
        &self,
        channel_id: Uuid,
        key: &PactSigningKeyRow,
    ) -> Result<PactSigningKeyRow> {
        let db = self.database();
        sqlx::query("INSERT INTO pact_signing_keys(channel_id,kid,private_key,encrypted,public_jwk) VALUES($1,$2,$3,$4,$5) ON CONFLICT (channel_id) DO NOTHING")
            .bind(channel_id).bind(&key.kid).bind(&key.private_key).bind(key.encrypted).bind(&key.public_jwk)
            .execute(db.pool()).await?;
        self.pact_signing_key(channel_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("PACT signing key missing after insert"))
    }

    pub async fn create_pact_device_authorization(
        &self,
        channel_id: Uuid,
        row: &PactDeviceAuthorizationRow,
    ) -> Result<()> {
        let db = self.database();
        sqlx::query("DELETE FROM pact_device_authorizations WHERE device_code_hash IN (SELECT device_code_hash FROM pact_device_authorizations WHERE expires_at<now() - interval '1 day' LIMIT 1000)")
            .execute(db.pool()).await?;
        sqlx::query("INSERT INTO pact_device_authorizations(device_code_hash,channel_id,client_id,user_code,requested_scope,interval_secs,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind(&row.device_code_hash).bind(channel_id).bind(&row.client_id).bind(&row.user_code)
            .bind(&row.requested_scope).bind(row.interval_secs).bind(row.expires_at)
            .execute(db.pool()).await?;
        Ok(())
    }

    /// Look up a device code for a token poll and record the poll time.
    /// Returns the row as it was before this poll.
    pub async fn poll_pact_device_authorization(
        &self,
        channel_id: Uuid,
        device_code_hash: &[u8],
    ) -> Result<Option<PactDeviceAuthorizationRow>> {
        let db = self.database();
        let mut tx = db.pool().begin().await?;
        let row = sqlx::query_as::<_, PactDeviceAuthorizationRow>("SELECT device_code_hash,client_id,user_code,requested_scope,status,grant_id,interval_secs,last_polled_at,expires_at FROM pact_device_authorizations WHERE device_code_hash=$1 AND channel_id=$2 FOR UPDATE")
        .bind(device_code_hash)
        .bind(channel_id)
        .fetch_optional(&mut *tx)
        .await?;
        if row.is_some() {
            sqlx::query("UPDATE pact_device_authorizations SET last_polled_at=now() WHERE device_code_hash=$1")
                .bind(device_code_hash)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(row)
    }

    /// Mark an approved device code used, once. Returns its grant id.
    pub async fn consume_pact_device_authorization(
        &self,
        channel_id: Uuid,
        device_code_hash: &[u8],
    ) -> Result<Option<String>> {
        let db = self.database();
        Ok(sqlx::query_scalar::<_, Option<String>>(
            "UPDATE pact_device_authorizations SET status='consumed' WHERE device_code_hash=$1 AND channel_id=$2 AND status='approved' RETURNING grant_id",
        )
        .bind(device_code_hash)
        .bind(channel_id)
        .fetch_optional(db.pool())
        .await?
        .flatten())
    }

    /// The pending, unexpired request a company sign-in names by `user_code`.
    pub async fn pending_pact_device_authorization(
        &self,
        channel_id: Uuid,
        user_code: &str,
    ) -> Result<Option<PactDeviceAuthorizationRow>> {
        let db = self.database();
        Ok(sqlx::query_as::<_, PactDeviceAuthorizationRow>("SELECT device_code_hash,client_id,user_code,requested_scope,status,grant_id,interval_secs,last_polled_at,expires_at FROM pact_device_authorizations WHERE channel_id=$1 AND user_code=$2 AND status='pending' AND expires_at>now()")
        .bind(channel_id)
        .bind(user_code)
        .fetch_optional(db.pool())
        .await?)
    }

    /// Record the user's refusal. `false` when the request is no longer pending.
    pub async fn deny_pact_device_authorization(
        &self,
        channel_id: Uuid,
        device_code_hash: &[u8],
        brand_user_id: &str,
    ) -> Result<bool> {
        let db = self.database();
        let updated = sqlx::query("UPDATE pact_device_authorizations SET status='denied', brand_user_id=$3 WHERE device_code_hash=$1 AND channel_id=$2 AND status='pending' AND expires_at>now()")
            .bind(device_code_hash).bind(channel_id).bind(brand_user_id)
            .execute(db.pool()).await?;
        Ok(updated.rows_affected() == 1)
    }

    /// Record the user's approval: create `grant` and attach it to the
    /// request, atomically. `false` when the request is no longer pending.
    pub async fn approve_pact_device_authorization(
        &self,
        device_code_hash: &[u8],
        grant: &PactGrantRow,
    ) -> Result<bool> {
        let db = self.database();
        let mut tx = db.pool().begin().await?;
        sqlx::query("INSERT INTO pact_grants(id,channel_id,client_id,brand_user_id,scope,expires_at) VALUES($1,$2,$3,$4,$5,$6)")
            .bind(&grant.id).bind(grant.channel_id).bind(&grant.client_id).bind(&grant.brand_user_id)
            .bind(&grant.scope).bind(grant.expires_at)
            .execute(&mut *tx).await?;
        let updated = sqlx::query("UPDATE pact_device_authorizations SET status='approved', brand_user_id=$3, grant_id=$4 WHERE device_code_hash=$1 AND channel_id=$2 AND status='pending' AND expires_at>now()")
            .bind(device_code_hash).bind(grant.channel_id).bind(&grant.brand_user_id).bind(&grant.id)
            .execute(&mut *tx).await?;
        if updated.rows_affected() != 1 {
            tx.rollback().await?;
            return Ok(false);
        }
        tx.commit().await?;
        Ok(true)
    }

    pub async fn pact_grant(
        &self,
        channel_id: Uuid,
        grant_id: &str,
    ) -> Result<Option<PactGrantRow>> {
        let db = self.database();
        Ok(sqlx::query_as::<_, PactGrantRow>("SELECT id,channel_id,client_id,brand_user_id,scope,expires_at,revoked_at FROM pact_grants WHERE id=$1 AND channel_id=$2")
        .bind(grant_id)
        .bind(channel_id)
        .fetch_optional(db.pool())
        .await?)
    }

    pub async fn insert_pact_refresh_token(
        &self,
        token_hash: &[u8],
        grant_id: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<()> {
        let db = self.database();
        sqlx::query(
            "INSERT INTO pact_refresh_tokens(token_hash,grant_id,expires_at) VALUES($1,$2,$3)",
        )
        .bind(token_hash)
        .bind(grant_id)
        .bind(expires_at)
        .execute(db.pool())
        .await?;
        Ok(())
    }

    /// Redeem a refresh token once. Returns its grant id; a used, expired or
    /// unknown token returns `None`.
    pub async fn use_pact_refresh_token(&self, token_hash: &[u8]) -> Result<Option<String>> {
        let db = self.database();
        Ok(sqlx::query_scalar::<_, String>(
            "UPDATE pact_refresh_tokens SET used_at=now() WHERE token_hash=$1 AND used_at IS NULL AND expires_at>now() RETURNING grant_id",
        )
        .bind(token_hash)
        .fetch_optional(db.pool())
        .await?)
    }

    /// Record a company sign-in assertion's `jti`. `false` when it was
    /// already redeemed.
    pub async fn record_pact_assertion(
        &self,
        channel_id: Uuid,
        jti: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<bool> {
        let db = self.database();
        sqlx::query("DELETE FROM pact_used_assertions WHERE (channel_id,jti) IN (SELECT channel_id,jti FROM pact_used_assertions WHERE expires_at<now() LIMIT 1000)")
            .execute(db.pool()).await?;
        let inserted = sqlx::query("INSERT INTO pact_used_assertions(channel_id,jti,expires_at) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
            .bind(channel_id).bind(jti).bind(expires_at)
            .execute(db.pool()).await?;
        Ok(inserted.rows_affected() == 1)
    }
}
