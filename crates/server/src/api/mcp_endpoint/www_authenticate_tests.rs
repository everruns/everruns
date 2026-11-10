use super::*;
use axum::body::Body;
use axum::http::Request as HttpRequest;
use axum::response::IntoResponse;
use axum::routing::any;
use tower::ServiceExt;

const METADATA_URL: &str = "https://example.com/.well-known/oauth-protected-resource/mcp";

fn app_with_layer(status: StatusCode) -> Router {
    let header_value = parse_www_authenticate_value(METADATA_URL).expect("valid header");
    Router::new()
        .route("/mcp", any(move || async move { status.into_response() }))
        .layer(axum::middleware::from_fn(
            move |req: Request, next: Next| {
                let header_value = header_value.clone();
                async move { inject_www_authenticate(req, next, header_value).await }
            },
        ))
}

#[test]
fn header_value_matches_rfc9728_format() {
    let value = build_www_authenticate_value(METADATA_URL);
    assert_eq!(
        value,
        format!("Bearer realm=\"mcp\", resource_metadata=\"{METADATA_URL}\"")
    );
}

#[tokio::test]
async fn adds_header_on_401() {
    let app = app_with_layer(StatusCode::UNAUTHORIZED);
    let response = app
        .oneshot(
            HttpRequest::builder()
                .uri("/mcp")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let header = response
        .headers()
        .get(header::WWW_AUTHENTICATE)
        .expect("WWW-Authenticate header present on 401")
        .to_str()
        .unwrap();
    assert!(header.starts_with("Bearer realm=\"mcp\""));
    assert!(header.contains(&format!("resource_metadata=\"{METADATA_URL}\"")));
}

#[tokio::test]
async fn skips_header_on_non_401() {
    let app = app_with_layer(StatusCode::OK);
    let response = app
        .oneshot(
            HttpRequest::builder()
                .uri("/mcp")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get(header::WWW_AUTHENTICATE).is_none());
}

#[tokio::test]
async fn preserves_existing_header() {
    let header_value = parse_www_authenticate_value(METADATA_URL).expect("valid header");
    let app = Router::new()
        .route(
            "/mcp",
            any(|| async {
                let mut response = StatusCode::UNAUTHORIZED.into_response();
                response.headers_mut().insert(
                    header::WWW_AUTHENTICATE,
                    "Basic realm=\"x\"".parse().unwrap(),
                );
                response
            }),
        )
        .layer(axum::middleware::from_fn(
            move |req: Request, next: Next| {
                let header_value = header_value.clone();
                async move { inject_www_authenticate(req, next, header_value).await }
            },
        ));
    let response = app
        .oneshot(
            HttpRequest::builder()
                .uri("/mcp")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response
            .headers()
            .get(header::WWW_AUTHENTICATE)
            .unwrap()
            .to_str()
            .unwrap(),
        "Basic realm=\"x\""
    );
}
