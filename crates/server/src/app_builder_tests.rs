use super::*;

#[test]
fn test_h2_config_defaults() {
    let config = Http2FlowConfig::from_values(None, None, None);
    assert_eq!(config.stream_window, 2 * 1024 * 1024); // 2 MB
    assert_eq!(config.connection_window, 16 * 1024 * 1024); // 16 MB
    assert_eq!(config.max_concurrent_streams, 256);
}

#[test]
fn test_h2_config_from_env() {
    let config = Http2FlowConfig::from_values(Some("4194304"), Some("33554432"), Some("512"));
    assert_eq!(config.stream_window, 4 * 1024 * 1024);
    assert_eq!(config.connection_window, 32 * 1024 * 1024);
    assert_eq!(config.max_concurrent_streams, 512);
}

#[test]
fn test_h2_config_invalid_env_uses_defaults() {
    let config = Http2FlowConfig::from_values(Some("not_a_number"), Some(""), None);
    assert_eq!(config.stream_window, 2 * 1024 * 1024); // falls back to default
    assert_eq!(config.connection_window, 16 * 1024 * 1024); // falls back to default
    assert_eq!(config.max_concurrent_streams, 256); // not set, default
}

// EVE-401: embedders can layer route-specific middleware on the auto-mounted
// personal access token CRUD router via `wrap_personal_access_token_routes()` without re-mounting.
#[tokio::test]
async fn wrap_personal_access_token_routes_applies_custom_layer() {
    use axum::body::Body;
    use axum::http::{HeaderName, HeaderValue, Request, StatusCode};
    use axum::routing::get;
    use tower::ServiceExt;
    use tower_http::set_header::SetResponseHeaderLayer;

    let base = Router::new().route("/v1/auth/personal-access-tokens", get(|| async { "ok" }));
    let wrap: PersonalAccessTokenRoutesWrapFn = Box::new(|r: Router| {
        r.layer(SetResponseHeaderLayer::if_not_present(
            HeaderName::from_static("x-eve-401-marker"),
            HeaderValue::from_static("applied"),
        ))
    });

    let router = apply_personal_access_token_routes_wrap(Some(wrap), base);

    let response = router
        .oneshot(
            Request::builder()
                .uri("/v1/auth/personal-access-tokens")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("x-eve-401-marker").unwrap(),
        "applied"
    );
}

#[tokio::test]
async fn wrap_personal_access_token_routes_passthrough_when_none() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use tower::ServiceExt;

    let base = Router::new().route("/v1/auth/personal-access-tokens", get(|| async { "ok" }));
    let router = apply_personal_access_token_routes_wrap(None, base);

    let response = router
        .oneshot(
            Request::builder()
                .uri("/v1/auth/personal-access-tokens")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get("x-eve-401-marker").is_none());
}
