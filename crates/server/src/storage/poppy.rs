//! Storage for Personal Agent Protocol ("Poppy") channels: Sessions,
//! conversations and the personal agent's accepted message ids. See
//! migration 203 and `knowledge/integrations/poppy-channel.md`.
use super::StorageBackend;
use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// One Poppy Session (spec 4): one user, through one personal agent, at one
/// channel.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PoppySessionRow {
    pub id: String,
    pub channel_id: Uuid,
    pub client_id: String,
    pub user_id: String,
    pub account: Option<String>,
    pub scope: String,
    pub created_at: DateTime<Utc>,
    pub renewed_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}

/// One conversation (spec 7), backed by one Everruns session.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PoppyConversationRow {
    pub session_id: Uuid,
    pub channel_id: Uuid,
    pub client_id: String,
    pub user_id: String,
    pub account: Option<String>,
    pub context: serde_json::Value,
    pub closed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// A personal-agent message id already accepted (spec 7.3).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PoppyMessageRow {
    pub session_id: Uuid,
    pub content_hash: Vec<u8>,
}

/// Who a Poppy conversation or message belongs to: the channel, the personal
/// agent's `client_id` and its User ID.
#[derive(Debug, Clone, Copy)]
pub struct PoppyOwner<'a> {
    pub channel_id: Uuid,
    pub client_id: &'a str,
    pub user_id: &'a str,
}

impl StorageBackend {
    pub async fn create_poppy_session(&self, row: &PoppySessionRow) -> Result<()> {
        let db = self.database();
        sqlx::query("INSERT INTO poppy_sessions(id,channel_id,client_id,user_id,account,scope) VALUES($1,$2,$3,$4,$5,$6)")
            .bind(&row.id).bind(row.channel_id).bind(&row.client_id).bind(&row.user_id)
            .bind(&row.account).bind(&row.scope)
            .execute(db.pool()).await?;
        Ok(())
    }

    pub async fn poppy_session(&self, id: &str) -> Result<Option<PoppySessionRow>> {
        let db = self.database();
        Ok(sqlx::query_as::<_, PoppySessionRow>(
            "SELECT id,channel_id,client_id,user_id,account,scope,created_at,renewed_at,ended_at FROM poppy_sessions WHERE id=$1",
        )
        .bind(id)
        .fetch_optional(db.pool())
        .await?)
    }

    /// Record that the Session got a new token.
    pub async fn renew_poppy_session(&self, id: &str) -> Result<()> {
        let db = self.database();
        sqlx::query("UPDATE poppy_sessions SET renewed_at=NOW() WHERE id=$1 AND ended_at IS NULL")
            .bind(id)
            .execute(db.pool())
            .await?;
        Ok(())
    }

    pub async fn create_poppy_conversation(
        &self,
        session_id: Uuid,
        owner: PoppyOwner<'_>,
        context: &serde_json::Value,
    ) -> Result<()> {
        let db = self.database();
        sqlx::query("INSERT INTO poppy_conversations(session_id,channel_id,client_id,user_id,context) VALUES($1,$2,$3,$4,$5)")
            .bind(session_id).bind(owner.channel_id).bind(owner.client_id).bind(owner.user_id).bind(context)
            .execute(db.pool()).await?;
        Ok(())
    }

    /// The conversation backed by `session_id`, when `owner` owns it.
    pub async fn poppy_conversation(
        &self,
        session_id: Uuid,
        owner: PoppyOwner<'_>,
    ) -> Result<Option<PoppyConversationRow>> {
        let db = self.database();
        Ok(sqlx::query_as::<_, PoppyConversationRow>(
            "SELECT session_id,channel_id,client_id,user_id,account,context,closed_at,created_at FROM poppy_conversations WHERE session_id=$1 AND channel_id=$2 AND client_id=$3 AND user_id=$4",
        )
        .bind(session_id)
        .bind(owner.channel_id)
        .bind(owner.client_id)
        .bind(owner.user_id)
        .fetch_optional(db.pool())
        .await?)
    }

    pub async fn set_poppy_conversation_context(
        &self,
        session_id: Uuid,
        context: &serde_json::Value,
    ) -> Result<()> {
        let db = self.database();
        sqlx::query("UPDATE poppy_conversations SET context=$2 WHERE session_id=$1")
            .bind(session_id)
            .bind(context)
            .execute(db.pool())
            .await?;
        Ok(())
    }

    /// Close the conversation. Returns when it was closed, the first close's
    /// time when it already was.
    pub async fn close_poppy_conversation(&self, session_id: Uuid) -> Result<DateTime<Utc>> {
        let db = self.database();
        Ok(sqlx::query_scalar(
            "UPDATE poppy_conversations SET closed_at=COALESCE(closed_at,NOW()) WHERE session_id=$1 RETURNING closed_at",
        )
        .bind(session_id)
        .fetch_one(db.pool())
        .await?)
    }

    pub async fn poppy_message(
        &self,
        owner: PoppyOwner<'_>,
        message_id: &str,
    ) -> Result<Option<PoppyMessageRow>> {
        let db = self.database();
        Ok(sqlx::query_as::<_, PoppyMessageRow>(
            "SELECT session_id,content_hash FROM poppy_messages WHERE channel_id=$1 AND client_id=$2 AND user_id=$3 AND message_id=$4",
        )
        .bind(owner.channel_id)
        .bind(owner.client_id)
        .bind(owner.user_id)
        .bind(message_id)
        .fetch_optional(db.pool())
        .await?)
    }

    /// Claim `message_id` for this owner. `false` when it was already taken,
    /// by a retry or a concurrent request.
    pub async fn insert_poppy_message(
        &self,
        owner: PoppyOwner<'_>,
        message_id: &str,
        session_id: Uuid,
        content_hash: &[u8],
    ) -> Result<bool> {
        let db = self.database();
        let inserted = sqlx::query("INSERT INTO poppy_messages(channel_id,client_id,user_id,message_id,session_id,content_hash) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING")
            .bind(owner.channel_id).bind(owner.client_id).bind(owner.user_id).bind(message_id)
            .bind(session_id).bind(content_hash)
            .execute(db.pool()).await?
            .rows_affected();
        Ok(inserted == 1)
    }

    /// Release a claimed message id whose message could not be delivered, so
    /// a retry can deliver it.
    pub async fn delete_poppy_message(
        &self,
        owner: PoppyOwner<'_>,
        message_id: &str,
    ) -> Result<()> {
        let db = self.database();
        sqlx::query("DELETE FROM poppy_messages WHERE channel_id=$1 AND client_id=$2 AND user_id=$3 AND message_id=$4")
            .bind(owner.channel_id).bind(owner.client_id).bind(owner.user_id).bind(message_id)
            .execute(db.pool()).await?;
        Ok(())
    }
}
