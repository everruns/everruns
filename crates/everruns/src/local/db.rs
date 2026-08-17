// Shared SQLite connection handle for the local stores.
//
// Decision: a single `parking_lot::Mutex<rusqlite::Connection>` per database
// file. The local stores are intended for embedded, single-process hosts where
// throughput is modest and a serialized connection keeps the code simple and
// correct. Connections are opened with WAL so a freshly-spawned process can
// reopen the same file (restart-survivability) without losing committed data.

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rusqlite::Connection;

use super::error::{LocalError, LocalResult};

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
        let path = path.as_ref();
        ensure_private_db_file(path)?;
        let conn = Connection::open(path)?;
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
        conn.busy_timeout(Duration::from_secs(5))?;
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

    pub(crate) fn with_conn_mut<T>(
        &self,
        f: impl FnOnce(&mut Connection) -> rusqlite::Result<T>,
    ) -> LocalResult<T> {
        let mut guard = self.conn.lock();
        f(&mut guard).map_err(LocalError::from)
    }
}

#[cfg(unix)]
fn ensure_private_db_file(path: &Path) -> std::io::Result<()> {
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
    {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }

    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "refusing to use symlinked local SQLite database {}",
                path.display()
            ),
        ));
    }
    if !metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "local SQLite database path is not a regular file: {}",
                path.display()
            ),
        ));
    }
    if metadata.uid() != current_uid() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "local SQLite database is not owned by the current user: {}",
                path.display()
            ),
        ));
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_private_db_file(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn current_uid() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn open_creates_private_db_file() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("local.db");
        SqliteDb::open(&path).unwrap();

        let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
