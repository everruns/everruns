//! A stand-in warehouse: an in-memory SQLite with a few weeks of orders.
//! A real app would connect with credentials from `Secret::named(...)` here,
//! where the model never sees them.

use std::sync::Mutex;

use rusqlite::Connection;
use rusqlite::types::ValueRef;
use serve::prelude::*;

pub struct Warehouse {
    conn: Mutex<Connection>,
}

#[derive(Serialize)]
pub struct Rows {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub truncated: bool,
}

impl Rows {
    /// Keep at most `limit` rows so a big result cannot flood the context.
    pub fn truncate(mut self, limit: usize) -> Self {
        if self.rows.len() > limit {
            self.rows.truncate(limit);
            self.truncated = true;
        }
        self
    }
}

impl Warehouse {
    fn seeded() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "CREATE TABLE orders (id INTEGER PRIMARY KEY, placed_at TEXT NOT NULL, amount_cents INTEGER NOT NULL);
             CREATE TABLE refunds (order_id INTEGER NOT NULL, refunded_at TEXT NOT NULL, amount_cents INTEGER NOT NULL);
             INSERT INTO orders VALUES
               (1, '2026-09-08', 12000), (2, '2026-09-10', 4500), (3, '2026-09-12', 9900),
               (4, '2026-09-14', 15000), (5, '2026-09-15', 2500), (6, '2026-09-17', 30000),
               (7, '2026-09-19', 7800),  (8, '2026-09-20', 11200), (9, '2026-09-22', 6400);
             INSERT INTO refunds VALUES (5, '2026-09-16', 2500), (6, '2026-09-18', 5000);",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Run one read-only statement.
    pub fn query(&self, sql: &str) -> Result<Rows> {
        let statement = sql.trim().trim_end_matches(';');
        let head = statement.to_lowercase();
        ensure!(
            head.starts_with("select") || head.starts_with("with"),
            "only SELECT queries are allowed"
        );
        ensure!(!statement.contains(';'), "one statement at a time");
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow!("warehouse lock poisoned"))?;
        let mut prepared = conn.prepare(statement)?;
        let columns: Vec<String> = prepared
            .column_names()
            .iter()
            .map(|c| c.to_string())
            .collect();
        let width = columns.len();
        let mut rows = Vec::new();
        let mut cursor = prepared.query([])?;
        while let Some(row) = cursor.next()? {
            let mut values = Vec::with_capacity(width);
            for index in 0..width {
                values.push(match row.get_ref(index)? {
                    ValueRef::Null => serde_json::Value::Null,
                    ValueRef::Integer(v) => json!(v),
                    ValueRef::Real(v) => json!(v),
                    ValueRef::Text(v) => json!(String::from_utf8_lossy(v)),
                    ValueRef::Blob(_) => json!("<blob>"),
                });
            }
            rows.push(values);
        }
        Ok(Rows {
            columns,
            rows,
            truncated: false,
        })
    }
}

/// The warehouse tools query through `cx.connection::<Warehouse>()`.
#[connection]
fn warehouse() -> Result<Warehouse> {
    Warehouse::seeded()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn net_revenue_for_the_demo_week() {
        let wh = Warehouse::seeded().unwrap();
        let rows = wh
            .query(
                "SELECT SUM(o.amount_cents - COALESCE(r.amount_cents, 0)) / 100.0 AS net_revenue \
                 FROM orders o LEFT JOIN refunds r ON r.order_id = o.id \
                 WHERE o.placed_at >= '2026-09-14' AND o.placed_at < '2026-09-21'",
            )
            .unwrap();
        assert_eq!(rows.columns, vec!["net_revenue"]);
        assert_eq!(rows.rows[0][0], json!(590.0));
    }

    #[test]
    fn writes_and_multiple_statements_are_refused() {
        let wh = Warehouse::seeded().unwrap();
        assert!(wh.query("DELETE FROM orders").is_err());
        assert!(wh.query("SELECT 1; DELETE FROM orders").is_err());
    }

    #[test]
    fn truncation_is_reported() {
        let wh = Warehouse::seeded().unwrap();
        let rows = wh.query("SELECT * FROM orders").unwrap().truncate(3);
        assert_eq!(rows.rows.len(), 3);
        assert!(rows.truncated);
    }
}
