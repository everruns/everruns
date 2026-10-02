//! # serve-agentcore (experimental)
//!
//! Run a [serve](https://docs.rs/everruns-serve) app on
//! [Amazon Bedrock AgentCore Runtime](https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/runtime-agui-protocol-contract.html),
//! part of the [Everruns](https://everruns.com) ecosystem.
//!
//! > **Experimental.** Like serve, this is a proof of concept with no
//! > compatibility promise.
//!
//! AgentCore Runtime runs one container per session in its own microVM and
//! talks to it over HTTP on port 8080. This crate serves that contract on
//! top of serve's host:
//!
//! - `GET /ping`: `{"status":"Healthy"}`, or `HealthyBusy` while a turn
//!   runs, so AgentCore keeps the session alive for background work.
//! - `POST /invocations`: an AG-UI 1.0 run of the app's agent, streamed as
//!   server-sent events. The body is AG-UI `RunAgentInput` (what the
//!   AgentCore AG-UI protocol forwards), or `{"prompt": "..."}` (what
//!   `InvokeAgentRuntime` callers usually send). The AG-UI thread defaults
//!   to the AgentCore session id from
//!   `X-Amzn-Bedrock-AgentCore-Runtime-Session-Id`.
//! - serve's own `/v1` wire API and `/health`, unchanged, for local use and
//!   `eval --against`.
//!
//! Everything else is serve's: the app, its agents and tools, approvals,
//! `ask_user`, and the durable SQLite session log (point `SERVE_DATA_DIR`
//! at AgentCore session storage so sessions survive microVM restarts).
//!
//! ```no_run
//! use serve::prelude::*;
//!
//! #[agent]
//! fn assistant() -> Agent {
//!     Agent::builder()
//!         .model("anthropic/claude-sonnet-5")
//!         .instructions("Be brief.")
//!         .build()
//! }
//!
//! #[tokio::main]
//! async fn main() -> serve::Result {
//!     // No command (what AgentCore runs) or `agentcore`: the AgentCore
//!     // contract on :8080. `dev`, `start`, `eval`, `manifest`, `deploy`:
//!     // serve's own commands.
//!     serve_agentcore::start(App::builder().discover().build()).await
//! }
//! ```

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::ffi::OsString;
use std::net::SocketAddr;

use anyhow::anyhow;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use serde_json::{Value, json};
use serve::{App, Mode, Server};

/// The header AgentCore Runtime sets on every request, naming the session
/// (and so the microVM) it routed to.
pub const SESSION_HEADER: &str = "x-amzn-bedrock-agentcore-runtime-session-id";

/// The port the AgentCore HTTP and AG-UI protocol contracts require.
pub const PORT: u16 = 8080;

/// The `agentcore` command line.
#[derive(Parser, Debug)]
#[command(
    about = "Serve this app on the Amazon Bedrock AgentCore Runtime contract (experimental)."
)]
struct Cli {
    /// Port to listen on. AgentCore requires 8080.
    #[arg(long, env = "PORT", default_value_t = PORT)]
    port: u16,
    /// The agent `/invocations` runs. Defaults to the app's default agent.
    #[arg(long, env = "SERVE_AGENTCORE_AGENT")]
    agent: Option<String>,
    /// Run in serve's `dev` mode (simulator fallback, no secret check)
    /// instead of `start`, to try the contract locally.
    #[arg(long)]
    dev: bool,
}

/// Run the app: with no command, or with `agentcore`, serve the AgentCore
/// Runtime contract; any other command is serve's (see [`serve::start`]).
pub async fn start(app: App) -> serve::Result {
    let mut args: Vec<OsString> = std::env::args_os().collect();
    match args.get(1).and_then(|arg| arg.to_str()) {
        None => {}
        Some("agentcore") => {
            args.remove(1);
        }
        Some(_) => return serve::start(app).await,
    }
    let cli = Cli::parse_from(args);
    let mode = if cli.dev { Mode::Dev } else { Mode::Start };
    let server = Server::new(app, mode, Some(serve::data_dir()?))?;
    let agent = agent(&server, cli.agent)?;

    let addr = SocketAddr::from(([0, 0, 0, 0], cli.port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!(
        "serve-agentcore · {} · build {} · agent {agent} · {mode:?} · listening on {addr}",
        server.app().name(),
        server.build_id(),
    );
    server.spawn_schedules();
    axum::serve(listener, router(&server, agent))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

/// The agent `/invocations` runs: `requested` when it names a top-level
/// agent, else the app's default agent.
fn agent(server: &Server, requested: Option<String>) -> serve::Result<String> {
    match requested {
        Some(name) => {
            let known = server
                .app()
                .manifest()
                .agents
                .iter()
                .any(|agent| agent.name == name && !agent.sub);
            if known {
                Ok(name)
            } else {
                Err(anyhow!(
                    "SERVE_AGENTCORE_AGENT `{name}` is not a top-level agent of this app"
                ))
            }
        }
        None => server.default_agent().ok_or_else(|| {
            anyhow!(
                "this app has several agents and none is the default; set SERVE_AGENTCORE_AGENT"
            )
        }),
    }
}

#[derive(Clone)]
struct Target {
    server: Server,
    agent: String,
}

/// The AgentCore contract (`/ping`, `/invocations`) for `agent`, merged with
/// serve's `/v1` wire API.
pub fn router(server: &Server, agent: String) -> Router {
    let target = Target {
        server: server.clone(),
        agent,
    };
    let contract = Router::new()
        .route("/ping", get(ping))
        .route("/invocations", post(invocations))
        .with_state(target);
    server.router().merge(contract)
}

/// `GET /ping`. No `time_of_last_update`: AgentCore tracks status changes
/// itself, and a timestamp that moves on every ping would keep an idle
/// session alive until its maximum lifetime.
async fn ping(State(target): State<Target>) -> Json<Value> {
    let status = if target.server.busy() {
        "HealthyBusy"
    } else {
        "Healthy"
    };
    Json(json!({ "status": status }))
}

/// `POST /invocations`: one AG-UI run of the target agent.
async fn invocations(State(target): State<Target>, headers: HeaderMap, body: Bytes) -> Response {
    let session = headers
        .get(SESSION_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty());
    match run_agent_input(&body, session) {
        Ok(input) => target.server.ag_ui(&target.agent, &input).await,
        Err(why) => problem(StatusCode::BAD_REQUEST, why),
    }
}

/// The AG-UI `RunAgentInput` body for an invocation.
///
/// A body with `messages` is already `RunAgentInput` and passes through,
/// with `threadId` (and `runId`) filled in when absent. A `{"prompt": ...}`
/// body becomes a one-message run. The thread is the body's `threadId`, else
/// the AgentCore session, so one AgentCore session is one serve session.
fn run_agent_input(body: &[u8], session: Option<&str>) -> Result<Vec<u8>, String> {
    let mut input: Value =
        serde_json::from_slice(body).map_err(|err| format!("invalid JSON body: {err}"))?;
    let Some(fields) = input.as_object_mut() else {
        return Err("the body must be a JSON object".into());
    };
    if !fields.contains_key("messages") {
        let Some(prompt) = fields.remove("prompt") else {
            return Err(
                "expected an AG-UI RunAgentInput (with `messages`) or `{\"prompt\": \"...\"}`"
                    .into(),
            );
        };
        let Some(prompt) = prompt.as_str().map(str::to_string) else {
            return Err("`prompt` must be a string".into());
        };
        fields.insert(
            "messages".into(),
            json!([{ "id": new_id(), "role": "user", "content": prompt }]),
        );
        for key in ["tools", "context"] {
            fields.entry(key).or_insert_with(|| json!([]));
        }
        for key in ["state", "forwardedProps"] {
            fields.entry(key).or_insert_with(|| json!({}));
        }
    }
    let has_thread = fields
        .get("threadId")
        .and_then(Value::as_str)
        .is_some_and(|thread| !thread.trim().is_empty());
    if !has_thread {
        let Some(session) = session else {
            return Err(format!(
                "no `threadId` in the body and no `{SESSION_HEADER}` header"
            ));
        };
        fields.insert("threadId".into(), json!(session));
    }
    fields.entry("runId").or_insert_with(|| json!(new_id()));
    serde_json::to_vec(&input).map_err(|err| err.to_string())
}

fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

/// RFC 9457 problem details, as serve's own routes answer.
fn problem(status: StatusCode, detail: String) -> Response {
    let body = json!({
        "title": status.canonical_reason().unwrap_or("Error"),
        "status": status.as_u16(),
        "detail": detail,
    });
    (
        status,
        [(header::CONTENT_TYPE, "application/problem+json")],
        body.to_string(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(bytes: Vec<u8>) -> Value {
        serde_json::from_slice(&bytes).unwrap()
    }

    #[test]
    fn prompt_becomes_a_one_message_run_on_the_agentcore_session() {
        let input = parse(run_agent_input(br#"{"prompt":"hi"}"#, Some("s-1")).unwrap());
        assert_eq!(input["threadId"], "s-1");
        assert_eq!(input["messages"][0]["role"], "user");
        assert_eq!(input["messages"][0]["content"], "hi");
        assert!(input["runId"].as_str().is_some_and(|id| !id.is_empty()));
        assert_eq!(input["tools"], json!([]));
        assert_eq!(input["forwardedProps"], json!({}));
        assert!(input.get("prompt").is_none());
    }

    #[test]
    fn run_agent_input_passes_through_and_keeps_its_thread() {
        let body = json!({
            "threadId": "t-1", "runId": "r-1",
            "messages": [{ "id": "m", "role": "user", "content": "x" }],
            "tools": [], "context": [], "state": {}, "forwardedProps": {}
        });
        let input = parse(run_agent_input(body.to_string().as_bytes(), Some("s-1")).unwrap());
        assert_eq!(input, body);
    }

    #[test]
    fn missing_thread_falls_back_to_the_session_header() {
        let body = br#"{"messages":[{"id":"m","role":"user","content":"x"}],"threadId":" "}"#;
        let input = parse(run_agent_input(body, Some("s-2")).unwrap());
        assert_eq!(input["threadId"], "s-2");
    }

    #[test]
    fn rejects_bodies_it_cannot_run() {
        let no_thread = run_agent_input(br#"{"prompt":"hi"}"#, None).unwrap_err();
        assert!(no_thread.contains(SESSION_HEADER), "{no_thread}");
        assert!(run_agent_input(b"not json", Some("s")).is_err());
        assert!(run_agent_input(b"[]", Some("s")).is_err());
        assert!(run_agent_input(br#"{"prompt":7}"#, Some("s")).is_err());
        assert!(run_agent_input(br#"{"input":"hi"}"#, Some("s")).is_err());
    }
}
