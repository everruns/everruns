//! OAuth metadata discovery follows the MCP authorization spec's well-known
//! fallbacks (RFC 9728 protected-resource, RFC 8414 / OpenID Connect
//! authorization-server metadata).

use super::*;
use crate::api::user_connections::mcp_oauth::{
    authorization_server_issuer, discover_resource_metadata,
};

/// Egress that answers only the listed `(method, url)` routes, 404 for
/// everything else, and records every URL it was asked for.
#[derive(Default)]
pub(super) struct RoutedEgress {
    routes: BTreeMap<(&'static str, String), (u16, serde_json::Value)>,
    seen: std::sync::Mutex<Vec<String>>,
}

impl RoutedEgress {
    pub(super) fn route(
        mut self,
        method: &'static str,
        url: &str,
        body: serde_json::Value,
    ) -> Self {
        self.routes.insert((method, url.to_string()), (200, body));
        self
    }

    fn status(mut self, method: &'static str, url: &str, status: u16) -> Self {
        self.routes
            .insert((method, url.to_string()), (status, serde_json::Value::Null));
        self
    }

    fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl EgressService for RoutedEgress {
    async fn send(&self, request: EgressRequest) -> everruns_core::EgressResult<EgressResponse> {
        self.seen.lock().unwrap().push(request.url.clone());
        let (status, body) = self
            .routes
            .get(&(
                if request.method == "POST" {
                    "POST"
                } else {
                    "GET"
                },
                request.url.clone(),
            ))
            .cloned()
            .unwrap_or((404, serde_json::Value::Null));
        Ok(EgressResponse {
            status,
            headers: BTreeMap::new(),
            body: serde_json::to_vec(&body).unwrap(),
        })
    }

    async fn send_stream(
        &self,
        _request: EgressRequest,
    ) -> everruns_core::EgressResult<everruns_core::EgressStreamResponse> {
        panic!("streaming egress is not used by OAuth discovery")
    }
}

pub(super) fn as_metadata(issuer: &str) -> serde_json::Value {
    serde_json::json!({
        "issuer": issuer,
        "authorization_endpoint": "https://1.1.1.1/authorize",
        "token_endpoint": "https://1.1.1.1/token",
        "registration_endpoint": "https://1.1.1.1/register"
    })
}

// RFC 8414 §3.1: an issuer with a path publishes its metadata with the
// well-known segment inserted between host and path.
#[tokio::test]
async fn oauth_discovery_inserts_well_known_before_the_issuer_path() {
    let egress = RoutedEgress::default().route(
        "GET",
        "https://1.1.1.1/.well-known/oauth-authorization-server/tenant",
        as_metadata("https://1.1.1.1/tenant"),
    );

    let metadata = discover_oauth_server_metadata(&egress, "https://1.1.1.1/tenant")
        .await
        .expect("path-inserted metadata is found");

    assert_eq!(metadata.token_endpoint, "https://1.1.1.1/token");
}

// MCP authorization spec: an OpenID Connect provider may publish only
// `openid-configuration`.
#[tokio::test]
async fn oauth_discovery_falls_back_to_openid_configuration() {
    let egress = RoutedEgress::default().route(
        "GET",
        "https://1.1.1.1/.well-known/openid-configuration",
        as_metadata("https://1.1.1.1"),
    );

    let metadata = discover_oauth_server_metadata(&egress, "https://1.1.1.1")
        .await
        .expect("openid-configuration is accepted");

    assert_eq!(metadata.authorization_endpoint, "https://1.1.1.1/authorize");
    assert_eq!(
        egress.seen(),
        [
            "https://1.1.1.1/.well-known/oauth-authorization-server",
            "https://1.1.1.1/.well-known/openid-configuration",
        ]
    );
}

// OpenID Connect Discovery 1.0 appends the well-known segment to an issuer
// with a path (Keycloak realms, for example); it is the last candidate.
#[tokio::test]
async fn oauth_discovery_tries_every_candidate_for_an_issuer_with_a_path() {
    let egress = RoutedEgress::default().route(
        "GET",
        "https://1.1.1.1/realms/acme/.well-known/oauth-authorization-server",
        as_metadata("https://1.1.1.1/realms/acme"),
    );

    discover_oauth_server_metadata(&egress, "https://1.1.1.1/realms/acme")
        .await
        .expect("the pre-spec path-appended location is still accepted");

    assert_eq!(
        egress.seen(),
        [
            "https://1.1.1.1/.well-known/oauth-authorization-server/realms/acme",
            "https://1.1.1.1/.well-known/openid-configuration/realms/acme",
            "https://1.1.1.1/realms/acme/.well-known/openid-configuration",
            "https://1.1.1.1/realms/acme/.well-known/oauth-authorization-server",
        ]
    );
}

// Only "not published" moves on; any other failure is the answer.
#[tokio::test]
async fn oauth_discovery_stops_at_a_non_404_failure() {
    let egress = RoutedEgress::default()
        .status(
            "GET",
            "https://1.1.1.1/.well-known/oauth-authorization-server",
            503,
        )
        .route(
            "GET",
            "https://1.1.1.1/.well-known/openid-configuration",
            as_metadata("https://1.1.1.1"),
        );

    let error = discover_oauth_server_metadata(&egress, "https://1.1.1.1")
        .await
        .unwrap_err();

    assert_eq!(error.0, StatusCode::BAD_GATEWAY);
    assert_eq!(egress.seen().len(), 1);
}

// RFC 9728 §3.1: the `/mcp` endpoint's metadata sits at the path-inserted
// URL; the root document is only the fallback.
#[tokio::test]
async fn protected_resource_discovery_prefers_the_path_inserted_document() {
    let egress = RoutedEgress::default().route(
        "GET",
        "https://8.8.8.8/.well-known/oauth-protected-resource/mcp",
        serde_json::json!({ "authorization_servers": ["https://1.1.1.1"] }),
    );

    let metadata = discover_resource_metadata(&egress, "https://8.8.8.8/mcp?key=secret")
        .await
        .unwrap();

    assert_eq!(
        authorization_server_issuer(metadata.as_ref(), "https://8.8.8.8/mcp").unwrap(),
        "https://1.1.1.1"
    );
    // The query (which can carry a key) is never sent to discovery.
    assert_eq!(
        egress.seen(),
        ["https://8.8.8.8/.well-known/oauth-protected-resource/mcp"]
    );
}

#[tokio::test]
async fn protected_resource_discovery_falls_back_to_the_root_document() {
    let egress = RoutedEgress::default().route(
        "GET",
        "https://8.8.8.8/.well-known/oauth-protected-resource",
        serde_json::json!({ "authorization_servers": ["https://1.1.1.1"] }),
    );

    let metadata = discover_resource_metadata(&egress, "https://8.8.8.8/mcp")
        .await
        .unwrap();

    assert!(metadata.is_some());
    assert_eq!(egress.seen().len(), 2);
}

// Legacy MCP servers publish no protected-resource metadata; their origin
// is the authorization server.
#[tokio::test]
async fn without_protected_resource_metadata_the_origin_is_discovered() {
    let egress = RoutedEgress::default().route(
        "GET",
        "https://8.8.8.8/.well-known/oauth-authorization-server",
        serde_json::json!({
            "issuer": "https://8.8.8.8",
            "authorization_endpoint": "https://8.8.8.8/authorize",
            "token_endpoint": "https://8.8.8.8/token"
        }),
    );

    let metadata = discover_resource_metadata(&egress, "https://8.8.8.8/v1/mcp")
        .await
        .unwrap();
    assert!(metadata.is_none());
    let issuer = authorization_server_issuer(metadata.as_ref(), "https://8.8.8.8/v1/mcp").unwrap();
    assert_eq!(issuer, "https://8.8.8.8");
    let server = discover_oauth_server_metadata(&egress, &issuer)
        .await
        .expect("the origin's metadata is used");
    assert_eq!(server.token_endpoint, "https://8.8.8.8/token");
}

#[tokio::test]
async fn protected_resource_discovery_stops_at_a_non_404_failure() {
    let egress = RoutedEgress::default().status(
        "GET",
        "https://8.8.8.8/.well-known/oauth-protected-resource/mcp",
        500,
    );

    let error = discover_resource_metadata(&egress, "https://8.8.8.8/mcp")
        .await
        .unwrap_err();

    assert_eq!(error.0, StatusCode::BAD_GATEWAY);
    assert_eq!(egress.seen().len(), 1);
}

#[tokio::test]
async fn oauth_discovery_fails_when_no_candidate_is_published() {
    let egress = RoutedEgress::default();

    let error = discover_oauth_server_metadata(&egress, "https://1.1.1.1")
        .await
        .unwrap_err();

    assert_eq!(error.0, StatusCode::BAD_GATEWAY);
}
