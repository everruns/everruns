// MCP scripting catalog — inventory is the only source of truth.

use bashkit::{ScriptingToolSet, ToolArgs, ToolDef};
use std::future::Future;
use std::pin::Pin;

/// Shared context for inventory-registered domain commands.
#[derive(Clone)]
pub struct CatalogContext {
    pub state: super::AppState,
    pub caller: everruns_core::permissions::Caller,
}

impl CatalogContext {
    /// Convert to a domain Ctx for inventory-registered command dispatch.
    pub fn to_domain_ctx(&self) -> crate::domains::common::Ctx {
        let mut ctx = crate::domains::common::Ctx::new(
            self.caller.clone(),
            self.state.db.clone(),
            self.state.capability_service.clone(),
            self.state.encryption.clone(),
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
        if let Some(store) = &self.state.workflow_store {
            ctx = ctx.with_workflow_store(store.clone());
        }
        ctx
    }
}

/// Build a ScriptingToolSet from inventory-registered commands only.
pub fn build_toolset(ctx: CatalogContext) -> ScriptingToolSet {
    let mut builder = ScriptingToolSet::builder("everruns")
        .short_description("Everruns API operations as bash builtins")
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
        let def = command_descriptor_to_def(desc);
        let callback = make_inventory_callback(desc, ctx.clone());
        builder = builder.async_tool(def, callback);
    }

    builder.build()
}

fn command_descriptor_to_def(desc: &crate::domains::common::CommandDescriptor) -> ToolDef {
    let meta = (desc.meta)();
    ToolDef::new(meta.name, meta.description)
        .with_schema((desc.param_schema)())
        .with_category(meta.category)
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
