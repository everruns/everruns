//! The person's own MCP servers for a v2 session (`user_mcp.rs`), on a real
//! database: a cell's calls reach the list of the person whose turn it is,
//! resolved into what an executor runs, and nobody else's.

use super::*;
use crate::domains::data_plane::user_mcp;
use crate::domains::mcp_servers::user_servers::{AddUserMcpServerRequest, UserMcpServers};
use everruns_contracts::typed_id::SessionId;
use everruns_core::McpServerAuthMode;

/// The org's internal caller, as the route builds it.
fn internal_ctx(cp: &ControlPlane, org_id: i64) -> crate::domains::common::Ctx {
    crate::domains::common::Ctx::new(
        everruns_core::Caller::internal(org_id),
        cp.db.clone(),
        Arc::new(crate::services::CapabilityService::new(cp.db.clone(), None)),
        cp.encryption.clone(),
        Arc::new(everruns_core::DefaultPermissionResolver),
    )
}

fn server(name: &str, auth: McpServerAuthMode, api_key: Option<&str>) -> AddUserMcpServerRequest {
    AddUserMcpServerRequest {
        name: Some(name.to_string()),
        url: Some(format!("https://{name}.example.com/mcp")),
        auth_mode: Some(auth),
        api_key: api_key.map(str::to_string),
        ..Default::default()
    }
}

/// A Console Chat session whose turn `message` a person sent, the person
/// with a server that takes their key and one they sign in to.
async fn chat_turn(cp: &ControlPlane) -> (SessionId, uuid::Uuid) {
    let chat = super::console_chat(cp).await;
    // A member of the org, whose turns run as their default end user.
    let person = cp.db.create_test_user(uuid::Uuid::now_v7()).await;
    cp.db
        .add_organization_member(ORG, person, "member")
        .await
        .unwrap();
    let subject = cp.db.default_virtual_user(ORG, person).await.unwrap().id;
    let mut row = crate::storage::test_database::test_session_row(ORG);
    row.harness_id = Some(chat.harness_id);
    row.agent_id = Some(chat.id);
    let session = cp.db.create_session(row).await.unwrap().id;
    let message = uuid::Uuid::now_v7();
    cp.db
        .record_runtime_invocation(
            ORG,
            session,
            message,
            Some(subject),
            Some(person),
            Some(chat.id.uuid()),
        )
        .await
        .unwrap();
    let theirs = UserMcpServers {
        db: &cp.db,
        encryption: cp.encryption.as_deref(),
        org_id: ORG,
        owner: subject.uuid(),
        egress: None,
    };
    theirs
        .add(server("wiki", McpServerAuthMode::ApiKey, Some("wiki-key")))
        .await
        .unwrap();
    theirs
        .add(server("notes", McpServerAuthMode::OAuth, None))
        .await
        .unwrap();
    (session, message)
}

#[tokio::test]
async fn a_cell_gets_the_turns_persons_own_servers_and_their_list() {
    let (cp, _, _) = control_plane().await;
    let (session, message) = chat_turn(&cp).await;
    let ctx = internal_ctx(&cp, ORG);
    let turn =
        json!({ "session_id": session.to_string(), "input_message_id": message.to_string() });

    let answer = user_mcp::servers(&ctx, &turn).await;
    let servers = &answer["ok"];
    // A key travels as the server's header; the turn is the person's.
    assert_eq!(
        servers["wiki"]["url"],
        json!("https://wiki.example.com/mcp"),
        "{answer}"
    );
    assert_eq!(
        servers["wiki"]["headers"]["Authorization"],
        json!("Bearer wiki-key")
    );
    assert!(servers["wiki"].get("actsAs").is_none(), "{answer}");
    // A sign-in keeps its provider, for a per-use token as the person.
    assert_eq!(
        servers["notes"]["url"],
        json!("https://notes.example.com/mcp")
    );
    assert_eq!(servers["notes"]["auth_mode"], json!("oauth"));
    assert_eq!(servers["notes"]["actsAs"], json!("user"));
    assert!(
        servers["notes"]["oauth_provider_id"]
            .as_str()
            .is_some_and(|p| p.starts_with("mcp_oauth_")),
        "{answer}"
    );
    assert!(servers["notes"].get("headers").is_none(), "{answer}");

    // The manage tools read the same list.
    let mut list = turn.clone();
    list["call"] = json!({ "op": "list" });
    let listed = user_mcp::store(&ctx, &list).await;
    let names: Vec<&str> = listed["ok"]["servers"]
        .as_array()
        .unwrap_or_else(|| panic!("{listed}"))
        .iter()
        .filter_map(|s| s["name"].as_str())
        .collect();
    assert!(
        names.contains(&"wiki") && names.contains(&"notes"),
        "{listed}"
    );

    // "This chat only" reaches the session's storage (its cell's, for a
    // session there), with the v1 checks: no longer refused outright.
    let mut chat_only = turn.clone();
    chat_only["call"] = json!({
        "op": "add_to_chat",
        "name": "docs",
        "server": { "url": "https://docs.example.com/mcp" },
    });
    let added = user_mcp::store(&ctx, &chat_only).await;
    assert!(
        added.get("ok").is_some() || added["error"]["kind"] == json!("invalid"),
        "{added}"
    );

    // A call outside a turn has no person: no servers.
    let unattended = json!({ "session_id": session.to_string(), "input_message_id": null });
    assert_eq!(
        user_mcp::servers(&ctx, &unattended).await,
        json!({ "ok": {} })
    );
    // A turn never recorded names no one either.
    let unknown = json!({
        "session_id": session.to_string(),
        "input_message_id": uuid::Uuid::now_v7().to_string(),
    });
    assert_eq!(user_mcp::servers(&ctx, &unknown).await, json!({ "ok": {} }));
    // Another org's caller does not find the session.
    let stranger = internal_ctx(&cp, ORG + 1);
    assert_eq!(
        user_mcp::servers(&stranger, &turn).await,
        json!({ "error": "session not found" })
    );
}

/// The session's own MCP records are read from its storage values, under
/// their reserved prefix only; one that does not parse is left out.
#[test]
fn a_sessions_mcp_records_are_its_reserved_values() {
    let record = everruns_core::session_mcp_servers::SessionMcpServer {
        name: "docs".into(),
        server: serde_json::from_value(json!({ "url": "https://docs.example.com/mcp" })).unwrap(),
        source: everruns_core::session_mcp_servers::SessionMcpServerSource::UserMcp,
    };
    let values = std::collections::BTreeMap::from([
        (
            "session_mcp:docs".to_owned(),
            serde_json::to_string(&record).unwrap(),
        ),
        ("session_mcp:broken".to_owned(), "{".to_owned()),
        ("notes".to_owned(), serde_json::to_string(&record).unwrap()),
    ]);
    let servers = user_mcp::records_of(&values);
    assert_eq!(servers.len(), 1, "{servers:?}");
    assert_eq!(servers["docs"].url, "https://docs.example.com/mcp");
}
