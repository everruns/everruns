use super::super::models::*;
use super::*;
use crate::api::common::Pagination;
use chrono::Utc;
use everruns_core::message_filter::{MessageFilter, MessageQuery};
use everruns_core::{AgentId, AgentVersionId, DEFAULT_ORG_ID, HarnessId, PrincipalId, SessionId};
use everruns_platform::{SessionParticipantKind, SessionParticipantRole};

/// Default pagination for tests (large enough to not truncate).
fn default_pagination() -> Pagination {
    Pagination::new(0, 1000)
}

fn test_harness_id() -> HarnessId {
    HarnessId::from_uuid(uuid::Uuid::nil())
}

fn test_session_input(agent_id: Option<AgentId>) -> CreateSessionRow {
    CreateSessionRow {
        source: everruns_platform::SessionSource::Api,
        workspace_id: None,
        org_id: DEFAULT_ORG_ID,
        app_id: None,
        harness_id: None,
        agent_id,
        agent_version_id: None,
        agent_config_hash: None,
        agent_identity_id: None,
        owner_principal_id: PrincipalId::from_seed(1),
        resolved_owner_user_id: None,
        title: None,
        locale: None,
        tags: vec![],
        model_id: None,
        capabilities: serde_json::json!([]),
        tools: serde_json::json!([]),
        mcp_servers: serde_json::json!({}),
        system_prompt: None,
        initial_files: serde_json::Value::Array(vec![]),
        hints: None,
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        blueprint_id: None,
        blueprint_config: None,
        parent_session_id: None,
        budget_root_session_id: None,
    }
}

#[tokio::test]
async fn test_create_and_get_agent() {
    let db = InMemoryDatabase::new();

    let agent = db
        .create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: "test-agent".to_string(),
                display_name: Some("Test Agent".to_string()),
                description: Some("A test agent".to_string()),
                system_prompt: "You are helpful".to_string(),
                default_model_id: None,

                harness_id: test_harness_id(),
                tags: vec!["test".to_string()],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(agent.name, "test-agent");
    assert_eq!(agent.display_name, Some("Test Agent".to_string()));

    let fetched = db.get_agent(DEFAULT_ORG_ID, agent.id).await.unwrap();
    assert!(fetched.is_some());
    let fetched = fetched.unwrap();
    assert_eq!(fetched.name, "test-agent");
    assert_eq!(fetched.display_name, Some("Test Agent".to_string()));
}

#[tokio::test]
async fn test_declarative_capability_storage_searches_name_and_display_name() {
    let db = InMemoryDatabase::new();
    let row = db
        .create_declarative_capability(
            DEFAULT_ORG_ID,
            CreateDeclarativeCapabilityRow {
                public_id: everruns_core::DeclarativeCapabilityId::new().to_string(),
                name: "research_pack".to_string(),
                display_name: Some("Research Pack".to_string()),
                description: "Curated research defaults".to_string(),
                definition: serde_json::json!({
                    "name": "research_pack",
                    "display_name": "Research Pack",
                    "description": "Curated research defaults"
                }),
            },
        )
        .await
        .unwrap();

    assert!(row.public_id.starts_with("cap_"));
    assert_eq!(row.display_name.as_deref(), Some("Research Pack"));

    let by_public_id = db
        .get_declarative_capability_by_public_id(DEFAULT_ORG_ID, &row.public_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(by_public_id.name, "research_pack");

    let by_display_name = db
        .list_declarative_capabilities(DEFAULT_ORG_ID, Some("research"), false)
        .await
        .unwrap();
    assert_eq!(by_display_name.len(), 1);

    let updated = db
        .update_declarative_capability(
            DEFAULT_ORG_ID,
            row.id,
            UpdateDeclarativeCapability {
                display_name: None,
                definition: Some(serde_json::json!({
                    "name": "research_pack",
                    "description": "Curated research defaults"
                })),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.display_name, None);
}

#[tokio::test]
async fn test_create_and_list_sessions() {
    let db = InMemoryDatabase::new();

    let agent = db
        .create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: "test-agent".to_string(),
                display_name: Some("Test Agent".to_string()),
                description: None,
                system_prompt: String::new(),
                default_model_id: None,

                harness_id: test_harness_id(),
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
            },
        )
        .await
        .unwrap();

    let session = db
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: DEFAULT_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: everruns_core::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            title: Some("Test Session".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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

    let pagination = crate::api::common::Pagination::new(0, 20);
    let (sessions, total) = db
        .list_sessions(
            DEFAULT_ORG_ID,
            &SessionListFilters {
                agent_id: Some(agent.id),
                ..Default::default()
            },
            pagination,
        )
        .await
        .unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(total, 1);
    assert_eq!(sessions[0].id, session.id);
}

#[tokio::test]
async fn test_set_session_fork_lineage_roundtrip() {
    let db = InMemoryDatabase::new();

    let new_session = || CreateSessionRow {
        source: everruns_platform::SessionSource::Api,
        workspace_id: None,
        org_id: DEFAULT_ORG_ID,
        app_id: None,
        harness_id: None,
        agent_id: None,
        agent_version_id: None,
        agent_config_hash: None,
        agent_identity_id: None,
        owner_principal_id: everruns_core::PrincipalId::from_seed(1),
        resolved_owner_user_id: None,
        title: None,
        locale: None,
        tags: vec![],
        model_id: None,
        capabilities: serde_json::json!([]),
        tools: serde_json::json!([]),
        mcp_servers: serde_json::json!({}),
        system_prompt: None,
        initial_files: serde_json::Value::Array(vec![]),
        hints: None,
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        blueprint_id: None,
        blueprint_config: None,
        parent_session_id: None,
        budget_root_session_id: None,
    };

    let parent = db.create_session(new_session()).await.unwrap();
    let child = db.create_session(new_session()).await.unwrap();

    // Fresh sessions have no fork lineage.
    assert_eq!(child.forked_from_session_id, None);
    assert_eq!(child.forked_from_sequence, None);

    db.set_session_fork_lineage(child.id, parent.id, Some(7))
        .await
        .unwrap();

    let reloaded_child = db
        .get_session(DEFAULT_ORG_ID, child.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reloaded_child.forked_from_session_id, Some(parent.id));
    assert_eq!(reloaded_child.forked_from_sequence, Some(7));

    // The parent is untouched.
    let reloaded_parent = db
        .get_session(DEFAULT_ORG_ID, parent.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reloaded_parent.forked_from_session_id, None);
}

#[tokio::test]
async fn detached_budget_root_override_is_canonical_and_org_scoped() {
    let db = InMemoryDatabase::new();
    let root = db
        .create_session(test_session_input(None))
        .await
        .expect("root session");

    let mut detached_input = test_session_input(None);
    detached_input.budget_root_session_id = Some(root.id);
    let detached = db
        .create_session(detached_input)
        .await
        .expect("detached peer");
    assert_eq!(detached.parent_session_id, None);
    assert_eq!(detached.root_session_id, Some(root.id));

    let mut chain_input = test_session_input(None);
    chain_input.budget_root_session_id = Some(detached.id);
    let chained = db
        .create_session(chain_input)
        .await
        .expect("detached chain");
    assert_eq!(chained.root_session_id, Some(root.id));

    // A normal fork has lineage but no internal budget-root override, so its
    // storage root remains independent.
    let ordinary_fork = db
        .create_session(test_session_input(None))
        .await
        .expect("ordinary fork storage row");
    assert_eq!(ordinary_fork.root_session_id, Some(ordinary_fork.id));

    let mut cross_org = test_session_input(None);
    cross_org.org_id = DEFAULT_ORG_ID + 1;
    cross_org.budget_root_session_id = Some(root.id);
    let error = db
        .create_session(cross_org)
        .await
        .expect_err("cross-org budget linkage must be rejected");
    assert!(error.to_string().contains("not found in organization"));
}

#[tokio::test]
async fn test_create_session_seeds_agent_and_user_participants() {
    let db = InMemoryDatabase::new();
    let agent_id = AgentId::new();

    let session = db
        .create_session(test_session_input(Some(agent_id)))
        .await
        .unwrap();
    let participants = db
        .list_session_participants(DEFAULT_ORG_ID, session.id)
        .await
        .unwrap();

    assert_eq!(participants.len(), 2);
    assert_eq!(session.agent_id, Some(agent_id));

    let host = participants
        .iter()
        .map(SessionParticipantRow::to_core)
        .find(|participant| participant.role == SessionParticipantRole::Host)
        .unwrap();
    assert_eq!(host.kind, SessionParticipantKind::Agent);
    assert_eq!(host.agent_id, Some(agent_id));
    assert_eq!(host.principal_id, PrincipalId::from_seed(1));

    let user = participants
        .iter()
        .map(SessionParticipantRow::to_core)
        .find(|participant| participant.kind == SessionParticipantKind::User)
        .unwrap();
    assert_eq!(user.role, SessionParticipantRole::Member);
    assert_eq!(user.agent_id, None);
    assert_eq!(user.principal_id, PrincipalId::from_seed(1));
    assert_eq!(user.display_name.as_deref(), Some("User"));
}

#[tokio::test]
async fn test_user_participant_uses_and_tracks_profile_name() {
    let db = InMemoryDatabase::new();
    let user = db
        .create_user(CreateUserRow {
            email: "mykhailo@example.com".to_string(),
            name: "Mykhailo Chalyi".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .unwrap();
    let principal = db
        .create_principal(CreatePrincipalRow {
            id: PrincipalId::new(),
            org_id: DEFAULT_ORG_ID,
            kind: "user".to_string(),
            subject_id: Some(user.id),
            parent_principal_id: None,
            resolved_user_id: Some(user.id),
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();
    let mut input = test_session_input(None);
    input.owner_principal_id = principal.id;
    input.resolved_owner_user_id = Some(user.id);

    let session = db.create_session(input).await.unwrap();
    let initial = db
        .list_session_participants(DEFAULT_ORG_ID, session.id)
        .await
        .unwrap();
    assert_eq!(initial[0].display_name.as_deref(), Some("Mykhailo Chalyi"));

    db.update_user(
        user.id,
        UpdateUser {
            name: Some("Mike Chalyi".to_string()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let updated = db
        .list_session_participants(DEFAULT_ORG_ID, session.id)
        .await
        .unwrap();
    assert_eq!(updated[0].display_name.as_deref(), Some("Mike Chalyi"));
}

#[tokio::test]
async fn test_create_session_seeds_agent_participant_version() {
    let db = InMemoryDatabase::new();
    let agent_id = AgentId::new();
    let agent_version_id = AgentVersionId::new();
    let mut input = test_session_input(Some(agent_id));
    input.agent_version_id = Some(agent_version_id);
    input.agent_config_hash = Some("config-hash".to_string());

    let session = db.create_session(input).await.unwrap();
    let participants = db
        .list_session_participants(DEFAULT_ORG_ID, session.id)
        .await
        .unwrap();

    assert_eq!(session.agent_version_id, Some(agent_version_id));
    let host = participants
        .iter()
        .map(SessionParticipantRow::to_core)
        .find(|participant| participant.role == SessionParticipantRole::Host)
        .unwrap();
    assert_eq!(host.agent_id, Some(agent_id));
    assert_eq!(host.agent_version_id, Some(agent_version_id));
}

#[tokio::test]
async fn test_create_session_without_agent_seeds_user_participant_only() {
    let db = InMemoryDatabase::new();

    let session = db.create_session(test_session_input(None)).await.unwrap();
    let participants = db
        .list_session_participants(DEFAULT_ORG_ID, session.id)
        .await
        .unwrap();

    assert_eq!(participants.len(), 1);
    let participant = participants[0].to_core();
    assert_eq!(participant.kind, SessionParticipantKind::User);
    assert_eq!(participant.role, SessionParticipantRole::Member);
    assert_eq!(participant.agent_id, None);
}

#[tokio::test]
async fn test_create_session_participant_rejects_second_active_host() {
    let db = InMemoryDatabase::new();
    let agent_id = AgentId::new();

    let session = db
        .create_session(test_session_input(Some(agent_id)))
        .await
        .unwrap();

    let err = db
        .create_session_participant(CreateSessionParticipantRow {
            org_id: DEFAULT_ORG_ID,
            session_id: session.id,
            kind: SessionParticipantKind::Agent,
            agent_id: Some(AgentId::new()),
            agent_version_id: None,
            principal_id: PrincipalId::from_seed(1),
            display_name: None,
            role: SessionParticipantRole::Host,
            joined_at: None,
        })
        .await
        .unwrap_err();

    assert!(
        err.to_string()
            .contains("session already has an active host participant")
    );
}

#[tokio::test]
async fn test_ensure_active_user_session_participant_is_idempotent() {
    let db = InMemoryDatabase::new();
    let session = db.create_session(test_session_input(None)).await.unwrap();
    let principal_id = PrincipalId::from_seed(42);

    let input = CreateSessionParticipantRow {
        org_id: DEFAULT_ORG_ID,
        session_id: session.id,
        kind: SessionParticipantKind::User,
        agent_id: None,
        agent_version_id: None,
        principal_id,
        display_name: Some("Alice".to_string()),
        role: SessionParticipantRole::Member,
        joined_at: None,
    };
    let first = db
        .ensure_active_user_session_participant(input.clone())
        .await
        .unwrap();
    let second = db
        .ensure_active_user_session_participant(input)
        .await
        .unwrap();

    assert_eq!(first.id, second.id);
    assert_eq!(second.display_name.as_deref(), Some("Alice"));
    let active_for_principal = db
        .list_session_participants(DEFAULT_ORG_ID, session.id)
        .await
        .unwrap()
        .into_iter()
        .filter(|row| {
            row.kind == "user" && row.principal_id == principal_id && row.left_at.is_none()
        })
        .count();
    assert_eq!(active_for_principal, 1);
}

#[tokio::test]
async fn test_leave_session_participant_preserves_history() {
    let db = InMemoryDatabase::new();
    let session = db.create_session(test_session_input(None)).await.unwrap();
    let member = db
        .create_session_participant(CreateSessionParticipantRow {
            org_id: DEFAULT_ORG_ID,
            session_id: session.id,
            kind: SessionParticipantKind::Agent,
            agent_id: Some(AgentId::new()),
            agent_version_id: None,
            principal_id: PrincipalId::from_seed(1),
            display_name: None,
            role: SessionParticipantRole::Member,
            joined_at: None,
        })
        .await
        .unwrap();

    let left = db
        .leave_session_participant(DEFAULT_ORG_ID, session.id, member.id)
        .await
        .unwrap()
        .expect("participant should exist");
    assert!(left.left_at.is_some());

    let participants = db
        .list_session_participants(DEFAULT_ORG_ID, session.id)
        .await
        .unwrap();
    assert_eq!(participants.len(), 2);
    assert_eq!(
        participants
            .iter()
            .find(|row| row.id == member.id)
            .and_then(|row| row.left_at),
        left.left_at
    );
}

#[tokio::test]
async fn test_session_aggregate_stats_by_agent_and_harness() {
    let db = InMemoryDatabase::new();

    let agent = db
        .create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: "stats-agent".to_string(),
                display_name: Some("Stats Agent".to_string()),
                description: None,
                system_prompt: String::new(),
                default_model_id: None,

                harness_id: test_harness_id(),
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
            },
        )
        .await
        .unwrap();
    let harness = db
        .create_harness(
            DEFAULT_ORG_ID,
            CreateHarnessRow {
                name: "stats-harness".to_string(),
                display_name: Some("Stats Harness".to_string()),
                description: None,
                system_prompt: Some(String::new()),
                parent_harness_id: None,
                default_model_id: None,
                tags: vec![],
                initial_files: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                embedder_metadata: serde_json::json!({}),
                is_built_in: false,
            },
        )
        .await
        .unwrap();

    let session = db
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: DEFAULT_ORG_ID,
            app_id: None,
            harness_id: Some(harness.id),
            agent_id: Some(agent.id),
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: everruns_core::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            title: Some("Stats Session".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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
    let started_at = Utc::now() - chrono::Duration::seconds(10);
    let finished_at = started_at + chrono::Duration::seconds(4);
    db.update_session(
        DEFAULT_ORG_ID,
        session.id,
        UpdateSession {
            status: Some("idle".to_string()),
            started_at: Some(started_at),
            finished_at: Some(finished_at),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    db.create_event(CreateEventRow {
        session_id: session.id,
        event_type: "turn.started".to_string(),
        ts: started_at,
        context: serde_json::json!({}),
        data: serde_json::json!({}),
        metadata: None,
        tags: None,
    })
    .await
    .unwrap();

    let stats = db
        .session_aggregate_stats(DEFAULT_ORG_ID, Some(agent.id), None)
        .await
        .unwrap();
    assert_eq!(stats.session_count, 1);
    assert_eq!(stats.idle_session_count, 1);
    assert_eq!(stats.execution_count, 1);
    assert_eq!(stats.total_session_duration_ms, 4000);
    assert_eq!(stats.last_execution_at, Some(started_at));

    let harness_stats = db
        .session_aggregate_stats(DEFAULT_ORG_ID, None, Some(harness.id))
        .await
        .unwrap();
    assert_eq!(harness_stats.session_count, 1);
    assert_eq!(harness_stats.execution_count, 1);
}

#[tokio::test]
async fn test_session_updated_at() {
    let db = InMemoryDatabase::new();

    let agent = db
        .create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: "test-agent".to_string(),
                display_name: Some("Test Agent".to_string()),
                description: None,
                system_prompt: String::new(),
                default_model_id: None,

                harness_id: test_harness_id(),
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
            },
        )
        .await
        .unwrap();

    // Create session - updated_at should equal created_at
    let session = db
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: DEFAULT_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: everruns_core::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            title: Some("Test Session".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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

    assert_eq!(session.created_at, session.updated_at);
    let original_updated_at = session.updated_at;

    // Small delay to ensure different timestamp
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    // Update session - updated_at should change
    let updated = db
        .update_session(
            DEFAULT_ORG_ID,
            session.id,
            UpdateSession {
                title: Some("Updated Title".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();

    assert!(updated.updated_at > original_updated_at);
    assert_eq!(updated.title, Some("Updated Title".to_string()));
}

#[tokio::test]
async fn test_events_sequence() {
    use chrono::Utc;

    let db = InMemoryDatabase::new();

    let agent = db
        .create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: "test-agent".to_string(),
                display_name: Some("Test Agent".to_string()),
                description: None,
                system_prompt: String::new(),
                default_model_id: None,

                harness_id: test_harness_id(),
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
            },
        )
        .await
        .unwrap();

    let session = db
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: DEFAULT_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: everruns_core::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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

    // Create multiple events
    for i in 0..3 {
        db.create_event(CreateEventRow {
            session_id: session.id,
            event_type: "input.message".to_string(),
            ts: Utc::now(),
            context: serde_json::json!({}),
            data: serde_json::json!({"content": format!("Message {}", i)}),
            metadata: None,
            tags: None,
        })
        .await
        .unwrap();
    }

    let events = db
        .list_events(session.id, None, None, &[], &[], None, None)
        .await
        .unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].sequence, 1);
    assert_eq!(events[1].sequence, 2);
    assert_eq!(events[2].sequence, 3);
}

#[tokio::test]
async fn test_list_message_events_filtered_keep_head_loads_head_and_tail() {
    use chrono::Utc;

    let db = InMemoryDatabase::new();

    let agent = db
        .create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: "test-agent".to_string(),
                display_name: Some("Test Agent".to_string()),
                description: None,
                system_prompt: String::new(),
                default_model_id: None,

                harness_id: test_harness_id(),
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
            },
        )
        .await
        .unwrap();

    let session = db
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: DEFAULT_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: everruns_core::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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

    // Six messages, far more than the tail window.
    for i in 0..6 {
        db.create_event(CreateEventRow {
            session_id: session.id,
            event_type: "input.message".to_string(),
            ts: Utc::now(),
            context: serde_json::json!({}),
            data: serde_json::json!({"content": format!("Message {}", i)}),
            metadata: None,
            tags: None,
        })
        .await
        .unwrap();
    }

    // limit=2 (tail) + keep_head=1 (anchor): expect sequences [1, 5, 6].
    let query = MessageQuery::new(session.id)
        .with_limit(2)
        .with_keep_head(1);
    let events = db.list_message_events_filtered(&query).await.unwrap();
    let seqs: Vec<i32> = events.iter().map(|e| e.sequence).collect();
    assert_eq!(seqs, vec![1, 5, 6]);

    // keep_head=0 stays tail-only: latest 2.
    let tail_only = MessageQuery::new(session.id)
        .with_limit(2)
        .with_keep_head(0);
    let events = db.list_message_events_filtered(&tail_only).await.unwrap();
    let seqs: Vec<i32> = events.iter().map(|e| e.sequence).collect();
    assert_eq!(seqs, vec![5, 6]);

    // Overlapping windows must not duplicate: keep_head + limit >= total.
    let overlap = MessageQuery::new(session.id)
        .with_limit(5)
        .with_keep_head(3);
    let events = db.list_message_events_filtered(&overlap).await.unwrap();
    let seqs: Vec<i32> = events.iter().map(|e| e.sequence).collect();
    assert_eq!(seqs, vec![1, 2, 3, 4, 5, 6]);
}

#[tokio::test]
async fn test_list_message_events_filtered_caps_unbounded_history() {
    // A session larger than MESSAGE_SAFETY_LIMIT must not return an unbounded
    // result set from the (offset=None, limit=None) full-history branch. The
    // cap keeps the most recent N rows so the prompt window stays anchored to
    // recent history.
    let cap = crate::storage::repository::MESSAGE_SAFETY_LIMIT;
    let total = cap + 25;

    let db = InMemoryDatabase::new();
    let session = db
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: DEFAULT_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: everruns_core::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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

    for i in 0..total {
        db.create_event(CreateEventRow {
            session_id: session.id,
            event_type: "input.message".to_string(),
            ts: Utc::now(),
            context: serde_json::json!({}),
            data: serde_json::json!({"content": format!("Message {}", i)}),
            metadata: None,
            tags: None,
        })
        .await
        .unwrap();
    }

    // Unbounded query (no offset, no limit): capped to MESSAGE_SAFETY_LIMIT.
    let query = MessageQuery::new(session.id);
    let events = db.list_message_events_filtered(&query).await.unwrap();
    assert_eq!(events.len(), cap, "unbounded read must be capped");
    // Cap keeps the most recent rows in ascending sequence order.
    assert_eq!(events.first().unwrap().sequence, (total - cap + 1) as i32);
    assert_eq!(events.last().unwrap().sequence, total as i32);

    // The non-filtered full-history read is capped identically.
    let limited = db
        .list_message_events_limited(session.id, None)
        .await
        .unwrap();
    assert_eq!(limited.len(), cap);
    assert_eq!(limited.first().unwrap().sequence, (total - cap + 1) as i32);
    assert_eq!(limited.last().unwrap().sequence, total as i32);
}

#[tokio::test]
async fn test_session_connection_resolution_uses_resolved_owner_user() {
    let db = InMemoryDatabase::new();

    let owner = db
        .create_user(CreateUserRow {
            email: format!("owner-{}@example.com", Uuid::now_v7()),
            name: "Owner".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .unwrap();
    let other = db
        .create_user(CreateUserRow {
            email: format!("other-{}@example.com", Uuid::now_v7()),
            name: "Other".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .unwrap();

    db.add_organization_member(DEFAULT_ORG_ID, owner.id, "member")
        .await
        .unwrap();
    db.add_organization_member(DEFAULT_ORG_ID, other.id, "member")
        .await
        .unwrap();

    let session = db
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: DEFAULT_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: everruns_core::PrincipalId::from_seed(42),
            resolved_owner_user_id: Some(owner.id),
            title: Some("connection-owner-scope".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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

    db.upsert_user_connection(CreateUserConnectionRow {
        user_id: other.id,
        provider: "gitlab".to_string(),
        connection_type: "oauth".to_string(),
        provider_user_id: Some("other-gitlab".to_string()),
        provider_username: Some("other".to_string()),
        access_token_encrypted: Some(b"other-token".to_vec()),
        refresh_token_encrypted: None,
        scopes: Some("api".to_string()),
        expires_at: None,
        installation_id: None,
        provider_metadata: Some(serde_json::json!({ "user": "other" })),
    })
    .await
    .unwrap();
    db.upsert_user_connection(CreateUserConnectionRow {
        user_id: other.id,
        provider: "github".to_string(),
        connection_type: "oauth".to_string(),
        provider_user_id: Some("other-github".to_string()),
        provider_username: Some("other".to_string()),
        access_token_encrypted: None,
        refresh_token_encrypted: None,
        scopes: Some("contents:read".to_string()),
        expires_at: None,
        installation_id: Some(222),
        provider_metadata: None,
    })
    .await
    .unwrap();

    assert_eq!(
        db.get_connection_token_for_session(session.id, "gitlab")
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        db.get_connection_metadata_for_session(session.id, "gitlab")
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        db.get_connection_user_for_session(session.id, "gitlab")
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        db.get_installation_id_for_session(session.id, "github")
            .await
            .unwrap(),
        None
    );

    db.upsert_user_connection(CreateUserConnectionRow {
        user_id: owner.id,
        provider: "gitlab".to_string(),
        connection_type: "oauth".to_string(),
        provider_user_id: Some("owner-gitlab".to_string()),
        provider_username: Some("owner".to_string()),
        access_token_encrypted: Some(b"owner-token".to_vec()),
        refresh_token_encrypted: None,
        scopes: Some("api".to_string()),
        expires_at: None,
        installation_id: None,
        provider_metadata: Some(serde_json::json!({ "user": "owner" })),
    })
    .await
    .unwrap();
    db.upsert_user_connection(CreateUserConnectionRow {
        user_id: owner.id,
        provider: "github".to_string(),
        connection_type: "oauth".to_string(),
        provider_user_id: Some("owner-github".to_string()),
        provider_username: Some("owner".to_string()),
        access_token_encrypted: None,
        refresh_token_encrypted: None,
        scopes: Some("contents:read".to_string()),
        expires_at: None,
        installation_id: Some(111),
        provider_metadata: None,
    })
    .await
    .unwrap();

    assert_eq!(
        db.get_connection_token_for_session(session.id, "gitlab")
            .await
            .unwrap(),
        Some(b"owner-token".to_vec())
    );
    assert_eq!(
        db.get_connection_metadata_for_session(session.id, "gitlab")
            .await
            .unwrap(),
        Some(serde_json::json!({ "user": "owner" }))
    );
    assert_eq!(
        db.get_connection_user_for_session(session.id, "gitlab")
            .await
            .unwrap(),
        Some(owner.id)
    );
    assert_eq!(
        db.get_installation_id_for_session(session.id, "github")
            .await
            .unwrap(),
        Some(111)
    );
}

#[tokio::test]
async fn test_unpin_session_is_scoped_by_org() {
    let db = InMemoryDatabase::new();
    let user_id = Uuid::now_v7();

    let session = db
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: DEFAULT_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: everruns_core::PrincipalId::from_seed(1),
            resolved_owner_user_id: Some(user_id),
            title: Some("Pinned Session".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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

    db.pin_session(user_id, session.id, DEFAULT_ORG_ID)
        .await
        .unwrap();

    let removed_wrong_org = db
        .unpin_session(user_id, session.id, DEFAULT_ORG_ID + 1)
        .await
        .unwrap();
    assert!(!removed_wrong_org);

    let pinned_after_wrong_org = db
        .list_pinned_session_ids(user_id, DEFAULT_ORG_ID)
        .await
        .unwrap();
    assert_eq!(pinned_after_wrong_org, vec![session.id]);

    let removed_correct_org = db
        .unpin_session(user_id, session.id, DEFAULT_ORG_ID)
        .await
        .unwrap();
    assert!(removed_correct_org);
}

/// Helper: create an agent + session for event filter tests.
async fn create_session_with_events(db: &InMemoryDatabase) -> SessionId {
    let agent = db
        .create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: "filter-test-agent".to_string(),
                display_name: Some("Filter Test Agent".to_string()),
                description: None,
                system_prompt: String::new(),
                default_model_id: None,

                harness_id: test_harness_id(),
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
            },
        )
        .await
        .unwrap();

    let session = db
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: DEFAULT_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: everruns_core::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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

    // Create events of different types
    for event_type in [
        "input.message",
        "output.message.delta",
        "output.message.completed",
        "turn.started",
        "turn.completed",
        "reason.thinking.delta",
    ] {
        db.create_event(CreateEventRow {
            session_id: session.id,
            event_type: event_type.to_string(),
            ts: Utc::now(),
            context: serde_json::json!({}),
            data: serde_json::json!({}),
            metadata: None,
            tags: None,
        })
        .await
        .unwrap();
    }

    session.id
}

#[tokio::test]
async fn test_count_events_no_materialization() {
    let db = InMemoryDatabase::new();
    let session_id = create_session_with_events(&db).await;

    // Total count (6 events created by helper)
    let count = db.count_events(session_id, &[]).await.unwrap();
    assert_eq!(count, 6);

    // Count excluding delta types
    let count = db
        .count_events(
            session_id,
            &[
                "output.message.delta".to_string(),
                "reason.thinking.delta".to_string(),
            ],
        )
        .await
        .unwrap();
    assert_eq!(count, 4); // 6 - 2 delta types

    // Count for non-existent session
    let other_session = SessionId::new();
    let count = db.count_events(other_session, &[]).await.unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn test_list_events_filter_types_positive() {
    let db = InMemoryDatabase::new();
    let session_id = create_session_with_events(&db).await;

    // Positive filter: only turn events
    let events = db
        .list_events(
            session_id,
            None,
            None,
            &["turn.started".to_string(), "turn.completed".to_string()],
            &[],
            None,
            None,
        )
        .await
        .unwrap();

    assert_eq!(events.len(), 2);
    assert!(events.iter().all(|e| e.event_type.starts_with("turn.")));
}

#[tokio::test]
async fn test_list_events_filter_types_single() {
    let db = InMemoryDatabase::new();
    let session_id = create_session_with_events(&db).await;

    // Positive filter: single type
    let events = db
        .list_events(
            session_id,
            None,
            None,
            &["input.message".to_string()],
            &[],
            None,
            None,
        )
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "input.message");
}

#[tokio::test]
async fn test_list_events_filter_types_empty_returns_all() {
    let db = InMemoryDatabase::new();
    let session_id = create_session_with_events(&db).await;

    // Empty types = return all (6 events created)
    let events = db
        .list_events(session_id, None, None, &[], &[], None, None)
        .await
        .unwrap();

    assert_eq!(events.len(), 6);
}

#[tokio::test]
async fn test_list_events_filter_types_no_match() {
    let db = InMemoryDatabase::new();
    let session_id = create_session_with_events(&db).await;

    // Types that don't exist — empty result
    let events = db
        .list_events(
            session_id,
            None,
            None,
            &["nonexistent.type".to_string()],
            &[],
            None,
            None,
        )
        .await
        .unwrap();

    assert!(events.is_empty());
}

#[tokio::test]
async fn test_list_events_exclude_only() {
    let db = InMemoryDatabase::new();
    let session_id = create_session_with_events(&db).await;

    // Exclude delta events (2 of 6)
    let events = db
        .list_events(
            session_id,
            None,
            None,
            &[],
            &[
                "output.message.delta".to_string(),
                "reason.thinking.delta".to_string(),
            ],
            None,
            None,
        )
        .await
        .unwrap();

    assert_eq!(events.len(), 4);
    assert!(events.iter().all(|e| !e.event_type.contains("delta")));
}

#[tokio::test]
async fn test_list_events_types_and_exclude_combined() {
    let db = InMemoryDatabase::new();
    let session_id = create_session_with_events(&db).await;

    // types narrows to turn.* + input.message (3 events),
    // then exclude removes turn.completed (1 event) → 2 events remain
    let events = db
        .list_events(
            session_id,
            None,
            None,
            &[
                "turn.started".to_string(),
                "turn.completed".to_string(),
                "input.message".to_string(),
            ],
            &["turn.completed".to_string()],
            None,
            None,
        )
        .await
        .unwrap();

    assert_eq!(events.len(), 2);
    let types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
    assert!(types.contains(&"turn.started"));
    assert!(types.contains(&"input.message"));
    assert!(!types.contains(&"turn.completed"));
}

#[tokio::test]
async fn test_list_events_types_fully_excluded() {
    let db = InMemoryDatabase::new();
    let session_id = create_session_with_events(&db).await;

    // types selects one event, exclude removes that same type → empty
    let events = db
        .list_events(
            session_id,
            None,
            None,
            &["input.message".to_string()],
            &["input.message".to_string()],
            None,
            None,
        )
        .await
        .unwrap();

    assert!(events.is_empty());
}

#[tokio::test]
async fn test_list_events_since_id_uses_sequence_ordering() {
    let db = InMemoryDatabase::new();
    let session_id = create_session_with_events(&db).await;

    // Get all events to find the ID of the second event
    let all_events = db
        .list_events(session_id, None, None, &[], &[], None, None)
        .await
        .unwrap();
    assert_eq!(all_events.len(), 6);

    let second_event_id = all_events[1].id;
    let second_event_seq = all_events[1].sequence;

    // Using since_id should return events after that event's sequence
    let events_after_id = db
        .list_events(
            session_id,
            None,
            Some(second_event_id),
            &[],
            &[],
            None,
            None,
        )
        .await
        .unwrap();

    // Using since_sequence with the same sequence should return the same events
    let events_after_seq = db
        .list_events(
            session_id,
            Some(second_event_seq),
            None,
            &[],
            &[],
            None,
            None,
        )
        .await
        .unwrap();

    assert_eq!(events_after_id.len(), events_after_seq.len());
    assert_eq!(events_after_id.len(), 4); // 6 total - 2 skipped = 4
    for (a, b) in events_after_id.iter().zip(events_after_seq.iter()) {
        assert_eq!(a.id, b.id);
        assert_eq!(a.sequence, b.sequence);
    }

    // Results should be ordered by sequence
    for window in events_after_id.windows(2) {
        assert!(
            window[0].sequence < window[1].sequence,
            "events must be ordered by sequence"
        );
    }
}

#[tokio::test]
async fn test_list_events_since_id_unknown_returns_empty() {
    let db = InMemoryDatabase::new();
    let session_id = create_session_with_events(&db).await;

    // Using a since_id that doesn't exist should return no events
    let unknown_id = EventId::new();
    let events = db
        .list_events(session_id, None, Some(unknown_id), &[], &[], None, None)
        .await
        .unwrap();

    assert!(events.is_empty());
}

#[tokio::test]
async fn test_list_events_since_id_takes_precedence_over_since_sequence() {
    let db = InMemoryDatabase::new();
    let session_id = create_session_with_events(&db).await;

    let all_events = db
        .list_events(session_id, None, None, &[], &[], None, None)
        .await
        .unwrap();

    // Provide since_id of 4th event but since_sequence of 1st event.
    // since_id should take precedence (return events after 4th).
    let fourth_event_id = all_events[3].id;
    let events = db
        .list_events(
            session_id,
            Some(all_events[0].sequence),
            Some(fourth_event_id),
            &[],
            &[],
            None,
            None,
        )
        .await
        .unwrap();

    assert_eq!(events.len(), 2); // events 5 and 6
    assert_eq!(events[0].sequence, all_events[4].sequence);
    assert_eq!(events[1].sequence, all_events[5].sequence);
}

#[tokio::test]
async fn test_list_events_default_cap_keeps_earliest_forward_window() {
    let db = InMemoryDatabase::new();
    let session_id = create_session_with_events(&db).await;

    // Add enough events so forward catch-up exceeds the implicit 10k safety cap.
    for _ in 0..10_010 {
        db.create_event(CreateEventRow {
            session_id,
            event_type: "output.message.delta".to_string(),
            ts: Utc::now(),
            context: serde_json::json!({}),
            data: serde_json::json!({}),
            metadata: None,
            tags: None,
        })
        .await
        .unwrap();
    }

    let all_events = db
        .list_events(session_id, None, None, &[], &[], None, Some(20_000))
        .await
        .unwrap();
    assert!(all_events.len() > 10_000);

    // Simulate SSE catch-up by querying with since_id and no explicit limit.
    let second_event_id = all_events[1].id;
    let events = db
        .list_events(
            session_id,
            None,
            Some(second_event_id),
            &[],
            &[],
            None,
            None,
        )
        .await
        .unwrap();

    assert_eq!(events.len(), 10_000);
    // Forward path must return the earliest page after the cursor, not the newest page.
    assert_eq!(events[0].sequence, all_events[2].sequence);
    assert_eq!(events.last().unwrap().sequence, all_events[10_001].sequence);
}

#[tokio::test]
async fn test_list_events_advanced_filters_by_turn_and_tool() {
    use crate::storage::models::ListEventsParams;

    let db = InMemoryDatabase::new();
    let session_id = create_session_with_events(&db).await;

    // Add a turn-tagged tool event we can target.
    db.create_event(CreateEventRow {
        session_id,
        event_type: "tool.completed".to_string(),
        ts: Utc::now(),
        context: serde_json::json!({"turn_id": "turn_aaa"}),
        data: serde_json::json!({"tool_name": "fetch"}),
        metadata: None,
        tags: Some(vec!["error".to_string()]),
    })
    .await
    .unwrap();
    db.create_event(CreateEventRow {
        session_id,
        event_type: "tool.completed".to_string(),
        ts: Utc::now(),
        context: serde_json::json!({"turn_id": "turn_bbb"}),
        data: serde_json::json!({"tool_name": "search"}),
        metadata: None,
        tags: Some(vec!["ok".to_string()]),
    })
    .await
    .unwrap();

    let by_turn = db
        .list_events_advanced(&ListEventsParams {
            session_id,
            turn_id: Some("turn_aaa".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(by_turn.len(), 1);
    assert_eq!(by_turn[0].event_type, "tool.completed");

    let by_tool = db
        .list_events_advanced(&ListEventsParams {
            session_id,
            tool_name: Some("search".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(by_tool.len(), 1);

    let by_tag = db
        .list_events_advanced(&ListEventsParams {
            session_id,
            tags: vec!["error".to_string()],
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(by_tag.len(), 1);
}

#[tokio::test]
async fn test_list_events_advanced_around_id_scoped_to_session() {
    use crate::storage::models::ListEventsParams;

    let db = InMemoryDatabase::new();
    let session_a = create_session_with_events(&db).await;
    let session_b = create_session_with_events(&db).await;

    // Pick an event id from session B and try to anchor against session A.
    let foreign_event_id = db
        .list_events(session_b, None, None, &[], &[], None, None)
        .await
        .unwrap()[0]
        .id;

    let result = db
        .list_events_advanced(&ListEventsParams {
            session_id: session_a,
            around_id: Some(foreign_event_id),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(
        result.is_empty(),
        "around_id from another session must yield empty"
    );
}

#[tokio::test]
async fn test_list_events_advanced_since_id_scoped_to_session() {
    use crate::storage::models::ListEventsParams;

    let db = InMemoryDatabase::new();
    let session_a = create_session_with_events(&db).await;
    let session_b = create_session_with_events(&db).await;

    // Foreign since_id must NOT advance the cursor for session A; we should
    // still see all session A events.
    let foreign_event_id = db
        .list_events(session_b, None, None, &[], &[], None, None)
        .await
        .unwrap()[0]
        .id;

    let result = db
        .list_events_advanced(&ListEventsParams {
            session_id: session_a,
            since_id: Some(foreign_event_id),
            order_desc: true, // force advanced path
            ..Default::default()
        })
        .await
        .unwrap();
    let baseline = db
        .list_events(session_a, None, None, &[], &[], None, None)
        .await
        .unwrap();
    assert_eq!(result.len(), baseline.len());
}

#[tokio::test]
async fn test_list_events_advanced_order_desc_returns_newest_first() {
    use crate::storage::models::ListEventsParams;

    let db = InMemoryDatabase::new();
    let session_id = create_session_with_events(&db).await;
    let asc = db
        .list_events_advanced(&ListEventsParams {
            session_id,
            order_desc: false,
            ..Default::default()
        })
        .await
        .unwrap();
    let desc = db
        .list_events_advanced(&ListEventsParams {
            session_id,
            order_desc: true,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(!asc.is_empty());
    assert_eq!(asc.len(), desc.len());
    let mut asc_seq: Vec<_> = asc.iter().map(|e| e.sequence).collect();
    let desc_seq: Vec<_> = desc.iter().map(|e| e.sequence).collect();
    asc_seq.reverse();
    assert_eq!(asc_seq, desc_seq);
}

#[tokio::test]
async fn test_events_summary_counts_by_type() {
    let db = InMemoryDatabase::new();
    let session_id = create_session_with_events(&db).await;

    let summary = db.events_summary(session_id).await.unwrap();
    assert_eq!(summary.total, 6);
    let turn_started = summary
        .by_type
        .iter()
        .find(|c| c.event_type == "turn.started")
        .unwrap();
    assert_eq!(turn_started.count, 1);
    assert!(summary.first_ts.is_some());
    assert!(summary.last_ts.is_some());
}

#[tokio::test]
async fn test_list_events_with_limit() {
    let db = InMemoryDatabase::new();
    let session_id = create_session_with_events(&db).await;

    // 6 events total. limit=3 should return last 3.
    let events = db
        .list_events(session_id, None, None, &[], &[], None, Some(3))
        .await
        .unwrap();
    assert_eq!(events.len(), 3);
    // Should be the last 3 events in sequence order
    assert!(events[0].sequence < events[1].sequence);
    assert!(events[1].sequence < events[2].sequence);
}

#[tokio::test]
async fn test_list_events_with_limit_and_before_sequence() {
    let db = InMemoryDatabase::new();
    let session_id = create_session_with_events(&db).await;

    let all = db
        .list_events(session_id, None, None, &[], &[], None, None)
        .await
        .unwrap();
    assert_eq!(all.len(), 6);

    // Get last 2 events before the 5th event's sequence
    let fifth_seq = all[4].sequence;
    let events = db
        .list_events(session_id, None, None, &[], &[], Some(fifth_seq), Some(2))
        .await
        .unwrap();
    assert_eq!(events.len(), 2);
    // Should be events 3 and 4 (0-indexed)
    assert_eq!(events[0].id, all[2].id);
    assert_eq!(events[1].id, all[3].id);
}

#[tokio::test]
async fn test_list_events_limit_greater_than_total() {
    let db = InMemoryDatabase::new();
    let session_id = create_session_with_events(&db).await;

    // limit=1000 but only 6 events exist — returns all 6
    let events = db
        .list_events(session_id, None, None, &[], &[], None, Some(1000))
        .await
        .unwrap();
    assert_eq!(events.len(), 6);
}

#[tokio::test]
async fn test_list_events_limit_with_exclude() {
    let db = InMemoryDatabase::new();
    let session_id = create_session_with_events(&db).await;

    // 6 events, 2 are deltas. Exclude deltas + limit=2 → last 2 non-delta events
    let events = db
        .list_events(
            session_id,
            None,
            None,
            &[],
            &[
                "output.message.delta".to_string(),
                "reason.thinking.delta".to_string(),
            ],
            None,
            Some(2),
        )
        .await
        .unwrap();
    assert_eq!(events.len(), 2);
    assert!(events.iter().all(|e| !e.event_type.contains("delta")));
}

#[tokio::test]
async fn test_count_non_delta_events() {
    let db = InMemoryDatabase::new();
    let session_id = create_session_with_events(&db).await;

    // 6 events total, 2 are deltas → 4 non-delta
    let delta_types = vec![
        "output.message.delta".to_string(),
        "reason.thinking.delta".to_string(),
    ];
    let count = db.count_events(session_id, &delta_types).await.unwrap();
    assert_eq!(count, 4);
}

#[tokio::test]
async fn test_find_turn_boundary() {
    let db = InMemoryDatabase::new();
    let session_id = create_session_with_events(&db).await;

    let all = db
        .list_events(session_id, None, None, &[], &[], None, None)
        .await
        .unwrap();

    // Find the turn.started event
    let turn_started = all.iter().find(|e| e.event_type == "turn.started").unwrap();

    // Searching at or after the turn.started sequence should find it
    let boundary = db
        .find_turn_boundary(session_id, turn_started.sequence + 1)
        .await
        .unwrap();
    assert_eq!(boundary, Some(turn_started.sequence));

    // Searching before the turn.started sequence should find nothing
    // (if turn.started is the first turn event)
    let boundary = db
        .find_turn_boundary(session_id, turn_started.sequence - 1)
        .await
        .unwrap();
    assert!(boundary.is_none());
}

#[tokio::test]
async fn test_list_events_empty_session_with_limit() {
    let db = InMemoryDatabase::new();
    let agent = db
        .create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: "empty-agent".to_string(),
                display_name: Some("Empty Agent".to_string()),
                description: None,
                system_prompt: String::new(),
                default_model_id: None,

                harness_id: test_harness_id(),
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
            },
        )
        .await
        .unwrap();

    let session = db
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: DEFAULT_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: everruns_core::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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

    // Empty session with limit should return empty
    let events = db
        .list_events(session.id, None, None, &[], &[], None, Some(200))
        .await
        .unwrap();
    assert!(events.is_empty());
}

#[tokio::test]
async fn test_sessions_pagination() {
    let db = InMemoryDatabase::new();

    let agent = db
        .create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: "test-agent".to_string(),
                display_name: Some("Test Agent".to_string()),
                description: None,
                system_prompt: String::new(),
                default_model_id: None,

                harness_id: test_harness_id(),
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
            },
        )
        .await
        .unwrap();

    // Create 15 sessions
    for i in 0..15 {
        db.create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: DEFAULT_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: everruns_core::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            title: Some(format!("Session {}", i)),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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
    }

    // Test default pagination (all sessions fit within limit)
    let pagination = crate::api::common::Pagination::new(0, 20);
    let (sessions, total) = db
        .list_sessions(
            DEFAULT_ORG_ID,
            &SessionListFilters {
                agent_id: Some(agent.id),
                ..Default::default()
            },
            pagination,
        )
        .await
        .unwrap();
    assert_eq!(total, 15);
    assert_eq!(sessions.len(), 15);

    // Test with limit=5
    let pagination = crate::api::common::Pagination::new(0, 5);
    let (sessions, total) = db
        .list_sessions(
            DEFAULT_ORG_ID,
            &SessionListFilters {
                agent_id: Some(agent.id),
                ..Default::default()
            },
            pagination,
        )
        .await
        .unwrap();
    assert_eq!(total, 15);
    assert_eq!(sessions.len(), 5);

    // Test with offset=5, limit=5
    let pagination = crate::api::common::Pagination::new(5, 5);
    let (sessions, total) = db
        .list_sessions(
            DEFAULT_ORG_ID,
            &SessionListFilters {
                agent_id: Some(agent.id),
                ..Default::default()
            },
            pagination,
        )
        .await
        .unwrap();
    assert_eq!(total, 15);
    assert_eq!(sessions.len(), 5);

    // Test last partial page (offset=10, limit=10 should return 5)
    let pagination = crate::api::common::Pagination::new(10, 10);
    let (sessions, total) = db
        .list_sessions(
            DEFAULT_ORG_ID,
            &SessionListFilters {
                agent_id: Some(agent.id),
                ..Default::default()
            },
            pagination,
        )
        .await
        .unwrap();
    assert_eq!(total, 15);
    assert_eq!(sessions.len(), 5);

    // Test beyond range (offset=20)
    let pagination = crate::api::common::Pagination::new(20, 10);
    let (sessions, total) = db
        .list_sessions(
            DEFAULT_ORG_ID,
            &SessionListFilters {
                agent_id: Some(agent.id),
                ..Default::default()
            },
            pagination,
        )
        .await
        .unwrap();
    assert_eq!(total, 15);
    assert_eq!(sessions.len(), 0);
}

#[tokio::test]
async fn test_sessions_pagination_ordering() {
    let db = InMemoryDatabase::new();

    let agent = db
        .create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: "test-agent".to_string(),
                display_name: Some("Test Agent".to_string()),
                description: None,
                system_prompt: String::new(),
                default_model_id: None,

                harness_id: test_harness_id(),
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
            },
        )
        .await
        .unwrap();

    // Create sessions with sequential titles
    for i in 1..=5 {
        db.create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: DEFAULT_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: everruns_core::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            title: Some(format!("Session {}", i)),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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
        // Small delay to ensure different created_at timestamps
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    }

    // Sessions should be ordered by created_at DESC (newest first)
    let pagination = crate::api::common::Pagination::new(0, 10);
    let (sessions, _) = db
        .list_sessions(
            DEFAULT_ORG_ID,
            &SessionListFilters {
                agent_id: Some(agent.id),
                ..Default::default()
            },
            pagination,
        )
        .await
        .unwrap();

    assert_eq!(sessions.len(), 5);
    // Most recent session should be first
    assert_eq!(sessions[0].title, Some("Session 5".to_string()));
    assert_eq!(sessions[4].title, Some("Session 1".to_string()));
}

// TM-TENANT-008: Verify org-scoped user listing prevents cross-tenant enumeration
#[tokio::test]
async fn test_list_users_by_org_isolation() {
    let db = InMemoryDatabase::new();

    // Create two orgs
    let org1 = db
        .create_organization(CreateOrganizationRow {
            public_id: "org_00000000000000000000000000000010".to_string(),
            name: "Org 1".to_string(),
            created_by: None,
        })
        .await
        .unwrap();
    let org2 = db
        .create_organization(CreateOrganizationRow {
            public_id: "org_00000000000000000000000000000020".to_string(),
            name: "Org 2".to_string(),
            created_by: None,
        })
        .await
        .unwrap();

    // Create three users
    let user1 = db
        .create_user(CreateUserRow {
            email: "alice@example.com".to_string(),
            name: "Alice".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .unwrap();
    let user2 = db
        .create_user(CreateUserRow {
            email: "bob@example.com".to_string(),
            name: "Bob".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .unwrap();
    let _user3 = db
        .create_user(CreateUserRow {
            email: "charlie@example.com".to_string(),
            name: "Charlie".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .unwrap();

    // Add users to different orgs
    db.add_organization_member(org1.org_id, user1.id, "member")
        .await
        .unwrap();
    db.add_organization_member(org1.org_id, user2.id, "member")
        .await
        .unwrap();
    db.add_organization_member(org2.org_id, user2.id, "member")
        .await
        .unwrap();
    // user3 not in any of these orgs

    // Org1 should see alice + bob
    let org1_users = db.list_users_by_org(org1.org_id, None).await.unwrap();
    assert_eq!(org1_users.len(), 2);
    let org1_emails: Vec<_> = org1_users.iter().map(|u| u.email.as_str()).collect();
    assert!(org1_emails.contains(&"alice@example.com"));
    assert!(org1_emails.contains(&"bob@example.com"));

    // Org2 should see only bob
    let org2_users = db.list_users_by_org(org2.org_id, None).await.unwrap();
    assert_eq!(org2_users.len(), 1);
    assert_eq!(org2_users[0].email, "bob@example.com");

    // Charlie should not appear in either org
    assert!(!org1_emails.contains(&"charlie@example.com"));

    // Search within org should filter
    let search_results = db
        .list_users_by_org(org1.org_id, Some("alice"))
        .await
        .unwrap();
    assert_eq!(search_results.len(), 1);
    assert_eq!(search_results[0].email, "alice@example.com");

    // Search in org2 for alice should return nothing (alice not in org2)
    let cross_org_search = db
        .list_users_by_org(org2.org_id, Some("alice"))
        .await
        .unwrap();
    assert_eq!(cross_org_search.len(), 0);
}

#[tokio::test]
async fn test_audit_log_create_and_list() {
    let db = InMemoryDatabase::new();

    let user_id = Uuid::now_v7();
    db.create_audit_log(CreateAuditLogRow {
        org_id: DEFAULT_ORG_ID,
        actor_id: Some(user_id),
        event_type: "auth.login.success".to_string(),
        ip_address: Some("1.2.3.4".to_string()),
        metadata: serde_json::json!({"method": "password"}),
        domain: "management".to_string(),
        action: "auth.login.success".to_string(),
        target_type: None,
        target_id: None,
    })
    .await
    .unwrap();

    db.create_audit_log(CreateAuditLogRow {
        org_id: DEFAULT_ORG_ID,
        actor_id: None,
        event_type: "auth.login.failure".to_string(),
        ip_address: Some("5.6.7.8".to_string()),
        metadata: serde_json::json!({"reason": "invalid_password"}),
        domain: "management".to_string(),
        action: "auth.login.failure".to_string(),
        target_type: None,
        target_id: None,
    })
    .await
    .unwrap();

    // List all
    let logs = db
        .list_audit_logs(AuditLogQuery {
            org_id: DEFAULT_ORG_ID,
            limit: 50,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(logs.len(), 2);
    // Newest first
    assert_eq!(logs[0].event_type, "auth.login.failure");

    // Filter by event type prefix
    let success_only = db
        .list_audit_logs(AuditLogQuery {
            org_id: DEFAULT_ORG_ID,
            limit: 50,
            event_type_prefix: Some("auth.login.success"),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(success_only.len(), 1);
    assert_eq!(success_only[0].event_type, "auth.login.success");

    // Filter by actor
    let actor_logs = db
        .list_audit_logs(AuditLogQuery {
            org_id: DEFAULT_ORG_ID,
            limit: 50,
            actor_id: Some(user_id),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(actor_logs.len(), 1);
    assert_eq!(actor_logs[0].actor_id, Some(user_id));

    // Limit
    let limited = db
        .list_audit_logs(AuditLogQuery {
            org_id: DEFAULT_ORG_ID,
            limit: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(limited.len(), 1);
}

#[tokio::test]
async fn test_audit_log_org_isolation() {
    let db = InMemoryDatabase::new();

    let org2 = db
        .create_organization(CreateOrganizationRow {
            public_id: "org_00000000000000000000000000000099".to_string(),
            name: "Other Org".to_string(),
            created_by: None,
        })
        .await
        .unwrap();

    db.create_audit_log(CreateAuditLogRow {
        org_id: DEFAULT_ORG_ID,
        actor_id: None,
        event_type: "auth.login.success".to_string(),
        ip_address: None,
        metadata: serde_json::json!({}),
        domain: "management".to_string(),
        action: "auth.login.success".to_string(),
        target_type: None,
        target_id: None,
    })
    .await
    .unwrap();

    db.create_audit_log(CreateAuditLogRow {
        org_id: org2.org_id,
        actor_id: None,
        event_type: "auth.login.failure".to_string(),
        ip_address: None,
        metadata: serde_json::json!({}),
        domain: "management".to_string(),
        action: "auth.login.failure".to_string(),
        target_type: None,
        target_id: None,
    })
    .await
    .unwrap();

    let org1_logs = db
        .list_audit_logs(AuditLogQuery {
            org_id: DEFAULT_ORG_ID,
            limit: 50,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(org1_logs.len(), 1);
    assert_eq!(org1_logs[0].event_type, "auth.login.success");

    let org2_logs = db
        .list_audit_logs(AuditLogQuery {
            org_id: org2.org_id,
            limit: 50,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(org2_logs.len(), 1);
    assert_eq!(org2_logs[0].event_type, "auth.login.failure");
}

#[tokio::test]
async fn test_audit_log_retention_delete() {
    let db = InMemoryDatabase::new();

    db.create_audit_log(CreateAuditLogRow {
        org_id: DEFAULT_ORG_ID,
        actor_id: None,
        event_type: "auth.login.success".to_string(),
        ip_address: None,
        metadata: serde_json::json!({}),
        domain: "management".to_string(),
        action: "auth.login.success".to_string(),
        target_type: None,
        target_id: None,
    })
    .await
    .unwrap();

    // Delete logs before future timestamp → removes all
    let deleted = db
        .delete_audit_logs_before(Utc::now() + chrono::Duration::hours(1))
        .await
        .unwrap();
    assert_eq!(deleted, 1);

    let logs = db
        .list_audit_logs(AuditLogQuery {
            org_id: DEFAULT_ORG_ID,
            limit: 50,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(logs.is_empty());
}

// ─── Audit domain filtering tests (EVE-226) ───

#[tokio::test]
async fn test_audit_log_domain_filtering() {
    let db = InMemoryDatabase::new();

    db.create_audit_log(CreateAuditLogRow {
        org_id: DEFAULT_ORG_ID,
        actor_id: None,
        event_type: "management.member.invited".to_string(),
        ip_address: None,
        metadata: serde_json::json!({}),
        domain: "management".to_string(),
        action: "management.member.invited".to_string(),
        target_type: Some("member".to_string()),
        target_id: Some("usr_abc".to_string()),
    })
    .await
    .unwrap();

    db.create_audit_log(CreateAuditLogRow {
        org_id: DEFAULT_ORG_ID,
        actor_id: None,
        event_type: "agent.run.started".to_string(),
        ip_address: None,
        metadata: serde_json::json!({}),
        domain: "agent".to_string(),
        action: "agent.run.started".to_string(),
        target_type: Some("session".to_string()),
        target_id: Some("ses_xyz".to_string()),
    })
    .await
    .unwrap();

    db.create_audit_log(CreateAuditLogRow {
        org_id: DEFAULT_ORG_ID,
        actor_id: None,
        event_type: "management.harness.created".to_string(),
        ip_address: None,
        metadata: serde_json::json!({}),
        domain: "management".to_string(),
        action: "management.harness.created".to_string(),
        target_type: Some("harness".to_string()),
        target_id: Some("harness_001".to_string()),
    })
    .await
    .unwrap();

    // Filter by management domain
    let mgmt = db
        .list_audit_logs(AuditLogQuery {
            org_id: DEFAULT_ORG_ID,
            limit: 50,
            domain: Some("management"),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(mgmt.len(), 2);

    // Filter by agent domain
    let agent = db
        .list_audit_logs(AuditLogQuery {
            org_id: DEFAULT_ORG_ID,
            limit: 50,
            domain: Some("agent"),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(agent.len(), 1);
    assert_eq!(agent[0].action, "agent.run.started");

    // Filter by specific action
    let action = db
        .list_audit_logs(AuditLogQuery {
            org_id: DEFAULT_ORG_ID,
            limit: 50,
            action: Some("management.member.invited"),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(action.len(), 1);
    assert_eq!(action[0].target_type.as_deref(), Some("member"));
    assert_eq!(action[0].target_id.as_deref(), Some("usr_abc"));

    // Domain + action combined
    let combined = db
        .list_audit_logs(AuditLogQuery {
            org_id: DEFAULT_ORG_ID,
            limit: 50,
            domain: Some("management"),
            action: Some("management.harness.created"),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(combined.len(), 1);
    assert_eq!(combined[0].target_type.as_deref(), Some("harness"));
}

#[tokio::test]
async fn test_audit_log_target_fields() {
    let db = InMemoryDatabase::new();

    let row = db
        .create_audit_log(CreateAuditLogRow {
            org_id: DEFAULT_ORG_ID,
            actor_id: None,
            event_type: "management.agent.created".to_string(),
            ip_address: Some("10.0.0.1".to_string()),
            metadata: serde_json::json!({"name": "test-agent"}),
            domain: "management".to_string(),
            action: "management.agent.created".to_string(),
            target_type: Some("agent".to_string()),
            target_id: Some("agent_00000000000000000000000000000001".to_string()),
        })
        .await
        .unwrap();

    assert_eq!(row.domain, "management");
    assert_eq!(row.action, "management.agent.created");
    assert_eq!(row.target_type.as_deref(), Some("agent"));
    assert_eq!(
        row.target_id.as_deref(),
        Some("agent_00000000000000000000000000000001")
    );
}

// ─── Search / command-palette tests ───

/// Helper: create an agent with given name + description
async fn create_test_agent(
    db: &InMemoryDatabase,
    name: &str,
    description: Option<&str>,
) -> AgentRow {
    db.create_agent(
        DEFAULT_ORG_ID,
        CreateAgentRow {
            public_id: AgentId::new().to_string(),
            name: name.to_string(),
            display_name: Some("Test Agent".to_string()),
            description: description.map(|d| d.to_string()),
            system_prompt: String::new(),
            default_model_id: None,

            harness_id: test_harness_id(),
            tags: vec![],
            initial_files: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
        },
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn test_search_agents_no_filter_returns_all() {
    let db = InMemoryDatabase::new();
    create_test_agent(&db, "Alpha", None).await;
    create_test_agent(&db, "Beta", None).await;

    let (results, _total) = db
        .list_agents(DEFAULT_ORG_ID, None, false, default_pagination())
        .await
        .unwrap();
    assert_eq!(results.len(), 2);
}

#[tokio::test]
async fn test_search_agents_empty_string_returns_all() {
    let db = InMemoryDatabase::new();
    create_test_agent(&db, "Alpha", None).await;

    let (results, _total) = db
        .list_agents(DEFAULT_ORG_ID, Some(""), false, default_pagination())
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
}

#[tokio::test]
async fn test_search_agents_single_word_match() {
    let db = InMemoryDatabase::new();
    create_test_agent(&db, "Customer Support Bot", None).await;
    create_test_agent(&db, "Code Reviewer", None).await;

    let (results, _total) = db
        .list_agents(
            DEFAULT_ORG_ID,
            Some("customer"),
            false,
            default_pagination(),
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Customer Support Bot");
}

#[tokio::test]
async fn test_search_agents_case_insensitive() {
    let db = InMemoryDatabase::new();
    create_test_agent(&db, "Customer Support Bot", None).await;

    let (results, _total) = db
        .list_agents(
            DEFAULT_ORG_ID,
            Some("CUSTOMER"),
            false,
            default_pagination(),
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
}

#[tokio::test]
async fn test_search_agents_multi_word_all_must_match() {
    let db = InMemoryDatabase::new();
    create_test_agent(&db, "Customer Support Bot", None).await;
    create_test_agent(&db, "Customer Feedback Analyzer", None).await;

    let (results, _total) = db
        .list_agents(
            DEFAULT_ORG_ID,
            Some("customer bot"),
            false,
            default_pagination(),
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Customer Support Bot");
}

#[tokio::test]
async fn test_search_agents_matches_description() {
    let db = InMemoryDatabase::new();
    create_test_agent(&db, "Helper", Some("Handles billing inquiries")).await;

    let (results, _total) = db
        .list_agents(DEFAULT_ORG_ID, Some("billing"), false, default_pagination())
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Helper");
}

#[tokio::test]
async fn test_search_agents_cross_field_match() {
    let db = InMemoryDatabase::new();
    create_test_agent(&db, "Daytona Coder", Some("cloud sandbox agent")).await;

    // "daytona" in name, "sandbox" in description → both must match
    let (results, _total) = db
        .list_agents(
            DEFAULT_ORG_ID,
            Some("daytona sandbox"),
            false,
            default_pagination(),
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
}

#[tokio::test]
async fn test_search_agents_no_match() {
    let db = InMemoryDatabase::new();
    create_test_agent(&db, "Customer Support Bot", None).await;

    let (results, _total) = db
        .list_agents(
            DEFAULT_ORG_ID,
            Some("zzz_nonexistent"),
            false,
            default_pagination(),
        )
        .await
        .unwrap();
    assert!(results.is_empty());
}

#[tokio::test]
async fn test_search_agents_poem_does_not_crash() {
    let db = InMemoryDatabase::new();
    create_test_agent(&db, "Alpha", None).await;

    // A full poem pasted into search — should not crash or hang
    let poem = "Roses are red, violets are blue, \
                    sugar is sweet, and so are you. \
                    The sky is wide, the ocean deep, \
                    these memories I shall forever keep. \
                    Through winding roads and starlit nights, \
                    we chase our dreams to greater heights.";
    let (results, _total) = db
        .list_agents(DEFAULT_ORG_ID, Some(poem), false, default_pagination())
        .await
        .unwrap();
    // No agent should match a poem
    assert!(results.is_empty());
}

#[tokio::test]
async fn test_search_agents_poem_token_cap() {
    let db = InMemoryDatabase::new();
    // Agent whose name contains many words from the poem
    create_test_agent(&db, "roses are red violets are blue sugar is sweet", None).await;

    // Query with >MAX_SEARCH_TOKENS words — only first 8 tokens used
    let long_query = "roses are red violets are blue sugar is sweet and so are you forever";
    let (results, _total) = db
        .list_agents(
            DEFAULT_ORG_ID,
            Some(long_query),
            false,
            default_pagination(),
        )
        .await
        .unwrap();
    // First 8 tokens: "roses are red violets are blue sugar is"
    // All present in agent name → should match
    assert_eq!(results.len(), 1);
}

#[tokio::test]
async fn test_search_agents_special_characters() {
    let db = InMemoryDatabase::new();
    create_test_agent(&db, "Agent v2.0 (beta)", None).await;
    create_test_agent(&db, "my-agent_v1", None).await;

    let (results, _total) = db
        .list_agents(DEFAULT_ORG_ID, Some("v2.0"), false, default_pagination())
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Agent v2.0 (beta)");
}

#[tokio::test]
async fn test_search_agents_unicode() {
    let db = InMemoryDatabase::new();
    create_test_agent(&db, "日本語エージェント", Some("テスト用")).await;
    create_test_agent(&db, "English Agent", None).await;

    let (results, _total) = db
        .list_agents(DEFAULT_ORG_ID, Some("日本語"), false, default_pagination())
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "日本語エージェント");
}

#[tokio::test]
async fn test_search_agents_emoji() {
    let db = InMemoryDatabase::new();
    create_test_agent(&db, "🤖 Robot Helper", None).await;
    create_test_agent(&db, "Normal Agent", None).await;

    let (results, _total) = db
        .list_agents(DEFAULT_ORG_ID, Some("🤖"), false, default_pagination())
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
}

#[tokio::test]
async fn test_search_agents_whitespace_normalization() {
    let db = InMemoryDatabase::new();
    create_test_agent(&db, "Customer Support Bot", None).await;

    // Extra spaces, tabs, etc.
    let (results, _total) = db
        .list_agents(
            DEFAULT_ORG_ID,
            Some("  customer   bot  "),
            false,
            default_pagination(),
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
}

#[tokio::test]
async fn test_search_sessions_by_title() {
    let db = InMemoryDatabase::new();
    let agent = create_test_agent(&db, "Agent", None).await;

    db.create_session(CreateSessionRow {
        source: everruns_platform::SessionSource::Api,
        workspace_id: None,
        org_id: DEFAULT_ORG_ID,
        app_id: None,
        harness_id: None,
        agent_id: Some(agent.id),
        agent_version_id: None,
        agent_config_hash: None,
        agent_identity_id: None,
        owner_principal_id: everruns_core::PrincipalId::from_seed(1),
        resolved_owner_user_id: None,
        title: Some("Debug production memory leak".to_string()),
        locale: None,
        tags: vec![],
        model_id: None,
        capabilities: serde_json::json!([]),
        tools: serde_json::json!([]),
        mcp_servers: serde_json::json!({}),
        system_prompt: None,
        initial_files: serde_json::Value::Array(vec![]),
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

    db.create_session(CreateSessionRow {
        source: everruns_platform::SessionSource::Api,
        workspace_id: None,
        org_id: DEFAULT_ORG_ID,
        app_id: None,
        harness_id: None,
        agent_id: Some(agent.id),
        agent_version_id: None,
        agent_config_hash: None,
        agent_identity_id: None,
        owner_principal_id: everruns_core::PrincipalId::from_seed(1),
        resolved_owner_user_id: None,
        title: Some("Refactor auth module".to_string()),
        locale: None,
        tags: vec![],
        model_id: None,
        capabilities: serde_json::json!([]),
        tools: serde_json::json!([]),
        mcp_servers: serde_json::json!({}),
        system_prompt: None,
        initial_files: serde_json::Value::Array(vec![]),
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

    let pagination = crate::api::common::Pagination::new(0, 20);
    let (results, total) = db
        .list_sessions(
            DEFAULT_ORG_ID,
            &SessionListFilters {
                search: Some("memory leak".to_string()),
                ..Default::default()
            },
            pagination,
        )
        .await
        .unwrap();
    assert_eq!(total, 1);
    assert_eq!(results.len(), 1);
    assert!(results[0].title.as_ref().unwrap().contains("memory leak"));
}

#[tokio::test]
async fn test_search_sessions_with_agent_filter() {
    let db = InMemoryDatabase::new();
    let agent1 = create_test_agent(&db, "Agent1", None).await;
    let agent2 = create_test_agent(&db, "Agent2", None).await;

    db.create_session(CreateSessionRow {
        source: everruns_platform::SessionSource::Api,
        workspace_id: None,
        org_id: DEFAULT_ORG_ID,
        app_id: None,
        harness_id: None,
        agent_id: Some(agent1.id),
        agent_version_id: None,
        agent_config_hash: None,
        agent_identity_id: None,
        owner_principal_id: everruns_core::PrincipalId::from_seed(1),
        resolved_owner_user_id: None,
        title: Some("Shared keyword session".to_string()),
        locale: None,
        tags: vec![],
        model_id: None,
        capabilities: serde_json::json!([]),
        tools: serde_json::json!([]),
        mcp_servers: serde_json::json!({}),
        system_prompt: None,
        initial_files: serde_json::Value::Array(vec![]),
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

    db.create_session(CreateSessionRow {
        source: everruns_platform::SessionSource::Api,
        workspace_id: None,
        org_id: DEFAULT_ORG_ID,
        app_id: None,
        harness_id: None,
        agent_id: Some(agent2.id),
        agent_version_id: None,
        agent_config_hash: None,
        agent_identity_id: None,
        owner_principal_id: everruns_core::PrincipalId::from_seed(1),
        resolved_owner_user_id: None,
        title: Some("Shared keyword session".to_string()),
        locale: None,
        tags: vec![],
        model_id: None,
        capabilities: serde_json::json!([]),
        tools: serde_json::json!([]),
        mcp_servers: serde_json::json!({}),
        system_prompt: None,
        initial_files: serde_json::Value::Array(vec![]),
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

    let pagination = crate::api::common::Pagination::new(0, 20);
    // Search + agent filter combined
    let (results, total) = db
        .list_sessions(
            DEFAULT_ORG_ID,
            &SessionListFilters {
                agent_id: Some(agent1.id),
                search: Some("shared keyword".to_string()),
                ..Default::default()
            },
            pagination,
        )
        .await
        .unwrap();
    assert_eq!(total, 1);
    assert_eq!(results.len(), 1);
}

#[tokio::test]
async fn test_search_sessions_poem_input() {
    let db = InMemoryDatabase::new();

    let pagination = crate::api::common::Pagination::new(0, 20);
    let poem = "Shall I compare thee to a summer's day? \
                    Thou art more lovely and more temperate. \
                    Rough winds do shake the darling buds of May, \
                    And summer's lease hath all too short a date.";
    let (results, total) = db
        .list_sessions(
            DEFAULT_ORG_ID,
            &SessionListFilters {
                search: Some(poem.to_string()),
                ..Default::default()
            },
            pagination,
        )
        .await
        .unwrap();
    assert_eq!(total, 0);
    assert!(results.is_empty());
}

#[tokio::test]
async fn test_matches_search_tokens_unit() {
    // Direct unit tests on the helper function
    assert!(matches_search_tokens(None, &["anything"]));
    assert!(matches_search_tokens(Some(""), &["anything"]));
    assert!(matches_search_tokens(Some("  "), &["anything"]));
    assert!(matches_search_tokens(Some("hello"), &["hello world"]));
    assert!(matches_search_tokens(
        Some("hello world"),
        &["hello", "world"]
    ));
    assert!(!matches_search_tokens(Some("missing"), &["hello world"]));
    // Multi-word: all tokens must match
    assert!(!matches_search_tokens(
        Some("hello missing"),
        &["hello world"]
    ));
    // Case insensitive
    assert!(matches_search_tokens(Some("HELLO"), &["hello"]));
}

#[tokio::test]
async fn test_search_skills() {
    let db = InMemoryDatabase::new();

    db.create_skill(
        DEFAULT_ORG_ID,
        CreateSkillRow {
            public_id: SkillId::new().to_string(),
            name: "Web Scraper".to_string(),
            description: "Scrapes web pages for data".to_string(),
            license: Some("MIT".to_string()),
            compatibility: Some("*".to_string()),
            metadata: serde_json::json!({}),
            allowed_tools: None,
            instructions: "scrape it".to_string(),
            source_type: "inline".to_string(),
            archive_data: None,
            version: "1.0.0".to_string(),
        },
    )
    .await
    .unwrap();

    db.create_skill(
        DEFAULT_ORG_ID,
        CreateSkillRow {
            public_id: SkillId::new().to_string(),
            name: "Code Formatter".to_string(),
            description: "Formats code using prettier".to_string(),
            license: Some("MIT".to_string()),
            compatibility: Some("*".to_string()),
            metadata: serde_json::json!({}),
            allowed_tools: None,
            instructions: "format it".to_string(),
            source_type: "inline".to_string(),
            archive_data: None,
            version: "1.0.0".to_string(),
        },
    )
    .await
    .unwrap();

    let results = db
        .list_skills(DEFAULT_ORG_ID, Some("scraper"), false)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Web Scraper");

    // Cross-field: "code prettier"
    let results = db
        .list_skills(DEFAULT_ORG_ID, Some("code prettier"), false)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Code Formatter");
}

#[tokio::test]
async fn test_search_apps() {
    let db = InMemoryDatabase::new();
    let agent = create_test_agent(&db, "Agent", None).await;
    let harness_id = Uuid::now_v7();

    db.create_app(
        DEFAULT_ORG_ID,
        CreateAppRow {
            public_id: "app_test1".to_string(),
            name: "Slack Bot".to_string(),
            description: Some("Slack integration for support".to_string()),
            harness_id,
            agent_id: Some(agent.id.into()),
            agent_version_policy: "default".to_string(),
            agent_version_id: None,
            agent_identity_id: None,
            owner_principal_id: everruns_core::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            channel_type: Some("slack".to_string()),
            channel_config: serde_json::json!({}),
            channel_config_encrypted: None,
        },
    )
    .await
    .unwrap();

    db.create_app(
        DEFAULT_ORG_ID,
        CreateAppRow {
            public_id: "app_test2".to_string(),
            name: "Web Widget".to_string(),
            description: Some("Embeddable chat widget".to_string()),
            harness_id,
            agent_id: Some(agent.id.into()),
            agent_version_policy: "default".to_string(),
            agent_version_id: None,
            agent_identity_id: None,
            owner_principal_id: everruns_core::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            channel_type: Some("web".to_string()),
            channel_config: serde_json::json!({}),
            channel_config_encrypted: None,
        },
    )
    .await
    .unwrap();

    let results = db
        .list_apps(DEFAULT_ORG_ID, Some("slack"), false)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Slack Bot");

    // Multi-word across name + description
    let results = db
        .list_apps(DEFAULT_ORG_ID, Some("widget chat"), false)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Web Widget");
}

// ─── MessageFilter::Search tests (EVE-87) ───

/// Helper: create a session with events containing specific content.
async fn create_session_with_content_events(db: &InMemoryDatabase) -> SessionId {
    let agent = db
        .create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: "search-test-agent".to_string(),
                display_name: Some("Search Test Agent".to_string()),
                description: None,
                system_prompt: String::new(),
                default_model_id: None,

                harness_id: test_harness_id(),
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
            },
        )
        .await
        .unwrap();

    let session = db
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: DEFAULT_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: everruns_core::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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

    // Create events with different content
    let events_data = vec![
        (
            "input.message",
            serde_json::json!({
                "message": {
                    "id": "message_01933b5a00007000800000000000001",
                    "role": "user",
                    "content": [{"type": "text", "text": "Hello, how are you?"}]
                }
            }),
        ),
        (
            "output.message.completed",
            serde_json::json!({
                "message": {
                    "id": "message_01933b5a00007000800000000000002",
                    "role": "assistant",
                    "content": [{"type": "text", "text": "I am doing great, thank you!"}]
                }
            }),
        ),
        (
            "input.message",
            serde_json::json!({
                "message": {
                    "id": "message_01933b5a00007000800000000000003",
                    "role": "user",
                    "content": [{"type": "text", "text": "Tell me about Rust programming"}]
                }
            }),
        ),
        (
            "tool.completed",
            serde_json::json!({
                "tool_name": "search",
                "result": [{"type": "text", "text": "Rust is a systems language"}]
            }),
        ),
        (
            "output.message.completed",
            serde_json::json!({
                "message": {
                    "id": "message_01933b5a00007000800000000000004",
                    "role": "assistant",
                    "content": [{"type": "text", "text": "Here is information about Rust"}]
                }
            }),
        ),
        // Event with no content field
        ("turn.started", serde_json::json!({"turn_id": "abc123"})),
    ];

    for (event_type, data) in events_data {
        db.create_event(CreateEventRow {
            session_id: session.id,
            event_type: event_type.to_string(),
            ts: Utc::now(),
            context: serde_json::json!({}),
            data,
            metadata: None,
            tags: None,
        })
        .await
        .unwrap();
    }

    session.id
}

#[tokio::test]
async fn test_search_filter_matches_content_field() {
    let db = InMemoryDatabase::new();
    let session_id = create_session_with_content_events(&db).await;

    let query =
        MessageQuery::new(session_id).with_filter(MessageFilter::Search("Rust".to_string()));

    let events = db.list_message_events_filtered(&query).await.unwrap();
    // Should match: "Tell me about Rust programming", "Rust is a systems language",
    // "Here is information about Rust"
    assert_eq!(events.len(), 3);
}

#[tokio::test]
async fn test_search_filter_case_insensitive() {
    let db = InMemoryDatabase::new();
    let session_id = create_session_with_content_events(&db).await;

    let query =
        MessageQuery::new(session_id).with_filter(MessageFilter::Search("rust".to_string()));

    let events = db.list_message_events_filtered(&query).await.unwrap();
    assert_eq!(events.len(), 3);

    // Also uppercase
    let query =
        MessageQuery::new(session_id).with_filter(MessageFilter::Search("RUST".to_string()));

    let events = db.list_message_events_filtered(&query).await.unwrap();
    assert_eq!(events.len(), 3);
}

#[tokio::test]
async fn test_search_filter_no_match() {
    let db = InMemoryDatabase::new();
    let session_id = create_session_with_content_events(&db).await;

    let query = MessageQuery::new(session_id)
        .with_filter(MessageFilter::Search("nonexistent_xyz".to_string()));

    let events = db.list_message_events_filtered(&query).await.unwrap();
    assert!(events.is_empty());
}

#[tokio::test]
async fn test_search_filter_skips_events_without_content() {
    let db = InMemoryDatabase::new();
    let session_id = create_session_with_content_events(&db).await;

    // "abc123" is in turn_id, not content — should not match
    let query =
        MessageQuery::new(session_id).with_filter(MessageFilter::Search("abc123".to_string()));

    let events = db.list_message_events_filtered(&query).await.unwrap();
    assert!(events.is_empty());
}

#[tokio::test]
async fn test_search_filter_partial_match() {
    let db = InMemoryDatabase::new();
    let session_id = create_session_with_content_events(&db).await;

    let query =
        MessageQuery::new(session_id).with_filter(MessageFilter::Search("great".to_string()));

    let events = db.list_message_events_filtered(&query).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "output.message.completed");
}

#[tokio::test]
async fn test_search_filter_combined_with_event_type() {
    let db = InMemoryDatabase::new();
    let session_id = create_session_with_content_events(&db).await;

    // Search for "Rust" but only in input.message events
    let query = MessageQuery::new(session_id)
        .with_filter(MessageFilter::EventTypes(vec!["input.message".to_string()]))
        .with_filter(MessageFilter::Search("Rust".to_string()));

    let events = db.list_message_events_filtered(&query).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "input.message");
}

#[tokio::test]
async fn test_list_sessions_waiting_tool_results_before() {
    let db = InMemoryDatabase::new();
    let now = Utc::now();
    let cutoff = now - chrono::Duration::minutes(5);

    // Create 3 sessions: one waiting+old, one waiting+recent, one active+old
    let s1 = db
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: DEFAULT_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: everruns_core::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!({}),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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
    let s2 = db
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: DEFAULT_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: everruns_core::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!({}),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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
    let s3 = db
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: DEFAULT_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: everruns_core::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!({}),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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

    // Manually set statuses and updated_at
    {
        let mut sessions = db.sessions.write();
        if let Some(s) = sessions.get_mut(&s1.id) {
            s.status = "waiting_for_tool_results".to_string();
            s.updated_at = now - chrono::Duration::minutes(10); // old, should match
        }
        if let Some(s) = sessions.get_mut(&s2.id) {
            s.status = "waiting_for_tool_results".to_string();
            s.updated_at = now - chrono::Duration::minutes(1); // recent, should NOT match
        }
        if let Some(s) = sessions.get_mut(&s3.id) {
            s.status = "active".to_string();
            s.updated_at = now - chrono::Duration::minutes(10); // old but active, should NOT match
        }
    }

    let result = db
        .list_sessions_waiting_tool_results_before(cutoff)
        .await
        .unwrap();

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, s1.id);
    assert_eq!(result[0].1, DEFAULT_ORG_ID);
}

#[tokio::test]
async fn test_search_filter_empty_string_matches_all() {
    let db = InMemoryDatabase::new();
    let session_id = create_session_with_content_events(&db).await;

    // Empty search string should match events that have content field
    // (empty string is contained in any string)
    let query = MessageQuery::new(session_id).with_filter(MessageFilter::Search(String::new()));

    let events = db.list_message_events_filtered(&query).await.unwrap();
    // All default message-type events have content, so all should match
    // Default types: input.message, output.message.completed, tool.completed
    assert_eq!(events.len(), 5);
}

#[tokio::test]
async fn test_filtered_message_limit_keeps_latest_events_chronological() {
    let db = InMemoryDatabase::new();
    let session_id = create_session_with_content_events(&db).await;

    let query = MessageQuery::new(session_id).with_limit(2);

    let events = db.list_message_events_filtered(&query).await.unwrap();

    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_type, "tool.completed");
    assert_eq!(events[1].event_type, "output.message.completed");
}

#[tokio::test]
async fn test_session_system_prompt_and_initial_files_round_trip() {
    let db = InMemoryDatabase::new();

    let initial_files = serde_json::json!([
        {"path": "/workspace/hello.txt", "content": "hello", "encoding": "text"},
        {"path": "/workspace/config.json", "content": "{}", "encoding": "text"}
    ]);

    let session = db
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: DEFAULT_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: everruns_core::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            title: Some("Override Test".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: Some("You are a session-level override".to_string()),
            initial_files: initial_files.clone(),
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

    assert_eq!(
        session.system_prompt,
        Some("You are a session-level override".to_string())
    );
    assert_eq!(session.initial_files, initial_files);

    // Verify round-trip via get
    let fetched = db
        .get_session(DEFAULT_ORG_ID, session.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fetched.system_prompt,
        Some("You are a session-level override".to_string())
    );
    assert_eq!(fetched.initial_files, initial_files);
}

#[tokio::test]
async fn test_session_system_prompt_defaults_to_none() {
    let db = InMemoryDatabase::new();

    let session = db
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: DEFAULT_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: everruns_core::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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

    assert_eq!(session.system_prompt, None);
    assert_eq!(session.initial_files, serde_json::json!([]));
}

#[tokio::test]
async fn test_delete_user_account() {
    let db = InMemoryDatabase::new();

    let user = db
        .create_user(CreateUserRow {
            email: "delete@example.com".to_string(),
            name: "Delete Me".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .unwrap();

    // Create personal access token for user
    db.create_personal_access_token(CreatePersonalAccessTokenRow {
        user_id: user.id,
        name: "test-token".to_string(),
        token_hash: "hash123".to_string(),
        token_prefix: "evr_pat_".to_string(),
        scopes: vec![],
        expires_at: None,
        metadata: serde_json::json!({}),
    })
    .await
    .unwrap();

    // Create refresh token for user
    db.create_refresh_token(CreateRefreshTokenRow {
        user_id: user.id,
        token_hash: "refresh_hash".to_string(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
    })
    .await
    .unwrap();

    // Add user to default org
    db.add_organization_member(DEFAULT_ORG_ID, user.id, "member")
        .await
        .unwrap();

    // Verify user exists with related data
    assert!(db.get_user(user.id).await.unwrap().is_some());
    assert_eq!(
        db.list_personal_access_tokens_for_user(user.id)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        db.is_organization_member(DEFAULT_ORG_ID, user.id)
            .await
            .unwrap()
    );

    // Delete account
    let deleted = db.delete_user_account(user.id).await.unwrap();
    assert!(deleted);

    // Verify cascading delete of all user data
    assert!(db.get_user(user.id).await.unwrap().is_none());
    assert_eq!(
        db.list_personal_access_tokens_for_user(user.id)
            .await
            .unwrap()
            .len(),
        0
    );
    assert!(
        !db.is_organization_member(DEFAULT_ORG_ID, user.id)
            .await
            .unwrap()
    );

    // Deleting non-existent user returns false
    let deleted_again = db.delete_user_account(user.id).await.unwrap();
    assert!(!deleted_again);
}

#[tokio::test]
async fn test_export_user_data() {
    let db = InMemoryDatabase::new();

    let user = db
        .create_user(CreateUserRow {
            email: "export@example.com".to_string(),
            name: "Export User".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: Some("local".to_string()),
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .unwrap();

    // Create personal access token
    db.create_personal_access_token(CreatePersonalAccessTokenRow {
        user_id: user.id,
        name: "my-token".to_string(),
        token_hash: "hash456".to_string(),
        token_prefix: "evr_pat_".to_string(),
        scopes: vec!["read".to_string()],
        expires_at: None,
        metadata: serde_json::json!({}),
    })
    .await
    .unwrap();

    // Export data
    let export = db.export_user_data(user.id).await.unwrap().unwrap();

    assert_eq!(export["user"]["email"], "export@example.com");
    assert_eq!(export["user"]["name"], "Export User");
    assert_eq!(
        export["personal_access_tokens"].as_array().unwrap().len(),
        1
    );
    assert_eq!(export["personal_access_tokens"][0]["name"], "my-token");
    // Verify no sensitive data is exported
    assert!(
        export["personal_access_tokens"][0]
            .get("token_hash")
            .is_none()
    );
    assert!(export.get("exported_at").is_some());

    // Non-existent user returns None
    let missing = db.export_user_data(uuid::Uuid::now_v7()).await.unwrap();
    assert!(missing.is_none());
}

#[tokio::test]
async fn test_user_preferences_crud_and_isolation() {
    let db = InMemoryDatabase::new();
    let user_a = uuid::Uuid::now_v7();
    let user_b = uuid::Uuid::now_v7();

    // Missing key reads as None.
    assert!(
        db.get_user_preference(user_a, "theme")
            .await
            .unwrap()
            .is_none()
    );

    // Set creates the row.
    let created = db
        .set_user_preference(user_a, "theme", "\"dark\"", 100)
        .await
        .unwrap();
    assert_eq!(created.key, "theme");
    assert_eq!(created.value, "\"dark\"");

    // Set again upserts (updates value, keeps identity, no duplicate row).
    let updated = db
        .set_user_preference(user_a, "theme", "\"light\"", 100)
        .await
        .unwrap();
    assert_eq!(updated.id, created.id, "upsert must reuse the same row");
    assert_eq!(updated.value, "\"light\"");
    assert_eq!(
        db.list_user_preferences(user_a, 100).await.unwrap().len(),
        1
    );

    // Preferences are isolated per user.
    db.set_user_preference(user_b, "theme", "\"system\"", 100)
        .await
        .unwrap();
    let quota_error = db
        .set_user_preference(user_b, "locale", "\"en\"", 1)
        .await
        .unwrap_err();
    assert_eq!(
        quota_error.to_string(),
        super::super::backend::USER_PREFERENCE_LIMIT_EXCEEDED
    );
    assert_eq!(
        db.get_user_preference(user_a, "theme")
            .await
            .unwrap()
            .unwrap()
            .value,
        "\"light\""
    );
    assert_eq!(
        db.list_user_preferences(user_b, 100).await.unwrap().len(),
        1
    );

    // Delete removes only the targeted key and reports whether a row was hit.
    assert!(db.delete_user_preference(user_a, "theme").await.unwrap());
    assert!(!db.delete_user_preference(user_a, "theme").await.unwrap());
    assert!(
        db.get_user_preference(user_a, "theme")
            .await
            .unwrap()
            .is_none()
    );
    // user_b is unaffected by user_a's delete.
    assert_eq!(
        db.list_user_preferences(user_b, 100).await.unwrap().len(),
        1
    );
}

// Account linking: signing in with an OAuth provider whose verified email
// matches an existing password account must attach the provider identity to
// that account (same email = same account) WITHOUT dropping password auth.
// Mirrors the linking branch in `oauth_callback` (crates/server/src/auth/routes.rs).
#[tokio::test]
async fn link_oauth_identity_attaches_provider_and_preserves_password() {
    let db = InMemoryDatabase::new();

    let email = format!("linker-{}@example.com", Uuid::now_v7());
    let user = db
        .create_user(CreateUserRow {
            email: email.clone(),
            name: "Linker".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: Some("argon2-hash".to_string()),
            email_verified: true,
            auth_provider: Some("local".to_string()),
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .unwrap();

    // Before linking, the Google identity resolves to nobody.
    assert!(
        db.get_user_by_oauth("google", "google-sub-1")
            .await
            .unwrap()
            .is_none()
    );

    let linked = db
        .link_oauth_identity(user.id, "google", "google-sub-1")
        .await
        .unwrap()
        .expect("existing user linked");
    assert_eq!(linked.id, user.id);

    // Google login now resolves to the same account.
    let by_oauth = db
        .get_user_by_oauth("google", "google-sub-1")
        .await
        .unwrap()
        .expect("oauth lookup resolves to linked account");
    assert_eq!(by_oauth.id, user.id);

    // Password auth is preserved: hash intact and email lookup still works, so
    // password login and password reset keep functioning for the linked user.
    assert_eq!(by_oauth.password_hash.as_deref(), Some("argon2-hash"));
    let by_email = db.get_user_by_email(&email).await.unwrap().unwrap();
    assert_eq!(by_email.id, user.id);
    assert_eq!(by_email.password_hash.as_deref(), Some("argon2-hash"));

    // Linking a non-existent user is a no-op (None), not an error.
    assert!(
        db.link_oauth_identity(Uuid::now_v7(), "google", "x")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn link_oauth_identity_preserves_other_provider_logins() {
    let db = InMemoryDatabase::new();
    let user = db
        .create_user(CreateUserRow {
            email: format!("multi-oauth-{}@example.com", Uuid::now_v7()),
            name: "Multi OAuth".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: Some("github".to_string()),
            auth_provider_id: Some("github-user-1".to_string()),
            external_id: None,
        })
        .await
        .unwrap();

    let linked = db
        .link_oauth_identity(user.id, "google", "google-user-1")
        .await
        .unwrap()
        .expect("second provider linked");
    assert_eq!(linked.id, user.id);

    for (provider, provider_id) in [("github", "github-user-1"), ("google", "google-user-1")] {
        assert_eq!(
            db.get_user_by_oauth(provider, provider_id)
                .await
                .unwrap()
                .expect("provider login resolves")
                .id,
            user.id
        );
    }
}

#[tokio::test]
async fn link_oauth_identity_does_not_replace_existing_provider_subject() {
    let db = InMemoryDatabase::new();
    let user = db
        .create_user(CreateUserRow {
            email: format!("provider-lock-{}@example.com", Uuid::now_v7()),
            name: "Provider Lock".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: Some("google".to_string()),
            auth_provider_id: Some("google-original".to_string()),
            external_id: None,
        })
        .await
        .unwrap();
    let other_user = db
        .create_user(CreateUserRow {
            email: format!("provider-lock-other-{}@example.com", Uuid::now_v7()),
            name: "Other User".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .unwrap();

    assert!(
        db.link_oauth_identity(user.id, "google", "google-replacement")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        db.get_user_by_oauth("google", "google-original")
            .await
            .unwrap()
            .expect("original identity remains")
            .id,
        user.id
    );
    assert!(
        db.get_user_by_oauth("google", "google-replacement")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.link_oauth_identity(other_user.id, "google", "google-original")
            .await
            .unwrap()
            .is_none()
    );
}

// EVE-704: user email is a case-insensitive identity. Registering `John@x.com`
// then `john@x.com` must resolve to a single account, and login / OAuth-linking
// lookups must find that account regardless of the casing supplied. This test
// fails against the pre-fix backend (verbatim store + exact-match lookup),
// where the second casing would appear as a distinct, unfound account.
#[tokio::test]
async fn test_user_email_is_case_insensitive_identity() {
    let db = InMemoryDatabase::new();

    // Registered with mixed case and stray surrounding whitespace.
    let created = db
        .create_user(CreateUserRow {
            email: "  John.Doe@Example.COM ".to_string(),
            name: "John".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: Some("argon2-hash".to_string()),
            email_verified: true,
            auth_provider: Some("local".to_string()),
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .unwrap();

    // Stored in canonical (trim + lowercase) form.
    assert_eq!(created.email, "john.doe@example.com");

    // Every casing of the same mailbox resolves to the one account — this is the
    // register pre-check (duplicate signup blocked), login, and OAuth-linking
    // lookup path in `auth::routes`.
    for lookup in [
        "john.doe@example.com",
        "John.Doe@Example.COM",
        "JOHN.DOE@EXAMPLE.COM",
        "  john.doe@example.com  ",
    ] {
        let found = db
            .get_user_by_email(lookup)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("email lookup {lookup:?} must resolve to the account"));
        assert_eq!(
            found.id, created.id,
            "lookup {lookup:?} resolved to wrong account"
        );
    }
}

#[test]
fn test_normalize_email_trims_and_lowercases() {
    assert_eq!(normalize_email("  Alice@Example.COM "), "alice@example.com");
    assert_eq!(normalize_email("alice@example.com"), "alice@example.com");
    assert_eq!(normalize_email("\tBob@X.io\n"), "bob@x.io");
}

// ============================================
// Agent trigger round-trips (EVE-757)
// ============================================

fn schedule_trigger_input(agent_id: AgentId) -> CreateAgentTriggerRow {
    CreateAgentTriggerRow {
        org_id: DEFAULT_ORG_ID,
        id: everruns_core::TriggerId::new(),
        agent_id,
        trigger_type: "schedule".to_string(),
        config: serde_json::json!({
            "cron_expression": "0 0 * * * *",
            "timezone": "UTC",
            "session_mode": "shared_session",
            "message": "hello",
        }),
        enabled: true,
        durable_schedule_id: None,
        execution_harness_id: None,
        execution_owner_principal_id: None,
        execution_resolved_owner_user_id: None,
        execution_agent_identity_id: None,
        execution_app_id: None,
    }
}

#[tokio::test]
async fn test_agent_trigger_create_get_list_update_delete_round_trip() {
    let db = InMemoryDatabase::new();
    let agent_id = AgentId::new();

    // Create
    let created = db
        .create_agent_trigger(schedule_trigger_input(agent_id))
        .await
        .unwrap();
    assert_eq!(created.status, "active");
    assert_eq!(created.trigger_type, "schedule");
    assert!(created.enabled);
    assert_eq!(created.agent_id, agent_id);

    // Config round-trips into the typed core accessor.
    let trigger = everruns_platform::AgentTrigger {
        id: created.id,
        agent_id: created.agent_id,
        trigger_type: created.trigger_type.as_str().into(),
        config: created.config.clone(),
        enabled: created.enabled,
        created_at: created.created_at,
        updated_at: created.updated_at,
        archived_at: created.archived_at,
        deleted_at: created.deleted_at,
    };
    let schedule = trigger.schedule_config().unwrap();
    assert_eq!(schedule.cron_expression, "0 0 * * * *");
    assert_eq!(schedule.message, "hello");

    // Get
    let fetched = db
        .get_agent_trigger(DEFAULT_ORG_ID, created.id)
        .await
        .unwrap()
        .expect("trigger exists");
    assert_eq!(fetched.id, created.id);

    // Cross-org isolation.
    assert!(
        db.get_agent_trigger(999, created.id)
            .await
            .unwrap()
            .is_none()
    );

    // List
    let listed = db
        .list_agent_triggers(DEFAULT_ORG_ID, None, false)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);

    // Update
    let updated = db
        .update_agent_trigger(
            DEFAULT_ORG_ID,
            created.id,
            UpdateAgentTrigger {
                enabled: Some(false),
                config: Some(serde_json::json!({
                    "cron_expression": "0 5 * * * *",
                    "message": "updated",
                })),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .expect("update returns row");
    assert!(!updated.enabled);
    assert_eq!(updated.config["message"], serde_json::json!("updated"));

    // Soft delete (archive)
    assert!(
        db.delete_agent_trigger(DEFAULT_ORG_ID, created.id)
            .await
            .unwrap()
    );
    let after_delete = db
        .get_agent_trigger(DEFAULT_ORG_ID, created.id)
        .await
        .unwrap()
        .expect("row still present after soft delete");
    assert_eq!(after_delete.status, "archived");
    assert!(after_delete.archived_at.is_some());

    // Archived rows are excluded unless include_archived.
    assert!(
        db.list_agent_triggers(DEFAULT_ORG_ID, None, false)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        db.list_agent_triggers(DEFAULT_ORG_ID, None, true)
            .await
            .unwrap()
            .len(),
        1
    );

    // Second delete is a no-op (already archived, not active).
    assert!(
        !db.delete_agent_trigger(DEFAULT_ORG_ID, created.id)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn test_agent_trigger_set_durable_schedule_id() {
    let db = InMemoryDatabase::new();
    let created = db
        .create_agent_trigger(schedule_trigger_input(AgentId::new()))
        .await
        .unwrap();
    assert!(created.durable_schedule_id.is_none());

    let schedule_id = uuid::Uuid::now_v7();
    let bound = db
        .set_agent_trigger_durable_schedule_id(DEFAULT_ORG_ID, created.id, Some(schedule_id))
        .await
        .unwrap()
        .expect("bind returns row");
    assert_eq!(bound.durable_schedule_id, Some(schedule_id));

    // Clearing the binding works too.
    let cleared = db
        .set_agent_trigger_durable_schedule_id(DEFAULT_ORG_ID, created.id, None)
        .await
        .unwrap()
        .expect("clear returns row");
    assert!(cleared.durable_schedule_id.is_none());
}

#[tokio::test]
async fn test_agent_trigger_list_filters_by_agent() {
    let db = InMemoryDatabase::new();
    let agent_a = AgentId::new();
    let agent_b = AgentId::new();

    db.create_agent_trigger(schedule_trigger_input(agent_a))
        .await
        .unwrap();
    db.create_agent_trigger(schedule_trigger_input(agent_a))
        .await
        .unwrap();
    db.create_agent_trigger(schedule_trigger_input(agent_b))
        .await
        .unwrap();

    let for_a = db
        .list_agent_triggers(DEFAULT_ORG_ID, Some(agent_a), false)
        .await
        .unwrap();
    assert_eq!(for_a.len(), 2);
    assert!(for_a.iter().all(|t| t.agent_id == agent_a));

    let for_b = db
        .list_agent_triggers(DEFAULT_ORG_ID, Some(agent_b), false)
        .await
        .unwrap();
    assert_eq!(for_b.len(), 1);

    let all = db
        .list_agent_triggers(DEFAULT_ORG_ID, None, false)
        .await
        .unwrap();
    assert_eq!(all.len(), 3);
}
