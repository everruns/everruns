//! Online admission for a server-scoped actor journal reader.

use super::Database;
use everruns_contracts::typed_id::SessionId;

impl Database {
    /// Check the server-resolved executor/grant and current lease together.
    /// Grant and actor orgs must also agree with the session's actual owner.
    ///
    /// Admission linearizes at this statement's snapshot. A grant or lease revocation
    /// committed before it is refused; a read admitted earlier may finish
    /// afterward. Do not cache this decision. Expiry uses the database clock,
    /// not a timestamp or TTL supplied by the executor.
    pub(crate) async fn actor_journal_read_allowed(
        &self,
        org_id: i64,
        session_id: SessionId,
        holder: &str,
        fence: i64,
        grant_id: uuid::Uuid,
        executor_id: uuid::Uuid,
    ) -> anyhow::Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM session_actors a \
             JOIN sessions s ON s.id = a.session_id \
             JOIN worker_journal_read_grants g ON g.session_id = a.session_id \
             LEFT JOIN agents ag ON ag.id=g.agent_id AND ag.org_id=g.org_id \
             WHERE a.session_id = $1 AND a.org_id = $2 AND s.org_id = $2 \
               AND a.lease_holder = $3 AND a.lease_fence = $4 \
               AND a.lease_expires_at > clock_timestamp() \
               AND g.id = $5 AND g.executor_id = $6 AND g.org_id = $2 \
               AND g.lease_holder = $3 AND g.lease_fence = $4 \
               AND g.revoked_at IS NULL AND g.created_at <= clock_timestamp() \
               AND g.expires_at > clock_timestamp() \
               AND g.actor_session_id=a.session_id AND a.cancel_requested_at IS NULL \
               AND s.archived_at IS NULL \
               AND g.actor_authorization_epoch=a.worker_authorization_epoch \
               AND g.session_authorization_epoch=s.worker_authorization_epoch \
               AND g.agent_id IS NOT DISTINCT FROM s.agent_id \
               AND (g.agent_id IS NULL OR (ag.status='active' \
                    AND g.agent_authorization_epoch=ag.worker_authorization_epoch)))",
        )
        .bind(session_id)
        .bind(org_id)
        .bind(holder)
        .bind(fence)
        .bind(grant_id)
        .bind(executor_id)
        .fetch_one(&self.pool)
        .await?)
    }
    /// Revoke one journal authorization under the trusted control plane's org.
    /// No worker RPC or public route exposes this operation. Missing, foreign
    /// and already revoked references all return false. Revocation is monotonic.
    pub async fn revoke_worker_journal_read_grant(
        &self,
        org_id: i64,
        grant_id: uuid::Uuid,
    ) -> anyhow::Result<bool> {
        Ok(sqlx::query(
            "UPDATE worker_journal_read_grants SET revoked_at = clock_timestamp() \
             WHERE id = $1 AND org_id = $2 AND revoked_at IS NULL",
        )
        .bind(grant_id)
        .bind(org_id)
        .execute(&self.pool)
        .await?
        .rows_affected()
            == 1)
    }
}
