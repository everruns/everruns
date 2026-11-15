//! Persistent, session-scoped databases under an application-owned directory.
use super::{executor, limits::*, validation::validate_database_name};
use async_trait::async_trait;
use chrono::Utc;
use everruns_contracts::{session_sqldb::*, typed_id::SessionId};
use rusqlite::Connection;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub struct LocalSqlDbStore {
    root: Arc<PathBuf>,
    gate: Arc<Mutex<()>>,
}
impl LocalSqlDbStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, SessionSqlDbError> {
        let mut directories = std::fs::DirBuilder::new();
        directories.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            directories.mode(0o700);
        }
        directories.create(root.as_ref()).map_err(internal)?;
        let root = std::fs::canonicalize(root).map_err(internal)?;
        Ok(Self {
            root: Arc::new(root),
            gate: Arc::new(Mutex::new(())),
        })
    }
    fn directory(&self, session: SessionId) -> Result<PathBuf, SessionSqlDbError> {
        let dir = self.root.join(session.to_string());
        std::fs::create_dir_all(&dir).map_err(internal)?;
        if std::fs::symlink_metadata(&dir)
            .map_err(internal)?
            .file_type()
            .is_symlink()
        {
            return Err(SessionSqlDbError::InvalidDatabaseName(
                "session directory is a symlink".into(),
            ));
        }
        if !std::fs::canonicalize(&dir)
            .map_err(internal)?
            .starts_with(self.root.as_ref())
        {
            return Err(SessionSqlDbError::InvalidDatabaseName(
                "session directory escapes storage".into(),
            ));
        }
        Ok(dir)
    }
    fn path(&self, session: SessionId, name: &str) -> Result<PathBuf, SessionSqlDbError> {
        validate_database_name(name).map_err(SessionSqlDbError::from)?;
        let file = self.directory(session)?.join(format!("{name}.sqlite"));
        if let Ok(metadata) = std::fs::symlink_metadata(&file)
            && (!metadata.is_file() || metadata.file_type().is_symlink())
        {
            return Err(SessionSqlDbError::InvalidDatabaseName(
                "database must be a regular file".into(),
            ));
        }
        Ok(file)
    }
    fn info(path: &Path, name: &str) -> Result<DatabaseInfo, SessionSqlDbError> {
        let metadata = path.metadata().map_err(internal)?;
        Ok(DatabaseInfo {
            name: name.into(),
            size_bytes: metadata.len() as i64,
            page_count: (metadata.len() / PAGE_SIZE as u64) as i32,
            created_at: metadata
                .created()
                .map(chrono::DateTime::<Utc>::from)
                .unwrap_or_else(|_| Utc::now()),
            updated_at: metadata
                .modified()
                .map(chrono::DateTime::<Utc>::from)
                .unwrap_or_else(|_| Utc::now()),
        })
    }
    fn files(&self, session: SessionId) -> Result<Vec<PathBuf>, SessionSqlDbError> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(self.directory(session)?).map_err(internal)? {
            let entry = entry.map_err(internal)?;
            if entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "sqlite")
            {
                let metadata = entry.file_type().map_err(internal)?;
                if metadata.is_symlink() || !metadata.is_file() {
                    return Err(internal("invalid database file"));
                }
                files.push(entry.path());
            }
        }
        files.sort();
        Ok(files)
    }
    fn connect(
        &self,
        session: SessionId,
        name: &str,
        create: bool,
    ) -> Result<Connection, SessionSqlDbError> {
        let path = self.path(session, name)?;
        let files = self.files(session)?;
        if !path.exists() {
            if !create {
                return Err(SessionSqlDbError::DatabaseNotFound(name.into()));
            }
            if files.len() >= MAX_DATABASES_PER_SESSION {
                return Err(SessionSqlDbError::LimitExceeded(
                    "too many databases".into(),
                ));
            }
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(internal(error)),
            }
        }
        let connection = Connection::open(&path).map_err(internal)?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(internal)?;
        executor::configure_connection(&connection).map_err(SessionSqlDbError::from)?;
        // SQLite enforces this limit during a statement, before committing growth.
        let other_bytes: u64 = files
            .iter()
            .filter(|file| **file != path)
            .map(|file| file.metadata().map(|m| m.len()).unwrap_or(0))
            .sum();
        let remaining = (MAX_SESSION_TOTAL_SIZE_BYTES as u64)
            .saturating_sub(other_bytes)
            .min(MAX_DB_SIZE_BYTES as u64);
        if remaining < PAGE_SIZE as u64 {
            return Err(SessionSqlDbError::LimitExceeded(
                "session database storage exhausted".into(),
            ));
        }
        connection
            .pragma_update(
                None,
                "max_page_count",
                (remaining / PAGE_SIZE as u64) as i64,
            )
            .map_err(internal)?;
        Ok(connection)
    }
    async fn blocking<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Self) -> Result<T, SessionSqlDbError> + Send + 'static,
    ) -> Result<T, SessionSqlDbError> {
        let store = self.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = store
                .gate
                .lock()
                .map_err(|_| internal("database lock poisoned"))?;
            // THREAT[TM-FW-004]: file locks fence quota checks across stores and processes.
            let path = store.root.join(".lock");
            if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(internal("invalid storage lock"));
            }
            let mut options = std::fs::OpenOptions::new();
            options.read(true).write(true).create(true).truncate(false);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let lock = options.open(path).map_err(internal)?;
            lock.lock().map_err(internal)?;
            f(&store)
        })
        .await
        .map_err(internal)?
    }
}
fn internal(_error: impl std::fmt::Display) -> SessionSqlDbError {
    SessionSqlDbError::Internal("Local SQL storage operation failed".into())
}
#[async_trait]
impl SessionSqlDbStore for LocalSqlDbStore {
    async fn create_database(
        &self,
        session: SessionId,
        name: &str,
    ) -> Result<DatabaseInfo, SessionSqlDbError> {
        let name = name.to_string();
        self.blocking(move |s| {
            let path = s.path(session, &name)?;
            if path.exists() {
                return Err(SessionSqlDbError::DatabaseAlreadyExists(name));
            }
            let connection = s.connect(session, &name, true)?;
            drop(connection);
            Self::info(&path, &name)
        })
        .await
    }
    async fn list_databases(
        &self,
        session: SessionId,
    ) -> Result<Vec<DatabaseInfo>, SessionSqlDbError> {
        self.blocking(move |s| {
            s.files(session)?
                .iter()
                .map(|path| {
                    Self::info(
                        path,
                        path.file_stem()
                            .and_then(|v| v.to_str())
                            .unwrap_or_default(),
                    )
                })
                .collect()
        })
        .await
    }
    async fn get_database(
        &self,
        session: SessionId,
        name: &str,
    ) -> Result<Option<DatabaseInfo>, SessionSqlDbError> {
        let name = name.to_string();
        self.blocking(move |s| {
            let path = s.path(session, &name)?;
            if path.exists() {
                Self::info(&path, &name).map(Some)
            } else {
                Ok(None)
            }
        })
        .await
    }
    async fn delete_database(
        &self,
        session: SessionId,
        name: &str,
    ) -> Result<bool, SessionSqlDbError> {
        let name = name.to_string();
        self.blocking(move |s| {
            let path = s.path(session, &name)?;
            match std::fs::remove_file(path) {
                Ok(()) => Ok(true),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(e) => Err(internal(e)),
            }
        })
        .await
    }
    async fn sql_execute(
        &self,
        session: SessionId,
        name: &str,
        sql: &str,
    ) -> Result<SqlExecuteResult, SessionSqlDbError> {
        let (name, sql) = (name.to_string(), sql.to_string());
        self.blocking(move |s| {
            executor::execute_statement(&mut s.connect(session, &name, true)?, &sql)
                .map_err(Into::into)
        })
        .await
    }
    async fn sql_query(
        &self,
        session: SessionId,
        name: &str,
        sql: &str,
    ) -> Result<SqlQueryResult, SessionSqlDbError> {
        let (name, sql) = (name.to_string(), sql.to_string());
        self.blocking(move |s| {
            executor::execute_query(&mut s.connect(session, &name, false)?, &sql)
                .map_err(Into::into)
        })
        .await
    }
    async fn sql_schema(
        &self,
        session: SessionId,
        name: &str,
        table: Option<&str>,
    ) -> Result<Vec<TableSchema>, SessionSqlDbError> {
        let (name, table) = (name.to_string(), table.map(str::to_string));
        self.blocking(move |s| {
            let mut connection = s.connect(session, &name, false)?;
            if let Some(table) = table {
                super::authorizer::install_authorizer(&mut connection)
                    .map_err(SessionSqlDbError::from)?;
                executor::get_table_schema(&connection, &table)
                    .map(|t| vec![t])
                    .map_err(Into::into)
            } else {
                executor::get_schema(&mut connection).map_err(Into::into)
            }
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn reopened_stores_share_limits_and_never_follow_session_or_database_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let a = LocalSqlDbStore::open(root.path()).unwrap();
        let b = LocalSqlDbStore::open(root.path()).unwrap();
        let session = SessionId::new();
        let (a_result, b_result) = tokio::join!(
            a.sql_execute(session, "a", "CREATE TABLE t(v)"),
            b.sql_execute(session, "b", "CREATE TABLE t(v)")
        );
        a_result.unwrap();
        b_result.unwrap();
        for i in 2..MAX_DATABASES_PER_SESSION {
            a.create_database(session, &format!("db_{i}"))
                .await
                .unwrap();
        }
        assert!(matches!(
            b.create_database(session, "overflow").await,
            Err(SessionSqlDbError::LimitExceeded(_))
        ));
        assert!(
            a.sql_execute(session, "../escape", "SELECT 1")
                .await
                .is_err()
        );
        for sql in [
            "ATTACH DATABASE ':memory:' AS escape",
            "SELECT load_extension('x')",
            "CREATE VIRTUAL TABLE bad USING fts5(text)",
        ] {
            assert!(a.sql_execute(session, "a", sql).await.is_err(), "{sql}");
        }
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            let other = SessionId::new();
            std::os::unix::fs::symlink(outside.path(), root.path().join(other.to_string()))
                .unwrap();
            assert!(
                a.sql_execute(other, "a", "CREATE TABLE escaped(v)")
                    .await
                    .is_err()
            );
            assert!(std::fs::read_dir(outside.path()).unwrap().next().is_none());
            let file = outside.path().join("secret");
            std::fs::write(&file, b"private").unwrap();
            std::os::unix::fs::symlink(
                &file,
                root.path().join(session.to_string()).join("evil.sqlite"),
            )
            .unwrap();
            assert!(a.sql_query(session, "evil", "SELECT 1").await.is_err());
            assert_eq!(std::fs::read(&file).unwrap(), b"private");
        }
    }
}
