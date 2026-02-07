// Shared query execution logic
//
// Operates on any rusqlite::Connection (VFS-backed or in-memory).
// Handles authorizer installation, progress handler, result mapping.

use crate::authorizer::install_authorizer;
use crate::error::SqlDbError;
use crate::limits::{
    MAX_RESULT_ROWS, MAX_RESULT_SIZE_BYTES, PROGRESS_HANDLER_INTERVAL, QUERY_TIMEOUT_SECS,
};
use crate::types::{ColumnSchema, SqlExecuteResult, SqlQueryResult, TableSchema};
use rusqlite::Connection;
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tracing::debug;

/// Configure a connection with safe defaults.
///
/// Call once after opening a connection, before the authorizer is installed.
pub fn configure_connection(conn: &Connection) -> Result<(), SqlDbError> {
    conn.execute_batch(
        "PRAGMA journal_mode=MEMORY;
         PRAGMA page_size=4096;
         PRAGMA foreign_keys=ON;",
    )
    .map_err(|e| SqlDbError::Internal(format!("failed to configure connection: {e}")))?;
    Ok(())
}

/// Install a progress handler that interrupts after timeout.
fn install_progress_handler(conn: &Connection, interrupted: Arc<AtomicBool>) {
    conn.progress_handler(
        PROGRESS_HANDLER_INTERVAL,
        Some(move || interrupted.load(Ordering::Relaxed)),
    );
}

/// Remove the progress handler.
fn clear_progress_handler(conn: &Connection) {
    conn.progress_handler(0, None::<fn() -> bool>);
}

/// Execute a read-only SQL query, returning columns and rows as JSON.
pub fn execute_query(conn: &Connection, sql: &str) -> Result<SqlQueryResult, SqlDbError> {
    install_authorizer(conn);

    let interrupted = Arc::new(AtomicBool::new(false));
    install_progress_handler(conn, interrupted.clone());

    // Start timeout tracker
    let start = Instant::now();
    let timeout_secs = QUERY_TIMEOUT_SECS;

    // Set up timeout interrupt in a separate check
    let interrupt_handle = conn.get_interrupt_handle();
    let interrupted_clone = interrupted.clone();
    let timeout_thread = std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if interrupted_clone.load(Ordering::Relaxed) {
                break;
            }
            if start.elapsed().as_secs() >= timeout_secs {
                interrupted_clone.store(true, Ordering::Relaxed);
                interrupt_handle.interrupt();
                break;
            }
        }
    });

    let result = execute_query_inner(conn, sql);

    // Signal timeout thread to stop
    interrupted.store(true, Ordering::Relaxed);
    let _ = timeout_thread.join();

    // Remove progress handler
    clear_progress_handler(conn);

    result
}

fn execute_query_inner(conn: &Connection, sql: &str) -> Result<SqlQueryResult, SqlDbError> {
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| SqlDbError::QueryError(e.to_string()))?;

    let columns: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let col_count = columns.len();

    let mut rows = Vec::new();
    let mut truncated = false;
    let mut total_size: usize = 0;

    let mut raw_rows = stmt
        .query([])
        .map_err(|e| SqlDbError::QueryError(e.to_string()))?;

    while let Some(row) = raw_rows
        .next()
        .map_err(|e| SqlDbError::QueryError(e.to_string()))?
    {
        if rows.len() >= MAX_RESULT_ROWS {
            truncated = true;
            break;
        }

        let mut json_row = Vec::with_capacity(col_count);
        for i in 0..col_count {
            let val = sqlite_value_to_json(row, i)?;
            total_size += estimate_json_size(&val);
            json_row.push(val);
        }

        if total_size > MAX_RESULT_SIZE_BYTES {
            truncated = true;
            break;
        }

        rows.push(json_row);
    }

    let row_count = rows.len();

    Ok(SqlQueryResult {
        columns,
        rows,
        row_count,
        truncated,
    })
}

/// Execute a write SQL statement, returning affected row count.
pub fn execute_statement(conn: &Connection, sql: &str) -> Result<SqlExecuteResult, SqlDbError> {
    install_authorizer(conn);

    let interrupted = Arc::new(AtomicBool::new(false));
    install_progress_handler(conn, interrupted.clone());

    let start = Instant::now();
    let timeout_secs = QUERY_TIMEOUT_SECS;
    let interrupt_handle = conn.get_interrupt_handle();
    let interrupted_clone = interrupted.clone();
    let timeout_thread = std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if interrupted_clone.load(Ordering::Relaxed) {
                break;
            }
            if start.elapsed().as_secs() >= timeout_secs {
                interrupted_clone.store(true, Ordering::Relaxed);
                interrupt_handle.interrupt();
                break;
            }
        }
    });

    let result = conn
        .execute_batch(sql)
        .map_err(|e| SqlDbError::QueryError(e.to_string()));

    interrupted.store(true, Ordering::Relaxed);
    let _ = timeout_thread.join();
    clear_progress_handler(conn);

    result?;

    let rows_affected = conn.changes();
    debug!(rows_affected, "SQL execute completed");

    Ok(SqlExecuteResult { rows_affected })
}

/// Get schema for all tables in the database.
pub fn get_schema(conn: &Connection) -> Result<Vec<TableSchema>, SqlDbError> {
    install_authorizer(conn);

    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .map_err(|e| SqlDbError::QueryError(e.to_string()))?;

    let table_names: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| SqlDbError::QueryError(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| SqlDbError::QueryError(e.to_string()))?;

    let mut tables = Vec::new();
    for table_name in table_names {
        let table = get_table_schema(conn, &table_name)?;
        tables.push(table);
    }

    Ok(tables)
}

/// Get schema for a specific table.
pub fn get_table_schema(conn: &Connection, table_name: &str) -> Result<TableSchema, SqlDbError> {
    // Get columns via PRAGMA table_info
    let pragma_sql = format!("PRAGMA table_info(\"{}\")", table_name.replace('"', "\"\""));
    let mut stmt = conn
        .prepare(&pragma_sql)
        .map_err(|e| SqlDbError::QueryError(e.to_string()))?;

    let columns: Vec<ColumnSchema> = stmt
        .query_map([], |row| {
            Ok(ColumnSchema {
                name: row.get(1)?,
                column_type: row.get::<_, String>(2).unwrap_or_default(),
                notnull: row.get::<_, bool>(3).unwrap_or(false),
                pk: row.get::<_, i32>(5).unwrap_or(0) > 0,
                default_value: row.get(4).ok(),
            })
        })
        .map_err(|e| SqlDbError::QueryError(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| SqlDbError::QueryError(e.to_string()))?;

    // Get row count
    let count_sql = format!(
        "SELECT COUNT(*) FROM \"{}\"",
        table_name.replace('"', "\"\"")
    );
    let row_count: i64 = conn
        .query_row(&count_sql, [], |row| row.get(0))
        .unwrap_or(0);

    Ok(TableSchema {
        name: table_name.to_string(),
        columns,
        row_count,
    })
}

/// Convert a SQLite value to JSON.
fn sqlite_value_to_json(row: &rusqlite::Row<'_>, idx: usize) -> Result<Value, SqlDbError> {
    use rusqlite::types::ValueRef;

    let val_ref = row
        .get_ref(idx)
        .map_err(|e| SqlDbError::QueryError(e.to_string()))?;

    Ok(match val_ref {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::Number(serde_json::Number::from(i)),
        ValueRef::Real(f) => serde_json::Number::from_f64(f)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        ValueRef::Text(t) => {
            let s = std::str::from_utf8(t)
                .map_err(|e| SqlDbError::QueryError(format!("invalid UTF-8: {e}")))?;
            Value::String(s.to_string())
        }
        ValueRef::Blob(b) => {
            use base64::Engine;
            Value::String(base64::engine::general_purpose::STANDARD.encode(b))
        }
    })
}

/// Rough estimate of JSON value size in bytes.
fn estimate_json_size(val: &Value) -> usize {
    match val {
        Value::Null => 4,
        Value::Bool(_) => 5,
        Value::Number(n) => n.to_string().len(),
        Value::String(s) => s.len() + 2,
        Value::Array(a) => a.iter().map(estimate_json_size).sum::<usize>() + a.len(),
        Value::Object(o) => o.values().map(estimate_json_size).sum::<usize>() + o.len() * 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        configure_connection(&conn).unwrap();
        conn
    }

    #[test]
    fn test_execute_and_query() {
        let conn = setup_conn();
        execute_statement(
            &conn,
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, age INTEGER)",
        )
        .unwrap();

        execute_statement(
            &conn,
            "INSERT INTO users VALUES (1, 'Alice', 30), (2, 'Bob', 25)",
        )
        .unwrap();

        let result = execute_query(&conn, "SELECT * FROM users ORDER BY id").unwrap();
        assert_eq!(result.columns, vec!["id", "name", "age"]);
        assert_eq!(result.row_count, 2);
        assert_eq!(result.rows[0], vec![json!(1), json!("Alice"), json!(30)]);
        assert_eq!(result.rows[1], vec![json!(2), json!("Bob"), json!(25)]);
        assert!(!result.truncated);
    }

    #[test]
    fn test_null_values() {
        let conn = setup_conn();
        execute_statement(&conn, "CREATE TABLE t (id INTEGER, val TEXT)").unwrap();
        execute_statement(&conn, "INSERT INTO t VALUES (1, NULL)").unwrap();

        let result = execute_query(&conn, "SELECT * FROM t").unwrap();
        assert_eq!(result.rows[0][1], Value::Null);
    }

    #[test]
    fn test_real_values() {
        let conn = setup_conn();
        execute_statement(&conn, "CREATE TABLE t (val REAL)").unwrap();
        execute_statement(&conn, "INSERT INTO t VALUES (2.72)").unwrap();

        let result = execute_query(&conn, "SELECT * FROM t").unwrap();
        assert_eq!(result.rows[0][0], json!(2.72));
    }

    #[test]
    fn test_blob_values() {
        let conn = setup_conn();
        execute_statement(&conn, "CREATE TABLE t (val BLOB)").unwrap();
        conn.execute("INSERT INTO t VALUES (?1)", [&[0u8, 1, 2, 3] as &[u8]])
            .unwrap();

        let result = execute_query(&conn, "SELECT * FROM t").unwrap();
        // BLOB should be base64 encoded
        assert_eq!(result.rows[0][0], json!("AAECAw=="));
    }

    #[test]
    fn test_schema_introspection() {
        let conn = setup_conn();
        execute_statement(
            &conn,
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, age INTEGER)",
        )
        .unwrap();
        execute_statement(
            &conn,
            "INSERT INTO users VALUES (1, 'Alice', 30), (2, 'Bob', 25)",
        )
        .unwrap();

        let schema = get_schema(&conn).unwrap();
        assert_eq!(schema.len(), 1);
        assert_eq!(schema[0].name, "users");
        assert_eq!(schema[0].row_count, 2);
        assert_eq!(schema[0].columns.len(), 3);

        assert_eq!(schema[0].columns[0].name, "id");
        assert!(schema[0].columns[0].pk);

        assert_eq!(schema[0].columns[1].name, "name");
        assert!(schema[0].columns[1].notnull);
    }

    #[test]
    fn test_query_error() {
        let conn = setup_conn();
        let result = execute_query(&conn, "SELECT * FROM nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_execute_statement_returns_rows_affected() {
        let conn = setup_conn();
        execute_statement(&conn, "CREATE TABLE t (id INTEGER, val TEXT)").unwrap();
        execute_statement(&conn, "INSERT INTO t VALUES (1, 'a'), (2, 'b'), (3, 'c')").unwrap();

        let result = execute_statement(&conn, "DELETE FROM t WHERE id > 1").unwrap();
        assert_eq!(result.rows_affected, 2);
    }

    #[test]
    fn test_multiple_tables_schema() {
        let conn = setup_conn();
        execute_statement(&conn, "CREATE TABLE a (id INTEGER)").unwrap();
        execute_statement(&conn, "CREATE TABLE b (id INTEGER, val TEXT)").unwrap();

        let schema = get_schema(&conn).unwrap();
        assert_eq!(schema.len(), 2);
        let names: Vec<&str> = schema.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"a"));
        assert!(names.contains(&"b"));
    }

    use serde_json::json;
}
