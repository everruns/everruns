// Agent discovery endpoints: MCP Server Card and auth.md.
//
// Decision: both are public (no auth). An agent that has no credentials yet is
//   exactly the caller that needs to read them.
// Decision: every value is derived from live server config (issuer URL, auth
//   mode, MCP server name/version) rather than hardcoded, so a self-hosted
//   deployment describes itself correctly and cannot drift from the running
//   service.
// Decision: when the server is not actually protecting anything (AuthMode::None,
//   local development) the documents say so instead of advertising an OAuth
//   flow that is not enforced. Publishing discovery metadata for a capability
//   that does not exist sends agents at endpoints that will reject them.
//
// See: SEP-1649 (MCP Server Card), https://isitagentready.com/ authMd check.

use axum::{
    Json, Router,
    http::header,
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::{Value, json};

use crate::auth::config::AuthMode;

#[derive(Clone)]
pub struct AppState {
    /// Server root, e.g. `https://app.everruns.com`. Also the OAuth issuer.
    pub root_url: String,
    /// API base, e.g. `https://app.everruns.com/api`.
    pub api_base_url: String,
    /// Drives whether the documents describe OAuth and personal access tokens
    /// as available.
    pub auth_mode: AuthMode,
    /// MCP server identity, mirroring what `initialize` reports.
    pub mcp_server_name: String,
    pub mcp_server_version: String,
}

impl AppState {
    pub fn new(
        root_url: impl Into<String>,
        api_base_url: impl Into<String>,
        auth_mode: AuthMode,
        mcp_server_name: impl Into<String>,
        mcp_server_version: impl Into<String>,
    ) -> Self {
        Self {
            root_url: root_url.into().trim_end_matches('/').to_string(),
            api_base_url: api_base_url.into().trim_end_matches('/').to_string(),
            auth_mode,
            mcp_server_name: mcp_server_name.into(),
            mcp_server_version: mcp_server_version.into(),
        }
    }

    /// True when credentials are actually enforced, so the discovery documents
    /// should describe how to obtain them.
    fn auth_enforced(&self) -> bool {
        self.auth_mode != AuthMode::None
    }
}

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/.well-known/mcp/server-card.json", get(mcp_server_card))
        .route("/auth.md", get(auth_md))
        .with_state(state)
}

/// JSON with permissive CORS. Discovery documents are fetched cross-origin by
/// browser-based agents, and they contain nothing sensitive.
fn json_public(body: Value) -> Response {
    ([(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")], Json(body)).into_response()
}

/// GET /.well-known/mcp/server-card.json — MCP Server Card (SEP-1649).
///
/// Describes the MCP endpoint this server exposes so a client can decide
/// whether to connect before running an OAuth flow.
async fn mcp_server_card(axum::extract::State(state): axum::extract::State<AppState>) -> Response {
    let root = &state.root_url;

    let mut card = json!({
        "serverInfo": {
            "name": state.mcp_server_name,
            "version": state.mcp_server_version,
        },
        "endpoint": format!("{root}/mcp"),
        "transport": "streamable-http",
        // Mirrors the capabilities advertised by `initialize` in
        // api/mcp_endpoint: tools and resources, no prompts.
        "capabilities": {
            "tools": { "listChanged": false },
            "resources": {},
        },
    });

    if state.auth_enforced() {
        card["authentication"] = json!({
            "type": "oauth2",
            "protectedResourceMetadata":
                format!("{root}/.well-known/oauth-protected-resource/mcp"),
            "authorizationServerMetadata":
                format!("{root}/.well-known/oauth-authorization-server"),
            "registrationEndpoint": format!("{root}/oauth/register"),
            "scopesSupported": ["mcp"],
            "documentation": format!("{root}/auth.md"),
        });
    } else {
        card["authentication"] = json!({ "type": "none" });
    }

    json_public(card)
}

/// Markdown with permissive CORS.
fn markdown_public(body: String) -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/markdown; charset=utf-8"),
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
        ],
        body,
    )
        .into_response()
}

/// GET /auth.md — how an agent obtains credentials for this server.
async fn auth_md(axum::extract::State(state): axum::extract::State<AppState>) -> Response {
    markdown_public(render_auth_md(&state))
}

fn render_auth_md(state: &AppState) -> String {
    let root = &state.root_url;
    let api = &state.api_base_url;

    if !state.auth_enforced() {
        return format!(
            "# auth.md\n\n\
             How an agent obtains credentials for this Everruns server.\n\n\
             ## No credentials required\n\n\
             This server runs with authentication disabled (`AUTH_MODE=none`), \
             a local-development mode in which every request is treated as an \
             administrator. Call the API at `{api}` and the MCP endpoint at \
             `{root}/mcp` without any `Authorization` header.\n\n\
             Do not expose a server in this mode to a network you do not control.\n\n\
             Documentation: <https://docs.everruns.com/>\n"
        );
    }

    format!(
        "# auth.md\n\n\
         How an AI agent obtains credentials for this Everruns server.\n\n\
         Everruns is a durable agent platform. Credentials below are bound to a \
         human-owned account; there is no anonymous access.\n\n\
         ## Method 1: MCP over OAuth 2.1 (preferred for agents)\n\n\
         The MCP endpoint supports OAuth dynamic client registration, so a client \
         can register itself without a human pre-provisioning it.\n\n\
         | Field | Value |\n\
         | --- | --- |\n\
         | MCP endpoint (resource) | `{root}/mcp` |\n\
         | Protected resource metadata | `{root}/.well-known/oauth-protected-resource/mcp` |\n\
         | Authorization server metadata | `{root}/.well-known/oauth-authorization-server` |\n\
         | Issuer | `{root}` |\n\
         | Registration endpoint (RFC 7591) | `{root}/oauth/register` |\n\
         | PKCE | required, `S256` |\n\
         | Scopes | `mcp` |\n\
         | Credential presentation | `Authorization: Bearer <access_token>` |\n\n\
         Fetch the protected resource metadata first, then the authorization \
         server metadata, and use the endpoints they return rather than the table \
         above. Register a client, run the authorization code flow with PKCE (a \
         human approves in a browser), then call `{root}/mcp` with the access \
         token.\n\n\
         Access tokens issued by this flow are audience-restricted to `{root}/mcp`. \
         They are **not** accepted by the REST API; see method 2 for that.\n\n\
         ## Method 2: Personal access token (REST API)\n\n\
         For server-to-server use where no browser is available, a human issues a \
         personal access token from the operator console under \
         **Settings -> Personal access tokens** and hands it to the agent out of \
         band.\n\n\
         | Field | Value |\n\
         | --- | --- |\n\
         | API base | `{api}` |\n\
         | Token format | opaque string prefixed `evr_pat_` |\n\
         | Credential presentation | `Authorization: Bearer evr_pat_...` |\n\n\
         ```bash\n\
         curl -X POST {api}/v1/agents \\\n  \
         -H \"Authorization: Bearer $EVERRUNS_TOKEN\" \\\n  \
         -H \"Content-Type: application/json\" \\\n  \
         -d '{{\"name\":\"Assistant\",\"system_prompt\":\"You are helpful.\"}}'\n\
         ```\n\n\
         A personal access token carries the full authority of the account that \
         issued it. Store it as a secret, never in source control, and rotate it \
         from the screen that issued it.\n\n\
         ## Credential handling\n\n\
         - Send credentials only over TLS, and only to the origin that issued them.\n\
         - Present tokens in the `Authorization` header, never in a query string.\n\
         - Refresh access tokens rather than re-running the authorization flow.\n\
         - On `401`, re-read the protected resource metadata before retrying: \
         endpoints and scopes can change.\n\n\
         ## Related discovery documents\n\n\
         - <{root}/.well-known/mcp/server-card.json> - MCP server card\n\
         - <{root}/.well-known/oauth-authorization-server> - authorization server metadata\n\
         - <https://docs.everruns.com/> - full documentation\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(mode: AuthMode) -> AppState {
        AppState::new(
            "https://app.example.com/",
            "https://app.example.com/api/",
            mode,
            "everruns",
            "0.25.0",
        )
    }

    #[test]
    fn trailing_slashes_are_trimmed() {
        let s = state(AuthMode::Full);
        assert_eq!(s.root_url, "https://app.example.com");
        assert_eq!(s.api_base_url, "https://app.example.com/api");
    }

    #[test]
    fn auth_md_documents_both_credential_paths_when_enforced() {
        let md = render_auth_md(&state(AuthMode::External));
        assert!(md.starts_with("# auth.md"), "H1 must contain auth.md");
        assert!(md.contains("https://app.example.com/mcp"));
        assert!(md.contains("/.well-known/oauth-protected-resource/mcp"));
        assert!(md.contains("evr_pat_"));
        assert!(md.contains("https://app.example.com/api/v1/agents"));
    }

    #[test]
    fn auth_md_states_that_mcp_tokens_do_not_reach_the_rest_api() {
        // MCP access tokens are audience-bound to `{root}/mcp` and typed
        // `mcp_access`; the REST API rejects them. An agent that assumes
        // otherwise burns a full OAuth flow before finding out.
        let md = render_auth_md(&state(AuthMode::Full));
        assert!(md.contains("not** accepted by the REST API"));
    }

    #[test]
    fn auth_md_does_not_advertise_oauth_when_auth_is_disabled() {
        let md = render_auth_md(&state(AuthMode::None));
        assert!(md.contains("No credentials required"));
        assert!(!md.contains("oauth-protected-resource"));
        assert!(!md.contains("evr_pat_"));
    }

    #[tokio::test]
    async fn server_card_reports_real_identity_and_capabilities() {
        let body = card_json(AuthMode::External).await;
        assert_eq!(body["serverInfo"]["name"], "everruns");
        assert_eq!(body["serverInfo"]["version"], "0.25.0");
        assert_eq!(body["endpoint"], "https://app.example.com/mcp");
        assert_eq!(body["capabilities"]["tools"]["listChanged"], false);
        assert!(body["capabilities"].get("prompts").is_none());
        assert_eq!(body["authentication"]["type"], "oauth2");
    }

    #[tokio::test]
    async fn server_card_reports_no_auth_when_auth_is_disabled() {
        let body = card_json(AuthMode::None).await;
        assert_eq!(body["authentication"]["type"], "none");
    }

    async fn card_json(mode: AuthMode) -> Value {
        let response = mcp_server_card(axum::extract::State(state(mode))).await;
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        serde_json::from_slice(&bytes).expect("json")
    }
}
