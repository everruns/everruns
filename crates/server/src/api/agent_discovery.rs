// Agent discovery endpoints: MCP Server Card, auth.md, and the site-level
// documents an agent reads when it only knows the origin (robots.txt, llms.txt,
// sitemap.xml, RFC 9727 API catalog, root RFC 9728 protected resource metadata).
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
// Decision: the origin root is the UI, which renders nothing without
//   JavaScript. These documents are the agent-facing front door instead; the
//   reverse proxy routes them here, adds `Link` headers on `/`, and answers
//   `Accept: text/markdown` on `/` with llms.txt (see local/Caddyfile).
// Decision: the root `/.well-known/oauth-protected-resource` repeats the
//   path-derived `/mcp` document (RFC 9728 §3.1). The only protected resource
//   is `{root}/mcp`, and scanners and some clients probe the bare path first.
//   It is not an `agent_auth` (auth.md registration) advertisement: accounts
//   are human-owned, so there is no agent self-registration to describe.
//
// See: SEP-1649 (MCP Server Card), RFC 9727 (API catalog), RFC 9728 (protected
//   resource metadata), https://llmstxt.org/, https://isitagentready.com/.

use axum::{
    Json, Router,
    http::header,
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::{Value, json};

use crate::api::mcp_endpoint::discovery::{MCP_ICON_PATH, server_info};
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

    /// Discovery documents for this process. Name and version are the running
    /// server's identity, the same value `initialize` reports.
    pub fn for_deployment(
        root_url: impl Into<String>,
        api_base_url: impl Into<String>,
        auth_mode: AuthMode,
    ) -> Self {
        Self::new(
            root_url,
            api_base_url,
            auth_mode,
            crate::api::mcp_endpoint::MCP_SERVER_NAME,
            crate::api::mcp_endpoint::MCP_SERVER_VERSION,
        )
    }
}

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/.well-known/mcp/server-card.json", get(mcp_server_card))
        .route("/auth.md", get(auth_md))
        .route(MCP_ICON_PATH, get(mcp_icon))
        .route("/llms.txt", get(llms_txt))
        .route("/robots.txt", get(robots_txt))
        .route("/sitemap.xml", get(sitemap_xml))
        .route("/.well-known/api-catalog", get(api_catalog))
        .route(
            "/.well-known/oauth-protected-resource",
            get(protected_resource_metadata),
        )
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

    // `version` and `serverInfo.version` are the same implementation version.
    // Catalogs read one or the other; a single source keeps them from drifting.
    let mut card = json!({
        "version": state.mcp_server_version,
        "serverInfo": {
            "name": state.mcp_server_name,
            "version": state.mcp_server_version,
            "title": "Everruns",
            "description": PRODUCT_SUMMARY,
            "websiteUrl": PRODUCT_SITE,
            "documentationUrl": DOCS_URL,
            "icons": server_info(Some(root))["icons"],
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

/// Text document with permissive CORS.
fn text_public(content_type: &'static str, body: String) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
        ],
        body,
    )
        .into_response()
}

fn markdown_public(body: String) -> Response {
    text_public("text/markdown; charset=utf-8", body)
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
         | MCP endpoint (resource) | <{root}/mcp> |\n\
         | Protected resource metadata | <{root}/.well-known/oauth-protected-resource/mcp> |\n\
         | Authorization server metadata | <{root}/.well-known/oauth-authorization-server> |\n\
         | Issuer | <{root}> |\n\
         | Registration endpoint (RFC 7591) | <{root}/oauth/register> |\n\
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
         | API base | <{api}> |\n\
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
         - <{root}/llms.txt> - what this server offers, for agents\n\
         - <https://docs.everruns.com/> - full documentation\n"
    )
}

const PRODUCT_SITE: &str = "https://everruns.com";
const DOCS_URL: &str = "https://docs.everruns.com/";
const API_DOCS_URL: &str = "https://docs.everruns.com/api/";
const PRODUCT_SUMMARY: &str = "Durable AI agent platform. Create and run agents whose sessions \
     survive crashes, restarts, and worker loss.";

/// GET /.well-known/mcp/icon.png — the icon `serverInfo.icons` points at.
async fn mcp_icon() -> Response {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
        ],
        include_bytes!("../../assets/mcp-icon.png").as_slice(),
    )
        .into_response()
}

/// GET /llms.txt — the agent-facing front door (https://llmstxt.org/).
///
/// Also what the proxy returns for `GET /` with `Accept: text/markdown`, so an
/// agent that only knows the origin learns where the MCP endpoint is.
async fn llms_txt(axum::extract::State(state): axum::extract::State<AppState>) -> Response {
    markdown_public(render_llms_txt(&state))
}

fn render_llms_txt(state: &AppState) -> String {
    let root = &state.root_url;
    let api = &state.api_base_url;
    let auth = if state.auth_enforced() {
        "OAuth 2.1 with dynamic client registration: a standard MCP client \
         registers itself, a human approves once in the browser, and the client \
         receives a token. The REST API takes a personal access token instead."
    } else {
        "Authentication is disabled on this server (local development): call \
         the MCP endpoint and the API without credentials."
    };
    format!(
        "# Everruns\n\n\
         > {PRODUCT_SUMMARY} This server, <{root}>, hosts the Everruns API and \
         an MCP endpoint. The web console at the root needs a browser; agents \
         use the endpoints below.\n\n\
         ## Connect an agent\n\n\
         - [MCP endpoint]({root}/mcp): streamable HTTP. Add this URL as a remote \
         MCP server in Claude, ChatGPT, Cursor, or any MCP client. {auth}\n\
         - [auth.md]({root}/auth.md): how to obtain credentials, step by step\n\
         - [MCP server card]({root}/.well-known/mcp/server-card.json): transport, \
         capabilities, and how to authenticate\n\n\
         ## API\n\n\
         - [REST API]({api}): agents, sessions, messages, events\n\
         - [OpenAPI document]({root}/api-doc/openapi.json)\n\
         - [API catalog]({root}/.well-known/api-catalog): RFC 9727 linkset\n\n\
         ## Docs\n\n\
         - [Documentation]({DOCS_URL})\n\
         - [API reference]({API_DOCS_URL})\n\
         - [Everruns]({PRODUCT_SITE})\n"
    )
}

/// GET /robots.txt — keep crawlers out of the console, let them read the
/// agent documents. Named AI crawlers are listed so their rules are explicit.
async fn robots_txt(axum::extract::State(state): axum::extract::State<AppState>) -> Response {
    let root = &state.root_url;
    let user_agents = [
        "*",
        "GPTBot",
        "OAI-SearchBot",
        "ChatGPT-User",
        "ClaudeBot",
        "Claude-SearchBot",
        "Claude-User",
        "PerplexityBot",
        "Google-Extended",
    ]
    .map(|agent| format!("User-agent: {agent}\n"))
    .concat();
    text_public(
        "text/plain; charset=utf-8",
        format!(
            "# The console needs a signed-in browser. Agents start at /llms.txt.\n\
             {user_agents}\
             Content-Signal: search=yes, ai-input=yes, ai-train=no\n\
             Allow: /llms.txt\n\
             Allow: /auth.md\n\
             Allow: /.well-known/\n\
             Allow: /api-doc/openapi.json\n\
             Disallow: /\n\n\
             Sitemap: {root}/sitemap.xml\n"
        ),
    )
}

/// GET /sitemap.xml — the public, crawlable documents on this origin.
async fn sitemap_xml(axum::extract::State(state): axum::extract::State<AppState>) -> Response {
    let urls = ["/llms.txt", "/auth.md"]
        .map(|path| format!("  <url><loc>{}{path}</loc></url>\n", state.root_url))
        .concat();
    text_public(
        "application/xml; charset=utf-8",
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n\
             {urls}</urlset>\n"
        ),
    )
}

/// GET /.well-known/api-catalog — RFC 9727 linkset of the APIs on this origin:
/// the REST API and the MCP endpoint.
async fn api_catalog(axum::extract::State(state): axum::extract::State<AppState>) -> Response {
    let root = &state.root_url;
    let body = json!({
        "linkset": [
            {
                "anchor": state.api_base_url,
                "service-desc": [{
                    "href": format!("{root}/api-doc/openapi.json"),
                    "type": "application/vnd.oai.openapi+json",
                }],
                "service-doc": [{ "href": API_DOCS_URL, "type": "text/html" }],
                "status": [{ "href": format!("{root}/health"), "type": "application/json" }],
            },
            {
                "anchor": format!("{root}/mcp"),
                "service-desc": [{
                    "href": format!("{root}/.well-known/mcp/server-card.json"),
                    "type": "application/json",
                }],
                "service-doc": [{ "href": format!("{root}/auth.md"), "type": "text/markdown" }],
            },
        ]
    });
    (
        [
            (
                header::CONTENT_TYPE,
                "application/linkset+json; profile=\"https://www.rfc-editor.org/info/rfc9727\"",
            ),
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
        ],
        body.to_string(),
    )
        .into_response()
}

/// GET /.well-known/oauth-protected-resource — root alias of the path-derived
/// `/.well-known/oauth-protected-resource/mcp` served by `auth::mcp_oauth`.
/// The fields must match that document; the test below pins them.
async fn protected_resource_metadata(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Response {
    if !state.auth_enforced() {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    }
    let root = &state.root_url;
    json_public(json!({
        "resource": format!("{root}/mcp"),
        "authorization_servers": [root],
        "bearer_methods_supported": ["header"],
        "scopes_supported": ["mcp"],
    }))
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
            "9.9.9",
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
        // A sentinel, not the package version: this proves the card copies the
        // configured identity instead of embedding its own literal.
        assert_eq!(body["version"], "9.9.9");
        assert_eq!(body["serverInfo"]["version"], "9.9.9");
        assert_eq!(body["endpoint"], "https://app.example.com/mcp");
        assert_eq!(body["capabilities"]["tools"]["listChanged"], false);
        assert!(body["capabilities"].get("prompts").is_none());
        assert_eq!(body["authentication"]["type"], "oauth2");
    }

    #[tokio::test]
    async fn server_card_reports_the_deployed_package_version() {
        let state = AppState::for_deployment(
            "https://app.example.com",
            "https://app.example.com/api",
            AuthMode::External,
        );
        let response = mcp_server_card(axum::extract::State(state)).await;
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let body: Value = serde_json::from_slice(&bytes).expect("json");
        let package_version = env!("CARGO_PKG_VERSION");
        assert_eq!(body["version"], package_version);
        assert_eq!(body["serverInfo"]["name"], "everruns");
        assert_eq!(body["serverInfo"]["version"], package_version);
    }

    #[tokio::test]
    async fn server_card_reports_no_auth_when_auth_is_disabled() {
        let body = card_json(AuthMode::None).await;
        assert_eq!(body["authentication"]["type"], "none");
    }

    #[test]
    fn auth_md_table_urls_are_autolinks_not_code_spans() {
        // A scanner that extracts URLs from the table read
        // `.../oauth-protected-resource/mcp` with the closing backtick attached.
        let md = render_auth_md(&state(AuthMode::External));
        assert!(
            md.contains("| <https://app.example.com/.well-known/oauth-protected-resource/mcp> |")
        );
        assert!(!md.contains("`https://app.example.com/.well-known/"));
    }

    #[test]
    fn llms_txt_points_agents_at_the_mcp_endpoint() {
        let md = render_llms_txt(&state(AuthMode::External));
        assert!(md.starts_with("# Everruns\n\n> "));
        assert!(md.contains("[MCP endpoint](https://app.example.com/mcp)"));
        assert!(md.contains("[auth.md](https://app.example.com/auth.md)"));
        assert!(md.contains("https://app.example.com/api-doc/openapi.json"));
        assert!(md.contains("dynamic client registration"));
    }

    #[test]
    fn llms_txt_does_not_advertise_oauth_when_auth_is_disabled() {
        let md = render_llms_txt(&state(AuthMode::None));
        assert!(md.contains("Authentication is disabled"));
        assert!(!md.contains("OAuth"));
    }

    #[tokio::test]
    async fn robots_txt_allows_agent_documents_and_names_ai_crawlers() {
        let body = body_text(robots_txt(axum::extract::State(state(AuthMode::Full))).await).await;
        assert!(body.contains("User-agent: GPTBot\n"));
        assert!(body.contains("User-agent: ClaudeBot\n"));
        assert!(body.contains("Content-Signal: search=yes, ai-input=yes, ai-train=no\n"));
        assert!(body.contains("Allow: /llms.txt\n"));
        assert!(body.contains("Allow: /.well-known/\n"));
        assert!(body.contains("Disallow: /\n"));
        assert!(body.contains("Sitemap: https://app.example.com/sitemap.xml\n"));
    }

    #[tokio::test]
    async fn sitemap_lists_the_public_documents() {
        let body = body_text(sitemap_xml(axum::extract::State(state(AuthMode::Full))).await).await;
        assert!(body.contains("<loc>https://app.example.com/llms.txt</loc>"));
        assert!(body.contains("<loc>https://app.example.com/auth.md</loc>"));
    }

    #[tokio::test]
    async fn api_catalog_is_an_rfc9727_linkset_of_rest_and_mcp() {
        let response = api_catalog(axum::extract::State(state(AuthMode::Full))).await;
        let content_type = response.headers()[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .to_string();
        assert!(content_type.starts_with("application/linkset+json"));
        let body: Value = serde_json::from_str(&body_text(response).await).expect("json");
        let linkset = body["linkset"].as_array().expect("linkset");
        assert_eq!(linkset[0]["anchor"], "https://app.example.com/api");
        assert_eq!(
            linkset[0]["service-desc"][0]["href"],
            "https://app.example.com/api-doc/openapi.json"
        );
        assert_eq!(linkset[1]["anchor"], "https://app.example.com/mcp");
    }

    #[tokio::test]
    async fn root_protected_resource_metadata_describes_the_mcp_resource() {
        let response =
            protected_resource_metadata(axum::extract::State(state(AuthMode::External))).await;
        let body: Value = serde_json::from_str(&body_text(response).await).expect("json");
        // Same fields as `auth::mcp_oauth`'s path-derived document.
        assert_eq!(
            body,
            json!({
                "resource": "https://app.example.com/mcp",
                "authorization_servers": ["https://app.example.com"],
                "bearer_methods_supported": ["header"],
                "scopes_supported": ["mcp"],
            })
        );
    }

    #[tokio::test]
    async fn root_protected_resource_metadata_is_absent_without_auth() {
        let response =
            protected_resource_metadata(axum::extract::State(state(AuthMode::None))).await;
        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn server_card_describes_the_product() {
        let body = card_json(AuthMode::External).await;
        assert_eq!(body["serverInfo"]["title"], "Everruns");
        assert_eq!(body["serverInfo"]["documentationUrl"], DOCS_URL);
        assert!(
            body["serverInfo"]["description"]
                .as_str()
                .unwrap()
                .contains("Durable")
        );
    }

    async fn body_text(response: Response) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        String::from_utf8(bytes.to_vec()).expect("utf8")
    }

    async fn card_json(mode: AuthMode) -> Value {
        let response = mcp_server_card(axum::extract::State(state(mode))).await;
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        serde_json::from_slice(&bytes).expect("json")
    }
}
