//! Regression contract for the pre-facade coding CLI feature set.

use clap::{CommandFactory, Parser};
use everruns::Model;
use everruns_coding_cli::{Cli, ProviderChoice, agent_builder};

#[test]
fn provider_print_approval_and_model_flags_remain_available() {
    let cli = Cli::try_parse_from([
        "ercode",
        "--provider",
        "anthropic",
        "--model",
        "claude-sonnet-4-5",
        "--ask",
        "--print",
        "inspect the repository",
    ])
    .expect("historical flags parse");
    assert_eq!(cli.provider(), ProviderChoice::Anthropic);
    assert_eq!(cli.model(), "claude-sonnet-4-5");
    assert_eq!(cli.prompt_text().as_deref(), Some("inspect the repository"));
    assert!(!cli.ask(), "print mode must never block for approval");

    let help = Cli::command().render_long_help().to_string();
    for expected in [
        "--provider <PROVIDER>",
        "--model <MODEL>",
        "--print <PROMPT>",
        "--ask",
        "--mcp-config <FILE>",
        "--reasoning-effort <EFFORT>",
    ] {
        assert!(help.contains(expected), "missing {expected:?} in:\n{help}");
    }
}

#[test]
fn interactive_surface_keeps_multiline_tui_and_history_preserving_model_switch() {
    let source = include_str!("../src/main.rs");
    for contract in [
        "TextArea::default()",
        "KeyModifiers::ALT | KeyModifiers::SHIFT",
        ".resume(session.session_id())",
        "command.starts_with(\"/model \")",
    ] {
        assert!(
            source.contains(contract),
            "missing historical contract {contract}"
        );
    }
}

#[tokio::test]
async fn coding_capability_catalog_is_available_through_the_facade() {
    let agent = agent_builder()
        .model(Model::simulated("ready"))
        .build()
        .expect("coding agent builds");
    let context = agent.session().inspect().await.expect("inspect tools");
    let tools = context
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    for expected in [
        "read_file",
        "write_file",
        "list_skills",
        "write_todos",
        "web_fetch",
        "duckduckgo_instant_answer",
    ] {
        assert!(
            tools.contains(&expected),
            "missing {expected}; got {tools:?}"
        );
    }
    assert!(
        tools.iter().any(|name| name.contains("bash")),
        "bashkit shell tool missing; got {tools:?}"
    );
}
