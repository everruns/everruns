use super::*;
use chrono::DurationRound;

#[tokio::test]
async fn test_create_and_get_agent() {
    let db = StorageBackend::test_database();

    let agent = db
        .create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: "test-agent".to_string(),
                display_name: Some("Test Agent".to_string()),
                description: Some("A test agent".to_string()),
                intro_markdown: None,
                short_description: None,
                starters: serde_json::json!([]),
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
                environments: None,
                is_built_in: false,
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
    let db = StorageBackend::test_database();
    let row = db
        .create_declarative_capability(
            DEFAULT_ORG_ID,
            CreateDeclarativeCapabilityRow {
                public_id: everruns_contracts::typed_id::DeclarativeCapabilityId::new().to_string(),
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
    let db = StorageBackend::test_database();

    let agent = db
        .create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: "test-agent".to_string(),
                display_name: Some("Test Agent".to_string()),
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: serde_json::json!([]),
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
                environments: None,
                is_built_in: false,
            },
        )
        .await
        .unwrap();

    let session = db
        .create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            agent_id: Some(agent.id),
            owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
            title: Some("Test Session".to_string()),
            ..Default::default()
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
    let db = StorageBackend::test_database();

    let new_session = || CreateSessionRow {
        org_id: DEFAULT_ORG_ID,
        owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
        ..Default::default()
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
    let db = StorageBackend::test_database();
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
    let db = StorageBackend::test_database();
    let agent_id = AgentId::from_uuid(db.create_test_agent(DEFAULT_ORG_ID, Uuid::now_v7()).await);

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
    let db = StorageBackend::test_database();
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
async fn test_create_session_records_agent_revision_and_seeds_agent_participant() {
    let db = StorageBackend::test_database();
    let agent_id = AgentId::from_uuid(db.create_test_agent(DEFAULT_ORG_ID, Uuid::now_v7()).await);
    let mut input = test_session_input(Some(agent_id));
    input.agent_revision = Some(7);

    let session = db.create_session(input).await.unwrap();
    let participants = db
        .list_session_participants(DEFAULT_ORG_ID, session.id)
        .await
        .unwrap();

    assert_eq!(session.agent_revision, Some(7));
    let host = participants
        .iter()
        .map(SessionParticipantRow::to_core)
        .find(|participant| participant.role == SessionParticipantRole::Host)
        .unwrap();
    assert_eq!(host.agent_id, Some(agent_id));
}

#[tokio::test]
async fn test_create_session_without_agent_seeds_user_participant_only() {
    let db = StorageBackend::test_database();

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
    let db = StorageBackend::test_database();
    let agent_id = AgentId::from_uuid(db.create_test_agent(DEFAULT_ORG_ID, Uuid::now_v7()).await);

    let session = db
        .create_session(test_session_input(Some(agent_id)))
        .await
        .unwrap();

    let err = db
        .create_session_participant(CreateSessionParticipantRow {
            org_id: DEFAULT_ORG_ID,
            session_id: session.id,
            kind: SessionParticipantKind::Agent,
            agent_id: Some(AgentId::from_uuid(
                db.create_test_agent(DEFAULT_ORG_ID, Uuid::now_v7()).await,
            )),
            principal_id: PrincipalId::from_seed(1),
            display_name: None,
            role: SessionParticipantRole::Host,
            joined_at: None,
        })
        .await
        .unwrap_err();

    assert!(
        format!("{err:#}").contains("session_participants_one_active_host_idx"),
        "{err:#}"
    );
}

#[tokio::test]
async fn test_ensure_active_user_session_participant_is_idempotent() {
    let db = StorageBackend::test_database();
    let session = db.create_session(test_session_input(None)).await.unwrap();
    let principal_id = db.create_test_principal(PrincipalId::from_seed(42)).await;

    let input = CreateSessionParticipantRow {
        org_id: DEFAULT_ORG_ID,
        session_id: session.id,
        kind: SessionParticipantKind::User,
        agent_id: None,
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
    let db = StorageBackend::test_database();
    let session = db.create_session(test_session_input(None)).await.unwrap();
    let member = db
        .create_session_participant(CreateSessionParticipantRow {
            org_id: DEFAULT_ORG_ID,
            session_id: session.id,
            kind: SessionParticipantKind::Agent,
            agent_id: Some(AgentId::from_uuid(
                db.create_test_agent(DEFAULT_ORG_ID, Uuid::now_v7()).await,
            )),
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
    let db = StorageBackend::test_database();

    let agent = db
        .create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: "stats-agent".to_string(),
                display_name: Some("Stats Agent".to_string()),
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: serde_json::json!([]),
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
                environments: None,
                is_built_in: false,
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
                icon: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: serde_json::json!([]),
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
            org_id: DEFAULT_ORG_ID,
            harness_id: Some(harness.id),
            agent_id: Some(agent.id),
            owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
            title: Some("Stats Session".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();
    // PostgreSQL keeps microseconds, so drop the nanoseconds up front.
    let started_at = (Utc::now() - chrono::Duration::seconds(10))
        .duration_trunc(chrono::Duration::microseconds(1))
        .unwrap();
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
    let db = StorageBackend::test_database();

    let agent = db
        .create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: "test-agent".to_string(),
                display_name: Some("Test Agent".to_string()),
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: serde_json::json!([]),
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
                environments: None,
                is_built_in: false,
            },
        )
        .await
        .unwrap();

    // Create session - updated_at should equal created_at
    let session = db
        .create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            agent_id: Some(agent.id),
            owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
            title: Some("Test Session".to_string()),
            ..Default::default()
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

    let db = StorageBackend::test_database();

    let agent = db
        .create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: "test-agent".to_string(),
                display_name: Some("Test Agent".to_string()),
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: serde_json::json!([]),
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
                environments: None,
                is_built_in: false,
            },
        )
        .await
        .unwrap();

    let session = db
        .create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            agent_id: Some(agent.id),
            owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
            ..Default::default()
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

    let db = StorageBackend::test_database();

    let agent = db
        .create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: "test-agent".to_string(),
                display_name: Some("Test Agent".to_string()),
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: serde_json::json!([]),
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
                environments: None,
                is_built_in: false,
            },
        )
        .await
        .unwrap();

    let session = db
        .create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            agent_id: Some(agent.id),
            owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
            ..Default::default()
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

    let db = StorageBackend::test_database();
    let session = db
        .create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
            ..Default::default()
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
async fn test_session_connections_never_use_management_owner_lineage() {
    let db = StorageBackend::test_database();

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
    db.create_test_virtual_user_with_id(DEFAULT_ORG_ID, other.id)
        .await;
    db.create_test_virtual_user_with_id(DEFAULT_ORG_ID, owner.id)
        .await;

    let session = db
        .create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            owner_principal_id: db
                .create_test_principal(everruns_contracts::typed_id::PrincipalId::from_seed(42))
                .await,
            resolved_owner_user_id: Some(owner.id),
            title: Some("connection-owner-scope".to_string()),
            ..Default::default()
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
}

#[tokio::test]
async fn test_unpin_session_is_scoped_by_org() {
    let db = StorageBackend::test_database();
    let user_id = db.create_test_user(Uuid::now_v7()).await;
    db.create_test_virtual_user_with_id(DEFAULT_ORG_ID, user_id)
        .await;

    let session = db
        .create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
            resolved_owner_user_id: Some(user_id),
            title: Some("Pinned Session".to_string()),
            ..Default::default()
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

#[tokio::test]
async fn test_count_events_no_materialization() {
    let db = StorageBackend::test_database();
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
    let db = StorageBackend::test_database();
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
    let db = StorageBackend::test_database();
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
    let db = StorageBackend::test_database();
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
    let db = StorageBackend::test_database();
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
    let db = StorageBackend::test_database();
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
    let db = StorageBackend::test_database();
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
