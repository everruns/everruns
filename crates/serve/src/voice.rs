//! The voice channel: every top-level agent takes calls and speaks its
//! answers. Requires the `voice` feature.
//!
//! - Delegated mode (the default): browser WebRTC calls at
//!   `POST /v1/channels/{agent}/voice/calls`.
//! - Cascaded mode: a WebSocket at `GET /v1/channels/{agent}/voice/ws`
//!   carrying 16-bit PCM, mono, 24 kHz, both ways, plus JSON control messages
//!   (`call`, `clear`, `transcript`, `ended`).
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
//! - The provider key always stays here. Delegated calls use server-side SDP
//!   exchange, so audio never passes through this process; cascaded calls
//!   relay audio over the WebSocket, which is also the transport native apps
//!   can use.
//! - `[voice] mode` picks one transport for the whole app; the other one
//!   answers 400 naming the right route.
//! - Running WebRTC calls live in this process, by provider call id. The end
//!   route finds a call only on the replica that placed it. A WebSocket call
//!   ends when its socket closes.
//! - Calls are unauthenticated like every serve route; a public deployment
//!   puts the app behind its own auth.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use axum::Json;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::{Html, IntoResponse, Response};
use everruns::providers::openai::OpenAI;
use everruns::voice::{
    AUDIO_SAMPLE_RATE, AudioOut, CallAudio, Realtime, SpeechProvider, VoiceCall, VoiceCallEvent,
    VoiceChannel, VoiceChannelConfig, VoiceError, VoiceMode,
};
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
/// `model` is the one the channel's mode speaks with: the speech-to-speech
/// model when delegated, the text-to-speech model when cascaded.
pub(crate) fn speech(model: &str, openai_key: Option<&str>, allow_offline: bool) -> Option<Speech> {
    if model == "sim" || model.starts_with("sim/") {
        return Some(Speech::Simulated);
    }
    match openai_key {
        Some(key) => Some(Speech::OpenAI(key.to_string())),
        None => allow_offline.then_some(Speech::Simulated),
    }
}

fn provider(host: &Host, config: &VoiceChannelConfig) -> crate::Result<SpeechProvider> {
    let model = match config.mode {
        VoiceMode::Cascaded => config.speech_model(),
        _ => config.model.as_str(),
    };
    let cascaded = config.mode == VoiceMode::Cascaded;
    match speech(
        model,
        host.gateway.openai_key.as_deref(),
        host.mode.allow_offline(),
    ) {
        Some(Speech::Simulated) if cascaded => Ok(everruns::voice::Speech::simulated().into()),
        Some(Speech::Simulated) => Ok(Realtime::simulated().into()),
        Some(Speech::OpenAI(key)) if cascaded => Ok(OpenAI::new(key).speech().into()),
        Some(Speech::OpenAI(key)) => Ok(OpenAI::new(key).realtime().into()),
        None => Err(ApiError::BadRequest(
            "voice calls need a speech provider: set OPENAI_API_KEY".into(),
        )
        .into()),
    }
}

fn voice_error(err: VoiceError) -> anyhow::Error {
    match err {
        VoiceError::Invalid(why) => anyhow::Error::from(ApiError::BadRequest(why)),
        other => anyhow::Error::from(other),
    }
}

/// The session a call talks on: `session_id` when it belongs to `agent`,
/// else a new session tagged `voice`.
async fn call_session(
    host: &Host,
    agent: &str,
    session_id: Option<String>,
) -> crate::Result<String> {
    match session_id {
        Some(id) => {
            let row = host.session_row(&id)?;
            if row.agent != agent {
                return Err(ApiError::BadRequest(format!(
                    "session {id} belongs to agent {}",
                    row.agent
                ))
                .into());
            }
            Ok(id)
        }
        None => {
            host.create_session(NewSession {
                agent: Some(agent.to_string()),
                tags: vec!["voice".into()],
                metadata: Some(json!({ "channel": "voice", "agent": agent })),
                ..NewSession::default()
            })
            .await
        }
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
            Html(
                PAGE.replace("{{agent}}", &escape(&agent))
                    .replace("{{mode}}", mode_name(config(&host).mode)),
            ),
        )
            .into_response(),
        Err(err) => crate::server::Failure::from(err).into_response(),
    }
}

fn mode_name(mode: VoiceMode) -> &'static str {
    match mode {
        VoiceMode::Cascaded => "cascaded",
        _ => "delegated",
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
    if config.mode == VoiceMode::Cascaded {
        return Err(ApiError::BadRequest(format!(
            "this app's voice channel is cascaded: connect a WebSocket to {}/ws",
            route(agent)
        ))
        .into());
    }
    let provider = provider(host, &config)?;
    let session_id = call_session(host, agent, body.session_id).await?;
    let session = host.session(&session_id).await?;
    let channel = VoiceChannel::with_config(provider, config.clone());
    let call = channel
        .accept_webrtc(&session, &body.sdp)
        .await
        .map_err(voice_error)?;
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

/// `GET /v1/channels/{agent}/voice/ws` query.
#[derive(Deserialize)]
pub(crate) struct SocketQuery {
    /// Talk on an existing session of the same agent; a new one otherwise.
    #[serde(default)]
    session_id: Option<String>,
}

/// `GET /v1/channels/{agent}/voice/ws`: a cascaded call over a WebSocket.
///
/// The call is placed before the upgrade, so a bad request or a refused call
/// answers with an HTTP error instead of a socket that closes at once.
pub(crate) async fn socket(
    State(host): State<Arc<Host>>,
    Path(agent): Path<String>,
    Query(query): Query<SocketQuery>,
    upgrade: WebSocketUpgrade,
) -> Response {
    match open(&host, &agent, query.session_id).await {
        Ok((session_id, call, audio)) => {
            upgrade.on_upgrade(move |socket| relay(socket, session_id, call, audio))
        }
        Err(err) => crate::server::Failure::from(err).into_response(),
    }
}

async fn open(
    host: &Host,
    agent: &str,
    session_id: Option<String>,
) -> crate::Result<(String, VoiceCall, CallAudio)> {
    top_level(host, agent)?;
    let config = config(host);
    if config.mode != VoiceMode::Cascaded {
        return Err(ApiError::BadRequest(format!(
            "this app's voice channel is delegated: POST a WebRTC offer to {}/calls",
            route(agent)
        ))
        .into());
    }
    let provider = provider(host, &config)?;
    let session_id = call_session(host, agent, session_id).await?;
    let session = host.session(&session_id).await?;
    let (call, audio) = VoiceChannel::with_config(provider, config)
        .accept_audio(&session)
        .await
        .map_err(voice_error)?;
    Ok((session_id, call, audio))
}

/// Carry one cascaded call over its socket until either side hangs up.
async fn relay(mut socket: WebSocket, session_id: String, call: VoiceCall, audio: CallAudio) {
    let CallAudio { input, mut output } = audio;
    let mut events = call.events();
    let hello = json!({
        "type": "call",
        "session_id": session_id,
        "call_id": call.call_id(),
        "sample_rate": AUDIO_SAMPLE_RATE,
        "format": "pcm16",
    });
    if send_json(&mut socket, &hello).await.is_err() {
        return;
    }
    loop {
        let sent = tokio::select! {
            message = socket.recv() => match message {
                Some(Ok(Message::Binary(frame))) => {
                    if input.send(frame.to_vec()).await.is_err() {
                        break;
                    }
                    Ok(())
                }
                // Control messages from the caller are reserved.
                Some(Ok(Message::Text(_) | Message::Ping(_) | Message::Pong(_))) => Ok(()),
                Some(Ok(Message::Close(_)) | Err(_)) | None => break,
            },
            out = output.recv() => match out {
                Some(AudioOut::Audio(pcm)) => socket.send(Message::Binary(pcm.into())).await,
                Some(AudioOut::Clear) => send_json(&mut socket, &json!({ "type": "clear" })).await,
                None => break,
            },
            event = events.recv() => match event {
                Ok(event) => match transcript(&event) {
                    Some(line) => send_json(&mut socket, &line).await,
                    None => Ok(()),
                },
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => Ok(()),
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
        };
        if sent.is_err() {
            break;
        }
    }
    drop(input);
    let ended = match call.end().await {
        Ok(summary) => json!({
            "type": "ended",
            "utterances": summary.utterances,
            "spoken_chunks": summary.spoken_chunks,
            "interruptions": summary.interruptions,
        }),
        Err(err) => json!({ "type": "ended", "error": err.to_string() }),
    };
    let _ = send_json(&mut socket, &ended).await;
    let _ = socket.send(Message::Close(None)).await;
}

async fn send_json(socket: &mut WebSocket, value: &Value) -> Result<(), axum::Error> {
    socket.send(Message::Text(value.to_string().into())).await
}

/// The transcript line a call event shows the caller, if any.
fn transcript(event: &VoiceCallEvent) -> Option<Value> {
    match event {
        VoiceCallEvent::InputTranscriptCompleted { transcript, .. } => {
            Some(json!({ "type": "transcript", "role": "user", "text": transcript }))
        }
        VoiceCallEvent::OutputTranscriptCompleted { transcript, .. } => {
            Some(json!({ "type": "transcript", "role": "assistant", "text": transcript }))
        }
        VoiceCallEvent::OutputInterrupted { heard, .. } => {
            Some(json!({ "type": "interrupted", "heard": heard }))
        }
        _ => None,
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
    let model = match config.mode {
        VoiceMode::Cascaded => config.speech_model().to_string(),
        _ => config.model.clone(),
    };
    json!({
        "mode": config.mode,
        "model": model,
        "voice": config.voice,
        "endpoints": endpoints,
    })
}
