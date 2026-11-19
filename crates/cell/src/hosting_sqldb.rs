//! A session's SQL databases over the cell, as the `sql_database` tools'
//! store (`SessionSqlDbStore`) and the Databases API's. The index and its
//! rules are `sqldb.rs`. Split from `hosting.rs` for size.
//!
//! Decisions:
//! - SQLite runs on a working copy in a local directory of this process,
//!   one per session, with the shared executor, authorizer and limits
//!   (`everruns::database::runtime::LocalSqlDbStore`, opened per session so
//!   a slow query of one session never waits on another's lock). Before a
//!   call the copy is brought to the index: a file whose hash the index no
//!   longer names is dropped, and the database the call names is fetched
//!   from the object store if the copy lacks it. A warm session runs its
//!   queries without a round trip.
//! - After a call that may write, the file's hash decides: unchanged, no
//!   commit; changed, the file is stored by its hash and committed over the
//!   hash the call started from. A refused commit drops the copy's file, so
//!   the next call starts from the index again, and the call fails: a write
//!   the session does not hold is never reported as done.
//! - One call per session at a time in a process. Copies of sessions not in
//!   use are dropped beyond [`COPIES_MAX`]; the object store keeps the truth.

use super::*;

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::OnceLock;

use everruns::database::runtime::LocalSqlDbStore;
use everruns_contracts::session_sqldb::{
    DatabaseInfo, SessionSqlDbError, SessionSqlDbStore, SqlExecuteResult, SqlQueryResult,
    TableSchema,
};

use crate::sqldb::{CONFLICT, Database, LIMIT, SqlChange};

type SqlResult<T> = std::result::Result<T, SessionSqlDbError>;

/// Sessions whose working copies a process keeps while not in use.
const COPIES_MAX: usize = 64;

/// A session's SQL databases in this hosting's cell.
#[derive(Clone)]
pub struct CellSqlDb {
    inner: Arc<Inner>,
    /// Whether writes commit under the epoch this hosting holds: the tools'
    /// do, the Databases API's do not (a gateway write).
    fenced: bool,
}

impl RaftHosting {
    /// Sessions' SQL databases as their tools use them: fenced writes.
    pub fn databases(&self) -> CellSqlDb {
        CellSqlDb {
            inner: self.inner.clone(),
            fenced: true,
        }
    }

    /// Sessions' SQL databases as the Databases API changes them: unfenced,
    /// like any gateway write.
    pub fn databases_api(&self) -> CellSqlDb {
        CellSqlDb {
            inner: self.inner.clone(),
            fenced: false,
        }
    }
}

/// One session's working copy.
struct Copy {
    /// The session's databases, as `{dir}/{session}/{name}.sqlite`.
    store: LocalSqlDbStore,
    dir: PathBuf,
    /// The file hash each database in the copy holds.
    held: BTreeMap<String, String>,
}

#[derive(Default)]
struct Copies {
    by_session: HashMap<SessionId, Arc<tokio::sync::Mutex<Option<Copy>>>>,
    /// Least recently used first.
    order: VecDeque<SessionId>,
}

struct Workdir {
    root: PathBuf,
    copies: parking_lot::Mutex<Copies>,
}

fn workdir() -> &'static Workdir {
    static WORKDIR: OnceLock<Workdir> = OnceLock::new();
    WORKDIR.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("everruns-cell-sqldb-{}", std::process::id()));
        // Copies of an earlier process with this id are not this one's.
        let _ = std::fs::remove_dir_all(&root);
        Workdir {
            root,
            copies: Default::default(),
        }
    })
}

impl Workdir {
    /// The session's copy slot, marked as just used; drops the copies of the
    /// least recently used sessions not in use beyond [`COPIES_MAX`].
    fn slot(&self, session: SessionId) -> Arc<tokio::sync::Mutex<Option<Copy>>> {
        let mut copies = self.copies.lock();
        copies.order.retain(|s| *s != session);
        copies.order.push_back(session);
        let slot = copies.by_session.entry(session).or_default().clone();
        let mut excess = copies.by_session.len().saturating_sub(COPIES_MAX);
        let mut index = 0;
        while excess > 0 && index < copies.order.len() {
            let candidate = copies.order[index];
            let idle = copies
                .by_session
                .get(&candidate)
                .is_some_and(|slot| Arc::strong_count(slot) == 1);
            if candidate != session && idle {
                copies.order.remove(index);
                if let Some(slot) = copies.by_session.remove(&candidate)
                    && let Ok(mut copy) = slot.try_lock()
                    && let Some(copy) = copy.take()
                {
                    let _ = std::fs::remove_dir_all(copy.dir);
                }
                excess -= 1;
            } else {
                index += 1;
            }
        }
        slot
    }
}

fn internal(error: impl std::fmt::Display) -> SessionSqlDbError {
    SessionSqlDbError::Internal(error.to_string())
}

fn sha256_of(bytes: &[u8]) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(bytes))
}

impl CellSqlDb {
    fn store(&self) -> Result<crate::store::Store> {
        match &self.inner.source {
            Source::Node(_) => Ok(self.inner.node()?.shared.store.clone()),
            Source::Remote(remote) => Ok(remote.shared.store.clone()),
        }
    }

    /// The session's databases as its state holds them.
    pub(crate) async fn index(&self, session: SessionId) -> Result<BTreeMap<String, Database>> {
        self.inner.read(session, |s| s.sqldb.entries.clone()).await
    }

    async fn index_sql(&self, session: SessionId) -> SqlResult<BTreeMap<String, Database>> {
        self.index(session).await.map_err(internal)
    }

    /// Run `f` on the session's working copy, brought to `index` and holding
    /// `wanted` if the index has it.
    async fn with_copy<T>(
        &self,
        session: SessionId,
        index: &BTreeMap<String, Database>,
        wanted: Option<&str>,
        f: impl AsyncFnOnce(&mut Copy) -> SqlResult<T>,
    ) -> SqlResult<T> {
        let slot = workdir().slot(session);
        let mut guard = slot.lock().await;
        if guard.is_none() {
            let dir = workdir().root.join(session.to_string());
            let opened = dir.clone();
            let store = tokio::task::spawn_blocking(move || LocalSqlDbStore::open(opened))
                .await
                .map_err(internal)??;
            *guard = Some(Copy {
                store,
                dir,
                held: BTreeMap::new(),
            });
        }
        let Some(copy) = guard.as_mut() else {
            return Err(internal("no working copy"));
        };
        self.sync(session, copy, index, wanted).await?;
        f(copy).await
    }

    /// Bring `copy` to `index`: drop what the index no longer names, fetch
    /// `wanted`.
    async fn sync(
        &self,
        session: SessionId,
        copy: &mut Copy,
        index: &BTreeMap<String, Database>,
        wanted: Option<&str>,
    ) -> SqlResult<()> {
        let files = copy.dir.join(session.to_string());
        let stale: Vec<String> = copy
            .held
            .iter()
            .filter(|(name, sha)| index.get(*name).map(|d| &d.sha256) != Some(*sha))
            .map(|(name, _)| name.clone())
            .collect();
        for name in stale {
            copy.held.remove(&name);
        }
        // Anything on disk the copy does not hold is a leftover of a failed
        // call: drop it, so the local limits count only real databases.
        let held: Vec<String> = copy.held.keys().cloned().collect();
        let scan = files.clone();
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            let entries = match std::fs::read_dir(&scan) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error),
            };
            for entry in entries {
                let path = entry?.path();
                let name = path
                    .file_stem()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
                    .to_owned();
                if !held.contains(&name) {
                    std::fs::remove_file(&path)?;
                }
            }
            Ok(())
        })
        .await
        .map_err(internal)?
        .map_err(internal)?;
        let Some(name) = wanted else {
            return Ok(());
        };
        let Some(database) = index.get(name) else {
            return Ok(());
        };
        if copy.held.contains_key(name) {
            return Ok(());
        }
        let bytes = self
            .store()
            .map_err(internal)?
            .get_blob(&database.sha256)
            .await
            .map_err(internal)?;
        if sha256_of(&bytes) != database.sha256 {
            return Err(internal("a database file is not the one its session names"));
        }
        let (dir, file) = (files.clone(), files.join(format!("{name}.sqlite")));
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            std::fs::create_dir_all(&dir)?;
            let partial = file.with_extension("partial");
            std::fs::write(&partial, &bytes)?;
            std::fs::rename(partial, file)
        })
        .await
        .map_err(internal)?
        .map_err(internal)?;
        copy.held.insert(name.to_owned(), database.sha256.clone());
        Ok(())
    }

    /// Commit the copy's file of `name` if it changed from `before`.
    async fn publish(
        &self,
        session: SessionId,
        copy: &mut Copy,
        name: &str,
        before: Option<&Database>,
    ) -> SqlResult<()> {
        let file = copy
            .dir
            .join(session.to_string())
            .join(format!("{name}.sqlite"));
        let bytes = match tokio::task::spawn_blocking(move || std::fs::read(file))
            .await
            .map_err(internal)?
        {
            Ok(bytes) => bytes,
            // The call made no database (a refused name, say).
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(internal(error)),
        };
        let sha256 = sha256_of(&bytes);
        if before.is_some_and(|d| d.sha256 == sha256) {
            copy.held.insert(name.to_owned(), sha256);
            return Ok(());
        }
        // Not held until committed: a failure below leaves the file a
        // leftover the next call drops.
        copy.held.remove(name);
        let size = bytes.len() as u64;
        self.store()
            .map_err(internal)?
            .put_blob(bytes)
            .await
            .map_err(internal)?;
        let change = SqlChange::Put {
            name: name.to_owned(),
            sha256: sha256.clone(),
            size,
            expected: before.map(|d| d.sha256.clone()),
        };
        self.commit(session, change).await?;
        copy.held.insert(name.to_owned(), sha256);
        Ok(())
    }

    /// Commit `change` under the epoch held here, if fenced; its reply.
    async fn commit(&self, session: SessionId, change: SqlChange) -> SqlResult<Value> {
        let fence = if self.fenced {
            self.inner.epochs.lock().get(&session).copied()
        } else {
            None
        };
        let response = self
            .inner
            .propose(
                session,
                Command::SqlDb {
                    session,
                    fence,
                    op_id: uuid::Uuid::new_v4().to_string(),
                    change,
                    now_ms: now_ms(),
                },
            )
            .await
            .map_err(internal)?;
        let value = match response {
            Response::Value { value } => value,
            Response::Fenced => {
                return Err(internal("database write after the session's turn moved on"));
            }
            other => return Err(internal(refused(session, other))),
        };
        let reason = || value["reason"].as_str().unwrap_or_default().to_owned();
        match value["refused"].as_str() {
            None => Ok(value),
            Some(LIMIT) => Err(SessionSqlDbError::LimitExceeded(reason())),
            Some(CONFLICT) => Err(internal(reason())),
            Some(_) => Err(SessionSqlDbError::InvalidDatabaseName(reason())),
        }
    }

    /// Run a call that may write `name` (creating it), and commit its file.
    async fn write<T: Send + 'static>(
        &self,
        session: SessionId,
        name: &str,
        call: impl AsyncFnOnce(&LocalSqlDbStore) -> SqlResult<T>,
    ) -> SqlResult<T> {
        let index = self.index_sql(session).await?;
        let before = index.get(name).cloned();
        self.with_copy(session, &index, Some(name), async |copy| {
            let result = call(&copy.store).await;
            // Commit even after a failed statement: the call may have made
            // the database, as every host's `sql_execute` does.
            self.publish(session, copy, name, before.as_ref()).await?;
            result
        })
        .await
    }

    /// Run a read-only call on `name`.
    async fn read<T: Send + 'static>(
        &self,
        session: SessionId,
        name: &str,
        call: impl AsyncFnOnce(&LocalSqlDbStore) -> SqlResult<T>,
    ) -> SqlResult<T> {
        let index = self.index_sql(session).await?;
        if !index.contains_key(name) {
            return Err(SessionSqlDbError::DatabaseNotFound(name.to_owned()));
        }
        self.with_copy(session, &index, Some(name), async |copy| {
            call(&copy.store).await
        })
        .await
    }
}

#[async_trait]
impl SessionSqlDbStore for CellSqlDb {
    async fn create_database(&self, session: SessionId, name: &str) -> SqlResult<DatabaseInfo> {
        everruns::database::runtime::validation::validate_database_name(name)
            .map_err(SessionSqlDbError::from)?;
        if self.index_sql(session).await?.contains_key(name) {
            return Err(SessionSqlDbError::DatabaseAlreadyExists(name.to_owned()));
        }
        self.write(session, name, async |store| {
            store.create_database(session, name).await
        })
        .await?;
        self.index_sql(session)
            .await?
            .get(name)
            .map(|d| d.info(name))
            .ok_or_else(|| internal("database missing after create"))
    }

    async fn list_databases(&self, session: SessionId) -> SqlResult<Vec<DatabaseInfo>> {
        Ok(self
            .index_sql(session)
            .await?
            .iter()
            .map(|(name, d)| d.info(name))
            .collect())
    }

    async fn get_database(
        &self,
        session: SessionId,
        name: &str,
    ) -> SqlResult<Option<DatabaseInfo>> {
        Ok(self
            .index_sql(session)
            .await?
            .get(name)
            .map(|d| d.info(name)))
    }

    async fn delete_database(&self, session: SessionId, name: &str) -> SqlResult<bool> {
        let value = self
            .commit(
                session,
                SqlChange::Delete {
                    name: name.to_owned(),
                },
            )
            .await?;
        // The copy's file goes at the next call, which finds it unheld.
        Ok(value["changed"] == true)
    }

    async fn sql_execute(
        &self,
        session: SessionId,
        db_name: &str,
        sql: &str,
    ) -> SqlResult<SqlExecuteResult> {
        self.write(session, db_name, async |store| {
            store.sql_execute(session, db_name, sql).await
        })
        .await
    }

    async fn sql_query(
        &self,
        session: SessionId,
        db_name: &str,
        sql: &str,
    ) -> SqlResult<SqlQueryResult> {
        self.read(session, db_name, async |store| {
            store.sql_query(session, db_name, sql).await
        })
        .await
    }

    async fn sql_schema(
        &self,
        session: SessionId,
        db_name: &str,
        table: Option<&str>,
    ) -> SqlResult<Vec<TableSchema>> {
        self.read(session, db_name, async |store| {
            store.sql_schema(session, db_name, table).await
        })
        .await
    }
}
