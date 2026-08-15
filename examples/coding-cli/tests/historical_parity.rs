//! Contract for the intentionally small coding CLI surface.

use clap::{CommandFactory, Parser};
use everruns::{
    AgentInstructionsConfig, BashkitShell, DuckDuckGo, FileSystem, InMemoryEngine, IntoCapability,
    Model, Skills, StatelessTodoList, WebFetch,
};
use everruns_coding_cli::{Cli, coding_agent};

#[test]
fn print_approval_and_model_flags_remain_available() {
    let cli = Cli::try_parse_from([
        "ercode",
        "--model",
        "gpt-5.6-terra",
        "--ask",
        "--print",
        "inspect the repository",
    ])
    .expect("historical flags parse");
    assert_eq!(cli.model(), "gpt-5.6-terra");
    assert_eq!(cli.prompt_text().as_deref(), Some("inspect the repository"));
    assert!(!cli.ask(), "print mode must never block for approval");

    let help = Cli::command().render_long_help().to_string();
    for expected in [
        "--model <MODEL>",
        "--print <PROMPT>",
        "--ask",
        "--reasoning-effort <EFFORT>",
    ] {
        assert!(help.contains(expected), "missing {expected:?} in:\n{help}");
    }
    assert!(!help.contains("--mcp-config"));
}

#[test]
fn interactive_surface_keeps_the_multiline_tui() {
    let source = include_str!("../src/main.rs");
    for contract in [
        "TextArea::default()",
        "KeyModifiers::ALT | KeyModifiers::SHIFT",
    ] {
        assert!(
            source.contains(contract),
            "missing historical contract {contract}"
        );
    }
}

#[test]
fn typed_capability_values_come_from_their_owning_packages() {
    let capabilities = [
        FileSystem.into_capability(),
        BashkitShell::new().enable_http(true).into_capability(),
        AgentInstructionsConfig::default().into_capability(),
        Skills.into_capability(),
        StatelessTodoList.into_capability(),
        WebFetch::new().enable_file_download(true).into_capability(),
        DuckDuckGo.into_capability(),
    ];
    let ids = capabilities
        .iter()
        .map(|capability| capability.capability_ref().id())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        [
            "session_file_system",
            "bashkit_shell",
            "agent_instructions",
            "skills",
            "stateless_todo_list",
            "web_fetch",
            "duckduckgo",
        ]
    );
    assert_eq!(
        capabilities[1].capability_ref().config_value()["enable_http"],
        true
    );
    assert_eq!(
        capabilities[5].capability_ref().config_value()["enable_file_download"],
        true
    );
}

#[tokio::test]
async fn coding_capabilities_are_available_through_the_facade() {
    let agent = coding_agent(Model::simulated("ready"))
        .build()
        .expect("coding agent builds");
    let context = InMemoryEngine::new()
        .create(agent)
        .inspect()
        .await
        .expect("inspect tools");
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
