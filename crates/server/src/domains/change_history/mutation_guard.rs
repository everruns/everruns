//! Resource locks shared by ordinary updates and guarded history restores.
//! Nontransactional writers use a bounded separate connection: acquiring a
//! second connection from a one-connection request pool would deadlock.
use super::{Change, ChangeAction, EntityKind, SubjectId};
use crate::domains::common::{CommandError, Ctx};
use sqlx::{Connection, PgConnection};
use std::{
    future::Future,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::sync::Semaphore;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Key {
    org: i64,
    kind: EntityKind,
    reference: String,
}
impl Key {
    pub(crate) fn new(ctx: &Ctx, kind: EntityKind, reference: &str) -> Self {
        Self {
            org: if kind == EntityKind::UserProfile {
                0
            } else if kind == EntityKind::Schedule {
                everruns_core::DEFAULT_ORG_ID
            } else {
                ctx.org_id()
            },
            kind,
            reference: canonical_ref(kind, reference),
        }
    }
    fn text(&self) -> String {
        format!("{}:{}:{}", self.org, self.kind.as_str(), self.reference)
    }
}

pub(crate) fn canonical_ref(kind: EntityKind, reference: &str) -> String {
    super::snapshot::ref_uuid(reference).map_or_else(
        || reference.to_owned(),
        |id| {
            let prefix = match kind {
                EntityKind::OrganizationNetwork | EntityKind::OrganizationDecision => Some("org"),
                _ => kind.id_prefix(),
            };
            prefix.map_or_else(
                || id.to_string(),
                |prefix| format!("{prefix}_{}", id.simple()),
            )
        },
    )
}
tokio::task_local! { static HELD: Vec<Key>; }

pub(crate) fn subject(ctx: &Ctx, change: Change, params: &serde_json::Value) -> Option<Key> {
    let Change::Subject { kind, action, id } = change else {
        return None;
    };
    if kind == EntityKind::Session
        || matches!(
            action,
            ChangeAction::Created | ChangeAction::Forked | ChangeAction::Imported
        )
    {
        return None;
    }
    let reference = match id {
        SubjectId::CallerOrg => &ctx.caller.org_public_id,
        SubjectId::Param(field) => params.get(field)?.as_str()?,
        SubjectId::Output(_) => params
            .get(super::snapshot::restore_command(kind)?.1)?
            .as_str()?,
    };
    Some(Key::new(ctx, kind, reference))
}
fn busy() -> CommandError {
    CommandError::conflict("This resource is being changed. Keep your draft and try again.")
        .with_code("entity_change_busy")
}
pub(crate) async fn run<T, F>(
    ctx: &Ctx,
    key: Key,
    transactional: bool,
    future: F,
) -> Result<T, CommandError>
where
    F: Future<Output = Result<T, CommandError>> + Send,
    T: Send,
{
    let mut held = HELD.try_with(Clone::clone).unwrap_or_default();
    if held.contains(&key) {
        return future.await;
    }
    held.push(key.clone());
    let name = key.text();
    if transactional {
        return crate::storage::transaction::scope(
            &ctx.db,
            async {
                let wait = async {
                    loop {
                        let locked: bool = sqlx::query_scalar(
                            "SELECT pg_try_advisory_xact_lock(hashtextextended($1, 1314))",
                        )
                        .bind(&name)
                        .fetch_one(ctx.db.tx_pool())
                        .await
                        .map_err(|e| CommandError::internal(e.into()))?;
                        if locked {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    Ok::<(), CommandError>(())
                };
                tokio::time::timeout(Duration::from_secs(3), wait)
                    .await
                    .map_err(|_| busy())??;
                HELD.scope(held, future).await
            },
            CommandError::internal,
        )
        .await;
    }
    let lease = acquire(ctx, &key).await?;
    let result = HELD.scope(held, future).await;
    drop(lease);
    result
}

pub(crate) struct Lease {
    _connection: PgConnection,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

pub(crate) async fn acquire(ctx: &Ctx, key: &Key) -> Result<Lease, CommandError> {
    let name = key.text();
    static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    let slots = SLOTS.get_or_init(|| Arc::new(Semaphore::new(16))).clone();
    let _permit = tokio::time::timeout(Duration::from_secs(3), slots.acquire_owned())
        .await
        .map_err(|_| busy())?
        .map_err(|e| CommandError::internal(e.into()))?;
    let mut connection = tokio::time::timeout(
        Duration::from_secs(5),
        PgConnection::connect_with(&ctx.db.pool().connect_options()),
    )
    .await
    .map_err(|_| busy())?
    .map_err(|e| CommandError::internal(e.into()))?;
    let wait = async {
        loop {
            let locked: bool =
                sqlx::query_scalar("SELECT pg_try_advisory_lock(hashtextextended($1, 1314))")
                    .bind(&name)
                    .fetch_one(&mut connection)
                    .await
                    .map_err(|e| CommandError::internal(e.into()))?;
            if locked {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok::<(), CommandError>(())
    };
    tokio::time::timeout(Duration::from_secs(3), wait)
        .await
        .map_err(|_| busy())??;
    // Dropping a dedicated connection also releases the lock on cancellation.
    Ok(Lease {
        _connection: connection,
        _permit,
    })
}
