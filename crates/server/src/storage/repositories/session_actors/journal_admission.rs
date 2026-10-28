//! Online admission for a server-scoped actor journal reader.

use super::Database;
use everruns_contracts::typed_id::SessionId;

impl Database {
    /// Check the server-resolved org/session and the current lease before a
    /// journal read. The session's own org must agree with the actor index.
    ///
    /// Admission linearizes at this statement's snapshot. A lease revocation
    /// committed before it is refused; a read admitted earlier may finish
    /// afterward. Do not cache this decision. Expiry uses the database clock,
    /// not a timestamp or TTL supplied by the executor.
    pub(crate) async fn actor_journal_read_allowed(
        &self,
        org_id: i64,
        session_id: SessionId,
        holder: &str,
        fence: i64,
    ) -> anyhow::Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM session_actors a \
             JOIN sessions s ON s.id = a.session_id \
             WHERE a.session_id = $1 AND a.org_id = $2 AND s.org_id = $2 \
               AND a.lease_holder = $3 AND a.lease_fence = $4 \
               AND a.lease_expires_at > clock_timestamp())",
        )
        .bind(session_id)
        .bind(org_id)
        .bind(holder)
        .bind(fence)
        .fetch_one(&self.pool)
        .await?)
    }
}
