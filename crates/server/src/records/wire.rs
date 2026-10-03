//! Server-edge mapping between control-plane records and internal transport DTOs.
use everruns_contracts::typed_id::{AgentId, HarnessId, SessionId};
use everruns_internal_protocol::{
    ConversionError, datetime_to_proto_timestamp, encode_capability_configs, json_to_proto_struct,
    proto, proto_struct_to_json, proto_timestamp_to_datetime, proto_uuid_to_uuid,
    uuid_to_proto_uuid,
};
fn prefixed_id(prefix: &str, value: &proto::Uuid) -> String {
    format!("{prefix}_{}", value.value.replace('-', ""))
}

/// Convert proto Agent to the stored platform Agent record using JSON.
///
/// EVE-877: the stored record lives in `everruns-capabilities`; the proto shape is
/// unchanged. Both endpoints of this wire (server, worker) are platform-side.
pub fn proto_agent_to_schema(
    value: proto::Agent,
) -> Result<crate::records::Agent, ConversionError> {
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
pub fn schema_agent_to_proto(value: &crate::records::Agent) -> proto::Agent {
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
        capabilities: encode_capability_configs(&value.capabilities),
    }
}

/// Convert schemas Harness to proto Harness
pub fn schema_harness_to_proto(value: &crate::records::Harness) -> proto::Harness {
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
        capabilities: encode_capability_configs(&value.capabilities),
    }
}

/// Convert proto Harness to schemas Harness
pub fn proto_harness_to_schema(
    value: proto::Harness,
) -> Result<crate::records::Harness, ConversionError> {
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
/// (EVE-882: the persisted aggregate lives in `everruns-capabilities`).
pub fn proto_session_to_schema(
    value: proto::Session,
) -> Result<crate::records::Session, ConversionError> {
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
pub fn schema_session_to_proto(value: &crate::records::Session) -> proto::Session {
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
