use std::sync::Arc;

use everruns_contracts::dataplane::Connection;
use everruns_contracts::session_sandbox::SessionSandboxCredential;
use everruns_contracts::typed_id::SessionId;
use everruns_core::McpServerActsAs;
use everruns_core::connection_services::UserConnectionResolver;
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::connections;
use crate::providers::OrgMaterial;

/// An org's material: what the control plane pushed, and no control plane
/// server (a standalone shard, or one whose create brought none).
#[derive(Default)]
struct Material {
    /// What the control plane pushed: settings and secrets, by id.
    pushed: Mutex<std::collections::HashMap<String, Value>>,
    secrets: Mutex<std::collections::HashMap<String, Vec<u8>>>,
    owner: Mutex<Option<String>>,
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

    fn owner(&self, _session: SessionId) -> Option<String> {
        self.owner.lock().clone()
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
async fn sandbox_keys_and_mcp_grants_stay_on_the_data_plane() {
    let material = Arc::new(Material::default());
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
}

/// Without the control plane's credentials server (a standalone shard),
/// the tools get what a host without org settings gives: no connections,
/// no search settings, no approval policy beyond the agent's own.
#[tokio::test]
async fn a_shard_without_a_control_plane_answers_as_a_host_without_org_settings() {
    let material = Arc::new(Material::default());
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
    assert!(
        resolver
            .get_tool_approval_policy(session)
            .await
            .unwrap()
            .is_none()
    );
}

/// Anything but the four credential lookups is refused before it leaves
/// the executor.
#[tokio::test]
async fn only_credential_lookups_cross_to_the_control_plane() {
    use everruns_internal_protocol::commands::InternalCommandTransport as _;
    let material = Arc::new(Material::default());
    let weak: Arc<dyn OrgMaterial> = material.clone();
    let lookups = super::org_lookups(Some(&Arc::downgrade(&weak)), SessionId::new()).unwrap();
    for command in [
        "worker_knowledge",
        "worker_poppy_call",
        "worker_get_tool_approval_policy",
        "worker_get_sandbox_connection_token",
        "agents_delete",
    ] {
        let refused = lookups
            .execute_internal_command(command, json!({}))
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(
            refused.kind,
            everruns_internal_protocol::proto::command_error::Kind::Forbidden as i32,
            "{command}"
        );
    }
}

/// The org's settings, search keys and each member's own approval settings
/// come from its pushed config, so a tool reads them while the control
/// plane is down.
#[tokio::test]
async fn pushed_org_settings_are_read_without_asking_the_control_plane() {
    use everruns_contracts::dataplane::{
        TOOL_APPROVAL_SETTING, WEB_SEARCH_SETTING, personal_approval_setting, web_search_key_secret,
    };
    let material = Arc::new(Material::default());
    let account = uuid::Uuid::new_v4();
    material.pushed.lock().insert(
        WEB_SEARCH_SETTING.into(),
        serde_json::to_value(
            everruns_contracts::runtime::web_search::WebSearchOrgSettings::default(),
        )
        .unwrap(),
    );
    material.pushed.lock().insert(
        TOOL_APPROVAL_SETTING.into(),
        json!({ "allowed_modes": ["normal"] }),
    );
    material
        .secrets
        .lock()
        .insert(web_search_key_secret(account), b"bs-key".to_vec());
    let session = SessionId::new();
    let resolver = resolver(&material, session);

    let settings = resolver.get_web_search_settings(session).await.unwrap();
    assert!(settings.is_some());
    let key = resolver
        .get_web_search_account_key(session, account)
        .await
        .unwrap();
    assert_eq!(key.as_deref(), Some("bs-key"));
    // Not one of the org's search accounts: none.
    let other = resolver
        .get_web_search_account_key(session, uuid::Uuid::new_v4())
        .await
        .unwrap();
    assert_eq!(other, None);

    // The org's policy; no owner, or an owner with nothing pushed: none of
    // their own.
    let policy = resolver
        .get_tool_approval_policy(session)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(policy.org.allowed_modes, vec!["normal".to_owned()]);
    assert_eq!(policy.personal, Default::default());
    let owner = uuid::Uuid::new_v4().to_string();
    *material.owner.lock() = Some(owner.clone());
    let none_pushed = resolver
        .get_tool_approval_policy(session)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(none_pushed.personal, Default::default());

    // The owner's own settings, pushed for them.
    let personal = everruns_contracts::tool_approval_policy::PersonalToolApproval {
        default_mode: Some("normal".into()),
        ..Default::default()
    };
    material.pushed.lock().insert(
        personal_approval_setting(&owner),
        serde_json::to_value(&personal).unwrap(),
    );
    let policy = resolver
        .get_tool_approval_policy(session)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(policy.personal, personal.sanitized());
}

#[tokio::test]
async fn pushed_egress_grants_are_enforced_and_nothing_is_granted_without_them() {
    let org = everruns_contracts::typed_id::OrgId::new();
    let material = Arc::new(Material::default());
    let weak: Arc<dyn OrgMaterial> = material.clone();
    // Not pushed yet, or no control plane: nothing granted.
    assert!(
        super::org_grant(Some(Arc::downgrade(&weak)), &org)
            .await
            .is_empty()
    );
    assert!(super::org_grant(None, &org).await.is_empty());
    material.pushed.lock().insert(
        everruns_contracts::dataplane::EGRESS_SETTING.into(),
        json!({ "patterns": ["api.example.com"], "open": false }),
    );
    let grant = super::org_grant(Some(Arc::downgrade(&weak)), &org).await;
    assert!(!grant.open);
    let extension = grant.extension.expect("granted");
    assert!(extension.is_url_allowed("https://api.example.com/v1"));
}

mod credentials_server {
    //! Credential lookups through the control plane's credentials server:
    //! one tool call each, acting as the person who sent the message the
    //! lookup names, from the session's record.

    use everruns_contracts::runtime::capabilities::injected::{
        CREDENTIAL_TOOL, CREDENTIALS, SessionActor,
    };
    use everruns_contracts::typed_id::{MessageId, SessionId};
    use everruns_internal_protocol::commands::InternalCommandTransport;
    use serde_json::{Value, json};

    use crate::controller_fixture::Fixture;

    const URL: &str = "https://93.184.215.14/api/v1/injected/credentials/mcp";

    #[tokio::test]
    async fn a_credential_lookup_acts_as_the_sender_of_the_message_it_names() {
        let fixture = Fixture::new(
            CREDENTIALS,
            URL,
            &[(CREDENTIAL_TOOL, json!({ "ok": { "token": "t0k" } }))],
        );
        let session = SessionId::new();
        let earlier = MessageId::new();
        let open = SessionActor {
            sub: "usr_1".into(),
            turn: Some(MessageId::new().uuid().to_string()),
        };
        fixture.executor.open.lock().insert(session, open.clone());
        fixture
            .executor
            .senders
            .lock()
            .insert(earlier, "usr_2".into());
        let lookups = super::super::org_lookups(Some(&fixture.material), session).unwrap();
        let lookup = |input: Value| {
            let lookups = lookups.clone();
            async move {
                lookups
                    .execute_internal_command(
                        "worker_get_connection_token",
                        json!({
                            "session_id": session.to_string(),
                            "provider": "github",
                            "input_message_id": input,
                            "acting_user": "usr_forged",
                        }),
                    )
                    .await
                    .unwrap()
                    .unwrap()
            }
        };

        assert_eq!(
            lookup(json!(earlier.uuid().to_string())).await,
            json!({ "token": "t0k" })
        );
        lookup(json!("open")).await;
        lookup(Value::Null).await;

        let asked = fixture.asked.lock();
        assert_eq!(asked.len(), 3);
        let (tool, arguments, act) = &asked[0];
        assert_eq!(tool, CREDENTIAL_TOOL);
        assert_eq!(arguments["call"], "connection_token");
        assert_eq!(arguments["params"], json!({ "provider": "github" }));
        assert_eq!(
            act.as_ref(),
            Some(&SessionActor {
                sub: "usr_2".into(),
                turn: Some(earlier.uuid().to_string()),
            })
        );
        assert_eq!(asked[1].2.as_ref(), Some(&open));
        assert_eq!(asked[2].2, None);
    }

    #[tokio::test]
    async fn a_refused_credential_keeps_its_kind() {
        let fixture = Fixture::new(
            CREDENTIALS,
            URL,
            &[(
                CREDENTIAL_TOOL,
                json!({ "error": { "kind": 3, "message": "no connection" } }),
            )],
        );
        let session = SessionId::new();
        let lookups = super::super::org_lookups(Some(&fixture.material), session).unwrap();
        let refused = lookups
            .execute_internal_command(
                "worker_get_github_token",
                json!({ "session_id": session.to_string(), "input_message_id": null }),
            )
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(refused.kind, 3);
        assert_eq!(refused.message, "no connection");
    }

    /// A per-turn MCP grant's token is asked for the session's open turn,
    /// as its person; never kept.
    #[tokio::test]
    async fn an_mcp_grant_token_is_asked_for_the_open_turn() {
        let fixture = Fixture::new(
            CREDENTIALS,
            URL,
            &[(CREDENTIAL_TOOL, json!({ "ok": "mcp-t0k" }))],
        );
        let session = SessionId::new();
        let open = SessionActor {
            sub: "usr_1".into(),
            turn: Some(MessageId::new().uuid().to_string()),
        };
        fixture.executor.open.lock().insert(session, open.clone());
        let lookups = super::super::org_lookups(Some(&fixture.material), session).unwrap();
        for _ in 0..2 {
            let token = lookups
                .open_turn_mcp_token(session, "my_notes", "mcp_oauth_notes", "user")
                .await
                .unwrap();
            assert_eq!(token.as_deref(), Some("mcp-t0k"));
        }
        let asked = fixture.asked.lock();
        assert_eq!(asked.len(), 2, "never kept");
        let (_, arguments, act) = &asked[0];
        assert_eq!(arguments["call"], "mcp_connection_token");
        assert_eq!(
            arguments["params"],
            json!({ "server_prefix": "my_notes", "provider": "mcp_oauth_notes", "acts_as": "user" })
        );
        assert_eq!(act.as_ref(), Some(&open));
    }
}

mod platform_commands_server {
    //! What the control plane's platform commands server answers besides
    //! the platform commands: the session's spend and its agent's saved
    //! scripts.

    use std::sync::Arc;

    use everruns_contracts::runtime::capabilities::injected::{
        PLATFORM_COMMAND_TOOL, PLATFORM_COMMANDS, SessionActor,
    };
    use everruns_contracts::runtime::saved_scripts::{SavedScript, SavedScriptStore};
    use everruns_contracts::runtime::session_spend::SessionSpendReaderExt;
    use everruns_contracts::typed_id::{MessageId, SessionId, TurnId};
    use serde_json::json;

    use crate::controller_fixture::Fixture;

    const URL: &str = "https://93.184.215.14/api/v1/injected/platform_commands/mcp";

    #[tokio::test]
    async fn a_sessions_spend_is_priced_by_the_control_plane_for_no_one() {
        use everruns_contracts::runtime::session_spend::{SessionSpend, SpendBasis};
        let spend = SessionSpend {
            session_id: "session_1".into(),
            total_usd: 0.25,
            basis: SpendBasis::PriceList,
            partial: false,
            models: Default::default(),
            background: Default::default(),
            images: Default::default(),
            sandboxes: Vec::new(),
            subagents: Default::default(),
            current_turn_usd: Some(0.01),
            wasted_usd: 0.0,
            not_included: Vec::new(),
        };
        let fixture = Fixture::new(
            PLATFORM_COMMANDS,
            URL,
            &[(PLATFORM_COMMAND_TOOL, serde_json::to_value(&spend).unwrap())],
        );
        let session = SessionId::new();
        fixture.executor.open.lock().insert(
            session,
            SessionActor {
                sub: "usr_1".into(),
                turn: None,
            },
        );
        let backends = super::super::attach(
            everruns_core::host::HostBackends::in_memory(),
            Arc::new(super::Keys),
            Some(&fixture.material),
            session,
        );
        let extensions = backends.tool_context_extensions_factory.expect("spend")(0, session);
        let reader = extensions
            .get::<SessionSpendReaderExt>()
            .expect("spend")
            .0
            .clone();
        let turn = TurnId::new();
        let read = reader.session_spend(session, Some(turn)).await.unwrap();
        assert_eq!(read, Some(spend));
        let asked = fixture.asked.lock();
        let (tool, arguments, act) = &asked[0];
        assert_eq!(tool, PLATFORM_COMMAND_TOOL);
        assert_eq!(arguments["operation"], "spend");
        assert_eq!(arguments["arguments"]["turn_id"], json!(turn.to_string()));
        assert_eq!(*act, None, "acts for no one");
    }

    #[tokio::test]
    async fn saved_scripts_list_for_no_one_and_save_as_the_turns_sender() {
        let fixture = Fixture::new(
            PLATFORM_COMMANDS,
            URL,
            &[(
                PLATFORM_COMMAND_TOOL,
                json!([{ "name": "deploy", "description": "d", "body": "echo hi" }]),
            )],
        );
        let session = SessionId::new();
        let input = MessageId::new();
        fixture
            .executor
            .senders
            .lock()
            .insert(input, "usr_2".into());
        let store = crate::saved_scripts::ShardSavedScripts::new(
            fixture.material.clone(),
            session,
            URL.to_owned(),
        );
        let listed = store.list().await.unwrap();
        assert_eq!(listed[0].name, "deploy");

        let script = SavedScript {
            name: "deploy".into(),
            description: "d".into(),
            input_schema: None,
            body: "echo hi".into(),
        };
        // Unbound: refused before anything is sent.
        assert!(store.save(script.clone()).await.is_err());
        // The fixture answers a list for the save too, which is no script.
        let bound = store.for_execution(input.uuid()).unwrap();
        assert!(bound.save(script).await.is_err());

        let asked = fixture.asked.lock();
        assert_eq!(asked.len(), 2);
        assert_eq!(asked[0].1["operation"], "scripts");
        assert_eq!(asked[0].2, None);
        assert_eq!(asked[1].1["operation"], "save_script");
        assert_eq!(asked[1].1["arguments"]["name"], "deploy");
        assert_eq!(
            asked[1].2.as_ref().map(|act| act.sub.as_str()),
            Some("usr_2")
        );
    }
}
