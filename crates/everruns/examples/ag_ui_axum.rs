//! Serve an agent to AG-UI clients (CopilotKit, `@ag-ui/client`) from axum.
//!
//! Run with:
//!
//! ```text
//! cargo run -p everruns --features ag-ui --example ag_ui_axum
//! curl -N localhost:3000/ag-ui -H 'content-type: application/json' -d '{
//!   "threadId": "t1", "runId": "r1", "protocolVersion": "1.0",
//!   "messages": [{ "id": "m1", "role": "user", "content": "Hi" }]
//! }'
//! ```
//!
//! One session per AG-UI thread, resolved by `AgUiThreads`; one
//! `InterruptGate` answers every session's `ask_user` questions and tool
//! approvals through AG-UI interrupts. The simulated model keeps the example
//! offline; swap in a real provider. With the `local` feature, pass
//! `SqliteThreadStore::local(&config)` and an agent on that `LocalConfig` to
//! keep threads across restarts.

use std::convert::Infallible;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use everruns::ag_ui::{AgUiError, AgUiOptions, AgUiThreads, InterruptGate, RunAgentInput};
use everruns::{Agent, Engine, Model};
use futures::StreamExt;

#[derive(Clone)]
struct App {
    threads: AgUiThreads,
    gate: InterruptGate,
}

async fn ag_ui(State(app): State<App>, Json(input): Json<RunAgentInput>) -> Response {
    let options = AgUiOptions::new().gate(app.gate.clone());
    match app.threads.run(input, options).await {
        Ok(run) => {
            let events = run.map(|event| SseEvent::default().json_data(&event));
            let events = events.map(|event| Ok::<_, Infallible>(event.unwrap_or_default()));
            Sse::new(events)
                .keep_alive(KeepAlive::default())
                .into_response()
        }
        Err(AgUiError::InvalidInput(why)) => (StatusCode::BAD_REQUEST, why).into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let gate = InterruptGate::new();
    let agent = Agent::builder()
        .instructions("You are a helpful assistant.")
        .model(Model::simulated("Hello from Everruns."))
        .ask_user(gate.clone())
        .approver(gate.clone())
        .build()?;
    let app = App {
        threads: AgUiThreads::new(Engine::new(), agent),
        gate,
    };
    let router = Router::new().route("/ag-ui", post(ag_ui)).with_state(app);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    println!("AG-UI endpoint: http://127.0.0.1:3000/ag-ui");
    axum::serve(listener, router).await?;
    Ok(())
}
