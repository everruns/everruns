// SessionSqlDbStore trait implementation for InMemorySqlDbBackend
//
// Wraps synchronous InMemorySqlDbBackend calls with spawn_blocking
// since SQLite operations can take up to 30 seconds (query timeout).

use async_trait::async_trait;
use everruns_core::session_sqldb::{
    DatabaseInfo, SessionSqlDbError, SessionSqlDbStore, SqlExecuteResult, SqlQueryResult,
    TableSchema,
};
use everruns_core::typed_id::SessionId;
use std::sync::Arc;

use crate::memory::InMemorySqlDbBackend;

/// Async wrapper around InMemorySqlDbBackend implementing SessionSqlDbStore.
#[derive(Clone)]
pub struct InMemorySqlDbStore {
    inner: Arc<InMemorySqlDbBackend>,
}

impl InMemorySqlDbStore {
    pub fn new(backend: Arc<InMemorySqlDbBackend>) -> Self {
        Self { inner: backend }
    }

    /// Get a reference to the underlying backend.
    pub fn backend(&self) -> &Arc<InMemorySqlDbBackend> {
        &self.inner
    }
}

#[async_trait]
impl SessionSqlDbStore for InMemorySqlDbStore {
    async fn create_database(
        &self,
        session_id: SessionId,
        name: &str,
    ) -> Result<DatabaseInfo, SessionSqlDbError> {
        let inner = self.inner.clone();
        let name = name.to_string();
        tokio::task::spawn_blocking(move || inner.create_database(session_id, &name))
            .await
            .map_err(|e| SessionSqlDbError::Internal(format!("spawn_blocking failed: {e}")))?
            .map_err(Into::into)
    }

    async fn list_databases(
        &self,
        session_id: SessionId,
    ) -> Result<Vec<DatabaseInfo>, SessionSqlDbError> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || inner.list_databases(session_id))
            .await
            .map_err(|e| SessionSqlDbError::Internal(format!("spawn_blocking failed: {e}")))?
            .map_err(Into::into)
    }

    async fn get_database(
        &self,
        session_id: SessionId,
        name: &str,
    ) -> Result<Option<DatabaseInfo>, SessionSqlDbError> {
        let inner = self.inner.clone();
        let name = name.to_string();
        tokio::task::spawn_blocking(move || inner.get_database(session_id, &name))
            .await
            .map_err(|e| SessionSqlDbError::Internal(format!("spawn_blocking failed: {e}")))?
            .map_err(Into::into)
    }

    async fn delete_database(
        &self,
        session_id: SessionId,
        name: &str,
    ) -> Result<bool, SessionSqlDbError> {
        let inner = self.inner.clone();
        let name = name.to_string();
        tokio::task::spawn_blocking(move || inner.delete_database(session_id, &name))
            .await
            .map_err(|e| SessionSqlDbError::Internal(format!("spawn_blocking failed: {e}")))?
            .map_err(Into::into)
    }

    async fn sql_execute(
        &self,
        session_id: SessionId,
        db_name: &str,
        sql: &str,
    ) -> Result<SqlExecuteResult, SessionSqlDbError> {
        let inner = self.inner.clone();
        let db_name = db_name.to_string();
        let sql = sql.to_string();
        tokio::task::spawn_blocking(move || inner.execute(session_id, &db_name, &sql))
            .await
            .map_err(|e| SessionSqlDbError::Internal(format!("spawn_blocking failed: {e}")))?
            .map_err(Into::into)
    }

    async fn sql_query(
        &self,
        session_id: SessionId,
        db_name: &str,
        sql: &str,
    ) -> Result<SqlQueryResult, SessionSqlDbError> {
        let inner = self.inner.clone();
        let db_name = db_name.to_string();
        let sql = sql.to_string();
        tokio::task::spawn_blocking(move || inner.query(session_id, &db_name, &sql))
            .await
            .map_err(|e| SessionSqlDbError::Internal(format!("spawn_blocking failed: {e}")))?
            .map_err(Into::into)
    }

    async fn sql_schema(
        &self,
        session_id: SessionId,
        db_name: &str,
        table: Option<&str>,
    ) -> Result<Vec<TableSchema>, SessionSqlDbError> {
        let inner = self.inner.clone();
        let db_name = db_name.to_string();
        let table = table.map(|s| s.to_string());
        tokio::task::spawn_blocking(
            move || -> Result<Vec<TableSchema>, crate::error::SqlDbError> {
                let schemas = inner.get_schema(session_id, &db_name)?;
                match &table {
                    Some(table_name) => Ok(schemas
                        .into_iter()
                        .filter(|t| t.name == *table_name)
                        .collect()),
                    None => Ok(schemas),
                }
            },
        )
        .await
        .map_err(|e| SessionSqlDbError::Internal(format!("spawn_blocking failed: {e}")))?
        .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_id() -> SessionId {
        SessionId::from(uuid::Uuid::new_v4())
    }

    #[tokio::test]
    async fn test_store_create_and_list() {
        let backend = Arc::new(InMemorySqlDbBackend::new());
        let store = InMemorySqlDbStore::new(backend);
        let sid = session_id();

        let info = store.create_database(sid, "test_db").await.unwrap();
        assert_eq!(info.name, "test_db");

        let list = store.list_databases(sid).await.unwrap();
        assert_eq!(list.len(), 1);
    }

    #[tokio::test]
    async fn test_store_execute_and_query() {
        let backend = Arc::new(InMemorySqlDbBackend::new());
        let store = InMemorySqlDbStore::new(backend);
        let sid = session_id();

        store
            .sql_execute(
                sid,
                "mydb",
                "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)",
            )
            .await
            .unwrap();

        store
            .sql_execute(sid, "mydb", "INSERT INTO users VALUES (1, 'Alice')")
            .await
            .unwrap();

        let result = store
            .sql_query(sid, "mydb", "SELECT * FROM users")
            .await
            .unwrap();
        assert_eq!(result.row_count, 1);
        assert_eq!(result.columns, vec!["id", "name"]);
    }

    #[tokio::test]
    async fn test_store_schema() {
        let backend = Arc::new(InMemorySqlDbBackend::new());
        let store = InMemorySqlDbStore::new(backend);
        let sid = session_id();

        store
            .sql_execute(
                sid,
                "mydb",
                "CREATE TABLE t1 (id INTEGER PRIMARY KEY, val TEXT NOT NULL)",
            )
            .await
            .unwrap();

        let schema = store.sql_schema(sid, "mydb", None).await.unwrap();
        assert_eq!(schema.len(), 1);
        assert_eq!(schema[0].name, "t1");
        assert_eq!(schema[0].columns.len(), 2);

        // Filter to specific table
        let schema = store.sql_schema(sid, "mydb", Some("t1")).await.unwrap();
        assert_eq!(schema.len(), 1);

        let schema = store.sql_schema(sid, "mydb", Some("nope")).await.unwrap();
        assert!(schema.is_empty());
    }

    #[tokio::test]
    async fn test_store_delete() {
        let backend = Arc::new(InMemorySqlDbBackend::new());
        let store = InMemorySqlDbStore::new(backend);
        let sid = session_id();

        store.create_database(sid, "mydb").await.unwrap();
        assert!(store.delete_database(sid, "mydb").await.unwrap());
        assert!(!store.delete_database(sid, "mydb").await.unwrap());
    }

    #[tokio::test]
    async fn test_store_query_nonexistent() {
        let backend = Arc::new(InMemorySqlDbBackend::new());
        let store = InMemorySqlDbStore::new(backend);
        let sid = session_id();

        let result = store.sql_query(sid, "nope", "SELECT 1").await;
        assert!(result.is_err());
    }
}
