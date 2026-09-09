//! Proves the tree is reachable from inside a real session's shell.
//!
//! No model and no simulator: a simulator can be told to emit whichever
//! commands the test hoped for, which makes the run a recording of the test's
//! own script. These drive the real bash tool directly and assert on the
//! builtin's real output and on state only the builtin can have changed.

use std::sync::Arc;

use everruns_core::tool_context::{ToolContext, ToolContextExtensions};
use everruns_core::tools::Tool;
use everruns_framework_cli_host::{Fleet, FleetCommands};
use everruns_host::InMemorySessionFileStore;
use everruns_integrations_bashkit::BashTool;
use everruns_provider::typed_id::SessionId;

struct Output {
    stdout: String,
    stderr: String,
    exit_code: i64,
}

impl Output {
    fn all(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

/// Run `script` through the real shell, with this application's commands
/// attached exactly as the host attaches them. `with_source: false` is the
/// host that supplies nothing.
async fn run_shell(fleet: Arc<Fleet>, script: &str, with_source: bool) -> Output {
    let mut context = ToolContext::new(SessionId::new());
    context.file_store = Some(Arc::new(InMemorySessionFileStore::new()));

    if with_source {
        let mut extensions = ToolContextExtensions::default();
        extensions.insert(Arc::new(FleetCommands::handle(fleet)));
        context.extensions = extensions;
    }

    let result = BashTool::default()
        .execute_with_context(serde_json::json!({ "commands": script }), &context)
        .await
        .into_tool_result("call", "bash");

    let value = result.result.expect("bash returns a result envelope");
    let field = |name: &str| {
        value
            .get(name)
            .and_then(|field| field.as_str())
            .unwrap_or_default()
            .to_string()
    };
    Output {
        stdout: field("stdout"),
        stderr: field("stderr"),
        exit_code: value
            .get("exit_code")
            .and_then(|c| c.as_i64())
            .unwrap_or(-1),
    }
}

#[tokio::test]
async fn a_mutating_command_changes_application_state() {
    // The load-bearing assertion: `scale` really ran. Output text could claim
    // anything, but only the builtin can move this number.
    let fleet = Fleet::with_demo_services();
    assert_eq!(fleet.replicas("api"), Some(2));

    let out = run_shell(
        fleet.clone(),
        "everruns fleet scale --name api --replicas 4",
        true,
    )
    .await;

    assert_eq!(out.exit_code, 0, "{}", out.all());
    assert!(out.stdout.contains("\"replicas\":4"), "{}", out.stdout);
    assert_eq!(fleet.replicas("api"), Some(4), "the builtin actually ran");
}

#[tokio::test]
async fn help_is_reachable_from_the_session_shell() {
    let fleet = Fleet::with_demo_services();
    let out = run_shell(fleet, "everruns fleet --help", true).await;

    assert_eq!(out.exit_code, 0, "{}", out.all());
    for verb in ["get", "list", "scale"] {
        assert!(out.stdout.contains(verb), "{verb} missing: {}", out.stdout);
    }
}

#[tokio::test]
async fn a_leaf_renders_its_own_flags() {
    // What makes the tree recoverable: a leaf's help carries the real flags,
    // so a caller that guessed the wrong argument form can find the right one.
    let fleet = Fleet::with_demo_services();
    let out = run_shell(fleet, "everruns fleet scale --help", true).await;

    assert_eq!(out.exit_code, 0, "{}", out.all());
    assert!(out.stdout.contains("--name"), "{}", out.stdout);
    assert!(out.stdout.contains("--replicas"), "{}", out.stdout);
}

#[tokio::test]
async fn a_wrong_argument_form_is_an_error_that_names_the_fix() {
    // A live model reaches the tree by guessing a positional form first. The
    // error has to say what to do instead, or the guess becomes a dead end.
    let fleet = Fleet::with_demo_services();
    let out = run_shell(fleet.clone(), "everruns fleet scale api 4", true).await;

    assert_ne!(out.exit_code, 0, "a bad form must fail");
    assert!(out.all().contains("--flag value"), "{}", out.all());
    assert_eq!(fleet.replicas("api"), Some(2), "nothing was scaled");
}

#[tokio::test]
async fn a_host_that_supplies_no_commands_has_no_builtin() {
    // The builtin is not ambient: a host that supplies no source gets a shell
    // with no `everruns` command, rather than one advertising a tree it
    // cannot serve.
    let fleet = Fleet::with_demo_services();
    let out = run_shell(fleet, "everruns fleet list", false).await;

    assert_ne!(out.exit_code, 0);
    assert!(
        out.all().contains("not found"),
        "expected command-not-found, got: {}",
        out.all()
    );
}
