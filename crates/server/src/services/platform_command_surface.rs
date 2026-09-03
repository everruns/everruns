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
        Operation::Discover => discover(arguments, &context.domain_ctx.feature_flags),
        Operation::Query => script(arguments, context, catalog::ToolsetMode::ReadOnly).await,
        Operation::Execute => script(arguments, context, catalog::ToolsetMode::Full).await,
    }
}

fn discover(
    arguments: &Value,
    feature_flags: &everruns_platform::FeatureFlags,
) -> Result<String, String> {
    let show_all = arguments
        .get("all")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let requested_schemas = arguments
        .get("include_schemas")
        .and_then(Value::as_bool)
        .unwrap_or(!show_all);

    let mut entries =
        crate::domains::common::catalog_entries_with_schemas(requested_schemas, feature_flags);
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
        let normalized_query = normalize_operation_name(query);
        if entries
            .iter()
            .any(|entry| normalize_operation_name(entry.name) == normalized_query)
        {
            entries.retain(|entry| normalize_operation_name(entry.name) == normalized_query);
        } else {
            let mut scored = entries
                .into_iter()
                .filter_map(|entry| {
                    let score = catalog_entry_score(&entry, query);
                    (score > 0).then_some((score, entry))
                })
                .collect::<Vec<_>>();
            scored.sort_by_key(|(score, entry)| {
                (
                    std::cmp::Reverse(*score),
                    std::cmp::Reverse(entry.read_only),
                    entry.category,
                    entry.name,
                )
            });
            // Resource-oriented phrases may include a product/entity name that
            // is not present in operation metadata. Return the best bounded
            // read paths for the matching entity families instead of either a
            // giant inventory or a misleading empty result.
            scored.truncate(12);
            entries = scored.into_iter().map(|(_, entry)| entry).collect();
        }
    }

    // Broad schema searches can exceed the model context on a few matches.
    // Keep discovery as a two-step catalog interaction: compact name search,
    // then one exact-name lookup with its runnable schema.
    let include_schemas = requested_schemas && (show_all || entries.len() <= 1);

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
            value["bash_usage"] = json!(catalog::bash_usage(entry.name, &entry.input_schema));
            value["output_fields"] = json!(catalog::schema_field_paths(&entry.output_schema));
            if serde_json::to_vec(&value).is_ok_and(|encoded| encoded.len() > 2_000) {
                value
                    .as_object_mut()
                    .expect("operation object")
                    .remove("input_schema");
                value
                    .as_object_mut()
                    .expect("operation object")
                    .remove("output_schema");
                value["schemas_omitted"] = json!(
                    "Expanded schemas exceeded the discovery size limit. bash_usage and output_fields remain authoritative for scripting."
                );
            }
        }
        value
    };

    let mut value = if show_all {
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
            "result_scope": "operation_catalog",
            "shape_hints": output_shape_hints(),
            "script_guidance": script_guidance(),
        })
    } else {
        json!({
            "count": entries.len(),
            "include_schemas": include_schemas,
            "operations": entries.iter().map(operation_json).collect::<Vec<_>>(),
            "result_scope": "operation_catalog",
            "shape_hints": output_shape_hints(),
            "script_guidance": script_guidance(),
        })
    };
    if !show_all && requested_schemas && !include_schemas {
        value["refine_hint"] = json!(
            "Multiple operations matched. Schemas were omitted to keep the result compact; call discover once more with the exact operation name you need."
        );
    }
    if !show_all && entries.is_empty() {
        value["resource_absence_warning"] = json!(
            "No operation names matched. This searches the operation catalog only and does not establish that any platform resource is absent. Find the relevant list/get operation, then inspect current state with query."
        );
    }

    serde_json::to_string_pretty(&value).map_err(|error| format!("Internal error: {error}"))
}

fn script_guidance() -> Value {
    json!([
        "Invoke operations as Bash builtins using exactly the flags shown in bash_usage.",
        "Builtins do not implement --help; discover the exact command name for its schema and bash_usage.",
        "Filesystem redirection is disabled; keep intermediate JSON in shell variables and pipe it directly to jq.",
        "Pass array and object flag values as JSON text.",
        "Capture JSON output and use jq for dependent IDs; combine dependent mutations in one execute script.",
        "create_mcp_server returns capability_ref for create_agent --capabilities; there is no separate MCP attachment operation."
    ])
}

fn catalog_entry_score(entry: &crate::domains::common::CommandCatalogEntry, query: &str) -> u16 {
    let name = normalize_operation_name(entry.name);
    let category = normalize_search_term(entry.category);
    let description = entry.description.to_ascii_lowercase();
    let terms = query
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(normalize_search_term)
        .filter(|term| term.len() > 1)
        .collect::<Vec<_>>();

    let mut score: u16 = 0;
    for term in terms {
        if name
            .split_whitespace()
            .any(|part| normalize_search_term(part) == term)
        {
            score = score.saturating_add(8);
        } else if name.contains(&term) {
            score = score.saturating_add(5);
        }
        if category == term {
            score = score.saturating_add(7);
        } else if category.contains(&term) {
            score = score.saturating_add(3);
        }
        if description.contains(&term) {
            score = score.saturating_add(1);
        }
    }
    if score > 0 && entry.read_only {
        score = score.saturating_add(4);
    }
    if score > 0 && (name.starts_with("list ") || name.starts_with("get ")) {
        score = score.saturating_add(4);
    }
    score
}

fn normalize_search_term(value: &str) -> String {
    let normalized = value.trim().to_ascii_lowercase();
    normalized
        .strip_suffix("ies")
        .map(|stem| format!("{stem}y"))
        .or_else(|| normalized.strip_suffix('s').map(str::to_string))
        .unwrap_or(normalized)
}

fn normalize_operation_name(value: &str) -> String {
    value
        .replace(['_', '-'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
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
        Ok(response) => script_outcome(response.exit_code, response.stdout, response.stderr),
        Err(_) => Err(format!("Command timed out after {timeout_ms}ms")),
    }
}

/// Shape a finished script run into the tool result.
///
/// A multi-command script exits 0 whenever its *last* command succeeds, so a
/// mid-script failure would otherwise be invisible: the caller sees only the
/// surviving stdout (often a jq object with null fields) and no hint that a
/// create failed. Keep stdout first and authoritative, then append the
/// captured stderr so partial failures are actionable.
fn script_outcome(exit_code: i32, stdout: String, stderr: String) -> Result<String, String> {
    if exit_code == 0 {
        let trimmed_stderr = stderr.trim();
        if trimmed_stderr.is_empty() {
            return Ok(stdout);
        }
        return Ok(format!(
            "{}\n\n{PARTIAL_FAILURE_LABEL}\n{}",
            stdout.trim_end(),
            sanitize_script_error(trimmed_stderr)
        ));
    }

    let combined = if stderr.is_empty() {
        stdout
    } else if stdout.is_empty() {
        stderr
    } else {
        format!("{stdout}\n{stderr}")
    };
    let trimmed = combined.trim();
    if trimmed.is_empty() {
        Err(format!("Command failed with exit code {exit_code}"))
    } else {
        Err(sanitize_script_error(trimmed))
    }
}

/// Marks stderr captured from a script that still exited 0. Some commands in it
/// failed, so the caller must verify state with `query` before reporting success.
const PARTIAL_FAILURE_LABEL: &str = "script_stderr (one or more commands in this script failed even though the script exited 0; verify the resulting state with query before reporting success):";

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_outcome_surfaces_stderr_from_a_partially_failed_script() {
        // A create failed mid-script; the trailing `jq -s` still exited 0 and
        // produced an object with null fields. Without the stderr section the
        // caller cannot tell a failure from an intentionally empty result.
        let outcome = script_outcome(
            0,
            "{\n  \"agent\": null\n}\n".to_string(),
            "create_agent: bad_request: Agent name must contain only lowercase letters, digits, and hyphens\n"
                .to_string(),
        )
        .expect("exit code 0 stays a success");
        assert!(outcome.contains("\"agent\": null"), "stdout is preserved");
        assert!(
            outcome.contains(PARTIAL_FAILURE_LABEL),
            "stderr is labelled"
        );
        assert!(
            outcome.contains("Agent name must contain only lowercase letters"),
            "the failing command's error is surfaced: {outcome}"
        );
    }

    #[test]
    fn script_outcome_returns_bare_stdout_when_nothing_failed() {
        let outcome = script_outcome(0, "{\"id\":\"agent_1\"}".to_string(), String::new())
            .expect("clean run");
        assert_eq!(outcome, "{\"id\":\"agent_1\"}");
    }

    #[test]
    fn script_outcome_reports_a_nonzero_exit_as_an_error() {
        let error = script_outcome(
            2,
            String::new(),
            "create_agent: bad_request: boom\n".to_string(),
        )
        .expect_err("non-zero exit is an error");
        assert!(error.contains("boom"), "{error}");
        assert!(!error.contains(PARTIAL_FAILURE_LABEL));
    }

    fn discover_for_test(arguments: &Value) -> Result<String, String> {
        discover(
            arguments,
            &everruns_platform::FeatureFlags {
                notifications: true,
                evals: true,
                skills: true,
                memory: true,
                knowledge: true,
                plugins: true,
                app_budgets: true,
                agent_versions: true,
                voice: true,
                agent_delegation: true,
                observers: true,
                public_chat: true,
                webmcp: true,
                machine_payments: true,
            },
        )
    }

    #[test]
    fn jq_errors_are_concise_and_omit_input() {
        let error = sanitize_script_error(
            "jq: runtime error: Cannot index array with string data; input: You are a secret prompt",
        );

        assert!(error.contains("output_shape/output_schema"));
        assert!(!error.contains("secret prompt"));
        assert!(error.len() < 240);
    }

    #[test]
    fn exact_discovery_returns_one_operation_with_runnable_usage() {
        let output = discover_for_test(&json!({
            "query": "create_agent",
            "include_schemas": true
        }))
        .expect("discover create_agent");
        let value: Value = serde_json::from_str(&output).expect("discover JSON");

        assert_eq!(value["count"], 1);
        assert_eq!(value["operations"][0]["name"], "create_agent");
        let usage = value["operations"][0]["bash_usage"]
            .as_str()
            .expect("bash_usage");
        assert!(usage.contains("--name"));
        assert!(usage.contains("--system_prompt"));
        assert!(usage.contains("--default_model_id"));
        assert!(!usage.contains("--model_id"));
        assert!(usage.contains("--capabilities '[{"), "usage was {usage}");
        assert!(
            usage.contains(r#""ref":"current_time""#),
            "usage was {usage}"
        );
        assert!(
            value["script_guidance"]
                .as_array()
                .expect("script_guidance")
                .iter()
                .any(|line| line.as_str().is_some_and(|line| line.contains("--help")))
        );

        let mcp_output = discover_for_test(&json!({ "query": "create_mcp_server" }))
            .expect("discover create_mcp_server");
        let mcp_value: Value = serde_json::from_str(&mcp_output).expect("MCP discover JSON");
        let mcp_usage = mcp_value["operations"][0]["bash_usage"]
            .as_str()
            .expect("MCP bash_usage");
        assert!(mcp_usage.contains("--auth_mode"));
        assert!(mcp_usage.contains("--api_key"));
        assert!(!mcp_usage.contains("--authentication_type"));
        assert!(mcp_output.len() < 10_000, "MCP discovery must stay compact");

        let trigger_output = discover_for_test(&json!({ "query": "create_agent_trigger" }))
            .expect("discover create_agent_trigger");
        let trigger_value: Value =
            serde_json::from_str(&trigger_output).expect("trigger discover JSON");
        let trigger_usage = trigger_value["operations"][0]["bash_usage"]
            .as_str()
            .expect("trigger bash_usage");
        assert!(trigger_usage.contains("--agent_id"));
        assert!(trigger_usage.contains("--cron_expression"));
        assert!(trigger_usage.contains("--message"));

        let models_output =
            discover_for_test(&json!({ "query": "list_models" })).expect("discover list_models");
        let models_value: Value =
            serde_json::from_str(&models_output).expect("model discover JSON");
        let model_fields = models_value["operations"][0]["output_fields"]
            .as_array()
            .expect("model output fields");
        for field in ["id", "display_name", "model_id"] {
            assert!(
                model_fields
                    .iter()
                    .any(|value| value.as_str() == Some(field)),
                "missing {field} in {model_fields:?}"
            );
        }
        assert!(
            models_output.len() < 20_000,
            "exact discovery must stay compact"
        );

        let natural_name_output = discover_for_test(&json!({
            "query": "create agent",
            "include_schemas": true
        }))
        .expect("natural operation discovery");
        let natural_name_value: Value =
            serde_json::from_str(&natural_name_output).expect("natural discover JSON");
        assert_eq!(natural_name_value["count"], 1);
        assert_eq!(natural_name_value["operations"][0]["name"], "create_agent");
        assert!(
            natural_name_value["operations"][0]
                .get("bash_usage")
                .is_some()
        );
        assert!(natural_name_value.get("refine_hint").is_none());

        let broad_output = discover_for_test(&json!({
            "query": "agent",
            "include_schemas": true
        }))
        .expect("broad agent discovery");
        let broad_value: Value = serde_json::from_str(&broad_output).expect("broad discover JSON");
        assert!(broad_value["count"].as_u64().is_some_and(|count| count > 1));
        assert_eq!(broad_value["include_schemas"], false);
        assert!(broad_value["operations"][0].get("input_schema").is_none());
        assert!(broad_value["operations"][0].get("bash_usage").is_none());
        assert!(
            broad_value["refine_hint"]
                .as_str()
                .is_some_and(|hint| hint.contains("exact operation name"))
        );
    }

    #[test]
    fn resource_language_resolves_authoritative_read_operations() {
        for (query, expected) in [
            (
                "resend email capability plugin",
                ["list_plugins", "list_capabilities"],
            ),
            ("customer support agent", ["list_agents", "get_agent"]),
            ("generic harness", ["list_harnesses", "get_harness"]),
            (
                "oauth connections",
                ["list_user_connections", "list_connection_providers"],
            ),
        ] {
            let output = discover_for_test(&json!({ "query": query, "include_schemas": false }))
                .expect("resource-oriented discovery");
            let value: Value = serde_json::from_str(&output).expect("discover JSON");
            let names = value["operations"]
                .as_array()
                .expect("operations")
                .iter()
                .filter_map(|operation| operation["name"].as_str())
                .collect::<Vec<_>>();
            for operation in expected {
                assert!(
                    names.contains(&operation),
                    "{query:?} should suggest {operation}; got {names:?}"
                );
            }
        }
    }

    #[test]
    fn discovery_score_saturates_for_many_matching_terms() {
        let query = "agent ".repeat(4_096);
        let output = discover_for_test(&json!({ "query": query, "include_schemas": false }))
            .expect("discovery with many matching terms");
        let value: Value = serde_json::from_str(&output).expect("discover JSON");
        let names = value["operations"]
            .as_array()
            .expect("operations")
            .iter()
            .filter_map(|operation| operation["name"].as_str())
            .collect::<Vec<_>>();

        assert!(names.contains(&"list_agents"), "got {names:?}");
    }

    #[test]
    fn zero_matches_are_explicitly_not_resource_absence_evidence() {
        let output =
            discover_for_test(&json!({ "query": "zzqxvbnm" })).expect("zero-match discovery");
        let value: Value = serde_json::from_str(&output).expect("discover JSON");

        assert_eq!(value["count"], 0);
        assert_eq!(value["result_scope"], "operation_catalog");
        assert!(
            value["resource_absence_warning"]
                .as_str()
                .is_some_and(|warning| warning.contains("does not establish"))
        );
    }

    #[test]
    fn feature_gated_discovery_omits_operations_for_disabled_features() {
        let output = discover(
            &json!({ "query": "list_evals" }),
            &everruns_platform::FeatureFlags::default(),
        )
        .expect("discover disabled eval operation");
        let value: Value = serde_json::from_str(&output).expect("discover JSON");

        assert!(
            value["operations"]
                .as_array()
                .expect("operations")
                .iter()
                .all(|operation| operation["category"] != "evals"
                    && operation["name"] != "list_evals")
        );
    }
}
