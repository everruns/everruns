//! Serve an agent to AG-UI clients (CopilotKit, `@ag-ui/client`) from axum.
//!
//! Run with:
//!
//! ```text
//! cargo run -p everruns --features ag-ui-axum --example ag_ui_axum
//! curl -N localhost:3000/ag-ui -H 'authorization: Bearer dev-token' \
//!   -H 'content-type: application/json' -d '{
//!   "threadId": "t1", "runId": "r1", "protocolVersion": "1.0",
//!   "messages": [{ "id": "m1", "role": "user", "content": "Hi" }]
//! }'
//! ```
//!
//! `AgUiHandler` is the whole endpoint: it checks the bearer token, resolves
//! each AG-UI thread to a session with `AgUiThreads`, and streams the run as
//! SSE. One `InterruptGate` answers every session's `ask_user` questions and
//! tool approvals through AG-UI interrupts. The simulated model keeps the
//! example offline; swap in a real provider. With the `local` feature, pass
//! `SqliteThreadStore::local(&config)` and an agent on that `LocalConfig` to
//! keep threads across restarts.

use axum::Router;
use everruns::ag_ui::{AgUiHandler, AgUiOptions, AgUiThreads, InterruptGate, StaticToken};
use everruns::{Agent, Engine, Model};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let gate = InterruptGate::new();
    let agent = Agent::builder()
        .instructions("You are a helpful assistant.")
        .model(Model::simulated("Hello from Everruns."))
        .ask_user(gate.clone())
        .approver(gate.clone())
        .build()?;
    let token = std::env::var("AG_UI_TOKEN").unwrap_or_else(|_| "dev-token".to_string());
    let handler = AgUiHandler::new(
        AgUiThreads::new(Engine::new(), agent),
        StaticToken::bearer(token),
    )
    .options(AgUiOptions::new().gate(gate));
    let router = Router::new().route("/ag-ui", handler.route());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    println!("AG-UI endpoint: http://127.0.0.1:3000/ag-ui");
    axum::serve(listener, router).await?;
    Ok(())
}
