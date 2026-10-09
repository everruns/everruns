//! PostgreSQL contract for `GET /v1/agents/activity`: turns land in the hour
//! they started, failures count separately, old turns fall outside the window,
//! and the agent's load and triggers come back keyed by its public id. Channel
//! audience counts distinct people and the median time to the first reply.

use chrono::{Duration, Utc};
use serde_json::json;
use sqlx::PgPool;

use everruns_contracts::typed_id::TriggerId;
use everruns_core::DEFAULT_ORG_ID;
use everruns_server::api::agent_activity::build_overview;
use everruns_server::setup::org_init;
use everruns_server::storage::{
    CreateAgentChannelRow, CreateAgentTriggerRow, CreateEventRow, Database, StorageBackend,
    UpdateSession,
};

use crate::repository_conformance_test::{agent_input, create_test_principal, session_input};
use crate::test_harness::get_database_url;

async fn turn(
    backend: &StorageBackend,
    session_id: everruns_contracts::typed_id::SessionId,
    kind: &str,
    ago: Duration,
) {
    event(backend, session_id, kind, Utc::now() - ago, None).await;
}

async fn event(
    backend: &StorageBackend,
    session_id: everruns_contracts::typed_id::SessionId,
    kind: &str,
    ts: chrono::DateTime<Utc>,
    metadata: Option<serde_json::Value>,
) {
    backend
        .create_event(CreateEventRow {
            session_id,
            event_type: kind.to_string(),
            ts,
            context: json!({}),
            data: json!({}),
            metadata,
            tags: None,
        })
        .await
        .expect("create event");
}

#[tokio::test]
async fn activity_buckets_turns_by_hour_and_reports_load_and_triggers() {
    let pool = PgPool::connect(&get_database_url())
        .await
        .expect("connect to PostgreSQL");
    let backend = StorageBackend::from_database(Database::new(pool));
    org_init::initialize_org_harnesses(&backend, DEFAULT_ORG_ID)
        .await
        .expect("initialize built-in harnesses");
    let harness_id = org_init::generic_harness_id(&backend, DEFAULT_ORG_ID)
        .await
        .expect("generic harness id");
    let agent = backend
        .create_agent(
            DEFAULT_ORG_ID,
            agent_input(format!("activity-{}", uuid::Uuid::now_v7()), harness_id),
        )
        .await
        .expect("create agent");

    let owner = create_test_principal(&backend, "agent-activity").await;
    let mut input = session_input(owner, "agent-activity");
    input.agent_id = Some(agent.id);
    let session = backend.create_session(input).await.expect("create session");

    turn(&backend, session.id, "turn.started", Duration::minutes(150)).await;
    turn(
        &backend,
        session.id,
        "turn.completed",
        Duration::minutes(149),
    )
    .await;
    turn(&backend, session.id, "turn.started", Duration::minutes(30)).await;
    turn(&backend, session.id, "turn.failed", Duration::minutes(29)).await;
    // Outside the 24-hour window.
    turn(&backend, session.id, "turn.started", Duration::hours(30)).await;

    backend
        .update_session(
            DEFAULT_ORG_ID,
            session.id,
            UpdateSession {
                status: Some("active".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("mark session active")
        .expect("session exists");

    backend
        .create_agent_trigger(CreateAgentTriggerRow {
            org_id: DEFAULT_ORG_ID,
            id: TriggerId::new(),
            agent_id: agent.id,
            trigger_type: "schedule".to_string(),
            ingress_id: None,
            config: json!({
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
        })
        .await
        .expect("create trigger");

    let now = Utc::now();
    let rows = backend
        .agent_activity(DEFAULT_ORG_ID, now)
        .await
        .expect("load activity");
    let overview = build_overview(rows, now, 1000);
    let mine = overview
        .agents
        .iter()
        .find(|activity| activity.agent_id == agent.public_id)
        .expect("agent appears in activity");

    assert_eq!(mine.running_sessions, 1);
    assert_eq!(
        (mine.runs, mine.failed),
        (2, 1),
        "the 30-hour-old turn is outside"
    );
    // Hour 2 ago is index 21, hour 0 ago is index 23 (oldest first).
    assert_eq!(mine.hourly[21].runs, 1);
    assert_eq!(mine.hourly[23].runs, 1);
    assert_eq!(mine.hourly[23].failed, 1);
    assert!(mine.last_turn_at.is_some());
    assert_eq!(mine.triggers.len(), 1);
    assert_eq!(mine.triggers[0].trigger_type, "schedule");
}

#[tokio::test]
async fn channel_audience_counts_people_once_and_takes_the_median_first_reply() {
    let pool = PgPool::connect(&get_database_url())
        .await
        .expect("connect to PostgreSQL");
    let backend = StorageBackend::from_database(Database::new(pool));
    org_init::initialize_org_harnesses(&backend, DEFAULT_ORG_ID)
        .await
        .expect("initialize built-in harnesses");
    let harness_id = org_init::generic_harness_id(&backend, DEFAULT_ORG_ID)
        .await
        .expect("generic harness id");
    let agent = backend
        .create_agent(
            DEFAULT_ORG_ID,
            agent_input(format!("audience-{}", uuid::Uuid::now_v7()), harness_id),
        )
        .await
        .expect("create agent");
    let owner = create_test_principal(&backend, "agent-audience").await;
    let channel = backend
        .create_agent_channel(
            DEFAULT_ORG_ID,
            CreateAgentChannelRow {
                agent_id: agent.id.uuid(),
                public_id: format!("appchan_{}", uuid::Uuid::now_v7().simple()),
                channel_type: "public_chat".to_string(),
                channel_config: json!({}),
                channel_config_encrypted: None,
                auth: None,
                auth_encrypted: None,
                enabled: true,
                status: "live".to_string(),
                virtual_user_id: None,
                owner_principal_id: owner.uuid(),
                resolved_owner_user_id: None,
            },
        )
        .await
        .expect("create channel");

    let visitor =
        |principal: &str| Some(json!({ "type": "virtual_user", "principal_id": principal }));
    // Two sessions from one visitor (replies in 2s and 4s), one from another
    // visitor that never got a reply, and one with no identity at all.
    let replies = [
        (Some("p-one"), Some(2)),
        (Some("p-one"), Some(4)),
        (Some("p-two"), None),
        (None, Some(60)),
    ];
    let asked = Utc::now() - Duration::minutes(10);
    for (person, reply_secs) in replies {
        let mut input = session_input(owner, "agent-audience");
        input.agent_id = Some(agent.id);
        input.channel_id = Some(channel.channel_id);
        let session = backend.create_session(input).await.expect("create session");
        event(
            &backend,
            session.id,
            "input.message",
            asked,
            person.and_then(visitor),
        )
        .await;
        if let Some(secs) = reply_secs {
            event(
                &backend,
                session.id,
                "output.message.completed",
                asked + Duration::seconds(secs),
                None,
            )
            .await;
        }
    }

    let now = Utc::now();
    let rows = backend
        .agent_activity(DEFAULT_ORG_ID, now)
        .await
        .expect("load activity");
    let overview = build_overview(rows, now, 1000);
    let mine = overview
        .channels
        .iter()
        .find(|activity| activity.channel_id == channel.channel_public_id)
        .expect("channel appears in activity");

    assert_eq!(mine.sessions, 4);
    assert_eq!(mine.people, 2, "p-one is counted once");
    assert_eq!(mine.identified_sessions, 3);
    // Replies in 2s, 4s and 60s: the median is 4s.
    assert_eq!(mine.median_first_reply_ms, Some(4000));
    assert!(overview.totals.people_reached >= 2);
}
