use super::*;
use crate::services::command_catalog::catalog::{ToolsetMode, scripted_descriptor};
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::{get, post};
use tower::ServiceExt;

// --- Test 1: scope parsing (Decision 1) -----------------------------------

#[test]
fn full_scope_parses_only_from_star() {
    assert_eq!(PatScope::parse(&["*"]), PatScope::Full);
    assert_eq!(PatScope::parse(&["*", "*"]), PatScope::Full);
}

#[test]
fn read_scope_parses_only_from_read() {
    assert_eq!(PatScope::parse(&["read"]), PatScope::Read);
    assert_eq!(PatScope::parse(&["read", "read"]), PatScope::Read);
}

#[test]
fn everything_else_parses_to_no_access() {
    let empty: [&str; 0] = [];
    assert_eq!(PatScope::parse(&empty), PatScope::NoAccess);
    assert_eq!(PatScope::parse(&["write"]), PatScope::NoAccess);
    assert_eq!(PatScope::parse(&["admin"]), PatScope::NoAccess);
    assert_eq!(PatScope::parse(&["READ"]), PatScope::NoAccess);
    assert_eq!(PatScope::parse(&[""]), PatScope::NoAccess);
    // Legacy pre-EVE-701 mixes are dropped, never honored as `read`.
    assert_eq!(PatScope::parse(&["read", "write"]), PatScope::NoAccess);
    // `*` mixed with anything else is not full access.
    assert_eq!(PatScope::parse(&["*", "read"]), PatScope::NoAccess);
    assert_eq!(PatScope::parse(&["read", "*"]), PatScope::NoAccess);
    // A reserved `<area>:<access>` scope is not shipped yet.
    assert_eq!(PatScope::parse(&["agents:read"]), PatScope::NoAccess);
}

#[test]
fn stored_scopes_parse_fail_closed() {
    use serde_json::json;
    assert_eq!(PatScope::parse_stored(&json!(["*"])), PatScope::Full);
    assert_eq!(PatScope::parse_stored(&json!(["read"])), PatScope::Read);
    assert_eq!(PatScope::parse_stored(&json!([])), PatScope::NoAccess);
    assert_eq!(PatScope::parse_stored(&json!(null)), PatScope::NoAccess);
    assert_eq!(PatScope::parse_stored(&json!("*")), PatScope::NoAccess);
    assert_eq!(PatScope::parse_stored(&json!(["*", 1])), PatScope::NoAccess);
    assert_eq!(
        PatScope::parse_stored(&json!({"*": true})),
        PatScope::NoAccess
    );
}

#[test]
fn permits_follows_the_class_table() {
    use RouteAccess::*;
    let classes = [Some(Read), Some(Write), Some(Never), Some(PerCommand), None];
    for class in classes {
        assert!(PatScope::Full.permits(class), "{class:?}");
        assert!(!PatScope::NoAccess.permits(class), "{class:?}");
    }
    assert!(PatScope::Read.permits(Some(Read)));
    assert!(PatScope::Read.permits(Some(PerCommand)));
    assert!(!PatScope::Read.permits(Some(Write)));
    assert!(!PatScope::Read.permits(Some(Never)));
    assert!(!PatScope::Read.permits(None), "unclassified fails closed");
}

// --- Classification -------------------------------------------------------

#[test]
fn normalize_template_erases_parameter_names() {
    assert_eq!(
        normalize_template("/v1/agents/{agent_id}/endpoints/{channel_id}"),
        "/v1/agents/{}/endpoints/{}"
    );
    assert_eq!(
        normalize_template("/v1/memories/{memory_id}/fs/{*path}"),
        normalize_template("/v1/memories/{memory_id}/fs/{path}")
    );
    assert_eq!(normalize_template("/v1/agents"), "/v1/agents");
}

#[test]
fn decision_4_routes_are_never() {
    let never = [
        ("GET", "/v1/users/me/export"),
        ("GET", "/v1/auth/personal-access-tokens"),
        ("POST", "/v1/auth/personal-access-tokens"),
        ("DELETE", "/v1/auth/personal-access-tokens/{token_id}"),
        ("POST", "/v1/auth/login"),
        ("GET", "/v1/auth/cli/callback"),
        ("POST", "/v1/users/me/switch-org"),
        ("DELETE", "/v1/users/me"),
        ("POST", "/v1/orgs"),
        ("POST", "/v1/me/invitations/{public_id}/accept"),
        ("POST", "/v1/agents/{agent_id}/channels/{channel_id}/keys"),
        (
            "POST",
            "/v1/agents/{agent_id}/channels/{channel_id}/keys/{key_id}/rotate",
        ),
        ("POST", "/v1/evals/{eval_id}/runs/{run_id}/share"),
        ("GET", "/v1/providers/{id}/oauth/authorize"),
        ("GET", "/v1/slack/install"),
        ("DELETE", "/v1/user/connected-clients/{grant_id}"),
        ("PUT", "/v1/sessions/{session_id}/storage/secrets"),
    ];
    for (method, path) in never {
        assert_eq!(
            classify(&Method::from_bytes(method.as_bytes()).unwrap(), path),
            Some(RouteAccess::Never),
            "{method} {path}"
        );
    }
}

#[test]
fn auth_me_is_read_and_commands_route_is_per_command() {
    assert_eq!(
        classify(&Method::GET, "/v1/auth/me"),
        Some(RouteAccess::Read)
    );
    assert_eq!(
        classify(&Method::HEAD, "/v1/auth/me"),
        Some(RouteAccess::Read),
        "HEAD is served by the GET handler"
    );
    assert_eq!(
        classify(&Method::POST, "/v1/commands/{name}"),
        Some(RouteAccess::PerCommand)
    );
    assert_eq!(
        classify(&Method::GET, "/v1/commands"),
        Some(RouteAccess::Read)
    );
}

#[test]
fn unknown_routes_are_unclassified() {
    assert_eq!(classify(&Method::GET, "/v1/not-a-route"), None);
    assert_eq!(classify(&Method::GET, "/mcp"), None);
}

// --- Test 5: predicate parity ---------------------------------------------

/// The class of every command-backed route equals membership in the MCP
/// `query` (`ToolsetMode::ReadOnly`) toolset, so PAT scopes and MCP `query`
/// cannot drift apart. The only exception is a Decision 4 `Never` override,
/// which must be a reviewed row of the table.
#[test]
fn command_backed_routes_match_the_read_only_query_toolset() {
    let names: Vec<&str> = crate::api::command_http::http_command_names().collect();
    assert!(!names.is_empty(), "no command-backed routes registered");
    for name in names {
        let desc = inventory::iter::<CommandDescriptor>
            .into_iter()
            .find(|desc| (desc.meta)().name == name)
            .unwrap_or_else(|| panic!("http command {name} has no descriptor"));
        let meta = (desc.meta)();
        let method = Method::from_bytes(meta.method.as_bytes()).unwrap();
        let class = classify(&method, meta.path);
        assert_eq!(class, Some(classify_command(desc)), "{name}");
        if class == Some(RouteAccess::Never) {
            assert!(
                routes::ROUTES
                    .iter()
                    .chain(routes::UNDOCUMENTED_ROUTES)
                    .any(|(m, p, access)| {
                        *access == RouteAccess::Never
                            && route_key(m, p) == route_key(meta.method, meta.path)
                    }),
                "{name} is Never without a table row"
            );
            continue;
        }
        let in_query = scripted_descriptor(name, ToolsetMode::ReadOnly).is_some();
        let in_scripting = scripted_descriptor(name, ToolsetMode::Full).is_some();
        let read = class == Some(RouteAccess::Read);
        if in_scripting {
            assert_eq!(
                read, in_query,
                "{name}: PAT class {class:?} disagrees with MCP query membership"
            );
        } else {
            // Not a scripting command at all (durable control plane, internal
            // worker plumbing): the same predicate still decides.
            assert_eq!(
                read,
                crate::services::command_catalog::catalog::is_read_only_command(desc),
                "{name}"
            );
        }
    }
}

// --- The layer ------------------------------------------------------------

async fn seen_access(app: Router, method: &str, uri: &str) -> String {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    String::from_utf8(body.to_vec()).unwrap()
}

fn echo_routes() -> Router {
    async fn echo(access: Option<axum::Extension<RouteAccess>>) -> String {
        format!("{:?}", access.map(|axum::Extension(access)| access))
    }
    Router::new()
        .route("/v1/auth/me", get(echo))
        .route("/v1/users/me/export", get(echo))
        .route("/v1/commands/{name}", post(echo))
        .route("/v1/unlisted", get(echo))
}

#[tokio::test]
async fn layer_inserts_the_class_of_the_matched_route() {
    for prefix in ["", "/api"] {
        let routes = classify_api_routes(echo_routes(), prefix);
        let app = crate::server::build_router_with_prefix(routes, prefix);
        let cases = [
            ("GET", "/v1/auth/me", "Some(Read)"),
            ("GET", "/v1/users/me/export", "Some(Never)"),
            ("POST", "/v1/commands/list_agents", "Some(PerCommand)"),
            ("GET", "/v1/unlisted", "None"),
        ];
        for (method, path, want) in cases {
            let uri = format!("{prefix}{path}");
            assert_eq!(seen_access(app.clone(), method, &uri).await, want, "{uri}");
        }
    }
}
