//! HTTP session-storage routes enforce the session permission policies.
//!
//! `Command::run` is the shared gate, and these routes are the HTTP caller.
//! A same-org principal whose resolver withholds session access must not
//! receive plaintext values or secret names.

use crate::test_harness;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use axum::http::StatusCode;
use everruns_contracts::typed_id::SessionId;
use everruns_core::host::TurnBackend;
use everruns_core::{Caller, Permission, PermissionResolver};
use everruns_server::domains::sessions::record::Session;
use everruns_server::storage::{UpsertSessionKeyValue, UpsertSessionSecret};
use serde_json::{Value, json};
use test_harness::TestServer;

const PLAINTEXT: &str = "payroll-plaintext-9f3a";
const USER_KEY: &str = "payroll";
const SECRET_NAME: &str = "NIGHTLY_PAYROLL_TOKEN";
const INTERNAL_KV_KEY: &str = "tool_approval/forged-decision";
const INTERNAL_SECRET: &str = "session_sandbox";

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

/// Withholds `org:sessions:manage` (which backs `SESSION_VIEW` and
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

fn assert_forbidden_without_contents(response: test_harness::TestResponse, route: &str) {
    let response = response.assert_status(StatusCode::FORBIDDEN);
    let body = response.text();
    assert!(
        !body.contains(PLAINTEXT) && !body.contains(SECRET_NAME),
        "{route}: leaked storage contents: {body}"
    );
}

/// THREAT[TM-AUTHZ-023]: HTTP list routes consult the custom resolver, and an
/// authorized caller still sees user entries with internal names filtered out.
#[tokio::test]
async fn session_storage_http_routes_honor_the_session_view_policy() {
    let policy = Arc::new(DenySessionAccess::default());
    let server = TestServer::in_memory_with_runner_and_permission_resolver(
        Arc::new(IdleRunner),
        policy.clone(),
    )
    .await;
    let session: Session = server
        .post(
            "/v1/sessions",
            json!({ "harness_id": server.seed_base_harness_id, "title": "Storage" }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    server
        .db
        .upsert_session_key_value(UpsertSessionKeyValue {
            session_id: session.id,
            key: USER_KEY.to_string(),
            value: PLAINTEXT.to_string(),
        })
        .await
        .expect("seed user key");
    server
        .db
        .upsert_session_key_value(UpsertSessionKeyValue {
            session_id: session.id,
            key: INTERNAL_KV_KEY.to_string(),
            value: "hidden-approval".to_string(),
        })
        .await
        .expect("seed internal key");
    server
        .db
        .upsert_session_secret(UpsertSessionSecret {
            session_id: session.id,
            name: SECRET_NAME.to_string(),
            value_encrypted: b"ciphertext".to_vec(),
        })
        .await
        .expect("seed user secret");
    server
        .db
        .upsert_session_secret(UpsertSessionSecret {
            session_id: session.id,
            name: INTERNAL_SECRET.to_string(),
            value_encrypted: b"sandbox".to_vec(),
        })
        .await
        .expect("seed internal secret");

    let keys_url = format!("/v1/sessions/{}/storage/keys", session.id);
    let secrets_url = format!("/v1/sessions/{}/storage/secrets", session.id);

    let keys: Value = server
        .get(&keys_url)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(keys["data"][0]["key"], USER_KEY);
    assert_eq!(keys["data"][0]["value"], PLAINTEXT);
    assert_eq!(keys["data"].as_array().map(Vec::len), Some(1));

    let secrets: Value = server
        .get(&secrets_url)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(secrets["data"][0]["name"], SECRET_NAME);
    assert_eq!(secrets["data"].as_array().map(Vec::len), Some(1));

    policy.armed.store(true, Ordering::SeqCst);
    assert_forbidden_without_contents(server.get(&keys_url).await, "list keys");
    assert_forbidden_without_contents(server.get(&secrets_url).await, "list secrets");
    assert_forbidden_without_contents(
        server
            .put(
                &secrets_url,
                json!({ "secrets": { "EXTRA_TOKEN": "value" } }),
            )
            .await,
        "batch set",
    );
    assert_forbidden_without_contents(
        server.delete(&format!("{secrets_url}/{SECRET_NAME}")).await,
        "delete secret",
    );
}
