use super::*;
use tonic::service::Interceptor;

// Env-var-mutating tests must not run in parallel.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
