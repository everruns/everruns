//! Persistent SQL tools for agent sessions. Stability: alpha.
/// Errors from validation, query execution and storage.
pub use everruns_contracts::session_sqldb::SessionSqlDbError as DatabaseError;
use std::{fmt, path::Path, sync::Arc};
/// A directory of session-scoped SQLite databases, outside the agent workspace.
#[derive(Clone)]
pub struct SqlDatabase {
    pub(crate) store: Arc<dyn everruns_contracts::session_sqldb::SessionSqlDbStore>,
}
impl SqlDatabase {
    /// Open private application storage. SQL can never select a filesystem path.
    pub fn open(directory: impl AsRef<Path>) -> Result<Self, DatabaseError> {
        Ok(Self {
            store: Arc::new(runtime::LocalSqlDbStore::open(directory)?),
        })
    }
}
impl fmt::Debug for SqlDatabase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SqlDatabase { .. }")
    }
}
impl crate::AgentBuilder {
    /// Add persistent, isolated SQL databases and the standard SQL tools.
    pub fn sql_database(mut self, database: SqlDatabase) -> Self {
        self.native_services.database = Some(database);
        self = self.capability("session_sql_database");
        self
    }
}

/// Shared SQL execution implementation for host adapters.
#[doc(hidden)]
pub mod runtime;
