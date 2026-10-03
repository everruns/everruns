#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! gRPC protocol for Everruns worker ↔ control-plane communication.
//!
//! `everruns-internal-protocol` is part of the [Everruns](https://everruns.com)
//! ecosystem. It is an internal building block that defines the gRPC service and
//! message types Everruns workers use to talk to the control-plane server, and
//! the conversions between protobuf and Everruns domain types. It is not a stable
//! public API.

// Decision: gRPC with tonic (industry standard, already in stack)
// Decision: Use google.protobuf.Value/Struct for JSON values instead of strings
// Decision: Proto is transport layer, Rust schemas remain source of truth

mod capability_wire;
mod credential_fingerprint;
mod json_wire;
#[cfg(test)]
mod rolling_upgrade_tests;
mod slack_action_wire;
use chrono::{DateTime, TimeZone, Utc};
use everruns_contracts::typed_id::{EventId, ExecId, MessageId, SessionId, TurnId};

pub mod proto {
    tonic::include_proto!("everruns.internal");
}

pub use credential_fingerprint::credential_fingerprint;
pub use json_wire::{
    json_array_to_proto_list, json_object_to_proto_struct, json_to_proto_list,
    json_to_proto_struct, json_to_proto_value, proto_list_to_json, proto_struct_to_json,
    proto_value_to_json,
};
pub use proto::worker_service_client::WorkerServiceClient;
pub use proto::worker_service_server::{WorkerService, WorkerServiceServer};
// ============================================================================
// Error types
// ============================================================================

#[derive(Debug)]
pub enum ConversionError {
    MissingField(&'static str),
    InvalidUuid(uuid::Error),
    JsonError(serde_json::Error),
}

impl std::fmt::Display for ConversionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConversionError::MissingField(field) => write!(f, "Missing required field: {}", field),
            ConversionError::InvalidUuid(e) => write!(f, "Invalid UUID: {}", e),
            ConversionError::JsonError(e) => write!(f, "JSON error: {}", e),
        }
    }
}

impl std::error::Error for ConversionError {}

impl From<uuid::Error> for ConversionError {
    fn from(e: uuid::Error) -> Self {
        ConversionError::InvalidUuid(e)
    }
}

impl From<serde_json::Error> for ConversionError {
    fn from(e: serde_json::Error) -> Self {
        ConversionError::JsonError(e)
    }
}

impl From<ConversionError> for tonic::Status {
    fn from(e: ConversionError) -> Self {
        tonic::Status::invalid_argument(e.to_string())
    }
}

// ============================================================================
// Conversion traits for basic types
// ============================================================================

/// Convert from proto Uuid to uuid::Uuid
pub fn proto_uuid_to_uuid(value: &proto::Uuid) -> Result<uuid::Uuid, ConversionError> {
    uuid::Uuid::parse_str(&value.value).map_err(ConversionError::from)
}

/// Convert from uuid::Uuid to proto Uuid
pub fn uuid_to_proto_uuid(value: uuid::Uuid) -> proto::Uuid {
    proto::Uuid {
        value: value.to_string(),
    }
}

/// Build a typed-id string (`<prefix>_<hex>`) from a proto Uuid, stripping the
/// dashes the typed-id layer does not use. Centralizes the prefixed-id
/// construction the proto↔schema conversions repeat (EVE-652).
fn prefixed_id(prefix: &str, value: &proto::Uuid) -> String {
    format!("{prefix}_{}", value.value.replace('-', ""))
}

/// Convert from proto Timestamp to chrono `DateTime<Utc>`
pub fn proto_timestamp_to_datetime(value: &proto::Timestamp) -> DateTime<Utc> {
    Utc.timestamp_opt(value.seconds, value.nanos as u32)
        .single()
        .unwrap_or_else(|| {
            // EVE-652: an out-of-range timestamp previously fell back to "now"
            // silently, corrupting event sequencing / audit timing. Keep the
            // fallback (this is an infallible conversion) but make the loss visible.
            tracing::warn!(
                seconds = value.seconds,
                nanos = value.nanos,
                "internal-protocol: out-of-range proto timestamp; falling back to current time"
            );
            Utc::now()
        })
}

/// Convert from chrono `DateTime<Utc>` to proto Timestamp
pub fn datetime_to_proto_timestamp(value: DateTime<Utc>) -> proto::Timestamp {
    proto::Timestamp {
        seconds: value.timestamp(),
        nanos: value.timestamp_subsec_nanos() as i32,
    }
}

// ============================================================================
// Event data serialization/deserialization
// ============================================================================

/// Deserialize event data from JSON based on event_type
///
/// Uses the core library's deserialize_event_data function which handles
/// type-directed deserialization and returns Unsupported for unknown types.
fn deserialize_event_data(event_type: &str, data: serde_json::Value) -> everruns_core::EventData {
    everruns_core::events::deserialize_event_data(event_type, data)
}

/// Fallback for an event-data serialization failure on this infallible path.
///
/// EVE-652: every arm of [`serialize_event_data`] previously used
/// `.unwrap_or_default()`, silently emitting an empty `{}`/`null` and dropping the
/// whole event payload on any serde error. Serialization of well-formed domain
/// structs does not fail in practice, so a hit here signals upstream corruption
/// that must not vanish silently. We keep the empty placeholder (this conversion
/// is infallible by design) but log the loss.
fn event_data_serialize_fallback(err: serde_json::Error) -> serde_json::Value {
    tracing::error!(
        error = %err,
        "internal-protocol: failed to serialize event data; emitting empty payload (data lost)"
    );
    serde_json::Value::Null
}

/// Serialize EventData to JSON Value
///
/// Converts the typed EventData variant to its JSON representation.
fn serialize_event_data(data: &everruns_core::EventData) -> serde_json::Value {
    use everruns_core::EventData;
    fn to_json(d: &impl serde::Serialize) -> serde_json::Value {
        serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
    }

    match data {
        EventData::InputMessage(d) => to_json(d),
        EventData::OutputMessageStarted(d) => to_json(d),
        EventData::OutputMessageDelta(d) => to_json(d),
        EventData::OutputMessageReplaced(d) => to_json(d),
        EventData::OutputMessageCompleted(d) => to_json(d),
        EventData::TurnStarted(d) => to_json(d),
        EventData::TurnCompleted(d) => to_json(d),
        EventData::TurnFailed(d) => to_json(d),
        EventData::TurnSealed(d) => to_json(d),
        EventData::TurnCancelled(d) => to_json(d),
        EventData::ReasonStarted(d) => to_json(d),
        EventData::ReasonCompleted(d) => to_json(d),
        EventData::ReasonRecovered(d) => to_json(d),
        EventData::CapabilityUsage(d) => to_json(d),
        EventData::ActStarted(d) => to_json(d),
        EventData::ActCompleted(d) => to_json(d),
        EventData::ToolStarted(d) => to_json(d),
        EventData::ToolCompleted(d) => to_json(d),
        EventData::ToolProgress(d) => to_json(d),
        EventData::ToolHostedCall(d) => to_json(d),
        EventData::ToolOutputDelta(d) => to_json(d),
        EventData::ToolCallRequested(d) => to_json(d),
        EventData::LlmGeneration(d) => to_json(d),
        EventData::ReasonThinkingStarted(d) => to_json(d),
        EventData::ReasonThinkingDelta(d) => to_json(d),
        EventData::ReasonThinkingCompleted(d) => to_json(d),
        EventData::ReasonItem(d) => to_json(d),
        EventData::SessionStarted(d) => to_json(d),
        EventData::SessionActivated(d) => to_json(d),
        EventData::SessionIdled(d) => to_json(d),
        EventData::SessionTitleUpdated(d) => to_json(d),
        EventData::SessionModelChanged(d) => to_json(d),
        EventData::EnvironmentInstanceLost(d) | EventData::EnvironmentRecovered(d) => to_json(d),
        EventData::TaskCreated(d) => to_json(d),
        EventData::TaskUpdated(d) => to_json(d),
        EventData::TaskMessageSent(d) => to_json(d),
        EventData::TaskMessageReceived(d) => to_json(d),
        EventData::ContextCompacting(d) => to_json(d),
        EventData::ContextCompacted(d) => to_json(d),
        EventData::ContextCompactionSkipped(d) => to_json(d),
        EventData::ContextCompactionFailed(d) => to_json(d),
        EventData::BudgetWarning(d)
        | EventData::BudgetPaused(d)
        | EventData::BudgetExhausted(d)
        | EventData::BudgetResumed(d) => to_json(d),
        EventData::FileWritten(d) => to_json(d),
        EventData::VoiceSessionStarted(d) => to_json(d),
        EventData::VoiceInputTranscriptDelta(d)
        | EventData::VoiceInputTranscriptCompleted(d)
        | EventData::VoiceOutputTranscriptDelta(d)
        | EventData::VoiceOutputTranscriptCompleted(d) => to_json(d),
        EventData::VoiceSessionEnded(d) => to_json(d),
        EventData::VoiceSessionFailed(d) => to_json(d),
        EventData::TranscriptRepaired(d) => to_json(d),
        EventData::ToolCallRepaired(d) => to_json(d),
        EventData::Unsupported { data, .. } => {
            // Should not happen in production - unsupported events are filtered before reaching here
            data.clone()
        }
    }
}

// ============================================================================
// Conversion to/from schemas types
// ============================================================================

/// Convert proto Agent to the stored platform Agent record using JSON.
///
/// EVE-877: the stored record lives in `everruns-platform`; the proto shape is
/// unchanged. Both endpoints of this wire (server, worker) are platform-side.
pub fn proto_agent_to_schema(
    value: proto::Agent,
) -> Result<everruns_platform::Agent, ConversionError> {
    // Serialize proto to JSON, then deserialize to schema type
    // This is simpler and more maintainable than field-by-field conversion
    let tags: Vec<String> = vec![];
    let capabilities: Vec<serde_json::Value> = if value.capabilities.is_empty() {
        value
            .capability_ids
            .iter()
            .map(|id| serde_json::json!({"ref": id, "config": {}}))
            .collect()
    } else {
        value
            .capabilities
            .iter()
            .map(|config| serde_json::from_str(config).map_err(ConversionError::from))
            .collect::<Result<_, _>>()?
    };

    // Convert UUID string to prefixed format for typed IDs.
    // EVE-652: a missing id previously became "" and failed later as an opaque
    // JsonError; surface it as the precise MissingField instead.
    let id_str = value
        .id
        .as_ref()
        .map(|u| prefixed_id("agent", u))
        .ok_or(ConversionError::MissingField("id"))?;
    let model_id_str = value
        .default_model_id
        .as_ref()
        .map(|u| prefixed_id("model", u));
    let harness_id_str = value
        .harness_id
        .as_ref()
        .map(|u| prefixed_id("harness", u))
        .ok_or(ConversionError::MissingField("harness_id"))?;

    let json = serde_json::json!({
        "id": id_str,
        "name": value.name,
        "display_name": value.display_name,
        "description": if value.description.is_empty() { None } else { Some(&value.description) },
        "system_prompt": value.system_prompt,
        "default_model_id": model_id_str,
        "harness_id": harness_id_str,
        "tags": tags,
        "capabilities": capabilities,
        "status": value.status,
        "created_at": value.created_at.as_ref().map(|t| proto_timestamp_to_datetime(t).to_rfc3339()),
        "updated_at": value.updated_at.as_ref().map(|t| proto_timestamp_to_datetime(t).to_rfc3339()),
        "parallel_tool_calls": value.parallel_tool_calls,
    });
    serde_json::from_value(json).map_err(ConversionError::from)
}

/// Convert the stored platform Agent record to proto Agent
pub fn schema_agent_to_proto(value: &everruns_platform::Agent) -> proto::Agent {
    proto::Agent {
        service_virtual_user_id: value
            .service_virtual_user_id
            .map(|id| uuid_to_proto_uuid(id.uuid())),
        id: Some(uuid_to_proto_uuid(value.internal_id)),
        name: value.name.clone(),
        description: value.description.clone().unwrap_or_default(),
        system_prompt: value.system_prompt.clone(),
        default_model_id: value
            .default_model_id
            .map(|id| uuid_to_proto_uuid(id.uuid())),
        temperature: None,
        max_tokens: None,
        status: value.status.to_string(),
        created_at: Some(datetime_to_proto_timestamp(value.created_at)),
        updated_at: Some(datetime_to_proto_timestamp(value.updated_at)),
        // Extract capability IDs from AgentCapabilityConfig
        capability_ids: value
            .capabilities
            .iter()
            .map(|c| c.capability_id().to_string())
            .collect(),
        display_name: value.display_name.clone(),
        parallel_tool_calls: value.parallel_tool_calls,
        harness_id: Some(uuid_to_proto_uuid(value.harness_id.uuid())),
        capabilities: capability_wire::encode_configs(&value.capabilities),
    }
}

/// Convert schemas Harness to proto Harness
pub fn schema_harness_to_proto(value: &everruns_platform::Harness) -> proto::Harness {
    proto::Harness {
        id: Some(uuid_to_proto_uuid(value.id.uuid())),
        name: value.name.clone(),
        description: value.description.clone().unwrap_or_default(),
        // proto carries a plain string; absent base prompt maps to "".
        system_prompt: value.system_prompt.clone().unwrap_or_default(),
        default_model_id: value
            .default_model_id
            .map(|id| uuid_to_proto_uuid(id.uuid())),
        status: value.status.to_string(),
        created_at: Some(datetime_to_proto_timestamp(value.created_at)),
        updated_at: Some(datetime_to_proto_timestamp(value.updated_at)),
        capability_ids: value
            .capabilities
            .iter()
            .map(|c| c.capability_id().to_string())
            .collect(),
        tags: value.tags.clone(),
        parent_harness_id: value
            .parent_harness_id
            .map(|id| uuid_to_proto_uuid(id.uuid())),
        is_built_in: value.is_built_in,
        display_name: value.display_name.clone(),
        capabilities: capability_wire::encode_configs(&value.capabilities),
    }
}

/// Convert proto Harness to schemas Harness
pub fn proto_harness_to_schema(
    value: proto::Harness,
) -> Result<everruns_platform::Harness, ConversionError> {
    // EVE-652: a missing harness id previously became "" (opaque downstream
    // failure); surface the precise MissingField instead.
    let id_str = value
        .id
        .as_ref()
        .map(|u| prefixed_id("harness", u))
        .ok_or(ConversionError::MissingField("id"))?;
    let model_id_str = value
        .default_model_id
        .as_ref()
        .map(|u| prefixed_id("model", u));
    let parent_harness_id_str = value
        .parent_harness_id
        .as_ref()
        .map(|u| prefixed_id("harness", u));

    let capabilities: Vec<serde_json::Value> = if value.capabilities.is_empty() {
        value
            .capability_ids
            .iter()
            .map(|id| serde_json::json!({"ref": id, "config": {}}))
            .collect()
    } else {
        value
            .capabilities
            .iter()
            .map(|config| serde_json::from_str(config).map_err(ConversionError::from))
            .collect::<Result<_, _>>()?
    };

    let json = serde_json::json!({
        "id": id_str,
        "name": value.name,
        "display_name": value.display_name,
        "description": if value.description.is_empty() { None } else { Some(&value.description) },
        "system_prompt": value.system_prompt,
        "parent_harness_id": parent_harness_id_str,
        "default_model_id": model_id_str,
        "tags": value.tags,
        "capabilities": capabilities,
        "is_built_in": value.is_built_in,
        "status": value.status,
        "created_at": value.created_at.as_ref().map(|t| proto_timestamp_to_datetime(t).to_rfc3339()),
        "updated_at": value.updated_at.as_ref().map(|t| proto_timestamp_to_datetime(t).to_rfc3339()),
    });
    serde_json::from_value(json).map_err(ConversionError::from)
}

/// Convert proto Session to the stored platform Session record using JSON
/// (EVE-882: the persisted aggregate lives in `everruns-platform`).
pub fn proto_session_to_schema(
    value: proto::Session,
) -> Result<everruns_platform::Session, ConversionError> {
    let tags = value.tags.clone();
    let started_at: Option<String> = None;
    let finished_at: Option<String> = None;

    // Convert UUID string to prefixed format for typed IDs.
    // EVE-652: the session id is required; a missing id previously became ""
    // and failed later as an opaque JsonError. Surface MissingField instead.
    let session_uuid = value
        .id
        .as_ref()
        .ok_or(ConversionError::MissingField("id"))?;
    let id_str = prefixed_id("session", session_uuid);
    // The proto Session does not carry a workspace id yet; reconstruct it from
    // the session id under the workspace.id == session.id equality invariant
    // (see knowledge/runtime-resources/workspace.md). Revisit when shared workspaces add it to proto.
    let workspace_id_str = prefixed_id("wsp", session_uuid);
    let agent_id_str = value.agent_id.as_ref().map(|u| prefixed_id("agent", u));
    let agent_version_id_str = value
        .agent_version_id
        .as_ref()
        .map(|u| prefixed_id("agentver", u));
    let harness_id_str = value
        .harness_id
        .as_ref()
        .map(|u| prefixed_id("harness", u))
        .unwrap_or_else(|| {
            // EVE-652: a session without a harness id falls back to the nil
            // harness (preserved behavior). Log so the substitution is visible.
            tracing::warn!(
                session_id = %id_str,
                "internal-protocol: proto Session missing harness_id; using nil harness"
            );
            format!("harness_{}", uuid::Uuid::nil().simple())
        });
    let model_id_str = value
        .default_model_id
        .as_ref()
        .map(|u| prefixed_id("model", u));
    let owner_principal_id_str = value
        .owner_principal_id
        .as_ref()
        .map(|u| prefixed_id("principal", u))
        .ok_or(ConversionError::MissingField("owner_principal_id"))?;
    let parent_session_id_str = value
        .parent_session_id
        .as_ref()
        .map(|u| prefixed_id("session", u));
    // EVE-652: malformed blueprint config JSON previously vanished (None) with no
    // trace. Keep it optional but log which session carried bad config.
    let blueprint_config = value.blueprint_config_json.as_deref().and_then(|json| {
        match serde_json::from_str::<serde_json::Value>(json) {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::warn!(
                    session_id = %id_str,
                    error = %e,
                    "internal-protocol: dropping unparseable blueprint_config_json"
                );
                None
            }
        }
    });

    // EVE-652: a capability that fails to parse previously truncated the array
    // silently, weakening the session's declared capability set with no signal.
    // Surface each drop (this conversion stays lenient to avoid failing the whole
    // session on one bad entry, but the loss is no longer silent).
    let capabilities: Vec<serde_json::Value> = value
        .capabilities
        .iter()
        .filter_map(|c| match serde_json::from_str(c) {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::warn!(
                    session_id = %id_str,
                    error = %e,
                    "internal-protocol: dropping unparseable session capability"
                );
                None
            }
        })
        .collect();

    let json = serde_json::json!({
        "id": id_str,
        "workspace_id": workspace_id_str,
        "organization_id": value.organization_id,
        "harness_id": harness_id_str,
        "agent_id": agent_id_str,
        "agent_version_id": agent_version_id_str,
        "owner_principal_id": owner_principal_id_str,
        "resolved_owner_user_id": value
            .resolved_owner_user_id
            .as_ref()
            .map(|u| u.value.clone()),
        "owner": Option::<serde_json::Value>::None,
        "effective_owner": Option::<serde_json::Value>::None,
        "title": if value.title.is_empty() { None } else { Some(&value.title) },
        "goal": value.goal.clone(),
        "locale": if value.locale.is_empty() { None } else { Some(&value.locale) },
        "tags": tags,
        "model_id": model_id_str,
        "capabilities": capabilities,
        "status": value.status,
        "created_at": value.created_at.as_ref().map(|t| proto_timestamp_to_datetime(t).to_rfc3339()),
        "updated_at": value.updated_at.as_ref().map(|t| proto_timestamp_to_datetime(t).to_rfc3339()),
        "started_at": started_at,
        "finished_at": finished_at,
        "hints": value.hints.as_ref().map(proto_struct_to_json),
        "parent_session_id": parent_session_id_str,
        "blueprint_id": value.blueprint_id,
        "blueprint_config": blueprint_config,
        "parallel_tool_calls": value.parallel_tool_calls,
    });
    serde_json::from_value(json).map_err(ConversionError::from)
}

/// Convert schemas Session to proto Session
pub fn schema_session_to_proto(value: &everruns_platform::Session) -> proto::Session {
    proto::Session {
        id: Some(uuid_to_proto_uuid(value.id.uuid())),
        agent_id: value.agent_id.map(|id| uuid_to_proto_uuid(id.uuid())),
        agent_version_id: value
            .agent_version_id
            .map(|id| uuid_to_proto_uuid(id.uuid())),
        title: value.title.clone().unwrap_or_default(),
        goal: value.goal.clone(),
        locale: value.locale.clone().unwrap_or_default(),
        status: value.status.to_string(),
        created_at: Some(datetime_to_proto_timestamp(value.created_at)),
        updated_at: Some(datetime_to_proto_timestamp(value.updated_at)),
        default_model_id: value.model_id.map(|id| uuid_to_proto_uuid(id.uuid())),
        organization_id: value.organization_id.clone(),
        capabilities: value
            .capabilities
            .iter()
            .filter_map(|c| match serde_json::to_string(c) {
                Ok(s) => Some(s),
                Err(e) => {
                    // EVE-652: surface a dropped capability instead of silently
                    // shrinking the array sent to the worker.
                    tracing::error!(
                        session_id = %value.id,
                        error = %e,
                        "internal-protocol: failed to serialize session capability; dropping it"
                    );
                    None
                }
            })
            .collect(),
        harness_id: Some(uuid_to_proto_uuid(value.harness_id.uuid())),
        tags: value.tags.clone(),
        owner_principal_id: Some(uuid_to_proto_uuid(value.owner_principal_id.uuid())),
        resolved_owner_user_id: value.resolved_owner_user_id.map(uuid_to_proto_uuid),
        parent_session_id: value
            .parent_session_id
            .map(|id| uuid_to_proto_uuid(id.uuid())),
        blueprint_id: value.blueprint_id.clone(),
        blueprint_config_json: value.blueprint_config.as_ref().and_then(|config| {
            serde_json::to_string(config)
                .map_err(|e| {
                    // EVE-652: don't silently drop blueprint config on serialize error.
                    tracing::error!(
                        session_id = %value.id,
                        error = %e,
                        "internal-protocol: failed to serialize blueprint_config; omitting it"
                    );
                })
                .ok()
        }),
        hints: value.hints.as_ref().map(|h| {
            let json = serde_json::to_value(h).unwrap_or_else(|e| {
                // EVE-652: empty hints on serialize error, but make it visible.
                tracing::error!(
                    session_id = %value.id,
                    error = %e,
                    "internal-protocol: failed to serialize session hints; sending empty"
                );
                serde_json::Value::Null
            });
            json_to_proto_struct(&json)
        }),
        system_prompt: value.system_prompt.clone(),
        initial_files_json: serde_json::to_string(&value.initial_files).unwrap_or_else(|e| {
            // EVE-652: empty initial_files on serialize error, but make it visible.
            tracing::error!(
                session_id = %value.id,
                error = %e,
                "internal-protocol: failed to serialize initial_files; sending empty"
            );
            String::new()
        }),
        parallel_tool_calls: value.parallel_tool_calls,
    }
}

/// Convert proto Message to schemas Message
pub fn proto_message_to_schema(
    value: proto::Message,
) -> Result<everruns_core::RuntimeMessage, ConversionError> {
    let id = value
        .id
        .as_ref()
        .ok_or(ConversionError::MissingField("id"))?;
    let id = proto_uuid_to_uuid(id)?;
    let created_at = value
        .created_at
        .as_ref()
        .map(proto_timestamp_to_datetime)
        .ok_or(ConversionError::MissingField("created_at"))?;

    // Convert prost ListValue to Vec<ContentPart>
    let content_json = value
        .content
        .as_ref()
        .map(proto_list_to_json)
        .unwrap_or_else(|| serde_json::Value::Array(vec![]));
    let content: Vec<everruns_core::ContentPart> = serde_json::from_value(content_json)?;

    // Convert prost Struct to Controls
    let controls: Option<everruns_core::Controls> = value
        .controls
        .as_ref()
        .map(|s| serde_json::from_value(proto_struct_to_json(s)))
        .transpose()?;

    // Convert prost Struct to metadata
    let metadata: Option<std::collections::HashMap<String, serde_json::Value>> = value
        .metadata
        .as_ref()
        .map(|s| serde_json::from_value(proto_struct_to_json(s)))
        .transpose()?;

    // Convert prost Struct to ExternalActor
    let external_actor: Option<everruns_core::ExternalActor> = value
        .external_actor
        .as_ref()
        .map(|s| serde_json::from_value(proto_struct_to_json(s)))
        .transpose()?;

    let role = parse_message_role(&value.role);

    Ok(everruns_core::RuntimeMessage {
        id: id.into(),
        role,
        content,
        // Phase crosses the worker boundary: dropping it here silently
        // downgraded every message to "unclassified" before it reached the API.
        phase: value
            .phase
            .as_deref()
            .and_then(everruns_contracts::ExecutionPhase::from_provider_str),
        phase_source: value
            .phase_source
            .as_deref()
            .and_then(everruns_contracts::PhaseSource::from_str_opt),
        controls,
        metadata,
        external_actor,
        created_at,
    })
}

/// Convert schemas Message to proto Message
pub fn schema_message_to_proto(value: &everruns_core::RuntimeMessage) -> proto::Message {
    // EVE-652: serializing a message field previously fell back to an empty
    // value silently — for `content` that means dropping the entire message
    // payload (text/images/tool calls). Keep the empty fallback (infallible
    // path) but log the loss with the message id and field name.
    let serialize_field =
        |field: &'static str, result: Result<serde_json::Value, serde_json::Error>| {
            result.unwrap_or_else(|e| {
            tracing::error!(
                message_id = %value.id,
                field,
                error = %e,
                "internal-protocol: failed to serialize message field; sending empty (data lost)"
            );
            serde_json::Value::Null
        })
        };

    // Convert content to ListValue
    let content_json = serialize_field("content", serde_json::to_value(&value.content));
    let content = Some(json_to_proto_list(&content_json));

    // Convert controls to Struct
    let controls = value
        .controls
        .as_ref()
        .map(|c| json_to_proto_struct(&serialize_field("controls", serde_json::to_value(c))));

    // Convert metadata to Struct
    let metadata = value
        .metadata
        .as_ref()
        .map(|m| json_to_proto_struct(&serialize_field("metadata", serde_json::to_value(m))));

    // Convert external_actor to Struct
    let external_actor = value.external_actor.as_ref().map(|ea| {
        json_to_proto_struct(&serialize_field("external_actor", serde_json::to_value(ea)))
    });

    proto::Message {
        id: Some(uuid_to_proto_uuid(value.id.uuid())),
        role: value.role.to_string(),
        content,
        controls,
        metadata,
        created_at: Some(datetime_to_proto_timestamp(value.created_at)),
        external_actor,
        phase: value.phase.map(|phase| phase.as_provider_str().to_string()),
        phase_source: value.phase_source.map(|source| source.as_str().to_string()),
    }
}

/// Convert proto Event to schemas Event
pub fn proto_event_to_schema(value: proto::Event) -> Result<everruns_core::Event, ConversionError> {
    let id = value
        .id
        .as_ref()
        .ok_or(ConversionError::MissingField("id"))?;
    let id = proto_uuid_to_uuid(id)?;
    let ts = value
        .ts
        .as_ref()
        .map(proto_timestamp_to_datetime)
        .ok_or(ConversionError::MissingField("ts"))?;

    let proto_context = value
        .context
        .as_ref()
        .ok_or(ConversionError::MissingField("context"))?;
    let session_id = proto_context
        .session_id
        .as_ref()
        .ok_or(ConversionError::MissingField("session_id"))?;
    let session_id = proto_uuid_to_uuid(session_id)?;

    let context = everruns_core::EventContext {
        turn_id: proto_context
            .turn_id
            .as_ref()
            .map(proto_uuid_to_uuid)
            .transpose()?
            .map(TurnId::from_uuid),
        input_message_id: proto_context
            .input_message_id
            .as_ref()
            .map(proto_uuid_to_uuid)
            .transpose()?
            .map(MessageId::from_uuid),
        exec_id: proto_context
            .exec_id
            .as_ref()
            .map(proto_uuid_to_uuid)
            .transpose()?
            .map(ExecId::from_uuid),
        // OTel-style trace/span fields
        trace_id: proto_context.trace_id.clone(),
        span_id: proto_context.span_id.clone(),
        parent_span_id: proto_context.parent_span_id.clone(),
    };

    // Convert Struct data to EventData based on event_type
    let data_struct = value
        .data
        .as_ref()
        .ok_or(ConversionError::MissingField("data"))?;
    let data_json = proto_struct_to_json(data_struct);
    let data = deserialize_event_data(&value.event_type, data_json);

    // Convert optional metadata from prost Struct
    let metadata: Option<serde_json::Value> = value.metadata.as_ref().map(proto_struct_to_json);

    Ok(everruns_core::Event {
        id: EventId::from_uuid(id),
        event_type: value.event_type,
        ts,
        session_id: SessionId::from_uuid(session_id),
        context,
        data,
        metadata,
        tags: if value.tags.is_empty() {
            None
        } else {
            Some(value.tags)
        },
        sequence: Some(value.sequence),
    })
}

/// Convert schemas Event to proto Event
pub fn schema_event_to_proto(value: &everruns_core::Event) -> proto::Event {
    // Serialize EventData to JSON, then convert to Struct
    let data_json = serialize_event_data(&value.data);
    let data_struct = json_to_proto_struct(&data_json);

    proto::Event {
        id: Some(uuid_to_proto_uuid(value.id.uuid())),
        event_type: value.event_type.clone(),
        ts: Some(datetime_to_proto_timestamp(value.ts)),
        context: Some(proto::EventContext {
            session_id: Some(uuid_to_proto_uuid(value.session_id.uuid())),
            turn_id: value
                .context
                .turn_id
                .as_ref()
                .map(|id| uuid_to_proto_uuid(id.uuid())),
            input_message_id: value
                .context
                .input_message_id
                .as_ref()
                .map(|id| uuid_to_proto_uuid(id.uuid())),
            exec_id: value
                .context
                .exec_id
                .as_ref()
                .map(|id| uuid_to_proto_uuid(id.uuid())),
            // OTel-style trace/span fields
            trace_id: value.context.trace_id.clone(),
            span_id: value.context.span_id.clone(),
            parent_span_id: value.context.parent_span_id.clone(),
        }),
        data: Some(data_struct),
        metadata: value.metadata.as_ref().map(json_to_proto_struct),
        tags: value.tags.clone().unwrap_or_default(),
        sequence: value.sequence.unwrap_or(0),
    }
}

/// Convert proto EventRequest to schemas EventRequest
pub fn proto_event_request_to_schema(
    value: proto::EventRequest,
) -> Result<everruns_core::EventRequest, ConversionError> {
    let ts = value
        .ts
        .as_ref()
        .map(proto_timestamp_to_datetime)
        .ok_or(ConversionError::MissingField("ts"))?;

    let proto_context = value
        .context
        .as_ref()
        .ok_or(ConversionError::MissingField("context"))?;
    let session_id = proto_context
        .session_id
        .as_ref()
        .ok_or(ConversionError::MissingField("session_id"))?;
    let session_id = proto_uuid_to_uuid(session_id)?;

    let context = everruns_core::EventContext {
        turn_id: proto_context
            .turn_id
            .as_ref()
            .map(proto_uuid_to_uuid)
            .transpose()?
            .map(TurnId::from_uuid),
        input_message_id: proto_context
            .input_message_id
            .as_ref()
            .map(proto_uuid_to_uuid)
            .transpose()?
            .map(MessageId::from_uuid),
        exec_id: proto_context
            .exec_id
            .as_ref()
            .map(proto_uuid_to_uuid)
            .transpose()?
            .map(ExecId::from_uuid),
        // OTel-style trace/span fields
        trace_id: proto_context.trace_id.clone(),
        span_id: proto_context.span_id.clone(),
        parent_span_id: proto_context.parent_span_id.clone(),
    };

    // Convert Struct data to EventData based on event_type
    let data_struct = value
        .data
        .as_ref()
        .ok_or(ConversionError::MissingField("data"))?;
    let data_json = proto_struct_to_json(data_struct);
    let data = deserialize_event_data(&value.event_type, data_json);

    // Convert optional metadata from prost Struct
    let metadata: Option<serde_json::Value> = value.metadata.as_ref().map(proto_struct_to_json);

    Ok(everruns_core::EventRequest {
        event_type: value.event_type,
        ts,
        session_id: SessionId::from_uuid(session_id),
        context,
        data,
        metadata,
        tags: if value.tags.is_empty() {
            None
        } else {
            Some(value.tags)
        },
    })
}

/// Convert schemas EventRequest to proto EventRequest
pub fn schema_event_request_to_proto(value: &everruns_core::EventRequest) -> proto::EventRequest {
    // Serialize EventData to JSON, then convert to Struct
    let data_json = serialize_event_data(&value.data);
    let data_struct = json_to_proto_struct(&data_json);

    proto::EventRequest {
        event_type: value.event_type.clone(),
        ts: Some(datetime_to_proto_timestamp(value.ts)),
        context: Some(proto::EventContext {
            session_id: Some(uuid_to_proto_uuid(value.session_id.uuid())),
            turn_id: value
                .context
                .turn_id
                .as_ref()
                .map(|id| uuid_to_proto_uuid(id.uuid())),
            input_message_id: value
                .context
                .input_message_id
                .as_ref()
                .map(|id| uuid_to_proto_uuid(id.uuid())),
            exec_id: value
                .context
                .exec_id
                .as_ref()
                .map(|id| uuid_to_proto_uuid(id.uuid())),
            // OTel-style trace/span fields
            trace_id: value.context.trace_id.clone(),
            span_id: value.context.span_id.clone(),
            parent_span_id: value.context.parent_span_id.clone(),
        }),
        data: Some(data_struct),
        metadata: value.metadata.as_ref().map(json_to_proto_struct),
        tags: value.tags.clone().unwrap_or_default(),
    }
}

/// Convert proto SessionFile to schemas SessionFile
pub fn proto_session_file_to_schema(
    value: proto::SessionFile,
) -> Result<everruns_core::SessionFile, ConversionError> {
    let id = value
        .id
        .as_ref()
        .ok_or(ConversionError::MissingField("id"))?;
    let id = proto_uuid_to_uuid(id)?;
    let session_id = value
        .session_id
        .as_ref()
        .ok_or(ConversionError::MissingField("session_id"))?;
    let session_id = proto_uuid_to_uuid(session_id)?;
    let created_at = value
        .created_at
        .as_ref()
        .map(proto_timestamp_to_datetime)
        .ok_or(ConversionError::MissingField("created_at"))?;
    let updated_at = value
        .updated_at
        .as_ref()
        .map(proto_timestamp_to_datetime)
        .ok_or(ConversionError::MissingField("updated_at"))?;

    Ok(everruns_core::SessionFile {
        id,
        session_id,
        path: value.path,
        name: value.name,
        content: value.content,
        encoding: value.encoding,
        is_directory: value.is_directory,
        is_readonly: value.is_readonly,
        size_bytes: value.size_bytes,
        created_at,
        updated_at,
    })
}

/// Convert schemas SessionFile to proto SessionFile
pub fn schema_session_file_to_proto(value: &everruns_core::SessionFile) -> proto::SessionFile {
    proto::SessionFile {
        id: Some(uuid_to_proto_uuid(value.id)),
        session_id: Some(uuid_to_proto_uuid(value.session_id)),
        path: value.path.clone(),
        name: value.name.clone(),
        content: value.content.clone(),
        encoding: value.encoding.clone(),
        is_directory: value.is_directory,
        is_readonly: value.is_readonly,
        size_bytes: value.size_bytes,
        created_at: Some(datetime_to_proto_timestamp(value.created_at)),
        updated_at: Some(datetime_to_proto_timestamp(value.updated_at)),
    }
}

/// Convert proto FileInfo to schemas FileInfo
pub fn proto_file_info_to_schema(
    value: proto::FileInfo,
) -> Result<everruns_core::FileInfo, ConversionError> {
    let id = value
        .id
        .as_ref()
        .ok_or(ConversionError::MissingField("id"))?;
    let id = proto_uuid_to_uuid(id)?;
    let session_id = value
        .session_id
        .as_ref()
        .ok_or(ConversionError::MissingField("session_id"))?;
    let session_id = proto_uuid_to_uuid(session_id)?;
    let created_at = value
        .created_at
        .as_ref()
        .map(proto_timestamp_to_datetime)
        .ok_or(ConversionError::MissingField("created_at"))?;
    let updated_at = value
        .updated_at
        .as_ref()
        .map(proto_timestamp_to_datetime)
        .ok_or(ConversionError::MissingField("updated_at"))?;

    Ok(everruns_core::FileInfo {
        id,
        session_id,
        path: value.path,
        name: value.name,
        is_directory: value.is_directory,
        is_readonly: value.is_readonly,
        size_bytes: value.size_bytes,
        created_at,
        updated_at,
    })
}

/// Convert schemas FileInfo to proto FileInfo
pub fn schema_file_info_to_proto(value: &everruns_core::FileInfo) -> proto::FileInfo {
    proto::FileInfo {
        id: Some(uuid_to_proto_uuid(value.id)),
        session_id: Some(uuid_to_proto_uuid(value.session_id)),
        path: value.path.clone(),
        name: value.name.clone(),
        is_directory: value.is_directory,
        is_readonly: value.is_readonly,
        size_bytes: value.size_bytes,
        created_at: Some(datetime_to_proto_timestamp(value.created_at)),
        updated_at: Some(datetime_to_proto_timestamp(value.updated_at)),
    }
}

/// Convert proto FileStat to schemas FileStat
pub fn proto_file_stat_to_schema(
    value: proto::FileStat,
) -> Result<everruns_core::FileStat, ConversionError> {
    let created_at = value
        .created_at
        .as_ref()
        .map(proto_timestamp_to_datetime)
        .ok_or(ConversionError::MissingField("created_at"))?;
    let updated_at = value
        .updated_at
        .as_ref()
        .map(proto_timestamp_to_datetime)
        .ok_or(ConversionError::MissingField("updated_at"))?;

    Ok(everruns_core::FileStat {
        path: value.path,
        name: value.name,
        is_directory: value.is_directory,
        is_readonly: value.is_readonly,
        size_bytes: value.size_bytes,
        created_at,
        updated_at,
    })
}

/// Convert schemas FileStat to proto FileStat
pub fn schema_file_stat_to_proto(value: &everruns_core::FileStat) -> proto::FileStat {
    proto::FileStat {
        path: value.path.clone(),
        name: value.name.clone(),
        is_directory: value.is_directory,
        is_readonly: value.is_readonly,
        size_bytes: value.size_bytes,
        created_at: Some(datetime_to_proto_timestamp(value.created_at)),
        updated_at: Some(datetime_to_proto_timestamp(value.updated_at)),
    }
}

/// Convert proto GrepMatch to schemas GrepMatch
pub fn proto_grep_match_to_schema(value: proto::GrepMatch) -> everruns_core::GrepMatch {
    everruns_core::GrepMatch {
        path: value.path,
        line_number: value.line_number as usize,
        line: value.line,
    }
}

/// Convert schemas GrepMatch to proto GrepMatch
pub fn schema_grep_match_to_proto(value: &everruns_core::GrepMatch) -> proto::GrepMatch {
    proto::GrepMatch {
        path: value.path.clone(),
        line_number: value.line_number as u64,
        line: value.line.clone(),
    }
}

// ============================================================================
// Session task registry conversions (EVE-642)
// ============================================================================
//
// Native protobuf conversions for the session-task RPC payloads. These replace
// the previous JSON-in-protobuf byte fields so the data is serialized once by
// protobuf framing and consumed directly on the worker. everruns-core remains
// the source of truth for lifecycle invariants (apply_task_update etc.).

use everruns_core::session_task as st;

fn session_task_state_to_proto(state: st::SessionTaskState) -> proto::SessionTaskState {
    match state {
        st::SessionTaskState::Queued => proto::SessionTaskState::Queued,
        st::SessionTaskState::Running => proto::SessionTaskState::Running,
        st::SessionTaskState::AwaitingInput => proto::SessionTaskState::AwaitingInput,
        st::SessionTaskState::Succeeded => proto::SessionTaskState::Succeeded,
        st::SessionTaskState::Failed => proto::SessionTaskState::Failed,
        st::SessionTaskState::Canceled => proto::SessionTaskState::Canceled,
    }
}

fn proto_to_session_task_state(state: proto::SessionTaskState) -> st::SessionTaskState {
    match state {
        // Unspecified defaults to Queued to match `From<&str>` on the domain enum.
        proto::SessionTaskState::Unspecified | proto::SessionTaskState::Queued => {
            st::SessionTaskState::Queued
        }
        proto::SessionTaskState::Running => st::SessionTaskState::Running,
        proto::SessionTaskState::AwaitingInput => st::SessionTaskState::AwaitingInput,
        proto::SessionTaskState::Succeeded => st::SessionTaskState::Succeeded,
        proto::SessionTaskState::Failed => st::SessionTaskState::Failed,
        proto::SessionTaskState::Canceled => st::SessionTaskState::Canceled,
    }
}

fn wake_policy_to_proto(policy: st::TaskWakePolicy) -> proto::TaskWakePolicy {
    match policy {
        st::TaskWakePolicy::Silent => proto::TaskWakePolicy::Silent,
        st::TaskWakePolicy::OnTerminal => proto::TaskWakePolicy::OnTerminal,
        st::TaskWakePolicy::OnActivity => proto::TaskWakePolicy::OnActivity,
    }
}

fn proto_to_wake_policy(policy: proto::TaskWakePolicy) -> st::TaskWakePolicy {
    match policy {
        proto::TaskWakePolicy::Unspecified | proto::TaskWakePolicy::Silent => {
            st::TaskWakePolicy::Silent
        }
        proto::TaskWakePolicy::OnTerminal => st::TaskWakePolicy::OnTerminal,
        proto::TaskWakePolicy::OnActivity => st::TaskWakePolicy::OnActivity,
    }
}

fn direction_to_proto(direction: st::TaskMessageDirection) -> proto::TaskMessageDirection {
    match direction {
        st::TaskMessageDirection::Inbound => proto::TaskMessageDirection::Inbound,
        st::TaskMessageDirection::Outbound => proto::TaskMessageDirection::Outbound,
    }
}

fn proto_to_direction(direction: proto::TaskMessageDirection) -> st::TaskMessageDirection {
    match direction {
        // Unspecified defaults to Inbound to match `From<&str>` on the domain enum.
        proto::TaskMessageDirection::Unspecified | proto::TaskMessageDirection::Inbound => {
            st::TaskMessageDirection::Inbound
        }
        proto::TaskMessageDirection::Outbound => st::TaskMessageDirection::Outbound,
    }
}

fn progress_to_proto(p: &st::TaskProgress) -> proto::TaskProgressProto {
    proto::TaskProgressProto {
        current: p.current,
        total: p.total,
        unit: p.unit.clone(),
        label: p.label.clone(),
    }
}

fn proto_to_progress(p: proto::TaskProgressProto) -> st::TaskProgress {
    st::TaskProgress {
        current: p.current,
        total: p.total,
        unit: p.unit,
        label: p.label,
    }
}

// Free-form JSON fields (spec, expected, message Data) carry canonical
// serde_json bytes: they are already `serde_json::Value`s, so serializing them
// exactly once is cheaper than walking them into a google.protobuf.Value tree
// (see benches/session_task_encoding.rs). Serialization of an in-memory Value
// does not fail in practice; a hit on the fallback signals upstream corruption
// and is logged rather than silently dropped.
fn json_value_to_bytes(value: &serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap_or_else(|e| {
        tracing::error!(error = %e, "internal-protocol: failed to encode free-form JSON; emitting null");
        b"null".to_vec()
    })
}

/// Decode canonical JSON bytes back into a Value. Empty bytes decode to Null so
/// an absent proto `bytes` field maps to `Value::Null` (the schema default).
fn bytes_to_json_value(bytes: &[u8]) -> serde_json::Value {
    if bytes.is_empty() {
        return serde_json::Value::Null;
    }
    serde_json::from_slice(bytes).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "internal-protocol: invalid free-form JSON bytes; using null");
        serde_json::Value::Null
    })
}

fn input_request_to_proto(r: &st::TaskInputRequest) -> proto::TaskInputRequestProto {
    proto::TaskInputRequestProto {
        id: r.id.clone(),
        prompt: r.prompt.clone(),
        // None -> empty bytes; Some(v) -> canonical JSON bytes.
        expected_json: r
            .expected
            .as_ref()
            .map(json_value_to_bytes)
            .unwrap_or_default(),
    }
}

fn proto_to_input_request(r: proto::TaskInputRequestProto) -> st::TaskInputRequest {
    st::TaskInputRequest {
        id: r.id,
        prompt: r.prompt,
        expected: if r.expected_json.is_empty() {
            None
        } else {
            Some(bytes_to_json_value(&r.expected_json))
        },
    }
}

fn task_error_to_proto(e: &st::TaskError) -> proto::TaskErrorProto {
    proto::TaskErrorProto {
        kind: e.kind.clone(),
        message: e.message.clone(),
    }
}

fn proto_to_task_error(e: proto::TaskErrorProto) -> st::TaskError {
    st::TaskError {
        kind: e.kind,
        message: e.message,
    }
}

fn artifact_to_proto(a: &st::TaskArtifact) -> proto::TaskArtifactProto {
    proto::TaskArtifactProto {
        name: a.name.clone(),
        artifact_type: a.artifact_type.clone(),
        path: a.path.clone(),
        url: a.url.clone(),
    }
}

fn proto_to_artifact(a: proto::TaskArtifactProto) -> st::TaskArtifact {
    st::TaskArtifact {
        name: a.name,
        artifact_type: a.artifact_type,
        path: a.path,
        url: a.url,
    }
}

fn links_to_proto(l: &st::TaskLinks) -> proto::TaskLinksProto {
    proto::TaskLinksProto {
        child_session_id: l.child_session_id.map(|id| uuid_to_proto_uuid(id.uuid())),
        remote_task_id: l.remote_task_id.clone(),
        resource_ids: l.resource_ids.clone(),
    }
}

fn proto_to_links(l: proto::TaskLinksProto) -> st::TaskLinks {
    st::TaskLinks {
        child_session_id: l
            .child_session_id
            .map(|u| SessionId::from_uuid(parse_uuid_lenient(&u.value))),
        remote_task_id: l.remote_task_id,
        resource_ids: l.resource_ids,
    }
}

fn message_part_to_proto(part: &st::TaskMessagePart) -> proto::TaskMessagePartProto {
    use proto::task_message_part_proto::Part;
    let part = match part {
        st::TaskMessagePart::Text { text } => Part::Text(text.clone()),
        st::TaskMessagePart::Data { data } => Part::DataJson(json_value_to_bytes(data)),
    };
    proto::TaskMessagePartProto { part: Some(part) }
}

fn proto_to_message_part(part: proto::TaskMessagePartProto) -> st::TaskMessagePart {
    use proto::task_message_part_proto::Part;
    match part.part {
        Some(Part::Text(text)) => st::TaskMessagePart::Text { text },
        Some(Part::DataJson(data)) => st::TaskMessagePart::Data {
            data: bytes_to_json_value(&data),
        },
        // A part with no variant set is malformed; represent it as empty text
        // rather than dropping the message. Logged so the loss is visible.
        None => {
            tracing::warn!(
                "internal-protocol: task message part missing variant; using empty text"
            );
            st::TaskMessagePart::Text {
                text: String::new(),
            }
        }
    }
}

/// Parse a raw UUID string leniently (session ids are validated upstream).
fn parse_uuid_lenient(value: &str) -> uuid::Uuid {
    uuid::Uuid::parse_str(value).unwrap_or_else(|e| {
        tracing::warn!(value = %value, error = %e, "internal-protocol: invalid session UUID; using nil");
        uuid::Uuid::nil()
    })
}

/// Convert a domain `SessionTask` to its native proto message.
pub fn session_task_to_proto(task: &st::SessionTask) -> proto::SessionTaskProto {
    proto::SessionTaskProto {
        id: task.id.clone(),
        session_id: Some(uuid_to_proto_uuid(task.session_id.uuid())),
        kind: task.kind.clone(),
        display_name: task.display_name.clone(),
        spec_json: json_value_to_bytes(&task.spec),
        state: session_task_state_to_proto(task.state) as i32,
        state_detail: task.state_detail.clone(),
        progress: task.progress.as_ref().map(progress_to_proto),
        input_request: task.input_request.as_ref().map(input_request_to_proto),
        cancel_requested_at: task.cancel_requested_at.map(datetime_to_proto_timestamp),
        summary: task.summary.clone(),
        result_path: task.result_path.clone(),
        artifacts: task.artifacts.iter().map(artifact_to_proto).collect(),
        error: task.error.as_ref().map(task_error_to_proto),
        attempt: task.attempt,
        worker_id: task.worker_id.clone(),
        heartbeat_at: task.heartbeat_at.map(datetime_to_proto_timestamp),
        links: Some(links_to_proto(&task.links)),
        wake_policy: wake_policy_to_proto(task.wake_policy) as i32,
        created_at: Some(datetime_to_proto_timestamp(task.created_at)),
        started_at: task.started_at.map(datetime_to_proto_timestamp),
        finished_at: task.finished_at.map(datetime_to_proto_timestamp),
        updated_at: Some(datetime_to_proto_timestamp(task.updated_at)),
    }
}

/// Convert a native proto `SessionTaskProto` back to the domain struct.
pub fn proto_to_session_task(
    p: proto::SessionTaskProto,
) -> Result<st::SessionTask, ConversionError> {
    // Capture enum accessors before moving owned fields out of `p`.
    let state = proto_to_session_task_state(p.state());
    let wake_policy = proto_to_wake_policy(p.wake_policy());
    let session_uuid = p
        .session_id
        .ok_or(ConversionError::MissingField("session_id"))?;
    Ok(st::SessionTask {
        id: p.id,
        session_id: SessionId::from_uuid(parse_uuid_lenient(&session_uuid.value)),
        // Storage-derived (EVE-680); surfaced on API storage reads via
        // `SessionTaskRow::to_task`, not carried over the worker protocol.
        root_session_id: None,
        kind: p.kind,
        display_name: p.display_name,
        spec: bytes_to_json_value(&p.spec_json),
        state,
        state_detail: p.state_detail,
        progress: p.progress.map(proto_to_progress),
        input_request: p.input_request.map(proto_to_input_request),
        cancel_requested_at: p
            .cancel_requested_at
            .as_ref()
            .map(proto_timestamp_to_datetime),
        summary: p.summary,
        result_path: p.result_path,
        artifacts: p.artifacts.into_iter().map(proto_to_artifact).collect(),
        error: p.error.map(proto_to_task_error),
        attempt: p.attempt,
        worker_id: p.worker_id,
        heartbeat_at: p.heartbeat_at.as_ref().map(proto_timestamp_to_datetime),
        links: p.links.map(proto_to_links).unwrap_or_default(),
        wake_policy,
        created_at: p
            .created_at
            .as_ref()
            .map(proto_timestamp_to_datetime)
            .ok_or(ConversionError::MissingField("created_at"))?,
        started_at: p.started_at.as_ref().map(proto_timestamp_to_datetime),
        finished_at: p.finished_at.as_ref().map(proto_timestamp_to_datetime),
        updated_at: p
            .updated_at
            .as_ref()
            .map(proto_timestamp_to_datetime)
            .ok_or(ConversionError::MissingField("updated_at"))?,
    })
}

/// Convert a domain `CreateSessionTask` to its native proto message.
pub fn create_session_task_to_proto(
    input: &st::CreateSessionTask,
) -> proto::CreateSessionTaskProto {
    proto::CreateSessionTaskProto {
        session_id: Some(uuid_to_proto_uuid(input.session_id.uuid())),
        id: input.id.clone(),
        kind: input.kind.clone(),
        display_name: input.display_name.clone(),
        spec_json: json_value_to_bytes(&input.spec),
        state: session_task_state_to_proto(input.state) as i32,
        links: Some(links_to_proto(&input.links)),
        wake_policy: wake_policy_to_proto(input.wake_policy) as i32,
    }
}

/// Convert a native proto `CreateSessionTaskProto` back to the domain struct.
pub fn proto_to_create_session_task(
    p: proto::CreateSessionTaskProto,
) -> Result<st::CreateSessionTask, ConversionError> {
    // Capture enum accessors before moving owned fields out of `p`.
    let state = proto_to_session_task_state(p.state());
    let wake_policy = proto_to_wake_policy(p.wake_policy());
    let session_uuid = p
        .session_id
        .ok_or(ConversionError::MissingField("session_id"))?;
    Ok(st::CreateSessionTask {
        session_id: SessionId::from_uuid(parse_uuid_lenient(&session_uuid.value)),
        id: p.id,
        kind: p.kind,
        display_name: p.display_name,
        spec: bytes_to_json_value(&p.spec_json),
        state,
        links: p.links.map(proto_to_links).unwrap_or_default(),
        wake_policy,
    })
}

/// Convert a domain `SessionTaskUpdate` to its native proto message.
pub fn session_task_update_to_proto(u: &st::SessionTaskUpdate) -> proto::SessionTaskUpdateProto {
    proto::SessionTaskUpdateProto {
        state: u.state.map(|s| session_task_state_to_proto(s) as i32),
        state_detail: u.state_detail.clone(),
        progress: u.progress.as_ref().map(progress_to_proto),
        input_request: u.input_request.as_ref().map(input_request_to_proto),
        summary: u.summary.clone(),
        result_path: u.result_path.clone(),
        artifacts: u
            .artifacts
            .as_ref()
            .map(|list| proto::TaskArtifactListProto {
                artifacts: list.iter().map(artifact_to_proto).collect(),
            }),
        error: u.error.as_ref().map(task_error_to_proto),
        links: u.links.as_ref().map(links_to_proto),
        worker_id: u.worker_id.clone(),
        heartbeat_at: u.heartbeat_at.map(datetime_to_proto_timestamp),
        expected_attempt: u.expected_attempt,
        increment_attempt: u.increment_attempt,
        append_artifact: u.append_artifact.as_ref().map(artifact_to_proto),
    }
}

/// Convert a native proto `SessionTaskUpdateProto` back to the domain struct.
pub fn proto_to_session_task_update(p: proto::SessionTaskUpdateProto) -> st::SessionTaskUpdate {
    st::SessionTaskUpdate {
        // `state` is an enum field wrapped in optional; decode the raw i32 only
        // when present so an absent update leaves the field unchanged.
        state: p.state.map(|s| {
            proto_to_session_task_state(
                proto::SessionTaskState::try_from(s)
                    .unwrap_or(proto::SessionTaskState::Unspecified),
            )
        }),
        state_detail: p.state_detail,
        progress: p.progress.map(proto_to_progress),
        input_request: p.input_request.map(proto_to_input_request),
        summary: p.summary,
        result_path: p.result_path,
        artifacts: p
            .artifacts
            .map(|list| list.artifacts.into_iter().map(proto_to_artifact).collect()),
        error: p.error.map(proto_to_task_error),
        links: p.links.map(proto_to_links),
        worker_id: p.worker_id,
        heartbeat_at: p.heartbeat_at.as_ref().map(proto_timestamp_to_datetime),
        expected_attempt: p.expected_attempt,
        increment_attempt: p.increment_attempt,
        append_artifact: p.append_artifact.map(proto_to_artifact),
    }
}

/// Convert a domain `TaskMessage` to its native proto message.
pub fn task_message_to_proto(m: &st::TaskMessage) -> proto::TaskMessageProto {
    proto::TaskMessageProto {
        id: m.id.clone(),
        task_id: m.task_id.clone(),
        direction: direction_to_proto(m.direction) as i32,
        content: m.content.iter().map(message_part_to_proto).collect(),
        in_reply_to: m.in_reply_to.clone(),
        created_at: Some(datetime_to_proto_timestamp(m.created_at)),
    }
}

/// Convert a native proto `TaskMessageProto` back to the domain struct.
pub fn proto_to_task_message(
    p: proto::TaskMessageProto,
) -> Result<st::TaskMessage, ConversionError> {
    let direction = p.direction();
    Ok(st::TaskMessage {
        id: p.id,
        task_id: p.task_id,
        direction: proto_to_direction(direction),
        content: p.content.into_iter().map(proto_to_message_part).collect(),
        in_reply_to: p.in_reply_to,
        created_at: p
            .created_at
            .as_ref()
            .map(proto_timestamp_to_datetime)
            .ok_or(ConversionError::MissingField("created_at"))?,
    })
}

/// Convert a domain `NewTaskMessage` to its native proto message.
pub fn new_task_message_to_proto(m: &st::NewTaskMessage) -> proto::NewTaskMessageProto {
    proto::NewTaskMessageProto {
        direction: direction_to_proto(m.direction) as i32,
        content: m.content.iter().map(message_part_to_proto).collect(),
        in_reply_to: m.in_reply_to.clone(),
        expected_attempt: m.expected_attempt,
    }
}

/// Convert a native proto `NewTaskMessageProto` back to the domain struct.
pub fn proto_to_new_task_message(p: proto::NewTaskMessageProto) -> st::NewTaskMessage {
    let direction = p.direction();
    st::NewTaskMessage {
        direction: proto_to_direction(direction),
        content: p.content.into_iter().map(proto_to_message_part).collect(),
        in_reply_to: p.in_reply_to,
        expected_attempt: p.expected_attempt,
    }
}

// ============================================================================
// Helper functions
// ============================================================================

fn parse_message_role(s: &str) -> everruns_core::RuntimeMessageRole {
    match s.to_lowercase().as_str() {
        "system" => everruns_core::RuntimeMessageRole::System,
        "user" => everruns_core::RuntimeMessageRole::User,
        "assistant" | "agent" => everruns_core::RuntimeMessageRole::Agent,
        "tool_result" => everruns_core::RuntimeMessageRole::ToolResult,
        _ => {
            // EVE-652: an unrecognized role used to be silently coerced to `User`,
            // which can mislabel provenance (e.g. an assistant message rendered as
            // a user turn). Keep the safe default but surface the coercion.
            tracing::warn!(role = %s, "internal-protocol: unknown message role; defaulting to User");
            everruns_core::RuntimeMessageRole::User
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[path = "conversion_tests.rs"]
mod tests;
