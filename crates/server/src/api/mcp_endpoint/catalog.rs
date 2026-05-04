// MCP scripting catalog — inventory is the only source of truth.

use bashkit::{ScriptingToolSet, ToolArgs, ToolDef};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::LazyLock;

/// Shared context for inventory-registered domain commands.
#[derive(Clone)]
pub struct CatalogContext {
    pub state: super::AppState,
    pub caller: everruns_core::permissions::Caller,
    pub link_builder: crate::api::common::UrlBuilder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolsetMode {
    Full,
    ReadOnly,
}

static INVENTORY_TOOL_DEFS: LazyLock<HashMap<&'static str, ToolDef>> = LazyLock::new(|| {
    inventory::iter::<crate::domains::common::CommandDescriptor>
        .into_iter()
        .map(|desc| {
            let meta = (desc.meta)();
            let schema = bashkit_inventory_schema((desc.param_schema)());
            let def = ToolDef::new(meta.name, meta.description)
                .with_schema(schema)
                .with_category(meta.category);
            (meta.name, def)
        })
        .collect()
});

impl CatalogContext {
    /// Convert to a domain Ctx for inventory-registered command dispatch.
    pub fn to_domain_ctx(&self) -> crate::domains::common::Ctx {
        let mut ctx = crate::domains::common::Ctx::new(
            self.caller.clone(),
            self.state.db.clone(),
            self.state.capability_service.clone(),
            self.state.encryption.clone(),
            self.state.auth.permission_resolver.clone(),
        )
        .with_session_service(self.state.session_service.clone())
        .with_message_service(self.state.message_service.clone())
        .with_event_service(self.state.event_service.clone())
        .with_session_file_service(self.state.session_file_service.clone())
        .with_runner(self.state.runner.clone())
        .with_fallback_harness_name(self.state.fallback_default_harness_name.clone())
        .with_chat_harness_name(self.state.chat_harness_name.clone())
        .with_chat_session_title(self.state.chat_session_title.clone());
        if let Some(service) = &self.state.session_sandbox_service {
            ctx = ctx.with_session_sandbox_service(service.clone());
        }
        if let Some(store) = &self.state.sqldb_store {
            ctx = ctx.with_sqldb_store(store.clone());
        }
        ctx.with_workflow_store(self.state.workflow_store.clone())
    }
}

/// Build a ScriptingToolSet from inventory-registered commands only.
pub fn build_toolset(ctx: CatalogContext, mode: ToolsetMode) -> ScriptingToolSet {
    let mut builder = ScriptingToolSet::builder("everruns")
        .short_description(match mode {
            ToolsetMode::Full => "Everruns API operations as bash builtins",
            ToolsetMode::ReadOnly => "Read-only Everruns API operations as bash builtins",
        })
        .limits(
            bashkit::ExecutionLimits::new()
                .max_commands(500)
                .max_loop_iterations(5000)
                .max_function_depth(50)
                .max_input_bytes(500_000)
                .max_ast_depth(50)
                .parser_timeout(std::time::Duration::from_secs(3)),
        );

    for desc in inventory::iter::<crate::domains::common::CommandDescriptor> {
        // THREAT[TM-MCP-002]: MCP `query` must not expose mutating server
        // tools. Read-only classification is backed by inventory tests.
        if mode == ToolsetMode::ReadOnly && !(desc.read_only)() {
            continue;
        }
        let def = command_descriptor_to_def(desc);
        let callback = make_inventory_callback(desc, ctx.clone());
        builder = builder.async_tool_fn(def, callback);
    }

    builder.build()
}

fn command_descriptor_to_def(desc: &crate::domains::common::CommandDescriptor) -> ToolDef {
    let meta = (desc.meta)();
    INVENTORY_TOOL_DEFS
        .get(meta.name)
        .cloned()
        .unwrap_or_else(|| {
            ToolDef::new(meta.name, meta.description)
                .with_schema(bashkit_inventory_schema((desc.param_schema)()))
                .with_category(meta.category)
        })
}

fn bashkit_inventory_schema(mut schema: serde_json::Value) -> serde_json::Value {
    let Some(properties) = schema
        .get_mut("properties")
        .and_then(|value| value.as_object_mut())
    else {
        return schema;
    };

    // EVE-409: bashkit's flag parser only coerces scalar types and falls back
    // to `string` for anything else, leaving array/object fields as raw strings
    // by the time they reach the dispatcher. Generic operations (`update_agent
    // --capabilities '[...]'`, `--tags '[...]'`, etc.) then fail to deserialize.
    // Re-shape every non-scalar property to `type: string` with a JSON-text
    // hint here, and let `make_inventory_callback` parse those JSON strings
    // back into structured values before dispatch.
    for (_name, property) in properties.iter_mut() {
        let Some(object) = property.as_object_mut() else {
            continue;
        };
        let is_non_scalar = object
            .get("type")
            .and_then(|value| value.as_str())
            .is_some_and(|value| matches!(value, "array" | "object"))
            || object.contains_key("items")
            || object.contains_key("properties");
        if !is_non_scalar {
            continue;
        }
        object.insert(
            "type".to_string(),
            serde_json::Value::String("string".to_string()),
        );
        // Drop array-only metadata that no longer applies after retyping; this
        // keeps `tools/list` consumers from generating bogus item validators.
        object.remove("items");
        let description = object
            .get("description")
            .and_then(|value| value.as_str())
            .map(|value| format!("{value} Pass JSON text in bash mode."))
            .unwrap_or_else(|| "Pass JSON text in bash mode.".to_string());
        object.insert(
            "description".to_string(),
            serde_json::Value::String(description),
        );
    }

    schema
}

/// Walk the original param schema and parse JSON-text values for properties
/// whose declared type is array or object back into structured JSON. Bashkit's
/// flag parser keeps these as raw strings; the domain dispatcher expects the
/// typed shape.
///
/// Empty / whitespace-only strings are left untouched so optional fields that
/// rely on `deserialize_opt_json_value_lenient` (e.g. `channel_config`) keep
/// their `--flag ''` → `None` behavior. The dispatcher applies the same trim
/// rule for non-empty strings, so we only intervene when there is JSON to
/// parse on the bash side.
fn coerce_json_text_params(
    schema: &serde_json::Value,
    params: &mut serde_json::Value,
) -> Result<(), String> {
    let (Some(properties), Some(params_obj)) = (
        schema.get("properties").and_then(|value| value.as_object()),
        params.as_object_mut(),
    ) else {
        return Ok(());
    };
    for (name, property) in properties.iter() {
        let needs_parse = property
            .get("type")
            .and_then(|value| value.as_str())
            .is_some_and(|value| matches!(value, "array" | "object"))
            || property.get("items").is_some()
            || property.get("properties").is_some();
        if !needs_parse {
            continue;
        }
        let Some(raw) = params_obj.get(name) else {
            continue;
        };
        let serde_json::Value::String(text) = raw else {
            continue;
        };
        if text.trim().is_empty() {
            continue;
        }
        let parsed = serde_json::from_str::<serde_json::Value>(text.trim())
            .map_err(|err| format!("--{name}: invalid JSON ({err})"))?;
        params_obj.insert(name.clone(), parsed);
    }
    Ok(())
}

fn make_inventory_callback(
    desc: &'static crate::domains::common::CommandDescriptor,
    ctx: CatalogContext,
) -> impl Fn(ToolArgs) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>>
+ Send
+ Sync
+ 'static {
    move |args: ToolArgs| {
        let mut params = args.params;
        let domain_ctx = ctx.to_domain_ctx();
        let link_builder = ctx.link_builder.clone();
        Box::pin(async move {
            // Schema rewriting in `bashkit_inventory_schema` advertises array
            // and object fields as `string` so bashkit's parser accepts JSON
            // text on the command line. Translate them back here so the
            // domain dispatcher sees the structured value it expects.
            let original_schema = (desc.param_schema)();
            coerce_json_text_params(&original_schema, &mut params)?;
            let result = (desc.dispatch)(params, &domain_ctx)
                .await
                .map_err(|error| error.to_string())?;
            decorate_command_output(&result, &link_builder)
        })
    }
}

fn decorate_command_output(
    result: &str,
    link_builder: &crate::api::common::UrlBuilder,
) -> Result<String, String> {
    let mut value = match serde_json::from_str::<serde_json::Value>(result) {
        Ok(value) => value,
        Err(_) => return Ok(result.to_string()),
    };
    link_builder.decorate_value_links(&mut value);
    serde_json::to_string(&value).map_err(|error| format!("Internal error: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_command_defs_expose_structured_param_schemas() {
        let desc = inventory::iter::<crate::domains::common::CommandDescriptor>
            .into_iter()
            .find(|desc| (desc.meta)().name == "create_agent")
            .expect("create_agent command descriptor");

        let def = command_descriptor_to_def(desc);
        let properties = def
            .input_schema
            .get("properties")
            .and_then(|value| value.as_object())
            .expect("create_agent schema properties");

        assert!(properties.contains_key("name"));
        assert!(properties.contains_key("system_prompt"));
        assert_ne!(
            def.input_schema.get("additionalProperties"),
            Some(&serde_json::Value::Bool(true))
        );
    }

    #[test]
    fn command_output_decoration_adds_ui_link_for_mcp_execute_results() {
        let builder =
            crate::api::common::UrlBuilder::new("https://api.example/api", "https://app.example");
        let result = decorate_command_output(
            r#"{"id":"agent_00000000000000000000000000000001","name":"demo"}"#,
            &builder,
        )
        .expect("decorated output");
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();

        assert_eq!(
            value["self_url"],
            "https://api.example/api/v1/agents/agent_00000000000000000000000000000001"
        );
        assert_eq!(
            value["ui_link"],
            "https://app.example/agents/agent_00000000000000000000000000000001"
        );
    }

    #[test]
    fn bashkit_schema_retypes_array_and_object_fields_to_string() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string" },
                "capabilities": {
                    "type": "array",
                    "items": { "type": "object" },
                    "description": "List of capabilities"
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "channel_config": {
                    "type": "object",
                    "description": "Channel config"
                }
            }
        });
        let rewritten = bashkit_inventory_schema(schema);
        let props = rewritten["properties"].as_object().unwrap();
        // Scalars unchanged
        assert_eq!(props["id"]["type"], "string");
        // Arrays and objects retyped to string with JSON-text hint
        for key in ["capabilities", "tags", "channel_config"] {
            assert_eq!(
                props[key]["type"], "string",
                "{key} should be retyped to string",
            );
            assert!(
                props[key]["description"]
                    .as_str()
                    .unwrap()
                    .contains("JSON text in bash mode"),
            );
            assert!(
                props[key].get("items").is_none(),
                "items metadata should be dropped for {key}",
            );
        }
    }

    #[test]
    fn coerce_parses_json_text_for_array_and_object_fields() {
        let schema = serde_json::json!({
            "properties": {
                "id": { "type": "string" },
                "capabilities": { "type": "array", "items": { "type": "object" } },
                "tags": { "type": "array", "items": { "type": "string" } }
            }
        });
        let mut params = serde_json::json!({
            "id": "agent_1",
            "capabilities": "[{\"ref\":\"current_time\"}]",
            "tags": "[\"a\",\"b\"]"
        });
        coerce_json_text_params(&schema, &mut params).expect("coerce");
        assert_eq!(params["id"], "agent_1");
        assert_eq!(
            params["capabilities"],
            serde_json::json!([{"ref": "current_time"}]),
        );
        assert_eq!(params["tags"], serde_json::json!(["a", "b"]));
    }

    #[test]
    fn coerce_surfaces_json_parse_errors_with_field_name() {
        let schema = serde_json::json!({
            "properties": {
                "capabilities": { "type": "array", "items": {} }
            }
        });
        let mut params = serde_json::json!({ "capabilities": "[not-json" });
        let err = coerce_json_text_params(&schema, &mut params).unwrap_err();
        assert!(err.starts_with("--capabilities:"), "got: {err}");
        assert!(err.contains("invalid JSON"));
    }

    #[test]
    fn coerce_leaves_empty_and_whitespace_strings_untouched() {
        // Optional non-scalar fields like `channel_config` rely on
        // `deserialize_opt_json_value_lenient`, which maps empty/whitespace
        // strings to `None`. Eagerly parsing those would regress the bash
        // ergonomics (`--channel_config ''` should still mean "unset").
        let schema = serde_json::json!({
            "properties": {
                "channel_config": { "type": "object", "properties": {} }
            }
        });
        for blank in ["", "   ", "\t\n"] {
            let mut params = serde_json::json!({ "channel_config": blank });
            coerce_json_text_params(&schema, &mut params).expect("coerce");
            assert_eq!(
                params["channel_config"],
                serde_json::Value::String(blank.to_string()),
                "blank input {blank:?} should pass through untouched",
            );
        }
    }

    #[test]
    fn coerce_passes_through_already_parsed_values() {
        let schema = serde_json::json!({
            "properties": {
                "capabilities": { "type": "array", "items": {} }
            }
        });
        // Non-bash MCP clients send the structured value directly; the coercer
        // must leave it alone.
        let original = serde_json::json!([{ "ref": "current_time" }]);
        let mut params = serde_json::json!({ "capabilities": original.clone() });
        coerce_json_text_params(&schema, &mut params).expect("coerce");
        assert_eq!(params["capabilities"], original);
    }

    #[test]
    fn command_output_decoration_preserves_non_json_results() {
        let builder =
            crate::api::common::UrlBuilder::new("https://api.example/api", "https://app.example");

        assert_eq!(
            decorate_command_output("plain result", &builder).expect("decorated output"),
            "plain result"
        );
    }
}
