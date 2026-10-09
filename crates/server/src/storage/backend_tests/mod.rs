//! Repository behavior on a real database: each test runs on a private
//! copy of the migrated schema (`StorageBackend::test_database`).

use super::CreateAgentTriggerRow;
use super::*;
use crate::common_dto::Pagination;
use crate::domains::sessions::record::{SessionParticipantKind, SessionParticipantRole};
use chrono::Utc;
use everruns_contracts::typed_id::{AgentId, HarnessId, PrincipalId, SessionId};
use everruns_contracts::typed_id::{EventId, SkillId};
use everruns_core::DEFAULT_ORG_ID;
use everruns_core::message_filter::{MessageFilter, MessageQuery};
use uuid::Uuid;
/// Default pagination for tests (large enough to not truncate).
pub(super) fn default_pagination() -> Pagination {
    Pagination::new(0, 1000)
}

pub(super) fn test_harness_id() -> HarnessId {
    HarnessId::from_uuid(uuid::Uuid::nil())
}

pub(super) fn test_session_input(agent_id: Option<AgentId>) -> CreateSessionRow {
    CreateSessionRow {
        playground_user_id: None,
        source: crate::domains::sessions::record::SessionSource::Api,
        workspace_id: None,
        org_id: DEFAULT_ORG_ID,
        app_id: None,
        channel_id: None,
        trigger_id: None,
        harness_id: None,
        agent_id,
        agent_revision: None,
        virtual_user_id: None,
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

/// Helper: create an agent + session for event filter tests.
pub(super) async fn create_session_with_events(db: &StorageBackend) -> SessionId {
    let agent = db
        .create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: format!("filter-test-agent-{}", Uuid::now_v7().simple()),
                display_name: Some("Filter Test Agent".to_string()),
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

/// Helper: create a session with events containing specific content.
pub(super) async fn create_session_with_content_events(db: &StorageBackend) -> SessionId {
    let agent = db
        .create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: "search-test-agent".to_string(),
                display_name: Some("Search Test Agent".to_string()),
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

pub(super) fn schedule_trigger_input(agent_id: AgentId) -> CreateAgentTriggerRow {
    CreateAgentTriggerRow {
        org_id: DEFAULT_ORG_ID,
        id: everruns_contracts::typed_id::TriggerId::new(),
        agent_id,
        trigger_type: "schedule".to_string(),
        ingress_id: None,
        config: serde_json::json!({
            "cron_expression": "0 0 * * * *",
            "timezone": "UTC",
            "session_mode": "shared_session",
            "message": "hello",
        }),
        config_encrypted: None,
        enabled: true,
        durable_schedule_id: None,
        execution_harness_id: None,
        execution_owner_principal_id: None,
        execution_resolved_owner_user_id: None,
        execution_virtual_user_id: None,
        execution_app_id: None,
        legacy_alias_id: None,
        legacy_alias_name: None,
    }
}

/// Helper: create an agent with given name + description
pub(super) async fn create_test_agent(
    db: &StorageBackend,
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
    .unwrap()
}

/// Set a session's status and `updated_at`, bypassing the trigger that would
/// stamp `updated_at` with the current time.
pub(super) async fn set_session_status_and_updated_at(
    db: &StorageBackend,
    id: SessionId,
    status: &str,
    updated_at: chrono::DateTime<Utc>,
) {
    let mut conn = db.database().pool().acquire().await.unwrap();
    sqlx::query("SET session_replication_role = replica")
        .execute(&mut *conn)
        .await
        .unwrap();
    sqlx::query("UPDATE sessions SET status = $2, updated_at = $3 WHERE id = $1")
        .bind(id)
        .bind(status)
        .bind(updated_at)
        .execute(&mut *conn)
        .await
        .unwrap();
    sqlx::query("SET session_replication_role = origin")
        .execute(&mut *conn)
        .await
        .unwrap();
}

mod agent_scripts;
mod part1;
mod part2;
mod part3;
