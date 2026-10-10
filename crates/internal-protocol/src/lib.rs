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
pub use capability_wire::encode_configs as encode_capability_configs;
mod credential_fingerprint;
mod json_wire;
mod slack_action_wire;

// The published capability consumes the same neutral action identity as the wire.
use chrono::{DateTime, TimeZone, Utc};
pub use everruns_contracts::slack_action;
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
pub fn prefixed_id(prefix: &str, value: &proto::Uuid) -> String {
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
        EventData::ConversationMessage(d) => to_json(d),
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
        EventData::ToolNestedCall(d) => to_json(d),
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
        EventData::SandboxInstanceLost(d) | EventData::SandboxRecovered(d) => to_json(d),
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
        EventData::VoiceOutputInterrupted(d) => to_json(d),
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
