//! The voice channel: every top-level agent takes browser calls at
//! `POST /v1/channels/{agent}/voice/calls` and speaks its answers. Requires the
//! `voice` feature.
//!
//! Decisions:
//! - Built in, like AG-UI, not a [`Channel`](crate::Channel): a call is a
//!   live connection, not a webhook answered later. The call itself is
//!   `everruns::voice::VoiceChannel`, so it runs the same voice loop as the
//!   Framework and the everruns server.
//! - Voice is a channel, not part of the agent: every top-level agent gets an
//!   endpoint, and one session can take typed messages and calls. The
//!   optional `[voice]` section of `serve.toml` sets how calls sound for the
//!   whole app.
//! - The speech provider is OpenAI with `OPENAI_API_KEY`. Without a key, `dev`
//!   and `eval` fall back to the offline simulator, as models do; `start`
//!   refuses the call. `model = "sim"` forces the simulator.
//! - Browser WebRTC with server-side SDP exchange only. The provider key stays
//!   here and audio never passes through this process.
//! - Running calls live in this process, by provider call id. The end route
//!   finds a call only on the replica that placed it.
//! - Calls are unauthenticated like every serve route; a public deployment
//!   puts the app behind its own auth.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use axum::Json;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{Html, IntoResponse, Response};
use everruns::providers::openai::OpenAI;
use everruns::voice::{Realtime, VoiceCall, VoiceChannel, VoiceChannelConfig, VoiceError};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::host::{ApiError, Host, NewSession};

/// The base route of `agent`'s voice channel.
pub(crate) fn route(agent: &str) -> String {
    format!("/v1/channels/{agent}/voice")
}

/// The browser test page, served at the channel's base route.
const PAGE: &str = include_str!("voice.html");

/// `POST /v1/channels/{agent}/voice/calls`.
#[derive(Deserialize)]
pub(crate) struct CallBody {
    /// The browser's WebRTC SDP offer, with a microphone track.
    sdp: String,
    /// Talk on an existing session of the same agent; a new one otherwise.
    #[serde(default)]
    session_id: Option<String>,
}

struct Running {
    agent: String,
    call: VoiceCall,
}

/// Calls in progress, by provider call id. Finished calls are swept when the
/// next call starts.
fn calls() -> &'static Mutex<HashMap<String, Running>> {
    static CALLS: OnceLock<Mutex<HashMap<String, Running>>> = OnceLock::new();
    CALLS.get_or_init(Default::default)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The app's call settings: `[voice]` in `serve.toml`, else the defaults.
pub(crate) fn config(host: &Host) -> VoiceChannelConfig {
    host.app.inner.config.voice.clone().unwrap_or_default()
}

/// Reject a `[voice]` section a call would refuse, before serving anything.
pub(crate) fn check(host: &Host) -> crate::Result<()> {
    config(host)
        .validate()
        .map_err(|why| anyhow::anyhow!("serve.toml [voice]: {why}"))
}

/// Where a call's speech goes.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Speech {
    Simulated,
    OpenAI(String),
}

/// Pick the speech provider: `sim` models are simulated, an OpenAI key wins
/// otherwise, and without one only an offline-capable mode simulates.
pub(crate) fn speech(model: &str, openai_key: Option<&str>, allow_offline: bool) -> Option<Speech> {
    if model == "sim" || model.starts_with("sim/") {
        return Some(Speech::Simulated);
    }
    match openai_key {
        Some(key) => Some(Speech::OpenAI(key.to_string())),
        None => allow_offline.then_some(Speech::Simulated),
    }
}

fn realtime(host: &Host, config: &VoiceChannelConfig) -> crate::Result<Realtime> {
    match speech(
        &config.model,
        host.gateway.openai_key.as_deref(),
        host.mode.allow_offline(),
    ) {
        Some(Speech::Simulated) => Ok(Realtime::simulated()),
        Some(Speech::OpenAI(key)) => Ok(OpenAI::new(key).realtime()),
        None => Err(ApiError::BadRequest(
            "voice calls need a speech provider: set OPENAI_API_KEY".into(),
        )
        .into()),
    }
}

fn top_level(host: &Host, agent: &str) -> crate::Result<()> {
    if host.app.agent(agent).is_none_or(|entry| entry.sub) {
        return Err(ApiError::NotFound(format!("agent {agent}")).into());
    }
    Ok(())
}

/// `GET /v1/channels/{agent}/voice`: a page that calls the agent from the
/// browser, for trying a voice channel in `dev`.
pub(crate) async fn page(State(host): State<Arc<Host>>, Path(agent): Path<String>) -> Response {
    match top_level(&host, &agent) {
        Ok(()) => (
            [(header::CACHE_CONTROL, "no-store")],
            Html(PAGE.replace("{{agent}}", &escape(&agent))),
        )
            .into_response(),
        Err(err) => crate::server::Failure::from(err).into_response(),
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// `POST /v1/channels/{agent}/voice/calls`: place a call. Answers the session id,
/// the call id and the SDP answer for the browser.
pub(crate) async fn call(
    State(host): State<Arc<Host>>,
    Path(agent): Path<String>,
    Json(body): Json<CallBody>,
) -> Response {
    match place(&host, &agent, body).await {
        Ok(value) => Json(value).into_response(),
        Err(err) => crate::server::Failure::from(err).into_response(),
    }
}

async fn place(host: &Host, agent: &str, body: CallBody) -> crate::Result<Value> {
    top_level(host, agent)?;
    if body.sdp.trim().is_empty() {
        return Err(ApiError::BadRequest("sdp is required".into()).into());
    }
    let config = config(host);
    let realtime = realtime(host, &config)?;
    let session_id = match body.session_id {
        Some(id) => {
            let row = host.session_row(&id)?;
            if row.agent != agent {
                return Err(ApiError::BadRequest(format!(
                    "session {id} belongs to agent {}",
                    row.agent
                ))
                .into());
            }
            id
        }
        None => {
            host.create_session(NewSession {
                agent: Some(agent.to_string()),
                tags: vec!["voice".into()],
                metadata: Some(json!({ "channel": "voice", "agent": agent })),
                ..NewSession::default()
            })
            .await?
        }
    };
    let session = host.session(&session_id).await?;
    let channel = VoiceChannel::with_config(realtime, config.clone());
    let call = channel
        .accept_webrtc(&session, &body.sdp)
        .await
        .map_err(|err| match err {
            VoiceError::Invalid(why) => anyhow::Error::from(ApiError::BadRequest(why)),
            other => anyhow::Error::from(other),
        })?;
    let response = json!({
        "session_id": session_id,
        "call_id": call.call_id(),
        "model": config.model,
        "voice": config.voice,
        "answer_sdp": call.answer_sdp(),
    });
    let mut calls = lock(calls());
    calls.retain(|_, running| !running.call.is_finished());
    calls.insert(
        call.call_id().to_string(),
        Running {
            agent: agent.to_string(),
            call,
        },
    );
    Ok(response)
}

/// `POST /v1/channels/{agent}/voice/calls/{call_id}/end`: hang up. Answers what
/// the call did.
pub(crate) async fn end(Path((agent, call_id)): Path<(String, String)>) -> Response {
    let running = {
        let mut calls = lock(calls());
        match calls.get(&call_id) {
            Some(running) if running.agent == agent => calls.remove(&call_id),
            _ => None,
        }
    };
    let Some(running) = running else {
        return crate::server::Failure::from(ApiError::NotFound(format!("call {call_id}")))
            .into_response();
    };
    match running.call.end().await {
        Ok(summary) => Json(json!({
            "call_id": call_id,
            "utterances": summary.utterances,
            "spoken_chunks": summary.spoken_chunks,
            "interruptions": summary.interruptions,
        }))
        .into_response(),
        Err(err) => crate::server::Failure::from(err).into_response(),
    }
}

/// Settings the agent card shows for the voice channel.
pub(crate) fn card(host: &Host) -> Value {
    let config = config(host);
    let endpoints: serde_json::Map<String, Value> = host
        .app
        .inner
        .agents
        .iter()
        .filter(|agent| !agent.sub)
        .map(|agent| (agent.name.clone(), json!(route(&agent.name))))
        .collect();
    json!({
        "mode": config.mode,
        "model": config.model,
        "voice": config.voice,
        "endpoints": endpoints,
    })
}
