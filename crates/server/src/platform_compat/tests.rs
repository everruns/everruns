#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use axum::{
    body::{Body, Bytes},
    extract::{MatchedPath, Path},
    http::{Method, StatusCode},
    middleware::{Next, from_fn},
    routing::get,
};
use http_body_util::BodyExt;

const PATHS: &[&str] = &[
    "/v1/auth/external/receiver",
    "/v1/platform/apps/{id}",
    "/v1/platform/user/connections",
    "/v1/user/connections/{provider}/callback",
    "/v1/platform/agents",
    "/v1/platform/agents/{id}",
    "/v1/platform/workspaces/{id}/fs/{path}",
    "/v1/platform/model-profiles/{key}",
    "/v1/platform/hosted/orgs",
    "/v1/platform/virtual-users/{id}",
    "/v1/virtual-users/me",
    "/v1/channels/{id}/sessions",
    "/v1/apps/{id}/webhooks/{channel}",
    "/v1/providers/{id}/oauth/callback",
    "/v1/platform/providers/{id}/oauth/authorize",
    "/v1/platform/sessions/{id}/sse",
];

async fn guard(request: Request, next: Next) -> Response {
    let Some(path) = request.extensions().get::<MatchedPath>() else {
        return next.run(request).await;
    };
    let path = path.as_str();
    assert!(path.contains("/v1/platform/"), "{path}");
    match request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
    {
        Some("Bearer full") => next.run(request).await,
        Some("Bearer read")
            if request.method() == Method::GET || request.method() == Method::HEAD =>
        {
            next.run(request).await
        }
        Some("Bearer read") => StatusCode::FORBIDDEN.into_response(),
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}

use axum::response::IntoResponse;
fn app(prefix: &str) -> Router {
    let managed = Router::new()
        .route("/v1/platform/apps/{id}", get(|| async { "managed app" }))
        .route(
            "/v1/platform/user/connections/{provider}/verify",
            axum::routing::post(|| async { "verified" }),
        )
        .route(
            "/v1/platform/agents",
            get(|| async { "agents" }).post(|request: Request| async {
                let uri = request
                    .extensions()
                    .get::<OriginalUri>()
                    .unwrap()
                    .0
                    .to_string();
                assert_eq!(request.headers()["idempotency-key"], "unchanged");
                let body = request.into_body().collect().await.unwrap().to_bytes();
                (
                    StatusCode::CREATED,
                    format!("{uri}|{}", String::from_utf8_lossy(&body)),
                )
            }),
        )
        .route(
            "/v1/platform/agents/{id}",
            get(|Path(id): Path<String>| async move { id }),
        )
        .route(
            "/v1/platform/workspaces/{id}/fs/{*path}",
            get(|request: Request| async move { request.uri().to_string() }),
        )
        .route(
            "/v1/platform/virtual-users/{id}",
            get(|| async { "management" }),
        )
        .route(
            "/v1/platform/model-profiles/{*key}",
            get(|request: Request| async move { request.uri().to_string() }),
        )
        .route(
            "/v1/platform/sessions/{id}/sse",
            get(|| async {
                let first = futures::stream::once(async {
                    Ok::<_, std::io::Error>(Bytes::from_static(b"data: first\n\n"))
                });
                use futures::StreamExt;
                Body::from_stream(first.chain(futures::stream::pending()))
            }),
        )
        .layer(from_fn(guard));
    let routes = managed
        .route(
            "/v1/apps/{id}/public-chat",
            get(|| async { "published chat" }),
        )
        .route(
            "/v1/user/connections/{provider}/callback",
            get(|| async { "connection callback" }),
        )
        .route("/v1/platform/auth/config", get(|| async { "auth" }))
        .route("/v1/virtual-users/me", get(|| async { "self" }))
        .route("/v1/virtual-users/{id}", get(|| async { "runtime" }))
        .route(
            "/v1/channels/{id}/sessions",
            get(|| async { "agent execution" }),
        )
        .route(
            "/v1/apps/{id}/webhooks/{channel}",
            get(|| async { "webhook" }),
        )
        .route("/v1/auth/callback/{provider}", get(|| async { "callback" }))
        .route("/v1/auth/external/receiver", get(|| async { "bridge" }))
        .route(
            "/v1/providers/{id}/oauth/callback",
            get(|| async { "provider callback" }),
        );
    let routes = if prefix.is_empty() {
        routes
    } else {
        Router::new().nest(prefix, routes)
    };
    with_legacy_routes(routes, PATHS.iter().map(|s| (*s).to_owned()), prefix)
}

fn request(method: &str, uri: &str, token: &str) -> Request {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("idempotency-key", "unchanged")
        .body(Body::from("payload"))
        .unwrap()
}

#[tokio::test]
async fn legacy_writes_preserve_method_body_query_and_canonical_original_uri() {
    for prefix in ["", "/api", "/custom-api", "/v1"] {
        let response = app(prefix)
            .oneshot(request(
                "POST",
                &format!("{prefix}/v1/agents?tag=a%2Fb"),
                "full",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(response.headers()["deprecation"], LEGACY_DEPRECATION);
        assert!(!response.headers().contains_key("location"));
        assert!(!response.headers().contains_key("sunset"));
        assert!(
            response.headers()["link"]
                .to_str()
                .unwrap()
                .contains(&format!("<{prefix}/v1/platform/agents?tag=a%2Fb>"))
        );
        assert_eq!(
            response.into_body().collect().await.unwrap().to_bytes(),
            format!("{prefix}/v1/platform/agents?tag=a%2Fb|payload")
        );
    }
}

#[tokio::test]
async fn legacy_and_canonical_share_authorization_and_method_handling() {
    for namespace in ["/v1", "/v1/platform"] {
        for (method, token, expected) in [
            ("GET", "read", 200),
            ("POST", "read", 403),
            ("GET", "invalid", 401),
            ("DELETE", "full", 405),
            ("HEAD", "full", 200),
        ] {
            let response = app("")
                .oneshot(request(method, &format!("{namespace}/agents"), token))
                .await
                .unwrap();
            assert_eq!(
                response.status().as_u16(),
                expected,
                "{namespace} {method} {token}"
            );
            assert_eq!(
                response.headers().contains_key("deprecation"),
                namespace == "/v1"
            );
        }
    }
}

#[tokio::test]
async fn parameters_and_encoded_files_keep_their_values() {
    let profile = app("")
        .oneshot(request("GET", "/v1/model-profiles/provider/model", "read"))
        .await
        .unwrap();
    assert_eq!(profile.status(), StatusCode::OK);
    assert_eq!(
        profile.into_body().collect().await.unwrap().to_bytes(),
        "/v1/platform/model-profiles/provider/model"
    );
    let response = app("")
        .oneshot(request("GET", "/v1/agents/agent_123", "full"))
        .await
        .unwrap();
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "agent_123"
    );
    let response = app("")
        .oneshot(request(
            "GET",
            "/v1/workspaces/ws_1/fs/a%20b/note.txt?download=1",
            "read",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "/v1/platform/workspaces/ws_1/fs/a%20b/note.txt?download=1"
    );
}

#[tokio::test]
async fn published_protocols_callbacks_and_self_service_are_not_deprecated() {
    for uri in [
        "/v1/apps/app_1/public-chat",
        "/v1/user/connections/example/callback",
        "/v1/channels/channel_1/sessions",
        "/v1/apps/app_1/webhooks/channel_1",
        "/v1/auth/callback/google",
        "/v1/auth/external/receiver",
        "/v1/providers/provider_1/oauth/callback",
        "/v1/virtual-users/me",
    ] {
        let response = app("").oneshot(request("GET", uri, "full")).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        assert!(!response.headers().contains_key("deprecation"), "{uri}");
    }
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(br#"{"token_type":"runtime_access"}"#);
    let response = app("")
        .oneshot(request(
            "GET",
            "/v1/virtual-users/person_1",
            &format!("header.{payload}.signature"),
        ))
        .await
        .unwrap();
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "runtime"
    );
    let response = app("")
        .oneshot(request("GET", "/v1/virtual-users/person_1", "full"))
        .await
        .unwrap();
    assert_eq!(response.headers()["deprecation"], LEGACY_DEPRECATION);
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "management"
    );
}

#[tokio::test]
async fn unknown_routes_and_canonical_auth_are_unchanged() {
    for (uri, expected, deprecated) in [
        ("/v1/unknown", 404, false),
        ("/v1/platform/auth/config", 200, false),
        ("/v1/auth/config", 200, true),
    ] {
        let response = app("").oneshot(request("GET", uri, "full")).await.unwrap();
        assert_eq!(response.status().as_u16(), expected);
        assert_eq!(response.headers().contains_key("deprecation"), deprecated);
    }
}

#[tokio::test]
async fn legacy_sse_is_forwarded_without_buffering() {
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        app("").oneshot(request("GET", "/v1/sessions/session_1/sse", "read")),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().contains_key("deprecation"));
    let mut body = response.into_body();
    let frame = tokio::time::timeout(std::time::Duration::from_secs(1), body.frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(frame.into_data().unwrap(), "data: first\n\n");
}

#[tokio::test]
async fn embedder_aliases_use_one_shared_rate_limit() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let count = Arc::new(AtomicUsize::new(0));
    let route = Router::new()
        .route(
            "/v1/platform/hosted/orgs",
            get(|| async { "orgs" }).post(|| async { "created" }),
        )
        .layer(from_fn(move |request: Request, next: Next| {
            let count = count.clone();
            async move {
                assert_eq!(request.uri().path(), "/v1/platform/hosted/orgs");
                if count.fetch_add(1, Ordering::SeqCst) >= 2 {
                    StatusCode::TOO_MANY_REQUESTS.into_response()
                } else {
                    next.run(request).await
                }
            }
        }));
    let router = with_legacy_routes(route, PATHS.iter().map(|s| (*s).to_owned()), "");
    for (uri, status) in [
        ("/v1/hosted/orgs", 200),
        ("/v1/platform/hosted/orgs", 200),
        ("/v1/hosted/orgs", 429),
    ] {
        assert_eq!(
            router
                .clone()
                .oneshot(request("POST", uri, "full"))
                .await
                .unwrap()
                .status()
                .as_u16(),
            status
        );
    }
}

#[tokio::test]
async fn embedder_namespaces_preserve_signed_receivers() {
    let routes = Router::new()
        .route("/v1/platform/ops/users", get(|| async { "operators" }))
        .route("/v1/platform/hosted/profile", get(|| async { "profile" }))
        .route(
            "/v1/hosted/billing/webhook",
            axum::routing::post(|| async { "signed receiver" }),
        );
    let router = with_legacy_routes(
        routes,
        [
            "/v1/platform/ops/{*path}",
            "/v1/platform/hosted/{*path}",
            "/v1/hosted/billing/webhook",
        ]
        .map(str::to_owned),
        "",
    );
    for (method, path, deprecated) in [
        ("GET", "/v1/ops/users", true),
        ("GET", "/v1/hosted/profile", true),
        ("POST", "/v1/hosted/billing/webhook", false),
    ] {
        let response = router
            .clone()
            .oneshot(request(method, path, "full"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(
            response.headers().contains_key("deprecation"),
            deprecated,
            "{path}"
        );
    }
}

#[tokio::test]
async fn root_observers_see_canonical_route_labels_for_both_prefixes() {
    let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let observations = observed.clone();
    let app = Router::new()
        .nest(
            "/api",
            Router::new().route("/v1/platform/agents", get(|| async { "ok" })),
        )
        .layer(from_fn(move |request: Request, next: Next| {
            let observed = observations.clone();
            async move {
                observed.lock().unwrap().push(
                    request
                        .extensions()
                        .get::<MatchedPath>()
                        .unwrap()
                        .as_str()
                        .to_owned(),
                );
                next.run(request).await
            }
        }));
    let app = with_legacy_routes(app, PATHS.iter().map(|s| (*s).to_owned()), "/api");
    for path in ["/api/v1/agents", "/api/v1/platform/agents"] {
        assert_eq!(
            app.clone()
                .oneshot(request("GET", path, "read"))
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
    }
    assert_eq!(
        *observed.lock().unwrap(),
        ["/api/v1/platform/agents", "/api/v1/platform/agents"]
    );
}

#[tokio::test]
async fn unlisted_management_operations_keep_the_same_namespace_permissions() {
    // The schema inventories the resource root, not this credential operation.
    for (token, expected) in [("full", StatusCode::OK), ("read", StatusCode::FORBIDDEN)] {
        let response = app("")
            .oneshot(request(
                "POST",
                "/v1/user/connections/example/verify",
                token,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        assert_eq!(response.headers()["deprecation"], LEGACY_DEPRECATION);
    }
}
