// Shared SQLite connection handle for the local stores.
//
// Decision: a single `parking_lot::Mutex<rusqlite::Connection>` per database
// file. The local stores are intended for embedded, single-process hosts where
// throughput is modest and a serialized connection keeps the code simple and
// correct. Connections are opened with WAL so a freshly-spawned process can
// reopen the same file (restart-survivability) without losing committed data.

use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use rusqlite::Connection;

use crate::error::{LocalError, LocalResult};

/// A shared, serialized SQLite connection. Cloning shares the underlying
/// connection (the `Arc<Mutex<..>>` is shared), so all stores built from the
/// same handle write to the same file.
#[derive(Clone)]
pub struct SqliteDb {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteDb {
    /// Open (creating if needed) a file-backed database at `path`.
    pub fn open(path: impl AsRef<Path>) -> LocalResult<Self> {
        let conn = Connection::open(path.as_ref())?;
        Self::from_connection(conn)
    }

    /// Open an in-memory database (for tests). Each call is an independent DB.
    pub fn open_in_memory() -> LocalResult<Self> {
        let conn = Connection::open_in_memory()?;
        Self::from_connection(conn)
    }

    fn from_connection(conn: Connection) -> LocalResult<Self> {
        // WAL improves concurrent read/write behavior and survives process
        // restarts; foreign_keys keeps message rows tied to their task.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Run a closure with exclusive access to the connection.
    pub fn with_conn<T>(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> LocalResult<T> {
        let guard = self.conn.lock();
        f(&guard).map_err(LocalError::from)
    }
}
