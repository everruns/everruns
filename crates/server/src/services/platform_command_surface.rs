//! Transport-neutral implementation of the catalog-backed platform tools.
//!
//! MCP and the built-in Platform capability both delegate here so command
//! discovery, Bashkit behavior, limits, and error sanitization stay identical.

use crate::api::mcp_endpoint::{catalog, positional};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Discover,
    Query,
    Execute,
}

pub async fn invoke(
    operation: Operation,
    arguments: &Value,
    context: catalog::CatalogContext,
) -> Result<String, String> {
    match operation {
        Operation::Discover => discover(arguments),
        Operation::Query => script(arguments, context, catalog::ToolsetMode::ReadOnly).await,
        Operation::Execute => script(arguments, context, catalog::ToolsetMode::Full).await,
    }
}

fn discover(arguments: &Value) -> Result<String, String> {
    let show_all = arguments
        .get("all")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let include_schemas = arguments
        .get("include_schemas")
        .and_then(Value::as_bool)
        .unwrap_or(!show_all);

    let mut entries = crate::domains::common::catalog_entries_with_schemas(include_schemas);
    entries.sort_by_key(|entry| (entry.category, entry.name));

    if !show_all {
        let query = arguments
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if query.is_empty() {
            return Err(
                "Provide a 'query' to search or set 'all: true' to list everything.".into(),
            );
        }
        entries.retain(|entry| catalog_entry_matches(entry, query));
    }

    let operation_json = |entry: &crate::domains::common::CommandCatalogEntry| {
        let mut value = json!({
            "name": entry.name,
            "category": entry.category,
            "description": entry.description,
            "read_only": entry.read_only,
            "output_shape": entry.output_shape,
        });
        if let Some(positional_arg) = entry.positional_arg {
            value["positional_arg"] = json!(positional_arg);
        }
        if include_schemas {
            value["input_schema"] = entry.input_schema.clone();
            value["output_schema"] = entry.output_schema.clone();
        }
        value
    };

    let value = if show_all {
        let mut by_category: std::collections::BTreeMap<&str, Vec<Value>> =
            std::collections::BTreeMap::new();
        for entry in &entries {
            by_category
                .entry(entry.category)
                .or_default()
                .push(operation_json(entry));
        }
        let categories = by_category
            .into_iter()
            .map(|(category, operations)| {
                json!({
                    "category": category,
                    "count": operations.len(),
                    "operations": operations,
                })
            })
            .collect::<Vec<_>>();
        json!({
            "count": entries.len(),
            "include_schemas": include_schemas,
            "categories": categories,
            "shape_hints": output_shape_hints(),
        })
    } else {
        json!({
            "count": entries.len(),
            "include_schemas": include_schemas,
            "operations": entries.iter().map(operation_json).collect::<Vec<_>>(),
            "shape_hints": output_shape_hints(),
        })
    };

    serde_json::to_string_pretty(&value).map_err(|error| format!("Internal error: {error}"))
}

fn catalog_entry_matches(entry: &crate::domains::common::CommandCatalogEntry, query: &str) -> bool {
    let haystack = format!(
        "{} {} {} {}",
        entry.name,
        entry.name.replace('_', " "),
        entry.category,
        entry.description
    )
    .to_lowercase();
    let normalized_query = query.replace('_', " ").to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|term| haystack.contains(term))
        || normalized_query
            .split_whitespace()
            .all(|term| haystack.contains(term))
}

fn output_shape_hints() -> Value {
    json!({
        "array": "Root JSON value is an array. Use jq '.[]'.",
        "paginated": "Root JSON value is an object with data/total/offset/limit. Use jq '.data[]'.",
        "unknown": "Inspect output_schema or run a small sample before choosing a jq path."
    })
}

async fn script(
    arguments: &Value,
    context: catalog::CatalogContext,
    mode: catalog::ToolsetMode,
) -> Result<String, String> {
    let commands = arguments
        .get("commands")
        .and_then(Value::as_str)
        .ok_or("Missing required parameter: commands")?;
    let timeout_ms = arguments
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(30_000)
        .min(60_000);

    let rewritten = positional::rewrite(commands, positional::positional_map());
    let tool = catalog::build_toolset(context, mode);
    let request = bashkit::ToolRequest::new(rewritten);
    let result = tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        bashkit::Tool::execute(&tool, request),
    )
    .await;

    match result {
        Ok(response) if response.exit_code == 0 => Ok(response.stdout),
        Ok(response) => {
            let combined = if response.stderr.is_empty() {
                response.stdout
            } else if response.stdout.is_empty() {
                response.stderr
            } else {
                format!("{}\n{}", response.stdout, response.stderr)
            };
            let trimmed = combined.trim();
            if trimmed.is_empty() {
                Err(format!(
                    "Command failed with exit code {}",
                    response.exit_code
                ))
            } else {
                Err(sanitize_script_error(trimmed))
            }
        }
        Err(_) => Err(format!("Command timed out after {timeout_ms}ms")),
    }
}

fn sanitize_script_error(message: &str) -> String {
    if message.contains("jq: runtime error")
        || (message.contains("jq: error") && message.contains("Cannot index"))
    {
        let cause = if message.to_lowercase().contains("cannot index") {
            "cannot index input with requested path"
        } else {
            "filter failed against input value"
        };
        return format!(
            "jq: runtime error: {cause}. Input value omitted; use discover output_shape/output_schema to choose the jq path."
        );
    }
    message.to_string()
}
