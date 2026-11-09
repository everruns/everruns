//! SQLite connection ownership for embedded hosts.
//!
//! Hosts keep their schemas and runtime projections. Opening connections,
//! consistent backups, and SQLite's process-wide VFS selection live here so
//! the database boundary covers embedded persistence as well as PostgreSQL.

use std::ops::{Deref, DerefMut};
use std::path::Path;

pub use rusqlite::{Error, OptionalExtension, Result, Row, TransactionBehavior, params, types};
// Existing embedded hosts pass query callbacks typed against this handle.
// The wrapper owns construction while preserving that callback type identity.
pub use rusqlite::Connection as QueryConnection;

/// An embedded database connection opened through this crate.
pub struct Connection(rusqlite::Connection);

impl Deref for Connection {
    type Target = rusqlite::Connection;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Connection {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Open or create a file-backed SQLite database.
pub fn open(path: impl AsRef<Path>) -> Result<Connection> {
    rusqlite::Connection::open(path).map(Connection)
}

/// Open an independent in-memory SQLite database.
pub fn open_in_memory() -> Result<Connection> {
    rusqlite::Connection::open_in_memory().map(Connection)
}

/// Make a consistent snapshot, including committed WAL contents.
pub fn backup(source: impl AsRef<Path>, destination: impl AsRef<Path>) -> Result<()> {
    rusqlite::Connection::open(source)?.backup(
        rusqlite::MAIN_DB,
        destination,
        rusqlite::backup::NO_PROGRESS,
    )
}

/// Select SQLite's built-in dot-file VFS for subsequent connection opens.
///
/// The caller must select this before opening databases. This process-wide
/// setting supports mounts that cannot provide POSIX byte-range locks.
pub fn use_dotfile_locks() {
    // SAFETY: SQLite owns built-in VFS pointers for the process lifetime.
    // The NUL-terminated name selects one; registration changes the default
    // used by subsequent opens and initializes SQLite if necessary.
    unsafe {
        let vfs = rusqlite::ffi::sqlite3_vfs_find(c"unix-dotfile".as_ptr());
        if !vfs.is_null() {
            rusqlite::ffi::sqlite3_vfs_register(vfs, 1);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn backup_includes_committed_wal_contents() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.db");
        let destination = dir.path().join("backup.db");
        let db = super::open(&source).unwrap();
        db.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;
             CREATE TABLE items (value TEXT);
             INSERT INTO items VALUES ('committed in WAL');",
        )
        .unwrap();
        assert!(dir.path().join("source.db-wal").exists());

        super::backup(&source, &destination).unwrap();

        let copy = super::open(&destination).unwrap();
        let value: String = copy
            .query_row("SELECT value FROM items", [], |row| row.get(0))
            .unwrap();
        assert_eq!(value, "committed in WAL");
    }

    #[test]
    fn bundled_sqlite_has_wal_page_size_fix() {
        // Older SQLite can expose page-cache memory through a crafted DB/WAL pair.
        assert!(
            rusqlite::version_number() >= 3_054_000,
            "SQLite {} predates the WAL page-size fix",
            rusqlite::version()
        );
    }
}
