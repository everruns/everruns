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

use chrono::{DateTime, TimeZone, Utc};
use everruns_provider::typed_id::{EventId, ExecId, MessageId, SessionId, TurnId};
use prost_types::{ListValue, Struct, Value, value::Kind};

// Generated protobuf code
pub mod proto {
    tonic::include_proto!("everruns.internal");
}

// Re-export for convenience
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
// Conversion between prost_types and serde_json
// ============================================================================

/// Convert prost_types::Value to serde_json::Value
///
/// Note: Proto Struct's NumberValue is always f64, but we preserve integer
/// types when possible to ensure correct deserialization into u32/u64 fields.
pub fn proto_value_to_json(value: &Value) -> serde_json::Value {
    match &value.kind {
        Some(Kind::NullValue(_)) => serde_json::Value::Null,
        Some(Kind::BoolValue(b)) => serde_json::Value::Bool(*b),
        Some(Kind::NumberValue(n)) => {
            // Check if the number is a whole number that can be represented as an integer.
            // This is important because proto Struct's NumberValue is always f64,
            // but many Rust structs have u32/u64 fields that can't deserialize floats.
            if n.fract() == 0.0 {
                // Try to convert to i64 first (handles negative integers and most cases)
                if *n >= i64::MIN as f64 && *n <= i64::MAX as f64 {
                    return serde_json::Value::Number(serde_json::Number::from(*n as i64));
                }
                // For very large positive integers, try u64
                if *n >= 0.0 && *n <= u64::MAX as f64 {
                    return serde_json::Value::Number(serde_json::Number::from(*n as u64));
                }
            }
            // Fall back to f64 for actual floating point numbers
            serde_json::Number::from_f64(*n)
                .map(serde_json::Value::Number)
                .unwrap_or_else(|| {
                    // EVE-652: NaN/Infinity has no JSON number representation.
                    // It becomes null (unchanged), but the loss is now visible.
                    tracing::warn!(
                        value = *n,
                        "internal-protocol: non-finite number has no JSON representation; emitting null"
                    );
                    serde_json::Value::Null
                })
        }
        Some(Kind::StringValue(s)) => serde_json::Value::String(s.clone()),
        Some(Kind::ListValue(list)) => proto_list_to_json(list),
        Some(Kind::StructValue(s)) => proto_struct_to_json(s),
        None => serde_json::Value::Null,
    }
}

/// Convert prost_types::ListValue to serde_json::Value (array)
pub fn proto_list_to_json(list: &ListValue) -> serde_json::Value {
    serde_json::Value::Array(list.values.iter().map(proto_value_to_json).collect())
}

/// Convert prost_types::Struct to serde_json::Value (object)
pub fn proto_struct_to_json(s: &Struct) -> serde_json::Value {
    serde_json::Value::Object(
        s.fields
            .iter()
            .map(|(k, v)| (k.clone(), proto_value_to_json(v)))
            .collect(),
    )
}

/// Convert serde_json::Value to prost_types::Value
pub fn json_to_proto_value(value: &serde_json::Value) -> Value {
    Value {
        kind: Some(match value {
            serde_json::Value::Null => Kind::NullValue(0),
            serde_json::Value::Bool(b) => Kind::BoolValue(*b),
            serde_json::Value::Number(n) => Kind::NumberValue(n.as_f64().unwrap_or_else(|| {
                // EVE-652: a JSON number outside f64 range previously became 0.0
                // silently. Keep the fallback but surface the loss.
                tracing::warn!(
                    number = %n,
                    "internal-protocol: JSON number not representable as f64; using 0.0"
                );
                0.0
            })),
            serde_json::Value::String(s) => Kind::StringValue(s.clone()),
            serde_json::Value::Array(arr) => Kind::ListValue(json_array_to_proto_list(arr)),
            serde_json::Value::Object(obj) => Kind::StructValue(json_object_to_proto_struct(obj)),
        }),
    }
}

/// Convert JSON array to prost_types::ListValue
pub fn json_array_to_proto_list(arr: &[serde_json::Value]) -> ListValue {
    ListValue {
        values: arr.iter().map(json_to_proto_value).collect(),
    }
}

/// Convert JSON object to prost_types::Struct
pub fn json_object_to_proto_struct(obj: &serde_json::Map<String, serde_json::Value>) -> Struct {
    Struct {
        fields: obj
            .iter()
            .map(|(k, v)| (k.clone(), json_to_proto_value(v)))
            .collect(),
    }
}

/// Convert serde_json::Value to prost_types::ListValue (assumes array)
pub fn json_to_proto_list(value: &serde_json::Value) -> ListValue {
    match value {
        serde_json::Value::Array(arr) => json_array_to_proto_list(arr),
        _ => ListValue { values: vec![] },
    }
}

/// Convert serde_json::Value to prost_types::Struct (assumes object)
pub fn json_to_proto_struct(value: &serde_json::Value) -> Struct {
    match value {
        serde_json::Value::Object(obj) => json_object_to_proto_struct(obj),
        _ => Struct {
            fields: std::collections::BTreeMap::new(),
        },
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
/// Note: Unsupported events should not reach serialization - they are filtered earlier.
fn serialize_event_data(data: &everruns_core::EventData) -> serde_json::Value {
    use everruns_core::EventData;

    match data {
        EventData::InputMessage(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::OutputMessageStarted(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::OutputMessageDelta(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::OutputMessageReplaced(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::OutputMessageCompleted(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::TurnStarted(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::TurnCompleted(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::TurnFailed(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::TurnSealed(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::TurnCancelled(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::ReasonStarted(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::ReasonCompleted(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::ReasonRecovered(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::CapabilityUsage(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::ActStarted(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::ActCompleted(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::ToolStarted(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::ToolCompleted(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::ToolProgress(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::ToolOutputDelta(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::ToolCallRequested(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::LlmGeneration(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::ReasonThinkingStarted(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::ReasonThinkingDelta(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::ReasonThinkingCompleted(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::ReasonItem(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::SessionStarted(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::SessionActivated(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::SessionIdled(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::SessionTitleUpdated(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::TaskCreated(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::TaskUpdated(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::TaskMessageSent(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::TaskMessageReceived(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::ContextCompacting(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::ContextCompacted(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::BudgetWarning(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::BudgetPaused(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::BudgetExhausted(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::BudgetResumed(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::FileWritten(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::VoiceSessionStarted(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::VoiceInputTranscriptDelta(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::VoiceInputTranscriptCompleted(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::VoiceOutputTranscriptDelta(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::VoiceOutputTranscriptCompleted(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::VoiceSessionEnded(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::VoiceSessionFailed(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::TranscriptRepaired(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
        EventData::ToolCallRepaired(d) => {
            serde_json::to_value(d).unwrap_or_else(event_data_serialize_fallback)
        }
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
        capabilities: value
            .capabilities
            .iter()
            .map(|config| serde_json::to_string(config).expect("capability config serializes"))
            .collect(),
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
        capabilities: value
            .capabilities
            .iter()
            .map(|config| serde_json::to_string(config).expect("capability config serializes"))
            .collect(),
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
) -> Result<everruns_core::Message, ConversionError> {
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

    Ok(everruns_core::Message {
        id: id.into(),
        role,
        content,
        // Phase crosses the worker boundary: dropping it here silently
        // downgraded every message to "unclassified" before it reached the API.
        phase: value
            .phase
            .as_deref()
            .and_then(everruns_provider::ExecutionPhase::from_provider_str),
        phase_source: value
            .phase_source
            .as_deref()
            .and_then(everruns_provider::PhaseSource::from_str_opt),
        controls,
        metadata,
        external_actor,
        created_at,
    })
}

/// Convert schemas Message to proto Message
pub fn schema_message_to_proto(value: &everruns_core::Message) -> proto::Message {
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
        phase: value
            .phase
            .map(|phase| phase.as_provider_str().to_string()),
        phase_source: value
            .phase_source
            .map(|source| source.as_str().to_string()),
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

fn parse_message_role(s: &str) -> everruns_core::MessageRole {
    match s.to_lowercase().as_str() {
        "system" => everruns_core::MessageRole::System,
        "user" => everruns_core::MessageRole::User,
        "assistant" | "agent" => everruns_core::MessageRole::Agent,
        "tool_result" => everruns_core::MessageRole::ToolResult,
        _ => {
            // EVE-652: an unrecognized role used to be silently coerced to `User`,
            // which can mislabel provenance (e.g. an assistant message rendered as
            // a user turn). Keep the safe default but surface the coercion.
            tracing::warn!(role = %s, "internal-protocol: unknown message role; defaulting to User");
            everruns_core::MessageRole::User
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_proto_value_to_json_preserves_integers() {
        // Integer zero
        let value = Value {
            kind: Some(Kind::NumberValue(0.0)),
        };
        let json = proto_value_to_json(&value);
        assert!(json.is_number());
        assert_eq!(json.as_i64(), Some(0));
        // Verify it's an integer, not a float (important for serde deserialization)
        let json_str = serde_json::to_string(&json).unwrap();
        assert_eq!(json_str, "0", "Should serialize as integer, not 0.0");

        // Positive integer
        let value = Value {
            kind: Some(Kind::NumberValue(42.0)),
        };
        let json = proto_value_to_json(&value);
        assert_eq!(json.as_i64(), Some(42));
        let json_str = serde_json::to_string(&json).unwrap();
        assert_eq!(json_str, "42");

        // Negative integer
        let value = Value {
            kind: Some(Kind::NumberValue(-100.0)),
        };
        let json = proto_value_to_json(&value);
        assert_eq!(json.as_i64(), Some(-100));

        // Large u64 value (like duration_ms)
        let value = Value {
            kind: Some(Kind::NumberValue(5314.0)),
        };
        let json = proto_value_to_json(&value);
        assert_eq!(json.as_u64(), Some(5314));
    }

    #[test]
    fn test_proto_value_to_json_preserves_floats() {
        // Actual float with fractional part
        let value = Value {
            kind: Some(Kind::NumberValue(1.5)),
        };
        let json = proto_value_to_json(&value);
        assert!(json.is_f64());
        assert!((json.as_f64().unwrap() - 1.5).abs() < f64::EPSILON);

        // Negative float
        let value = Value {
            kind: Some(Kind::NumberValue(-2.5)),
        };
        let json = proto_value_to_json(&value);
        assert!(json.is_f64());
        assert!((json.as_f64().unwrap() - (-2.5)).abs() < f64::EPSILON);
    }

    #[test]
    fn test_proto_struct_roundtrip_with_integers() {
        // Test that integers survive a JSON -> Proto -> JSON roundtrip
        let original = serde_json::json!({
            "tool_call_count": 2,
            "success_count": 5,
            "duration_ms": 5314
        });

        // Convert to proto struct
        let proto_struct = json_to_proto_struct(&original);

        // Convert back to JSON
        let result = proto_struct_to_json(&proto_struct);

        // Verify integers are preserved
        assert_eq!(result["tool_call_count"].as_u64(), Some(2));
        assert_eq!(result["success_count"].as_u64(), Some(5));
        assert_eq!(result["duration_ms"].as_u64(), Some(5314));

        // Most importantly: verify they can deserialize into u32/u64
        #[derive(serde::Deserialize)]
        struct TestStruct {
            tool_call_count: u32,
            success_count: u32,
            duration_ms: u64,
        }

        let deserialized: TestStruct = serde_json::from_value(result).unwrap();
        assert_eq!(deserialized.tool_call_count, 2);
        assert_eq!(deserialized.success_count, 5);
        assert_eq!(deserialized.duration_ms, 5314);
    }

    #[test]
    fn test_reason_completed_data_roundtrip() {
        use everruns_core::events::ReasonCompletedData;

        // Create test data
        let data = ReasonCompletedData::success("Test response", true, 3, Some(2000), None);

        // Serialize to JSON
        let json = serde_json::to_value(&data).unwrap();

        // Convert to proto struct and back
        let proto_struct = json_to_proto_struct(&json);
        let result_json = proto_struct_to_json(&proto_struct);

        // Deserialize back to ReasonCompletedData
        let result: ReasonCompletedData = serde_json::from_value(result_json).unwrap();

        assert!(result.success);
        assert_eq!(result.tool_call_count, 3);
        assert!(result.has_tool_calls);
    }

    #[test]
    fn test_proto_agent_includes_capability_ids() {
        use chrono::Utc;
        use everruns_capability::CapabilityRef as AgentCapabilityConfig;
        use uuid::Uuid;

        // Create an Agent with capabilities
        let id = Uuid::now_v7();
        let agent = everruns_platform::Agent {
            public_id: everruns_provider::typed_id::AgentId::from_uuid(id),
            internal_id: id,
            name: "test-agent".to_string(),
            display_name: Some("Test Agent".to_string()),
            description: Some("Test description".to_string()),
            system_prompt: "You are a helpful assistant".to_string(),
            default_model_id: None,
            harness_id: everruns_provider::typed_id::HarnessId::new(),
            default_version_id: None,
            forked_from_agent_id: None,
            forked_from_version_id: None,
            root_agent_id: None,
            tags: vec!["slack:thread:123.456".to_string()],
            capabilities: vec![
                AgentCapabilityConfig::new("tools:read_file"),
                AgentCapabilityConfig::new("tools:write_file"),
            ],
            initial_files: vec![],
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            tools: vec![],
            mcp_servers: Default::default(),
            status: everruns_platform::AgentStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            archived_at: None,
            deleted_at: None,
            usage: None,
        };

        // Convert to proto
        let proto_agent = schema_agent_to_proto(&agent);

        // Verify capability_ids are preserved
        assert!(
            proto_agent
                .capability_ids
                .contains(&"tools:read_file".to_string())
        );
        assert!(
            proto_agent
                .capability_ids
                .contains(&"tools:write_file".to_string())
        );

        // Convert back to schema
        let schema_agent = proto_agent_to_schema(proto_agent).unwrap();

        // Verify capabilities survive roundtrip
        // Check capability IDs are preserved (config defaults to empty)
        let cap_ids: Vec<&str> = schema_agent
            .capabilities
            .iter()
            .map(|c| c.capability_id())
            .collect();
        assert!(cap_ids.contains(&"tools:read_file"));
        assert!(cap_ids.contains(&"tools:write_file"));
    }

    #[test]
    fn test_proto_agent_without_capabilities() {
        use chrono::Utc;
        use uuid::Uuid;

        // Create an Agent without capabilities
        let id = Uuid::now_v7();
        let agent = everruns_platform::Agent {
            public_id: everruns_provider::typed_id::AgentId::from_uuid(id),
            internal_id: id,
            name: "test-agent".to_string(),
            display_name: Some("Test Agent".to_string()),
            description: None,
            system_prompt: "You are a helpful assistant".to_string(),
            default_model_id: None,
            harness_id: everruns_provider::typed_id::HarnessId::new(),
            default_version_id: None,
            forked_from_agent_id: None,
            forked_from_version_id: None,
            root_agent_id: None,
            tags: vec!["slack:thread:123.456".to_string()],
            capabilities: vec![],
            initial_files: vec![],
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            tools: vec![],
            mcp_servers: Default::default(),
            status: everruns_platform::AgentStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            archived_at: None,
            deleted_at: None,
            usage: None,
        };

        // Convert to proto
        let proto_agent = schema_agent_to_proto(&agent);

        // Verify capability_ids are empty
        assert!(proto_agent.capability_ids.is_empty());

        // Convert back to schema
        let schema_agent = proto_agent_to_schema(proto_agent).unwrap();

        // Verify capabilities remain empty
        assert!(schema_agent.capabilities.is_empty());
    }

    #[test]
    fn test_message_thinking_field_roundtrip() {
        use chrono::Utc;
        use everruns_core::{ContentPart, Message, MessageRole};
        use uuid::Uuid;

        // Create a Message with thinking content (simulating extended thinking model output)
        let thinking_content = "Let me analyze this step by step...\n\n1. First, I need to understand the problem.\n2. Then, I'll formulate a solution.".to_string();
        let message = Message {
            id: Uuid::now_v7().into(),
            role: MessageRole::Agent,
            content: vec![ContentPart::text(
                "Here is my response based on my analysis.",
            )],
            phase: None,
            thinking: Some(thinking_content.clone()),
            thinking_signature: Some("test_signature_abc123".to_string()),
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: Utc::now(),
        };

        // Convert to proto
        let proto_message = schema_message_to_proto(&message);

        // Verify thinking field is set in proto
        assert_eq!(proto_message.thinking, Some(thinking_content.clone()));
        assert_eq!(
            proto_message.thinking_signature,
            Some("test_signature_abc123".to_string())
        );

        // Convert back to schema
        let schema_message = proto_message_to_schema(proto_message).unwrap();

        // Verify thinking field survives roundtrip
        assert_eq!(schema_message.thinking, Some(thinking_content));
        assert_eq!(schema_message.role, MessageRole::Agent);
    }

    #[test]
    fn test_message_without_thinking_roundtrip() {
        use chrono::Utc;
        use everruns_core::{ContentPart, Message, MessageRole};
        use uuid::Uuid;

        // Create a Message without thinking content (normal model output)
        let message = Message {
            id: Uuid::now_v7().into(),
            role: MessageRole::Agent,
            content: vec![ContentPart::text("A simple response without thinking.")],
            phase: None,
            reasoning: Vec::new(),
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: Utc::now(),
        };

        // Convert to proto
        let proto_message = schema_message_to_proto(&message);

        // Verify thinking field is None in proto
        assert_eq!(proto_message.thinking, None);

        // Convert back to schema
        let schema_message = proto_message_to_schema(proto_message).unwrap();

        // Verify thinking field remains None
        assert_eq!(schema_message.thinking, None);
    }

    #[test]
    fn test_external_actor_proto_roundtrip() {
        use chrono::Utc;
        use everruns_core::{ContentPart, ExternalActor, Message, MessageRole};
        use uuid::Uuid;

        let actor = ExternalActor {
            actor_id: "U0123456789".to_string(),
            actor_name: Some("Alice".to_string()),
            source: "slack".to_string(),
            metadata: Some(
                [("channel".to_string(), "C999".to_string())]
                    .into_iter()
                    .collect(),
            ),
        };

        let message = Message {
            id: Uuid::now_v7().into(),
            role: MessageRole::User,
            content: vec![ContentPart::text("Hello")],
            phase: None,
            reasoning: Vec::new(),
            controls: None,
            metadata: None,
            external_actor: Some(actor.clone()),
            created_at: Utc::now(),
        };

        let proto_message = schema_message_to_proto(&message);
        assert!(proto_message.external_actor.is_some());

        let schema_message = proto_message_to_schema(proto_message).unwrap();
        let roundtripped = schema_message.external_actor.unwrap();
        assert_eq!(roundtripped.actor_id, "U0123456789");
        assert_eq!(roundtripped.actor_name, Some("Alice".to_string()));
        assert_eq!(roundtripped.source, "slack");
        assert_eq!(
            roundtripped
                .metadata
                .as_ref()
                .unwrap()
                .get("channel")
                .unwrap(),
            "C999"
        );
    }

    #[test]
    fn test_external_actor_none_proto_roundtrip() {
        use chrono::Utc;
        use everruns_core::{ContentPart, Message, MessageRole};
        use uuid::Uuid;

        let message = Message {
            id: Uuid::now_v7().into(),
            role: MessageRole::User,
            content: vec![ContentPart::text("Hello")],
            phase: None,
            reasoning: Vec::new(),
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: Utc::now(),
        };

        let proto_message = schema_message_to_proto(&message);
        assert!(proto_message.external_actor.is_none());

        let schema_message = proto_message_to_schema(proto_message).unwrap();
        assert!(schema_message.external_actor.is_none());
    }

    #[test]
    fn test_proto_session_roundtrip_includes_organization_id() {
        use chrono::Utc;
        use everruns_capability::CapabilityRef as AgentCapabilityConfig;

        let now = Utc::now();
        let session_id = everruns_provider::typed_id::SessionId::new();
        let session = everruns_platform::Session {
            source: Default::default(),
            activity: Default::default(),
            run_summary: None,
            id: session_id,
            // Equality invariant: workspace.id == session.id for default sessions.
            workspace_id: everruns_provider::typed_id::WorkspaceId::from_uuid(session_id.uuid()),
            organization_id: "org_00000000000000000000000000000001".to_string(),
            harness_id: everruns_provider::typed_id::HarnessId::new(),
            agent_id: None,
            agent_version_id: None,
            agent_identity_id: None,
            owner_principal_id: everruns_provider::typed_id::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            owner: None,
            effective_owner: None,
            title: Some("Test Session".to_string()),
            goal: None,
            locale: None,
            preview: None,
            output_preview: None,
            tags: vec!["slack:thread:123.456".to_string()],
            model_id: None,
            capabilities: vec![AgentCapabilityConfig::new("session")],
            tools: vec![],
            system_prompt: None,
            initial_files: vec![],
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: Some(true),
            mcp_servers: Default::default(),
            status: everruns_platform::SessionStatus::Idle,
            created_at: now,
            updated_at: now,
            started_at: None,
            finished_at: None,
            usage: None,
            is_pinned: None,
            archived_at: None,
            active_schedule_count: None,
            event_count: None,
            task_count: None,
            file_count: None,
            features: vec![],
            parent_session_id: Some(everruns_provider::typed_id::SessionId::new()),
            forked_from_session_id: None,
            forked_from_sequence: None,
            blueprint_id: Some("oracle".to_string()),
            blueprint_config: Some(serde_json::json!({ "depth": "focused" })),
        };

        // Convert to proto
        let proto_session = schema_session_to_proto(&session);
        assert_eq!(
            proto_session.organization_id,
            "org_00000000000000000000000000000001"
        );
        assert_eq!(proto_session.capabilities.len(), 1);
        assert_eq!(proto_session.tags, vec!["slack:thread:123.456".to_string()]);
        assert_eq!(proto_session.blueprint_id.as_deref(), Some("oracle"));
        assert_eq!(proto_session.parallel_tool_calls, Some(true));

        // Convert back to schema
        let schema_session = proto_session_to_schema(proto_session).unwrap();
        assert_eq!(
            schema_session.organization_id,
            "org_00000000000000000000000000000001"
        );
        assert_eq!(schema_session.id, session.id);
        // workspace_id is reconstructed from the session id (equality invariant)
        // since the proto Session does not carry it yet.
        assert_eq!(schema_session.workspace_id, session.workspace_id);
        assert_eq!(schema_session.harness_id, session.harness_id);
        assert_eq!(
            schema_session.owner_principal_id,
            session.owner_principal_id
        );
        assert_eq!(
            schema_session.tags,
            vec!["slack:thread:123.456".to_string()]
        );
        assert_eq!(schema_session.capabilities.len(), 1);
        assert_eq!(schema_session.capabilities[0].capability_id(), "session");
        assert_eq!(schema_session.parent_session_id, session.parent_session_id);
        assert_eq!(schema_session.blueprint_id, session.blueprint_id);
        assert_eq!(schema_session.blueprint_config, session.blueprint_config);
        assert_eq!(schema_session.parallel_tool_calls, Some(true));
    }

    #[test]
    fn test_conversion_error_to_tonic_status() {
        let err = ConversionError::MissingField("session_id");
        let status: tonic::Status = err.into();
        assert_eq!(status.code(), tonic::Code::InvalidArgument);
        assert!(
            status
                .message()
                .contains("Missing required field: session_id")
        );

        let err = ConversionError::JsonError(
            serde_json::from_str::<serde_json::Value>("invalid").unwrap_err(),
        );
        let status: tonic::Status = err.into();
        assert_eq!(status.code(), tonic::Code::InvalidArgument);
        assert!(status.message().contains("JSON error"));
    }

    // EVE-652: a proto entity missing its required id used to build an empty-string
    // typed id and fail later as an opaque JsonError. It now fails with the precise
    // MissingField, instead of silently corrupting the id.
    #[test]
    fn test_proto_agent_missing_id_is_missing_field_error() {
        let proto_agent = proto::Agent {
            id: None,
            ..Default::default()
        };
        let err = proto_agent_to_schema(proto_agent).unwrap_err();
        assert!(
            matches!(err, ConversionError::MissingField("id")),
            "expected MissingField(\"id\"), got {err:?}"
        );
    }

    #[test]
    fn test_proto_harness_missing_id_is_missing_field_error() {
        let proto_harness = proto::Harness {
            id: None,
            ..Default::default()
        };
        let err = proto_harness_to_schema(proto_harness).unwrap_err();
        assert!(
            matches!(err, ConversionError::MissingField("id")),
            "expected MissingField(\"id\"), got {err:?}"
        );
    }

    #[test]
    fn agent_capability_config_survives_proto_round_trip() {
        let definition = everruns_core::DeclarativeCapabilityDefinition {
            name: "resend".to_string(),
            description: "Send email".to_string(),
            ..Default::default()
        };
        let config = serde_json::json!({
            "ref": "plugin:plugin_019fda530ed27b4291c67d9f786961d9",
            "config": serde_json::to_value(&definition).unwrap()
        });
        let proto_agent = proto::Agent {
            id: Some(uuid_to_proto_uuid(uuid::Uuid::new_v4())),
            harness_id: Some(uuid_to_proto_uuid(uuid::Uuid::new_v4())),
            name: "mailer".to_string(),
            status: "active".to_string(),
            capabilities: vec![config.to_string()],
            created_at: Some(datetime_to_proto_timestamp(chrono::Utc::now())),
            updated_at: Some(datetime_to_proto_timestamp(chrono::Utc::now())),
            ..Default::default()
        };

        let agent = proto_agent_to_schema(proto_agent).unwrap();

        assert_eq!(agent.capabilities[0].config_value()["name"], "resend");
        serde_json::from_value::<everruns_core::DeclarativeCapabilityDefinition>(
            agent.capabilities[0].config_value().clone(),
        )
        .expect("plugin definition remains runtime-loadable");
    }

    #[test]
    fn agent_proto_without_full_configs_falls_back_to_capability_ids() {
        let proto_agent = proto::Agent {
            id: Some(uuid_to_proto_uuid(uuid::Uuid::new_v4())),
            harness_id: Some(uuid_to_proto_uuid(uuid::Uuid::new_v4())),
            name: "legacy".to_string(),
            status: "active".to_string(),
            capability_ids: vec!["session".to_string()],
            created_at: Some(datetime_to_proto_timestamp(chrono::Utc::now())),
            updated_at: Some(datetime_to_proto_timestamp(chrono::Utc::now())),
            ..Default::default()
        };

        let agent = proto_agent_to_schema(proto_agent).unwrap();

        assert_eq!(agent.capabilities[0].capability_id(), "session");
        assert_eq!(
            agent.capabilities[0].config_value().clone(),
            serde_json::json!({})
        );
    }

    #[test]
    fn test_proto_session_missing_id_is_missing_field_error() {
        let proto_session = proto::Session {
            id: None,
            ..Default::default()
        };
        let err = proto_session_to_schema(proto_session).unwrap_err();
        assert!(
            matches!(err, ConversionError::MissingField("id")),
            "expected MissingField(\"id\"), got {err:?}"
        );
    }

    // EVE-652: an unparseable capability string is dropped (lenient, so one bad
    // entry can't fail the whole session) but no longer silently — the valid
    // capabilities still round-trip and the array is not otherwise corrupted.
    #[test]
    fn test_proto_session_drops_unparseable_capability_but_keeps_valid() {
        use chrono::Utc;
        use everruns_capability::CapabilityRef as AgentCapabilityConfig;

        let now = Utc::now();
        let session_id = everruns_provider::typed_id::SessionId::new();
        let session = everruns_platform::Session {
            source: Default::default(),
            activity: Default::default(),
            run_summary: None,
            id: session_id,
            workspace_id: everruns_provider::typed_id::WorkspaceId::from_uuid(session_id.uuid()),
            organization_id: "org_00000000000000000000000000000001".to_string(),
            harness_id: everruns_provider::typed_id::HarnessId::new(),
            agent_id: None,
            agent_version_id: None,
            agent_identity_id: None,
            owner_principal_id: everruns_provider::typed_id::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            owner: None,
            effective_owner: None,
            title: None,
            goal: None,
            locale: None,
            preview: None,
            output_preview: None,
            tags: vec![],
            model_id: None,
            capabilities: vec![AgentCapabilityConfig::new("session")],
            tools: vec![],
            system_prompt: None,
            initial_files: vec![],
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            mcp_servers: Default::default(),
            status: everruns_platform::SessionStatus::Idle,
            created_at: now,
            updated_at: now,
            started_at: None,
            finished_at: None,
            usage: None,
            is_pinned: None,
            archived_at: None,
            active_schedule_count: None,
            event_count: None,
            task_count: None,
            file_count: None,
            features: vec![],
            parent_session_id: None,
            blueprint_id: None,
            blueprint_config: None,
            forked_from_session_id: None,
            forked_from_sequence: None,
        };

        let mut proto_session = schema_session_to_proto(&session);
        // Inject a malformed capability JSON string alongside the valid one.
        proto_session
            .capabilities
            .push("{not valid json".to_string());
        assert_eq!(proto_session.capabilities.len(), 2);

        let schema_session = proto_session_to_schema(proto_session).unwrap();
        // The bad entry is dropped; the valid capability survives.
        assert_eq!(schema_session.capabilities.len(), 1);
        assert_eq!(schema_session.capabilities[0].capability_id(), "session");
    }

    // EVE-652: prefixed_id centralizes the `<prefix>_<hex>` construction that the
    // conversions repeat; it strips dashes so the typed-id layer parses the value.
    #[test]
    fn test_prefixed_id_strips_dashes_and_prefixes() {
        let u = proto::Uuid {
            value: "0191e1a2-3b4c-7d8e-9f00-112233445566".to_string(),
        };
        assert_eq!(
            prefixed_id("agent", &u),
            "agent_0191e1a23b4c7d8e9f00112233445566"
        );
    }

    // EVE-652: known roles map exactly; an unknown role still defaults to User
    // (now logged) rather than erroring or being dropped.
    #[test]
    fn test_parse_message_role_known_and_unknown() {
        use everruns_core::MessageRole;
        assert!(matches!(parse_message_role("system"), MessageRole::System));
        assert!(matches!(parse_message_role("USER"), MessageRole::User));
        assert!(matches!(
            parse_message_role("assistant"),
            MessageRole::Agent
        ));
        assert!(matches!(parse_message_role("agent"), MessageRole::Agent));
        assert!(matches!(
            parse_message_role("tool_result"),
            MessageRole::ToolResult
        ));
        assert!(matches!(
            parse_message_role("something_unknown"),
            MessageRole::User
        ));
    }

    // ------------------------------------------------------------------------
    // Session task registry native-proto round-trip fidelity (EVE-642)
    // ------------------------------------------------------------------------

    fn sample_session_task() -> st::SessionTask {
        st::SessionTask {
            id: "task_abc123".to_string(),
            session_id: SessionId::new(),
            root_session_id: None,
            kind: st::TASK_KIND_SUBAGENT.to_string(),
            display_name: "Investigate flake".to_string(),
            spec: serde_json::json!({
                "instructions": "run tests",
                "retries": 3,
                "nested": { "a": [1, 2, 3], "b": null, "flag": true },
            }),
            state: st::SessionTaskState::AwaitingInput,
            state_detail: Some("iteration 4/10".to_string()),
            progress: Some(st::TaskProgress {
                current: Some(4),
                total: Some(10),
                unit: Some("steps".to_string()),
                label: Some("running".to_string()),
            }),
            input_request: Some(st::TaskInputRequest {
                id: "req_1".to_string(),
                prompt: "Approve?".to_string(),
                expected: Some(serde_json::json!({ "type": "boolean" })),
            }),
            cancel_requested_at: Some(Utc.timestamp_opt(1_700_000_100, 0).unwrap()),
            summary: Some("did the thing".to_string()),
            result_path: Some("/.tasks/task_abc123/result.json".to_string()),
            artifacts: vec![
                st::TaskArtifact {
                    name: "report".to_string(),
                    artifact_type: "file".to_string(),
                    path: Some("/report.md".to_string()),
                    url: None,
                },
                st::TaskArtifact {
                    name: "pr".to_string(),
                    artifact_type: "url".to_string(),
                    path: None,
                    url: Some("https://example.com/pr/1".to_string()),
                },
            ],
            error: Some(st::TaskError {
                kind: "timeout".to_string(),
                message: "exceeded deadline".to_string(),
            }),
            attempt: 2,
            worker_id: Some("worker-7".to_string()),
            heartbeat_at: Some(Utc.timestamp_opt(1_700_000_200, 500_000_000).unwrap()),
            links: st::TaskLinks {
                child_session_id: Some(SessionId::new()),
                remote_task_id: Some("rt_9".to_string()),
                resource_ids: vec!["res_1".to_string(), "res_2".to_string()],
            },
            wake_policy: st::TaskWakePolicy::OnActivity,
            created_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            started_at: Some(Utc.timestamp_opt(1_700_000_050, 0).unwrap()),
            finished_at: None,
            updated_at: Utc.timestamp_opt(1_700_000_300, 0).unwrap(),
        }
    }

    /// Full round-trip must preserve the task through the native proto and back
    /// so switching the wire format from JSON-in-bytes to native protobuf is
    /// lossless. Comparison is done via canonical JSON so the numeric widening
    /// in `google.protobuf.Value` (spec/expected) is accounted for.
    #[test]
    fn session_task_native_proto_round_trip() {
        let original = sample_session_task();
        let proto = session_task_to_proto(&original);
        let restored = proto_to_session_task(proto).expect("round-trip");

        let a = serde_json::to_value(&original).unwrap();
        let b = serde_json::to_value(&restored).unwrap();
        assert_eq!(a, b, "session task must survive native-proto round-trip");
    }

    #[test]
    fn create_session_task_native_proto_round_trip() {
        let original = st::CreateSessionTask {
            session_id: SessionId::new(),
            id: Some("task_seed".to_string()),
            kind: st::TASK_KIND_BACKGROUND_TOOL.to_string(),
            display_name: "seed".to_string(),
            spec: serde_json::json!({ "k": "v", "n": 12 }),
            state: st::SessionTaskState::Running,
            links: st::TaskLinks {
                child_session_id: None,
                remote_task_id: Some("rt".to_string()),
                resource_ids: vec!["res".to_string()],
            },
            wake_policy: st::TaskWakePolicy::OnTerminal,
        };
        let proto = create_session_task_to_proto(&original);
        let restored = proto_to_create_session_task(proto).expect("round-trip");

        assert_eq!(
            serde_json::to_value(&original).unwrap(),
            serde_json::to_value(&restored).unwrap()
        );
    }

    #[test]
    fn session_task_update_native_proto_round_trip() {
        // A fully-populated update (all Option/Vec fields set).
        let full = st::SessionTaskUpdate {
            state: Some(st::SessionTaskState::Failed),
            state_detail: Some("boom".to_string()),
            progress: Some(st::TaskProgress {
                current: Some(1),
                total: None,
                unit: None,
                label: Some("x".to_string()),
            }),
            input_request: Some(st::TaskInputRequest {
                id: "r".to_string(),
                prompt: "p".to_string(),
                expected: None,
            }),
            summary: Some("s".to_string()),
            result_path: Some("/r.json".to_string()),
            artifacts: Some(vec![st::TaskArtifact {
                name: "a".to_string(),
                artifact_type: "file".to_string(),
                path: Some("/a".to_string()),
                url: None,
            }]),
            error: Some(st::TaskError {
                kind: "orphaned".to_string(),
                message: "m".to_string(),
            }),
            links: Some(st::TaskLinks::default()),
            worker_id: Some("w".to_string()),
            heartbeat_at: Some(Utc.timestamp_opt(1_700_000_000, 0).unwrap()),
            expected_attempt: Some(3),
            increment_attempt: true,
        };
        let restored = proto_to_session_task_update(session_task_update_to_proto(&full));
        assert_eq!(
            serde_json::to_value(&full).unwrap(),
            serde_json::to_value(&restored).unwrap()
        );

        // An empty update must stay empty: crucially, an absent `artifacts`
        // must not become `Some(vec![])`, which would wrongly clear artifacts.
        let empty = st::SessionTaskUpdate::default();
        let restored_empty = proto_to_session_task_update(session_task_update_to_proto(&empty));
        assert!(restored_empty.state.is_none());
        assert!(
            restored_empty.artifacts.is_none(),
            "absent artifacts update must round-trip as None, not Some(empty)"
        );
        assert!(!restored_empty.increment_attempt);
        assert!(restored_empty.expected_attempt.is_none());

        // An explicit empty artifact list (clear all) must survive as Some(empty).
        let clear = st::SessionTaskUpdate {
            artifacts: Some(vec![]),
            ..Default::default()
        };
        let restored_clear = proto_to_session_task_update(session_task_update_to_proto(&clear));
        assert_eq!(restored_clear.artifacts, Some(vec![]));
    }

    #[test]
    fn task_message_native_proto_round_trip() {
        let msg = st::TaskMessage {
            id: "tmsg_1".to_string(),
            task_id: "task_1".to_string(),
            direction: st::TaskMessageDirection::Outbound,
            content: vec![
                st::TaskMessagePart::text("hello"),
                st::TaskMessagePart::Data {
                    data: serde_json::json!({ "k": [1, 2], "b": true }),
                },
            ],
            in_reply_to: Some("tmsg_0".to_string()),
            created_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
        };
        let restored = proto_to_task_message(task_message_to_proto(&msg)).expect("round-trip");
        assert_eq!(
            serde_json::to_value(&msg).unwrap(),
            serde_json::to_value(&restored).unwrap()
        );
    }

    #[test]
    fn new_task_message_native_proto_round_trip() {
        let msg = st::NewTaskMessage {
            direction: st::TaskMessageDirection::Inbound,
            content: vec![st::TaskMessagePart::text("answer")],
            in_reply_to: Some("req_1".to_string()),
            expected_attempt: Some(5),
        };
        let restored = proto_to_new_task_message(new_task_message_to_proto(&msg));
        assert_eq!(
            serde_json::to_value(&msg).unwrap(),
            serde_json::to_value(&restored).unwrap()
        );
    }
}
