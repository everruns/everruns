use serve::prelude::*;

use crate::connections::warehouse::{Rows, Warehouse};

/// Estimated data scanned, in GB. A query with no `WHERE` reads every order.
/// Matched as a word, since models often put `WHERE` on its own line.
fn scan_gb(sql: &str) -> f64 {
    let filtered = sql
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|word| word.eq_ignore_ascii_case("where"));
    if filtered { 2.0 } else { 120.0 }
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
    cx.progress("querying…").await;
    Ok(wh.query(&sql)?.truncate(500))
}

#[cfg(test)]
mod tests {
    use super::scan_gb;

    #[test]
    fn where_on_its_own_line_counts_as_filtered() {
        assert_eq!(scan_gb("SELECT *\nFROM orders o\nWHERE o.id = 1"), 2.0);
        assert_eq!(scan_gb("select * from orders where id = 1"), 2.0);
    }

    #[test]
    fn unfiltered_scan_is_large() {
        assert_eq!(scan_gb("SELECT * FROM orders"), 120.0);
        assert_eq!(scan_gb("SELECT nowhere_col FROM orders"), 120.0);
    }
}
