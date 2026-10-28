//! Resolve identity, opaque capability, grant and lease in one uncached admission.
use super::Database;
use super::rows::WorkerJournalCapabilityBinding;

impl Database {
    pub(crate) async fn resolve_worker_journal_capability(
        &self,
        certificate_sha256: &[u8; 32],
        capability_sha256: &[u8; 32],
    ) -> anyhow::Result<Option<WorkerJournalCapabilityBinding>> {
        let mut connection = self.pool.acquire().await?;
        Self::resolve_worker_journal_capability_on(
            &mut connection,
            certificate_sha256,
            capability_sha256,
        )
        .await
    }

    pub(super) async fn resolve_worker_journal_capability_on(
        connection: &mut sqlx::PgConnection,
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
             JOIN session_actors a ON a.session_id = g.actor_session_id AND a.org_id = g.org_id \
             LEFT JOIN agents ag ON ag.id=g.agent_id AND ag.org_id=g.org_id \
             WHERE c.capability_sha256 = $1 AND i.certificate_sha256 = $2 \
               AND c.revoked_at IS NULL AND c.created_at <= clock_timestamp() \
               AND c.expires_at > clock_timestamp() \
               AND i.revoked_at IS NULL AND i.created_at <= clock_timestamp() \
               AND i.expires_at > clock_timestamp() \
               AND g.revoked_at IS NULL AND g.created_at <= clock_timestamp() \
               AND g.expires_at > clock_timestamp() \
               AND a.lease_holder = g.lease_holder AND a.lease_fence = g.lease_fence \
               AND a.lease_expires_at > clock_timestamp() \
               AND a.cancel_requested_at IS NULL AND s.archived_at IS NULL \
               AND g.actor_authorization_epoch=a.worker_authorization_epoch \
               AND g.session_authorization_epoch=s.worker_authorization_epoch \
               AND g.agent_id IS NOT DISTINCT FROM s.agent_id \
               AND (g.agent_id IS NULL OR (ag.status='active' \
                    AND g.agent_authorization_epoch=ag.worker_authorization_epoch)) \
               AND EXISTS (SELECT 1 FROM worker_journal_grant_dependencies d WHERE d.grant_id=g.id) \
               AND NOT EXISTS (SELECT 1 FROM worker_journal_grant_dependencies d \
                 LEFT JOIN resource_grants rg ON rg.id=d.source_grant_id AND rg.org_id=g.org_id \
                 LEFT JOIN resource_access ra ON ra.id=d.source_resource_id AND ra.org_id=g.org_id \
                 WHERE d.grant_id=g.id AND (rg.id IS NULL OR ra.id IS NULL \
                   OR rg.resource_id<>ra.id OR rg.revoked_at IS NOT NULL OR ra.revoked_at IS NOT NULL \
                   OR rg.revision<>d.grant_revision OR ra.revision<>d.resource_revision \
                   OR rg.worker_authorization_epoch<>d.grant_epoch \
                   OR ra.worker_authorization_epoch<>d.resource_epoch \
                   OR (rg.expires_at IS NOT NULL AND rg.expires_at<=clock_timestamp()) \
                   OR rg.consumer_scope<>'session' OR rg.consumer_session_id IS DISTINCT FROM g.session_id \
                   OR rg.usage<>'service' OR ra.scope<>'organization' OR ra.usage<>'service'))",
        )
        .bind(capability_sha256.as_slice())
        .bind(certificate_sha256.as_slice())
        .fetch_optional(connection)
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
