//! Session permission policy on the session schedule routes (EVE-1179).
//!
//! The legacy `/v1/sessions/{id}/schedules...` routes used to authenticate by
//! org membership only. Reads (`list`, `get`) now require `SESSION_VIEW` and
//! writes (`update`, `delete`, `trigger`) require `SESSION_MANAGE` — both rest
//! on `org:sessions:manage`, so a caller stripped of that permission gets 403
//! on every route before any schedule is looked up.

use crate::test_harness;

use async_trait::async_trait;
use axum::http::StatusCode;
use everruns_core::{Caller, Permission, PermissionResolver};
use everruns_provider::typed_id::{AgentId, HarnessId, MessageId, SessionId};
use everruns_worker::AgentRunner;
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use test_harness::TestServer;
use uuid::Uuid;

/// No-op runner: the policy checks run before any run is touched.
struct PolicyProbeRunner;

#[async_trait]
impl AgentRunner for PolicyProbeRunner {
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
        _resolution_id: Uuid,
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

/// Withholds `org:sessions:manage`, but only once armed. Fixture setup
/// (creating the agent and session) runs unarmed because session creation
/// needs the same permission.
#[derive(Default)]
struct DenySessionManagement {
    armed: AtomicBool,
}

impl PermissionResolver for DenySessionManagement {
    fn has_permission(&self, _caller: &Caller, permission: &Permission) -> bool {
        if *permission == Permission::OrgSessionsManage {
            !self.armed.load(Ordering::SeqCst)
        } else {
            true
        }
    }

    fn caller_permissions(&self, caller: &Caller) -> Vec<Permission> {
        Permission::ALL
            .iter()
            .copied()
            .filter(|permission| self.has_permission(caller, permission))
            .collect()
    }
}

async fn fixture() -> (
    TestServer,
    Arc<DenySessionManagement>,
    everruns_platform::Session,
) {
    let policy = Arc::new(DenySessionManagement::default());
    let server = TestServer::in_memory_with_runner_and_permission_resolver(
        Arc::new(PolicyProbeRunner),
        policy.clone(),
    )
    .await;
    let agent = server
        .post(
            "/v1/agents",
            &json!({ "name": "policy-probe", "system_prompt": "probe" }),
        )
        .await
        .json::<everruns_platform::Agent>();
    let session = server
        .post("/v1/sessions", &json!({ "agent_id": agent.public_id }))
        .await
        .assert_status(StatusCode::CREATED)
        .json::<everruns_platform::Session>();
    (server, policy, session)
}

#[tokio::test]
async fn list_schedules_requires_session_view() {
    let (server, policy, session) = fixture().await;
    policy.armed.store(true, Ordering::SeqCst);

    server
        .get(&format!("/v1/sessions/{}/schedules", session.id))
        .await
        .assert_status(StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn get_schedule_requires_session_view() {
    let (server, policy, session) = fixture().await;
    policy.armed.store(true, Ordering::SeqCst);

    server
        .get(&format!(
            "/v1/sessions/{}/schedules/{}",
            session.id, "sched_00000000000000000000000000000000"
        ))
        .await
        .assert_status(StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn update_schedule_requires_session_manage() {
    let (server, policy, session) = fixture().await;
    policy.armed.store(true, Ordering::SeqCst);

    server
        .patch(
            &format!(
                "/v1/sessions/{}/schedules/{}",
                session.id, "sched_00000000000000000000000000000000"
            ),
            &json!({ "enabled": false }),
        )
        .await
        .assert_status(StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn delete_schedule_requires_session_manage() {
    let (server, policy, session) = fixture().await;
    policy.armed.store(true, Ordering::SeqCst);

    server
        .delete(&format!(
            "/v1/sessions/{}/schedules/{}",
            session.id, "sched_00000000000000000000000000000000"
        ))
        .await
        .assert_status(StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn trigger_schedule_requires_session_manage() {
    let (server, policy, session) = fixture().await;
    policy.armed.store(true, Ordering::SeqCst);

    server
        .post(
            &format!(
                "/v1/sessions/{}/schedules/{}/trigger",
                session.id, "sched_00000000000000000000000000000000"
            ),
            &json!({}),
        )
        .await
        .assert_status(StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn schedules_still_reachable_with_session_permission() {
    let (server, _policy, session) = fixture().await;

    // Never armed: the same caller lists successfully, so the 403s above are
    // the policy and not a broken route.
    let schedules = server
        .get(&format!("/v1/sessions/{}/schedules", session.id))
        .await
        .json::<Vec<serde_json::Value>>();
    assert!(schedules.is_empty());
}
