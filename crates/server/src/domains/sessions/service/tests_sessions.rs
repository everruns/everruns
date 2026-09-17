//! Tests: sessions.

use super::*;
use crate::domains::common::Command;
use crate::domains::{agents::types::CreateAgentRequest, harnesses::types::CreateHarnessRequest};
use crate::kernel_imports::{Caller, DEFAULT_ORG_ID, InitialFile};
use crate::services::PrincipalService;
use crate::storage::{StorageBackend, UpdateAgent};

use super::tests_support::*;

#[test]
fn sanitize_session_capabilities_removes_daytona_base_url_overrides() {
    let capabilities = vec![AgentCapabilityConfig::with_config(
        SESSION_SANDBOX_CAPABILITY_ID,
        serde_json::json!({
            "provider": "daytona",
            "provider_config": {
                "api_base": "https://attacker.example",
                "toolbox_base": "https://attacker.example/toolbox",
                "workspace_path": "/home/daytona/workspace",
            }
        }),
    )];

    let sanitized = sanitize_session_capabilities(capabilities);
    let provider_config = sanitized[0]
        .config_value()
        .get("provider_config")
        .and_then(serde_json::Value::as_object)
        .expect("provider_config should be object");

    assert!(!provider_config.contains_key("api_base"));
    assert!(!provider_config.contains_key("toolbox_base"));
    assert_eq!(
        provider_config
            .get("workspace_path")
            .and_then(serde_json::Value::as_str),
        Some("/home/daytona/workspace")
    );
}

#[test]
fn sanitize_session_capabilities_keeps_non_sandbox_capabilities() {
    let capabilities = vec![AgentCapabilityConfig::with_config(
        "shell",
        serde_json::json!({
            "provider_config": {
                "api_base": "https://example.com"
            }
        }),
    )];

    let sanitized = sanitize_session_capabilities(capabilities.clone());
    assert_eq!(sanitized, capabilities);
}

#[tokio::test]
async fn session_list_lookup_count_is_independent_of_page_size() {
    let db = Arc::new(StorageBackend::in_memory());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let ctx = test_ctx(caller.clone(), db.clone()).await;

    let parent = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
        name: "list-parent-harness".to_string(),
        display_name: Some("List Parent Harness".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: Some("parent".to_string()),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(&ctx)
    .await
    .unwrap();
    let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
        name: "list-child-harness".to_string(),
        display_name: Some("List Child Harness".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: Some("child".to_string()),
        parent_harness_id: Some(parent.id),
        default_model_id: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(&ctx)
    .await
    .unwrap();
    let agent = crate::domains::agents::CreateAgent(CreateAgentRequest {
        id: None,
        name: "list-agent".to_string(),
        display_name: Some("List Agent".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: "agent".to_string(),
        default_model_id: None,
        harness_id: None,
        harness_name: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![],
        tools: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
    })
    .execute(&ctx)
    .await
    .unwrap();
    let service = SessionService::new(db.clone());

    for index in 0..20 {
        let mut request = build_create_request(harness.id, Some(agent.public_id), None);
        request.title = Some(format!("List session {index}"));
        service
            .create(
                &caller,
                harness.id.uuid(),
                Some(agent.internal_id),
                Some(agent.public_id),
                SessionSource::Api,
                request,
            )
            .await
            .unwrap();
    }

    db.reset_session_list_lookup_count();
    let (one, _) = service
        .list(
            &caller,
            Some(everruns_platform::ANONYMOUS_USER_ID),
            &SessionListFilters::default(),
            Pagination {
                limit: 1,
                offset: 0,
            },
        )
        .await
        .unwrap();
    let one_lookup_count = db.session_list_lookup_count();
    assert_eq!(one.len(), 1);

    db.reset_session_list_lookup_count();
    let (twenty, _) = service
        .list(
            &caller,
            Some(everruns_platform::ANONYMOUS_USER_ID),
            &SessionListFilters::default(),
            Pagination {
                limit: 20,
                offset: 0,
            },
        )
        .await
        .unwrap();
    let twenty_lookup_count = db.session_list_lookup_count();
    assert_eq!(twenty.len(), 20);

    assert_eq!(
        twenty_lookup_count, one_lookup_count,
        "session-list storage lookups must stay bounded as page size grows"
    );
    assert_eq!(
        twenty_lookup_count, 9,
        "session-list hydration should use the fixed batch-query budget"
    );

    db.set_session_list_lookup_delay_ms(2);
    db.reset_session_list_lookup_count();
    let legacy_started = tokio::time::Instant::now();
    let (legacy_rows, _) = db
        .list_sessions(
            DEFAULT_ORG_ID,
            &SessionListFilters::default(),
            Pagination {
                limit: 20,
                offset: 0,
            },
        )
        .await
        .unwrap();
    let mut legacy_sessions: Vec<Session> = legacy_rows
        .into_iter()
        .map(|row| SessionService::row_to_session(row, &caller.org_public_id, None))
        .collect();
    for session in &mut legacy_sessions {
        service
            .hydrate_ownership(DEFAULT_ORG_ID, session)
            .await
            .unwrap();
    }
    for session in &mut legacy_sessions {
        service
            .resolve_effective_harness(DEFAULT_ORG_ID, session.harness_id)
            .await
            .unwrap();
        service
            .populate_features(DEFAULT_ORG_ID, session)
            .await
            .unwrap();
    }
    for session in &mut legacy_sessions {
        service
            .resolve_session_agent_id(DEFAULT_ORG_ID, session)
            .await
            .unwrap();
    }
    let legacy_ids: Vec<Uuid> = legacy_sessions
        .iter()
        .map(|session| session.id.uuid())
        .collect();
    db.get_session_previews(&legacy_ids).await.unwrap();
    db.get_session_output_previews(&legacy_ids).await.unwrap();
    db.list_pinned_session_ids(everruns_platform::ANONYMOUS_USER_ID, DEFAULT_ORG_ID)
        .await
        .unwrap();
    let legacy_elapsed = legacy_started.elapsed();
    let legacy_lookup_count = db.session_list_lookup_count();

    db.reset_session_list_lookup_count();
    let batched_started = tokio::time::Instant::now();
    service
        .list(
            &caller,
            Some(everruns_platform::ANONYMOUS_USER_ID),
            &SessionListFilters::default(),
            Pagination {
                limit: 20,
                offset: 0,
            },
        )
        .await
        .unwrap();
    let batched_elapsed = batched_started.elapsed();
    let batched_lookup_count = db.session_list_lookup_count();
    db.set_session_list_lookup_delay_ms(0);

    eprintln!(
        "sessions-list benchmark (20 rows, 2ms simulated DB latency): before={legacy_lookup_count} lookups/{legacy_elapsed:?}, after={batched_lookup_count} lookups/{batched_elapsed:?}"
    );
    assert_eq!(legacy_lookup_count, 244);
    assert_eq!(batched_lookup_count, 9);
    assert!(
        batched_elapsed * 5 < legacy_elapsed,
        "batched hydration should be materially faster under cross-cloud latency"
    );
}

#[tokio::test]
async fn session_list_batch_hydration_preserves_response_fields() {
    let db = Arc::new(StorageBackend::in_memory());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let ctx = test_ctx(caller.clone(), db.clone()).await;

    let parent = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
        name: "hydration-parent".to_string(),
        display_name: Some("Hydration Parent".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: Some("parent".to_string()),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: vec![AgentCapabilityConfig::new("session_file_system")],
        initial_files: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(&ctx)
    .await
    .unwrap();
    let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
        name: "hydration-child".to_string(),
        display_name: Some("Hydration Child".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: Some("child".to_string()),
        parent_harness_id: Some(parent.id),
        default_model_id: None,
        tags: vec![],
        capabilities: vec![AgentCapabilityConfig::new("session_tasks")],
        initial_files: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(&ctx)
    .await
    .unwrap();
    let agent = crate::domains::agents::CreateAgent(CreateAgentRequest {
        id: None,
        name: "hydration-agent".to_string(),
        display_name: Some("Hydration Agent".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: "agent".to_string(),
        default_model_id: None,
        harness_id: None,
        harness_name: None,
        tags: vec![],
        capabilities: vec![AgentCapabilityConfig::new("session_schedule")],
        initial_files: vec![],
        tools: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
    })
    .execute(&ctx)
    .await
    .unwrap();
    let service = SessionService::new(db.clone());

    let mut agent_request = build_create_request(harness.id, Some(agent.public_id), None);
    agent_request.title = Some("agent session".to_string());
    agent_request.capabilities = vec![AgentCapabilityConfig::new("session_storage")];
    let agent_session = service
        .create(
            &caller,
            harness.id.uuid(),
            Some(agent.internal_id),
            Some(agent.public_id),
            SessionSource::Api,
            agent_request,
        )
        .await
        .unwrap();
    let no_agent_session = service
        .create(
            &caller,
            harness.id.uuid(),
            None,
            None,
            SessionSource::Api,
            build_create_request(harness.id, None, None),
        )
        .await
        .unwrap();

    for (event_type, text) in [
        ("input.message", "input preview"),
        ("output.message.completed", "output preview"),
    ] {
        db.create_event(CreateEventRow {
            session_id: agent_session.id,
            event_type: event_type.to_string(),
            ts: chrono::Utc::now(),
            context: serde_json::json!({}),
            data: serde_json::json!({
                "message": {"content": [{"type": "text", "text": text}]}
            }),
            metadata: None,
            tags: None,
        })
        .await
        .unwrap();
    }
    db.pin_session(
        everruns_platform::ANONYMOUS_USER_ID,
        agent_session.id,
        DEFAULT_ORG_ID,
    )
    .await
    .unwrap();

    db.update_agent(
        DEFAULT_ORG_ID,
        AgentId::from_uuid(agent.internal_id),
        UpdateAgent {
            status: Some("deleted".to_string()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let missing_agent_id = AgentId::new();
    let missing_owner_id = PrincipalId::new();
    let missing_reference_session = db
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: DEFAULT_ORG_ID,
            app_id: None,
            endpoint_id: None,
            harness_id: Some(harness.id),
            agent_id: Some(missing_agent_id),
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: missing_owner_id,
            resolved_owner_user_id: None,
            title: Some("missing references".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::json!([]),
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            blueprint_id: None,
            blueprint_config: None,
            parent_session_id: None,
            budget_root_session_id: None,
        })
        .await
        .unwrap();

    let (sessions, total) = service
        .list(
            &caller,
            Some(everruns_platform::ANONYMOUS_USER_ID),
            &SessionListFilters::default(),
            Pagination {
                limit: 20,
                offset: 0,
            },
        )
        .await
        .unwrap();
    assert_eq!(total, 3);
    assert_eq!(sessions.len(), 3);
    assert!(
        sessions
            .windows(2)
            .all(|pair| pair[0].created_at >= pair[1].created_at)
    );

    let listed_agent = sessions
        .iter()
        .find(|session| session.id == agent_session.id)
        .unwrap();
    assert_eq!(listed_agent.agent_id, Some(agent.public_id));
    assert!(listed_agent.owner.is_some());
    assert_eq!(listed_agent.preview.as_deref(), Some("input preview"));
    assert_eq!(
        listed_agent.output_preview.as_deref(),
        Some("output preview")
    );
    assert_eq!(listed_agent.is_pinned, Some(true));
    for feature in ["file_system", "session_tasks", "schedules", "key_value"] {
        assert!(
            listed_agent.features.iter().any(|value| value == feature),
            "missing feature {feature}: {:?}",
            listed_agent.features
        );
    }

    let listed_no_agent = sessions
        .iter()
        .find(|session| session.id == no_agent_session.id)
        .unwrap();
    assert_eq!(listed_no_agent.agent_id, None);
    assert_eq!(listed_no_agent.is_pinned, Some(false));
    assert!(
        listed_no_agent
            .features
            .iter()
            .any(|value| value == "file_system")
    );
    assert!(
        listed_no_agent
            .features
            .iter()
            .any(|value| value == "session_tasks")
    );

    let listed_missing = sessions
        .iter()
        .find(|session| session.id == missing_reference_session.id)
        .unwrap();
    assert_eq!(listed_missing.agent_id, Some(missing_agent_id));
    assert!(listed_missing.owner.is_none());

    let (empty_page, empty_total) = service
        .list(
            &caller,
            Some(everruns_platform::ANONYMOUS_USER_ID),
            &SessionListFilters::default(),
            Pagination {
                limit: 20,
                offset: 100,
            },
        )
        .await
        .unwrap();
    assert!(empty_page.is_empty());
    assert_eq!(empty_total, 3);
}

#[tokio::test]
async fn resolved_model_id_tracks_default_and_preserves_explicit_binding() {
    let db = Arc::new(StorageBackend::in_memory());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let _ctx = test_ctx(caller.clone(), db.clone()).await;
    let service = SessionService::new(db.clone());
    let harness_id = org_init::base_harness_id(&db, caller.org_id).await.unwrap();

    let inherited = service
        .create(
            &caller,
            harness_id.uuid(),
            None,
            None,
            SessionSource::Api,
            build_create_request(harness_id, None, None),
        )
        .await
        .unwrap();
    assert_eq!(
        service
            .resolved_model_id(caller.org_id, &inherited)
            .await
            .unwrap(),
        None
    );

    let first_default = create_model(&db, caller.org_id, "first-default").await;
    db.upsert_organization_settings(caller.org_id, Some(first_default.uuid()))
        .await
        .unwrap();
    assert_eq!(
        service
            .resolved_model_id(caller.org_id, &inherited)
            .await
            .unwrap(),
        Some(first_default)
    );

    let second_default = create_model(&db, caller.org_id, "second-default").await;
    db.upsert_organization_settings(caller.org_id, Some(second_default.uuid()))
        .await
        .unwrap();
    assert_eq!(
        service
            .resolved_model_id(caller.org_id, &inherited)
            .await
            .unwrap(),
        Some(second_default)
    );

    let explicit = service
        .create(
            &caller,
            harness_id.uuid(),
            None,
            None,
            SessionSource::Api,
            build_create_request(harness_id, None, Some(first_default)),
        )
        .await
        .unwrap();
    assert_eq!(
        service
            .resolved_model_id(caller.org_id, &explicit)
            .await
            .unwrap(),
        Some(first_default)
    );

    let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
        name: "resolved-model-harness".to_string(),
        display_name: Some("Resolved Model Harness".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: None,
        parent_harness_id: None,
        default_model_id: Some(first_default),
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(&_ctx)
    .await
    .unwrap();
    let blueprint_session = service
        .create_blueprint_session(
            &caller,
            harness.id.uuid(),
            "test-blueprint".to_string(),
            None,
            SessionSource::Api,
            build_create_request(harness.id, None, None),
        )
        .await
        .unwrap();
    assert_eq!(blueprint_session.model_id, None);
    assert_eq!(
        service
            .resolved_model_id(caller.org_id, &blueprint_session)
            .await
            .unwrap(),
        Some(first_default)
    );
}

#[tokio::test]
async fn app_backreference_is_only_set_by_app_session_create() {
    let db = Arc::new(StorageBackend::in_memory());
    let session_service = SessionService::new(db.clone());
    let caller = Caller::internal(1);
    let ctx = test_ctx(caller.clone(), db.clone()).await;

    let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
        name: "app-backref-harness".to_string(),
        display_name: Some("App Backref Harness".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: Some("Harness prompt".to_string()),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(&ctx)
    .await
    .unwrap();

    let app_internal_id = Uuid::now_v7();
    // Build a real user principal to act as the App owner. This must be
    // distinct from the caller-derived default (the system principal that
    // `Caller::internal` resolves to) so the assertions below actually
    // exercise the override codepath rather than coincidentally matching.
    let principal_service = PrincipalService::new(db.clone());
    let user = db
        .create_user(crate::storage::CreateUserRow {
            external_id: None,
            email: "app-owner@example.com".to_string(),
            name: "App Owner".to_string(),
            avatar_url: None,
            roles: vec![],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
        })
        .await
        .unwrap();
    let app_owner = principal_service
        .ensure_user_principal(1, user.id)
        .await
        .unwrap();
    let caller_default_owner = principal_service
        .default_owner_principal(&caller, None)
        .await
        .unwrap();
    assert_ne!(
        app_owner.id, caller_default_owner.id,
        "test setup invariant: app owner must differ from caller-derived default",
    );

    let app_session = session_service
        .create_from_app(
            &caller,
            harness.id.uuid(),
            None,
            None,
            app_internal_id,
            Some(Uuid::new_v4()),
            app_owner.id,
            app_owner.resolved_user_id,
            SessionSource::Api,
            build_create_request(harness.id, None, None),
        )
        .await
        .unwrap();
    let stored_app_session = db
        .get_session(1, app_session.id)
        .await
        .unwrap()
        .expect("app session should be stored");
    assert_eq!(stored_app_session.app_id, Some(app_internal_id));
    // The override actually took effect: stored session is owned by the
    // App's owner, NOT the caller-derived system principal.
    assert_eq!(
        stored_app_session.owner_principal_id, app_owner.id,
        "create_from_app must persist the App's owner_principal_id",
    );
    assert_ne!(
        stored_app_session.owner_principal_id, caller_default_owner.id,
        "create_from_app must override the caller-derived default principal",
    );
    assert_eq!(
        stored_app_session.resolved_owner_user_id, app_owner.resolved_user_id,
        "create_from_app must persist the App's resolved_owner_user_id",
    );

    let normal_session = session_service
        .create(
            &caller,
            harness.id.uuid(),
            None,
            None,
            SessionSource::Api,
            build_create_request(harness.id, None, None),
        )
        .await
        .unwrap();
    let stored_normal_session = db
        .get_session(1, normal_session.id)
        .await
        .unwrap()
        .expect("normal session should be stored");
    assert_eq!(stored_normal_session.app_id, None);
}

#[tokio::test]
async fn fork_copies_history_files_and_records_lineage() {
    let db = Arc::new(StorageBackend::in_memory());
    let session_service = SessionService::new(db.clone());
    let caller = Caller::internal(1);
    let ctx = test_ctx(caller.clone(), db.clone()).await;

    let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
        name: "fork-harness".to_string(),
        display_name: Some("Fork Harness".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: Some("Harness prompt".to_string()),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(&ctx)
    .await
    .unwrap();

    let parent = session_service
        .create(
            &caller,
            harness.id.uuid(),
            None,
            None,
            SessionSource::Api,
            build_create_request(harness.id, None, None),
        )
        .await
        .unwrap();

    // Seed the parent with conversation history and a workspace file.
    for (etype, text) in [
        ("input.message", "hello"),
        ("output.message.completed", "hi there"),
    ] {
        db.create_event(CreateEventRow {
            session_id: parent.id,
            event_type: etype.to_string(),
            ts: chrono::Utc::now(),
            context: serde_json::json!({}),
            data: serde_json::json!({ "text": text }),
            metadata: None,
            tags: None,
        })
        .await
        .unwrap();
    }
    db.create_session_file(CreateSessionFileRow {
        session_id: SessionId::from_uuid(parent.workspace_id.uuid()),
        path: "/notes.txt".to_string(),
        content: Some(b"fork me".to_vec()),
        is_directory: false,
        is_readonly: false,
    })
    .await
    .unwrap();
    db.upsert_session_key_value(UpsertSessionKeyValue {
        session_id: parent.id,
        key: "state".to_string(),
        value: "ready".to_string(),
    })
    .await
    .unwrap();
    db.upsert_session_secret(UpsertSessionSecret {
        session_id: parent.id,
        name: "API_TOKEN".to_string(),
        value_encrypted: b"ciphertext".to_vec(),
    })
    .await
    .unwrap();

    let parent_events = db
        .list_events(parent.id, None, None, &[], &[], None, None)
        .await
        .unwrap();

    let child = session_service
        .fork(
            &caller,
            parent.id,
            ForkOverrides {
                title: Some("Branched".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Independent identity + recorded lineage.
    assert_ne!(child.id, parent.id);
    assert_ne!(child.workspace_id.uuid(), parent.workspace_id.uuid());
    assert_eq!(child.forked_from_session_id, Some(parent.id));
    assert_eq!(
        child.forked_from_sequence,
        parent_events.iter().map(|e| e.sequence).max()
    );
    assert_eq!(child.title.as_deref(), Some("Branched"));

    // History copied verbatim (same count and per-type ordering).
    let child_events = db
        .list_events(child.id, None, None, &[], &[], None, None)
        .await
        .unwrap();
    assert_eq!(child_events.len(), parent_events.len());
    let parent_types: Vec<_> = parent_events.iter().map(|e| e.event_type.clone()).collect();
    let child_types: Vec<_> = child_events.iter().map(|e| e.event_type.clone()).collect();
    assert_eq!(child_types, parent_types);

    // Workspace file copied into the child's isolated workspace.
    let copied = db
        .get_session_file(child.workspace_id.uuid(), "/notes.txt")
        .await
        .unwrap()
        .expect("forked workspace should contain the parent's file");
    assert_eq!(copied.content.as_deref(), Some(b"fork me".as_slice()));

    let copied_kv = db
        .get_session_key_value(child.id.uuid(), "state")
        .await
        .unwrap()
        .expect("forked session should contain KV");
    assert_eq!(copied_kv.value, "ready");
    let copied_secret = db
        .get_session_secret(child.id.uuid(), "API_TOKEN")
        .await
        .unwrap()
        .expect("forked session should contain secret");
    assert_eq!(copied_secret.value_encrypted, b"ciphertext");

    // The parent is untouched.
    let parent_after = db.get_session(1, parent.id).await.unwrap().unwrap();
    assert_eq!(parent_after.forked_from_session_id, None);
}

#[tokio::test]
async fn starter_files_are_copied_into_new_sessions() {
    let db = Arc::new(StorageBackend::in_memory());
    let session_service = SessionService::new(db.clone());
    let caller = Caller::internal(1);
    let ctx = test_ctx(caller.clone(), db.clone()).await;

    let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
        name: "harness".to_string(),
        display_name: Some("Harness".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: Some("Harness prompt".to_string()),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![
            InitialFile {
                path: "/workspace/config.txt".to_string(),
                content: "from harness".to_string(),
                encoding: "text".to_string(),
                is_readonly: false,
            },
            InitialFile {
                path: "/only-harness.txt".to_string(),
                content: "h-only".to_string(),
                encoding: "text".to_string(),
                is_readonly: true,
            },
        ],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(&ctx)
    .await
    .unwrap();

    let agent = crate::domains::agents::CreateAgent(CreateAgentRequest {
        id: None,
        name: "test-agent".to_string(),
        display_name: Some("Test Agent".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: "Agent prompt".to_string(),
        default_model_id: None,
        harness_id: None,
        harness_name: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![
            InitialFile {
                path: "/config.txt".to_string(),
                content: "from agent".to_string(),
                encoding: "text".to_string(),
                is_readonly: false,
            },
            InitialFile {
                path: "/binary.bin".to_string(),
                content: "AAE=".to_string(),
                encoding: "base64".to_string(),
                is_readonly: true,
            },
        ],
        tools: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
    })
    .execute(&ctx)
    .await
    .unwrap();

    let session = session_service
        .create(
            &caller,
            harness.id.uuid(),
            Some(agent.internal_id),
            Some(agent.public_id),
            SessionSource::Api,
            build_create_request(harness.id, Some(agent.public_id), None),
        )
        .await
        .unwrap();

    let file_service = WorkspaceFileService::new(db);
    let config = file_service
        .read_file(session.id.uuid(), "/config.txt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(config.content.as_deref(), Some("from agent"));

    let harness_only = file_service
        .read_file(session.id.uuid(), "/only-harness.txt")
        .await
        .unwrap()
        .unwrap();
    assert!(harness_only.is_readonly);
    assert_eq!(harness_only.content.as_deref(), Some("h-only"));

    let binary = file_service
        .read_file(session.id.uuid(), "/binary.bin")
        .await
        .unwrap()
        .unwrap();
    assert!(binary.is_readonly);
    assert_eq!(binary.encoding, "base64");
    assert_eq!(binary.content.as_deref(), Some("AAE="));
}

#[tokio::test]
async fn scoped_memories_are_auto_created_and_mounted_for_new_sessions() {
    let db = Arc::new(StorageBackend::in_memory());
    let session_service = SessionService::new(db.clone());
    let user = db
        .create_user(crate::storage::CreateUserRow {
            external_id: None,
            email: "memory-owner@example.com".to_string(),
            name: "Memory Owner".to_string(),
            avatar_url: None,
            roles: vec![],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
        })
        .await
        .unwrap();
    let caller = Caller {
        user_id: Some(user.id),
        ..external_caller(DEFAULT_ORG_ID)
    };
    let ctx = test_ctx(caller.clone(), db.clone()).await;

    let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
        name: "scoped-memory-harness".to_string(),
        display_name: Some("Scoped Memory Harness".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: Some("Harness prompt".to_string()),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(&ctx)
    .await
    .unwrap();

    let agent = crate::domains::agents::CreateAgent(CreateAgentRequest {
        id: None,
        name: "scoped-memory-agent".to_string(),
        display_name: Some("Scoped Memory Agent".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: "Agent prompt".to_string(),
        default_model_id: None,
        harness_id: None,
        harness_name: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![],
        tools: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
    })
    .execute(&ctx)
    .await
    .unwrap();

    let session = session_service
        .create(
            &caller,
            harness.id.uuid(),
            Some(agent.internal_id),
            Some(agent.public_id),
            SessionSource::Api,
            build_create_request(harness.id, Some(agent.public_id), None),
        )
        .await
        .unwrap();

    let file_service = WorkspaceFileService::new(db.clone());
    let agent_mount = file_service
        .stat(session.id.uuid(), AGENT_MEMORY_MOUNT_PATH)
        .await
        .unwrap()
        .expect("agent memory mount exists");
    assert!(agent_mount.is_directory);
    assert!(!agent_mount.is_readonly);

    let user_mount = file_service
        .stat(session.id.uuid(), USER_MEMORY_MOUNT_PATH)
        .await
        .unwrap()
        .expect("user memory mount exists");
    assert!(user_mount.is_directory);
    assert!(!user_mount.is_readonly);

    assert!(
        db.get_memory_by_scope_owner(
            DEFAULT_ORG_ID,
            "agent",
            Some(AgentId::from_uuid(agent.internal_id)),
            None,
        )
        .await
        .unwrap()
        .is_some()
    );
    assert!(
        db.get_memory_by_scope_owner(DEFAULT_ORG_ID, "user", None, Some(user.id))
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        db.list_memories(DEFAULT_ORG_ID, None, false)
            .await
            .unwrap()
            .is_empty(),
        "scoped memories stay hidden from org memory listing"
    );
}

#[tokio::test]
async fn session_initial_files_cannot_claim_reserved_memory_paths() {
    let db = Arc::new(StorageBackend::in_memory());
    let session_service = SessionService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let ctx = test_ctx(caller.clone(), db.clone()).await;

    let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
        name: "reserved-memory-path-harness".to_string(),
        display_name: Some("Reserved Memory Path Harness".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: Some("Harness prompt".to_string()),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(&ctx)
    .await
    .unwrap();

    let mut req = build_create_request(harness.id, None, None);
    req.initial_files.push(InitialFile {
        path: "/memory/agent/profile.md".to_string(),
        content: "owned by the server".to_string(),
        encoding: "text".to_string(),
        is_readonly: false,
    });

    let err = session_service
        .create(
            &caller,
            harness.id.uuid(),
            None,
            None,
            SessionSource::Api,
            req,
        )
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("reserved for server-managed memory"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn inherited_harness_starter_files_are_copied_into_new_sessions() {
    let db = Arc::new(StorageBackend::in_memory());
    let session_service = SessionService::new(db.clone());
    let caller = Caller::internal(1);
    let ctx = test_ctx(caller.clone(), db.clone()).await;

    let parent = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
        name: "parent".to_string(),
        display_name: Some("Parent".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: Some("Parent prompt".to_string()),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![
            InitialFile {
                path: "/workspace/config.txt".to_string(),
                content: "from parent".to_string(),
                encoding: "text".to_string(),
                is_readonly: false,
            },
            InitialFile {
                path: "/parent-only.txt".to_string(),
                content: "only parent".to_string(),
                encoding: "text".to_string(),
                is_readonly: true,
            },
        ],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(&ctx)
    .await
    .unwrap();

    let child = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
        name: "child".to_string(),
        display_name: Some("Child".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: Some("Child prompt".to_string()),
        parent_harness_id: Some(parent.id),
        default_model_id: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![InitialFile {
            path: "/config.txt".to_string(),
            content: "from child".to_string(),
            encoding: "text".to_string(),
            is_readonly: true,
        }],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(&ctx)
    .await
    .unwrap();

    let session = session_service
        .create(
            &caller,
            child.id.uuid(),
            None,
            None,
            SessionSource::Api,
            build_create_request(child.id, None, None),
        )
        .await
        .unwrap();

    let file_service = WorkspaceFileService::new(db);
    let config = file_service
        .read_file(session.id.uuid(), "/config.txt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(config.content.as_deref(), Some("from child"));

    let parent_only = file_service
        .read_file(session.id.uuid(), "/parent-only.txt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(parent_only.content.as_deref(), Some("only parent"));
}

#[tokio::test]
async fn archived_dependencies_cannot_be_assigned_in_dev_mode() {
    let db = Arc::new(StorageBackend::in_memory());
    let session_service = SessionService::new(db.clone());
    let caller = Caller::internal(1);
    let ctx = test_ctx(caller.clone(), db.clone()).await;

    let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
        name: "harness".to_string(),
        display_name: Some("Harness".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: Some("Harness prompt".to_string()),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(&ctx)
    .await
    .unwrap();

    let agent = crate::domains::agents::CreateAgent(CreateAgentRequest {
        id: None,
        name: "test-agent".to_string(),
        display_name: Some("Test Agent".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: "Agent prompt".to_string(),
        default_model_id: None,
        harness_id: None,
        harness_name: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![],
        tools: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
    })
    .execute(&ctx)
    .await
    .unwrap();

    crate::domains::harnesses::DeleteHarness {
        id: harness.id.to_string(),
    }
    .execute(&ctx)
    .await
    .unwrap();
    let error = session_service
        .create(
            &caller,
            harness.id.uuid(),
            Some(agent.internal_id),
            Some(agent.public_id),
            SessionSource::Api,
            build_create_request(harness.id, Some(agent.public_id), None),
        )
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Archived or deleted harnesses cannot be assigned")
    );

    let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
        name: "harness-2".to_string(),
        display_name: Some("Harness 2".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: Some("Harness prompt".to_string()),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(&ctx)
    .await
    .unwrap();
    crate::domains::agents::DeleteAgent {
        id: agent.public_id.to_string(),
    }
    .execute(&ctx)
    .await
    .unwrap();

    let error = session_service
        .create(
            &caller,
            harness.id.uuid(),
            Some(agent.internal_id),
            Some(agent.public_id),
            SessionSource::Api,
            build_create_request(harness.id, Some(agent.public_id), None),
        )
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Archived or deleted agents cannot be assigned")
    );
}
