//! Bind assignment authorization, journal page and head to one read snapshot.
use super::{
    ActorJournalRows, Database, WorkerJournalCapabilityBinding, WorkerJournalReadIdentity,
};

impl Database {
    pub(crate) async fn read_worker_assignment_journal(
        &self,
        binding: &WorkerJournalCapabilityBinding,
        identity: WorkerJournalReadIdentity<'_>,
        after_sequence: Option<i32>,
        limit: i64,
    ) -> anyhow::Result<Option<ActorJournalRows>> {
        let mut tx = everruns_durable::begin_transaction(&self.pool).await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        // THREAT[TM-AUTHZ-002]: the first snapshot also bounds data and head
        // reads. A retained admission cannot fetch events committed later.
        let floor = Self::worker_ingress_minimum_version_on(&mut tx).await?;
        let allowed = match identity {
            WorkerJournalReadIdentity::Assignment if floor.is_none() => {
                Self::actor_journal_read_allowed_on(&mut tx, binding).await?
            }
            WorkerJournalReadIdentity::Verified {
                certificate_sha256,
                capability_sha256,
            } if floor == Some(1) => {
                Self::resolve_worker_journal_capability_on(
                    &mut tx,
                    certificate_sha256,
                    capability_sha256,
                )
                .await?
                .as_ref()
                    == Some(binding)
            }
            _ => false,
        };
        if !allowed {
            tx.rollback().await?;
            return Ok(None);
        }
        let journal =
            Self::read_session_actor_journal_on(&mut tx, binding.session_id, after_sequence, limit)
                .await?;
        everruns_durable::commit_transaction(tx).await?;
        Ok(Some(journal))
    }
}
