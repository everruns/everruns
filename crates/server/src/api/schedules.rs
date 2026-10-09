// Durable Scheduled Tasks API routes
//
// Decision: All endpoints require explicit platform-user auth.
//   Matches the durable control-plane contract for global/system surfaces.
//
// Provides CRUD operations and management for scheduled tasks:
// - Schedule creation, listing, updates, and deletion
// - Pause/resume functionality
// - Manual triggering
// - Execution history

pub use crate::domains::schedules::types::{
    CreateScheduleRequest, ListExecutionsQuery, ListSchedulesQuery, ScheduleExecutionResponse,
    ScheduleExecutionsListResponse, ScheduleResponse, ScheduleStatsResponse, ScheduleTarget,
    ScheduleTargetResponse, SchedulesListResponse, TriggerResponse, UpdateScheduleRequest,
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, patch, post},
};
use everruns_durable::WorkflowEventStore;
use std::sync::Arc;
use uuid::Uuid;

use super::common::{ApiResult, ErrorResponse, impl_auth_state};
use crate::auth::{AuthState, PlatformUser, rate_limit::OrgRateLimiter};
use crate::domains::common::{Command, Ctx};
use crate::domains::schedules::{
    CreateSchedule, DeleteSchedule, GetExecution, GetSchedule, GetScheduleStats,
    ListScheduleExecutions, ListSchedules, PauseSchedule, ResumeSchedule, TriggerSchedule,
    UpdateScheduleCmd,
};
use crate::storage::StorageBackend;
use everruns_core::{Caller, DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, OrgRole};

/// App state for schedule routes
/// All schedule endpoints require platform-user auth.
#[derive(Clone)]
pub struct ScheduleAppState {
    db: Arc<StorageBackend>,
    store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
    auth: AuthState,
    org_rate_limiter: OrgRateLimiter,
}

impl_auth_state!(ScheduleAppState);

impl ScheduleAppState {
    /// Create new state with an optional workflow event store and auth state
    pub fn new(
        db: Arc<StorageBackend>,
        store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
        auth: AuthState,
    ) -> Self {
        Self {
            db,
            store,
            auth,
            org_rate_limiter: OrgRateLimiter::default(),
        }
    }

    pub fn with_org_rate_limiter(mut self, limiter: OrgRateLimiter) -> Self {
        self.org_rate_limiter = limiter;
        self
    }

    /// Get the store, returning an error response if not available
    fn get_store(
        &self,
    ) -> Result<&Arc<dyn WorkflowEventStore + Send + Sync>, (StatusCode, Json<ErrorResponse>)> {
        self.store.as_ref().ok_or_else(|| {
            ErrorResponse::new("Durable execution store not available".to_string())
                .into_response(StatusCode::SERVICE_UNAVAILABLE)
        })
    }

    fn ctx(&self, auth: &PlatformUser) -> Result<Ctx, (StatusCode, Json<ErrorResponse>)> {
        let caller = Caller {
            org_id: DEFAULT_ORG_ID,
            org_public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
            user_id: Some(auth.0.id),
            role: OrgRole::Owner,
            is_platform_user: auth.0.is_platform_user,
            is_internal: false,
        };
        Ok(Ctx::minimal(
            caller,
            self.db.clone(),
            None,
            self.auth.permission_resolver.clone(),
        )
        .with_workflow_store(Some(self.get_store()?.clone())))
    }
}

/// Create schedule routes
pub fn routes(state: ScheduleAppState) -> Router {
    Router::new()
        // CRUD
        .route("/v1/durable/schedules", post(create_schedule))
        .route("/v1/durable/schedules", get(list_schedules))
        .route("/v1/durable/schedules/{schedule_id}", get(get_schedule))
        .route(
            "/v1/durable/schedules/{schedule_id}",
            patch(update_schedule),
        )
        .route(
            "/v1/durable/schedules/{schedule_id}",
            delete(delete_schedule),
        )
        // Actions
        .route(
            "/v1/durable/schedules/{schedule_id}/pause",
            post(pause_schedule),
        )
        .route(
            "/v1/durable/schedules/{schedule_id}/resume",
            post(resume_schedule),
        )
        .route(
            "/v1/durable/schedules/{schedule_id}/trigger",
            post(trigger_schedule),
        )
        // Executions
        .route(
            "/v1/durable/schedules/{schedule_id}/executions",
            get(list_schedule_executions),
        )
        .route("/v1/durable/executions/{execution_id}", get(get_execution))
        // Stats
        .route(
            "/v1/durable/schedules/{schedule_id}/stats",
            get(get_schedule_stats),
        )
        .with_state(state)
}

// ============================================================================
// Route handlers
// ============================================================================

/// POST /v1/durable/schedules - Create a new schedule
#[utoipa::path(
    post,
    path = "/v1/durable/schedules",
    request_body = CreateScheduleRequest,
    responses(
        (status = 201, description = "Schedule created", body = ScheduleResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 401, description = "Authentication required"),
        (status = 409, description = "Schedule name already exists", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn create_schedule(
    auth: PlatformUser,
    State(state): State<ScheduleAppState>,
    Json(req): Json<CreateScheduleRequest>,
) -> Result<(StatusCode, Json<ScheduleResponse>), (StatusCode, Json<ErrorResponse>)> {
    if state
        .org_rate_limiter
        .check_schedule_create(auth.0.id)
        .await
        .is_err()
    {
        return Err(
            ErrorResponse::new("Too many requests. Please try again later.")
                .with_code("rate_limited")
                .with_retry_after(60)
                .into_response(StatusCode::TOO_MANY_REQUESTS),
        );
    }
    let schedule = CreateSchedule(req).run(&state.ctx(&auth)?).await?;
    Ok((StatusCode::CREATED, Json(schedule)))
}

/// GET /v1/durable/schedules - List schedules
#[utoipa::path(
    get,
    path = "/v1/durable/schedules",
    params(
        ("enabled" = Option<bool>, Query, description = "Filter by enabled status"),
        ("target_type" = Option<String>, Query, description = "Filter by target type"),
        ("offset" = Option<u32>, Query, description = "Pagination offset"),
        ("limit" = Option<u32>, Query, description = "Pagination limit")
    ),
    responses(
        (status = 200, description = "List of schedules", body = SchedulesListResponse),
        (status = 401, description = "Authentication required"),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn list_schedules(
    auth: PlatformUser,
    State(state): State<ScheduleAppState>,
    Query(query): Query<ListSchedulesQuery>,
) -> ApiResult<SchedulesListResponse> {
    let schedules = ListSchedules {
        enabled: query.enabled,
        target_type: query.target_type,
        offset: query.offset,
        limit: query.limit,
    }
    .run(&state.ctx(&auth)?)
    .await?;
    Ok(Json(schedules))
}

/// GET /v1/durable/schedules/:schedule_id - Get schedule details
#[utoipa::path(
    get,
    path = "/v1/durable/schedules/{schedule_id}",
    params(
        ("schedule_id" = Uuid, Path, description = "Schedule ID")
    ),
    responses(
        (status = 200, description = "Schedule details", body = ScheduleResponse),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Schedule not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn get_schedule(
    auth: PlatformUser,
    State(state): State<ScheduleAppState>,
    Path(schedule_id): Path<Uuid>,
) -> ApiResult<ScheduleResponse> {
    let schedule = GetSchedule { schedule_id }.run(&state.ctx(&auth)?).await?;
    Ok(Json(schedule))
}

/// PATCH /v1/durable/schedules/:schedule_id - Update schedule
#[utoipa::path(
    patch,
    path = "/v1/durable/schedules/{schedule_id}",
    params(
        ("schedule_id" = Uuid, Path, description = "Schedule ID")
    ),
    request_body = UpdateScheduleRequest,
    responses(
        (status = 200, description = "Schedule updated", body = ScheduleResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Schedule not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn update_schedule(
    auth: PlatformUser,
    State(state): State<ScheduleAppState>,
    Path(schedule_id): Path<Uuid>,
    Json(req): Json<UpdateScheduleRequest>,
) -> ApiResult<ScheduleResponse> {
    let schedule = UpdateScheduleCmd { schedule_id, req }
        .run(&state.ctx(&auth)?)
        .await?;
    Ok(Json(schedule))
}

/// DELETE /v1/durable/schedules/:schedule_id - Delete schedule
#[utoipa::path(
    delete,
    path = "/v1/durable/schedules/{schedule_id}",
    params(
        ("schedule_id" = Uuid, Path, description = "Schedule ID")
    ),
    responses(
        (status = 204, description = "Schedule deleted"),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Schedule not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn delete_schedule(
    auth: PlatformUser,
    State(state): State<ScheduleAppState>,
    Path(schedule_id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    DeleteSchedule { schedule_id }
        .run(&state.ctx(&auth)?)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /v1/durable/schedules/:schedule_id/pause - Pause schedule
#[utoipa::path(
    post,
    path = "/v1/durable/schedules/{schedule_id}/pause",
    params(
        ("schedule_id" = Uuid, Path, description = "Schedule ID")
    ),
    responses(
        (status = 200, description = "Schedule paused", body = ScheduleResponse),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Schedule not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn pause_schedule(
    auth: PlatformUser,
    State(state): State<ScheduleAppState>,
    Path(schedule_id): Path<Uuid>,
) -> ApiResult<ScheduleResponse> {
    let schedule = PauseSchedule { schedule_id }
        .run(&state.ctx(&auth)?)
        .await?;
    Ok(Json(schedule))
}

/// POST /v1/durable/schedules/:schedule_id/resume - Resume schedule
#[utoipa::path(
    post,
    path = "/v1/durable/schedules/{schedule_id}/resume",
    params(
        ("schedule_id" = Uuid, Path, description = "Schedule ID")
    ),
    responses(
        (status = 200, description = "Schedule resumed", body = ScheduleResponse),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Schedule not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn resume_schedule(
    auth: PlatformUser,
    State(state): State<ScheduleAppState>,
    Path(schedule_id): Path<Uuid>,
) -> ApiResult<ScheduleResponse> {
    let schedule = ResumeSchedule { schedule_id }
        .run(&state.ctx(&auth)?)
        .await?;
    Ok(Json(schedule))
}

/// POST /v1/durable/schedules/:schedule_id/trigger - Manually trigger schedule
#[utoipa::path(
    post,
    path = "/v1/durable/schedules/{schedule_id}/trigger",
    params(
        ("schedule_id" = Uuid, Path, description = "Schedule ID")
    ),
    responses(
        (status = 200, description = "Schedule triggered", body = TriggerResponse),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Schedule not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn trigger_schedule(
    auth: PlatformUser,
    State(state): State<ScheduleAppState>,
    Path(schedule_id): Path<Uuid>,
) -> ApiResult<TriggerResponse> {
    let response = TriggerSchedule { schedule_id }
        .run(&state.ctx(&auth)?)
        .await?;
    Ok(Json(response))
}

/// GET /v1/durable/schedules/:schedule_id/executions - List schedule executions
#[utoipa::path(
    get,
    path = "/v1/durable/schedules/{schedule_id}/executions",
    params(
        ("schedule_id" = Uuid, Path, description = "Schedule ID"),
        ("status" = Option<String>, Query, description = "Filter by status"),
        ("offset" = Option<u32>, Query, description = "Pagination offset"),
        ("limit" = Option<u32>, Query, description = "Pagination limit")
    ),
    responses(
        (status = 200, description = "List of executions", body = ScheduleExecutionsListResponse),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Schedule not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn list_schedule_executions(
    auth: PlatformUser,
    State(state): State<ScheduleAppState>,
    Path(schedule_id): Path<Uuid>,
    Query(query): Query<ListExecutionsQuery>,
) -> ApiResult<ScheduleExecutionsListResponse> {
    let executions = ListScheduleExecutions {
        schedule_id,
        status: query.status,
        offset: query.offset,
        limit: query.limit,
    }
    .run(&state.ctx(&auth)?)
    .await?;
    Ok(Json(executions))
}

/// GET /v1/durable/executions/:execution_id - Get execution details
#[utoipa::path(
    get,
    path = "/v1/durable/executions/{execution_id}",
    params(
        ("execution_id" = Uuid, Path, description = "Execution ID")
    ),
    responses(
        (status = 200, description = "Execution details", body = ScheduleExecutionResponse),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Execution not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn get_execution(
    auth: PlatformUser,
    State(state): State<ScheduleAppState>,
    Path(execution_id): Path<Uuid>,
) -> ApiResult<ScheduleExecutionResponse> {
    let execution = GetExecution { execution_id }
        .run(&state.ctx(&auth)?)
        .await?;
    Ok(Json(execution))
}

/// GET /v1/durable/schedules/:schedule_id/stats - Get schedule statistics
#[utoipa::path(
    get,
    path = "/v1/durable/schedules/{schedule_id}/stats",
    params(
        ("schedule_id" = Uuid, Path, description = "Schedule ID")
    ),
    responses(
        (status = 200, description = "Schedule statistics", body = ScheduleStatsResponse),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Schedule not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn get_schedule_stats(
    auth: PlatformUser,
    State(state): State<ScheduleAppState>,
    Path(schedule_id): Path<Uuid>,
) -> ApiResult<ScheduleStatsResponse> {
    let stats = GetScheduleStats { schedule_id }
        .run(&state.ctx(&auth)?)
        .await?;
    Ok(Json(stats))
}

// ============================================================================
// Helper functions
// ============================================================================

/// Calculate next trigger time from cron expression
#[cfg(test)]
fn calculate_next_trigger(
    cron_expression: &str,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, String> {
    use cron::Schedule;
    use std::str::FromStr;

    let schedule =
        Schedule::from_str(cron_expression).map_err(|e| format!("Invalid cron: {}", e))?;

    Ok(schedule.upcoming(chrono::Utc).next())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::backend::AuthBackend;
    use crate::records::OrgMembership;
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::Request;
    use chrono::Utc;
    use everruns_core::OrgRole;
    use everruns_durable::{
        ScheduleExecutionRow, ScheduleExecutionStatus, ScheduleRow, ScheduleTargetType,
    };
    use tower::ServiceExt;

    use crate::auth::config::{AuthConfig, AuthMode, JwtConfig};
    use crate::auth::middleware::{AuthError, AuthMethod, AuthUser};
    use crate::auth::routes::AuthConfigResponse;

    /// AuthState with auth disabled (anonymous admin user)
    fn test_auth_state_none() -> AuthState {
        let config = AuthConfig::default(); // mode = None
        AuthState::builtin(
            config,
            Arc::new(crate::storage::StorageBackend::test_database()),
        )
    }

    /// AuthState with auth enabled (requires valid JWT/API key)
    fn test_auth_state_full() -> AuthState {
        let config = AuthConfig {
            mode: AuthMode::Full,
            jwt: JwtConfig {
                secret: "test-secret-for-unit-tests-only".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        AuthState::builtin(
            config,
            Arc::new(crate::storage::StorageBackend::test_database()),
        )
    }

    /// Build schedule router with given auth state (no backing store)
    fn app_with_auth(auth: AuthState) -> Router {
        routes(ScheduleAppState::new(
            Arc::new(crate::storage::StorageBackend::test_database()),
            None,
            auth,
        ))
    }

    /// Verify unauthenticated request to the given method+path returns 401
    async fn assert_unauthenticated_rejected(method: &str, path: &str) {
        let app = app_with_auth(test_auth_state_full());

        let method = method.parse::<axum::http::Method>().unwrap();
        let req = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status().as_u16(),
            401,
            "Expected 401 for unauthenticated {} {}",
            resp.status(),
            path
        );
    }

    /// Verify authenticated request (no-auth mode) passes auth layer
    /// (may fail with 503 since no store, but should NOT be 401)
    async fn assert_authenticated_passes(method: &str, path: &str) {
        let app = app_with_auth(test_auth_state_none());

        let method = method.parse::<axum::http::Method>().unwrap();
        let req = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        // Should not be 401 - auth passed. Likely 503 (no store) or 400/404.
        assert_ne!(
            resp.status().as_u16(),
            401,
            "Authenticated user should not get 401 for {}",
            path
        );
    }

    // ---- Unauthenticated rejection tests (all endpoints) ----

    #[tokio::test]
    async fn schedule_routes_reject_unauthenticated() {
        let id = Uuid::now_v7();
        let cases: [(&str, String); 11] = [
            ("POST", "/v1/durable/schedules".to_string()),
            ("GET", "/v1/durable/schedules".to_string()),
            ("GET", format!("/v1/durable/schedules/{id}")),
            ("PATCH", format!("/v1/durable/schedules/{id}")),
            ("DELETE", format!("/v1/durable/schedules/{id}")),
            ("POST", format!("/v1/durable/schedules/{id}/pause")),
            ("POST", format!("/v1/durable/schedules/{id}/resume")),
            ("POST", format!("/v1/durable/schedules/{id}/trigger")),
            ("GET", format!("/v1/durable/schedules/{id}/executions")),
            ("GET", format!("/v1/durable/executions/{id}")),
            ("GET", format!("/v1/durable/schedules/{id}/stats")),
        ];
        for (method, path) in cases {
            assert_unauthenticated_rejected(method, &path).await;
        }
    }

    // ---- Authenticated access tests (no-auth mode passes auth layer) ----

    #[tokio::test]
    async fn schedule_routes_pass_authenticated() {
        let id = Uuid::now_v7();
        let cases: [(&str, String); 9] = [
            ("GET", "/v1/durable/schedules".to_string()),
            ("GET", format!("/v1/durable/schedules/{id}")),
            ("DELETE", format!("/v1/durable/schedules/{id}")),
            ("POST", format!("/v1/durable/schedules/{id}/pause")),
            ("POST", format!("/v1/durable/schedules/{id}/resume")),
            ("POST", format!("/v1/durable/schedules/{id}/trigger")),
            ("GET", format!("/v1/durable/schedules/{id}/executions")),
            ("GET", format!("/v1/durable/executions/{id}")),
            ("GET", format!("/v1/durable/schedules/{id}/stats")),
        ];
        for (method, path) in cases {
            assert_authenticated_passes(method, &path).await;
        }
    }

    #[derive(Clone)]
    struct MockAuthBackend {
        is_platform_user: bool,
    }

    #[async_trait]
    impl AuthBackend for MockAuthBackend {
        async fn validate_token(&self, _token: &str) -> Result<AuthUser, AuthError> {
            Ok(AuthUser {
                id: Uuid::new_v4(),
                email: "test@example.com".to_string(),
                name: "Test User".to_string(),
                roles: vec!["user".to_string()],
                is_platform_user: self.is_platform_user,
                auth_method: AuthMethod::Jwt,
                organizations: vec![OrgMembership {
                    org_id: 1,
                    public_id: "org_00000000000000000000000000000001".to_string(),
                    name: "Test Org".to_string(),
                    role: OrgRole::Owner,
                }],
            })
        }

        async fn validate_personal_access_token(
            &self,
            _token: &str,
        ) -> Result<AuthUser, AuthError> {
            Err(AuthError::unauthorized("not supported"))
        }

        fn auth_routes(&self) -> Option<Router> {
            None
        }

        fn auth_config_response(&self) -> AuthConfigResponse {
            AuthConfigResponse {
                mode: "full".to_string(),
                login_origin: None,
                password_auth_enabled: false,
                signup_enabled: false,
                oauth_providers: vec![],
                signup_email_confirm: false,
                captcha: None,
            }
        }
    }

    fn app_with_platform_user(is_platform_user: bool) -> Router {
        let config = AuthConfig {
            mode: AuthMode::Full,
            jwt: JwtConfig {
                secret: "test-secret-for-unit-tests-only".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        let auth = AuthState::new(config, Arc::new(MockAuthBackend { is_platform_user }));
        routes(ScheduleAppState::new(
            Arc::new(crate::storage::StorageBackend::test_database()),
            None,
            auth,
        ))
    }

    #[tokio::test]
    async fn non_platform_user_cannot_list_schedules() {
        let response = app_with_platform_user(false)
            .oneshot(
                Request::builder()
                    .uri("/v1/durable/schedules")
                    .header("Authorization", "Bearer test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn platform_user_can_list_schedules() {
        let response = app_with_platform_user(true)
            .oneshot(
                Request::builder()
                    .uri("/v1/durable/schedules")
                    .header("Authorization", "Bearer test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn non_platform_user_cannot_create_schedule() {
        let response = app_with_platform_user(false)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/durable/schedules")
                    .header("Authorization", "Bearer test-token")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    // ---- Existing unit tests ----

    #[test]
    fn test_calculate_next_trigger() {
        // (cron expression, expect ok-with-value)
        let cases = [
            ("0 * * * * * *", true), // 7-field cron: every minute
            ("invalid", false),
        ];
        for (cron, expect_ok) in cases {
            let result = calculate_next_trigger(cron);
            assert_eq!(
                result.is_ok(),
                expect_ok,
                "calculate_next_trigger({cron:?}) ok-ness mismatch"
            );
            if expect_ok {
                assert!(
                    result.unwrap().is_some(),
                    "calculate_next_trigger({cron:?}) expected Some"
                );
            }
        }
    }

    #[test]
    fn test_schedule_response_from_row() {
        let row = ScheduleRow {
            id: Uuid::now_v7(),
            name: "test-schedule".to_string(),
            description: Some("A test".to_string()),
            cron_expression: "0 * * * * * *".to_string(),
            timezone: "UTC".to_string(),
            target_type: ScheduleTargetType::Workflow,
            target_name: "my-workflow".to_string(),
            target_input: serde_json::json!({"key": "value"}),
            enabled: true,
            max_concurrent: Some(2),
            catch_up_missed: false,
            max_catch_up: None,
            retry_policy: None,
            last_triggered_at: None,
            next_trigger_at: Some(Utc::now()),
            claimed_by: None,
            claimed_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let response = ScheduleResponse::from(row);
        assert_eq!(response.name, "test-schedule");
        assert_eq!(response.target.target_type, "workflow");
        assert_eq!(response.target.name, "my-workflow");
        assert!(response.enabled);
    }

    #[test]
    fn test_execution_response_from_row() {
        let row = ScheduleExecutionRow {
            id: Uuid::now_v7(),
            schedule_id: Uuid::now_v7(),
            scheduled_at: Utc::now(),
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
            status: ScheduleExecutionStatus::Completed,
            workflow_id: Some(Uuid::now_v7()),
            task_id: None,
            error: None,
            duration_ms: Some(150),
            created_at: Utc::now(),
        };

        let response = ScheduleExecutionResponse::from(row);
        assert_eq!(response.status, "completed");
        assert!(response.workflow_id.is_some());
        assert_eq!(response.duration_ms, Some(150));
    }
}
