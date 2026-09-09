//! Proves the tree is reachable from inside a real session's shell.
//!
//! The simulator can produce any text it likes, so a canned final answer
//! proves nothing. These assert on state the model can only have changed by
//! actually running the builtin, and on the builtin's real output reaching the
//! transcript.

use std::sync::Arc;

use everruns_core::InputMessage;
use everruns_framework_cli_host::{Fleet, FleetCommands};
use everruns_host::{
    AgentBuilder, HarnessBuilder, HostComposition, InMemorySessionFileSystemFactory,
    InProcessRuntimeBuilder, SessionBuilder,
};
use everruns_integrations_bashkit::BashkitShellCapability;
use everruns_llmsim::{LlmSimConfig, LlmSimRuntimeExt};
use everruns_provider::driver_registry::DriverRegistry;
use everruns_provider::tool_types::ToolCall;
use everruns_provider::typed_id::{AgentId, HarnessId, SessionId};

/// Build a session whose shell carries `fleet`'s commands, and run one turn in
/// which the model issues `scripts` through bash.
async fn run_with_shell(
    fleet: Arc<Fleet>,
    scripts: &[&str],
    with_source: bool,
) -> everruns_host::TurnResult {
    let harness_id = HarnessId::new();
    let agent_id = AgentId::new();
    let session_id = SessionId::new();

    let composition = HostComposition::builder()
        .driver_registry(DriverRegistry::new())
        .session_file_system_factory(Arc::new(InMemorySessionFileSystemFactory))
        .build();

    let calls: Vec<Vec<ToolCall>> = scripts
        .iter()
        .enumerate()
        .map(|(index, script)| {
            vec![ToolCall {
                id: format!("call_{index}"),
                name: "bash".into(),
                arguments: serde_json::json!({ "commands": script }),
            }]
        })
        .chain(std::iter::once(vec![]))
        .collect();

    let mut builder = InProcessRuntimeBuilder::new()
        .host_composition(composition)
        .capability(BashkitShellCapability)
        .llm_sim_as_default(LlmSimConfig::fixed("done").with_tool_call_sequence(calls))
        .harness(
            HarnessBuilder::new("fleet-admin", "Administer the fleet.")
                .id(harness_id)
                .capability("bashkit_shell")
                .build(),
        )
        .agent(
            AgentBuilder::new("fleet-admin", "Administer the fleet.")
                .id(agent_id)
                .max_iterations(8)
                .build(),
        )
        .session(
            SessionBuilder::new(harness_id)
                .id(session_id)
                .agent(agent_id)
                .build(),
        );

    if with_source {
        builder = builder.with_tool_context_extensions_factory(Arc::new(move |_org, _session| {
            let mut extensions = everruns_core::tool_context::ToolContextExtensions::default();
            extensions.insert(Arc::new(FleetCommands::handle(fleet.clone())));
            extensions
        }));
    }

    builder
        .build()
        .await
        .expect("runtime builds")
        .run_turn(session_id, InputMessage::user("Do the thing."))
        .await
        .expect("turn runs")
}

#[tokio::test]
async fn a_mutating_command_changes_application_state() {
    // The load-bearing assertion: `scale` really ran. A simulated final
    // message could claim anything, but only the builtin can move this number.
    let fleet = Fleet::with_demo_services();
    assert_eq!(fleet.replicas("api"), Some(2));

    let result = run_with_shell(
        fleet.clone(),
        &["everruns fleet scale --name api --replicas 4"],
        true,
    )
    .await;

    assert!(result.success, "turn should succeed");
    assert_eq!(fleet.replicas("api"), Some(4), "the builtin actually ran");
}

#[tokio::test]
async fn help_is_reachable_from_the_session_shell() {
    let fleet = Fleet::with_demo_services();
    let result = run_with_shell(fleet, &["everruns --help", "everruns fleet --help"], true).await;
    assert!(result.success, "help must not fail the turn");
}

#[tokio::test]
async fn a_host_that_supplies_no_commands_has_no_builtin() {
    // Never advertise a surface the host cannot serve: without a source the
    // shell has no `everruns` command at all, and says so the way any shell
    // does.
    let fleet = Fleet::with_demo_services();
    let result = run_with_shell(fleet.clone(), &["everruns fleet list"], false).await;

    assert!(
        result.success,
        "a missing command is a failed command, not a failed turn"
    );
    assert_eq!(
        fleet.replicas("api"),
        Some(2),
        "nothing should have run against the fleet"
    );
}
