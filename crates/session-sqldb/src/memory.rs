// In-memory backend for session SQL databases
//
// Keeps SQLite connections alive in a HashMap keyed by (session_id, db_name).
// Each connection is wrapped in a Mutex for thread-safe access.
// Connection is Send (rusqlite bundled) so Mutex<Connection> is Send + Sync.

use crate::error::SqlDbError;
use crate::executor::{configure_connection, execute_query, execute_statement, get_schema};
use crate::limits::{MAX_DATABASES_PER_SESSION, MAX_DB_SIZE_BYTES, MAX_SESSION_TOTAL_SIZE_BYTES};
use crate::types::{DatabaseInfo, SqlExecuteResult, SqlQueryResult, TableSchema};
use crate::validation::validate_database_name;
use chrono::Utc;
use everruns_core::typed_id::SessionId;
use parking_lot::{Mutex, RwLock};
use rusqlite::{Connection, DatabaseName};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::debug;

/// Key for database lookup: (session_id, database_name)
type DbKey = (SessionId, String);

/// Stored database state
struct StoredDb {
    info: DatabaseInfo,
    /// Live SQLite connection
    conn: Arc<Mutex<Connection>>,
}

/// In-memory backend using live SQLite connections.
pub struct InMemorySqlDbBackend {
    databases: RwLock<HashMap<DbKey, StoredDb>>,
}

impl InMemorySqlDbBackend {
    pub fn new() -> Self {
        Self {
            databases: RwLock::new(HashMap::new()),
        }
    }

    /// Create a new configured in-memory connection.
    fn new_connection() -> Result<Connection, SqlDbError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| SqlDbError::Internal(format!("failed to open in-memory db: {e}")))?;
        configure_connection(&conn)?;
        Ok(conn)
    }

    /// Get the current database size by serializing (for tracking only).
    fn estimate_db_size(conn: &Connection) -> i64 {
        conn.serialize(DatabaseName::Main)
            .map(|data| data.len() as i64)
            .unwrap_or(0)
    }

    /// Update size tracking after a mutation.
    fn update_size(&self, session_id: SessionId, name: &str) {
        let key = (session_id, name.to_string());
        let mut dbs = self.databases.write();
        if let Some(stored) = dbs.get_mut(&key) {
            let conn = stored.conn.lock();
            let size_bytes = Self::estimate_db_size(&conn);
            stored.info.size_bytes = size_bytes;
            stored.info.page_count = ((size_bytes + 4095) / 4096) as i32;
            stored.info.updated_at = Utc::now();
        }
    }

    /// Check per-database and per-session size limits.
    fn check_size_limits(
        &self,
        session_id: SessionId,
        name: &str,
        new_size: i64,
    ) -> Result<(), SqlDbError> {
        if new_size > MAX_DB_SIZE_BYTES {
            return Err(SqlDbError::LimitExceeded(format!(
                "database size {} exceeds limit {}",
                new_size, MAX_DB_SIZE_BYTES
            )));
        }

        let dbs = self.databases.read();
        let other_size: i64 = dbs
            .iter()
            .filter(|(k, _)| k.0 == session_id && k.1 != name)
            .map(|(_, v)| v.info.size_bytes)
            .sum();
        if other_size + new_size > MAX_SESSION_TOTAL_SIZE_BYTES {
            return Err(SqlDbError::LimitExceeded(format!(
                "session total size would exceed limit {}",
                MAX_SESSION_TOTAL_SIZE_BYTES
            )));
        }

        Ok(())
    }

    /// Count databases for a session.
    fn count_databases(&self, session_id: SessionId) -> usize {
        self.databases
            .read()
            .keys()
            .filter(|(sid, _)| *sid == session_id)
            .count()
    }

    pub fn create_database(
        &self,
        session_id: SessionId,
        name: &str,
    ) -> Result<DatabaseInfo, SqlDbError> {
        validate_database_name(name)?;

        let key = (session_id, name.to_string());

        // Check if already exists
        if self.databases.read().contains_key(&key) {
            return Err(SqlDbError::DatabaseAlreadyExists(name.to_string()));
        }

        // Check database count limit
        if self.count_databases(session_id) >= MAX_DATABASES_PER_SESSION {
            return Err(SqlDbError::LimitExceeded(format!(
                "maximum {} databases per session",
                MAX_DATABASES_PER_SESSION
            )));
        }

        let conn = Self::new_connection()?;
        let now = Utc::now();
        let info = DatabaseInfo {
            name: name.to_string(),
            size_bytes: 0,
            page_count: 0,
            created_at: now,
            updated_at: now,
        };

        self.databases.write().insert(
            key,
            StoredDb {
                info: info.clone(),
                conn: Arc::new(Mutex::new(conn)),
            },
        );

        debug!(session_id = %session_id, name, "Created in-memory database");
        Ok(info)
    }

    pub fn get_database(
        &self,
        session_id: SessionId,
        name: &str,
    ) -> Result<Option<DatabaseInfo>, SqlDbError> {
        let key = (session_id, name.to_string());
        Ok(self.databases.read().get(&key).map(|d| d.info.clone()))
    }

    pub fn list_databases(&self, session_id: SessionId) -> Result<Vec<DatabaseInfo>, SqlDbError> {
        let dbs = self.databases.read();
        let mut result: Vec<DatabaseInfo> = dbs
            .iter()
            .filter(|(k, _)| k.0 == session_id)
            .map(|(_, v)| v.info.clone())
            .collect();
        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
    }

    pub fn delete_database(&self, session_id: SessionId, name: &str) -> Result<bool, SqlDbError> {
        let key = (session_id, name.to_string());
        Ok(self.databases.write().remove(&key).is_some())
    }

    /// Delete all databases for a session (used when session is deleted).
    pub fn delete_session_databases(&self, session_id: SessionId) {
        let mut dbs = self.databases.write();
        dbs.retain(|(sid, _), _| *sid != session_id);
    }

    pub fn query(
        &self,
        session_id: SessionId,
        db_name: &str,
        sql: &str,
    ) -> Result<SqlQueryResult, SqlDbError> {
        validate_database_name(db_name)?;

        let key = (session_id, db_name.to_string());
        let dbs = self.databases.read();
        let stored = dbs
            .get(&key)
            .ok_or_else(|| SqlDbError::DatabaseNotFound(db_name.to_string()))?;

        let conn = stored.conn.lock();
        execute_query(&conn, sql)
    }

    pub fn execute(
        &self,
        session_id: SessionId,
        db_name: &str,
        sql: &str,
    ) -> Result<SqlExecuteResult, SqlDbError> {
        validate_database_name(db_name)?;

        // Auto-create database if it doesn't exist
        if self.get_database(session_id, db_name)?.is_none() {
            self.create_database(session_id, db_name)?;
        }

        let key = (session_id, db_name.to_string());
        let result = {
            let dbs = self.databases.read();
            let stored = dbs
                .get(&key)
                .ok_or_else(|| SqlDbError::DatabaseNotFound(db_name.to_string()))?;
            let conn = stored.conn.lock();
            let result = execute_statement(&conn, sql)?;

            // Check size limits after mutation
            let new_size = Self::estimate_db_size(&conn);
            drop(conn);
            drop(dbs);
            self.check_size_limits(session_id, db_name, new_size)?;
            result
        };

        // Update size tracking
        self.update_size(session_id, db_name);
        Ok(result)
    }

    pub fn get_schema(
        &self,
        session_id: SessionId,
        db_name: &str,
    ) -> Result<Vec<TableSchema>, SqlDbError> {
        validate_database_name(db_name)?;

        let key = (session_id, db_name.to_string());
        let dbs = self.databases.read();
        let stored = dbs
            .get(&key)
            .ok_or_else(|| SqlDbError::DatabaseNotFound(db_name.to_string()))?;

        let conn = stored.conn.lock();
        get_schema(&conn)
    }
}

impl Default for InMemorySqlDbBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::typed_id::SessionId;

    fn session_id() -> SessionId {
        SessionId::from(uuid::Uuid::new_v4())
    }

    #[test]
    fn test_create_and_list() {
        let backend = InMemorySqlDbBackend::new();
        let sid = session_id();

        let info = backend.create_database(sid, "test_db").unwrap();
        assert_eq!(info.name, "test_db");
        assert_eq!(info.size_bytes, 0);

        let list = backend.list_databases(sid).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "test_db");
    }

    #[test]
    fn test_create_duplicate() {
        let backend = InMemorySqlDbBackend::new();
        let sid = session_id();

        backend.create_database(sid, "test_db").unwrap();
        let result = backend.create_database(sid, "test_db");
        assert!(matches!(result, Err(SqlDbError::DatabaseAlreadyExists(_))));
    }

    #[test]
    fn test_delete() {
        let backend = InMemorySqlDbBackend::new();
        let sid = session_id();

        backend.create_database(sid, "test_db").unwrap();
        assert!(backend.delete_database(sid, "test_db").unwrap());
        assert!(!backend.delete_database(sid, "test_db").unwrap());
        assert!(backend.list_databases(sid).unwrap().is_empty());
    }

    #[test]
    fn test_session_isolation() {
        let backend = InMemorySqlDbBackend::new();
        let sid1 = session_id();
        let sid2 = session_id();

        backend.create_database(sid1, "db").unwrap();
        backend.create_database(sid2, "db").unwrap();

        assert_eq!(backend.list_databases(sid1).unwrap().len(), 1);
        assert_eq!(backend.list_databases(sid2).unwrap().len(), 1);

        backend.delete_session_databases(sid1);
        assert!(backend.list_databases(sid1).unwrap().is_empty());
        assert_eq!(backend.list_databases(sid2).unwrap().len(), 1);
    }

    #[test]
    fn test_max_databases_per_session() {
        let backend = InMemorySqlDbBackend::new();
        let sid = session_id();

        for i in 0..MAX_DATABASES_PER_SESSION {
            backend.create_database(sid, &format!("db_{i}")).unwrap();
        }

        let result = backend.create_database(sid, "one_too_many");
        assert!(matches!(result, Err(SqlDbError::LimitExceeded(_))));
    }

    #[test]
    fn test_execute_and_query_lifecycle() {
        let backend = InMemorySqlDbBackend::new();
        let sid = session_id();

        // Auto-creates database
        backend
            .execute(
                sid,
                "mydb",
                "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)",
            )
            .unwrap();

        backend
            .execute(
                sid,
                "mydb",
                "INSERT INTO users VALUES (1, 'Alice'), (2, 'Bob')",
            )
            .unwrap();

        let result = backend
            .query(sid, "mydb", "SELECT * FROM users ORDER BY id")
            .unwrap();
        assert_eq!(result.row_count, 2);
        assert_eq!(result.columns, vec!["id", "name"]);

        // Verify data persists across calls
        let result2 = backend
            .query(sid, "mydb", "SELECT COUNT(*) as cnt FROM users")
            .unwrap();
        assert_eq!(result2.rows[0][0], serde_json::json!(2));
    }

    #[test]
    fn test_query_nonexistent_database() {
        let backend = InMemorySqlDbBackend::new();
        let sid = session_id();

        let result = backend.query(sid, "nope", "SELECT 1");
        assert!(matches!(result, Err(SqlDbError::DatabaseNotFound(_))));
    }

    #[test]
    fn test_schema() {
        let backend = InMemorySqlDbBackend::new();
        let sid = session_id();

        backend
            .execute(
                sid,
                "mydb",
                "CREATE TABLE t1 (id INTEGER PRIMARY KEY, val TEXT NOT NULL);
                 CREATE TABLE t2 (x REAL);
                 INSERT INTO t1 VALUES (1, 'a');",
            )
            .unwrap();

        let schema = backend.get_schema(sid, "mydb").unwrap();
        assert_eq!(schema.len(), 2);

        let t1 = schema.iter().find(|t| t.name == "t1").unwrap();
        assert_eq!(t1.row_count, 1);
        assert_eq!(t1.columns.len(), 2);
        assert!(t1.columns[0].pk);
        assert!(t1.columns[1].notnull);
    }

    #[test]
    fn test_size_tracking() {
        let backend = InMemorySqlDbBackend::new();
        let sid = session_id();

        backend
            .execute(sid, "mydb", "CREATE TABLE t (val TEXT)")
            .unwrap();

        let info = backend.get_database(sid, "mydb").unwrap().unwrap();
        // After creating a table, size should be > 0
        assert!(info.size_bytes > 0);
    }

    #[test]
    fn test_invalid_name_rejected() {
        let backend = InMemorySqlDbBackend::new();
        let sid = session_id();

        assert!(matches!(
            backend.create_database(sid, ""),
            Err(SqlDbError::InvalidDatabaseName(_))
        ));
        assert!(matches!(
            backend.create_database(sid, "1abc"),
            Err(SqlDbError::InvalidDatabaseName(_))
        ));
        assert!(matches!(
            backend.create_database(sid, "my-db"),
            Err(SqlDbError::InvalidDatabaseName(_))
        ));
    }
}
