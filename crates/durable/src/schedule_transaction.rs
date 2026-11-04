//! Join schedule configuration writes to a host's transaction on the same pool.
//! The host owns BEGIN/COMMIT/rollback. Worker claims keep their own connections.
use sqlx::{PgConnection, PgPool, Postgres, pool::PoolConnection};
use std::{
    future::Future,
    ops::{Deref, DerefMut},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::sync::{Mutex, OwnedMutexGuard};

type SharedConnection = Arc<Mutex<Option<PoolConnection<Postgres>>>>;
#[derive(Clone)]
struct Scope {
    pool: usize,
    connection: SharedConnection,
    savepoints: Arc<AtomicUsize>,
}
tokio::task_local! { static CURRENT: Scope; }
pub fn same_pool(a: &PgPool, b: &PgPool) -> bool {
    Arc::ptr_eq(&a.connect_options(), &b.connect_options())
}
pub async fn scope<F: Future>(
    pool: &PgPool,
    connection: SharedConnection,
    savepoints: Arc<AtomicUsize>,
    future: F,
) -> F::Output {
    CURRENT
        .scope(
            Scope {
                pool: Arc::as_ptr(&pool.connect_options()) as usize,
                connection,
                savepoints,
            },
            future,
        )
        .await
}
pub(crate) enum ScheduleConnection {
    Owned(PoolConnection<Postgres>),
    Shared(OwnedMutexGuard<Option<PoolConnection<Postgres>>>),
}
impl Deref for ScheduleConnection {
    type Target = PgConnection;
    fn deref(&self) -> &PgConnection {
        match self {
            Self::Owned(connection) => connection,
            Self::Shared(guard) => guard.as_ref().expect("checked open transaction"),
        }
    }
}
impl DerefMut for ScheduleConnection {
    fn deref_mut(&mut self) -> &mut PgConnection {
        match self {
            Self::Owned(connection) => connection,
            Self::Shared(guard) => guard.as_mut().expect("checked open transaction"),
        }
    }
}
pub(crate) async fn connection(pool: &PgPool) -> Result<ScheduleConnection, sqlx::Error> {
    if let Ok(scope) = CURRENT.try_with(Clone::clone)
        && scope.pool == Arc::as_ptr(&pool.connect_options()) as usize
    {
        if scope.savepoints.load(Ordering::SeqCst) > 0 {
            return Err(sqlx::Error::Protocol(
                "Cannot change a schedule while a savepoint holds the transaction".into(),
            ));
        }
        let guard = scope.connection.lock_owned().await;
        if guard.is_none() {
            return Err(sqlx::Error::Protocol(
                "Schedule transaction is closed".into(),
            ));
        }
        return Ok(ScheduleConnection::Shared(guard));
    }
    Ok(ScheduleConnection::Owned(pool.acquire().await?))
}
