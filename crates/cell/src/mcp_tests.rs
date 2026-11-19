use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use everruns_contracts::dataplane::{Connection, RefreshOwner};
use everruns_contracts::dataplane_seal::{Key, random_nonce, seal, unseal};
use everruns_core::{
    EgressError, EgressRequest, EgressResponse, EgressResult, EgressService, EgressStreamResponse,
    McpServerAuthMode,
};

use super::*;
use crate::control::{Feed, Orgs};

const ORG: &str = "org-a";
const KEY: Key = [7u8; 32];
const GRANT: &str = "mcp_oauth:grant-1";
const TOKEN_URL: &str = "https://auth.example/token";

/// The directory cell and the org's sealed secrets, in memory: what every
/// executor reads and leases through.
#[derive(Default)]
struct Directory {
    state: parking_lot::Mutex<(Orgs, Feed)>,
    sessions: parking_lot::Mutex<HashMap<SessionId, String>>,
    /// Host calls made: (org, command, params).
    lookups: parking_lot::Mutex<Vec<(String, String, serde_json::Value)>>,
}

impl Directory {
    fn put(&self, secret_id: &str, revision: u64, plain: &[u8]) {
        let sealed = seal(&KEY, "dk", ORG, secret_id, revision, plain, random_nonce()).unwrap();
        let mut state = self.state.lock();
        let config = state.0.orgs.entry(ORG.into()).or_default();
        config.secrets.insert(secret_id.into(), sealed);
    }

    fn connect(&self, id: &str) {
        let mut state = self.state.lock();
        let config = state.0.orgs.entry(ORG.into()).or_default();
        config.connections.insert(
            id.into(),
            Connection {
                connection_id: id.into(),
                kind: MCP_OAUTH.into(),
                settings: serde_json::json!({}),
                token_secret_id: id.into(),
                refresh_owner: RefreshOwner::DataPlane,
            },
        );
    }

    fn opened(&self, secret_id: &str) -> (u64, Vec<u8>) {
        let state = self.state.lock();
        let sealed = &state.0.orgs[ORG].secrets[secret_id];
        (sealed.revision, unseal(&KEY, ORG, sealed).unwrap())
    }
}

#[async_trait::async_trait]
impl OrgMaterial for Directory {
    fn tenant(&self, session: SessionId) -> Option<String> {
        self.sessions.lock().get(&session).cloned()
    }

    async fn connection(&self, org: &str, id: &str) -> anyhow::Result<Option<Connection>> {
        let state = self.state.lock();
        Ok(state
            .0
            .orgs
            .get(org)
            .and_then(|c| c.connections.get(id).cloned()))
    }

    async fn secret(&self, org: &str, secret_id: &str) -> anyhow::Result<Vec<u8>> {
        self.open_secret(org, secret_id)
            .await?
            .map(|s| s.plain)
            .ok_or_else(|| anyhow::anyhow!("no secret"))
    }

    async fn open_secret(
        &self,
        org: &str,
        secret_id: &str,
    ) -> anyhow::Result<Option<OpenedSecret>> {
        let state = self.state.lock();
        let Some(sealed) = state.0.orgs.get(org).and_then(|c| c.secrets.get(secret_id)) else {
            return Ok(None);
        };
        Ok(Some(OpenedSecret {
            revision: sealed.revision,
            plain: unseal(&KEY, org, sealed)?,
            data_key: KEY,
            data_key_id: "dk".into(),
        }))
    }

    async fn lookup(
        &self,
        org: &str,
        command: &str,
        params: serde_json::Value,
    ) -> anyhow::Result<everruns_contracts::host_calls::HostCallAnswer> {
        let n = {
            let mut lookups = self.lookups.lock();
            lookups.push((org.into(), command.into(), params));
            lookups.len()
        };
        Ok(everruns_contracts::host_calls::HostCallAnswer::Value(
            serde_json::json!(format!("person-token-{n}")),
        ))
    }

    async fn secret_op(&self, org: &str, op: SecretOp) -> anyhow::Result<SecretReply> {
        let mut state = self.state.lock();
        let (orgs, feed) = &mut *state;
        let now = chrono::Utc::now().timestamp_millis();
        match crate::secrets::apply(orgs, feed, 1, org, op, now) {
            crate::state::Response::Value { value } => Ok(serde_json::from_value(value)?),
            other => anyhow::bail!("{other:?}"),
        }
    }
}

/// A token endpoint that rotates the refresh token on every refresh, and
/// refuses one it already spent (as rotating providers do).
#[derive(Default)]
struct TokenEndpoint {
    refreshes: AtomicUsize,
    spent: parking_lot::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl EgressService for TokenEndpoint {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        assert_eq!(request.url, TOKEN_URL);
        // Slow enough that concurrent callers overlap.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let form = String::from_utf8(request.body.clone()).unwrap_or_default();
        let refresh = form
            .split('&')
            .find_map(|pair| pair.strip_prefix("refresh_token="))
            .unwrap_or_default()
            .to_owned();
        let mut spent = self.spent.lock();
        if spent.contains(&refresh) {
            return Ok(EgressResponse {
                status: 400,
                headers: BTreeMap::new(),
                body: br#"{"error":"invalid_grant"}"#.to_vec(),
            });
        }
        spent.push(refresh);
        let n = self.refreshes.fetch_add(1, Ordering::SeqCst) + 1;
        let body = serde_json::json!({
            "access_token": format!("access-{n}"),
            "refresh_token": format!("refresh-{n}"),
            "token_type": "Bearer",
            "expires_in": 3600,
        });
        Ok(EgressResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: body.to_string().into_bytes(),
        })
    }

    async fn send_stream(&self, _request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        Err(EgressError::Transport("unsupported".to_string()))
    }
}

fn expired_tokens() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "access_token": "access-0",
        "refresh_token": "refresh-0",
        "token_type": "Bearer",
        "expires_at": (chrono::Utc::now() - chrono::Duration::seconds(5)).to_rfc3339(),
        "token_endpoint": TOKEN_URL,
        "resource": "https://mcp.example/mcp",
        "client_id": "client-1",
    }))
    .unwrap()
}

fn executor(directory: &Arc<Directory>, egress: Arc<dyn EgressService>) -> Arc<OrgMcp> {
    let mcp = Arc::new(OrgMcp::new(egress));
    let material: Arc<dyn OrgMaterial> = directory.clone();
    // The same allocation as `directory`, which the test holds.
    mcp.bind(Arc::downgrade(&material));
    mcp
}

fn request(server: &str, session: SessionId) -> McpAuthRequest<'_> {
    McpAuthRequest {
        server_url: Some("https://mcp.example/mcp"),
        server_name: server,
        auth_mode: McpServerAuthMode::None,
        oauth_provider_id: None,
        session_id: Some(session.uuid()),
    }
}

/// An agent with one server, `crm`, acting through connection `id`.
fn crm(id: &str) -> Bindings {
    let mut bindings = Bindings::default();
    bindings.oauth.insert("crm".into(), id.into());
    bindings
}

#[test]
fn placeholders_leave_the_definition_and_literal_headers_stay() {
    let mut servers: everruns_core::ScopedMcpServers = serde_json::from_value(serde_json::json!({
        "docs": {
            "url": "https://docs.example/mcp",
            "headers": {
                "Authorization": "${MCP_DOCS_AUTHORIZATION}",
                "X-Team": "blue",
                "X-Api-Key": "${DOCS_KEY}",
            },
        },
        "open": { "url": "https://open.example/mcp" },
        "crm": {
            "url": "https://crm.example/mcp",
            "auth_mode": "oauth",
            "oauth_provider_id": GRANT,
        },
    }))
    .unwrap();
    let bindings = take_bindings(&mut servers);
    assert_eq!(
        bindings.headers.get("docs"),
        Some(&vec![
            (
                "Authorization".to_owned(),
                "MCP_DOCS_AUTHORIZATION".to_owned()
            ),
            ("X-Api-Key".to_owned(), "DOCS_KEY".to_owned()),
        ])
    );
    assert!(!bindings.headers.contains_key("open"));
    let headers = &servers["docs"].headers;
    assert_eq!(headers.len(), 1);
    assert_eq!(headers["X-Team"], "blue");
    // The connection leaves too, and with it any sign-in of the server's own.
    assert_eq!(bindings.oauth.get("crm").map(String::as_str), Some(GRANT));
    assert_eq!(servers["crm"].oauth_provider_id, None);
    assert_eq!(servers["crm"].auth_mode, McpServerAuthMode::None);
}

/// A server that acts as the turn's person keeps its provider out of the
/// definition, and its token is asked of the control plane for the
/// session's open turn on every use, never kept: the next turn may be
/// another person's.
#[tokio::test]
async fn a_person_acting_server_asks_for_the_open_turns_token_each_time() {
    let mut servers: everruns_core::ScopedMcpServers = serde_json::from_value(serde_json::json!({
        "My Notes": {
            "url": "https://notes.example/mcp",
            "auth_mode": "oauth",
            "oauth_provider_id": "mcp_oauth_notes",
            "acts_as": "user",
        },
        "plain": { "url": "https://plain.example/mcp", "auth_mode": "oauth" },
    }))
    .unwrap();
    let bindings = take_bindings(&mut servers);
    assert_eq!(
        bindings.per_turn.get("My Notes"),
        Some(&PerTurnGrant {
            provider: "mcp_oauth_notes".into(),
            acts_as: everruns_core::McpServerActsAs::User,
        })
    );
    let notes = &servers["My Notes"];
    assert_eq!(notes.oauth_provider_id, None);
    assert_eq!(notes.auth_mode, McpServerAuthMode::None);
    assert!(notes.acts_as.is_none());
    // A server acting as no one is left as it is.
    assert!(!bindings.per_turn.contains_key("plain"));

    let directory = Arc::new(Directory::default());
    let session = SessionId::new();
    directory.sessions.lock().insert(session, ORG.into());
    let auth = executor(&directory, Arc::new(TokenEndpoint::default())).for_agent(bindings);
    for n in 1..=2 {
        let credential = auth
            .authorization(&request("My Notes", session))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            credential.authorization,
            Some(format!("Bearer person-token-{n}"))
        );
    }
    let lookups = directory.lookups.lock();
    assert_eq!(lookups.len(), 2, "never kept");
    let (org, command, params) = &lookups[0];
    assert_eq!(org, ORG);
    assert_eq!(command, MCP_CONNECTION_COMMAND);
    assert_eq!(params["session_id"], session.to_string());
    assert_eq!(params["server_prefix"], "my_notes");
    assert_eq!(params["provider"], "mcp_oauth_notes");
    assert_eq!(params["acts_as"], "user");
    assert_eq!(params["input_message_id"], OPEN_TURN);
    assert_eq!(
        everruns_contracts::host_calls::by_command(command).map(|c| c.name),
        Some(everruns_contracts::host_calls::MCP_CONNECTION_TOKEN)
    );
}

#[tokio::test]
async fn a_sessions_org_fills_its_agents_header_placeholders() {
    let directory = Arc::new(Directory::default());
    directory.put(
        &mcp_binding_secret("MCP_DOCS_AUTHORIZATION"),
        1,
        b"Bearer docs-key",
    );
    let session = SessionId::new();
    directory.sessions.lock().insert(session, ORG.into());
    let mcp = executor(&directory, Arc::new(TokenEndpoint::default()));
    let mut bindings = Bindings::default();
    bindings.headers.insert(
        "docs".into(),
        vec![
            ("Authorization".into(), "MCP_DOCS_AUTHORIZATION".into()),
            ("X-Api-Key".into(), "NOT_SET".into()),
        ],
    );
    let auth = mcp.for_agent(bindings);
    let credential = auth
        .authorization(&request("docs", session))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(credential.authorization.as_deref(), Some("Bearer docs-key"));
    // No secret behind it: sent as written, as the platform's workers do.
    assert_eq!(credential.headers["X-Api-Key"], "${NOT_SET}");
    // Another org's session, or no session, gets nothing.
    let stranger = SessionId::new();
    let none = auth
        .authorization(&request("docs", stranger))
        .await
        .unwrap();
    assert!(none.is_none());
    let mut anonymous = request("docs", session);
    anonymous.session_id = None;
    assert!(auth.authorization(&anonymous).await.unwrap().is_none());
}

#[tokio::test]
async fn one_executor_refreshes_an_expired_grant_and_every_executor_uses_it() {
    let directory = Arc::new(Directory::default());
    directory.connect(GRANT);
    directory.put(GRANT, 1, &expired_tokens());
    let session = SessionId::new();
    directory.sessions.lock().insert(session, ORG.into());
    let endpoint = Arc::new(TokenEndpoint::default());
    let executors: Vec<Arc<OrgMcp>> = (0..4)
        .map(|_| executor(&directory, endpoint.clone()))
        .collect();
    let calls = executors.iter().map(|mcp| {
        let auth = mcp.for_agent(crm(GRANT));
        async move { auth.authorization(&request("crm", session)).await }
    });
    let credentials = futures::future::join_all(calls).await;
    for credential in credentials {
        assert_eq!(
            credential.unwrap().unwrap().authorization.as_deref(),
            Some("Bearer access-1")
        );
    }
    assert_eq!(endpoint.refreshes.load(Ordering::SeqCst), 1, "one refresh");
    let (revision, plain) = directory.opened(GRANT);
    assert_eq!(revision, 2);
    let stored: TokenSet = serde_json::from_slice(&plain).unwrap();
    assert_eq!(stored.refresh_token.as_deref(), Some("refresh-1"));
    assert_eq!(stored.token_endpoint.as_deref(), Some(TOKEN_URL));
    let state = directory.state.lock();
    assert!(matches!(
        state.1.records.back().map(|r| &r.record),
        Some(crate::dp::Record::SecretRefreshed { revision: 2, .. })
    ));
    assert_eq!(executors[0].refreshes.get(&[("outcome", "stored")]), 1);
}

#[tokio::test]
async fn a_fresh_grant_is_used_without_a_refresh_and_a_missing_one_gives_nothing() {
    let directory = Arc::new(Directory::default());
    directory.connect(GRANT);
    let mut fresh: serde_json::Value = serde_json::from_slice(&expired_tokens()).unwrap();
    fresh["expires_at"] =
        serde_json::json!((chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339());
    directory.put(GRANT, 1, &serde_json::to_vec(&fresh).unwrap());
    let session = SessionId::new();
    directory.sessions.lock().insert(session, ORG.into());
    let endpoint = Arc::new(TokenEndpoint::default());
    let mcp = executor(&directory, endpoint.clone());
    let credential = mcp
        .for_agent(crm(GRANT))
        .authorization(&request("crm", session))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(credential.authorization.as_deref(), Some("Bearer access-0"));
    assert_eq!(endpoint.refreshes.load(Ordering::SeqCst), 0);
    let missing = mcp
        .for_agent(crm("mcp_oauth:other"))
        .authorization(&request("crm", session))
        .await
        .unwrap();
    assert!(missing.is_none());
}

/// A server acting as the turn's person declares the host call it asks
/// per call, so the parity gate does not count it as undeclared; a pushed
/// service grant (an `mcp_oauth:` connection) asks nothing.
#[test]
fn a_person_acting_server_declares_its_token_host_call() {
    let definition = |provider: &str| {
        serde_json::json!({
            "name": "notes",
            "instructions": "Take notes.",
            "model": { "provider": "llmsim", "model": "echo" },
            "mcpServers": {
                "notes": {
                    "url": "https://notes.example/mcp",
                    "auth_mode": "oauth",
                    "oauth_provider_id": provider,
                    "actsAs": "user",
                }
            },
        })
    };
    let token = everruns_contracts::host_calls::MCP_CONNECTION_TOKEN;
    assert!(crate::host::declared_host_calls(&definition("mcp_oauth_notes")).contains(token));
    let pushed = format!("{MCP_OAUTH}:grant-1");
    assert!(!crate::host::declared_host_calls(&definition(&pushed)).contains(token));
}
