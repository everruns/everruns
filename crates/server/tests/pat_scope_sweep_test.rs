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
//! It also checks `POST /v1/commands/{name}` per command. `read` tokens
//! cannot be created through the API yet, so the rows are inserted directly.
//!
//! A dedicated single-test binary because the production builder reads
//! process environment, which is installed before Tokio starts.
//! Run with: `cargo test -p everruns-server --test pat_scope_sweep_test`
//! (needs `DATABASE_URL` or the local test database).

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
use everruns_server::storage::{CreatePersonalAccessTokenRow, CreateUserRow, StorageBackend};
use utoipa::OpenApi;

fn database_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        let port = std::env::var("DB_PORT").unwrap_or_else(|_| "9332".to_string());
        format!("postgres://everruns:everruns@localhost:{port}/everruns_test")
    })
}

fn configure_server_environment() {
    // SAFETY: this integration binary contains one synchronous test. It sets
    // every variable before constructing Tokio, and no sibling test can run in
    // this process.
    unsafe {
        std::env::set_var("DATABASE_URL", database_url());
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

#[test]
fn pat_scopes_hold_on_every_route_of_the_real_router() {
    configure_server_environment();
    let port = free_port();
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async move {
            let context = start_real_server(port).await;
            sweep(port, context).await;
        });
}
