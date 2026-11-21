use std::sync::{Arc, Weak};

use everruns_capabilities::capabilities::UserMcpCallInvoker;
use everruns_contracts::dataplane::Connection;
use everruns_contracts::host_calls::{HostCallAnswer, OPEN_TURN, USER_MCP_STORE};
use everruns_contracts::typed_id::{MessageId, SessionId};
use everruns_core::mcp::{UserMcpStoreCall, UserMcpStoreError, UserMcpStoreReply};
use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::mcp::PerTurnGrant;
use crate::providers::OrgMaterial;

/// A control plane whose store answers `answer`; every call is recorded.
struct ControlPlane {
    answer: Value,
    asked: Mutex<Vec<Value>>,
}

#[async_trait::async_trait]
impl OrgMaterial for ControlPlane {
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
        true
    }

    fn declared_host_calls(
        &self,
        _session: SessionId,
    ) -> Option<std::collections::BTreeSet<&'static str>> {
        Some([USER_MCP_STORE].into())
    }

    async fn lookup(
        &self,
        _org: &str,
        command: &str,
        params: Value,
    ) -> anyhow::Result<HostCallAnswer> {
        assert_eq!(command, USER_MCP_STORE);
        self.asked.lock().push(params);
        Ok(HostCallAnswer::Value(self.answer.clone()))
    }
}

fn store(cp: &Arc<ControlPlane>, session: SessionId) -> super::CellUserMcpStore {
    let material: Arc<dyn OrgMaterial> = cp.clone();
    let weak: Weak<dyn OrgMaterial> = Arc::downgrade(&material);
    // Kept alive by the test's `cp`.
    std::mem::forget(material);
    super::CellUserMcpStore {
        via: super::Via::HostCall(crate::lookups::org_lookups(Some(&weak), session).unwrap()),
        session,
    }
}

#[tokio::test]
async fn the_manage_tools_reach_the_persons_list_for_the_open_turn() {
    let cp = Arc::new(ControlPlane {
        answer: json!({ "ok": { "reply": "removed", "removed": true } }),
        asked: Mutex::default(),
    });
    let session = SessionId::new();
    let reply = store(&cp, session)
        .invoke(
            None,
            UserMcpStoreCall::Remove {
                name: "notes".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(reply, UserMcpStoreReply::Removed { removed: true });
    let asked = cp.asked.lock()[0].clone();
    assert_eq!(asked["session_id"], json!(session.to_string()));
    // The session's cell names the turn and its person, not the executor.
    assert_eq!(asked["input_message_id"], json!(OPEN_TURN));
    assert_eq!(asked["call"]["op"], json!("remove"));
    assert_eq!(asked["call"]["name"], json!("notes"));
}

#[tokio::test]
async fn a_refusal_keeps_its_kind() {
    let cp = Arc::new(ControlPlane {
        answer: json!({ "error": { "kind": "unavailable", "message": "not yours" } }),
        asked: Mutex::default(),
    });
    let error = store(&cp, SessionId::new())
        .invoke(None, UserMcpStoreCall::List)
        .await
        .unwrap_err();
    assert_eq!(error, UserMcpStoreError::Unavailable("not yours".into()));
}

#[test]
fn a_persons_grant_serves_only_their_server_at_its_address() {
    let session = SessionId::new();
    let grant = PerTurnGrant {
        provider: "mcp_oauth_notes".into(),
        acts_as: everruns_core::McpServerActsAs::User,
    };
    super::TURNS.lock().insert(
        session,
        super::Turn {
            message: MessageId::new(),
            servers: Default::default(),
            grants: [(
                "notes".to_owned(),
                ("https://notes.example/mcp".to_owned(), grant.clone()),
            )]
            .into(),
        },
    );
    let url = Some("https://notes.example/mcp");
    assert_eq!(super::grant(session.uuid(), "notes", url), Some(grant));
    // An agent's own server of the same name elsewhere: not the person's.
    assert_eq!(
        super::grant(session.uuid(), "notes", Some("https://other.example/mcp")),
        None
    );
    assert_eq!(super::grant(session.uuid(), "docs", url), None);
    assert_eq!(super::grant(SessionId::new().uuid(), "notes", url), None);
}

mod person_server {
    //! The person's servers through the control plane's `person_mcp`
    //! server: each call a tool call with a session token whose `act`
    //! names the person and the open turn, never the call's arguments.

    use std::collections::BTreeMap;
    use std::sync::{Arc, OnceLock};

    use anyhow::Result;
    use async_trait::async_trait;
    use everruns_capabilities::capabilities::UserMcpCallInvoker;
    use everruns_contracts::dataplane::Connection;
    use everruns_contracts::dataplane_seal::Key;
    use everruns_contracts::runtime::capabilities::injected::{
        InjectedCapabilityDefinition, PERSON_MCP, PERSON_MCP_SERVERS_TOOL, PERSON_MCP_STORE_TOOL,
        SessionActor, injected_capability_id,
    };
    use everruns_contracts::tools::ToolExecutionResult;
    use everruns_contracts::typed_id::{MessageId, SessionId};
    use everruns_core::mcp::injected_server::{
        InjectedCapabilityService, VerifiedSession, handle_injected_request,
    };
    use everruns_core::mcp::session_token::{SessionTokenSigner, SessionTokenVerifier};
    use everruns_core::mcp::{UserMcpStoreCall, UserMcpStoreReply};
    use everruns_core::tool_context::ToolContext;
    use everruns_core::tools::Tool;
    use everruns_core::{
        EgressRequest, EgressResponse, EgressResult, EgressService, EgressStreamResponse,
    };
    use parking_lot::Mutex;
    use serde_json::{Value, json};

    use crate::mcp::{ControllerServer, OrgMcp};
    use crate::providers::OrgMaterial;

    const URL: &str = "https://93.184.215.14/api/v1/injected/person_mcp/mcp";
    const KEY: Key = [7u8; 32];

    /// What each tool call carried: tool, arguments, the token's actor.
    type Asked = Arc<Mutex<Vec<(String, Value, Option<SessionActor>)>>>;

    struct Echo {
        name: &'static str,
        asked: Asked,
        actor: Option<SessionActor>,
        answer: Value,
    }

    #[async_trait]
    impl Tool for Echo {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "echo"
        }
        fn parameters_schema(&self) -> Value {
            json!({ "type": "object" })
        }
        async fn execute(&self, arguments: Value) -> ToolExecutionResult {
            self.asked
                .lock()
                .push((self.name.to_owned(), arguments, self.actor.clone()));
            ToolExecutionResult::success(Value::String(self.answer.to_string()))
        }
    }

    struct Service {
        definition: InjectedCapabilityDefinition,
        asked: Asked,
    }

    #[async_trait]
    impl InjectedCapabilityService for Service {
        fn definition(&self) -> &InjectedCapabilityDefinition {
            &self.definition
        }
        async fn tools(&self, session: &VerifiedSession) -> Result<Vec<Box<dyn Tool>>> {
            let tool = |name, answer| -> Box<dyn Tool> {
                Box::new(Echo {
                    name,
                    asked: self.asked.clone(),
                    actor: session.claims.act.clone(),
                    answer,
                })
            };
            Ok(vec![
                tool(
                    PERSON_MCP_STORE_TOOL,
                    json!({ "ok": { "reply": "removed", "removed": true } }),
                ),
                tool(PERSON_MCP_SERVERS_TOOL, json!({ "ok": {} })),
            ])
        }
        async fn tool_context(&self, session: &VerifiedSession) -> Result<ToolContext> {
            Ok(ToolContext::new(SessionId::from_uuid(session.session_id)))
        }
    }

    /// The control plane's endpoint, as the egress calls go out through.
    struct ControlPlane {
        service: Service,
        verifier: SessionTokenVerifier,
    }

    #[async_trait]
    impl EgressService for ControlPlane {
        async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
            assert_eq!(request.url, URL);
            let bearer = request
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                .and_then(|(_, value)| value.strip_prefix("Bearer "))
                .unwrap_or_default()
                .to_string();
            let Ok(claims) =
                self.verifier
                    .verify(&bearer, URL, &injected_capability_id(PERSON_MCP))
            else {
                return Ok(EgressResponse {
                    status: 401,
                    headers: BTreeMap::new(),
                    body: Vec::new(),
                });
            };
            let session = VerifiedSession::from_claims(claims).unwrap();
            let message: Value = serde_json::from_slice(&request.body).unwrap();
            Ok(
                match handle_injected_request(&self.service, &session, message).await {
                    Some(reply) => EgressResponse {
                        status: 200,
                        headers: [("content-type".to_string(), "application/json".to_string())]
                            .into(),
                        body: serde_json::to_vec(&reply).unwrap(),
                    },
                    None => EgressResponse {
                        status: 202,
                        headers: BTreeMap::new(),
                        body: Vec::new(),
                    },
                },
            )
        }

        async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
            let response = self.send(request).await?;
            Ok(EgressStreamResponse {
                status: response.status,
                headers: response.headers,
                body: Box::pin(futures::stream::iter([Ok(response.body)])),
            })
        }
    }

    /// An executor whose session's open turn a person opened. Makes no
    /// host calls.
    struct Executor {
        session: SessionId,
        actor: SessionActor,
        mcp: OnceLock<Arc<OrgMcp>>,
    }

    #[async_trait]
    impl OrgMaterial for Executor {
        fn tenant(&self, session: SessionId) -> Option<String> {
            (session == self.session).then(|| "org-a".into())
        }
        async fn connection(&self, _: &str, _: &str) -> anyhow::Result<Option<Connection>> {
            Ok(None)
        }
        async fn secret(&self, _: &str, _: &str) -> anyhow::Result<Vec<u8>> {
            anyhow::bail!("no secrets")
        }
        fn data_plane_key(&self) -> Option<Key> {
            Some(KEY)
        }
        fn turn_actor(&self, session: SessionId) -> Option<SessionActor> {
            (session == self.session).then(|| self.actor.clone())
        }
        fn mcp(&self) -> Option<Arc<OrgMcp>> {
            self.mcp.get().cloned()
        }
        async fn lookup(
            &self,
            _: &str,
            command: &str,
            _: Value,
        ) -> anyhow::Result<everruns_contracts::host_calls::HostCallAnswer> {
            panic!("the person's MCP servers made host call {command}")
        }
    }

    #[tokio::test]
    async fn the_store_and_the_turns_servers_go_to_the_person_mcp_server_as_the_turns_person() {
        let signer = SessionTokenSigner::from_pkcs8(
            everruns_contracts::dataplane_seal::SESSION_TOKEN_ISSUER,
            &everruns_contracts::dataplane_seal::session_token_pkcs8(&KEY),
        )
        .unwrap();
        let asked = Asked::default();
        let cp = Arc::new(ControlPlane {
            service: Service {
                definition: serde_json::from_value(json!({
                    "name": PERSON_MCP,
                    "description": "person",
                    "server": { "url": URL },
                }))
                .unwrap(),
                asked: asked.clone(),
            },
            verifier: SessionTokenVerifier::from_jwks(&signer.jwks(), vec![signer.issuer().into()])
                .unwrap(),
        });
        let session = SessionId::new();
        let turn = MessageId::new();
        let actor = SessionActor {
            sub: "usr_1".into(),
            turn: Some(turn.uuid().to_string()),
        };
        let executor = Arc::new(Executor {
            session,
            actor: actor.clone(),
            mcp: OnceLock::new(),
        });
        let mcp = Arc::new(OrgMcp::new(cp.clone()));
        let material: Arc<dyn OrgMaterial> = executor.clone();
        let weak = Arc::downgrade(&material);
        mcp.bind(weak.clone());
        let _ = executor.mcp.set(mcp);
        let server = ControllerServer::for_session(Some(&weak), session, PERSON_MCP, URL).unwrap();
        let via = super::super::Via::Server(server);

        let store = super::super::CellUserMcpStore {
            via: via.clone(),
            session,
        };
        let reply = store
            .invoke(
                None,
                UserMcpStoreCall::Remove {
                    name: "notes".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(reply, UserMcpStoreReply::Removed { removed: true });
        {
            let asked = asked.lock();
            let (tool, arguments, act) = &asked[0];
            assert_eq!(tool, PERSON_MCP_STORE_TOOL);
            assert_eq!(arguments["call"]["op"], json!("remove"));
            // The person and the turn come with the token only.
            assert!(arguments.get("session_id").is_none());
            assert!(arguments.get("input_message_id").is_none());
            assert_eq!(act.as_ref(), Some(&actor));
        }

        let answer = via
            .answer(
                everruns_contracts::host_calls::USER_MCP_SERVERS,
                json!({}),
                PERSON_MCP_SERVERS_TOOL,
                json!({ "person": true, "session_servers": false }),
            )
            .await
            .unwrap();
        assert_eq!(answer, json!({ "ok": {} }));
        let asked = asked.lock();
        assert_eq!(asked[1].0, PERSON_MCP_SERVERS_TOOL);
        assert_eq!(asked[1].1["person"], json!(true));
        drop(material);
    }
}
