// Schedule triggers that run in a Slack conversation, against a real database:
// who may target what, the first run opening the DM under the key a message
// from it binds, the next message and run landing in the same session, and a
// channel that can no longer post failing the run instead of falling back.

use std::time::Duration;

use everruns_contracts::typed_id::{SessionId, TriggerId};
use everruns_core::DEFAULT_ORG_ID;
use everruns_core::channel::SessionBinding;
use serde_json::{Value, json};

use crate::channels::slack::host_tests::{Slack, message, session_of, slack_config};
use crate::domains::agent_triggers::record::{ScheduleTriggerConfig, ScheduleTriggerTarget};
use crate::domains::agent_triggers::{CreateAgentTrigger, invoke_agent_trigger};
use crate::domains::common::{Command, CommandError, Ctx};
use crate::storage::{AgentRow, CreateAgentTriggerRow};

async fn agent(slack: &Slack) -> AgentRow {
    slack
        .db
        .get_agent(DEFAULT_ORG_ID, slack.context.agent_id.unwrap())
        .await
        .unwrap()
        .unwrap()
}

fn ctx(slack: &Slack) -> Ctx {
    Ctx::minimal_for_test(
        everruns_core::Caller::internal(DEFAULT_ORG_ID),
        slack.db.clone(),
        None,
    )
    .with_workflow_store(Some(std::sync::Arc::new(
        everruns_durable::PostgresWorkflowEventStore::new(slack.db.database().pool().clone()),
    )))
}

async fn create(slack: &Slack, request: Value) -> Result<ScheduleTriggerConfig, CommandError> {
    let agent = agent(slack).await;
    let mut body = json!({
        "agent_id": agent.public_id,
        "trigger_type": "schedule",
        "cron_expression": "0 7 * * *",
        "message": "Morning briefing",
        "session_mode": "shared_session",
    });
    body.as_object_mut()
        .unwrap()
        .extend(request.as_object().unwrap().clone());
    let command: CreateAgentTrigger = serde_json::from_value(body).unwrap();
    let trigger = command.execute(&ctx(slack)).await?;
    Ok(trigger.schedule_config().unwrap())
}

fn dm_target(slack: &Slack, key: &str) -> Value {
    json!({
        "target_channel_id": slack.channel.public_id.to_string(),
        "target_binding_key": key,
    })
}

#[tokio::test]
async fn only_the_agents_own_channel_in_a_conversation_it_binds_can_be_targeted() {
    let slack = Slack::new(slack_config(SessionBinding::Requester)).await;

    let config = create(&slack, dm_target(&slack, "slack:user:U1"))
        .await
        .unwrap();
    assert_eq!(
        config.target,
        Some(ScheduleTriggerTarget {
            channel_id: slack.channel.public_id,
            binding_key: "slack:user:U1".into(),
        })
    );

    // Not a key this channel binds, and not a key at all.
    for key in ["slack:channel:C1", "slack:thread:1.0", "U1"] {
        let error = create(&slack, dm_target(&slack, key)).await.unwrap_err();
        assert_eq!(error.status().as_u16(), 400, "{key}: {error}");
    }
    // Half a target.
    let error = create(
        &slack,
        json!({ "target_channel_id": slack.channel.public_id.to_string() }),
    )
    .await
    .unwrap_err();
    assert_eq!(error.status().as_u16(), 400);

    // Another agent's channel reads as not found.
    let other = Slack::new(slack_config(SessionBinding::Requester)).await;
    let error = create(&slack, dm_target(&other, "slack:user:U1"))
        .await
        .unwrap_err();
    assert_eq!(error.status().as_u16(), 404);

    // Only schedules run in a conversation.
    let error = create(
        &slack,
        json!({
            "trigger_type": "webhook",
            "token": "t",
            "target_channel_id": slack.channel.public_id.to_string(),
            "target_binding_key": "slack:user:U1",
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(error.status().as_u16(), 400);
}

/// A targeted schedule trigger written straight to storage, as one an older
/// write left behind would be.
async fn targeted_trigger(slack: &Slack, key: &str) -> TriggerId {
    let agent = agent(slack).await;
    let config = ScheduleTriggerConfig {
        cron_expression: "0 0 7 * * * *".into(),
        timezone: "UTC".into(),
        session_mode: SessionBinding::Shared,
        message: "Morning briefing".into(),
        script: None,
        target: Some(ScheduleTriggerTarget {
            channel_id: slack.channel.public_id,
            binding_key: key.into(),
        }),
    };
    let id = TriggerId::new();
    slack
        .db
        .create_agent_trigger(CreateAgentTriggerRow {
            org_id: DEFAULT_ORG_ID,
            id,
            agent_id: agent.id,
            trigger_type: "schedule".into(),
            ingress_id: None,
            config: serde_json::to_value(config).unwrap(),
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
        .unwrap();
    id
}

async fn fire(
    slack: &Slack,
    trigger: TriggerId,
) -> Result<crate::domains::agent_triggers::AgentTriggerInvocationResult, CommandError> {
    let agent = agent(slack).await;
    invoke_agent_trigger(
        &slack.db,
        &slack.state.session_service,
        &slack.state.message_service,
        slack.state.channels.as_deref(),
        None,
        DEFAULT_ORG_ID,
        &agent.public_id,
        &trigger.to_string(),
    )
    .await
}

async fn latest_input(slack: &Slack, session: SessionId) -> crate::storage::EventRow {
    slack
        .db
        .latest_input_message_event_since(session, 0)
        .await
        .unwrap()
        .expect("the run's input was recorded")
}

/// The delivery target the host saved for the session's turn.
async fn saved_target(slack: &Slack, session: SessionId) -> Value {
    for _ in 0..200 {
        let target: Option<Value> = sqlx::query_scalar(
            "SELECT target FROM channel_pending_deliveries WHERE session_id = $1",
        )
        .bind(session.uuid())
        .fetch_optional(slack.db.database().pool())
        .await
        .unwrap();
        if let Some(target) = target {
            return target;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the turn's delivery was never saved");
}

#[tokio::test]
async fn the_first_run_opens_the_dm_and_the_conversation_continues_in_its_session() {
    let slack = Slack::new(slack_config(SessionBinding::Requester)).await;
    let trigger = targeted_trigger(&slack, "slack:user:U1").await;

    let first = fire(&slack, trigger).await.unwrap();
    assert!(first.created_session);
    let session = first.session_id;
    // Bound under the key a message from U1 looks up.
    assert_eq!(
        slack.bound("slack:user:U1").await,
        Some(session.to_string())
    );

    // Nobody wrote yet, so the user id stands in for the DM, top-level.
    let input = latest_input(&slack, session).await;
    let metadata = &input.data["message"]["metadata"];
    assert_eq!(metadata["slack_channel"], "U1");
    assert_eq!(metadata["slack_reply_inline"], true);
    assert_eq!(metadata["agent_trigger_id"], trigger.to_string());
    // send_message's signed-ingress check accepts it.
    let app = slack.context.public_id.to_string();
    let endpoint = slack.channel.public_id.to_string();
    assert_eq!(
        crate::channels::slack::actions::signed_slack_target(&input, &app, &endpoint).unwrap(),
        Some(("U1".to_string(), String::new()))
    );
    let target = saved_target(&slack, session).await;
    assert_eq!(target["channel_id"], "U1");
    assert_eq!(target["thread_ref"], "");

    // Their reply in the DM continues the run's session.
    let mut dm = message("D1", "5.0", None);
    dm["event"]["channel_type"] = json!("im");
    let reply = slack.send(dm).await.unwrap();
    assert_eq!(session_of(&reply), session.to_string());

    // The next run speaks in the DM they wrote from.
    let second = fire(&slack, trigger).await.unwrap();
    assert!(!second.created_session);
    assert_eq!(second.session_id, session);
    let input = latest_input(&slack, session).await;
    assert_eq!(input.data["message"]["metadata"]["slack_channel"], "D1");
    assert_eq!(slack.bindings().await, 1);

    let deliveries = slack
        .db
        .list_agent_trigger_deliveries(DEFAULT_ORG_ID, trigger, 10)
        .await
        .unwrap();
    assert!(
        deliveries
            .iter()
            .all(|delivery| delivery.status == "dispatched"
                && delivery.session_id == Some(session.uuid()))
    );
}

#[tokio::test]
async fn a_channel_that_cannot_post_fails_the_run_without_a_session_of_its_own() {
    let slack = Slack::new(slack_config(SessionBinding::Requester)).await;
    let trigger = targeted_trigger(&slack, "slack:user:U1").await;
    sqlx::query("UPDATE agent_channels SET enabled = false WHERE id = $1")
        .bind(slack.channel.internal_id)
        .execute(slack.db.database().pool())
        .await
        .unwrap();

    let error = fire(&slack, trigger).await.unwrap_err();
    assert_eq!(error.status().as_u16(), 400, "{error}");
    assert_eq!(slack.bindings().await, 0);
    let deliveries = slack
        .db
        .list_agent_trigger_deliveries(DEFAULT_ORG_ID, trigger, 10)
        .await
        .unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].status, "failed");
    assert!(deliveries[0].session_id.is_none());
    let tagged: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE $1 = ANY(tags)")
        .bind(format!("agent_trigger:{trigger}"))
        .fetch_one(slack.db.database().pool())
        .await
        .unwrap();
    assert_eq!(tagged, 0, "no fallback to a session of the trigger's own");

    // A surface without the channel host fails the same way.
    let other = Slack::new(slack_config(SessionBinding::Requester)).await;
    let trigger = targeted_trigger(&other, "slack:user:U1").await;
    let agent = agent(&other).await;
    let error = invoke_agent_trigger(
        &other.db,
        &other.state.session_service,
        &other.state.message_service,
        None,
        None,
        DEFAULT_ORG_ID,
        &agent.public_id,
        &trigger.to_string(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.status().as_u16(), 500);
}
