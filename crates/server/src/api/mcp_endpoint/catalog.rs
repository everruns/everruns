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

    for (name, property) in properties.iter_mut() {
        if name != "channel_config" {
            continue;
        }
        let Some(object) = property.as_object_mut() else {
            continue;
        };
        object.insert(
            "type".to_string(),
            serde_json::Value::String("string".to_string()),
        );
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

fn make_inventory_callback(
    desc: &'static crate::domains::common::CommandDescriptor,
    ctx: CatalogContext,
) -> impl Fn(ToolArgs) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>>
+ Send
+ Sync
+ 'static {
    move |args: ToolArgs| {
        let params = args.params;
        let domain_ctx = ctx.to_domain_ctx();
        Box::pin(async move {
            (desc.dispatch)(params, &domain_ctx)
                .await
                .map_err(|error| error.to_string())
        })
    }
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
}
