use everruns_contracts::typed_id::{MessageId, SessionId};

use crate::mcp::PerTurnGrant;

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

    use everruns_capabilities::capabilities::UserMcpCallInvoker;
    use everruns_contracts::runtime::capabilities::injected::{
        PERSON_MCP, PERSON_MCP_SERVERS_TOOL, PERSON_MCP_STORE_TOOL, SessionActor,
    };
    use everruns_contracts::typed_id::{MessageId, SessionId};
    use everruns_core::mcp::{UserMcpStoreCall, UserMcpStoreReply};
    use serde_json::json;

    use crate::controller_fixture::Fixture;
    use crate::mcp::ControllerServer;

    const URL: &str = "https://93.184.215.14/api/v1/injected/person_mcp/mcp";

    #[tokio::test]
    async fn the_store_and_the_turns_servers_go_to_the_person_mcp_server_as_the_turns_person() {
        let fixture = Fixture::new(
            PERSON_MCP,
            URL,
            &[
                (
                    PERSON_MCP_STORE_TOOL,
                    json!({ "ok": { "reply": "removed", "removed": true } }),
                ),
                (PERSON_MCP_SERVERS_TOOL, json!({ "ok": {} })),
            ],
        );
        let session = SessionId::new();
        let actor = SessionActor {
            sub: "usr_1".into(),
            turn: Some(MessageId::new().uuid().to_string()),
        };
        fixture.executor.open.lock().insert(session, actor.clone());
        let server =
            ControllerServer::for_session(Some(&fixture.material), session, PERSON_MCP, URL)
                .unwrap();
        let store = super::super::ShardUserMcpStore {
            via: server.clone(),
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
            let asked = fixture.asked.lock();
            let (tool, arguments, act) = &asked[0];
            assert_eq!(tool, PERSON_MCP_STORE_TOOL);
            assert_eq!(arguments["call"]["op"], json!("remove"));
            // The person and the turn come with the token only.
            assert!(arguments.get("session_id").is_none());
            assert!(arguments.get("input_message_id").is_none());
            assert_eq!(act.as_ref(), Some(&actor));
        }

        let answer = super::super::answer(
            &server,
            PERSON_MCP_SERVERS_TOOL,
            json!({ "person": true, "session_servers": false }),
        )
        .await
        .unwrap();
        assert_eq!(answer, json!({ "ok": {} }));
        let asked = fixture.asked.lock();
        assert_eq!(asked[1].0, PERSON_MCP_SERVERS_TOOL);
        assert_eq!(asked[1].1["person"], json!(true));
    }

    #[tokio::test]
    async fn a_refusal_keeps_its_kind() {
        let fixture = Fixture::new(
            PERSON_MCP,
            URL,
            &[(
                PERSON_MCP_STORE_TOOL,
                json!({ "error": { "kind": "unavailable", "message": "not yours" } }),
            )],
        );
        let session = SessionId::new();
        let server =
            ControllerServer::for_session(Some(&fixture.material), session, PERSON_MCP, URL)
                .unwrap();
        let error = super::super::ShardUserMcpStore { via: server }
            .invoke(None, UserMcpStoreCall::List)
            .await
            .unwrap_err();
        assert_eq!(
            error,
            everruns_core::mcp::UserMcpStoreError::Unavailable("not yours".into())
        );
    }
}
