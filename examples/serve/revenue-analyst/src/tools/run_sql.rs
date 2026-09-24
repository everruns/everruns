use serve::prelude::*;

use crate::connections::warehouse::{Rows, Warehouse};

/// Estimated data scanned, in GB. A query with no `WHERE` reads every order.
fn scan_gb(sql: &str) -> f64 {
    if sql.to_lowercase().contains(" where ") {
        2.0
    } else {
        120.0
    }
}

/// Run a read-only SQL query against the warehouse. Load the `sql-style`
/// skill first.
#[tool(needs_approval = |a: &RunSql| scan_gb(&a.sql) > 50.0)]
async fn run_sql(
    cx: &Cx,
    /// A single SELECT (or WITH … SELECT) statement.
    sql: String,
) -> Result<Rows> {
    let wh = cx.connection::<Warehouse>()?; // credentials never reach the model
    cx.progress("querying…");
    Ok(wh.query(&sql)?.truncate(500))
}
