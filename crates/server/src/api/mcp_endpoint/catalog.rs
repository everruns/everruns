use bashkit::{ScriptingToolSet, ToolArgs, ToolDef};
use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Clone)]
pub struct CatalogContext {
    pub state: super::AppState,
    pub caller: everruns_core::permissions::Caller,
}

static INVENTORY_TOOL_DEFS: LazyLock<HashMap<&'static str, ToolDef>> = LazyLock::new(|| {
    inventory::iter::<crate::domains::common::CommandDescriptor>
        .into_iter()
        .map(|desc| {
            let meta = (desc.meta)();
            let def = ToolDef::new(meta.name, meta.description)
                .with_schema((desc.param_schema)())
                .with_category(meta.category);
            (meta.name, def)
        })
        .collect()
});

impl CatalogContext {
    pub fn to_domain_ctx(&self) -> crate::domains::common::Ctx {
        crate::domains::common::Ctx::new(
            self.caller.clone(),
            self.state.db.clone(),
            self.state.capability_service.clone(),
            self.state.encryption.clone(),
        )
        .with_mcp_runtime(crate::domains::common::McpRuntime {
            session_service: self.state.session_service.clone(),
            message_service: self.state.message_service.clone(),
            event_service: self.state.event_service.clone(),
            budget_service: self.state.budget_service.clone(),
            runner: self.state.runner.clone(),
            fallback_base_harness_name: self.state.fallback_base_harness_name.clone(),
        })
    }
}

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
        builder = builder.tool(def, callback);
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
                .with_schema((desc.param_schema)())
                .with_category(meta.category)
        })
}

fn make_inventory_callback(
    desc: &'static crate::domains::common::CommandDescriptor,
    ctx: CatalogContext,
) -> impl Fn(&ToolArgs) -> Result<String, String> + Send + Sync + 'static {
    move |args: &ToolArgs| {
        let params = args.params.clone();
        let domain_ctx = ctx.to_domain_ctx();
        tokio::task::block_in_place(|| {
            let handle = tokio::runtime::Handle::current();
            handle.block_on(async {
                (desc.dispatch)(params, &domain_ctx)
                    .await
                    .map_err(|e| e.to_string())
            })
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
