//! The URL handed to a data plane must reach the versioned route mounted by
//! Channels. Checking a generated URL against a router catches API-root drift.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use everruns_core::mcp::session_token::{SessionTokenSigner, SessionTokenVerifier};
use everruns_server::{
    channels::email::tools::EmailTools,
    injected::{InjectedCapabilities, routes},
    storage::StorageBackend,
};
use tower::ServiceExt;

#[tokio::test]
async fn api_root_and_versioned_root_reach_the_authenticated_email_endpoint() {
    let db = Arc::new(StorageBackend::test_database());
    for base in [
        "http://localhost/api",
        "http://localhost/api/",
        "http://localhost/api/v1/",
    ] {
        let signer = SessionTokenSigner::generate("https://dp.example").unwrap();
        let verifier =
            SessionTokenVerifier::from_jwks(&signer.jwks(), vec!["https://dp.example".into()])
                .unwrap();
        let state = InjectedCapabilities::new(base, Some(verifier))
            .register(Arc::new(EmailTools::new(db.clone(), base)));
        let url = state.definitions().remove(0).server.url;
        let router = Router::new().nest("/api", routes(Arc::new(state), "/v1"));
        let response = router
            .oneshot(
                Request::post(url.strip_prefix("http://localhost").unwrap())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // Missing auth reaches the capability and is refused, rather than
        // disappearing into a 404 at an unmounted discovery URL.
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{base}");
    }
}
