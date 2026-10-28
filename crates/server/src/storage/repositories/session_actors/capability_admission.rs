//! Resolve identity, opaque capability, grant and lease in one uncached admission.
use super::Database;
use super::rows::WorkerJournalCapabilityBinding;

impl Database {
    pub(crate) async fn resolve_worker_journal_capability(
        &self,
        certificate_sha256: &[u8; 32],
        capability_sha256: &[u8; 32],
    ) -> anyhow::Result<Option<WorkerJournalCapabilityBinding>> {
        Ok(sqlx::query_as(
            "SELECT g.id AS grant_id, g.executor_id, g.org_id, g.session_id, \
                    g.lease_holder AS holder, g.lease_fence AS fence \
             FROM worker_journal_capabilities c \
             JOIN worker_journal_read_grants g ON g.id = c.grant_id \
             JOIN worker_executor_identities i ON i.executor_id = g.executor_id \
             JOIN sessions s ON s.id = g.session_id AND s.org_id = g.org_id \
             JOIN session_actors a ON a.session_id = s.id AND a.org_id = g.org_id \
             WHERE c.capability_sha256 = $1 AND i.certificate_sha256 = $2 \
               AND c.revoked_at IS NULL AND c.created_at <= clock_timestamp() \
               AND c.expires_at > clock_timestamp() \
               AND i.revoked_at IS NULL AND i.created_at <= clock_timestamp() \
               AND i.expires_at > clock_timestamp() \
               AND g.revoked_at IS NULL AND g.created_at <= clock_timestamp() \
               AND g.expires_at > clock_timestamp() \
               AND a.lease_holder = g.lease_holder AND a.lease_fence = g.lease_fence \
               AND a.lease_expires_at > clock_timestamp()",
        )
        .bind(capability_sha256.as_slice())
        .bind(certificate_sha256.as_slice())
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Trusted control-plane operation only; no worker/public endpoint.
    pub async fn revoke_worker_executor_identity(
        &self,
        executor_id: uuid::Uuid,
        certificate_sha256: &[u8; 32],
    ) -> anyhow::Result<bool> {
        Ok(sqlx::query(
            "UPDATE worker_executor_identities SET revoked_at = clock_timestamp() \
             WHERE executor_id = $1 AND certificate_sha256 = $2 AND revoked_at IS NULL",
        )
        .bind(executor_id)
        .bind(certificate_sha256.as_slice())
        .execute(&self.pool)
        .await?
        .rows_affected()
            == 1)
    }

    /// Trusted org-scoped revocation; foreign/missing/already revoked is false.
    pub async fn revoke_worker_journal_capability(
        &self,
        org_id: i64,
        capability_sha256: &[u8; 32],
    ) -> anyhow::Result<bool> {
        Ok(sqlx::query(
            "UPDATE worker_journal_capabilities c SET revoked_at = clock_timestamp() \
             FROM worker_journal_read_grants g \
             WHERE c.grant_id = g.id AND g.org_id = $1 \
               AND c.capability_sha256 = $2 AND c.revoked_at IS NULL",
        )
        .bind(org_id)
        .bind(capability_sha256.as_slice())
        .execute(&self.pool)
        .await?
        .rows_affected()
            == 1)
    }
}
