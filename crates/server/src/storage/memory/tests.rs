use super::super::models::*;
use super::*;
use crate::api::common::Pagination;
use chrono::Utc;
use everruns_core::message_filter::{MessageFilter, MessageQuery};
use everruns_core::{AgentId, DEFAULT_ORG_ID, SessionId};

/// Default pagination for tests (large enough to not truncate).
fn default_pagination() -> Pagination {
    Pagination::new(0, 1000)
}

#[tokio::test]
async fn test_create_and_get_agent() {
    let db = InMemoryDatabase::new();

    let agent = db
        .create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: "Test Agent".to_string(),
                description: Some("A test agent".to_string()),
                system_prompt: "You are helpful".to_string(),
                default_model_id: None,
                tags: vec!["test".to_string()],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                network_access: None,
                max_iterations: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(agent.name, "Test Agent");

    let fetched = db.get_agent(DEFAULT_ORG_ID, agent.id).await.unwrap();
    assert!(fetched.is_some());
    assert_eq!(fetched.unwrap().name, "Test Agent");
}

#[tokio::test]
async fn test_create_and_list_sessions() {
    let db = InMemoryDatabase::new();

    let agent = db
        .create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: "Test Agent".to_string(),
                description: None,
                system_prompt: String::new(),
                default_model_id: None,
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                network_access: None,
                max_iterations: None,
            },
        )
        .await
        .unwrap();

    let session = db
        .create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_identity_id: None,
            title: Some("Test Session".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
            hints: None,
            network_access: None,
            max_iterations: None,
            blueprint_id: None,
            blueprint_config: None,
        })
        .await
        .unwrap();

    let pagination = crate::api::common::Pagination::new(0, 20);
    let (sessions, total) = db
        .list_sessions(DEFAULT_ORG_ID, Some(agent.id), None, pagination)
        .await
        .unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(total, 1);
    assert_eq!(sessions[0].id, session.id);
}

#[tokio::test]
async fn test_session_updated_at() {
    let db = InMemoryDatabase::new();

    let agent = db
        .create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: "Test Agent".to_string(),
                description: None,
                system_prompt: String::new(),
                default_model_id: None,
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                network_access: None,
                max_iterations: None,
            },
        )
        .await
        .unwrap();

    // Create session - updated_at should equal created_at
    let session = db
        .create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_identity_id: None,
            title: Some("Test Session".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
            hints: None,
            network_access: None,
            max_iterations: None,
            blueprint_id: None,
            blueprint_config: None,
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
                name: "Test Agent".to_string(),
                description: None,
                system_prompt: String::new(),
                default_model_id: None,
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                network_access: None,
                max_iterations: None,
            },
        )
        .await
        .unwrap();

    let session = db
        .create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_identity_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
            hints: None,
            network_access: None,
            max_iterations: None,
            blueprint_id: None,
            blueprint_config: None,
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

/// Helper: create an agent + session for event filter tests.
async fn create_session_with_events(db: &InMemoryDatabase) -> SessionId {
    let agent = db
        .create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: "Filter Test Agent".to_string(),
                description: None,
                system_prompt: String::new(),
                default_model_id: None,
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                network_access: None,
                max_iterations: None,
            },
        )
        .await
        .unwrap();

    let session = db
        .create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_identity_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
            hints: None,
            network_access: None,
            max_iterations: None,
            blueprint_id: None,
            blueprint_config: None,
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
                name: "Empty Agent".to_string(),
                description: None,
                system_prompt: String::new(),
                default_model_id: None,
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                network_access: None,
                max_iterations: None,
            },
        )
        .await
        .unwrap();

    let session = db
        .create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_identity_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
            hints: None,
            network_access: None,
            max_iterations: None,
            blueprint_id: None,
            blueprint_config: None,
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
                name: "Test Agent".to_string(),
                description: None,
                system_prompt: String::new(),
                default_model_id: None,
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                network_access: None,
                max_iterations: None,
            },
        )
        .await
        .unwrap();

    // Create 15 sessions
    for i in 0..15 {
        db.create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_identity_id: None,
            title: Some(format!("Session {}", i)),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
            hints: None,
            network_access: None,
            max_iterations: None,
            blueprint_id: None,
            blueprint_config: None,
        })
        .await
        .unwrap();
    }

    // Test default pagination (all sessions fit within limit)
    let pagination = crate::api::common::Pagination::new(0, 20);
    let (sessions, total) = db
        .list_sessions(DEFAULT_ORG_ID, Some(agent.id), None, pagination)
        .await
        .unwrap();
    assert_eq!(total, 15);
    assert_eq!(sessions.len(), 15);

    // Test with limit=5
    let pagination = crate::api::common::Pagination::new(0, 5);
    let (sessions, total) = db
        .list_sessions(DEFAULT_ORG_ID, Some(agent.id), None, pagination)
        .await
        .unwrap();
    assert_eq!(total, 15);
    assert_eq!(sessions.len(), 5);

    // Test with offset=5, limit=5
    let pagination = crate::api::common::Pagination::new(5, 5);
    let (sessions, total) = db
        .list_sessions(DEFAULT_ORG_ID, Some(agent.id), None, pagination)
        .await
        .unwrap();
    assert_eq!(total, 15);
    assert_eq!(sessions.len(), 5);

    // Test last partial page (offset=10, limit=10 should return 5)
    let pagination = crate::api::common::Pagination::new(10, 10);
    let (sessions, total) = db
        .list_sessions(DEFAULT_ORG_ID, Some(agent.id), None, pagination)
        .await
        .unwrap();
    assert_eq!(total, 15);
    assert_eq!(sessions.len(), 5);

    // Test beyond range (offset=20)
    let pagination = crate::api::common::Pagination::new(20, 10);
    let (sessions, total) = db
        .list_sessions(DEFAULT_ORG_ID, Some(agent.id), None, pagination)
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
                name: "Test Agent".to_string(),
                description: None,
                system_prompt: String::new(),
                default_model_id: None,
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                network_access: None,
                max_iterations: None,
            },
        )
        .await
        .unwrap();

    // Create sessions with sequential titles
    for i in 1..=5 {
        db.create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_identity_id: None,
            title: Some(format!("Session {}", i)),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
            hints: None,
            network_access: None,
            max_iterations: None,
            blueprint_id: None,
            blueprint_config: None,
        })
        .await
        .unwrap();
        // Small delay to ensure different created_at timestamps
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    }

    // Sessions should be ordered by created_at DESC (newest first)
    let pagination = crate::api::common::Pagination::new(0, 10);
    let (sessions, _) = db
        .list_sessions(DEFAULT_ORG_ID, Some(agent.id), None, pagination)
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
            description: description.map(|d| d.to_string()),
            system_prompt: String::new(),
            default_model_id: None,
            tags: vec![],
            initial_files: serde_json::json!([]),
            tools: serde_json::json!([]),
            network_access: None,
            max_iterations: None,
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
        org_id: DEFAULT_ORG_ID,
        harness_id: None,
        agent_id: Some(agent.id),
        agent_identity_id: None,
        title: Some("Debug production memory leak".to_string()),
        locale: None,
        tags: vec![],
        model_id: None,
        capabilities: serde_json::json!([]),
        tools: serde_json::json!([]),
        system_prompt: None,
        initial_files: serde_json::Value::Array(vec![]),
        hints: None,
        network_access: None,
        max_iterations: None,
        blueprint_id: None,
        blueprint_config: None,
    })
    .await
    .unwrap();

    db.create_session(CreateSessionRow {
        org_id: DEFAULT_ORG_ID,
        harness_id: None,
        agent_id: Some(agent.id),
        agent_identity_id: None,
        title: Some("Refactor auth module".to_string()),
        locale: None,
        tags: vec![],
        model_id: None,
        capabilities: serde_json::json!([]),
        tools: serde_json::json!([]),
        system_prompt: None,
        initial_files: serde_json::Value::Array(vec![]),
        hints: None,
        network_access: None,
        max_iterations: None,
        blueprint_id: None,
        blueprint_config: None,
    })
    .await
    .unwrap();

    let pagination = crate::api::common::Pagination::new(0, 20);
    let (results, total) = db
        .list_sessions(DEFAULT_ORG_ID, None, Some("memory leak"), pagination)
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
        org_id: DEFAULT_ORG_ID,
        harness_id: None,
        agent_id: Some(agent1.id),
        agent_identity_id: None,
        title: Some("Shared keyword session".to_string()),
        locale: None,
        tags: vec![],
        model_id: None,
        capabilities: serde_json::json!([]),
        tools: serde_json::json!([]),
        system_prompt: None,
        initial_files: serde_json::Value::Array(vec![]),
        hints: None,
        network_access: None,
        max_iterations: None,
        blueprint_id: None,
        blueprint_config: None,
    })
    .await
    .unwrap();

    db.create_session(CreateSessionRow {
        org_id: DEFAULT_ORG_ID,
        harness_id: None,
        agent_id: Some(agent2.id),
        agent_identity_id: None,
        title: Some("Shared keyword session".to_string()),
        locale: None,
        tags: vec![],
        model_id: None,
        capabilities: serde_json::json!([]),
        tools: serde_json::json!([]),
        system_prompt: None,
        initial_files: serde_json::Value::Array(vec![]),
        hints: None,
        network_access: None,
        max_iterations: None,
        blueprint_id: None,
        blueprint_config: None,
    })
    .await
    .unwrap();

    let pagination = crate::api::common::Pagination::new(0, 20);
    // Search + agent filter combined
    let (results, total) = db
        .list_sessions(
            DEFAULT_ORG_ID,
            Some(agent1.id),
            Some("shared keyword"),
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
        .list_sessions(DEFAULT_ORG_ID, None, Some(poem), pagination)
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
            agent_id: agent.id.into(),
            agent_identity_id: None,
            channel_type: "slack".to_string(),
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
            agent_id: agent.id.into(),
            agent_identity_id: None,
            channel_type: "web".to_string(),
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
                name: "Search Test Agent".to_string(),
                description: None,
                system_prompt: String::new(),
                default_model_id: None,
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                network_access: None,
                max_iterations: None,
            },
        )
        .await
        .unwrap();

    let session = db
        .create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_identity_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
            hints: None,
            network_access: None,
            max_iterations: None,
            blueprint_id: None,
            blueprint_config: None,
        })
        .await
        .unwrap();

    // Create events with different content
    let events_data = vec![
        (
            "input.message",
            serde_json::json!({"content": "Hello, how are you?"}),
        ),
        (
            "output.message.completed",
            serde_json::json!({"content": "I am doing great, thank you!"}),
        ),
        (
            "input.message",
            serde_json::json!({"content": "Tell me about Rust programming"}),
        ),
        (
            "tool.completed",
            serde_json::json!({"tool_name": "search", "content": "Rust is a systems language"}),
        ),
        (
            "output.message.completed",
            serde_json::json!({"content": "Here is information about Rust"}),
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
            org_id: DEFAULT_ORG_ID,
            harness_id: None,
            agent_id: None,
            agent_identity_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!({}),
            tools: serde_json::json!([]),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
            hints: None,
            network_access: None,
            max_iterations: None,
            blueprint_id: None,
            blueprint_config: None,
        })
        .await
        .unwrap();
    let s2 = db
        .create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            harness_id: None,
            agent_id: None,
            agent_identity_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!({}),
            tools: serde_json::json!([]),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
            hints: None,
            network_access: None,
            max_iterations: None,
            blueprint_id: None,
            blueprint_config: None,
        })
        .await
        .unwrap();
    let s3 = db
        .create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            harness_id: None,
            agent_id: None,
            agent_identity_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!({}),
            tools: serde_json::json!([]),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
            hints: None,
            network_access: None,
            max_iterations: None,
            blueprint_id: None,
            blueprint_config: None,
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
async fn test_session_system_prompt_and_initial_files_round_trip() {
    let db = InMemoryDatabase::new();

    let initial_files = serde_json::json!([
        {"path": "/workspace/hello.txt", "content": "hello", "encoding": "text"},
        {"path": "/workspace/config.json", "content": "{}", "encoding": "text"}
    ]);

    let session = db
        .create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            harness_id: None,
            agent_id: None,
            agent_identity_id: None,
            title: Some("Override Test".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            system_prompt: Some("You are a session-level override".to_string()),
            initial_files: initial_files.clone(),
            hints: None,
            network_access: None,
            max_iterations: None,
            blueprint_id: None,
            blueprint_config: None,
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
            org_id: DEFAULT_ORG_ID,
            harness_id: None,
            agent_id: None,
            agent_identity_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
            hints: None,
            network_access: None,
            max_iterations: None,
            blueprint_id: None,
            blueprint_config: None,
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

    // Create API key for user
    db.create_api_key(CreateApiKeyRow {
        org_id: DEFAULT_ORG_ID,
        user_id: user.id,
        name: "test-key".to_string(),
        key_hash: "hash123".to_string(),
        key_prefix: "evr_".to_string(),
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
    assert_eq!(db.list_api_keys_for_user(user.id).await.unwrap().len(), 1);
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
    assert_eq!(db.list_api_keys_for_user(user.id).await.unwrap().len(), 0);
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

    // Create API key
    db.create_api_key(CreateApiKeyRow {
        org_id: DEFAULT_ORG_ID,
        user_id: user.id,
        name: "my-key".to_string(),
        key_hash: "hash456".to_string(),
        key_prefix: "evr_".to_string(),
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
    assert_eq!(export["api_keys"].as_array().unwrap().len(), 1);
    assert_eq!(export["api_keys"][0]["name"], "my-key");
    // Verify no sensitive data is exported
    assert!(export["api_keys"][0].get("key_hash").is_none());
    assert!(export.get("exported_at").is_some());

    // Non-existent user returns None
    let missing = db.export_user_data(uuid::Uuid::now_v7()).await.unwrap();
    assert!(missing.is_none());
}
