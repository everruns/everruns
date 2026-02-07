// SQLite authorizer callback
//
// Blocks dangerous operations: ATTACH, LOAD_EXTENSION, virtual tables,
// and write-mode PRAGMAs. This is the primary security boundary for
// session SQL databases.

use rusqlite::Connection;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use tracing::warn;

/// Allowed read-only PRAGMAs for schema introspection.
const ALLOWED_PRAGMAS: &[&str] = &[
    "table_info",
    "table_list",
    "table_xinfo",
    "index_list",
    "index_info",
    "database_list",
    "foreign_key_list",
    "compile_options",
    "encoding",
    "page_count",
    "page_size",
    "freelist_count",
];

/// Install the authorizer on a connection.
///
/// Must be called before executing any user-provided SQL.
pub fn install_authorizer(conn: &Connection) {
    conn.authorizer(Some(authorizer_callback));
}

/// Remove the authorizer from a connection.
pub fn remove_authorizer(conn: &Connection) {
    conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
}

fn authorizer_callback(ctx: AuthContext<'_>) -> Authorization {
    match ctx.action {
        // Block ATTACH/DETACH — prevents accessing other database files
        AuthAction::Attach { .. } => {
            warn!("SQLite authorizer blocked ATTACH");
            Authorization::Deny
        }
        AuthAction::Detach { .. } => {
            warn!("SQLite authorizer blocked DETACH");
            Authorization::Deny
        }

        // Block virtual tables
        AuthAction::CreateVtable { .. } => {
            warn!("SQLite authorizer blocked CREATE VIRTUAL TABLE");
            Authorization::Deny
        }
        AuthAction::DropVtable { .. } => {
            warn!("SQLite authorizer blocked DROP VIRTUAL TABLE");
            Authorization::Deny
        }

        // Filter PRAGMAs — only allow known-safe read-only ones
        AuthAction::Pragma { pragma_name, .. } => {
            let name_lower = pragma_name.to_lowercase();
            if ALLOWED_PRAGMAS.contains(&name_lower.as_str()) {
                Authorization::Allow
            } else {
                warn!(pragma = %pragma_name, "SQLite authorizer blocked PRAGMA");
                Authorization::Deny
            }
        }

        // Block function calls to load_extension
        AuthAction::Function {
            function_name: "load_extension",
        } => {
            warn!("SQLite authorizer blocked load_extension");
            Authorization::Deny
        }

        // Allow everything else (SELECT, INSERT, UPDATE, DELETE, CREATE TABLE, etc.)
        _ => Authorization::Allow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        install_authorizer(&conn);
        conn
    }

    #[test]
    fn test_basic_operations_allowed() {
        let conn = setup_conn();
        conn.execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)")
            .unwrap();
        conn.execute("INSERT INTO t VALUES (1, 'test')", [])
            .unwrap();
        let val: String = conn
            .query_row("SELECT name FROM t WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(val, "test");
        conn.execute("UPDATE t SET name = 'updated' WHERE id = 1", [])
            .unwrap();
        conn.execute("DELETE FROM t WHERE id = 1", []).unwrap();
        conn.execute_batch("DROP TABLE t").unwrap();
    }

    #[test]
    fn test_attach_blocked() {
        let conn = setup_conn();
        let result = conn.execute_batch("ATTACH DATABASE ':memory:' AS other");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("authorized") || err.contains("denied") || err.contains("not authorized"),
            "unexpected error: {err}",
        );
    }

    #[test]
    fn test_dangerous_pragma_blocked() {
        let conn = setup_conn();
        let result = conn.execute_batch("PRAGMA journal_mode=WAL");
        assert!(result.is_err());
    }

    #[test]
    fn test_safe_pragma_allowed() {
        let conn = setup_conn();
        conn.execute_batch("CREATE TABLE t (id INTEGER)").unwrap();
        let _: Vec<(i32, String)> = conn
            .prepare("PRAGMA table_info('t')")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
    }

    #[test]
    fn test_create_virtual_table_blocked() {
        let conn = setup_conn();
        let result = conn.execute_batch("CREATE VIRTUAL TABLE vt USING fts5(content)");
        assert!(result.is_err());
    }

    #[test]
    fn test_authorizer_removal() {
        let conn = Connection::open_in_memory().unwrap();
        install_authorizer(&conn);
        assert!(
            conn.execute_batch("ATTACH DATABASE ':memory:' AS other")
                .is_err()
        );
        remove_authorizer(&conn);
        assert!(
            conn.execute_batch("ATTACH DATABASE ':memory:' AS other")
                .is_ok()
        );
    }
}
