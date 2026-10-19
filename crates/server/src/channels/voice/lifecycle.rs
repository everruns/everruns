//! Durable traces of a call: the leased resource that records it and the
//! `voice.*` events. Only sanitized metadata is stored.

use super::*;
use crate::kernel_imports::{LeasedResource, UpsertLeasedResource};
use everruns_core::events::{
    EventContext, EventData, EventRequest, VoiceOutputInterruptedData, VoiceSessionEndedData,
    VoiceSessionFailedData, VoiceSessionStartedData, VoiceTranscriptData,
};

pub(super) const VOICE_RESOURCE_TYPE: &str = "voice_connection";
/// Leased-resource provider label for voice connections.
const LEASE_PROVIDER: &str = "openai";
/// A call holds its lease for at most this long.
pub(super) const LEASE_SECONDS: u32 = 60 * 60;

pub(super) struct LeaseRecord<'a> {
    pub session_id: SessionId,
    pub voice_connection_id: &'a str,
    pub provider_call_id: Option<&'a str>,
    pub provider_id: &'a str,
    pub channel_id: Option<&'a str>,
    pub config: &'a VoiceChannelConfig,
}

pub(super) async fn upsert_lease(
    state: &AppState,
    record: LeaseRecord<'_>,
) -> Result<LeasedResource, (StatusCode, Json<ErrorResponse>)> {
    let mut metadata = json!({
        "status": "active",
        "mode": record.config.mode,
        "model": record.config.model,
        "voice": record.config.voice,
        "transport": "webrtc",
        "provider_id": record.provider_id,
    });
    if let Some(call_id) = record.provider_call_id {
        metadata["provider_call_id"] = json!(call_id);
    }
    if let Some(channel_id) = record.channel_id {
        metadata["channel_id"] = json!(channel_id);
    }
    state
        .leased_resource_store
        .upsert_resource(UpsertLeasedResource {
            session_id: record.session_id,
            provider: LEASE_PROVIDER.to_string(),
            resource_type: VOICE_RESOURCE_TYPE.to_string(),
            external_id: record.voice_connection_id.to_string(),
            display_name: Some("Voice Connection".to_string()),
            // Lease owners are virtual users (end users of an org); a voice
            // call is authorized through its session, so it records none.
            owner_user_id: None,
            connection_id: None,
            lease_duration_seconds: LEASE_SECONDS,
            metadata,
        })
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "failed to upsert voice leased resource");
            ErrorResponse::internal_error()
        })
}

pub(super) async fn release_lease(
    state: &AppState,
    session_id: SessionId,
    voice_connection_id: &str,
) -> Result<Option<LeasedResource>, (StatusCode, Json<ErrorResponse>)> {
    state
        .leased_resource_store
        .release_resource(
            session_id,
            LEASE_PROVIDER,
            VOICE_RESOURCE_TYPE,
            voice_connection_id,
        )
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "failed to release voice leased resource");
            ErrorResponse::internal_error()
        })
}

pub(super) async fn emit(state: &AppState, session_id: SessionId, data: impl Into<EventData>) {
    if let Err(error) = state
        .event_service
        .emit(EventRequest::new(session_id, EventContext::empty(), data))
        .await
    {
        tracing::warn!(session_id = %session_id, error = %error, "failed to emit voice event");
    }
}

pub(super) fn started(voice_connection_id: &str, config: &VoiceChannelConfig) -> EventData {
    VoiceSessionStartedData {
        voice_connection_id: voice_connection_id.to_string(),
        model: config.model.clone(),
        voice: config.voice.clone(),
        // Delegated calls do no reasoning in the speech model.
        reasoning_effort: "none".to_string(),
        transport: "webrtc".to_string(),
    }
    .into()
}

pub(super) fn ended(
    voice_connection_id: &str,
    reason: Option<String>,
    duration_ms: Option<u64>,
) -> EventData {
    VoiceSessionEndedData {
        voice_connection_id: voice_connection_id.to_string(),
        reason,
        duration_ms,
    }
    .into()
}

pub(super) fn failed(voice_connection_id: &str, error: &str) -> EventData {
    VoiceSessionFailedData {
        voice_connection_id: voice_connection_id.to_string(),
        error: error.to_string(),
    }
    .into()
}

pub(super) fn interrupted(
    voice_connection_id: &str,
    heard: String,
    unspoken: String,
    policy: everruns_contracts::voice::Interruption,
) -> EventData {
    let policy = match policy {
        everruns_contracts::voice::Interruption::Steer => "steer",
        everruns_contracts::voice::Interruption::Cancel => "cancel",
    };
    VoiceOutputInterruptedData {
        voice_connection_id: voice_connection_id.to_string(),
        heard,
        unspoken,
        policy: policy.to_string(),
    }
    .into()
}

/// A transcript event. `item_id` is set for caller speech, `response_id` for
/// spoken output.
pub(super) fn transcript(
    event_type: &str,
    voice_connection_id: &str,
    item_id: Option<String>,
    response_id: Option<String>,
    delta: String,
    accumulated: String,
) -> EventData {
    EventData::voice_transcript_event(
        VoiceTranscriptData {
            voice_connection_id: voice_connection_id.to_string(),
            phase: None,
            item_id,
            response_id,
            delta,
            accumulated,
        },
        event_type,
    )
}
