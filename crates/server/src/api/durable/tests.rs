use super::*;

// Trivial derive-only serde round-trips removed; covered by the derive + handler tests.

#[test]
fn test_health_status_reflects_worker_and_dlq_state() {
    // (system health, expected status)
    let cases = [
        (
            "no active workers",
            SystemHealth {
                total_workers: 5,
                active_workers: 0,
                workers_accepting: 0,
                total_capacity: 0,
                current_load: 0,
                pending_tasks: 0,
                claimed_tasks: 0,
                completed_tasks: 0,
                failed_tasks: 0,
                started_tasks: 0,
                running_workflows: 0,
                pending_workflows: 0,
                completed_workflows: 0,
                failed_workflows: 0,
                started_workflows: 0,
                dlq_size: 0,
            },
            "degraded",
        ),
        (
            "dlq has items",
            SystemHealth {
                total_workers: 2,
                active_workers: 2,
                workers_accepting: 2,
                total_capacity: 10,
                current_load: 0,
                pending_tasks: 0,
                claimed_tasks: 0,
                completed_tasks: 0,
                failed_tasks: 0,
                started_tasks: 0,
                running_workflows: 0,
                pending_workflows: 0,
                completed_workflows: 0,
                failed_workflows: 0,
                started_workflows: 0,
                dlq_size: 5,
            },
            "warning",
        ),
    ];
    for (case, health, expected_status) in cases {
        let response = HealthResponse::from(health);
        assert_eq!(response.status, expected_status, "case: {case}");
    }
}

#[test]
fn test_load_percentage_zero_when_no_capacity() {
    let health = SystemHealth {
        total_workers: 0,
        active_workers: 0,
        workers_accepting: 0,
        total_capacity: 0,
        current_load: 0,
        pending_tasks: 0,
        completed_tasks: 0,
        failed_tasks: 0,
        started_tasks: 0,
        claimed_tasks: 0,
        running_workflows: 0,
        pending_workflows: 0,
        completed_workflows: 0,
        failed_workflows: 0,
        started_workflows: 0,
        dlq_size: 0,
    };

    let response = HealthResponse::from(health);
    assert_eq!(response.load_percentage, 0.0);
}

#[tokio::test]
async fn disconnecting_event_encodes_client_frame() {
    use axum::{body::to_bytes, response::IntoResponse};

    for (reason, expected_reason) in [
        (DisconnectReason::ConnectionCycle, "connection_cycle"),
        (DisconnectReason::Error, "error"),
        (DisconnectReason::Shutdown, "shutdown"),
    ] {
        let mut config = SseStreamConfig::monitoring();
        config.disconnect_retry_ms = 2345;
        let event = disconnecting_event(reason, &config);
        let response =
            Sse::new(stream::iter([Ok::<_, std::convert::Infallible>(event)])).into_response();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let frame = std::str::from_utf8(&body).unwrap();
        assert!(frame.lines().any(|line| line == "event: disconnecting"));
        assert!(frame.lines().any(|line| line == "retry: 2345"));
        let data = frame
            .lines()
            .find_map(|line| line.strip_prefix("data: "))
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(data).unwrap();
        assert_eq!(
            parsed,
            serde_json::json!({"reason": expected_reason, "retry_ms": 2345})
        );
    }
}

// ============================================
// WorkersSummaryResponse tests
// ============================================

/// Helper to create a WorkerResponse for testing
fn make_worker(id: &str, status: &str, max_concurrency: u32, current_load: u32) -> WorkerResponse {
    WorkerResponse {
        id: id.to_string(),
        worker_group: Some("default".to_string()),
        activity_types: vec!["run_session".to_string()],
        max_concurrency,
        current_load,
        status: status.to_string(),
        accepting_tasks: status == "active",
        backpressure_reason: None,
        started_at: Utc::now(),
        last_heartbeat_at: Utc::now(),
        hostname: Some("host1".to_string()),
        version: Some("0.8.1".to_string()),
        metadata: None,
        tasks_completed: 100,
        tasks_failed: 2,
        avg_task_duration_ms: Some(500),
    }
}

/// Build a WorkersListResponse via the same shared summary fn list_workers() uses.
fn build_workers_list(workers: Vec<WorkerResponse>) -> WorkersListResponse {
    let total = workers.len();
    let summary = workers_summary(&workers);
    WorkersListResponse {
        data: workers,
        total,
        summary,
    }
}

/// Expected WorkersSummaryResponse shape for a table case below.
struct WorkersSummaryCase {
    name: &'static str,
    workers: Vec<WorkerResponse>,
    total: usize,
    active: usize,
    draining: usize,
    stopped: usize,
    total_capacity: usize,
    total_load: usize,
}

#[test]
fn test_workers_summary_counts_and_capacity() {
    // Only active workers count toward capacity/load; draining/stopped never do,
    // no matter how much load or capacity they report.
    let cases = [
        WorkersSummaryCase {
            name: "mixed_statuses",
            workers: vec![
                make_worker("w1", "active", 10, 3),
                make_worker("w2", "active", 8, 5),
                make_worker("w3", "draining", 10, 2),
                make_worker("w4", "stopped", 10, 0),
            ],
            total: 4,
            active: 2,
            draining: 1,
            stopped: 1,
            total_capacity: 18, // 10 + 8
            total_load: 8,      // 3 + 5
        },
        WorkersSummaryCase {
            name: "all_active",
            workers: vec![
                make_worker("w1", "active", 10, 5),
                make_worker("w2", "active", 10, 3),
                make_worker("w3", "active", 10, 7),
            ],
            total: 3,
            active: 3,
            draining: 0,
            stopped: 0,
            total_capacity: 30,
            total_load: 15,
        },
        WorkersSummaryCase {
            name: "no_active_workers",
            workers: vec![
                make_worker("w1", "draining", 10, 2),
                make_worker("w2", "stopped", 10, 0),
            ],
            total: 2,
            active: 0,
            draining: 1,
            stopped: 1,
            total_capacity: 0,
            total_load: 0,
        },
        WorkersSummaryCase {
            name: "empty_list",
            workers: vec![],
            total: 0,
            active: 0,
            draining: 0,
            stopped: 0,
            total_capacity: 0,
            total_load: 0,
        },
        WorkersSummaryCase {
            name: "only_active_contribute_to_capacity",
            // A draining worker with high load must NOT inflate totals.
            workers: vec![
                make_worker("w1", "active", 5, 2),
                make_worker("w2", "draining", 100, 80),
                make_worker("w3", "stopped", 50, 0),
            ],
            total: 3,
            active: 1,
            draining: 1,
            stopped: 1,
            total_capacity: 5, // Only w1
            total_load: 2,     // Only w1
        },
        WorkersSummaryCase {
            name: "single_active_worker",
            workers: vec![make_worker("w1", "active", 20, 15)],
            total: 1,
            active: 1,
            draining: 0,
            stopped: 0,
            total_capacity: 20,
            total_load: 15,
        },
        WorkersSummaryCase {
            name: "zero_load_active_workers",
            workers: vec![
                make_worker("w1", "active", 10, 0),
                make_worker("w2", "active", 10, 0),
            ],
            total: 2,
            active: 2,
            draining: 0,
            stopped: 0,
            total_capacity: 20,
            total_load: 0,
        },
    ];

    for case in cases {
        let resp = build_workers_list(case.workers);
        assert_eq!(resp.total, case.total, "case {}: total", case.name);
        assert_eq!(
            resp.summary.active, case.active,
            "case {}: active",
            case.name
        );
        assert_eq!(
            resp.summary.draining, case.draining,
            "case {}: draining",
            case.name
        );
        assert_eq!(
            resp.summary.stopped, case.stopped,
            "case {}: stopped",
            case.name
        );
        assert_eq!(
            resp.summary.total_capacity, case.total_capacity,
            "case {}: total_capacity",
            case.name
        );
        assert_eq!(
            resp.summary.total_load, case.total_load,
            "case {}: total_load",
            case.name
        );
    }
}

// ============================================
// MetricsCollector tests
// ============================================

#[tokio::test]
async fn test_metrics_collector_evicts_oldest_when_full() {
    let collector = MetricsCollector::new(3);

    for i in 0..5 {
        collector
            .push(MetricsPoint {
                timestamp: Utc::now(),
                running_workflows: i,
                pending_workflows: 0,
                pending_tasks: 0,
                claimed_tasks: 0,
                active_workers: 0,
                load_percentage: 0.0,
                dlq_size: 0,
                tasks_completed_total: 0,
                tasks_failed_total: 0,
                tasks_started_total: 0,
                workflows_completed_total: 0,
                workflows_failed_total: 0,
                workflows_started_total: 0,
            })
            .await;
    }

    let points = collector.get_points().await;
    assert_eq!(points.len(), 3);
    // Oldest (0, 1) should be evicted; remaining are 2, 3, 4
    assert_eq!(points[0].running_workflows, 2);
    assert_eq!(points[1].running_workflows, 3);
    assert_eq!(points[2].running_workflows, 4);
}

// ============================================
// started_tasks / started_workflows coverage
// ============================================

#[test]
fn test_health_response_from_system_health_maps_started_fields() {
    let health = SystemHealth {
        total_workers: 4,
        active_workers: 3,
        workers_accepting: 2,
        total_capacity: 40,
        current_load: 15,
        pending_tasks: 8,
        claimed_tasks: 15,
        completed_tasks: 200,
        failed_tasks: 20,
        started_tasks: 243,
        running_workflows: 5,
        pending_workflows: 3,
        completed_workflows: 100,
        failed_workflows: 10,
        started_workflows: 118,
        dlq_size: 2,
    };

    let response = HealthResponse::from(health);

    // Verify started fields are mapped through correctly
    assert_eq!(response.started_tasks, 243);
    assert_eq!(response.started_workflows, 118);

    // Verify all other fields still map correctly
    assert_eq!(response.total_workers, 4);
    assert_eq!(response.active_workers, 3);
    assert_eq!(response.workers_accepting, 2);
    assert_eq!(response.total_capacity, 40);
    assert_eq!(response.current_load, 15);
    assert_eq!(response.pending_tasks, 8);
    assert_eq!(response.claimed_tasks, 15);
    assert_eq!(response.completed_tasks, 200);
    assert_eq!(response.failed_tasks, 20);
    assert_eq!(response.running_workflows, 5);
    assert_eq!(response.pending_workflows, 3);
    assert_eq!(response.completed_workflows, 100);
    assert_eq!(response.failed_workflows, 10);
    assert_eq!(response.dlq_size, 2);
    assert_eq!(response.status, "warning"); // dlq_size > 0
    assert!((response.load_percentage - 37.5).abs() < 0.01); // 15/40 * 100
}

#[test]
fn test_metrics_sampler_maps_started_fields_from_health() {
    // Verify the mapping used by the metrics sampler: health.started_tasks -> tasks_started_total.
    // Calls the real mapping fn (shared with spawn_metrics_sampler) instead of re-deriving it.
    let health = SystemHealth {
        total_workers: 2,
        active_workers: 2,
        workers_accepting: 2,
        total_capacity: 20,
        current_load: 5,
        pending_tasks: 3,
        claimed_tasks: 5,
        completed_tasks: 80,
        failed_tasks: 8,
        started_tasks: 96,
        running_workflows: 4,
        pending_workflows: 1,
        completed_workflows: 40,
        failed_workflows: 3,
        started_workflows: 48,
        dlq_size: 0,
    };

    let point = metrics_point_from_health(&health);

    // Verify the mapping
    assert_eq!(point.tasks_started_total, 96);
    assert_eq!(point.workflows_started_total, 48);
    assert_eq!(point.tasks_completed_total, 80);
    assert_eq!(point.tasks_failed_total, 8);
    assert_eq!(point.workflows_completed_total, 40);
    assert_eq!(point.workflows_failed_total, 3);
    assert!((point.load_percentage - 25.0).abs() < 0.01);
}

// ============================================
// Integration test: get_health with InMemoryWorkflowEventStore
// ============================================

#[tokio::test]
async fn test_get_health_with_in_memory_store() {
    use everruns_durable::prelude::*;

    let store = Arc::new(InMemoryWorkflowEventStore::new());

    // Register a worker with fresh heartbeat
    store
        .register_worker(WorkerInfo {
            id: "test-worker".to_string(),
            worker_group: Some("default".to_string()),
            activity_types: vec!["act".to_string()],
            max_concurrency: 10,
            current_load: 3,
            status: "active".to_string(),
            accepting_tasks: true,
            backpressure_reason: None,
            started_at: Utc::now(),
            last_heartbeat_at: Utc::now(),
            hostname: None,
            version: None,
            metadata: None,
            tasks_completed: 0,
            tasks_failed: 0,
            avg_task_duration_ms: None,
        })
        .await
        .unwrap();

    // Create a pending workflow
    let wf_id = Uuid::now_v7();
    store
        .create_workflow(wf_id, "test", serde_json::json!({}), None)
        .await
        .unwrap();

    let health = store.get_system_health().await.unwrap();
    let response = HealthResponse::from(health);

    // Verify worker metrics
    assert_eq!(response.active_workers, 1);
    assert_eq!(response.workers_accepting, 1);
    assert_eq!(response.total_capacity, 10);
    assert_eq!(response.current_load, 3);
    assert!((response.load_percentage - 30.0).abs() < 0.01);

    // Verify workflow metrics
    assert_eq!(response.pending_workflows, 1);
    assert_eq!(response.running_workflows, 0);

    // Status should be healthy (active workers, no DLQ)
    assert_eq!(response.status, "healthy");
}

#[tokio::test]
async fn test_get_health_with_in_memory_store_full_scenario() {
    use everruns_durable::prelude::*;

    let store = Arc::new(InMemoryWorkflowEventStore::new());

    // Register worker
    store
        .register_worker(WorkerInfo {
            id: "w1".to_string(),
            worker_group: Some("default".to_string()),
            activity_types: vec!["act".to_string()],
            max_concurrency: 10,
            current_load: 0,
            status: "active".to_string(),
            accepting_tasks: true,
            backpressure_reason: None,
            started_at: Utc::now(),
            last_heartbeat_at: Utc::now(),
            hostname: None,
            version: None,
            metadata: None,
            tasks_completed: 0,
            tasks_failed: 0,
            avg_task_duration_ms: None,
        })
        .await
        .unwrap();

    // Create workflows in different states
    let wf_pending = Uuid::now_v7();
    store
        .create_workflow(wf_pending, "test", serde_json::json!({}), None)
        .await
        .unwrap();

    let wf_running = Uuid::now_v7();
    store
        .create_workflow(wf_running, "test", serde_json::json!({}), None)
        .await
        .unwrap();
    store
        .update_workflow_status(wf_running, WorkflowStatus::Running, None, None)
        .await
        .unwrap();

    // Create and enqueue tasks
    store
        .enqueue_task(TaskDefinition {
            workflow_id: Some(wf_running),
            activity_id: "a1".to_string(),
            activity_type: "act".to_string(),
            input: serde_json::json!({}),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();

    store
        .enqueue_task(TaskDefinition {
            workflow_id: Some(wf_running),
            activity_id: "a2".to_string(),
            activity_type: "act".to_string(),
            input: serde_json::json!({}),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();

    // Claim one task
    let claimed = store
        .claim_task("w1", &["act".to_string()], 1)
        .await
        .unwrap();
    let claimed_task_id = claimed[0].id;

    // Complete the claimed task
    store
        .complete_task(claimed_task_id, "w1", serde_json::json!({}))
        .await
        .unwrap();

    let health = store.get_system_health().await.unwrap();
    let response = HealthResponse::from(health);

    // Verify all metrics
    assert_eq!(response.active_workers, 1);
    assert_eq!(response.total_capacity, 10);
    assert_eq!(response.pending_workflows, 1);
    assert_eq!(response.running_workflows, 1);
    assert_eq!(response.pending_tasks, 1); // a2 still pending
    assert_eq!(response.claimed_tasks, 0); // task1 completed, not claimed
    assert_eq!(response.completed_tasks, 1);
    assert_eq!(response.started_tasks, 1); // task1 was claimed
    assert_eq!(response.started_workflows, 1); // wf_running has started_at
    assert_eq!(response.status, "healthy");
}

// ============================================
// Platform-user access control tests
// ============================================

mod platform_user_access {
    use super::*;
    use crate::auth::backend::AuthBackend;
    use crate::auth::config::{AuthConfig, AuthMode};
    use crate::auth::middleware::{AuthError, AuthMethod, AuthUser};
    use crate::auth::routes::AuthConfigResponse;
    use crate::domains::organizations::record::OrgMembership;
    use async_trait::async_trait;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use everruns_core::OrgRole;
    use tower::ServiceExt;

    /// Mock auth backend returning user with configurable platform access.
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
                ..AuthUser::anonymous()
            })
        }

        async fn validate_personal_access_token(
            &self,
            _token: &str,
        ) -> Result<AuthUser, AuthError> {
            Err(AuthError::unauthorized("Not supported"))
        }

        fn auth_routes(&self) -> Option<axum::Router> {
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

    /// Build test router with configurable platform-user access.
    fn test_app(is_platform_user: bool) -> axum::Router {
        let config = AuthConfig {
            mode: AuthMode::Full,
            jwt: crate::auth::config::JwtConfig {
                secret: "test-secret".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        let backend = Arc::new(MockAuthBackend { is_platform_user });
        let auth = AuthState::new(config, backend);
        let state = AppState::new(None, auth, None, "in_memory".to_string());
        routes(state)
    }

    /// Helper: send GET with Bearer token
    async fn get_with_auth(app: axum::Router, uri: &str) -> axum::http::StatusCode {
        app.oneshot(
            Request::builder()
                .uri(uri)
                .header("Authorization", "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
    }

    /// Helper: send POST with Bearer token
    async fn post_with_auth(app: axum::Router, uri: &str, body: &str) -> axum::http::StatusCode {
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("Authorization", "Bearer test-token")
                .header("Content-Type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
    }

    // --- Non-platform user gets 403 on protected read endpoints ---

    #[tokio::test]
    async fn non_platform_user_cannot_read_protected_endpoints() {
        let paths = [
            "/v1/platform/durable/health",
            "/v1/platform/durable/workers",
            "/v1/platform/durable/workflows",
            "/v1/platform/durable/tasks",
            "/v1/platform/durable/dlq",
            "/v1/platform/durable/circuit-breakers",
            "/v1/platform/durable/metrics/timeseries",
            "/v1/platform/durable/sse",
        ];
        for path in paths {
            let status = get_with_auth(test_app(false), path).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "path: {path}");
        }
    }

    #[tokio::test]
    async fn non_platform_user_can_read_durable_config() {
        let response = test_app(false)
            .oneshot(
                Request::builder()
                    .uri("/v1/platform/durable/config")
                    .header("Authorization", "Bearer test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["policies"]["durable.view"], false);
        assert_eq!(payload["policies"]["durable.manage"], false);
    }

    // --- Non-platform user gets 403 on mutate endpoints ---

    #[tokio::test]
    async fn non_platform_user_cannot_mutate_protected_endpoints() {
        let wf_id = Uuid::nil();
        let dlq_id = Uuid::nil();
        let cases: Vec<(String, &str)> = vec![
            ("/v1/platform/durable/workers/w1/drain".to_string(), "{}"),
            (
                format!("/v1/platform/durable/workflows/{wf_id}/cancel"),
                "{}",
            ),
            (
                "/v1/platform/durable/tasks".to_string(),
                r#"{"activity_type":"test","input":{}}"#,
            ),
            (format!("/v1/platform/durable/dlq/{dlq_id}/retry"), "{}"),
            (
                "/v1/platform/durable/circuit-breakers/test-key/open".to_string(),
                "{}",
            ),
        ];

        for (path, body) in cases {
            let status = post_with_auth(test_app(false), &path, body).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "path: {path}");
        }
    }

    // --- Platform user gets access (503 = store unavailable, not 403) ---

    #[tokio::test]
    async fn platform_user_can_access_protected_endpoints() {
        let paths = [
            "/v1/platform/durable/health",
            "/v1/platform/durable/workers",
            "/v1/platform/durable/workflows",
            "/v1/platform/durable/tasks",
            "/v1/platform/durable/dlq",
            "/v1/platform/durable/circuit-breakers",
            "/v1/platform/durable/sse",
        ];
        for path in paths {
            let status = get_with_auth(test_app(true), path).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "path: {path}");
        }
    }

    #[tokio::test]
    async fn durable_config_exposes_platform_policy_from_resolved_org() {
        let response = test_app(true)
            .oneshot(
                Request::builder()
                    .uri("/v1/platform/durable/config")
                    .header("Authorization", "Bearer test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["policies"]["durable.view"], true);
        assert_eq!(payload["policies"]["durable.manage"], true);
    }
}
