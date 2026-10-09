use super::*;

#[tokio::test]
async fn test_list_events_types_fully_excluded() {
    let db = StorageBackend::test_database();
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
    let db = StorageBackend::test_database();
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
    let db = StorageBackend::test_database();
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
    let db = StorageBackend::test_database();
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
    let db = StorageBackend::test_database();
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

    let db = StorageBackend::test_database();
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

    let db = StorageBackend::test_database();
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

    let db = StorageBackend::test_database();
    let session_a = create_session_with_events(&db).await;
    let session_b = create_session_with_events(&db).await;

    // A since_id from another session must not anchor session A's cursor:
    // it matches no row there, so the result is empty.
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
    assert!(
        result.is_empty(),
        "since_id from another session must yield empty"
    );
}

#[tokio::test]
async fn test_list_events_advanced_order_desc_returns_newest_first() {
    use crate::storage::models::ListEventsParams;

    let db = StorageBackend::test_database();
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
    let db = StorageBackend::test_database();
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
    let db = StorageBackend::test_database();
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
    let db = StorageBackend::test_database();
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
    let db = StorageBackend::test_database();
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
    let db = StorageBackend::test_database();
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
    let db = StorageBackend::test_database();
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
    let db = StorageBackend::test_database();
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
    let db = StorageBackend::test_database();
    let agent = db
        .create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: "empty-agent".to_string(),
                display_name: Some("Empty Agent".to_string()),
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

    // Empty session with limit should return empty
    let events = db
        .list_events(session.id, None, None, &[], &[], None, Some(200))
        .await
        .unwrap();
    assert!(events.is_empty());
}

#[tokio::test]
async fn test_sessions_pagination() {
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

    // Create 15 sessions
    for i in 0..15 {
        db.create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            agent_id: Some(agent.id),
            owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
            title: Some(format!("Session {}", i)),
            ..Default::default()
        })
        .await
        .unwrap();
    }

    // Test default pagination (all sessions fit within limit)
    let pagination = crate::common_dto::Pagination::new(0, 20);
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
    let pagination = crate::common_dto::Pagination::new(0, 5);
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
    let pagination = crate::common_dto::Pagination::new(5, 5);
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
    let pagination = crate::common_dto::Pagination::new(10, 10);
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
    let pagination = crate::common_dto::Pagination::new(20, 10);
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

    // Create sessions with sequential titles
    for i in 1..=5 {
        db.create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            agent_id: Some(agent.id),
            owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
            title: Some(format!("Session {}", i)),
            ..Default::default()
        })
        .await
        .unwrap();
        // Small delay to ensure different created_at timestamps
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    }

    // Sessions should be ordered by created_at DESC (newest first)
    let pagination = crate::common_dto::Pagination::new(0, 10);
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
    let db = StorageBackend::test_database();

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
    let db = StorageBackend::test_database();

    let user_id = db.create_test_user(Uuid::now_v7()).await;
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
    let db = StorageBackend::test_database();

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
    let db = StorageBackend::test_database();

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
    let db = StorageBackend::test_database();

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
    let db = StorageBackend::test_database();

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

#[tokio::test]
async fn test_search_agents_no_filter_returns_all() {
    let db = StorageBackend::test_database();
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
    let db = StorageBackend::test_database();
    create_test_agent(&db, "Alpha", None).await;

    let (results, _total) = db
        .list_agents(DEFAULT_ORG_ID, Some(""), false, default_pagination())
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
}

#[tokio::test]
async fn test_search_agents_single_word_match() {
    let db = StorageBackend::test_database();
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
    let db = StorageBackend::test_database();
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
