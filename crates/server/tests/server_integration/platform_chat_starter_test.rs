//! Platform Chat starter uniqueness and archived identity.

use crate::test_harness::{self, TestServer};

use axum::http::StatusCode;
use everruns_contracts::typed_id::PrincipalId;
use everruns_server::setup::org_init;
use everruns_server::storage::{CreatePrincipalRow, CreateSessionRow, Database, StorageBackend};
use serde_json::json;
use sqlx::PgPool;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

const TEST_ORG_ID: i64 = 1;
const STARTER_UNIQUE_INDEX: &str = "idx_sessions_platform_chat_starter_owner";

async fn create_test_pool() -> PgPool {
    PgPool::connect(&test_harness::get_database_url())
        .await
        .expect("connect to test database")
}

async fn create_test_backend() -> StorageBackend {
    StorageBackend::from_database(Database::new(create_test_pool().await))
}

async fn create_test_principal(backend: &StorageBackend, org_id: i64) -> PrincipalId {
    backend
        .create_principal(CreatePrincipalRow {
            id: PrincipalId::new(),
            org_id,
            kind: "system".to_string(),
            subject_id: Some(Uuid::now_v7()),
            parent_principal_id: None,
            resolved_user_id: None,
            metadata: json!({"source": "platform_chat_starter_test"}),
        })
        .await
        .expect("create principal")
        .id
}

#[tokio::test]
async fn platform_chat_starter_is_unique_per_owner_even_after_archive() {
    let backend = create_test_backend().await;
    let owner_principal_id = create_test_principal(&backend, TEST_ORG_ID).await;
    org_init::initialize_org_harnesses(&backend, TEST_ORG_ID)
        .await
        .expect("initialize built-in harnesses");
    let platform_chat = backend
        .get_harness_by_name(TEST_ORG_ID, "generic")
        .await
        .expect("load Platform Chat harness")
        .expect("Platform Chat harness is seeded");
    let agent = backend
        .get_agent_by_name(TEST_ORG_ID, "platform-chat")
        .await
        .unwrap()
        .unwrap();
    let starter = CreateSessionRow {
        playground_user_id: None,

        source: everruns_server::domains::sessions::record::SessionSource::Chat,
        workspace_id: None,
        org_id: TEST_ORG_ID,
        app_id: None,
        channel_id: None,
        trigger_id: None,
        harness_id: Some(platform_chat.id),
        agent_id: Some(agent.id),
        agent_revision: None,
        virtual_user_id: None,
        owner_principal_id,
        resolved_owner_user_id: None,
        title: Some("Platform Chat".to_string()),
        locale: None,
        tags: vec!["chat".to_string()],
        model_id: None,
        capabilities: json!([]),
        tools: json!([]),
        mcp_servers: json!({}),
        system_prompt: None,
        initial_files: json!([]),
        hints: None,
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        blueprint_id: None,
        blueprint_config: None,
        parent_session_id: None,
        budget_root_session_id: None,
    };

    let first = backend
        .create_session(starter.clone())
        .await
        .expect("starter");
    assert!(first.tags.contains(&"platform-chat-starter".to_string()));
    sqlx::query("UPDATE sessions SET archived_at = NOW() WHERE id = $1")
        .bind(first.id.uuid())
        .execute(&create_test_pool().await)
        .await
        .expect("archive starter");
    sqlx::query("UPDATE sessions SET tags = ARRAY['chat']::text[] WHERE id = $1")
        .bind(first.id.uuid())
        .execute(&create_test_pool().await)
        .await
        .expect("update starter tags");
    let kept_marker: bool = sqlx::query_scalar(
        "SELECT 'platform-chat-starter' = ANY(tags) FROM sessions WHERE id = $1",
    )
    .bind(first.id.uuid())
    .fetch_one(&create_test_pool().await)
    .await
    .expect("read starter tags");
    assert!(
        kept_marker,
        "session updates cannot remove starter identity"
    );

    let duplicate = backend
        .create_session(starter.clone())
        .await
        .expect_err("archiving must not permit a second starter");
    // CreateSession recognizes this expected race by constraint name.
    assert!(
        format!("{duplicate:?}").contains(STARTER_UNIQUE_INDEX),
        "{duplicate:?}"
    );

    let mut ordinary = starter;
    ordinary.title = None;
    backend
        .create_session(ordinary)
        .await
        .expect("ordinary platform chat threads remain allowed");
}

/// Collects formatted tracing output for assertions.
#[derive(Clone, Default)]
struct LogSink(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for LogSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// EVERRUNS-24: losing the starter race is expected (the UI adopts the existing
// thread on 409), so it must keep its 409 response without a WARN in Sentry.
#[tokio::test]
async fn duplicate_platform_chat_starter_is_409_without_warning() {
    let server = TestServer::new().await;
    let request = json!({
        "source": "chat",
        "harness_name": "platform-chat",
        "title": "Platform Chat",
        "tags": ["chat", "platform-chat-starter"],
    });
    // The test caller's starter may already exist from an earlier test.
    let first = server.post("/v1/sessions", request.clone()).await;
    assert!(
        matches!(first.status(), StatusCode::CREATED | StatusCode::CONFLICT),
        "unexpected first create: {} {}",
        first.status(),
        first.text()
    );

    let sink = LogSink::default();
    let writer = sink.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    // Thread-local: the in-process router runs on this current-thread runtime.
    let second = {
        let _guard = tracing::subscriber::set_default(subscriber);
        server.post("/v1/sessions", request).await
    };

    let second = second.assert_status(StatusCode::CONFLICT);
    let body = second.json_value();
    assert_eq!(body["code"], "already_exists", "{body}");
    assert_eq!(body["detail"], "Resource already exists", "{body}");
    let logged = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    assert!(
        !logged.contains("database uniqueness conflict"),
        "expected starter conflict to stay below WARN, got: {logged}"
    );
}

/// Exclude the permanent conversation before counting and paginating history.
#[tokio::test]
async fn platform_chat_history_pages_only_side_conversations() {
    let server = TestServer::new().await;
    server
        .post("/v1/sessions/platform-chat", json!({}))
        .await
        .assert_status(StatusCode::OK);
    server
        .post(
            "/v1/sessions",
            json!({"source":"chat", "agent_name":"platform-chat", "tags":["chat"]}),
        )
        .await
        .assert_status(StatusCode::CREATED);
    let query = format!(
        "/v1/sessions?source=chat&agent_id={}&mine=true&limit=1",
        server.seed_chat_agent_id
    );
    let all = server
        .get(&query)
        .await
        .assert_status(StatusCode::OK)
        .json_value();
    let sides = server
        .get(&format!("{query}&side_chats_only=true"))
        .await
        .assert_status(StatusCode::OK)
        .json_value();
    assert_eq!(
        all["total"].as_u64().unwrap(),
        sides["total"].as_u64().unwrap() + 1
    );
    assert_eq!(sides["data"].as_array().unwrap().len(), 1);
    assert!(
        !sides["data"][0]["tags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t == "platform-chat-starter")
    );
}
