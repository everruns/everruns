//! Fixtures shared by the api integration test modules.

use crate::test_harness;
use axum::http::StatusCode;
use chrono::{Duration, Utc};
use everruns_platform::Agent;
use everruns_platform::Session;
use everruns_provider::typed_id::ScheduleId;
use everruns_server::storage::models::CreateSessionScheduleRow;
use serde_json::{Value, json};
use test_harness::TestServer;

pub(crate) async fn create_schedule_test_session(server: &TestServer, title: &str) -> Session {
    server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "title": title
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json()
}

pub(crate) async fn seed_session_schedule(server: &TestServer, session: &Session) -> ScheduleId {
    let scheduled_at = Utc::now() + Duration::hours(1);
    let row = server
        .db
        .create_session_schedule(CreateSessionScheduleRow {
            org_id: 1,
            session_id: session.id,
            owner_principal_id: session.owner_principal_id,
            resolved_owner_user_id: session.resolved_owner_user_id,
            description: "Mismatch test schedule".to_string(),
            cron_expression: None,
            scheduled_at: Some(scheduled_at),
            timezone: "UTC".to_string(),
            next_trigger_at: Some(scheduled_at),
        })
        .await
        .unwrap();
    row.id
}

// ============================================
// Session Schedule API Tests
// ============================================

pub(crate) async fn create_llmsim_agent(server: &TestServer, name: &str) -> Agent {
    let provider: Value = server
        .post(
            "/v1/providers",
            json!({
                "name": format!("{name}-provider"),
                "provider_type": "llmsim"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let model: Value = server
        .post(
            &format!(
                "/v1/providers/{}/models",
                provider["id"].as_str().expect("provider id")
            ),
            json!({
                "model_id": format!("{name}-model-{}", uuid::Uuid::new_v4()),
                "display_name": format!("{name} model"),
                "enabled": true,
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    server
        .post(
            "/v1/agents",
            json!({
                "name": format!("{name}-agent"),
                "display_name": format!("{name} Agent"),
                "system_prompt": "You are a concise test agent.",
                "default_model_id": model["id"],
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json()
}
