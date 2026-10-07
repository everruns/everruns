// Control-plane codecs stay at the server transport boundary.
use super::wire::*;
use everruns_internal_protocol::*;

#[test]
fn test_proto_agent_includes_capability_ids() {
    use chrono::Utc;
    use everruns_contracts::CapabilityRef as AgentCapabilityConfig;
    use uuid::Uuid;

    // Create an Agent with capabilities
    let id = Uuid::now_v7();
    let agent = crate::records::Agent {
        is_built_in: false,
        avatar: None,
        service_virtual_user_id: None,

        public_id: everruns_contracts::typed_id::AgentId::from_uuid(id),
        internal_id: id,
        name: "test-agent".to_string(),
        display_name: Some("Test Agent".to_string()),
        description: Some("Test description".to_string()),
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: "You are a helpful assistant".to_string(),
        default_model_id: None,
        harness_id: everruns_contracts::typed_id::HarnessId::new(),
        forked_from_agent_id: None,
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
        sandbox_policy: None,
        tools: vec![],
        mcp_servers: Default::default(),
        status: crate::records::AgentStatus::Active,
        exposures_suspended: false,
        exposed: false,
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
    let agent = crate::records::Agent {
        is_built_in: false,
        avatar: None,
        service_virtual_user_id: None,

        public_id: everruns_contracts::typed_id::AgentId::from_uuid(id),
        internal_id: id,
        name: "test-agent".to_string(),
        display_name: Some("Test Agent".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: "You are a helpful assistant".to_string(),
        default_model_id: None,
        harness_id: everruns_contracts::typed_id::HarnessId::new(),
        forked_from_agent_id: None,
        root_agent_id: None,
        tags: vec!["slack:thread:123.456".to_string()],
        capabilities: vec![],
        initial_files: vec![],
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        sandbox_policy: None,
        tools: vec![],
        mcp_servers: Default::default(),
        status: crate::records::AgentStatus::Active,
        exposures_suspended: false,
        exposed: false,
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
fn test_proto_session_roundtrip_includes_organization_id() {
    use chrono::Utc;
    use everruns_contracts::CapabilityRef as AgentCapabilityConfig;

    let now = Utc::now();
    let session_id = everruns_contracts::typed_id::SessionId::new();
    let session = crate::records::Session {
        playground_user_id: None,
        source: Default::default(),
        activity: Default::default(),
        run_summary: None,
        id: session_id,
        // Equality invariant: workspace.id == session.id for default sessions.
        workspace_id: everruns_contracts::typed_id::WorkspaceId::from_uuid(session_id.uuid()),
        organization_id: "org_00000000000000000000000000000001".to_string(),
        harness_id: everruns_contracts::typed_id::HarnessId::new(),
        agent_id: None,
        agent_revision: None,
        virtual_user_id: None,
        owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
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
        status: crate::records::SessionStatus::Idle,
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
        parent_session_id: Some(everruns_contracts::typed_id::SessionId::new()),
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
fn test_proto_missing_id_is_missing_field_error() {
    let proto_agent = proto::Agent {
        service_virtual_user_id: None,

        id: None,
        ..Default::default()
    };
    let err = proto_agent_to_schema(proto_agent).unwrap_err();
    assert!(
        matches!(err, ConversionError::MissingField("id")),
        "agent: expected MissingField(\"id\"), got {err:?}"
    );

    let proto_harness = proto::Harness {
        id: None,
        ..Default::default()
    };
    let err = proto_harness_to_schema(proto_harness).unwrap_err();
    assert!(
        matches!(err, ConversionError::MissingField("id")),
        "harness: expected MissingField(\"id\"), got {err:?}"
    );

    let proto_session = proto::Session {
        id: None,
        ..Default::default()
    };
    let err = proto_session_to_schema(proto_session).unwrap_err();
    assert!(
        matches!(err, ConversionError::MissingField("id")),
        "session: expected MissingField(\"id\"), got {err:?}"
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
        service_virtual_user_id: None,

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
        service_virtual_user_id: None,

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
fn test_proto_session_drops_unparseable_capability_but_keeps_valid() {
    use chrono::Utc;
    use everruns_contracts::CapabilityRef as AgentCapabilityConfig;

    let now = Utc::now();
    let session_id = everruns_contracts::typed_id::SessionId::new();
    let session = crate::records::Session {
        playground_user_id: None,
        source: Default::default(),
        activity: Default::default(),
        run_summary: None,
        id: session_id,
        workspace_id: everruns_contracts::typed_id::WorkspaceId::from_uuid(session_id.uuid()),
        organization_id: "org_00000000000000000000000000000001".to_string(),
        harness_id: everruns_contracts::typed_id::HarnessId::new(),
        agent_id: None,
        agent_revision: None,
        virtual_user_id: None,
        owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
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
        status: crate::records::SessionStatus::Idle,
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
