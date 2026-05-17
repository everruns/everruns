use super::*;
use tonic::service::Interceptor;

// Env-var-mutating tests must not run in parallel.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

async fn test_worker_service() -> WorkerServiceImpl {
    let db = Arc::new(StorageBackend::in_memory());
    let grade = everruns_core::DeploymentGrade::Dev;
    let platform_definition = crate::oss_platform_definition_for_grade(grade);
    let encryption = Some(Arc::new(
        EncryptionService::new("kek-v1:8B3uCQ4Znx45hl5nB+PKVriRrj/KtEVM+wBZ2VGa9vY=", &[])
            .expect("valid test encryption key"),
    ));

    crate::seed::seed_all(&db, grade, &crate::seed::SeedAuthContext::default())
        .await
        .expect("seed test data");

    let event_service =
        EventService::with_listeners(db.clone(), crate::EventDelivery::in_memory(), vec![]);

    WorkerServiceImpl::new(event_service, db, encryption, None, platform_definition)
}

#[test]
fn test_image_info_row_to_proto_uses_raw_uuid_transport_value() {
    let image_id = everruns_core::ImageId::new();
    let proto = WorkerServiceImpl::image_info_row_to_proto(crate::storage::models::ImageInfoRow {
        id: image_id,
        org_id: everruns_core::DEFAULT_ORG_ID,
        filename: "generated-image.png".to_string(),
        content_type: "image/png".to_string(),
        size_bytes: 128,
        metadata: serde_json::json!({ "provider": "openai" }),
        created_at: chrono::Utc::now(),
    });

    let serialized_id = proto
        .id
        .expect("stored image info should include an id")
        .value;

    assert_eq!(serialized_id, image_id.uuid().to_string());
    assert_ne!(serialized_id, image_id.to_string());
}

#[test]
fn test_interceptor_allows_when_no_token_configured() {
    let mut interceptor = GrpcAuthInterceptor::new(None);
    let request = Request::new(());
    assert!(interceptor.call(request).is_ok());
}

#[test]
fn test_interceptor_allows_valid_bearer_token() {
    let mut interceptor = GrpcAuthInterceptor::new(Some("secret123".to_string()));
    let mut request = Request::new(());
    request
        .metadata_mut()
        .insert("authorization", "Bearer secret123".parse().unwrap());
    assert!(interceptor.call(request).is_ok());
}

#[test]
fn test_interceptor_rejects_missing_token() {
    let mut interceptor = GrpcAuthInterceptor::new(Some("secret123".to_string()));
    let request = Request::new(());
    let err = interceptor.call(request).unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unauthenticated);
    assert!(err.message().contains("Missing"));
}

#[test]
fn test_interceptor_rejects_wrong_token() {
    let mut interceptor = GrpcAuthInterceptor::new(Some("secret123".to_string()));
    let mut request = Request::new(());
    request
        .metadata_mut()
        .insert("authorization", "Bearer wrong_token".parse().unwrap());
    let err = interceptor.call(request).unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unauthenticated);
    assert!(err.message().contains("Invalid"));
}

#[test]
fn test_interceptor_rejects_non_bearer_scheme() {
    let mut interceptor = GrpcAuthInterceptor::new(Some("secret123".to_string()));
    let mut request = Request::new(());
    request
        .metadata_mut()
        .insert("authorization", "Basic secret123".parse().unwrap());
    let err = interceptor.call(request).unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unauthenticated);
}

#[tokio::test]
async fn test_list_commands_includes_platform_management_commands() {
    let service = test_worker_service().await;

    let response = service
        .list_commands(Request::new(ListCommandsRequest {}))
        .await
        .expect("list_commands should succeed")
        .into_inner();

    assert!(
        response
            .commands
            .iter()
            .any(|command| command.name == "list_harnesses" && command.api_version == "v1")
    );
    assert!(
        response
            .commands
            .iter()
            .any(|command| command.name == "create_app" && command.api_version == "v1")
    );
}

#[tokio::test]
async fn test_execute_command_lists_seeded_harnesses() {
    let service = test_worker_service().await;

    let response = service
        .execute_command(Request::new(ExecuteCommandRequest {
            name: "list_harnesses".to_string(),
            api_version: "v1".to_string(),
            params_json: br#"{}"#.to_vec(),
            org_id: everruns_core::DEFAULT_ORG_ID,
            user_id: None,
            idempotency_key: None,
            metadata: Default::default(),
        }))
        .await
        .expect("execute_command should succeed")
        .into_inner();

    let proto::execute_command_response::Result::OkJson(ok_json) =
        response.result.expect("command result should be present")
    else {
        panic!("expected OkJson response");
    };

    let harnesses: serde_json::Value =
        serde_json::from_slice(&ok_json).expect("response should be valid JSON");
    let names: Vec<&str> = harnesses
        .as_array()
        .expect("list_harnesses should return an array")
        .iter()
        .filter_map(|h| h.get("name").and_then(|name| name.as_str()))
        .collect();

    assert!(names.contains(&"platform-chat"));
}

#[tokio::test]
async fn test_execute_command_unknown_command_returns_bad_request_kind() {
    let service = test_worker_service().await;

    let response = service
        .execute_command(Request::new(ExecuteCommandRequest {
            name: "definitely_not_a_command".to_string(),
            api_version: "v1".to_string(),
            params_json: br#"{}"#.to_vec(),
            org_id: everruns_core::DEFAULT_ORG_ID,
            user_id: None,
            idempotency_key: None,
            metadata: Default::default(),
        }))
        .await
        .expect("execute_command should return a structured command error")
        .into_inner();

    let proto::execute_command_response::Result::Error(error) =
        response.result.expect("command result should be present")
    else {
        panic!("expected Error response");
    };

    assert_eq!(error.kind, 1);
    assert!(error.message.contains("Unknown command"));
}

#[tokio::test]
async fn test_memory_rpc_create_and_recall_round_trip() {
    let service = test_worker_service().await;
    let public_org_id = everruns_core::org_public_id_from_internal(everruns_core::DEFAULT_ORG_ID);

    let store = service
        .memory_get_or_create_default_store(Request::new(MemoryGetOrCreateDefaultStoreRequest {
            org_id: everruns_core::DEFAULT_ORG_ID,
            public_org_id,
        }))
        .await
        .expect("default store should be available")
        .into_inner()
        .store
        .expect("store response");

    service
        .memory_create(Request::new(MemoryCreateRequest {
            org_id: everruns_core::DEFAULT_ORG_ID,
            store_id: store.id.clone(),
            content: "The user's name is Mike and he prefers concise technical answers."
                .to_string(),
            content_parts_json: serde_json::to_vec(&vec![everruns_core::MemoryContentPart::text(
                "The user's name is Mike and he prefers concise technical answers.",
            )])
            .expect("content parts json"),
            kind: "fact".to_string(),
            importance: 7,
            tags: vec!["name".to_string(), "preference".to_string()],
        }))
        .await
        .expect("memory create should succeed");

    let recalled = service
        .memory_recall(Request::new(MemoryRecallRequest {
            org_id: everruns_core::DEFAULT_ORG_ID,
            store_id: Some(store.id),
            query: Some("Mike".to_string()),
            tags: vec![],
            has_tags: false,
            kind: None,
            limit: 10,
        }))
        .await
        .expect("memory recall should succeed")
        .into_inner();

    assert_eq!(recalled.total, 1);
    assert_eq!(recalled.memories.len(), 1);
    assert!(recalled.memories[0].content.contains("Mike"));
}

#[tokio::test]
async fn test_execute_command_uses_user_permissions() {
    use crate::storage::models::CreateUserRow;

    let service = test_worker_service().await;
    let user = service
        .db
        .create_user(CreateUserRow {
            email: "grpc-member@example.com".to_string(),
            name: "gRPC Member".to_string(),
            avatar_url: None,
            external_id: None,
            roles: vec![],
            password_hash: None,
            email_verified: true,
            auth_provider: Some("test".to_string()),
            auth_provider_id: None,
        })
        .await
        .expect("create user");
    service
        .db
        .ensure_membership(user.id, everruns_core::DEFAULT_ORG_ID, "member")
        .await
        .expect("ensure membership");

    let response = service
        .execute_command(Request::new(ExecuteCommandRequest {
            name: "create_harness".to_string(),
            api_version: "v1".to_string(),
            params_json: serde_json::to_vec(&serde_json::json!({
                "name": "forbidden-from-grpc",
                "system_prompt": "prompt",
                "tags": [],
                "capabilities": [],
                "initial_files": [],
                "mcp_servers": {},
            }))
            .expect("serialize params"),
            org_id: everruns_core::DEFAULT_ORG_ID,
            user_id: Some(user.id.to_string()),
            idempotency_key: None,
            metadata: Default::default(),
        }))
        .await
        .expect("execute_command should return a structured command error")
        .into_inner();

    let proto::execute_command_response::Result::Error(error) =
        response.result.expect("command result should be present")
    else {
        panic!("expected Error response");
    };

    assert_eq!(error.kind, 2);
    assert!(error.message.contains("Access denied"));
}

/// Acquire env lock, tolerating poison from #[should_panic] tests.
fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

// TM-DURABLE-002: require_grpc_auth_token panics when WORKER_GRPC_AUTH_TOKEN is unset
#[test]
#[should_panic(expected = "WORKER_GRPC_AUTH_TOKEN must be set")]
fn test_require_grpc_auth_token_panics_without_env() {
    let _lock = lock_env();
    unsafe { std::env::remove_var("WORKER_GRPC_AUTH_TOKEN") };
    require_grpc_auth_token();
}

#[test]
fn test_require_grpc_auth_token_returns_value() {
    let _lock = lock_env();
    unsafe { std::env::set_var("WORKER_GRPC_AUTH_TOKEN", "test-token-123") };
    let token = require_grpc_auth_token();
    assert_eq!(token, "test-token-123");
    unsafe { std::env::remove_var("WORKER_GRPC_AUTH_TOKEN") };
}

#[test]
fn test_grpc_server_tls_returns_none_when_no_env_vars() {
    let _lock = lock_env();
    unsafe {
        std::env::remove_var("WORKER_GRPC_TLS_CERT");
        std::env::remove_var("WORKER_GRPC_TLS_KEY");
        std::env::remove_var("WORKER_GRPC_TLS_CA_CERT");
    }
    let config = grpc_server_tls_from_env();
    assert!(
        config.is_none(),
        "Should return None when TLS not configured"
    );
}

#[test]
fn test_grpc_server_tls_returns_none_when_cert_empty() {
    let _lock = lock_env();
    unsafe {
        std::env::set_var("WORKER_GRPC_TLS_CERT", "");
        std::env::set_var("WORKER_GRPC_TLS_KEY", "");
    }
    let config = grpc_server_tls_from_env();
    assert!(config.is_none());
    unsafe {
        std::env::remove_var("WORKER_GRPC_TLS_CERT");
        std::env::remove_var("WORKER_GRPC_TLS_KEY");
    }
}

#[test]
fn test_grpc_server_tls_returns_config_with_valid_certs() {
    let _lock = lock_env();
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let cert_path = format!("{}/tests/fixtures/test-server-cert.pem", manifest);
    let key_path = format!("{}/tests/fixtures/test-server-key.pem", manifest);

    unsafe {
        std::env::set_var("WORKER_GRPC_TLS_CERT", &cert_path);
        std::env::set_var("WORKER_GRPC_TLS_KEY", &key_path);
        std::env::remove_var("WORKER_GRPC_TLS_CA_CERT");
    }

    let config = grpc_server_tls_from_env();
    assert!(
        config.is_some(),
        "Should return Some when cert+key are configured"
    );

    unsafe {
        std::env::remove_var("WORKER_GRPC_TLS_CERT");
        std::env::remove_var("WORKER_GRPC_TLS_KEY");
    }
}

#[test]
fn test_grpc_server_tls_with_client_ca() {
    let _lock = lock_env();
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let cert_path = format!("{}/tests/fixtures/test-server-cert.pem", manifest);
    let key_path = format!("{}/tests/fixtures/test-server-key.pem", manifest);
    let ca_path = format!("{}/tests/fixtures/test-ca.pem", manifest);

    unsafe {
        std::env::set_var("WORKER_GRPC_TLS_CERT", &cert_path);
        std::env::set_var("WORKER_GRPC_TLS_KEY", &key_path);
        std::env::set_var("WORKER_GRPC_TLS_CA_CERT", &ca_path);
    }

    let config = grpc_server_tls_from_env();
    assert!(
        config.is_some(),
        "Should return Some when cert+key+ca are configured"
    );

    unsafe {
        std::env::remove_var("WORKER_GRPC_TLS_CERT");
        std::env::remove_var("WORKER_GRPC_TLS_KEY");
        std::env::remove_var("WORKER_GRPC_TLS_CA_CERT");
    }
}

#[test]
#[should_panic(expected = "Failed to read WORKER_GRPC_TLS_CERT")]
fn test_grpc_server_tls_panics_on_missing_cert_file() {
    let _lock = lock_env();
    unsafe {
        std::env::set_var("WORKER_GRPC_TLS_CERT", "/nonexistent/cert.pem");
        std::env::set_var("WORKER_GRPC_TLS_KEY", "/nonexistent/key.pem");
    }
    let _config = grpc_server_tls_from_env();
    // cleanup won't run due to panic, but that's fine for test
}

// ========================================================================
// Session SQL database helper tests
// ========================================================================

#[test]
fn test_sqldb_error_to_status_maps_not_found() {
    use everruns_core::session_sqldb::SessionSqlDbError;
    let err = SessionSqlDbError::DatabaseNotFound("test_db".into());
    let status = sqldb_error_to_status(err);
    assert_eq!(status.code(), tonic::Code::NotFound);
}

#[test]
fn test_sqldb_error_to_status_maps_already_exists() {
    use everruns_core::session_sqldb::SessionSqlDbError;
    let err = SessionSqlDbError::DatabaseAlreadyExists("test_db".into());
    let status = sqldb_error_to_status(err);
    assert_eq!(status.code(), tonic::Code::AlreadyExists);
}

#[test]
fn test_sqldb_error_to_status_maps_invalid_name() {
    use everruns_core::session_sqldb::SessionSqlDbError;
    let err = SessionSqlDbError::InvalidDatabaseName("bad!name".into());
    let status = sqldb_error_to_status(err);
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
}

#[test]
fn test_sqldb_error_to_status_maps_limit_exceeded() {
    use everruns_core::session_sqldb::SessionSqlDbError;
    let err = SessionSqlDbError::LimitExceeded("max 10".into());
    let status = sqldb_error_to_status(err);
    assert_eq!(status.code(), tonic::Code::ResourceExhausted);
}

#[test]
fn test_sqldb_error_to_status_maps_query_error() {
    use everruns_core::session_sqldb::SessionSqlDbError;
    let err = SessionSqlDbError::QueryError("syntax error".into());
    let status = sqldb_error_to_status(err);
    assert_eq!(status.code(), tonic::Code::FailedPrecondition);
}

#[test]
fn test_sqldb_error_to_status_maps_timeout() {
    use everruns_core::session_sqldb::SessionSqlDbError;
    let err = SessionSqlDbError::QueryTimeout(30);
    let status = sqldb_error_to_status(err);
    assert_eq!(status.code(), tonic::Code::DeadlineExceeded);
}

#[test]
fn test_sqldb_error_to_status_maps_authorizer_blocked() {
    use everruns_core::session_sqldb::SessionSqlDbError;
    let err = SessionSqlDbError::AuthorizerBlocked("DROP TABLE".into());
    let status = sqldb_error_to_status(err);
    assert_eq!(status.code(), tonic::Code::PermissionDenied);
}

#[test]
fn test_sqldb_error_to_status_maps_result_too_large() {
    use everruns_core::session_sqldb::SessionSqlDbError;
    let err = SessionSqlDbError::ResultTooLarge("1MB limit".into());
    let status = sqldb_error_to_status(err);
    assert_eq!(status.code(), tonic::Code::FailedPrecondition);
}

#[test]
fn test_sqldb_error_to_status_maps_internal() {
    use everruns_core::session_sqldb::SessionSqlDbError;
    let err = SessionSqlDbError::Internal("unexpected".into());
    let status = sqldb_error_to_status(err);
    assert_eq!(status.code(), tonic::Code::Internal);
}

#[test]
fn test_json_value_to_proto_null() {
    let val = json_value_to_proto(serde_json::Value::Null);
    assert!(matches!(
        val.kind,
        Some(prost_types::value::Kind::NullValue(0))
    ));
}

#[test]
fn test_json_value_to_proto_string() {
    let val = json_value_to_proto(serde_json::Value::String("hello".into()));
    assert!(matches!(val.kind, Some(prost_types::value::Kind::StringValue(ref s)) if s == "hello"));
}

#[test]
fn test_json_value_to_proto_number() {
    let val = json_value_to_proto(serde_json::json!(42.0));
    assert!(
        matches!(val.kind, Some(prost_types::value::Kind::NumberValue(n)) if (n - 42.0).abs() < f64::EPSILON)
    );
}

#[test]
fn test_json_value_to_proto_bool() {
    let val = json_value_to_proto(serde_json::Value::Bool(true));
    assert!(matches!(
        val.kind,
        Some(prost_types::value::Kind::BoolValue(true))
    ));
}

#[test]
fn test_json_value_to_proto_array() {
    let val = json_value_to_proto(serde_json::json!([1, "two", null]));
    match val.kind {
        Some(prost_types::value::Kind::ListValue(list)) => {
            assert_eq!(list.values.len(), 3);
            assert!(matches!(
                list.values[0].kind,
                Some(prost_types::value::Kind::NumberValue(_))
            ));
            assert!(matches!(
                list.values[1].kind,
                Some(prost_types::value::Kind::StringValue(_))
            ));
            assert!(matches!(
                list.values[2].kind,
                Some(prost_types::value::Kind::NullValue(_))
            ));
        }
        _ => panic!("Expected ListValue"),
    }
}

#[test]
fn test_json_value_to_proto_object() {
    let val = json_value_to_proto(serde_json::json!({"key": "value", "num": 42}));
    match val.kind {
        Some(prost_types::value::Kind::StructValue(s)) => {
            assert_eq!(s.fields.len(), 2);
            assert!(s.fields.contains_key("key"));
            assert!(s.fields.contains_key("num"));
        }
        _ => panic!("Expected StructValue"),
    }
}

#[test]
fn test_db_info_to_proto_roundtrip() {
    use chrono::Utc;
    let now = Utc::now();
    let info = everruns_core::session_sqldb::DatabaseInfo {
        name: "test_db".into(),
        size_bytes: 4096,
        page_count: 1,
        created_at: now,
        updated_at: now,
    };
    let proto = db_info_to_proto(info);
    assert_eq!(proto.name, "test_db");
    assert_eq!(proto.size_bytes, 4096);
    assert_eq!(proto.page_count, 1);
    assert!(proto.created_at.is_some());
    assert!(proto.updated_at.is_some());
}
