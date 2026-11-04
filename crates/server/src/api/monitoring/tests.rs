use super::*;
use crate::auth::{
    backend::AuthBackend,
    config::{AuthConfig, AuthMode},
    middleware::{AuthError, AuthMethod, AuthUser},
    routes::AuthConfigResponse,
};
use crate::domains::organizations::record::OrgMembership;
use crate::records::{FeatureFlagGrade, FeatureFlagPolicy};
use crate::storage::test_database::test_session_row;
use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use everruns_core::{DeploymentGrade, OrgRole};
use tower::ServiceExt;

struct Backend {
    platform: bool,
}
#[async_trait]
impl AuthBackend for Backend {
    async fn validate_token(&self, _: &str) -> Result<AuthUser, AuthError> {
        let mut user = AuthUser::anonymous();
        user.is_platform_user = self.platform;
        user.auth_method = AuthMethod::Jwt;
        user.organizations = vec![OrgMembership {
            org_id: 2,
            public_id: everruns_core::org_public_id_from_internal(2),
            name: "Org A".into(),
            role: OrgRole::Owner,
        }];
        Ok(user)
    }
    async fn validate_personal_access_token(&self, _: &str) -> Result<AuthUser, AuthError> {
        Err(AuthError::unauthorized("Not supported"))
    }
    fn auth_routes(&self) -> Option<Router> {
        None
    }
    fn auth_config_response(&self) -> AuthConfigResponse {
        AuthConfigResponse {
            mode: "full".into(),
            login_origin: None,
            password_auth_enabled: false,
            signup_enabled: false,
            oauth_providers: vec![],
            signup_email_confirm: false,
            captcha: None,
        }
    }
}
async fn state(db: Arc<StorageBackend>, platform: bool) -> AppState {
    db.create_test_user(AuthUser::anonymous().id).await;
    db.ensure_membership(AuthUser::anonymous().id, 2, "owner")
        .await
        .unwrap();
    let mut auth = AuthState::new(
        AuthConfig {
            mode: AuthMode::Full,
            ..Default::default()
        },
        Arc::new(Backend { platform }),
    );
    auth.db = Some(db.clone());
    auth.feature_flag_policy = FeatureFlagPolicy::from_env(DeploymentGrade::Prod)
        .with_grade("monitoring", FeatureFlagGrade::Adoption);
    AppState::new(db, auth)
}

struct DenyReads;
impl everruns_core::PermissionResolver for DenyReads {
    fn has_permission(&self, _: &Caller, _: &everruns_core::Permission) -> bool {
        false
    }
    fn caller_permissions(&self, _: &Caller) -> Vec<everruns_core::Permission> {
        vec![]
    }
}

#[tokio::test]
async fn configured_session_read_denial_applies_to_all_org_endpoints() {
    let db = Arc::new(StorageBackend::test_database());
    db.replace_org_feature_flags(2, &HashMap::from([("monitoring".into(), true)]))
        .await
        .unwrap();
    let mut state = state(db, false).await;
    state.auth.permission_resolver = Arc::new(DenyReads);
    let app = routes(state);
    for uri in [
        "/v1/monitoring",
        "/v1/monitoring/actors",
        "/v1/monitoring/actors/session_00000000000000000000000000000001",
        "/v1/monitoring/timeline",
        "/v1/monitoring/timeline/window?from=2026-11-01T00:00:00Z&to=2026-11-01T01:00:00Z",
    ] {
        assert_eq!(get(app.clone(), uri).await.0, StatusCode::FORBIDDEN);
    }
    let (_, config) = get(app, "/v1/monitoring/config").await;
    assert_eq!(config["policies"]["monitoring.view"], false);
}
async fn get(app: Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let response = app
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("Authorization", "Bearer test-token")
                .header("X-Org-Id", everruns_core::org_public_id_from_internal(2))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn tenant_admin_cannot_access_any_fleet_route() {
    let db = Arc::new(StorageBackend::test_database());
    let app = routes(state(db, false).await);
    for uri in [
        "/v1/durable/monitoring",
        "/v1/durable/monitoring/actors",
        "/v1/durable/monitoring/actors/session_00000000000000000000000000000001",
        "/v1/durable/monitoring/timeline",
        "/v1/durable/monitoring/timeline/window?from=2026-11-01T00:00:00Z&to=2026-11-01T01:00:00Z",
    ] {
        assert_eq!(get(app.clone(), uri).await.0, StatusCode::FORBIDDEN);
    }
}

#[tokio::test]
async fn adoption_is_enforced_on_every_read_even_after_a_snapshot_is_cached() {
    let db = Arc::new(StorageBackend::test_database());
    let app = routes(state(db.clone(), false).await);
    assert_eq!(
        get(app.clone(), "/v1/monitoring").await.0,
        StatusCode::NOT_FOUND
    );
    db.replace_org_feature_flags(2, &HashMap::from([("monitoring".into(), true)]))
        .await
        .unwrap();
    assert_eq!(get(app.clone(), "/v1/monitoring").await.0, StatusCode::OK);
    db.replace_org_feature_flags(2, &HashMap::from([("monitoring".into(), false)]))
        .await
        .unwrap();
    for uri in [
        "/v1/monitoring",
        "/v1/monitoring/actors",
        "/v1/monitoring/actors/session_00000000000000000000000000000001",
        "/v1/monitoring/timeline",
    ] {
        assert_eq!(get(app.clone(), uri).await.0, StatusCode::NOT_FOUND);
    }
    assert_eq!(
        get(routes(state(db, true).await), "/v1/durable/monitoring")
            .await
            .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn org_scope_hides_foreign_actors_and_global_worker_capacity() {
    let db = Arc::new(StorageBackend::test_database());
    db.replace_org_feature_flags(2, &HashMap::from([("monitoring".into(), true)]))
        .await
        .unwrap();
    let a = db.create_session(test_session_row(2)).await.unwrap().id;
    let b = db.create_session(test_session_row(3)).await.unwrap().id;
    db.mark_session_actor_runnable(a, 2, 1).await.unwrap();
    db.mark_session_actor_runnable(b, 3, 1).await.unwrap();
    // The owning session is authoritative even if an actor row carries stale scope.
    sqlx::query("UPDATE session_actors SET org_id = 2 WHERE session_id = $1")
        .bind(b)
        .execute(db.pool())
        .await
        .unwrap();
    db.lease_session_actors("shared-worker", 10, Duration::from_secs(10), 32)
        .await
        .unwrap();
    let app = routes(state(db.clone(), false).await);
    let (status, body) = get(app.clone(), "/v1/monitoring?org_id=3").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["actors"], 1);
    assert_eq!(body["lease_events"]["acquired"], 1);
    assert_eq!(body["workers"][0]["actor_leases"], 1);
    assert!(body["workers"][0]["capacity"].is_null());
    assert!(body["workers"][0]["last_heartbeat_at"].is_null());
    assert_eq!(
        get(app, &format!("/v1/monitoring/actors/{b}")).await.0,
        StatusCode::NOT_FOUND
    );
    let (_, fleet) = get(routes(state(db, true).await), "/v1/durable/monitoring").await;
    assert_eq!(fleet["actors"], 2);
}

#[tokio::test]
async fn timeline_returns_zero_filled_buckets_scoped_to_the_org() {
    let db = Arc::new(StorageBackend::test_database());
    db.replace_org_feature_flags(2, &HashMap::from([("monitoring".into(), true)]))
        .await
        .unwrap();
    let a = db.create_session(test_session_row(2)).await.unwrap().id;
    let b = db.create_session(test_session_row(3)).await.unwrap().id;
    db.mark_session_actor_runnable(a, 2, 1).await.unwrap();
    db.mark_session_actor_runnable(b, 3, 1).await.unwrap();
    crate::background::monitoring_timeline::sample(&db, Utc::now())
        .await
        .unwrap();
    let app = routes(state(db.clone(), false).await);
    let (status, body) = get(app.clone(), "/v1/monitoring/timeline?range=1h").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["bucket_seconds"], 60);
    let buckets = body["buckets"].as_array().unwrap();
    assert_eq!(buckets.len(), 60);
    // Only this org's session is counted, and fleet worker data is absent.
    assert_eq!(body["summary"]["peak_active"], 1);
    assert!(buckets.iter().all(|b| b["workers"].is_null()));
    assert_eq!(buckets.iter().filter(|b| b["sampled"] == true).count(), 1);

    let (_, fleet) = get(
        routes(state(db, true).await),
        "/v1/durable/monitoring/timeline?range=1h",
    )
    .await;
    assert_eq!(fleet["summary"]["peak_active"], 2);

    for bad in [
        "/v1/monitoring/timeline?range=2y",
        "/v1/monitoring/timeline?bucket=7",
        "/v1/monitoring/timeline?from=2026-01-01T00:00:00Z&to=2026-03-01T00:00:00Z",
        "/v1/monitoring/timeline?range=30d&bucket=60",
        "/v1/monitoring/timeline/window?from=2026-11-01T00:00:00Z&to=2026-10-01T00:00:00Z",
    ] {
        assert_eq!(
            get(app.clone(), bad).await.0,
            StatusCode::BAD_REQUEST,
            "{bad}"
        );
    }
}
