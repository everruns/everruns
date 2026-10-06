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
//! - `GET /ws`: the same runs over AgentCore's WebSocket transport. Each text
//!   message is one invocation body; the run's AG-UI events come back as one
//!   text message each, so a connection can carry several runs in turn.
//! - serve's own `/v1` wire API and `/health`, unchanged, for local use and
//!   `eval --against`.
//!
//! Everything else is serve's: the app, its agents and tools, approvals,
//! `ask_user`, and the durable SQLite session log, kept on AgentCore session
//! storage (`/mnt/workspace`) when the runtime mounts it, so sessions survive
//! the microVM stopping.
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

use everruns::sqlite as rusqlite;
use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::anyhow;
use axum::body::{Body, Bytes};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use serde_json::{Value, json};
use serve::{App, Mode, Server};
use tokio::sync::OnceCell;
use tower::ServiceExt;

/// The header AgentCore Runtime sets on every request, naming the session
/// (and so the microVM) it routed to.
pub const SESSION_HEADER: &str = "x-amzn-bedrock-agentcore-runtime-session-id";

/// The port the AgentCore HTTP and AG-UI protocol contracts require.
pub const PORT: u16 = 8080;

/// Where serve-agentcore looks for AgentCore session storage when
/// `SERVE_DATA_DIR` is not set: the mount path the AgentCore docs use.
pub const SESSION_STORAGE: &str = "/mnt/workspace";

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

/// How [`router`] boots the app.
#[derive(Clone, Debug)]
pub struct Options {
    /// serve's mode: [`Mode::Start`] in production.
    pub mode: Mode,
    /// The agent `/invocations` runs; the app's default agent when `None`.
    pub agent: Option<String>,
    /// Where serve keeps its session log. `None` resolves on the first
    /// request (see [`Storage::resolve`]).
    pub data_dir: Option<PathBuf>,
    /// The root of the `[sandbox] kind = "microvm"` workspace. `None`
    /// resolves with the data dir.
    pub workspace: Option<PathBuf>,
}

impl Options {
    /// Production defaults: `start` mode, everything else resolved.
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            agent: None,
            data_dir: None,
            workspace: None,
        }
    }
}

/// Where a booted app keeps its state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Storage {
    /// serve's SQLite session log.
    pub data_dir: PathBuf,
    /// The microVM workspace: the shell's working directory and the root
    /// of the file tools.
    pub workspace: PathBuf,
    /// Whether this survives the microVM stopping.
    pub persistent: bool,
}

impl Storage {
    /// Resolve where state lives, in order:
    ///
    /// 1. `SERVE_DATA_DIR` or `DATABASE_URL` (serve's own settings), with the
    ///    workspace in `SERVE_WORKSPACE` or beside it. Taken as persistent:
    ///    the operator chose it.
    /// 2. AgentCore session storage mounted at [`SESSION_STORAGE`]: the log
    ///    in `/mnt/workspace/.serve`, the workspace at `/mnt/workspace`.
    /// 3. Otherwise a temporary directory, lost when the microVM stops.
    ///
    /// Resolved on the first request, not at boot: AgentCore mounts session
    /// storage only once the session is invoked.
    pub fn resolve(options: &Options) -> serve::Result<Self> {
        let workspace_env = std::env::var_os("SERVE_WORKSPACE").map(PathBuf::from);
        let explicit = options.data_dir.clone().map(Ok).or_else(|| {
            (env_set("SERVE_DATA_DIR") || env_set("DATABASE_URL")).then(serve::data_dir)
        });
        if let Some(data_dir) = explicit {
            let data_dir = data_dir?;
            let workspace = options
                .workspace
                .clone()
                .or(workspace_env)
                .unwrap_or_else(|| data_dir.join("workspace"));
            return Ok(Self {
                data_dir,
                workspace,
                persistent: true,
            });
        }
        let mount = Path::new(SESSION_STORAGE);
        if mount.is_dir() {
            return Ok(Self {
                data_dir: mount.join(".serve"),
                workspace: options
                    .workspace
                    .clone()
                    .or(workspace_env)
                    .unwrap_or_else(|| mount.to_path_buf()),
                persistent: true,
            });
        }
        let scratch = std::env::temp_dir().join("serve-agentcore");
        Ok(Self {
            data_dir: scratch.join(".serve"),
            workspace: options
                .workspace
                .clone()
                .or(workspace_env)
                .unwrap_or_else(|| scratch.join("workspace")),
            persistent: false,
        })
    }
}

fn env_set(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| !value.is_empty())
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
    let mut options = Options::new(if cli.dev { Mode::Dev } else { Mode::Start });
    options.agent = cli.agent;
    let name = app.name().to_string();
    sqlite_for_session_storage();
    let (router, agent) = router(app, options.clone())?;

    let addr = SocketAddr::from(([0, 0, 0, 0], cli.port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!(
        "serve-agentcore · {name} · agent {agent} · {:?} · listening on {addr}",
        options.mode
    );
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

/// Make SQLite usable on AgentCore session storage, for this process.
///
/// Decision: session storage is an NFSv4 mount whose POSIX byte-range locks
/// fail, so SQLite's default `unix` VFS (and `unix-excl`) answers every open
/// with "database is locked". `unix-dotfile` locks with a `.lock` file
/// beside the database instead, which the mount supports (tried on a live
/// runtime: `unix` and `unix-excl` fail, `unix-dotfile` and `unix-none`
/// work). It cannot share a WAL index, so databases stay in rollback-journal
/// mode: slower writes, the same durability. Only the binary entry calls
/// this, since the default VFS is process-wide.
fn sqlite_for_session_storage() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        DOTFILE_LOCKS.store(true, std::sync::atomic::Ordering::Relaxed);
        rusqlite::use_dotfile_locks();
    });
}

/// Whether this process locks SQLite with dot files (see
/// [`sqlite_for_session_storage`]).
static DOTFILE_LOCKS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Remove dot-file locks a stopped microVM left behind. A process killed
/// mid-write never deletes its `<db>.lock`, and the next boot would wait on
/// it forever. Safe at boot: no other process uses this session's storage.
fn clear_stale_locks(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let stale = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".db.lock"));
        if stale {
            eprintln!("serve-agentcore: removing stale lock {}", path.display());
            let _ = std::fs::remove_dir_all(&path).or_else(|_| std::fs::remove_file(&path));
        } else if path.is_dir() && entry.file_name() == "everruns" {
            clear_stale_locks(&path);
        }
    }
}

/// The AgentCore contract around `app`: `/ping`, `/invocations`, and serve's
/// `/v1` wire API. Returns the router and the agent `/invocations` runs.
///
/// The app is checked at once (secrets in `start`, every agent resolved), but
/// the server that keeps state boots on the first request other than
/// `/ping`, once AgentCore has mounted session storage.
pub fn router(app: App, options: Options) -> serve::Result<(Router, String)> {
    // A throwaway in-memory boot: fails fast on a bad app, names the agent.
    let probe = boot(app.clone(), &options, None)?;
    let agent = agent(&probe, options.agent.clone())?;
    drop(probe);
    let target = Target {
        app,
        options: Arc::new(options),
        agent: agent.clone(),
        booted: Arc::new(OnceCell::new()),
    };
    let router = Router::new()
        .route("/ping", get(ping))
        .route("/invocations", post(invocations))
        .route("/ws", get(ws))
        .fallback(forward)
        .with_state(target);
    Ok((router, agent))
}

/// Boot serve with the microVM adapter. `storage: None` boots in memory.
fn boot(app: App, options: &Options, storage: Option<&Storage>) -> serve::Result<Server> {
    let workspace = storage
        .map(|storage| storage.workspace.clone())
        .or_else(|| options.workspace.clone())
        .unwrap_or_else(|| PathBuf::from(SESSION_STORAGE));
    let mut builder =
        Server::builder(app, options.mode).microvm(move |agent| microvm(agent, &workspace));
    if let Some(storage) = storage {
        builder = builder.data_dir(storage.data_dir.clone());
    }
    builder.build()
}

/// `[sandbox] kind = "microvm"` on AgentCore: a real shell and file tools
/// over the session's workspace.
///
/// Decision: no kernel containment inside the microVM. AgentCore gives every
/// session its own microVM and sanitizes it afterwards, so the VM is the
/// boundary, the same one Harness and the AgentCore Code Interpreter rely
/// on. A second, in-VM policy would only stop the agent installing packages
/// in its own sandbox.
fn microvm(agent: everruns::AgentBuilder, workspace: &Path) -> everruns::AgentBuilder {
    agent
        .workspace(workspace)
        .workspace_policy(everruns::WorkspacePolicy::read_write())
        .capability(everruns::HostShell::new().containment(everruns::ContainmentMode::FullAccess))
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
    app: App,
    options: Arc<Options>,
    agent: String,
    booted: Arc<OnceCell<Booted>>,
}

struct Booted {
    server: Server,
    wire: Router,
}

impl Target {
    /// The stateful server, booted on first use.
    async fn booted(&self) -> Result<&Booted, Response> {
        self.booted
            .get_or_try_init(|| async {
                let storage = Storage::resolve(&self.options)?;
                if !storage.persistent {
                    eprintln!(
                        "serve-agentcore: no session storage at {SESSION_STORAGE} and no SERVE_DATA_DIR; \
                         sessions live in {} and are lost when the microVM stops",
                        storage.data_dir.display()
                    );
                }
                std::fs::create_dir_all(&storage.data_dir)?;
                if DOTFILE_LOCKS.load(std::sync::atomic::Ordering::Relaxed) {
                    clear_stale_locks(&storage.data_dir);
                }
                let server = boot(self.app.clone(), &self.options, Some(&storage))?;
                server.spawn_schedules();
                let wire = server.router();
                Ok::<_, serve::Error>(Booted { server, wire })
            })
            .await
            .map_err(|err| {
                // The platform shows callers only "error (500) from runtime";
                // the cause has to reach the runtime's CloudWatch logs.
                eprintln!("serve-agentcore: boot failed: {err:#}");
                problem(StatusCode::INTERNAL_SERVER_ERROR, format!("{err:#}"))
            })
    }
}

/// `GET /ping`. No `time_of_last_update`: AgentCore tracks status changes
/// itself, and a timestamp that moves on every ping would keep an idle
/// session alive until its maximum lifetime. Never boots the server.
async fn ping(State(target): State<Target>) -> Json<Value> {
    let busy = target
        .booted
        .get()
        .is_some_and(|booted| booted.server.busy());
    Json(json!({ "status": if busy { "HealthyBusy" } else { "Healthy" } }))
}

/// `POST /invocations`: one AG-UI run of the target agent.
async fn invocations(State(target): State<Target>, headers: HeaderMap, body: Bytes) -> Response {
    let session = headers
        .get(SESSION_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty());
    let input = match run_agent_input(&body, session) {
        Ok(input) => input,
        Err(why) => return problem(StatusCode::BAD_REQUEST, why),
    };
    match target.booted().await {
        Ok(booted) => logged(booted.server.ag_ui(&target.agent, &input).await),
        Err(response) => response,
    }
}

/// Log a run that never started: AgentCore hides the response from callers.
fn logged(response: Response) -> Response {
    if !response.status().is_success() {
        eprintln!("serve-agentcore: invocation answered {}", response.status());
    }
    response
}

/// `GET /ws`: AgentCore's WebSocket transport for the same runs.
///
/// Decision: a text message is exactly an `/invocations` body, and the reply
/// is that run's AG-UI events, one JSON event per text message (the SSE
/// payloads without their framing). Runs on one connection are sequential,
/// as on an AG-UI client. A body that cannot run gets a single `RUN_ERROR`
/// instead of closing the socket, so the client can send the next one.
async fn ws(
    State(target): State<Target>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    let session = headers
        .get(SESSION_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    upgrade.on_upgrade(move |socket| converse(target, session, socket))
}

async fn converse(target: Target, session: Option<String>, mut socket: WebSocket) {
    while let Some(Ok(message)) = socket.recv().await {
        let body = match message {
            Message::Text(text) => Bytes::from(text.as_str().to_owned()),
            Message::Binary(bytes) => bytes,
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) => continue,
        };
        let response = match run_agent_input(&body, session.as_deref()) {
            Ok(input) => match target.booted().await {
                Ok(booted) => logged(booted.server.ag_ui(&target.agent, &input).await),
                Err(response) => response,
            },
            Err(why) => problem(StatusCode::BAD_REQUEST, why),
        };
        if relay(response, &mut socket).await.is_err() {
            break;
        }
    }
}

/// Send one run's AG-UI events over the socket. A problem response (the run
/// never started) becomes one `RUN_ERROR`.
async fn relay(response: Response, socket: &mut WebSocket) -> Result<(), axum::Error> {
    use futures_util::StreamExt;

    if !response.status().is_success() {
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap_or_default();
        let detail = serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|problem| problem["detail"].as_str().map(str::to_string))
            .unwrap_or_else(|| status.to_string());
        let error =
            json!({ "type": "RUN_ERROR", "message": detail, "code": status.as_u16().to_string() });
        return socket.send(Message::Text(error.to_string().into())).await;
    }
    let mut stream = response.into_body().into_data_stream();
    let mut pending = String::new();
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else { break };
        pending.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(end) = pending.find("\n\n") {
            let frame: String = pending.drain(..end + 2).collect();
            if let Some(data) = sse_data(&frame) {
                socket.send(Message::Text(data.into())).await?;
            }
        }
    }
    if let Some(data) = sse_data(&pending) {
        socket.send(Message::Text(data.into())).await?;
    }
    Ok(())
}

/// The `data:` payload of an SSE frame, joined across lines.
fn sse_data(frame: &str) -> Option<String> {
    let lines: Vec<&str> = frame
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(|data| data.strip_prefix(' ').unwrap_or(data))
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Everything else: serve's `/v1` wire API and `/health`.
async fn forward(State(target): State<Target>, request: Request<Body>) -> Response {
    match target.booted().await {
        Ok(booted) => match booted.wire.clone().oneshot(request).await {
            Ok(response) => response,
            Err(never) => match never {},
        },
        Err(response) => response,
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
