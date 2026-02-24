// Session SQL Database Capability
//
// Provides session-scoped SQLite databases via three tools:
// - sql_execute: DDL/DML (CREATE TABLE, INSERT, UPDATE, DELETE). Auto-creates DB.
// - sql_query: Read-only SELECT queries returning columns + rows as JSON.
// - sql_schema: Introspect database schema (tables, columns, types).

use super::{Capability, CapabilityStatus};
use crate::session_sqldb::SessionSqlDbError;
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde_json::{Value, json};

/// Session SQL Database capability
pub struct SessionSqlDatabaseCapability;

impl Capability for SessionSqlDatabaseCapability {
    fn id(&self) -> &str {
        "session_sql_database"
    }

    fn name(&self) -> &str {
        "SQL Database"
    }

    fn description(&self) -> &str {
        "Session-scoped SQLite databases for structured data storage and querying."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("database")
    }

    fn category(&self) -> Option<&str> {
        Some("Data")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            r#"You have access to session-scoped SQL databases. Use these tools to create tables, insert data, and query data using standard SQLite SQL syntax.

**Tools:**
- `sql_execute`: Create tables, insert/update/delete data. Auto-creates the database if needed.
- `sql_query`: Run SELECT queries. Returns columns and rows as JSON.
- `sql_schema`: Inspect database schema (tables, columns, types).

**Guidelines:**
- Database names must be alphanumeric with underscores (e.g., "analytics", "user_data")
- Use `sql_schema` to inspect existing tables before querying
- Results are limited to 1000 rows per query
- Databases are session-scoped and isolated
- Standard SQLite SQL syntax is supported"#,
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(SqlExecuteTool),
            Box::new(SqlQueryTool),
            Box::new(SqlSchemaTool),
        ]
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["sql_database"]
    }
}

// ----------------------------------------------------------------------------
// Helper: convert SessionSqlDbError to ToolExecutionResult
// ----------------------------------------------------------------------------

fn sqldb_error_to_result(err: SessionSqlDbError) -> ToolExecutionResult {
    if err.is_tool_error() {
        ToolExecutionResult::tool_error(err.to_string())
    } else {
        ToolExecutionResult::internal_error_msg(err.to_string())
    }
}

// ----------------------------------------------------------------------------
// SqlExecuteTool
// ----------------------------------------------------------------------------

pub struct SqlExecuteTool;

#[async_trait]
impl Tool for SqlExecuteTool {
    fn name(&self) -> &str {
        "sql_execute"
    }

    fn display_name(&self) -> Option<&str> {
        Some("SQL Execute")
    }

    fn description(&self) -> &str {
        "Execute DDL/DML SQL (CREATE TABLE, INSERT, UPDATE, DELETE). Auto-creates database if it doesn't exist."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "database": {
                    "type": "string",
                    "description": "Database name (alphanumeric + underscores)"
                },
                "sql": {
                    "type": "string",
                    "description": "SQL statement(s) to execute"
                }
            },
            "required": ["database", "sql"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "sql_execute requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let database = match arguments.get("database").and_then(|v| v.as_str()) {
            Some(d) => d,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: database");
            }
        };

        let sql = match arguments.get("sql").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: sql");
            }
        };

        let store = match &context.sqldb_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error(
                    "SQL database not available in this context",
                );
            }
        };

        match store.sql_execute(context.session_id, database, sql).await {
            Ok(result) => ToolExecutionResult::success(json!({
                "database": database,
                "success": true,
                "rows_affected": result.rows_affected
            })),
            Err(e) => sqldb_error_to_result(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ----------------------------------------------------------------------------
// SqlQueryTool
// ----------------------------------------------------------------------------

pub struct SqlQueryTool;

#[async_trait]
impl Tool for SqlQueryTool {
    fn name(&self) -> &str {
        "sql_query"
    }

    fn display_name(&self) -> Option<&str> {
        Some("SQL Query")
    }

    fn description(&self) -> &str {
        "Execute a read-only SQL query (SELECT). Returns columns and rows as JSON."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "database": {
                    "type": "string",
                    "description": "Database name"
                },
                "sql": {
                    "type": "string",
                    "description": "SELECT query"
                }
            },
            "required": ["database", "sql"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "sql_query requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let database = match arguments.get("database").and_then(|v| v.as_str()) {
            Some(d) => d,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: database");
            }
        };

        let sql = match arguments.get("sql").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: sql");
            }
        };

        let store = match &context.sqldb_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error(
                    "SQL database not available in this context",
                );
            }
        };

        match store.sql_query(context.session_id, database, sql).await {
            Ok(result) => {
                let mut response = json!({
                    "database": database,
                    "columns": result.columns,
                    "rows": result.rows,
                    "row_count": result.row_count
                });
                if result.truncated {
                    response["truncated"] = json!(true);
                }
                ToolExecutionResult::success(response)
            }
            Err(e) => sqldb_error_to_result(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ----------------------------------------------------------------------------
// SqlSchemaTool
// ----------------------------------------------------------------------------

pub struct SqlSchemaTool;

#[async_trait]
impl Tool for SqlSchemaTool {
    fn name(&self) -> &str {
        "sql_schema"
    }

    fn display_name(&self) -> Option<&str> {
        Some("SQL Schema")
    }

    fn description(&self) -> &str {
        "Introspect database schema: tables, columns, types, and row counts."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "database": {
                    "type": "string",
                    "description": "Database name"
                },
                "table": {
                    "type": "string",
                    "description": "Specific table name (optional, omit to list all tables)"
                }
            },
            "required": ["database"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "sql_schema requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let database = match arguments.get("database").and_then(|v| v.as_str()) {
            Some(d) => d,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: database");
            }
        };

        let table = arguments.get("table").and_then(|v| v.as_str());

        let store = match &context.sqldb_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error(
                    "SQL database not available in this context",
                );
            }
        };

        match store.sql_schema(context.session_id, database, table).await {
            Ok(tables) => {
                let tables_json: Vec<Value> = tables
                    .into_iter()
                    .map(|t| {
                        json!({
                            "name": t.name,
                            "columns": t.columns.into_iter().map(|c| json!({
                                "name": c.name,
                                "type": c.column_type,
                                "notnull": c.notnull,
                                "pk": c.pk,
                                "default_value": c.default_value
                            })).collect::<Vec<_>>(),
                            "row_count": t.row_count
                        })
                    })
                    .collect();

                ToolExecutionResult::success(json!({
                    "database": database,
                    "tables": tables_json
                }))
            }
            Err(e) => sqldb_error_to_result(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typed_id::SessionId;

    #[test]
    fn test_capability_metadata() {
        let cap = SessionSqlDatabaseCapability;
        assert_eq!(cap.id(), "session_sql_database");
        assert_eq!(cap.name(), "SQL Database");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("database"));
        assert_eq!(cap.category(), Some("Data"));
    }

    #[test]
    fn test_capability_has_three_tools() {
        let cap = SessionSqlDatabaseCapability;
        let tools = cap.tools();
        assert_eq!(tools.len(), 3);

        let tool_names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(tool_names.contains(&"sql_execute"));
        assert!(tool_names.contains(&"sql_query"));
        assert!(tool_names.contains(&"sql_schema"));
    }

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = SessionSqlDatabaseCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("sql_execute"));
        assert!(prompt.contains("sql_query"));
        assert!(prompt.contains("sql_schema"));
        assert!(prompt.contains("SQLite"));
    }

    #[test]
    fn test_tools_require_context() {
        assert!(SqlExecuteTool.requires_context());
        assert!(SqlQueryTool.requires_context());
        assert!(SqlSchemaTool.requires_context());
    }

    #[tokio::test]
    async fn test_sql_execute_without_context() {
        let tool = SqlExecuteTool;
        let result = tool
            .execute(json!({"database": "test", "sql": "SELECT 1"}))
            .await;
        assert!(matches!(result, ToolExecutionResult::ToolError(_)));
    }

    #[tokio::test]
    async fn test_sql_execute_missing_params() {
        let tool = SqlExecuteTool;
        let context = ToolContext::new(SessionId::new());

        let result = tool
            .execute_with_context(json!({"database": "test"}), &context)
            .await;
        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("sql"));
        } else {
            panic!("Expected tool error for missing sql");
        }
    }

    #[tokio::test]
    async fn test_sql_execute_no_store() {
        let tool = SqlExecuteTool;
        let context = ToolContext::new(SessionId::new());

        let result = tool
            .execute_with_context(
                json!({"database": "test", "sql": "CREATE TABLE t (id INTEGER)"}),
                &context,
            )
            .await;
        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("not available"));
        } else {
            panic!("Expected tool error for missing store");
        }
    }
}
