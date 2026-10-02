//! `[sandbox] kind = "microvm"` on AgentCore: a real shell over the session
//! workspace, not bashkit.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::json;
use serve::prelude::*;
use serve::{AppConfig, Mode, sim};
use serve_agentcore::{Options, SESSION_HEADER, router};

/// Writes a file with the shell, then replies.
#[agent(default)]
fn builder() -> Agent {
    Agent::builder()
        .model("sim")
        .instructions("Test agent.")
        .offline(sim::script([
            sim::call(
                "bash",
                json!({ "command": "echo \"kernel $(uname -s)\" > note.txt && cat note.txt" }),
            ),
            sim::reply("written"),
        ]))
        .build()
}

#[tokio::test]
async fn microvm_sandbox_runs_a_real_shell_in_the_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let config = AppConfig::parse("[sandbox]\nkind = \"microvm\"\n").unwrap();
    let app = App::builder().discover().config(config).build();
    assert!(app.errors().is_empty(), "{:?}", app.errors());
    let mut options = Options::new(Mode::Eval);
    options.data_dir = Some(dir.path().join("data"));
    options.workspace = Some(dir.path().join("workspace"));
    let (router, _) = router(app, options).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let body = reqwest::Client::new()
        .post(format!("{base}/invocations"))
        .header(SESSION_HEADER, "microvm-session-000000000000000000000001")
        .json(&json!({ "prompt": "write a note" }))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(body.contains("RUN_FINISHED"), "{body}");

    // `uname` only exists on a real machine; bashkit would have no such file
    // on disk at all.
    let note = std::fs::read_to_string(dir.path().join("workspace").join("note.txt")).unwrap();
    let kernel = note.trim().strip_prefix("kernel ").unwrap();
    assert!(!kernel.is_empty(), "{note}");
}
