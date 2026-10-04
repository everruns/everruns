//! The A2A channel: `POST /v1/channels/{agent}/a2a` serves an agent to other agents
//! over A2A 1.0 JSON-RPC, with its Agent Card at
//! `GET /v1/channels/{agent}/a2a/.well-known/agent-card.json`. Requires the `a2a`
//! feature.
//!
//! Decisions:
//! - Built on the A2A Rust SDK (`a2a-server-lf`): serve implements only its
//!   `AgentExecutor` over a serve session turn. `DefaultRequestHandler` with
//!   an `InMemoryTaskStore` supplies the protocol: SendMessage (blocking),
//!   SendStreamingMessage, GetTask, ListTasks, CancelTask, SubscribeToTask,
//!   and JSON-RPC framing and errors. Hand-writing those would duplicate
//!   what the everruns server had to audit method by method.
//! - A2A 1.0 only, because that is what the SDK speaks: a request needs the
//!   `A2A-Version: 1.0` header (absent means 0.3, which is refused). The
//!   everruns server also speaks 0.3; serve does not.
//! - The path has the shape of the everruns server's endpoint route
//!   (`/v1/channels/{id}/a2a`, card under it), so a caller moves between serve and
//!   Everruns by base URL and id alone. Here the id is the agent's name:
//!   every top-level agent is an endpoint, as with AG-UI.
//! - One session per (agent, A2A `contextId`), kept in the thread map
//!   channels use (channel key `a2a:{agent}`), so a context survives a
//!   restart. Tasks do not: they live in the SDK's in-memory store, so after
//!   a restart `GetTask` forgets old task ids while the conversation behind
//!   a context continues.
//! - A task is one turn. Its events are `working`, then the final reply as
//!   one `response` artifact (the everruns server's artifact name), then
//!   `completed`; a failed turn ends `failed` with the error as the status
//!   message. Tool calls and intermediate text stay in the session, as on
//!   the server's endpoint.
//! - CancelTask cancels the turn running on the context's session. Two
//!   tasks on one context share that session, so the second steers the
//!   first's turn (the `/v1` messages route behaves the same).
//! - A pending approval or `ask_user` question parks the task in `working`
//!   until it is answered through the `/v1` routes; A2A has no answer for
//!   either here. The card advertises no security scheme: as on every serve
//!   route, auth is what the developer puts in front.
//! - The card's interface URL is absolute, from the request's `Host` (and
//!   `X-Forwarded-Proto`, default `http`, since a serve app usually runs on
//!   plain HTTP behind whatever terminates TLS).

use std::sync::{Arc, Weak};

use ::a2a::{
    A2AError, AgentCapabilities, AgentCard, AgentInterface, AgentSkill, Artifact, Message, Part,
    PartContent, Role, StreamResponse, TaskArtifactUpdateEvent, TaskState, TaskStatus,
    TaskStatusUpdateEvent,
};
use a2a_server::{
    AgentExecutor, DefaultRequestHandler, ExecutorContext, InMemoryTaskStore,
    WELL_KNOWN_AGENT_CARD_PATH, jsonrpc::jsonrpc_router,
};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::stream::BoxStream;
use serde_json::json;
use tokio::sync::mpsc;

use crate::host::{ApiError, Host, NewSession};

/// The route of `agent`'s A2A endpoint. Its card is under it, at
/// `/.well-known/agent-card.json`.
pub(crate) fn route(agent: &str) -> String {
    format!("/v1/channels/{agent}/a2a")
}

/// The thread-map channel key of `agent`'s A2A contexts.
fn thread_channel(agent: &str) -> String {
    format!("a2a:{agent}")
}

/// Mount every top-level agent's endpoint and card on `router`, and answer
/// any other name with a 404 problem.
pub(crate) fn routes(host: &Arc<Host>, mut router: Router<Arc<Host>>) -> Router<Arc<Host>> {
    let agents: Vec<&'static str> = host
        .app
        .inner
        .agents
        .iter()
        .filter(|agent| !agent.sub)
        .map(|agent| agent.name)
        .collect();
    for agent in agents {
        let executor = SessionExecutor {
            host: Arc::downgrade(host),
            agent,
        };
        let handler = DefaultRequestHandler::new(executor, InMemoryTaskStore::default())
            .with_capabilities(capabilities());
        let card = CardSource {
            host: Arc::downgrade(host),
            agent,
        };
        let endpoint = jsonrpc_router(Arc::new(handler))
            .route(WELL_KNOWN_AGENT_CARD_PATH, get(agent_card).with_state(card));
        router = router
            .nest_service(&route(agent), endpoint.clone())
            .nest_service(
                &route(agent).replacen("/v1/channels/", "/v1/e/", 1),
                endpoint,
            );
    }
    // Static agent paths win over these; a subagent or an unknown name lands
    // here.
    router
        .route("/v1/channels/{name}/a2a", post(unknown))
        .route("/v1/e/{name}/a2a", post(unknown))
        .route(
            &format!("/v1/e/{{name}}/a2a{WELL_KNOWN_AGENT_CARD_PATH}"),
            get(unknown),
        )
        .route(
            &format!("/v1/channels/{{name}}/a2a{WELL_KNOWN_AGENT_CARD_PATH}"),
            get(unknown),
        )
}

async fn unknown(Path(name): Path<String>) -> Response {
    crate::server::Failure::from(ApiError::NotFound(format!("agent {name}"))).into_response()
}

fn capabilities() -> AgentCapabilities {
    AgentCapabilities {
        streaming: Some(true),
        push_notifications: Some(false),
        extensions: None,
        extended_agent_card: None,
    }
}

// --- Agent Card ----------------------------------------------------------------

#[derive(Clone)]
struct CardSource {
    host: Weak<Host>,
    agent: &'static str,
}

/// `GET /v1/channels/{agent}/a2a/.well-known/agent-card.json`.
async fn agent_card(State(source): State<CardSource>, headers: HeaderMap) -> Response {
    let Some(host) = source.host.upgrade() else {
        return crate::server::Failure::from(anyhow::anyhow!("host is shutting down"))
            .into_response();
    };
    let url = match headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    {
        Some(authority) => {
            let scheme = headers
                .get("x-forwarded-proto")
                .and_then(|value| value.to_str().ok())
                .unwrap_or("http");
            format!("{scheme}://{authority}{}", route(source.agent))
        }
        None => route(source.agent),
    };
    let card = card(&host, source.agent, url);
    ([(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")], Json(card)).into_response()
}

/// `agent`'s Agent Card, its one JSON-RPC interface at `url`.
fn card(host: &Host, agent: &str, url: String) -> AgentCard {
    let manifest = host.app.manifest();
    let description = manifest
        .agents
        .iter()
        .find(|info| info.name == agent)
        .and_then(|info| info.description.clone())
        .unwrap_or_else(|| format!("The `{agent}` agent of {}", manifest.app.name));
    AgentCard {
        name: agent.to_string(),
        description: description.clone(),
        version: manifest.app.version.clone(),
        supported_interfaces: vec![AgentInterface::new(url, "JSONRPC")],
        capabilities: capabilities(),
        default_input_modes: vec!["text/plain".into()],
        default_output_modes: vec!["text/plain".into()],
        skills: vec![AgentSkill {
            id: agent.to_string(),
            name: agent.to_string(),
            description,
            tags: vec!["serve".into(), "everruns".into()],
            examples: None,
            input_modes: None,
            output_modes: None,
            security_requirements: None,
        }],
        provider: None,
        documentation_url: None,
        icon_url: None,
        security_schemes: None,
        security_requirements: None,
        signatures: None,
    }
}

// --- Executor ------------------------------------------------------------------

type Events = BoxStream<'static, Result<StreamResponse, A2AError>>;

/// Runs each A2A task as one turn of the context's serve session.
struct SessionExecutor {
    /// Weak: the router outlives no host, and must not keep one alive.
    host: Weak<Host>,
    agent: &'static str,
}

impl AgentExecutor for SessionExecutor {
    fn execute(&self, ctx: ExecutorContext) -> Events {
        let (tx, rx) = mpsc::channel(8);
        let host = self.host.clone();
        let agent = self.agent;
        tokio::spawn(async move { run_task(host, agent, ctx, tx).await });
        receive(rx)
    }

    fn cancel(&self, ctx: ExecutorContext) -> Events {
        let host = self.host.clone();
        let channel = thread_channel(self.agent);
        Box::pin(futures::stream::once(async move {
            if let Some(host) = host.upgrade()
                && let Ok(Some(session)) = host.thread_session(&channel, &ctx.context_id)
            {
                // `Ok(false)`: no turn ran; the task is canceled all the same.
                let _ = host.cancel(&session).await;
            }
            Ok(status(&ctx, TaskState::Canceled, None))
        }))
    }
}

fn receive(rx: mpsc::Receiver<Result<StreamResponse, A2AError>>) -> Events {
    Box::pin(futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|event| (event, rx))
    }))
}

/// One task: `working`, the turn, then its reply and `completed`, or
/// `failed`. Input the turn cannot use is a JSON-RPC error instead.
async fn run_task(
    host: Weak<Host>,
    agent: &'static str,
    ctx: ExecutorContext,
    tx: mpsc::Sender<Result<StreamResponse, A2AError>>,
) {
    let Some(input) = ctx.message.as_ref().and_then(message_input) else {
        let _ = tx
            .send(Err(A2AError::invalid_params(
                "the message needs a text or data part",
            )))
            .await;
        return;
    };
    let Some(host) = host.upgrade() else {
        let _ = tx
            .send(Err(A2AError::internal("host is shutting down")))
            .await;
        return;
    };
    if tx
        .send(Ok(status(&ctx, TaskState::Working, None)))
        .await
        .is_err()
    {
        return;
    }
    let outcome = async {
        let session = context_session(&host, agent, &ctx.context_id).await?;
        host.send(&session, input).await?.wait().await
    }
    .await;
    let events = match outcome {
        Ok(turn) if turn.success => vec![
            StreamResponse::ArtifactUpdate(TaskArtifactUpdateEvent {
                task_id: ctx.task_id.clone(),
                context_id: ctx.context_id.clone(),
                artifact: Artifact {
                    artifact_id: ::a2a::new_artifact_id(),
                    name: Some("response".into()),
                    description: None,
                    parts: vec![Part::text(turn.response)],
                    metadata: None,
                    extensions: None,
                },
                append: None,
                last_chunk: Some(true),
                metadata: None,
            }),
            status(&ctx, TaskState::Completed, None),
        ],
        Ok(turn) => {
            let why = turn.error.unwrap_or_else(|| "the turn failed".into());
            vec![status(&ctx, TaskState::Failed, Some(why))]
        }
        Err(err) => vec![status(&ctx, TaskState::Failed, Some(format!("{err:#}")))],
    };
    for event in events {
        if tx.send(Ok(event)).await.is_err() {
            return;
        }
    }
}

/// The session behind `agent`'s A2A context, created on its first task.
async fn context_session(host: &Host, agent: &str, context: &str) -> crate::Result<String> {
    let channel = thread_channel(agent);
    if let Some(session) = host.thread_session(&channel, context)? {
        return Ok(session);
    }
    let session = host
        .create_session(NewSession {
            agent: Some(agent.to_string()),
            metadata: Some(json!({ "channel": "a2a", "agent": agent, "context": context })),
            ..NewSession::default()
        })
        .await?;
    host.bind_thread(&channel, context, &session)?;
    Ok(session)
}

/// The turn input of an A2A message: its text parts, and its data parts as
/// JSON, in order. File parts are not read. `None` when nothing is left.
fn message_input(message: &Message) -> Option<String> {
    let parts: Vec<String> = message
        .parts
        .iter()
        .filter_map(|part| match &part.content {
            PartContent::Text(text) if !text.trim().is_empty() => Some(text.clone()),
            PartContent::Data(value) => Some(value.to_string()),
            _ => None,
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

/// A status update for `ctx`'s task, with `text` as an agent message.
fn status(ctx: &ExecutorContext, state: TaskState, text: Option<String>) -> StreamResponse {
    let message = text.map(|text| {
        let mut message = Message::new(Role::Agent, vec![Part::text(text)]);
        message.context_id = Some(ctx.context_id.clone());
        message.task_id = Some(ctx.task_id.clone());
        message
    });
    StreamResponse::StatusUpdate(TaskStatusUpdateEvent {
        task_id: ctx.task_id.clone(),
        context_id: ctx.context_id.clone(),
        status: TaskStatus {
            state,
            message,
            timestamp: None,
        },
        metadata: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_input_joins_text_and_data_parts() {
        let message = Message::new(
            Role::User,
            vec![
                Part::text("Summarize"),
                Part::text("  "),
                Part::data(json!({ "topic": "tides" })),
            ],
        );
        assert_eq!(
            message_input(&message).unwrap(),
            "Summarize\n\n{\"topic\":\"tides\"}"
        );
        assert!(message_input(&Message::new(Role::User, vec![Part::text(" ")])).is_none());
    }
}
