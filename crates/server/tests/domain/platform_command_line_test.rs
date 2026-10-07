//! The `everruns <noun> <verb>` spelling inside the scripted toolset (MCP
//! `execute` / `query` and the Platform capability).
//!
//! The line is resolved by the shared mapper as a real builtin, so these drive
//! the real interpreter: quoting, pipelines, command substitution, exit codes
//! and the read-only gate, end to end against a database.
//!
//! Run with: cargo test -p everruns-server --test domain platform_command_line_test::

use crate::test_harness;

use everruns_core::{Caller, DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, OrgRole};
use everruns_server::domains::common::Ctx;
use everruns_server::services::platform_command_surface::{CatalogContext, Operation, invoke};
use serde_json::json;
use test_harness::TestServer;

fn context(server: &TestServer) -> CatalogContext {
    let ctx = Ctx::minimal(
        Caller {
            org_id: DEFAULT_ORG_ID,
            org_public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
            user_id: Some(uuid::Uuid::nil()),
            role: OrgRole::Owner,
            is_platform_user: false,
            is_internal: false,
        },
        server.db.clone(),
        server.encryption.clone(),
        std::sync::Arc::new(everruns_core::DefaultPermissionResolver),
    );
    CatalogContext {
        domain_ctx: ctx,
        link_builder: everruns_server::api::common::UrlBuilder::new(
            "http://localhost",
            "http://localhost",
        ),
    }
}

async fn run(server: &TestServer, operation: Operation, script: &str) -> Result<String, String> {
    invoke(operation, &json!({ "commands": script }), context(server)).await
}

fn unique(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        &uuid::Uuid::now_v7().simple().to_string()[20..]
    )
}

#[tokio::test]
async fn help_is_bounded_and_composes_with_a_pipeline() {
    let server = TestServer::in_memory().await;

    let root = run(&server, Operation::Query, "everruns --help")
        .await
        .expect("root help");
    assert!(root.contains("agents"), "{root}");
    assert!(root.contains("mcp-servers"), "{root}");

    let node = run(&server, Operation::Query, "everruns agents")
        .await
        .expect("a bare noun lists its verbs");
    assert!(node.contains("triggers"), "{node}");

    let leaf = run(&server, Operation::Query, "everruns agents list --help")
        .await
        .expect("leaf help");
    assert!(leaf.contains("Wire name: list_agents"), "{leaf}");

    let head = run(&server, Operation::Query, "everruns --help | head -3")
        .await
        .expect("help survives a pipeline");
    assert!(head.lines().count() <= 3, "{head}");
}

#[tokio::test]
async fn a_created_agent_flows_through_command_substitution() {
    let server = TestServer::in_memory().await;
    let name = unique("line");

    // The prompt carries shell syntax: argv arrives already split, so it is
    // one value, not a second statement.
    let script = format!(
        "id=$(everruns agents create --name {name} --system-prompt 'Be brief; never guess' | jq -r .id)\n\
         everruns agents get \"$id\" | jq -r .system_prompt"
    );
    let out = run(&server, Operation::Execute, &script)
        .await
        .expect("create then read");
    assert_eq!(out.trim(), "Be brief; never guess");
}

#[tokio::test]
async fn query_refuses_a_mutating_spelling_but_reads() {
    let server = TestServer::in_memory().await;

    let error = run(
        &server,
        Operation::Query,
        "everruns agents create --name x --system-prompt p",
    )
    .await
    .expect_err("query must not mutate");
    assert!(error.contains("not available in query"), "{error}");

    run(&server, Operation::Query, "everruns agents list --limit 1")
        .await
        .expect("reads run in query");
}

#[tokio::test]
async fn flat_names_remain_aliases() {
    let server = TestServer::in_memory().await;
    run(&server, Operation::Query, "list_agents --limit 1 | jq .")
        .await
        .expect("flat name still runs");
}

#[tokio::test]
async fn a_wrong_word_or_flag_fails_with_guidance() {
    let server = TestServer::in_memory().await;

    let verb = run(&server, Operation::Query, "everruns agents lst")
        .await
        .expect_err("an unknown verb must not look like success");
    assert!(verb.contains("unknown command `lst`"), "{verb}");
    assert!(verb.contains("list"), "{verb}");

    let flag = run(&server, Operation::Query, "everruns agents list --limti 5")
        .await
        .expect_err("a misspelled flag is rejected before dispatch");
    assert!(flag.contains("--limit"), "{flag}");
}
