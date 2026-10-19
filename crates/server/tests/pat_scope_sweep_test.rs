//! Runtime sweep for personal access token scopes
//! (knowledge/security/pat-scopes.md, Tests item 3).
//!
//! Starts the production router (`ServerAppBuilder`) in `AUTH_MODE=full` and
//! sends one request per route in the universe (the OpenAPI document plus the
//! undocumented rows of the classification table) with a placeholder path:
//!
//! - with a `read` PAT, every `Write` or `Never` route answers exactly
//!   `403 insufficient_scope`, which catches a handler that validates tokens
//!   itself and skips the extractor; no `Read` route ever does, which catches
//!   a router spelling the classification layer does not resolve;
//! - with a `*` PAT, no route ever answers `insufficient_scope`.
//!
//! It also checks `POST /v1/commands/{name}` per command, and the surfaces of
//! Tests item 6 that validate a PAT outside the route table: the
//! `GET /v1/commands` listing and `/mcp` (`tools/list`, `tools/call`). The
//! sweep's tokens are inserted directly; `creation` mints them through
//! `POST /v1/auth/personal-access-tokens` instead and follows a `read` token
//! end to end, with `token_scope` on `GET /v1/auth/me` and the creation audit
//! event (rollout step 4).
//!
//! A dedicated single-test binary because the production builder reads
//! process environment, which is installed before the server runtime starts.
//! Run with: `cargo test -p everruns-server --test pat_scope_sweep_test`
//! The fixture owns a migrated database on the selected native or external cluster.

mod test_harness;

use std::collections::BTreeSet;
use std::time::Duration;

use axum::http::Method;
use everruns_core::DEFAULT_ORG_ID;
use everruns_server::app_builder::{ServerAppBuilder, ServerContext};
use everruns_server::auth::pat_scope::{
    INSUFFICIENT_SCOPE, RouteAccess, classify, classify_command, normalize_template, routes,
};
use everruns_server::auth::personal_access_token::generate_personal_access_token;
use everruns_server::domains::common::CommandDescriptor;
use everruns_server::openapi::ApiDoc;
use everruns_server::server::ServerConfig;
use everruns_server::storage::{
    AuditLogQuery, CreatePersonalAccessTokenRow, CreateUserRow, StorageBackend,
};
use utoipa::OpenApi;

fn configure_server_environment(database_url: &str) {
    // SAFETY: this integration binary contains one synchronous test. It sets
    // every variable before constructing the server runtime, and no sibling test can run in
    // this process.
    unsafe {
        std::env::set_var("DATABASE_URL", database_url);
        std::env::set_var("DEPLOYMENT_GRADE", "dev");
        std::env::set_var("AUTH_MODE", "full");
        std::env::set_var("AUTH_JWT_SECRET", "pat-scope-sweep-test-secret-0123456789");
        std::env::set_var("WORKER_GRPC_AUTH_TOKEN", "pat-scope-sweep-test");
        // One request per route, twice: the per-IP API limiter must not trip.
        std::env::set_var("RATE_LIMIT_API_REQUESTS_PER_MINUTE", "0");
        // Mount the feature-gated payments routes so they are swept too.
        std::env::set_var("FEATURE_MACHINE_PAYMENTS", "dev");
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind an ephemeral port")
        .local_addr()
        .unwrap()
        .port()
}

async fn start_real_server(port: u16) -> ServerContext {
    let config = ServerConfig {
        dev_mode: false,
        no_migrations: false,
        api_prefix: String::new(),
        cors_origins: vec![],
        addr: format!("127.0.0.1:{port}"),
        grpc_addr: "127.0.0.1:0".to_string(),
    };
    let (context_tx, context_rx) = tokio::sync::oneshot::channel();
    let mut server = tokio::spawn(
        ServerAppBuilder::new(config)
            .background_task(move |context| async move {
                let _ = context_tx.send(context);
                futures::future::pending::<()>().await;
            })
            .run(),
    );
    let context = tokio::select! {
        context = context_rx => context.expect("server context sender dropped"),
        result = &mut server => panic!("server exited during startup: {result:?}"),
        _ = tokio::time::sleep(Duration::from_secs(60)) => panic!("server startup timed out"),
    };
    let client = reqwest::Client::new();
    for _ in 0..120 {
        if let Ok(response) = client
            .get(format!("http://127.0.0.1:{port}/health"))
            .send()
            .await
            && response.status().is_success()
        {
            return context;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("server did not become healthy on port {port}");
}

/// A fresh owner of the default org holding one PAT with `scopes`.
async fn mint_pat(db: &StorageBackend, scopes: &[&str]) -> String {
    let user = db
        .create_user(CreateUserRow {
            email: format!("pat-sweep-{}@example.com", uuid::Uuid::now_v7()),
            name: "PAT sweep".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .expect("create sweep user");
    db.add_organization_member(DEFAULT_ORG_ID, user.id, "owner")
        .await
        .expect("join default org");
    let generated = generate_personal_access_token();
    db.create_personal_access_token(CreatePersonalAccessTokenRow {
        user_id: user.id,
        name: format!("sweep {scopes:?}"),
        token_hash: generated.token_hash,
        token_prefix: generated.token_prefix,
        scopes: scopes.iter().map(|s| s.to_string()).collect(),
        expires_at: None,
        metadata: serde_json::json!({}),
    })
    .await
    .expect("insert PAT row");
    generated.token
}

/// Write or Never routes that do not answer a `read` PAT with
/// `insufficient_scope`, each for a reason that is not a scope bypass. A row
/// that starts answering `insufficient_scope` fails the sweep as stale.
const NOT_DENIED_BY_THE_EXTRACTOR: &[(&str, &str, &str)] = &[
    // Agent Execution API, A2A, FCP, webhook and runtime-auth ingress: the
    // caller is authenticated by the channel's own credential (agent key,
    // signature, runtime token) after the channel is found, so a placeholder
    // channel is a 404 first. A PAT counts there only as an org member
    // (`agent_api_auth::org_member`), restricted to `*` in rollout step 3.
    ("POST", "/v1/apps/{}/a2a/{}", INGRESS),
    ("POST", "/v1/apps/{}/fcp", INGRESS),
    ("POST", "/v1/apps/{}/webhooks/{}", INGRESS),
    ("POST", "/v1/channels/{}/a2a", INGRESS),
    ("POST", "/v1/channels/{}/fcp", INGRESS),
    ("POST", "/v1/channels/{}/runtime-auth", INGRESS),
    ("POST", "/v1/channels/{}/sessions", INGRESS),
    ("POST", "/v1/channels/{}/sessions/{}/cancel", INGRESS),
    ("POST", "/v1/channels/{}/sessions/{}/messages", INGRESS),
    (
        "POST",
        "/v1/channels/{}/sessions/{}/question-answers",
        INGRESS,
    ),
    (
        "POST",
        "/v1/channels/{}/sessions/{}/tool-approvals",
        INGRESS,
    ),
    ("POST", "/v1/channels/{}/webhook", INGRESS),
    ("POST", "/v1/e/{}/a2a", INGRESS),
    ("POST", "/v1/e/{}/fcp", INGRESS),
    ("POST", "/v1/e/{}/runtime-auth", INGRESS),
    ("POST", "/v1/e/{}/sessions", INGRESS),
    ("POST", "/v1/e/{}/sessions/{}/cancel", INGRESS),
    ("POST", "/v1/e/{}/sessions/{}/messages", INGRESS),
    ("POST", "/v1/e/{}/sessions/{}/question-answers", INGRESS),
    ("POST", "/v1/e/{}/sessions/{}/tool-approvals", INGRESS),
    ("POST", "/v1/e/{}/webhook", INGRESS),
    // OAuth and app-install callbacks: they finish a flow whose start is
    // `Never`, and are bound to that flow's state, not to the bearer token.
    ("GET", "/v1/channels/{}/slack/oauth/callback", CALLBACK),
    ("GET", "/v1/connection-callbacks/github", CALLBACK),
    ("GET", "/v1/connection-callbacks/{}", CALLBACK),
    ("GET", "/v1/e/{}/slack/oauth/callback", CALLBACK),
    ("GET", "/v1/github/app-manifest/callback", CALLBACK),
    ("GET", "/v1/github/apps/{}/setup", CALLBACK),
    ("GET", "/v1/slack/workspaces/oauth/callback", CALLBACK),
    ("GET", "/v1/user/connections/github/callback", CALLBACK),
    ("GET", "/v1/user/connections/{}/callback", CALLBACK),
    // Documented but not routed in this configuration.
    ("POST", "/v1/agents/{}/endpoints/{}/voice/calls", NOT_ROUTED),
    ("POST", "/v1/sessions/{}/sandbox", NOT_ROUTED),
];
const INGRESS: &str = "channel ingress, own credential";
const CALLBACK: &str = "flow callback, bound to flow state";
const NOT_ROUTED: &str = "not routed without a voice alias or sandbox provider";

/// The reason a Write/Never route is exempt from the read-token denial, if it is.
fn not_denied_by_the_extractor(method: &str, path: &str) -> Option<&'static str> {
    let path = normalize_template(path);
    NOT_DENIED_BY_THE_EXTRACTOR
        .iter()
        .find(|(m, p, _)| *m == method && *p == path)
        .map(|(_, _, reason)| *reason)
}

/// `(method, path)` of every route the sweep covers.
fn universe() -> BTreeSet<(String, String)> {
    let doc = ApiDoc::openapi();
    let mut out = BTreeSet::new();
    for (path, item) in &doc.paths.paths {
        for (name, op) in [
            ("GET", item.get.is_some()),
            ("POST", item.post.is_some()),
            ("PUT", item.put.is_some()),
            ("PATCH", item.patch.is_some()),
            ("DELETE", item.delete.is_some()),
        ] {
            if op {
                out.insert((name.to_string(), path.clone()));
            }
        }
    }
    for (method, path, _) in routes::UNDOCUMENTED_ROUTES {
        out.insert((method.to_string(), path.to_string()));
    }
    assert!(out.len() > 100, "route universe is suspiciously small");
    out
}

/// Replace every `{param}` (and `{*path}`) segment with a placeholder.
fn placeholder_path(template: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut in_param = false;
    for c in template.chars() {
        match c {
            '{' => {
                in_param = true;
                out.push_str("placeholder");
            }
            '}' => in_param = false,
            _ if in_param => {}
            _ => out.push(c),
        }
    }
    out
}

struct Answer {
    status: u16,
    code: Option<String>,
    challenge: Option<String>,
}

impl Answer {
    fn is_insufficient_scope(&self) -> bool {
        self.code.as_deref() == Some(INSUFFICIENT_SCOPE)
    }

    fn is_exact_denial(&self) -> bool {
        self.status == 403
            && self.is_insufficient_scope()
            && self.challenge.as_deref() == Some("Bearer error=\"insufficient_scope\"")
    }
}

async fn send(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> Option<Answer> {
    let method = reqwest::Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = client
        .request(method.clone(), format!("{base}{path}"))
        .bearer_auth(token);
    if method != reqwest::Method::GET && method != reqwest::Method::DELETE {
        request = request.json(&body.unwrap_or_else(|| serde_json::json!({})));
    }
    // A streaming route may never finish; a timeout is "not a denial".
    let response = tokio::time::timeout(Duration::from_secs(10), request.send())
        .await
        .ok()?
        .ok()?;
    let status = response.status().as_u16();
    let challenge = response
        .headers()
        .get(reqwest::header::WWW_AUTHENTICATE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = tokio::time::timeout(Duration::from_secs(5), response.bytes())
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    let code = serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .and_then(|value| value["code"].as_str().map(str::to_string));
    Some(Answer {
        status,
        code,
        challenge,
    })
}

/// A command by wire name, checked to have the expected class.
fn command(name: &'static str, class: RouteAccess) -> &'static str {
    let desc = inventory::iter::<CommandDescriptor>
        .into_iter()
        .find(|desc| (desc.meta)().name == name)
        .unwrap_or_else(|| panic!("no command {name}"));
    assert_eq!(classify_command(desc), class, "{name}");
    name
}

async fn sweep(port: u16, context: ServerContext) {
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let read_token = mint_pat(&context.db, &["read"]).await;
    let full_token = mint_pat(&context.db, &["*"]).await;

    // `read`: Write and Never answer exactly 403 insufficient_scope; Read never does.
    let mut wrong = Vec::new();
    let mut denied = 0;
    for (method, path) in universe() {
        let class = classify(&Method::from_bytes(method.as_bytes()).unwrap(), &path);
        let answer = send(
            &client,
            &base,
            &read_token,
            &method,
            &placeholder_path(&path),
            None,
        )
        .await;
        match class {
            Some(RouteAccess::Write | RouteAccess::Never)
                if not_denied_by_the_extractor(&method, &path).is_some() =>
            {
                // A NOT_ROUTED row is mounted only in some configurations (CI has
                // a sandbox provider), so a denial there is correct, not stale.
                let routed_elsewhere =
                    not_denied_by_the_extractor(&method, &path) == Some(NOT_ROUTED);
                if !routed_elsewhere && answer.as_ref().is_some_and(Answer::is_insufficient_scope) {
                    wrong.push(format!(
                        "{method} {path}: now denied; remove its NOT_DENIED_BY_THE_EXTRACTOR row"
                    ));
                }
            }
            Some(RouteAccess::Write | RouteAccess::Never) => match &answer {
                Some(answer) if answer.is_exact_denial() => denied += 1,
                Some(answer) => wrong.push(format!(
                    "{method} {path} ({class:?}): {} {:?}",
                    answer.status, answer.code
                )),
                None => wrong.push(format!("{method} {path} ({class:?}): no answer")),
            },
            _ => {
                if answer.as_ref().is_some_and(Answer::is_insufficient_scope) {
                    wrong.push(format!("{method} {path} ({class:?}): denied a read token"));
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "read PAT sweep: {} routes misbehaved:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
    assert!(denied > 100, "only {denied} routes denied a read token");

    // Per command: a write command is 403, a read one runs, an unknown name is 404.
    let write_command = command("create_agent", RouteAccess::Write);
    let read_command = command("list_agents", RouteAccess::Read);
    let answer = send(
        &client,
        &base,
        &read_token,
        "POST",
        &format!("/v1/commands/{write_command}"),
        Some(serde_json::json!({"params": {}})),
    )
    .await
    .expect("write command answers");
    assert!(
        answer.is_exact_denial(),
        "{write_command}: {}",
        answer.status
    );
    let answer = send(
        &client,
        &base,
        &read_token,
        "POST",
        &format!("/v1/commands/{read_command}"),
        Some(serde_json::json!({"params": {}})),
    )
    .await
    .expect("read command answers");
    assert!(
        !answer.is_insufficient_scope() && answer.status != 403,
        "{read_command}: {} {:?}",
        answer.status,
        answer.code
    );
    let answer = send(
        &client,
        &base,
        &read_token,
        "POST",
        "/v1/commands/no_such_command",
        Some(serde_json::json!({"params": {}})),
    )
    .await
    .expect("unknown command answers");
    assert_eq!(answer.status, 404, "unknown command stays 404");

    // `*`: nothing is ever insufficient_scope. Deleting the account goes last,
    // so the token stays valid for the rest of the sweep.
    let mut routes: Vec<_> = universe().into_iter().collect();
    routes.sort_by_key(|(method, path)| method == "DELETE" && path == "/v1/users/me");
    let mut wrong = Vec::new();
    for (method, path) in routes {
        let answer = send(
            &client,
            &base,
            &full_token,
            &method,
            &placeholder_path(&path),
            None,
        )
        .await;
        if answer.as_ref().is_some_and(Answer::is_insufficient_scope) {
            wrong.push(format!("{method} {path}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "a * PAT got insufficient_scope on:\n{}",
        wrong.join("\n")
    );
}

async fn list_commands(client: &reqwest::Client, base: &str, token: &str) -> Vec<String> {
    let catalog: serde_json::Value = client
        .get(format!("{base}/v1/commands"))
        .bearer_auth(token)
        .send()
        .await
        .expect("list commands")
        .error_for_status()
        .expect("list commands status")
        .json()
        .await
        .expect("command catalog JSON");
    catalog["commands"]
        .as_array()
        .expect("commands array")
        .iter()
        .map(|entry| {
            entry["wire_name"]
                .as_str()
                .expect("command wire name")
                .to_string()
        })
        .collect()
}

async fn mcp(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    client
        .post(format!("{base}/mcp"))
        .bearer_auth(token)
        .header("MCP-Protocol-Version", "2026-07-28")
        .json(&serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
        .send()
        .await
        .expect("mcp request")
        .error_for_status()
        .expect("mcp status")
        .json()
        .await
        .expect("mcp JSON")
}

fn tool_names(list: &serde_json::Value) -> BTreeSet<String> {
    list["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_string())
        .collect()
}

/// The tools a `read` token may list and call on `/mcp`.
const READ_ONLY_MCP_TOOLS: &[&str] = &[
    "agent_get_card",
    "discover",
    "everruns_home",
    "list_organizations",
    "me",
    "query",
    "session_get_status",
    "session_view",
];

/// Tests item 6: `GET /v1/commands` and `/mcp` honor a `read` token's scope
/// and leave a `*` token as it was.
async fn surfaces(port: u16, db: &StorageBackend) {
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();
    let read_token = mint_pat(db, &["read"]).await;
    let full_token = mint_pat(db, &["*"]).await;

    // GET /v1/commands lists exactly the commands each token can run.
    let read_commands = list_commands(&client, &base, &read_token).await;
    let full_commands = list_commands(&client, &base, &full_token).await;
    let class = |name: &str| {
        let desc = inventory::iter::<CommandDescriptor>
            .into_iter()
            .find(|desc| (desc.meta)().name == name)
            .unwrap_or_else(|| panic!("no command {name}"));
        classify_command(desc)
    };
    assert!(read_commands.contains(&command("list_agents", RouteAccess::Read).to_string()));
    for name in &read_commands {
        assert_eq!(
            class(name),
            RouteAccess::Read,
            "{name} listed to a read token"
        );
    }
    let expected: Vec<&String> = full_commands
        .iter()
        .filter(|name| class(name) == RouteAccess::Read)
        .collect();
    assert_eq!(read_commands.iter().collect::<Vec<_>>(), expected);
    assert!(full_commands.contains(&command("create_agent", RouteAccess::Write).to_string()));

    // tools/list: a read token sees only the read-only tools.
    let read_tools = tool_names(
        &mcp(
            &client,
            &base,
            &read_token,
            "tools/list",
            serde_json::json!({}),
        )
        .await,
    );
    let expected: BTreeSet<String> = READ_ONLY_MCP_TOOLS.iter().map(|s| s.to_string()).collect();
    assert_eq!(read_tools, expected);
    let full_list = mcp(
        &client,
        &base,
        &full_token,
        "tools/list",
        serde_json::json!({}),
    )
    .await;
    let full_tools = tool_names(&full_list);
    assert!(full_tools.is_superset(&expected) && full_tools.contains("execute"));
    assert_eq!(full_list["result"]["cacheScope"], "public");

    // tools/call: execute is refused to a read token with insufficient_scope;
    // query still runs; a * token is never refused for its scope.
    let call = |name: &str, arguments: serde_json::Value| serde_json::json!({ "name": name, "arguments": arguments });
    let script = serde_json::json!({ "commands": "list_agents" });
    let denied = mcp(
        &client,
        &base,
        &read_token,
        "tools/call",
        call("execute", script.clone()),
    )
    .await;
    assert_eq!(denied["result"]["isError"], true, "{denied}");
    assert_eq!(
        denied["result"]["structuredContent"]["code"], INSUFFICIENT_SCOPE,
        "{denied}"
    );
    let queried = mcp(
        &client,
        &base,
        &read_token,
        "tools/call",
        call("query", script.clone()),
    )
    .await;
    assert_ne!(
        queried["result"]["structuredContent"]["code"], INSUFFICIENT_SCOPE,
        "{queried}"
    );
    let executed = mcp(
        &client,
        &base,
        &full_token,
        "tools/call",
        call("execute", script),
    )
    .await;
    assert_ne!(
        executed["result"]["structuredContent"]["code"], INSUFFICIENT_SCOPE,
        "{executed}"
    );
}

/// One JSON request, returning status, `code` and the body.
async fn call(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    method: reqwest::Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> (u16, serde_json::Value) {
    let mut request = client
        .request(method, format!("{base}{path}"))
        .bearer_auth(token);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.expect("request");
    let status = response.status().as_u16();
    let body = response.json().await.unwrap_or(serde_json::Value::Null);
    (status, body)
}

/// Rollout step 4: a signed-in user mints a `read` token through the API, and
/// it reads but cannot write; `token_scope` and the audit event follow the
/// scope; every other requested shape is refused.
async fn creation(port: u16, db: &StorageBackend) {
    use reqwest::Method;
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();

    let email = format!("pat-create-{}@example.com", uuid::Uuid::now_v7());
    let response = client
        .post(format!("{base}/v1/auth/register"))
        .json(&serde_json::json!({
            "email": email,
            "password": "pat-scope-creation-password-1",
            "name": "PAT creation",
        }))
        .send()
        .await
        .expect("register");
    assert_eq!(response.status().as_u16(), 201, "register");
    let session: serde_json::Value = response.json().await.expect("register JSON");
    let jwt = session["access_token"].as_str().expect("access token");
    // Signup creates no org here; join the default one so the token has one.
    let user = db
        .get_user_by_email(&email)
        .await
        .expect("lookup user")
        .expect("registered user");
    db.add_organization_member(DEFAULT_ORG_ID, user.id, "owner")
        .await
        .expect("join default org");

    let create = |scopes: Option<serde_json::Value>| {
        let mut body = serde_json::json!({ "name": format!("create {scopes:?}") });
        if let Some(scopes) = scopes {
            body["scopes"] = scopes;
        }
        call(
            &client,
            &base,
            jwt,
            Method::POST,
            "/v1/auth/personal-access-tokens",
            Some(body),
        )
    };

    // Only [], ["*"] and ["read"] are creatable.
    for rejected in [
        serde_json::json!(["*", "read"]),
        serde_json::json!(["read", "read"]),
        serde_json::json!(["write"]),
        serde_json::json!(["read", "write"]),
    ] {
        let (status, body) = create(Some(rejected.clone())).await;
        assert_eq!(status, 400, "{rejected}: {body}");
        assert_eq!(body["code"], "unsupported_pat_scopes", "{rejected}: {body}");
    }
    let (status, defaulted) = create(None).await;
    assert_eq!(status, 201, "{defaulted}");
    assert_eq!(defaulted["scopes"], serde_json::json!(["*"]));
    let (status, full) = create(Some(serde_json::json!(["*"]))).await;
    assert_eq!(status, 201, "{full}");
    assert_eq!(full["scopes"], serde_json::json!(["*"]));
    let (status, read) = create(Some(serde_json::json!(["read"]))).await;
    assert_eq!(status, 201, "{read}");
    assert_eq!(read["scopes"], serde_json::json!(["read"]));
    let read_token = read["token"].as_str().expect("read token");
    let full_token = full["token"].as_str().expect("full token");

    // The list shows each token's scope.
    let (status, list) = call(
        &client,
        &base,
        jwt,
        Method::GET,
        "/v1/auth/personal-access-tokens",
        None,
    )
    .await;
    assert_eq!(status, 200, "{list}");
    let listed = list["data"]
        .as_array()
        .expect("token list")
        .iter()
        .find(|item| item["id"] == read["id"])
        .expect("read token listed");
    assert_eq!(listed["scopes"], serde_json::json!(["read"]));

    // GET /v1/auth/me reports the scope of the credential, null for a JWT.
    for (token, expected) in [
        (read_token, serde_json::json!("read")),
        (full_token, serde_json::json!("*")),
        (jwt, serde_json::Value::Null),
    ] {
        let (status, me) = call(&client, &base, token, Method::GET, "/v1/auth/me", None).await;
        assert_eq!(status, 200, "{me}");
        assert_eq!(me["token_scope"], expected, "{me}");
        assert_eq!(me["email"], email.as_str());
    }

    // The read token reads, and is refused a write with 403 insufficient_scope.
    let (status, agents) = call(&client, &base, read_token, Method::GET, "/v1/agents", None).await;
    assert_eq!(status, 200, "read route with a read token: {agents}");
    let agent = serde_json::json!({ "name": "pat-scope-creation", "system_prompt": "hi" });
    let (status, denied) = call(
        &client,
        &base,
        read_token,
        Method::POST,
        "/v1/agents",
        Some(agent.clone()),
    )
    .await;
    assert_eq!(status, 403, "{denied}");
    assert_eq!(denied["code"], INSUFFICIENT_SCOPE, "{denied}");
    let (status, created) = call(
        &client,
        &base,
        full_token,
        Method::POST,
        "/v1/agents",
        Some(agent),
    )
    .await;
    assert_ne!(created["code"], INSUFFICIENT_SCOPE, "{status} {created}");

    // A PAT still cannot mint a PAT, whatever its scope.
    let (status, _) = call(
        &client,
        &base,
        full_token,
        Method::POST,
        "/v1/auth/personal-access-tokens",
        Some(serde_json::json!({ "name": "from a pat" })),
    )
    .await;
    assert_eq!(status, 403);

    // The creation audit event records the scopes (written in the background).
    // Token CRUD is user-level; its audit events go to the user's first org.
    let org_id = DEFAULT_ORG_ID;
    let read_id = read["id"].as_str().unwrap();
    let mut recorded = None;
    for _ in 0..40 {
        let rows = db
            .list_audit_logs(AuditLogQuery {
                org_id,
                limit: 50,
                before: None,
                event_type_prefix: Some("auth.personal_access_token.created"),
                actor_id: Some(user.id),
                domain: None,
                action: None,
            })
            .await
            .expect("audit logs");
        recorded = rows
            .into_iter()
            .find(|row| row.metadata["token_id"] == read_id);
        if recorded.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let recorded = recorded.expect("creation audit event for the read token");
    assert_eq!(recorded.metadata["scopes"], serde_json::json!(["read"]));
}

#[test]
fn pat_scopes_hold_on_every_route_of_the_real_router() {
    let database = test_harness::database_before_server_environment();
    configure_server_environment(database.url());
    let port = free_port();
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async move {
            let context = start_real_server(port).await;
            creation(port, &context.db).await;
            surfaces(port, &context.db).await;
            sweep(port, context).await;
        });
}
