use std::sync::Arc;

use everruns_contracts::dataplane::Connection;
use everruns_contracts::host_calls::HostCallAnswer;
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
    answer: Mutex<Option<anyhow::Result<HostCallAnswer>>>,
    asked: Mutex<Vec<(String, String, Value)>>,
    /// What the control plane pushed: settings and secrets, by id.
    pushed: Mutex<std::collections::HashMap<String, Value>>,
    secrets: Mutex<std::collections::HashMap<String, Vec<u8>>>,
    /// The host calls the session's agent declares; `None` for unknown.
    declared: Mutex<Option<std::collections::BTreeSet<&'static str>>>,
    undeclared: Mutex<Vec<String>>,
}

impl Material {
    fn new(reaches: bool, answer: anyhow::Result<HostCallAnswer>) -> Arc<Self> {
        Arc::new(Self {
            reaches,
            answer: Mutex::new(Some(answer)),
            asked: Mutex::new(Vec::new()),
            pushed: Mutex::default(),
            secrets: Mutex::default(),
            declared: Mutex::default(),
            undeclared: Mutex::default(),
        })
    }
}

#[async_trait::async_trait]
impl OrgMaterial for Material {
    fn tenant(&self, _session: SessionId) -> Option<String> {
        Some("org-a".into())
    }

    async fn connection(&self, _org: &str, id: &str) -> anyhow::Result<Option<Connection>> {
        Ok(self.pushed.lock().get(id).map(|settings| Connection {
            connection_id: id.to_owned(),
            kind: everruns_contracts::dataplane::ORG_SETTING.to_owned(),
            settings: settings.clone(),
            token_secret_id: String::new(),
            refresh_owner: Default::default(),
        }))
    }

    async fn secret(&self, _org: &str, _secret_id: &str) -> anyhow::Result<Vec<u8>> {
        anyhow::bail!("no secrets")
    }

    async fn open_secret(
        &self,
        _org: &str,
        secret_id: &str,
    ) -> anyhow::Result<Option<crate::mcp::OpenedSecret>> {
        Ok(self
            .secrets
            .lock()
            .get(secret_id)
            .map(|plain| crate::mcp::OpenedSecret {
                revision: 1,
                plain: plain.clone(),
                data_key: [0; 32],
                data_key_id: "dk".into(),
            }))
    }

    fn looks_up(&self) -> bool {
        self.reaches
    }

    fn declared_host_calls(
        &self,
        _session: SessionId,
    ) -> Option<std::collections::BTreeSet<&'static str>> {
        self.declared.lock().clone()
    }

    fn undeclared_host_call(&self, call: &str) {
        self.undeclared.lock().push(call.to_owned());
    }

    async fn lookup(
        &self,
        org: &str,
        command: &str,
        params: Value,
    ) -> anyhow::Result<HostCallAnswer> {
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
    let material = Material::new(true, Ok(HostCallAnswer::Value(json!("gh-token"))));
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

/// A host call the session's capabilities do not declare is counted, and
/// still made while the declarations are checked against the live fleet;
/// a declared one, or one every turn makes, is not counted.
#[tokio::test]
async fn an_undeclared_host_call_is_counted() {
    let session = SessionId::new();
    let material = Material::new(true, Ok(HostCallAnswer::Value(json!("gh-token"))));
    *material.declared.lock() = Some(Default::default());
    let token = resolver(&material, session)
        .get_connection_token(session, "github")
        .await
        .unwrap();
    assert_eq!(token.as_deref(), Some("gh-token"));
    assert_eq!(*material.undeclared.lock(), ["connection_token"]);

    let material = Material::new(true, Ok(HostCallAnswer::Value(json!("gh-token"))));
    *material.declared.lock() = Some(
        [everruns_contracts::host_calls::CONNECTION_TOKEN]
            .into_iter()
            .collect(),
    );
    resolver(&material, session)
        .get_connection_token(session, "github")
        .await
        .unwrap();
    assert!(material.undeclared.lock().is_empty());

    let material = Material::new(true, Ok(HostCallAnswer::Value(json!({}))));
    *material.declared.lock() = Some(Default::default());
    let lookups = super::org_lookups(
        Some(&(Arc::downgrade(&(material.clone() as Arc<dyn OrgMaterial>)))),
        session,
    )
    .unwrap();
    use everruns_internal_protocol::commands::InternalCommandTransport as _;
    lookups
        .execute_internal_command("worker_get_tool_approval_policy", json!({}))
        .await
        .unwrap()
        .unwrap();
    assert!(material.undeclared.lock().is_empty(), "every turn asks it");
}

#[tokio::test]
async fn sandbox_keys_and_mcp_grants_stay_on_the_data_plane() {
    let material = Material::new(true, Ok(HostCallAnswer::Value(json!("never"))));
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
        Ok(HostCallAnswer::Error {
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
    let material = Material::new(false, Ok(HostCallAnswer::Value(json!("never"))));
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
    let material = Material::new(true, Ok(HostCallAnswer::Value(json!({ "sent": true }))));
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
        Ok(HostCallAnswer::Value(json!({
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
        assert_eq!(asked[0].1, everruns_contracts::host_calls::SLACK_ACTION);
        assert_eq!(asked[0].2["session_id"], json!(session.to_string()));
        assert_eq!(asked[0].2["action"]["action"], json!("add_reaction"));
    }

    // The control plane's refusal is the model's to act on, intact.
    let refused = Material::new(
        true,
        Ok(HostCallAnswer::Value(
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

    let standalone = Material::new(false, Ok(HostCallAnswer::Value(json!("never"))));
    assert!(slack(&standalone, session).is_none());
}

#[tokio::test]
async fn a_company_refusal_reaches_the_model_and_anything_else_is_unavailable() {
    let call = || -> everruns_capabilities::capabilities::PoppyCall {
        serde_json::from_value(json!({ "action": "discover", "company": "acme.example" })).unwrap()
    };
    let session = SessionId::new();
    let refused = Material::new(
        true,
        Ok(HostCallAnswer::Error {
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

    let standalone = Material::new(false, Ok(HostCallAnswer::Value(json!("never"))));
    assert!(poppy(&standalone, session).is_none());
}

#[tokio::test]
async fn an_orgs_egress_grants_come_from_the_control_plane_as_that_org() {
    let org_id = everruns_contracts::runtime::organization::org_public_id_from_internal(42);
    let org: everruns_contracts::typed_id::OrgId = org_id.parse().unwrap();
    let material = Material::new(
        true,
        Ok(HostCallAnswer::Value(
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
    let material = Material::new(true, Ok(HostCallAnswer::Value(json!("3 agents"))));
    let session = SessionId::new();
    let weak: Arc<dyn OrgMaterial> = material.clone();
    let lookups = super::org_lookups(Some(&Arc::downgrade(&weak)), session).unwrap();
    let calls = crate::platform_calls::PlatformCalls::new(lookups, session, None);

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
    assert_eq!(asked[0].1, "platform_command_surface");
    assert_eq!(asked[0].2["session_id"], json!(session.to_string()));
    assert_eq!(asked[0].2["input_message_id"], json!(input.to_string()));
    assert!(asked[0].2.get("acting_user").is_none());
}

/// The org's settings and search keys come from its pushed config, so a
/// tool reads them while the control plane is down; the session owner's
/// own approval settings are still asked, and without an answer the turn
/// runs with none.
#[tokio::test]
async fn pushed_org_settings_are_read_without_asking_the_control_plane() {
    use everruns_contracts::dataplane::{
        TOOL_APPROVAL_SETTING, WEB_SEARCH_SETTING, web_search_key_secret,
    };
    let down = Material::new(true, Err(anyhow::anyhow!("connection refused")));
    let account = uuid::Uuid::new_v4();
    down.pushed.lock().insert(
        WEB_SEARCH_SETTING.into(),
        serde_json::to_value(
            everruns_contracts::runtime::web_search::WebSearchOrgSettings::default(),
        )
        .unwrap(),
    );
    down.pushed.lock().insert(
        TOOL_APPROVAL_SETTING.into(),
        json!({ "allowed_modes": ["normal"] }),
    );
    down.secrets
        .lock()
        .insert(web_search_key_secret(account), b"bs-key".to_vec());
    let session = SessionId::new();
    let resolver = resolver(&down, session);

    let settings = resolver.get_web_search_settings(session).await.unwrap();
    assert!(settings.is_some());
    let key = resolver
        .get_web_search_account_key(session, account)
        .await
        .unwrap();
    assert_eq!(key.as_deref(), Some("bs-key"));
    // Not one of the org's search accounts: none, and nothing asked.
    let other = resolver
        .get_web_search_account_key(session, uuid::Uuid::new_v4())
        .await
        .unwrap();
    assert_eq!(other, None);
    assert!(down.asked.lock().is_empty());

    // The org's policy from the config; the owner's settings could not be
    // asked, so none.
    let policy = resolver
        .get_tool_approval_policy(session)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(policy.org.allowed_modes, vec!["normal".to_owned()]);
    assert_eq!(policy.personal, Default::default());
    let asked = down.asked.lock();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].1, "worker_get_tool_approval_policy");
}

#[tokio::test]
async fn pushed_egress_grants_are_enforced_without_asking() {
    let org = everruns_contracts::typed_id::OrgId::new();
    let down = Material::new(true, Err(anyhow::anyhow!("connection refused")));
    down.pushed.lock().insert(
        everruns_contracts::dataplane::EGRESS_SETTING.into(),
        json!({ "patterns": ["api.example.com"], "open": false }),
    );
    let weak: Arc<dyn OrgMaterial> = down.clone();
    let grant = super::org_grant(Some(Arc::downgrade(&weak)), &org)
        .await
        .unwrap();
    assert!(!grant.open);
    assert!(grant.extension.is_some());
    assert!(down.asked.lock().is_empty());
}
