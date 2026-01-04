// Internal Protocol for Worker <-> Control Plane Communication
//
// Decision: gRPC with tonic (industry standard, already in stack)
// Decision: Use google.protobuf.Value/Struct for JSON values instead of strings
// Decision: Proto is transport layer, Rust schemas remain source of truth

use chrono::{DateTime, TimeZone, Utc};
use prost_types::{value::Kind, ListValue, Struct, Value};

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

/// Convert from proto Timestamp to chrono DateTime<Utc>
pub fn proto_timestamp_to_datetime(value: &proto::Timestamp) -> DateTime<Utc> {
    Utc.timestamp_opt(value.seconds, value.nanos as u32)
        .single()
        .unwrap_or_else(Utc::now)
}

/// Convert from chrono DateTime<Utc> to proto Timestamp
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
pub fn proto_value_to_json(value: &Value) -> serde_json::Value {
    match &value.kind {
        Some(Kind::NullValue(_)) => serde_json::Value::Null,
        Some(Kind::BoolValue(b)) => serde_json::Value::Bool(*b),
        Some(Kind::NumberValue(n)) => serde_json::Number::from_f64(*n)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
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
            serde_json::Value::Number(n) => Kind::NumberValue(n.as_f64().unwrap_or(0.0)),
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
/// Maps event_type string to the appropriate EventData variant and deserializes
/// the data payload accordingly.
fn deserialize_event_data(
    event_type: &str,
    data: serde_json::Value,
) -> Result<everruns_core::EventData, ConversionError> {
    use everruns_core::events::*;
    use everruns_core::EventData;

    Ok(match event_type {
        MESSAGE_USER => {
            let typed: MessageUserData = serde_json::from_value(data)?;
            EventData::MessageUser(typed)
        }
        MESSAGE_AGENT => {
            let typed: MessageAgentData = serde_json::from_value(data)?;
            EventData::MessageAgent(typed)
        }
        TURN_STARTED => {
            let typed: TurnStartedData = serde_json::from_value(data)?;
            EventData::TurnStarted(typed)
        }
        TURN_COMPLETED => {
            let typed: TurnCompletedData = serde_json::from_value(data)?;
            EventData::TurnCompleted(typed)
        }
        TURN_FAILED => {
            let typed: TurnFailedData = serde_json::from_value(data)?;
            EventData::TurnFailed(typed)
        }
        INPUT_RECEIVED => {
            let typed: InputReceivedData = serde_json::from_value(data)?;
            EventData::InputReceived(typed)
        }
        REASON_STARTED => {
            let typed: ReasonStartedData = serde_json::from_value(data)?;
            EventData::ReasonStarted(typed)
        }
        REASON_COMPLETED => {
            let typed: ReasonCompletedData = serde_json::from_value(data)?;
            EventData::ReasonCompleted(typed)
        }
        ACT_STARTED => {
            let typed: ActStartedData = serde_json::from_value(data)?;
            EventData::ActStarted(typed)
        }
        ACT_COMPLETED => {
            let typed: ActCompletedData = serde_json::from_value(data)?;
            EventData::ActCompleted(typed)
        }
        TOOL_CALL_STARTED => {
            let typed: ToolCallStartedData = serde_json::from_value(data)?;
            EventData::ToolCallStarted(typed)
        }
        TOOL_CALL_COMPLETED => {
            let typed: ToolCallCompletedData = serde_json::from_value(data)?;
            EventData::ToolCallCompleted(typed)
        }
        LLM_GENERATION => {
            let typed: LlmGenerationData = serde_json::from_value(data)?;
            EventData::LlmGeneration(typed)
        }
        SESSION_STARTED => {
            let typed: SessionStartedData = serde_json::from_value(data)?;
            EventData::SessionStarted(typed)
        }
        _ => {
            // Unknown event type - store as raw JSON
            EventData::Raw(data)
        }
    })
}

/// Serialize EventData to JSON Value
///
/// Converts the typed EventData variant to its JSON representation.
fn serialize_event_data(data: &everruns_core::EventData) -> serde_json::Value {
    use everruns_core::EventData;

    match data {
        EventData::MessageUser(d) => serde_json::to_value(d).unwrap_or_default(),
        EventData::MessageAgent(d) => serde_json::to_value(d).unwrap_or_default(),
        EventData::TurnStarted(d) => serde_json::to_value(d).unwrap_or_default(),
        EventData::TurnCompleted(d) => serde_json::to_value(d).unwrap_or_default(),
        EventData::TurnFailed(d) => serde_json::to_value(d).unwrap_or_default(),
        EventData::InputReceived(d) => serde_json::to_value(d).unwrap_or_default(),
        EventData::ReasonStarted(d) => serde_json::to_value(d).unwrap_or_default(),
        EventData::ReasonCompleted(d) => serde_json::to_value(d).unwrap_or_default(),
        EventData::ActStarted(d) => serde_json::to_value(d).unwrap_or_default(),
        EventData::ActCompleted(d) => serde_json::to_value(d).unwrap_or_default(),
        EventData::ToolCallStarted(d) => serde_json::to_value(d).unwrap_or_default(),
        EventData::ToolCallCompleted(d) => serde_json::to_value(d).unwrap_or_default(),
        EventData::LlmGeneration(d) => serde_json::to_value(d).unwrap_or_default(),
        EventData::SessionStarted(d) => serde_json::to_value(d).unwrap_or_default(),
        EventData::Raw(v) => v.clone(),
    }
}

// ============================================================================
// Conversion to/from schemas types
// ============================================================================

/// Convert proto Agent to schemas Agent using JSON
pub fn proto_agent_to_schema(value: proto::Agent) -> Result<everruns_core::Agent, ConversionError> {
    // Serialize proto to JSON, then deserialize to schema type
    // This is simpler and more maintainable than field-by-field conversion
    let tags: Vec<String> = vec![];
    let json = serde_json::json!({
        "id": value.id.as_ref().map(|u| &u.value).unwrap_or(&String::new()),
        "name": value.name,
        "description": if value.description.is_empty() { None } else { Some(&value.description) },
        "system_prompt": value.system_prompt,
        "default_model_id": value.default_model_id.as_ref().map(|u| &u.value),
        "tags": tags,
        "capabilities": value.capability_ids,
        "status": value.status,
        "created_at": value.created_at.as_ref().map(|t| proto_timestamp_to_datetime(t).to_rfc3339()),
        "updated_at": value.updated_at.as_ref().map(|t| proto_timestamp_to_datetime(t).to_rfc3339()),
    });
    serde_json::from_value(json).map_err(ConversionError::from)
}

/// Convert schemas Agent to proto Agent
pub fn schema_agent_to_proto(value: &everruns_core::Agent) -> proto::Agent {
    proto::Agent {
        id: Some(uuid_to_proto_uuid(value.id)),
        name: value.name.clone(),
        description: value.description.clone().unwrap_or_default(),
        system_prompt: value.system_prompt.clone(),
        default_model_id: value.default_model_id.map(uuid_to_proto_uuid),
        temperature: None,
        max_tokens: None,
        status: value.status.to_string(),
        created_at: Some(datetime_to_proto_timestamp(value.created_at)),
        updated_at: Some(datetime_to_proto_timestamp(value.updated_at)),
        capability_ids: value.capabilities.iter().map(|c| c.to_string()).collect(),
    }
}

/// Convert proto Session to schemas Session using JSON
pub fn proto_session_to_schema(
    value: proto::Session,
) -> Result<everruns_core::Session, ConversionError> {
    let tags: Vec<String> = vec![];
    let started_at: Option<String> = None;
    let finished_at: Option<String> = None;
    let json = serde_json::json!({
        "id": value.id.as_ref().map(|u| &u.value).unwrap_or(&String::new()),
        "agent_id": value.agent_id.as_ref().map(|u| &u.value).unwrap_or(&String::new()),
        "title": if value.title.is_empty() { None } else { Some(&value.title) },
        "tags": tags,
        "model_id": value.default_model_id.as_ref().map(|u| &u.value),
        "status": value.status,
        "created_at": value.created_at.as_ref().map(|t| proto_timestamp_to_datetime(t).to_rfc3339()),
        "started_at": started_at,
        "finished_at": finished_at,
    });
    serde_json::from_value(json).map_err(ConversionError::from)
}

/// Convert schemas Session to proto Session
pub fn schema_session_to_proto(value: &everruns_core::Session) -> proto::Session {
    proto::Session {
        id: Some(uuid_to_proto_uuid(value.id)),
        agent_id: Some(uuid_to_proto_uuid(value.agent_id)),
        title: value.title.clone().unwrap_or_default(),
        status: value.status.to_string(),
        created_at: Some(datetime_to_proto_timestamp(value.created_at)),
        updated_at: Some(datetime_to_proto_timestamp(value.created_at)), // Use created_at as fallback
        default_model_id: value.model_id.map(uuid_to_proto_uuid),
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

    let role = parse_message_role(&value.role);

    Ok(everruns_core::Message {
        id,
        role,
        content,
        controls,
        metadata,
        created_at,
    })
}

/// Convert schemas Message to proto Message
pub fn schema_message_to_proto(value: &everruns_core::Message) -> proto::Message {
    // Convert content to ListValue
    let content_json = serde_json::to_value(&value.content).unwrap_or_default();
    let content = Some(json_to_proto_list(&content_json));

    // Convert controls to Struct
    let controls = value.controls.as_ref().map(|c| {
        let json = serde_json::to_value(c).unwrap_or_default();
        json_to_proto_struct(&json)
    });

    // Convert metadata to Struct
    let metadata = value.metadata.as_ref().map(|m| {
        let json = serde_json::to_value(m).unwrap_or_default();
        json_to_proto_struct(&json)
    });

    proto::Message {
        id: Some(uuid_to_proto_uuid(value.id)),
        role: value.role.to_string(),
        content,
        controls,
        metadata,
        created_at: Some(datetime_to_proto_timestamp(value.created_at)),
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
            .transpose()?,
        input_message_id: proto_context
            .input_message_id
            .as_ref()
            .map(proto_uuid_to_uuid)
            .transpose()?,
        exec_id: proto_context
            .exec_id
            .as_ref()
            .map(proto_uuid_to_uuid)
            .transpose()?,
    };

    // Convert Struct data to EventData based on event_type
    let data_struct = value
        .data
        .as_ref()
        .ok_or(ConversionError::MissingField("data"))?;
    let data_json = proto_struct_to_json(data_struct);
    let data = deserialize_event_data(&value.event_type, data_json)?;

    // Convert optional metadata from prost Struct
    let metadata: Option<serde_json::Value> = value.metadata.as_ref().map(proto_struct_to_json);

    Ok(everruns_core::Event {
        id,
        event_type: value.event_type,
        ts,
        session_id,
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
        id: Some(uuid_to_proto_uuid(value.id)),
        event_type: value.event_type.clone(),
        ts: Some(datetime_to_proto_timestamp(value.ts)),
        context: Some(proto::EventContext {
            session_id: Some(uuid_to_proto_uuid(value.session_id)),
            turn_id: value.context.turn_id.map(uuid_to_proto_uuid),
            input_message_id: value.context.input_message_id.map(uuid_to_proto_uuid),
            exec_id: value.context.exec_id.map(uuid_to_proto_uuid),
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
            .transpose()?,
        input_message_id: proto_context
            .input_message_id
            .as_ref()
            .map(proto_uuid_to_uuid)
            .transpose()?,
        exec_id: proto_context
            .exec_id
            .as_ref()
            .map(proto_uuid_to_uuid)
            .transpose()?,
    };

    // Convert Struct data to EventData based on event_type
    let data_struct = value
        .data
        .as_ref()
        .ok_or(ConversionError::MissingField("data"))?;
    let data_json = proto_struct_to_json(data_struct);
    let data = deserialize_event_data(&value.event_type, data_json)?;

    // Convert optional metadata from prost Struct
    let metadata: Option<serde_json::Value> = value.metadata.as_ref().map(proto_struct_to_json);

    Ok(everruns_core::EventRequest {
        event_type: value.event_type,
        ts,
        session_id,
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
            session_id: Some(uuid_to_proto_uuid(value.session_id)),
            turn_id: value.context.turn_id.map(uuid_to_proto_uuid),
            input_message_id: value.context.input_message_id.map(uuid_to_proto_uuid),
            exec_id: value.context.exec_id.map(uuid_to_proto_uuid),
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

fn parse_message_role(s: &str) -> everruns_core::MessageRole {
    match s.to_lowercase().as_str() {
        "system" => everruns_core::MessageRole::System,
        "user" => everruns_core::MessageRole::User,
        "assistant" => everruns_core::MessageRole::Assistant,
        "tool_result" => everruns_core::MessageRole::ToolResult,
        _ => everruns_core::MessageRole::User,
    }
}
