// HTTP Message Signatures Directory endpoint (Web Bot Auth key directory)
//
// Decision: /.well-known/http-message-signatures-directory is public (no auth).
//   Target servers need to discover the deployment's public keys without
//   credentials, and must not meet a bot challenge doing so (draft
//   appendix C.2).
// Decision: the keys come from the same process-wide signer the egress
//   boundary signs with (`everruns_core::host::WebBotAuth::from_env`), so the
//   directory always lists exactly the keys that sign, plus any rotation keys.
// Decision: each response is signed per key over `@authority;req` and
//   `content-digest` (tag `http-message-signatures-directory`). Cloudflare
//   requires this signed directory to register a bot, and it stops the key
//   set from being re-served under another host. `@authority` is the Host
//   the request arrived with; proxies in front of the server preserve Host.
// Decision: served with the draft's media type, a one-day cache lifetime, and
//   `Access-Control-Allow-Origin: *` for browser-based verifiers.
// See: draft-ietf-webbotauth-httpsig-protocol-01, sections 5.5 and appendix B.

use axum::{
    Router,
    extract::State,
    http::{HeaderMap, HeaderName, HeaderValue, header},
    response::{IntoResponse, Response},
    routing::get,
};
use everruns_core::host::WebBotAuth;
use everruns_core::host::web_bot_auth::{
    DIRECTORY_MAX_AGE_SECS, DIRECTORY_MEDIA_TYPE, DIRECTORY_PATH,
};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    /// `None` when Web Bot Auth is not configured: the directory is empty.
    pub signer: Option<Arc<WebBotAuth>>,
}

impl AppState {
    pub fn from_env() -> Self {
        Self {
            signer: WebBotAuth::from_env(),
        }
    }
}

pub fn routes(state: AppState) -> Router {
    Router::new().route(DIRECTORY_PATH, get(signatures_directory).with_state(state))
}

async fn signatures_directory(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(DIRECTORY_MEDIA_TYPE),
    );
    response_headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    if let Ok(value) = HeaderValue::from_str(&format!("public, max-age={DIRECTORY_MAX_AGE_SECS}")) {
        response_headers.insert(header::CACHE_CONTROL, value);
    }

    let Some(signer) = state.signer else {
        return (response_headers, WebBotAuth::empty_directory_body()).into_response();
    };
    let authority = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .filter(|host| is_plain_authority(host));
    let Some(authority) = authority else {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "missing or invalid Host header",
        )
            .into_response();
    };
    match signer.directory(authority) {
        Ok(directory) => {
            for (name, value) in directory.headers {
                if let Ok(value) = HeaderValue::from_str(&value) {
                    response_headers.insert(HeaderName::from_static(name), value);
                }
            }
            (response_headers, directory.body).into_response()
        }
        Err(error) => {
            tracing::warn!(%error, "could not sign the key directory");
            (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "key directory unavailable",
            )
                .into_response()
        }
    }
}

/// A Host value safe to put into a signature base: host[:port], no spaces or
/// control characters.
fn is_plain_authority(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 255
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':' | b'[' | b']'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use tower::ServiceExt;

    const TEST_SEED: &str = "n4Ni-HpISpVObnQMW0wOhCKROaIKqKtW_2ZYb2p9KcU";

    async fn get(state: AppState, host: Option<&str>) -> Response {
        let mut request = Request::builder().uri(DIRECTORY_PATH);
        if let Some(host) = host {
            request = request.header(header::HOST, host);
        }
        routes(state)
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    fn signer() -> Option<Arc<WebBotAuth>> {
        Some(Arc::new(
            WebBotAuth::new(TEST_SEED, "https://app.example.com").unwrap(),
        ))
    }

    #[tokio::test]
    async fn unconfigured_directory_is_empty_with_the_draft_media_type() {
        let response = get(AppState { signer: None }, Some("app.example.com")).await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            DIRECTORY_MEDIA_TYPE
        );
        assert!(response.headers().get("signature").is_none());
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(&body[..], br#"{"keys":[]}"#);
    }

    #[tokio::test]
    async fn configured_directory_is_signed_for_the_requested_host() {
        let response = get(AppState { signer: signer() }, Some("App.Example.com")).await;
        assert_eq!(response.status(), 200);
        let headers = response.headers().clone();
        assert_eq!(headers[header::CONTENT_TYPE], DIRECTORY_MEDIA_TYPE);
        assert_eq!(headers[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
        assert_eq!(headers[header::CACHE_CONTROL], "public, max-age=86400");
        let input = headers["signature-input"].to_str().unwrap();
        assert!(input.starts_with("binding0=(\"@authority\";req \"content-digest\");"));
        assert!(input.contains("tag=\"http-message-signatures-directory\""));
        assert!(
            headers["signature"]
                .to_str()
                .unwrap()
                .starts_with("binding0=:")
        );
        assert!(
            headers["content-digest"]
                .to_str()
                .unwrap()
                .starts_with("sha-256=:")
        );
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let jwks: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            jwks["keys"][0]["kid"],
            "poqkLGiymh_W0uP6PZFw-dvez3QJT5SolqXBCW38r0U"
        );
        assert!(
            jwks["keys"][0].get("d").is_none(),
            "never publish the private key"
        );
    }

    #[tokio::test]
    async fn a_hostile_host_header_is_refused() {
        for host in ["a b", "evil.test/path", "user@evil.test", ""] {
            let response = get(AppState { signer: signer() }, Some(host)).await;
            assert_eq!(response.status(), 400, "{host:?}");
        }
    }
}
