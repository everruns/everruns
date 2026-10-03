//! Session schedule routes enforce the session permission policies.
//!
//! Schedules hang off a session, so reading one takes `SESSION_VIEW` and
//! changing or firing one takes `SESSION_MANAGE`. A same-org caller whose
//! resolver withholds those permissions must see neither the schedule contents
//! nor be able to change them.

use crate::test_harness;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use axum::http::StatusCode;
use chrono::{Duration, Utc};
use everruns_core::{Caller, Permission, PermissionResolver};
use everruns_platform::Session;
use everruns_contracts::typed_id::{AgentId, HarnessId, MessageId, ScheduleId, SessionId};
use everruns_server::storage::models::CreateSessionScheduleRow;
use everruns_worker::AgentRunner;
use serde_json::{Value, json};
use test_harness::TestServer;

const TEST_ORG_ID: i64 = 1;
const SECRET_DESCRIPTION: &str = "nightly payroll export to finance";

struct IdleRunner;

#[async_trait]
impl AgentRunner for IdleRunner {
    async fn start_run(
        &self,
        _org_id: i64,
        _session_id: SessionId,
        _harness_id: HarnessId,
        _agent_id: Option<AgentId>,
        _input_message_id: MessageId,
        _request_id: Option<String>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn resume_after_tool_results(
        &self,
        _session_id: SessionId,
        _resolution_id: uuid::Uuid,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn cancel_run(&self, _run_id: SessionId) -> anyhow::Result<()> {
        Ok(())
    }

    async fn is_running(&self, _run_id: SessionId) -> bool {
        false
    }

    async fn active_count(&self) -> usize {
        0
    }
}

/// Withholds `org:sessions:manage` (which backs both `SESSION_VIEW` and
/// `SESSION_MANAGE`) once armed, after the fixture session is created.
#[derive(Default)]
struct DenySessionAccess {
    armed: AtomicBool,
}

impl PermissionResolver for DenySessionAccess {
    fn has_permission(&self, _caller: &Caller, permission: &Permission) -> bool {
        !(self.armed.load(Ordering::SeqCst) && permission == &Permission::OrgSessionsManage)
    }

    fn caller_permissions(&self, caller: &Caller) -> Vec<Permission> {
        Permission::ALL
            .iter()
            .copied()
            .filter(|permission| self.has_permission(caller, permission))
            .collect()
    }
}

async fn create_session(server: &TestServer) -> Session {
    server
        .post(
            "/v1/sessions",
            json!({ "harness_id": server.seed_base_harness_id, "title": "Scheduled" }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json()
}

async fn seed_schedule(server: &TestServer, session: &Session) -> ScheduleId {
    let scheduled_at = Utc::now() + Duration::hours(1);
    server
        .db
        .create_session_schedule(CreateSessionScheduleRow {
            org_id: TEST_ORG_ID,
            session_id: session.id,
            owner_principal_id: session.owner_principal_id,
            resolved_owner_user_id: session.resolved_owner_user_id,
            description: SECRET_DESCRIPTION.to_string(),
            cron_expression: None,
            scheduled_at: Some(scheduled_at),
            timezone: "UTC".to_string(),
            next_trigger_at: Some(scheduled_at),
        })
        .await
        .expect("seed schedule")
        .id
}

async fn fixture(policy: Arc<DenySessionAccess>) -> (TestServer, Session, ScheduleId) {
    let server =
        TestServer::in_memory_with_runner_and_permission_resolver(Arc::new(IdleRunner), policy)
            .await;
    let session = create_session(&server).await;
    let schedule_id = seed_schedule(&server, &session).await;
    (server, session, schedule_id)
}

fn assert_forbidden_without_contents(response: test_harness::TestResponse, route: &str) {
    let response = response.assert_status(StatusCode::FORBIDDEN);
    let body = response.text();
    assert!(
        !body.contains(SECRET_DESCRIPTION),
        "{route}: leaked schedule contents: {body}"
    );
}

/// THREAT[TM-SCHED-007]: a same-org caller the resolver denies session access
/// cannot read schedules.
#[tokio::test]
async fn a_caller_without_session_view_cannot_read_schedules() {
    let policy = Arc::new(DenySessionAccess::default());
    let (server, session, schedule_id) = fixture(policy.clone()).await;
    policy.armed.store(true, Ordering::SeqCst);

    let list = format!("/v1/sessions/{}/schedules", session.id);
    assert_forbidden_without_contents(server.get(&list).await, "list");
    let one = format!("/v1/sessions/{}/schedules/{}", session.id, schedule_id);
    assert_forbidden_without_contents(server.get(&one).await, "get");
}

/// THREAT[TM-SCHED-007]: a same-org caller the resolver denies session
/// management cannot update, delete, or trigger schedules.
#[tokio::test]
async fn a_caller_without_session_manage_cannot_change_schedules() {
    let policy = Arc::new(DenySessionAccess::default());
    let (server, session, schedule_id) = fixture(policy.clone()).await;
    policy.armed.store(true, Ordering::SeqCst);

    let one = format!("/v1/sessions/{}/schedules/{}", session.id, schedule_id);
    assert_forbidden_without_contents(
        server.patch(&one, json!({ "enabled": false })).await,
        "update",
    );
    assert_forbidden_without_contents(server.delete(&one).await, "delete");
    assert_forbidden_without_contents(
        server.post(&format!("{one}/trigger"), json!({})).await,
        "trigger",
    );

    let persisted = server
        .db
        .get_session_schedule(TEST_ORG_ID, schedule_id)
        .await
        .expect("read schedule")
        .expect("schedule survives a refused delete");
    assert!(persisted.enabled, "refused update must not disable");
    assert_eq!(persisted.trigger_count, 0, "refused trigger must not fire");
    assert!(persisted.last_triggered_at.is_none());
}

/// The normal authorized workflow still works end to end.
#[tokio::test]
async fn an_authorized_caller_can_manage_schedules() {
    let (server, session, schedule_id) = fixture(Arc::new(DenySessionAccess::default())).await;
    let list = format!("/v1/sessions/{}/schedules", session.id);
    let one = format!("/v1/sessions/{}/schedules/{}", session.id, schedule_id);

    let listed: Value = server.get(&list).await.assert_status(StatusCode::OK).json();
    assert_eq!(listed[0]["description"], SECRET_DESCRIPTION);
    let fetched: Value = server.get(&one).await.assert_status(StatusCode::OK).json();
    assert_eq!(fetched["description"], SECRET_DESCRIPTION);

    let updated: Value = server
        .patch(&one, json!({ "enabled": false }))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(updated["enabled"], false);

    server
        .patch(&one, json!({ "enabled": true }))
        .await
        .assert_status(StatusCode::OK);
    let triggered: Value = server
        .post(&format!("{one}/trigger"), json!({}))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(triggered["trigger_count"], 1);

    server
        .delete(&one)
        .await
        .assert_status(StatusCode::NO_CONTENT);
    assert!(
        server
            .db
            .get_session_schedule(TEST_ORG_ID, schedule_id)
            .await
            .expect("read schedule")
            .is_none()
    );
}
