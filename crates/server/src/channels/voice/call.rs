//! Running one call: place it with the realtime provider, then drive it with
//! the shared voice loop against the session.

use super::*;
use crate::api::messages::{InputMessage, MessageRole};
use crate::domains::messages::CreateMessage;
use crate::domains::sessions::CancelSession;
use crate::kernel_imports::InputContentPart;
use async_trait::async_trait;
use everruns_contracts::runtime_provider::ProviderEndpoint;
use everruns_contracts::voice::{RealtimeSessionConfig, SharedRealtimeDriver};
use everruns_core::events::{
    VOICE_INPUT_TRANSCRIPT_COMPLETED, VOICE_INPUT_TRANSCRIPT_DELTA,
    VOICE_OUTPUT_TRANSCRIPT_COMPLETED, VOICE_OUTPUT_TRANSCRIPT_DELTA,
};
use everruns_core::voice::{
    AgentOutputMapper, VoiceLoop, VoiceLoopError, VoiceLoopEvent, VoiceSessionPort,
};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use uuid::Uuid;

pub(super) struct CallSettings {
    pub config: VoiceChannelConfig,
    pub channel_id: Option<String>,
    pub provider_id: Option<String>,
    pub sdp: String,
}

/// Place a call and start driving it in the background.
pub(super) async fn start(
    state: &AppState,
    org: &ResolvedOrg,
    session_id: SessionId,
    settings: CallSettings,
) -> Result<VoiceCallResponse, (StatusCode, Json<ErrorResponse>)> {
    if settings.sdp.trim().is_empty() {
        return Err(ErrorResponse::new("Missing SDP").into_response(StatusCode::BAD_REQUEST));
    }
    settings
        .config
        .validate()
        .map_err(|e| ErrorResponse::new(e).into_response(StatusCode::BAD_REQUEST))?;
    let binding = settings.provider_id.as_deref();
    let provider = state
        .provider_resolver
        .resolve_realtime(org.org_id, binding)
        .await
        .map_err(|error| resolution_error(binding, error))?;
    let voice_connection_id = format!("voice_conn_{}", Uuid::now_v7().simple());
    let session = RealtimeSessionConfig::from_channel(&settings.config)
        .with_safety_identifier(safety_identifier(org.org_id, org.user_id, session_id));
    let call = match provider
        .driver
        .accept_webrtc(&provider.endpoint, &session, &settings.sdp)
        .await
    {
        Ok(call) => call,
        Err(error) => {
            tracing::error!(error = %error, "realtime provider rejected the voice call");
            lifecycle::emit(
                state,
                session_id,
                lifecycle::failed(&voice_connection_id, "provider_error"),
            )
            .await;
            return Err(ErrorResponse::bad_gateway());
        }
    };
    let lease = lifecycle::upsert_lease(
        state,
        lifecycle::LeaseRecord {
            session_id,
            voice_connection_id: &voice_connection_id,
            provider_call_id: call.call_id.as_deref(),
            provider_id: &provider.provider_id,
            channel_id: settings.channel_id.as_deref(),
            config: &settings.config,
        },
    )
    .await?;
    lifecycle::emit(
        state,
        session_id,
        lifecycle::started(&voice_connection_id, &settings.config),
    )
    .await;
    if let Some(call_id) = call.call_id.clone() {
        let shutdown = CancellationToken::new();
        state
            .calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(voice_connection_id.clone(), shutdown.clone());
        tokio::spawn(run(
            state.clone(),
            org.clone(),
            session_id,
            voice_connection_id.clone(),
            settings.config.clone(),
            (provider.driver.clone(), provider.endpoint.clone(), call_id),
            shutdown,
        ));
    } else {
        tracing::warn!("realtime provider returned no call id; the call has no voice loop");
    }
    Ok(VoiceCallResponse {
        voice_connection_id,
        provider_call_id: call.call_id,
        provider: provider.provider_type,
        model: settings.config.model,
        voice: settings.config.voice,
        channel_id: settings.channel_id,
        expires_at: lease.lease_expires_at,
        answer_sdp: call.answer_sdp,
    })
}

/// End a call: stop it if it runs here, release its lease and record the end.
pub(super) async fn end(
    state: &AppState,
    session_id: SessionId,
    voice_connection_id: &str,
    reason: Option<String>,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let running = state
        .calls
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(voice_connection_id);
    if let Some(shutdown) = running {
        shutdown.cancel();
    }
    lifecycle::release_lease(state, session_id, voice_connection_id).await?;
    lifecycle::emit(
        state,
        session_id,
        lifecycle::ended(voice_connection_id, reason, None),
    )
    .await;
    Ok(())
}

async fn run(
    state: AppState,
    org: ResolvedOrg,
    session_id: SessionId,
    voice_connection_id: String,
    config: VoiceChannelConfig,
    (driver, endpoint, call_id): (SharedRealtimeDriver, ProviderEndpoint, String),
    shutdown: CancellationToken,
) {
    let started = Instant::now();
    let outcome = drive(
        &state,
        &org,
        session_id,
        &voice_connection_id,
        config,
        (driver, endpoint, call_id),
        shutdown.clone(),
    )
    .await;
    let ended_here = state
        .calls
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&voice_connection_id)
        .is_some();
    match outcome {
        Err(error) => {
            tracing::warn!(session_id = %session_id, voice_connection_id = %voice_connection_id, "voice call failed: {error}");
            lifecycle::emit(
                &state,
                session_id,
                lifecycle::failed(&voice_connection_id, "call_failed"),
            )
            .await;
        }
        // An explicit end already released the lease and recorded the end.
        Ok(_) if !ended_here => {}
        Ok(summary) => {
            tracing::info!(
                session_id = %session_id,
                utterances = summary.utterances,
                interruptions = summary.interruptions,
                "voice call ended"
            );
            let _ = lifecycle::release_lease(&state, session_id, &voice_connection_id).await;
            let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            lifecycle::emit(
                &state,
                session_id,
                lifecycle::ended(
                    &voice_connection_id,
                    Some("call_closed".into()),
                    Some(duration_ms),
                ),
            )
            .await;
        }
    }
}

async fn drive(
    state: &AppState,
    org: &ResolvedOrg,
    session_id: SessionId,
    voice_connection_id: &str,
    config: VoiceChannelConfig,
    (driver, endpoint, call_id): (SharedRealtimeDriver, ProviderEndpoint, String),
    shutdown: CancellationToken,
) -> Result<everruns_core::voice::VoiceLoopSummary, VoiceLoopError> {
    // Subscribe before the loop can send the first message, so no answer
    // event is missed.
    let mut events = state
        .event_delivery
        .subscribe(session_id.uuid())
        .await
        .map_err(|error| VoiceLoopError::Session(error.to_string()))?;
    let connection = driver.attach(&endpoint, &call_id).await?;
    let (agent_tx, agent_rx) = mpsc::channel(256);
    let pump_shutdown = shutdown.clone();
    let pump = tokio::spawn(async move {
        let mut mapper = AgentOutputMapper::default();
        loop {
            let event = tokio::select! {
                () = pump_shutdown.cancelled() => break,
                event = events.recv() => event,
            };
            let Some(event) = event else { break };
            // Only output, sent-message and turn events speak; skip
            // serializing the rest (tool results can be large).
            if !(event.event_type.starts_with("output.message.")
                || event.event_type.starts_with("turn.")
                || event.event_type == everruns_core::events::CONVERSATION_MESSAGE)
            {
                continue;
            }
            let data = serde_json::to_value(&event.data).unwrap_or_default();
            for output in mapper.map(&event.event_type, &data) {
                if agent_tx.send(output).await.is_err() {
                    return;
                }
            }
        }
    });
    // The lease is the upper bound on a call.
    let deadline = shutdown.clone();
    let cap = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(u64::from(lifecycle::LEASE_SECONDS))).await;
        deadline.cancel();
    });
    let port = ServerPort {
        state: state.clone(),
        org: org.clone(),
        session_id,
        voice_connection_id: voice_connection_id.to_string(),
        transcripts: Mutex::default(),
    };
    let result = VoiceLoop::new(config, port)
        .run(connection, agent_rx, shutdown.clone())
        .await;
    shutdown.cancel();
    cap.abort();
    pump.abort();
    result
}

/// The session side of a call on the platform.
struct ServerPort {
    state: AppState,
    org: ResolvedOrg,
    session_id: SessionId,
    voice_connection_id: String,
    /// Accumulated transcript text by item or response id.
    transcripts: Mutex<HashMap<String, String>>,
}

impl ServerPort {
    fn accumulate(&self, key: &str, delta: &str, complete: Option<&str>) -> String {
        let mut transcripts = self
            .transcripts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match complete {
            Some(full) => {
                let acc = transcripts.remove(key).unwrap_or_default();
                if full.is_empty() {
                    acc
                } else {
                    full.to_string()
                }
            }
            None => {
                let entry = transcripts.entry(key.to_string()).or_default();
                entry.push_str(delta);
                entry.clone()
            }
        }
    }
}

#[async_trait]
impl VoiceSessionPort for ServerPort {
    async fn send_utterance(&self, text: &str) -> Result<(), VoiceLoopError> {
        let metadata = HashMap::from([
            ("source".to_string(), json!("voice")),
            (
                "voice_connection_id".to_string(),
                json!(self.voice_connection_id),
            ),
        ]);
        CreateMessage {
            session_id: self.session_id.to_string(),
            message: InputMessage {
                role: MessageRole::User,
                content: vec![InputContentPart::text(text.to_string())],
            },
            addressed_participant_id: None,
            controls: None,
            metadata: Some(metadata),
            tags: Some(vec!["voice".to_string()]),
            external_actor: None,
            request_id: None,
        }
        .run(&self.state.ctx(&self.org))
        .await
        .map(|_| ())
        .map_err(|error| VoiceLoopError::Session(error.to_string()))
    }

    async fn cancel_turn(&self) -> Result<(), VoiceLoopError> {
        CancelSession {
            session_id: self.session_id.to_string(),
        }
        .run(&self.state.ctx(&self.org))
        .await
        .map(|_| ())
        .map_err(|error| VoiceLoopError::Session(error.to_string()))
    }

    async fn record(&self, event: VoiceLoopEvent) {
        let id = &self.voice_connection_id;
        let data = match event {
            VoiceLoopEvent::InputTranscriptDelta { item_id, delta } => {
                let acc = self.accumulate(&item_id, &delta, None);
                lifecycle::transcript(
                    VOICE_INPUT_TRANSCRIPT_DELTA,
                    id,
                    Some(item_id),
                    None,
                    delta,
                    acc,
                )
            }
            VoiceLoopEvent::InputTranscriptCompleted {
                item_id,
                transcript,
            } => {
                let acc = self.accumulate(&item_id, "", Some(&transcript));
                lifecycle::transcript(
                    VOICE_INPUT_TRANSCRIPT_COMPLETED,
                    id,
                    Some(item_id),
                    None,
                    String::new(),
                    acc,
                )
            }
            VoiceLoopEvent::OutputTranscriptDelta { response_id, delta } => {
                let acc = self.accumulate(&response_id, &delta, None);
                lifecycle::transcript(
                    VOICE_OUTPUT_TRANSCRIPT_DELTA,
                    id,
                    None,
                    Some(response_id),
                    delta,
                    acc,
                )
            }
            VoiceLoopEvent::OutputTranscriptCompleted {
                response_id,
                transcript,
            } => {
                let acc = self.accumulate(&response_id, "", Some(&transcript));
                lifecycle::transcript(
                    VOICE_OUTPUT_TRANSCRIPT_COMPLETED,
                    id,
                    None,
                    Some(response_id),
                    String::new(),
                    acc,
                )
            }
            VoiceLoopEvent::OutputInterrupted {
                heard,
                unspoken,
                policy,
            } => lifecycle::interrupted(id, heard, unspoken, policy),
            VoiceLoopEvent::AnswerStarted { latency_ms } => {
                tracing::info!(session_id = %self.session_id, latency_ms, "voice answer started");
                return;
            }
            VoiceLoopEvent::ProviderError { message } => {
                tracing::warn!(session_id = %self.session_id, "realtime provider error: {message}");
                return;
            }
        };
        lifecycle::emit(&self.state, self.session_id, data).await;
    }
}

/// Map a realtime provider resolution failure onto an HTTP error. A pinned
/// binding that cannot serve the call is the caller's mistake (400); without
/// one, missing configuration is a provider problem (502).
fn resolution_error(
    binding: Option<&str>,
    error: anyhow::Error,
) -> (StatusCode, Json<ErrorResponse>) {
    match binding {
        Some(provider_id) => ErrorResponse::new(format!(
            "Realtime provider '{provider_id}' is not available for voice: {error}"
        ))
        .into_response(StatusCode::BAD_REQUEST),
        None => {
            tracing::error!("No realtime provider for voice: {error}");
            ErrorResponse::bad_gateway()
        }
    }
}

/// Stable, privacy-preserving end-user id for provider abuse monitoring.
fn safety_identifier(org_id: i64, user_id: Option<Uuid>, session_id: SessionId) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"everruns:voice:v1:");
    hasher.update(org_id.to_be_bytes());
    hasher.update(b":");
    if let Some(user_id) = user_id {
        hasher.update(user_id.as_bytes());
    }
    hasher.update(b":");
    hasher.update(session_id.uuid().as_bytes());
    let digest = hex::encode(hasher.finalize());
    format!("evr_{}", &digest[..60])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safety_identifier_is_stable_and_opaque() {
        let session = SessionId::new();
        let a = safety_identifier(1, None, session);
        assert_eq!(a, safety_identifier(1, None, session));
        assert_ne!(a, safety_identifier(2, None, session));
        assert!(a.starts_with("evr_") && a.len() == 64);
    }
}
