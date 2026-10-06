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
use everruns_contracts::typed_id::{ScheduleId, SessionId};
use everruns_core::host::TurnBackend;
use everruns_core::{Caller, Permission, PermissionResolver};
use everruns_server::records::Session;
use everruns_server::storage::models::{CreateSessionScheduleRow, UpdateSession};
use serde_json::{Value, json};
use test_harness::TestServer;
use uuid::Uuid;

const TEST_ORG_ID: i64 = 1;
const SECRET_DESCRIPTION: &str = "nightly payroll export to finance";

struct IdleRunner;

#[async_trait]
impl TurnBackend for IdleRunner {
    async fn start_turn(
        &self,
        request: everruns_core::host::TurnRequest,
    ) -> everruns_contracts::error::Result<everruns_core::host::TurnTicket> {
        // The server drops its tickets; this one never resolves.
        Ok(everruns_core::host::TurnTicket::new(
            request.session_id,
            request.turn_id,
            std::future::pending(),
        ))
    }

    async fn cancel(&self, _session_id: SessionId) -> everruns_contracts::error::Result<bool> {
        Ok(false)
    }

    async fn is_running(&self, _session_id: SessionId) -> bool {
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

async fn create_platform_chat_session(server: &TestServer) -> Session {
    server
        .post("/v1/sessions", json!({ "agent_name": "platform-chat" }))
        .await
        .assert_status(StatusCode::CREATED)
        .json()
}

/// Hand the session to another user of the same org, so the test caller (the
/// anonymous user) still holds `SESSION_MANAGE` but is no longer the owner.
async fn reassign_owner(server: &TestServer, session: &Session, owner: Uuid) {
    server
        .db
        .update_session(
            TEST_ORG_ID,
            session.id,
            UpdateSession {
                resolved_owner_user_id: everruns_durable::UpdateField::Set(owner),
                ..Default::default()
            },
        )
        .await
        .expect("reassign session owner")
        .expect("session exists");
}

/// Update, trigger, then delete the schedule; each must succeed.
async fn manage_schedule(server: &TestServer, session: &Session, schedule_id: ScheduleId) {
    let one = format!("/v1/sessions/{}/schedules/{}", session.id, schedule_id);
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
}

/// THREAT[TM-AGENT-017]: Platform Chat acts with its persisted owner's
/// authority, so another same-org user holding `SESSION_MANAGE` must not
/// fire, disable, or delete the owner's schedules. Reads stay open, like
/// session reads.
#[tokio::test]
async fn a_non_owner_cannot_change_platform_chat_schedules() {
    let server = TestServer::in_memory_with_runner(Arc::new(IdleRunner)).await;
    let session = create_platform_chat_session(&server).await;
    let schedule_id = seed_schedule(&server, &session).await;
    reassign_owner(&server, &session, server.create_user("other-owner").await).await;

    let one = format!("/v1/sessions/{}/schedules/{}", session.id, schedule_id);
    server
        .patch(&one, json!({ "enabled": false }))
        .await
        .assert_status(StatusCode::FORBIDDEN);
    server
        .post(&format!("{one}/trigger"), json!({}))
        .await
        .assert_status(StatusCode::FORBIDDEN);
    server
        .delete(&one)
        .await
        .assert_status(StatusCode::FORBIDDEN);

    let persisted = server
        .db
        .get_session_schedule(TEST_ORG_ID, schedule_id)
        .await
        .expect("read schedule")
        .expect("schedule survives a refused delete");
    assert!(persisted.enabled, "refused update must not disable");
    assert_eq!(persisted.trigger_count, 0, "refused trigger must not fire");
    assert!(persisted.last_triggered_at.is_none());

    server.get(&one).await.assert_status(StatusCode::OK);
    server
        .get(&format!("/v1/sessions/{}/schedules", session.id))
        .await
        .assert_status(StatusCode::OK);
}

#[tokio::test]
async fn the_platform_chat_owner_can_change_its_schedules() {
    let server = TestServer::in_memory_with_runner(Arc::new(IdleRunner)).await;
    let session = create_platform_chat_session(&server).await;
    assert_eq!(
        session.resolved_owner_user_id,
        Some(everruns_server::records::ANONYMOUS_USER_ID)
    );
    let schedule_id = seed_schedule(&server, &session).await;

    manage_schedule(&server, &session, schedule_id).await;
}

/// The owner binding is Platform Chat specific: a regular session owned by
/// another user stays manageable by any caller holding `SESSION_MANAGE`.
#[tokio::test]
async fn other_sessions_schedules_are_not_owner_bound() {
    let server = TestServer::in_memory_with_runner(Arc::new(IdleRunner)).await;
    let session = create_session(&server).await;
    let schedule_id = seed_schedule(&server, &session).await;
    reassign_owner(&server, &session, server.create_user("other-owner").await).await;

    manage_schedule(&server, &session, schedule_id).await;
}
