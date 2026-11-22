use super::*;

use async_trait::async_trait;
use everruns_capabilities::sandbox_state::SandboxStateStore;
use everruns_contracts::session_sandbox::{
    SessionSandboxCredential, SessionSandboxCredentialSource, SessionSandboxInstance,
    SessionSandboxState,
};
use everruns_core::UserConnectionResolver;
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::domains::sandbox_templates::record::SandboxTemplateSpec;
use crate::storage::{
    DbSessionStorageStore, EncryptionService, PgSandboxCheckpointStore, StorageBackend,
};

fn observation(status: SessionSandboxStatus) -> SessionSandboxStatusResponse {
    SessionSandboxStatusResponse {
        provider: "daytona".into(),
        session_status: status,
        external_id: "current-sandbox".into(),
        display_name: None,
        workspace_path: None,
        metadata: json!({}),
    }
}

struct PinnedResolver(SessionId);

#[async_trait]
impl UserConnectionResolver for PinnedResolver {
    async fn get_sandbox_connection_token(
        &self,
        session: SessionId,
        provider: &str,
        credential: &SessionSandboxCredential,
    ) -> everruns_contracts::error::Result<Option<String>> {
        assert_eq!(session, self.0);
        assert_eq!(provider, "daytona");
        assert_eq!(
            credential.source,
            SessionSandboxCredentialSource::SessionUser
        );
        assert_eq!(credential.virtual_user_id, Some(Uuid::nil()));
        Ok(Some("test-pinned-token".into()))
    }

    async fn get_connection_token(
        &self,
        _session: SessionId,
        _provider: &str,
    ) -> everruns_contracts::error::Result<Option<String>> {
        panic!("provider polling must use the pinned credential");
    }
}

async fn fixture() -> (
    Arc<StorageBackend>,
    SessionSandboxService,
    SessionId,
    MockServer,
) {
    let db = Arc::new(StorageBackend::test_database());
    let session = db
        .create_session(crate::storage::test_database::test_session_row(
            everruns_core::DEFAULT_ORG_ID,
        ))
        .await
        .unwrap()
        .id;
    let provider = MockServer::start().await;
    let template: SandboxTemplateSpec = serde_json::from_value(json!({
        "target": {
            "kind": "managed", "provider": "daytona",
            "credential": {"source": "session_user", "virtual_user_id": Uuid::nil()}
        }
    }))
    .unwrap();
    let mut spec = crate::domains::sandbox_templates::resolution::resolve_spec(&template).unwrap();
    // The wire fixture replaces transport after validating the authored
    // template. A caller cannot configure a Daytona management endpoint.
    spec.target.options["api_base"] = json!(provider.uri());
    db.pin_primary_sandbox(session, "build", &spec)
        .await
        .unwrap();
    let at = (Utc::now() - chrono::Duration::days(2)).to_rfc3339();
    PgSandboxCheckpointStore::new(db.pool().clone())
        .save_state(
            session,
            &SessionSandboxState {
                sandbox: None,
                provider: "daytona".into(),
                status: SessionSandboxStatus::Running,
                instance: SessionSandboxInstance {
                    external_id: "current-sandbox".into(),
                    ..Default::default()
                },
                init_completed_at: None,
                last_init_error: None,
                created_at: at.clone(),
                updated_at: at,
            },
            None,
        )
        .await
        .unwrap();
    let storage = Arc::new(DbSessionStorageStore::new(
        db.database().clone(),
        EncryptionService::new("kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", &[]).unwrap(),
    ));
    let service =
        SessionSandboxService::new(db.clone(), storage, Some(Arc::new(PinnedResolver(session))));
    (db, service, session, provider)
}

#[tokio::test]
async fn automatic_status_sync_updates_fleet_without_changing_runtime_or_activity() {
    let (db, service, session, provider) = fixture().await;
    Mock::given(method("GET"))
        .and(path("/sandbox/current-sandbox"))
        .and(header("authorization", "Bearer test-pinned-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "current-sandbox", "state": "stopped"
        })))
        .expect(1)
        .mount(&provider)
        .await;
    let before = db.get_primary_sandbox(session).await.unwrap().unwrap();
    let store = PgSandboxCheckpointStore::new(db.pool().clone());
    let runtime = store.load_current_state(session).await.unwrap();
    let (first, overlapping) = tokio::join!(sync(db.pool(), &service), sync(db.pool(), &service));
    first.unwrap();
    overlapping.unwrap();
    let after = db.get_primary_sandbox(session).await.unwrap().unwrap();
    assert_eq!(after.observed_state, before.observed_state);
    assert_eq!(after.desired_state, before.desired_state);
    assert_eq!(after.last_activity_at, before.last_activity_at);
    assert_eq!(after.generation, before.generation);
    assert_eq!(store.load_current_state(session).await.unwrap(), runtime);
    let row = db
        .get_sandbox_fleet_row(everruns_core::DEFAULT_ORG_ID, before.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.fleet_state, "paused");
    assert!(!row.attention.contains(&"idle_running".into()));
    // The schedule can run again a minute later without another provider call.
    sync(db.pool(), &service).await.unwrap();
    assert!(candidates(db.pool(), 20).await.unwrap().is_empty());
}

#[tokio::test]
async fn automatic_status_sync_retains_state_on_errors_and_retries_after_cooldown() {
    let (db, service, session, provider) = fixture().await;
    Mock::given(method("GET"))
        .and(path("/sandbox/current-sandbox"))
        .respond_with(ResponseTemplate::new(403))
        .expect(1)
        .mount(&provider)
        .await;
    sync(db.pool(), &service).await.unwrap();
    let pinned = db.get_primary_sandbox(session).await.unwrap().unwrap();
    assert_eq!(pinned.observed_state, "ready");
    assert!(candidates(db.pool(), 20).await.unwrap().is_empty());
    provider.reset().await;
    sqlx::query("UPDATE sandboxes SET provider_status_checked_at = now() - interval '6 minutes'")
        .execute(db.pool())
        .await
        .unwrap();
    Mock::given(method("GET"))
        .and(path("/sandbox/current-sandbox"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&provider)
        .await;
    sync(db.pool(), &service).await.unwrap();
    assert_eq!(
        db.get_sandbox_fleet_row(everruns_core::DEFAULT_ORG_ID, pinned.id)
            .await
            .unwrap()
            .unwrap()
            .fleet_state,
        "lost"
    );
}

#[tokio::test]
async fn automatic_status_sync_rejects_stale_generations_activity_and_other_tenants() {
    let (db, _, session, _) = fixture().await;
    let candidate = candidates(db.pool(), 20).await.unwrap().remove(0);
    let mut wrong_org = candidates(db.pool(), 20).await.unwrap().remove(0);
    wrong_org.org_id += 1;
    assert!(
        !record_observation(
            db.pool(),
            &wrong_org,
            Some(observation(SessionSandboxStatus::Lost))
        )
        .await
        .unwrap()
    );
    sqlx::query("UPDATE sandboxes SET updated_at = now() WHERE id = $1")
        .bind(candidate.id)
        .execute(db.pool())
        .await
        .unwrap();
    assert!(
        !record_observation(
            db.pool(),
            &candidate,
            Some(observation(SessionSandboxStatus::Paused))
        )
        .await
        .unwrap()
    );
    let candidate = candidates(db.pool(), 20).await.unwrap().remove(0);
    sqlx::query("UPDATE sandboxes SET generation = generation + 1 WHERE id = $1")
        .bind(candidate.id)
        .execute(db.pool())
        .await
        .unwrap();
    assert!(
        !record_observation(
            db.pool(),
            &candidate,
            Some(observation(SessionSandboxStatus::Lost))
        )
        .await
        .unwrap()
    );
    let row = db.get_primary_sandbox(session).await.unwrap().unwrap();
    assert_eq!(row.observed_state, "ready");
}

#[tokio::test]
async fn automatic_status_sync_yields_to_new_runtime_and_shard_activity() {
    let (db, _, session, _) = fixture().await;
    let candidate = candidates(db.pool(), 20).await.unwrap().remove(0);
    let mut current = observation(SessionSandboxStatus::Paused);
    current.external_id = "shard-current".into();
    record_observation(db.pool(), &candidate, Some(current))
        .await
        .unwrap();
    let fleet_state = || async {
        db.get_sandbox_fleet_row(everruns_core::DEFAULT_ORG_ID, candidate.id)
            .await
            .unwrap()
            .unwrap()
            .fleet_state
    };
    assert_eq!(fleet_state().await, "paused");
    assert_eq!(
        db.get_sandbox_fleet_row(everruns_core::DEFAULT_ORG_ID, candidate.id)
            .await
            .unwrap()
            .unwrap()
            .external_id
            .as_deref(),
        Some("shard-current")
    );
    // A shard can resume independently of the control-plane runtime row.
    let at = Utc::now();
    db.dp_upsert_session_summary(crate::storage::DpSessionSummary {
        session_id: &session.to_string(),
        org_id: everruns_core::DEFAULT_ORG_ID,
        data_plane_id: "test-shard",
        agent_id: "test-agent",
        session_version: 1,
        title: "",
        status: "active",
        turns: 1,
        cost_micros: 0,
        created_at: at,
        last_activity_at: at,
    })
    .await
    .unwrap();
    assert_eq!(fleet_state().await, "running");
    // A later successful provider check can observe that resumed instance.
    record_observation(
        db.pool(),
        &candidate,
        Some(observation(SessionSandboxStatus::Paused)),
    )
    .await
    .unwrap();
    assert_eq!(fleet_state().await, "paused");
    sqlx::query("UPDATE sandboxes SET updated_at = now() WHERE id = $1")
        .bind(candidate.id)
        .execute(db.pool())
        .await
        .unwrap();
    assert_eq!(fleet_state().await, "running");
}
