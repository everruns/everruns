//! Downstream-style coverage for the unified Framework capability entrypoint.

#![cfg(feature = "builtins")]

use everruns::{
    Agent, BuildError, CapabilityRef, CapabilitySpec, CompactionConfig, FunctionTool,
    IntoCapability, Model, ToolSearch,
};
use serde_json::{Value, json};

fn builder() -> everruns::AgentBuilder {
    Agent::builder()
        .instructions("Use the configured capabilities.")
        .model(Model::simulated("done"))
}

#[tokio::test]
async fn typed_compaction_runs_offline() {
    let agent = builder()
        .capability(CompactionConfig::new().budget_percent(0.85))
        .build()
        .expect("typed compaction config builds");

    let turn = agent
        .session()
        .run("Keep this context useful.")
        .await
        .expect("offline turn runs");
    assert!(turn.success);
}

#[tokio::test]
async fn typed_tool_search_activates_the_model_adaptive_builtin() {
    let agent = builder()
        .capability(
            ToolSearch::automatic()
                .threshold(1)
                .never_defer(["get_current_time"]),
        )
        .capability("current_time")
        .build()
        .expect("typed tool search builds");

    let session = agent.session();
    let context = session.inspect().await.expect("context assembles");
    assert!(context.tools.iter().any(|tool| tool.name == "tool_search"));
    assert!(
        context
            .tools
            .iter()
            .any(|tool| tool.name == "get_current_time")
    );
}

#[tokio::test]
async fn dynamic_reference_activates_a_known_capability() {
    let agent = builder()
        .capability(CapabilityRef::new("current_time").config(json!({})))
        .build()
        .expect("dynamic reference builds");

    let session = agent.session();
    let context = session.inspect().await.expect("context assembles");
    assert!(
        context
            .tools
            .iter()
            .any(|tool| tool.name == "get_current_time")
    );
}

struct ExternalCurrentTime;

impl IntoCapability for ExternalCurrentTime {
    fn into_capability(self) -> CapabilitySpec {
        CapabilityRef::new("current_time").into()
    }
}

#[tokio::test]
async fn external_conversion_contract_needs_only_the_facade() {
    let agent = builder()
        .capability(ExternalCurrentTime)
        .build()
        .expect("external capability value builds");

    let session = agent.session();
    let context = session.inspect().await.expect("context assembles");
    assert!(
        context
            .tools
            .iter()
            .any(|tool| tool.name == "get_current_time")
    );
}

#[tokio::test]
async fn unknown_dynamic_reference_remains_an_open_noop_until_implemented() {
    let agent = builder()
        .capability(
            CapabilityRef::new("vendor.custom").config(json!({ "mode": "database-driven" })),
        )
        .build()
        .expect("open third-party reference builds");

    let turn = agent
        .session()
        .run("Continue without a local implementation.")
        .await
        .expect("unknown references do not break offline execution");
    assert!(turn.success);
}

#[test]
fn invalid_ids_and_json_boundaries_are_rejected() {
    for id in [
        "",
        "2fast",
        "has space",
        "vendor/custom",
        "__everruns_private",
    ] {
        let error = builder()
            .capability(CapabilityRef::new(id))
            .build()
            .expect_err("invalid capability id must fail");
        assert!(matches!(error, BuildError::InvalidCapability { .. }));
    }

    let error = builder()
        .capability(CapabilityRef::new("vendor.custom").config(json!(["not", "an", "object"])))
        .build()
        .expect_err("non-object config must fail");
    assert!(matches!(error, BuildError::InvalidCapability { .. }));
}

#[test]
fn dynamic_config_is_redacted_from_framework_debug_output() {
    let reference = CapabilityRef::new("vendor.custom").config(json!({
        "credential": "must-not-appear"
    }));
    assert!(!format!("{reference:?}").contains("must-not-appear"));

    let agent = builder()
        .capability(reference)
        .build()
        .expect("dynamic reference builds");
    assert!(!format!("{agent:?}").contains("must-not-appear"));
}

#[test]
fn known_builtin_configs_use_the_implementation_validator() {
    let error = builder()
        .capability(CapabilityRef::new("compaction").config(json!({ "budget_percent": 2.0 })))
        .build()
        .expect_err("invalid built-in config must fail at Agent build");
    assert!(matches!(
        error,
        BuildError::InvalidCapability { ref id, .. } if id == "compaction"
    ));

    let error = builder()
        .capability(CapabilityRef::new("auto_tool_search").config(json!({
            "never_defer": ["bad name"]
        })))
        .build()
        .expect_err("invalid typed tool-search field must fail");
    assert!(matches!(
        error,
        BuildError::InvalidCapability { ref id, .. } if id == "auto_tool_search"
    ));
}

#[test]
fn duplicate_ids_across_typed_dynamic_and_string_inputs_are_rejected() {
    let error = builder()
        .capability(CompactionConfig::new())
        .capability(CapabilityRef::new("compaction"))
        .build()
        .expect_err("typed and dynamic refs must collide");
    assert_eq!(
        error,
        BuildError::DuplicateCapability {
            id: "compaction".into(),
        }
    );

    let error = builder()
        .capability(ToolSearch::automatic())
        .capability("auto_tool_search")
        .build()
        .expect_err("typed and string refs must collide");
    assert_eq!(
        error,
        BuildError::DuplicateCapability {
            id: "auto_tool_search".into(),
        }
    );

    #[cfg(feature = "bashkit")]
    {
        let error = builder()
            .capability(CapabilityRef::new("virtual_bash"))
            .capability("bashkit_shell")
            .build()
            .expect_err("an enabled integration alias and canonical ID must collide");
        assert_eq!(
            error,
            BuildError::DuplicateCapability {
                id: "bashkit_shell".into(),
            }
        );
    }
}

#[test]
fn function_tool_and_capability_reference_do_not_overwrite_each_other() {
    let tool = FunctionTool::new(
        "vendor_lookup",
        "Look up a vendor record.",
        json!({ "type": "object", "properties": {} }),
        |_: Value| async move { Ok::<_, String>(json!({ "found": true })) },
    );
    let error = builder()
        .capability("vendor_lookup")
        .tool(tool)
        .build()
        .expect_err("function implementation and reference must collide");
    assert_eq!(
        error,
        BuildError::DuplicateCapability {
            id: "vendor_lookup".into(),
        }
    );

    let reserved = FunctionTool::new(
        "__everruns_framework_lifecycle_hooks",
        "Attempt to shadow a private implementation.",
        json!({ "type": "object", "properties": {} }),
        |_: Value| async move { Ok::<_, String>(json!({})) },
    );
    let error = builder()
        .tool(reserved)
        .build()
        .expect_err("function implementation must not use a reserved capability id");
    assert!(matches!(
        error,
        BuildError::InvalidCapability { ref id, .. }
            if id == "__everruns_framework_lifecycle_hooks"
    ));
}

#[cfg(feature = "capabilities")]
mod definitions {
    use everruns::{Agent, BuildError, CapabilityRef, Model, capability};

    #[derive(capability::Deserialize, capability::JsonSchema)]
    #[serde(crate = "everruns::capability::serde")]
    #[schemars(crate = "everruns::capability::schemars")]
    struct EchoInput {
        value: String,
    }

    #[derive(capability::Serialize, capability::JsonSchema)]
    #[serde(crate = "everruns::capability::serde")]
    #[schemars(crate = "everruns::capability::schemars")]
    struct EchoOutput {
        value: String,
    }

    struct Echo;

    #[capability::async_trait]
    impl capability::Handler for Echo {
        type Input = EchoInput;
        type Output = EchoOutput;
        type Error = capability::Error;

        fn name(&self) -> &str {
            "vendor_echo"
        }

        fn description(&self) -> &str {
            "Echo one vendor value."
        }

        async fn execute(
            &self,
            input: Self::Input,
            _context: capability::Context,
        ) -> Result<Self::Output, Self::Error> {
            Ok(EchoOutput { value: input.value })
        }
    }

    fn builder() -> everruns::AgentBuilder {
        Agent::builder()
            .instructions("Use vendor tools.")
            .model(Model::simulated("done"))
    }

    fn definition(id: &str) -> capability::Definition {
        capability::Definition::new(id, "Vendor", "Vendor-defined tools.").tool(Echo)
    }

    #[tokio::test]
    async fn code_definition_registers_and_activates_through_capability() {
        let agent = builder()
            .capability(definition("vendor.echo"))
            .build()
            .expect("code definition builds");

        let session = agent.session();
        let context = session.inspect().await.expect("context assembles");
        assert!(context.tools.iter().any(|tool| tool.name == "vendor_echo"));
    }

    #[test]
    fn definition_reference_and_builtin_implementation_collisions_are_rejected() {
        let error = builder()
            .capability(CapabilityRef::new("vendor.echo"))
            .capability(definition("vendor.echo"))
            .build()
            .expect_err("reference and implementation must collide");
        assert_eq!(
            error,
            BuildError::DuplicateCapability {
                id: "vendor.echo".into(),
            }
        );

        let error = builder()
            .capability(definition("current_time"))
            .build()
            .expect_err("code definition must not shadow a built-in");
        assert_eq!(
            error,
            BuildError::DuplicateCapability {
                id: "current_time".into(),
            }
        );
    }
}
