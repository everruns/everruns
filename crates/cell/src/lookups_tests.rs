use std::sync::Arc;

use everruns_contracts::dataplane::{Connection, LookupAnswer};
use everruns_contracts::session_sandbox::SessionSandboxCredential;
use everruns_contracts::typed_id::SessionId;
use everruns_core::McpServerActsAs;
use everruns_core::connection_services::UserConnectionResolver;
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::connections;
use crate::providers::OrgMaterial;

/// An org's material whose lookups answer from a script and are recorded.
struct Material {
    reaches: bool,
    answer: Mutex<Option<anyhow::Result<LookupAnswer>>>,
    asked: Mutex<Vec<(String, String, Value)>>,
}

impl Material {
    fn new(reaches: bool, answer: anyhow::Result<LookupAnswer>) -> Arc<Self> {
        Arc::new(Self {
            reaches,
            answer: Mutex::new(Some(answer)),
            asked: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait::async_trait]
impl OrgMaterial for Material {
    fn tenant(&self, _session: SessionId) -> Option<String> {
        Some("org-a".into())
    }

    async fn connection(&self, _org: &str, _id: &str) -> anyhow::Result<Option<Connection>> {
        Ok(None)
    }

    async fn secret(&self, _org: &str, _secret_id: &str) -> anyhow::Result<Vec<u8>> {
        anyhow::bail!("no secrets")
    }

    fn looks_up(&self) -> bool {
        self.reaches
    }

    async fn lookup(
        &self,
        org: &str,
        command: &str,
        params: Value,
    ) -> anyhow::Result<LookupAnswer> {
        self.asked
            .lock()
            .push((org.to_owned(), command.to_owned(), params));
        self.answer
            .lock()
            .take()
            .unwrap_or_else(|| anyhow::bail!("asked twice"))
    }
}

/// Sandbox keys that answer one fixed key.
struct Keys;

#[async_trait::async_trait]
impl UserConnectionResolver for Keys {
    async fn get_connection_token(
        &self,
        _session_id: SessionId,
        _provider: &str,
    ) -> everruns_contracts::error::Result<Option<String>> {
        Ok(None)
    }

    async fn get_sandbox_connection_token(
        &self,
        _session_id: SessionId,
        _provider: &str,
        _credential: &SessionSandboxCredential,
    ) -> everruns_contracts::error::Result<Option<String>> {
        Ok(Some("sandbox-key".into()))
    }
}

fn resolver(material: &Arc<Material>, session: SessionId) -> Arc<dyn UserConnectionResolver> {
    let material: Arc<dyn OrgMaterial> = material.clone();
    connections(Arc::new(Keys), Some(&Arc::downgrade(&material)), session)
}

#[tokio::test]
async fn a_connection_token_is_looked_up_for_the_session_as_its_org() {
    let material = Material::new(true, Ok(LookupAnswer::Value(json!("gh-token"))));
    let session = SessionId::new();
    let token = resolver(&material, session)
        .get_connection_token(session, "github")
        .await
        .unwrap();
    assert_eq!(token.as_deref(), Some("gh-token"));
    let asked = material.asked.lock();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].0, "org-a");
    assert_eq!(asked[0].1, "worker_get_connection_token");
    assert_eq!(asked[0].2["session_id"], json!(session.to_string()));
    assert_eq!(asked[0].2["provider"], json!("github"));
}

#[tokio::test]
async fn sandbox_keys_and_mcp_grants_stay_on_the_data_plane() {
    let material = Material::new(true, Ok(LookupAnswer::Value(json!("never"))));
    let session = SessionId::new();
    let resolver = resolver(&material, session);
    let credential = SessionSandboxCredential::default();
    assert_eq!(
        resolver
            .get_sandbox_connection_token(session, "daytona", &credential)
            .await
            .unwrap()
            .as_deref(),
        Some("sandbox-key")
    );
    assert_eq!(
        resolver
            .get_mcp_connection_token(session, "mcp_oauth_docs", McpServerActsAs::Service)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        resolver
            .get_connection_token(session, "mcp_oauth_docs")
            .await
            .unwrap(),
        None
    );
    assert!(material.asked.lock().is_empty());
}

#[tokio::test]
async fn a_refusal_and_an_unreachable_control_plane_fail_the_lookup() {
    let refused = Material::new(
        true,
        Ok(LookupAnswer::Error {
            kind: 3,
            message: "Session not found".into(),
        }),
    );
    let session = SessionId::new();
    let error = resolver(&refused, session)
        .get_connection_token(session, "github")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Session not found"), "{error}");

    let down = Material::new(true, Err(anyhow::anyhow!("connection refused")));
    let error = resolver(&down, session)
        .get_tool_approval_policy(session)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("could not be reached"),
        "{error}"
    );
}

#[tokio::test]
async fn a_cell_without_a_control_plane_answers_as_a_host_without_org_settings() {
    let material = Material::new(false, Ok(LookupAnswer::Value(json!("never"))));
    let session = SessionId::new();
    let resolver = resolver(&material, session);
    assert_eq!(
        resolver
            .get_connection_token(session, "github")
            .await
            .unwrap(),
        None
    );
    assert!(
        resolver
            .get_web_search_settings(session)
            .await
            .unwrap()
            .is_none()
    );
    assert!(material.asked.lock().is_empty());
}

/// The Poppy client `attach` installs for `session`, when it installs one.
fn poppy(
    material: &Arc<Material>,
    session: SessionId,
) -> Option<Arc<dyn everruns_capabilities::capabilities::PoppyInvoker>> {
    let material: Arc<dyn OrgMaterial> = material.clone();
    let backends = super::attach(
        everruns_core::host::HostBackends::in_memory(),
        Arc::new(Keys),
        Some(&Arc::downgrade(&material)),
        session,
    );
    let factory = backends.tool_context_extensions_factory?;
    factory(0, session)
        .get::<everruns_capabilities::capabilities::PoppyInvokerExt>()
        .map(|ext| ext.0.clone())
}

#[tokio::test]
async fn a_poppy_call_runs_in_the_control_plane_for_the_session() {
    let material = Material::new(true, Ok(LookupAnswer::Value(json!({ "sent": true }))));
    let session = SessionId::new();
    let call: everruns_capabilities::capabilities::PoppyCall =
        serde_json::from_value(json!({ "action": "discover", "company": "acme.example" })).unwrap();
    let answer = poppy(&material, session)
        .expect("installed")
        .invoke(call)
        .await
        .unwrap();
    assert_eq!(answer, json!({ "sent": true }));
    let asked = material.asked.lock();
    assert_eq!(asked[0].0, "org-a");
    assert_eq!(asked[0].1, "worker_poppy_call");
    assert_eq!(asked[0].2["session_id"], json!(session.to_string()));
    assert_eq!(asked[0].2["call"]["action"], json!("discover"));
    assert_eq!(asked[0].2["call"]["company"], json!("acme.example"));
}

fn slack(
    material: &Arc<Material>,
    session: SessionId,
) -> Option<Arc<dyn everruns_contracts::slack_action::SlackActionInvoker>> {
    let material: Arc<dyn OrgMaterial> = material.clone();
    let backends = super::attach(
        everruns_core::host::HostBackends::in_memory(),
        Arc::new(Keys),
        Some(&Arc::downgrade(&material)),
        session,
    );
    let factory = backends.tool_context_extensions_factory?;
    let extensions = factory(0, session);
    // Posts bind to the turn's input event, which lives in the cell: the
    // channel host delivers them, not a conversation sender.
    assert!(
        extensions
            .get::<everruns_contracts::runtime::conversation::ConversationSenderExt>()
            .is_none()
    );
    extensions
        .get::<everruns_contracts::slack_action::SlackActionInvokerExt>()
        .map(|ext| ext.0.clone())
}

#[tokio::test]
async fn a_slack_action_runs_as_the_sessions_endpoint_and_keeps_its_error() {
    use everruns_contracts::slack_action::{SlackAction, SlackActionError, SlackActionOutcome};
    let react = || SlackAction::AddReaction {
        channel: "C1".into(),
        timestamp: "1.2".into(),
        name: "eyes".into(),
    };
    let session = SessionId::new();
    let done = Material::new(
        true,
        Ok(LookupAnswer::Value(json!({
            "outcome": { "outcome": "reaction_added", "already_reacted": false }
        }))),
    );
    let outcome = slack(&done, session)
        .expect("installed")
        .invoke(react())
        .await;
    assert_eq!(
        outcome,
        Ok(SlackActionOutcome::ReactionAdded {
            already_reacted: false
        })
    );
    {
        let asked = done.asked.lock();
        assert_eq!(asked[0].1, everruns_contracts::dataplane::SLACK_ACTION);
        assert_eq!(asked[0].2["session_id"], json!(session.to_string()));
        assert_eq!(asked[0].2["action"]["action"], json!("add_reaction"));
    }

    // The control plane's refusal is the model's to act on, intact.
    let refused = Material::new(
        true,
        Ok(LookupAnswer::Value(
            json!({ "error": SlackActionError::NoSlackSession }),
        )),
    );
    let error = slack(&refused, session).unwrap().invoke(react()).await;
    assert_eq!(error, Err(SlackActionError::NoSlackSession));

    let down = Material::new(true, Err(anyhow::anyhow!("connection refused")));
    let error = slack(&down, session).unwrap().invoke(react()).await;
    assert!(
        matches!(error, Err(SlackActionError::Transient(_))),
        "{error:?}"
    );

    let standalone = Material::new(false, Ok(LookupAnswer::Value(json!("never"))));
    assert!(slack(&standalone, session).is_none());
}

/// The SQL database store `attach` installs for `session`, when it installs one.
fn sqldb(
    material: &Arc<Material>,
    session: SessionId,
) -> Option<Arc<dyn everruns_contracts::session_sqldb::SessionSqlDbStore>> {
    let material: Arc<dyn OrgMaterial> = material.clone();
    let backends = super::attach(
        everruns_core::host::HostBackends::in_memory(),
        Arc::new(Keys),
        Some(&Arc::downgrade(&material)),
        session,
    );
    let factory = backends.tool_context_extensions_factory?;
    factory(0, session)
        .get::<everruns_contracts::session_sqldb::SessionSqlDbStoreExt>()
        .map(|ext| ext.0.clone())
}

#[tokio::test]
async fn a_sql_database_lives_in_the_control_plane_and_keeps_its_error_kind() {
    use everruns_contracts::session_sqldb::SessionSqlDbError;
    let session = SessionId::new();
    let rows = Material::new(
        true,
        Ok(LookupAnswer::Value(json!({
            // As the control plane writes it: `truncated` left out when false.
            "ok": { "columns": ["n"], "rows": [[1]], "row_count": 1 }
        }))),
    );
    let result = sqldb(&rows, session)
        .expect("installed")
        .sql_query(session, "notes", "SELECT 1 AS n")
        .await
        .unwrap();
    assert_eq!(result.columns, vec!["n".to_owned()]);
    assert_eq!(result.rows, vec![vec![json!(1)]]);
    {
        let asked = rows.asked.lock();
        assert_eq!(asked[0].0, "org-a");
        assert_eq!(asked[0].1, everruns_contracts::dataplane::SESSION_SQL_DB);
        assert_eq!(
            asked[0].2,
            json!({
                "session_id": session.to_string(),
                "op": "query",
                "name": "notes",
                "sql": "SELECT 1 AS n",
            })
        );
    }

    // A timeout stays a timeout, so the tool tells the model what v1 does.
    let slow = Material::new(
        true,
        Ok(LookupAnswer::Value(
            json!({ "error": SessionSqlDbError::QueryTimeout(5) }),
        )),
    );
    let error = sqldb(&slow, session)
        .unwrap()
        .sql_execute(session, "notes", "DELETE FROM t")
        .await;
    assert!(
        matches!(error, Err(SessionSqlDbError::QueryTimeout(5))),
        "{error:?}"
    );

    // Another session's databases are refused before the call leaves.
    let other = Material::new(true, Ok(LookupAnswer::Value(json!({ "ok": [] }))));
    let error = sqldb(&other, session)
        .unwrap()
        .list_databases(SessionId::new())
        .await;
    assert!(
        matches!(error, Err(SessionSqlDbError::Internal(_))),
        "{error:?}"
    );
    assert!(other.asked.lock().is_empty());

    let down = Material::new(true, Err(anyhow::anyhow!("connection refused")));
    let error = sqldb(&down, session).unwrap().list_databases(session).await;
    assert!(
        matches!(error, Err(SessionSqlDbError::Internal(_))),
        "{error:?}"
    );

    let standalone = Material::new(false, Ok(LookupAnswer::Value(json!("never"))));
    assert!(sqldb(&standalone, session).is_none());
}

#[tokio::test]
async fn a_company_refusal_reaches_the_model_and_anything_else_is_unavailable() {
    let call = || -> everruns_capabilities::capabilities::PoppyCall {
        serde_json::from_value(json!({ "action": "discover", "company": "acme.example" })).unwrap()
    };
    let session = SessionId::new();
    let refused = Material::new(
        true,
        Ok(LookupAnswer::Error {
            kind: everruns_internal_protocol::proto::command_error::Kind::BadRequest as i32,
            message: "acme.example has no poppy.json".into(),
        }),
    );
    let error = poppy(&refused, session)
        .unwrap()
        .invoke(call())
        .await
        .unwrap_err();
    assert_eq!(error, "acme.example has no poppy.json");

    let down = Material::new(true, Err(anyhow::anyhow!("connection refused")));
    let error = poppy(&down, session)
        .unwrap()
        .invoke(call())
        .await
        .unwrap_err();
    assert_eq!(error, "Poppy is unavailable right now.");

    let standalone = Material::new(false, Ok(LookupAnswer::Value(json!("never"))));
    assert!(poppy(&standalone, session).is_none());
}

#[tokio::test]
async fn an_orgs_egress_grants_come_from_the_control_plane_as_that_org() {
    let org_id = everruns_contracts::runtime::organization::org_public_id_from_internal(42);
    let org: everruns_contracts::typed_id::OrgId = org_id.parse().unwrap();
    let material = Material::new(
        true,
        Ok(LookupAnswer::Value(
            json!({ "patterns": ["api.acme.example"] }),
        )),
    );
    let weak: Arc<dyn OrgMaterial> = material.clone();
    let grant = super::org_grant(Some(Arc::downgrade(&weak)), &org)
        .await
        .unwrap();
    let extension = grant.extension.expect("granted");
    assert!(extension.is_url_allowed("https://api.acme.example/v1"));
    {
        let asked = material.asked.lock();
        assert_eq!(asked[0].0, org_id);
        assert_eq!(asked[0].1, "worker_get_org_egress_allowlist");
    }

    // No control plane: nothing granted. A failed lookup is an error, which
    // the egress boundary enforces as no grant.
    assert!(super::org_grant(None, &org).await.unwrap().is_empty());
    let down = Material::new(true, Err(anyhow::anyhow!("connection refused")));
    let weak: Arc<dyn OrgMaterial> = down.clone();
    assert!(
        super::org_grant(Some(Arc::downgrade(&weak)), &org)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_platform_call_names_its_turn_and_never_a_person() {
    let material = Material::new(true, Ok(LookupAnswer::Value(json!("3 agents"))));
    let session = SessionId::new();
    let weak: Arc<dyn OrgMaterial> = material.clone();
    let lookups = super::org_lookups(Some(&Arc::downgrade(&weak)), session).unwrap();
    let calls = crate::platform_calls::PlatformCalls::new(lookups, session);

    // Outside a turn there is no person to act as.
    let error = calls
        .call("discover", json!({ "query": "agents" }))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("within a turn"), "{error}");
    assert!(material.asked.lock().is_empty());

    let input = everruns_contracts::typed_id::MessageId::new();
    let output = calls
        .bound(input)
        .call("discover", json!({ "query": "agents" }))
        .await
        .unwrap();
    assert_eq!(output, "3 agents");
    let asked = material.asked.lock();
    assert_eq!(
        asked[0].1,
        everruns_contracts::dataplane::PLATFORM_COMMAND_SURFACE
    );
    assert_eq!(asked[0].2["session_id"], json!(session.to_string()));
    assert_eq!(asked[0].2["input_message_id"], json!(input.to_string()));
    assert!(asked[0].2.get("acting_user").is_none());
}
