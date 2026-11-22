//! A tool's call-time lookups on the data plane (data plane v2, step
//! 11.6b): an executor asks through its directory shard, the shard passes the
//! request to the control plane over the link, and the answer comes back
//! sealed with the org's data key for the executor to open.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::sync::Arc;

use everruns_agentd::providers::OrgMaterial;
use everruns_contracts::host_calls::HostCallAnswer;
const PLATFORM_COMMAND_SURFACE: &str = "platform_command_surface";
use everruns_contracts::session_sandbox::SessionSandboxCredential;
use everruns_contracts::typed_id::{MessageId, SessionId};
use everruns_core::connection_services::UserConnectionResolver;
use serde_json::json;
use support::org::{DATA_KEY, DATA_KEY_ID, ORG, org_executor, org_fleet, session};

/// The sandbox keys a shard resolves itself: none here.
struct NoKeys;

#[async_trait::async_trait]
impl UserConnectionResolver for NoKeys {
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

#[tokio::test(flavor = "multi_thread")]
async fn a_tools_lookup_reaches_the_control_plane_and_comes_back_for_its_org() {
    let (fleet, cp) = org_fleet(&[]).await;
    cp.answer_lookups(ORG, DATA_KEY, DATA_KEY_ID);
    cp.set_lookup(
        "worker_get_connection_token",
        HostCallAnswer::Value(json!("gh-token-1")),
    );

    // A turn on the executor, which then knows the session's org.
    let session = session(&fleet, &cp).await;
    fleet.say(&session, "ping").await;
    fleet.turns(&session, 1).await;
    let id: SessionId = session.parse().unwrap();

    let material: Arc<dyn OrgMaterial> = org_executor(&fleet);
    let resolver = everruns_agentd::lookups::connections(
        Arc::new(NoKeys),
        Some(&Arc::downgrade(&material)),
        id,
    );
    let token = resolver.get_connection_token(id, "github").await.unwrap();
    assert_eq!(token.as_deref(), Some("gh-token-1"));
    let asked = cp.lookups();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0].0, ORG);
    assert_eq!(asked[0].1, "worker_get_connection_token");
    assert_eq!(asked[0].2["session_id"], json!(session));
    assert_eq!(asked[0].2["provider"], json!("github"));
    // Outside a turn it acts as no one.
    assert_eq!(asked[0].2["acting_user"], serde_json::Value::Null);

    // The control plane's refusal is the tool's failure, never a default.
    let error = resolver.get_web_search_settings(id).await.unwrap_err();
    assert!(error.to_string().contains("Session not found"), "{error}");

    // Sandbox keys and MCP grants stay with the data plane.
    assert_eq!(
        resolver
            .get_sandbox_connection_token(id, "daytona", &SessionSandboxCredential::default())
            .await
            .unwrap()
            .as_deref(),
        Some("sandbox-key")
    );
    assert_eq!(
        resolver
            .get_connection_token(id, "mcp_oauth_docs")
            .await
            .unwrap(),
        None
    );
    assert_eq!(cp.lookups().len(), 2);

    // The org's knowledge, for the `knowledge` tools, the same way.
    cp.set_lookup(
        "worker_knowledge",
        HostCallAnswer::Value(json!({ "results": [], "warnings": ["empty"], "truncated": false })),
    );
    let backends = everruns_agentd::lookups::attach(
        everruns_core::host::HostBackends::in_memory(),
        Arc::new(NoKeys),
        Some(&Arc::downgrade(&material)),
        id,
    );
    let factory = backends
        .tool_context_extensions_factory
        .expect("knowledge installed");
    let knowledge = factory(0, id)
        .get::<everruns_contracts::knowledge::KnowledgeRuntimeExt>()
        .expect("the knowledge runtime");
    let query: everruns_contracts::knowledge::KnowledgeQuery =
        serde_json::from_value(json!({ "query": "refunds" })).unwrap();
    let found = knowledge.0.search(id, query).await.unwrap();
    assert_eq!(found.warnings, vec!["empty".to_owned()]);
    let asked = cp.lookups();
    assert_eq!(asked.len(), 3, "{asked:?}");
    assert_eq!(asked[2].1, "worker_knowledge");
    assert_eq!(asked[2].2["operation"], json!("search"));
    assert_eq!(asked[2].2["query"]["query"], json!("refunds"));

    // A session the executor does not know is no org's: nothing is asked.
    let stranger = SessionId::new();
    let resolver = everruns_agentd::lookups::connections(
        Arc::new(NoKeys),
        Some(&Arc::downgrade(&material)),
        stranger,
    );
    assert_eq!(
        resolver
            .get_connection_token(stranger, "github")
            .await
            .unwrap(),
        None
    );
    assert_eq!(cp.lookups().len(), 3);
}

/// Admit `text` as `id` on `session` with `sender`, as a trusted admission
/// path does (the gateway from a verified org token, or the control plane),
/// on whichever node holds the session.
async fn send_as(fleet: &support::Fleet, session: &str, id: MessageId, sender: &str) {
    say_as(fleet, session, id, sender, "list my agents").await;
}

/// [`send_as`], saying `text`.
async fn say_as(fleet: &support::Fleet, session: &str, id: MessageId, sender: &str, text: &str) {
    let mut replies = Vec::new();
    for base in fleet.bases() {
        let reply = fleet
            .http
            .post(format!("{base}/shard/sessions/{session}/messages"))
            .json(&json!({ "text": text, "id": id, "sender": sender }))
            .send()
            .await
            .unwrap();
        let status = reply.status();
        let text = reply.text().await.unwrap();
        let body: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        if body["reply"] == "ok" {
            return;
        }
        replies.push(format!("{status} {text}"));
    }
    panic!("no node took the message: {replies:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_platform_call_acts_as_the_person_the_sessions_shard_names() {
    let (fleet, cp) = org_fleet(&[]).await;
    cp.answer_lookups(ORG, DATA_KEY, DATA_KEY_ID);
    cp.set_lookup(
        PLATFORM_COMMAND_SURFACE,
        HostCallAnswer::Value(json!("3 agents")),
    );
    let session = session(&fleet, &cp).await;
    fleet.say(&session, "ping").await;
    fleet.turns(&session, 1).await;

    let person = uuid::Uuid::new_v4().to_string();
    let theirs = MessageId::new();
    send_as(&fleet, &session, theirs, &person).await;
    fleet.turns(&session, 2).await;

    // The executor asks with a person of its own choosing: the shard that
    // forwards the call replaces it with the one who sent the message.
    let material: Arc<dyn OrgMaterial> = org_executor(&fleet);
    let ask = |message: MessageId| {
        json!({
            "session_id": session,
            "input_message_id": message,
            "operation": "discover",
            "arguments": { "query": "agents" },
            "acting_user": "someone-else",
        })
    };
    let answer = material
        .lookup(ORG, PLATFORM_COMMAND_SURFACE, ask(theirs))
        .await
        .unwrap();
    assert_eq!(answer, HostCallAnswer::Value(json!("3 agents")));
    let asked = cp.lookups();
    assert_eq!(asked[0].1, PLATFORM_COMMAND_SURFACE);
    assert_eq!(asked[0].2["acting_user"], json!(person));
    assert_eq!(asked[0].2["operation"], json!("discover"));

    // A message no one is recorded for (the first, sent without a verified
    // token) acts as no one.
    let first = MessageId::new();
    material
        .lookup(ORG, PLATFORM_COMMAND_SURFACE, ask(first))
        .await
        .unwrap();
    assert_eq!(cp.lookups()[1].2["acting_user"], serde_json::Value::Null);

    // A connection lookup of the person's turn names them too (the
    // resolver names the message as a bare UUID), so the control plane
    // resolves their connection, not the session owner's.
    cp.set_lookup(
        "worker_get_connection_token",
        HostCallAnswer::Value(json!("their-token")),
    );
    let id: SessionId = session.parse().unwrap();
    let resolver = everruns_agentd::lookups::connections(
        Arc::new(NoKeys),
        Some(&Arc::downgrade(&material)),
        id,
    )
    .for_execution(theirs.uuid())
    .expect("bound to the turn");
    let token = resolver.get_connection_token(id, "linear").await.unwrap();
    assert_eq!(token.as_deref(), Some("their-token"));
    let asked = cp.lookups();
    assert_eq!(asked[2].1, "worker_get_connection_token");
    assert_eq!(asked[2].2["input_message_id"], json!(theirs.uuid()));
    assert_eq!(asked[2].2["acting_user"], json!(person));
}

/// An MCP server acting as the turn's person asks for its token by session
/// (data plane v2 parity gap 8): the session's shard names the turn it has
/// open and the person who sent it, whatever the executor says; with no
/// turn open the call names no one.
#[tokio::test(flavor = "multi_thread")]
async fn an_mcp_token_for_the_open_turn_acts_as_its_sender() {
    const MCP_TOKEN: &str = everruns_agentd::mcp::MCP_CONNECTION_COMMAND;
    let (fleet, cp) = org_fleet(&[]).await;
    cp.answer_lookups(ORG, DATA_KEY, DATA_KEY_ID);
    cp.set_lookup(MCP_TOKEN, HostCallAnswer::Value(json!("their-mcp-token")));

    // A session whose turn parks on a person's approval, so it stays open.
    let session = SessionId::new().to_string();
    let (status, created) = cp
        .call(
            &fleet.base(),
            reqwest::Method::POST,
            &format!("/dp/v1/orgs/{ORG}/sessions"),
            &json!({
                "id": session,
                "agent_ref": "helper",
                "definition": everruns_agentd_testkit::definition("approve").unwrap(),
            }),
        )
        .await
        .unwrap();
    assert_eq!(status, reqwest::StatusCode::CREATED, "{created}");
    let person = uuid::Uuid::new_v4().to_string();
    let theirs = MessageId::new();
    send_as(&fleet, &session, theirs, &person).await;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !fleet
        .events(&session)
        .await
        .iter()
        .any(|e| e["type"] == "tool.call_requested")
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the turn never parked"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let material: Arc<dyn OrgMaterial> = org_executor(&fleet);
    let ask = |session: &str| {
        json!({
            "session_id": session,
            "server_prefix": "notes",
            "provider": "mcp_oauth_notes",
            "acts_as": "user",
            "input_message_id": everruns_contracts::host_calls::OPEN_TURN,
            "acting_user": "someone-else",
        })
    };
    let answer = material
        .lookup(ORG, MCP_TOKEN, ask(&session))
        .await
        .unwrap();
    assert_eq!(answer, HostCallAnswer::Value(json!("their-mcp-token")));
    let asked = cp.lookups();
    let (_, command, params) = asked.last().unwrap();
    assert_eq!(command, MCP_TOKEN);
    assert_eq!(params["input_message_id"], json!(theirs.uuid()));
    assert_eq!(params["acting_user"], json!(person));

    // A session with no turn open: no message, no one.
    let idle = support::org::session(&fleet, &cp).await;
    material.lookup(ORG, MCP_TOKEN, ask(&idle)).await.unwrap();
    let asked = cp.lookups();
    let (_, _, params) = asked.last().unwrap();
    assert_eq!(params["input_message_id"], serde_json::Value::Null);
    assert_eq!(params["acting_user"], serde_json::Value::Null);
}

/// The shell's `everruns` command (data plane v2 parity gap 7): an agent
/// with a shell and the `platform` capability runs `everruns ...` in its
/// shell, and the line goes to the control plane's catalog as the
/// `execute` tool's would, acting as the person who sent the turn.
#[tokio::test(flavor = "multi_thread")]
async fn the_shells_everruns_command_runs_the_catalog_as_the_turns_person() {
    let (fleet, cp) = org_fleet(&[]).await;
    cp.answer_lookups(ORG, DATA_KEY, DATA_KEY_ID);
    cp.set_lookup(
        PLATFORM_COMMAND_SURFACE,
        HostCallAnswer::Value(json!("3 agents")),
    );

    let session = SessionId::new().to_string();
    let mut definition = everruns_agentd_testkit::definition("files").unwrap();
    definition["capabilities"] = json!([{ "ref": "bashkit_shell" }, { "ref": "platform" }]);
    let (status, created) = cp
        .call(
            &fleet.base(),
            reqwest::Method::POST,
            &format!("/dp/v1/orgs/{ORG}/sessions"),
            &json!({ "id": session, "agent_ref": "shell", "definition": definition }),
        )
        .await
        .unwrap();
    assert_eq!(status, reqwest::StatusCode::CREATED, "{created}");

    let person = uuid::Uuid::new_v4().to_string();
    let theirs = MessageId::new();
    say_as(&fleet, &session, theirs, &person, "sh everruns agents list").await;
    fleet.turns(&session, 1).await;

    let asked = cp.lookups();
    let events = fleet.events(&session).await;
    let (_, _, params) = asked
        .iter()
        .find(|(_, command, _)| command == PLATFORM_COMMAND_SURFACE)
        .unwrap_or_else(|| panic!("the shell never asked the catalog: {asked:?}\n{events:#?}"));
    assert_eq!(params["operation"], json!("execute"));
    assert_eq!(
        params["arguments"]["commands"],
        json!("everruns agents list")
    );
    assert_eq!(params["input_message_id"], json!(theirs.to_string()));
    assert_eq!(params["acting_user"], json!(person));

    // The catalog's answer is the shell's output.
    let completed = events
        .iter()
        .find(|e| e["type"] == "tool.completed")
        .unwrap_or_else(|| panic!("no tool call completed: {events:?}"));
    assert!(completed.to_string().contains("3 agents"), "{completed}");
}

/// A session's Memory (data plane v2 parity, Phase B): the control plane
/// names the session's Memory mounts with the create, and the agent's file
/// tools reach them in the control plane, one host call per operation. A
/// session without Memory never calls.
#[tokio::test(flavor = "multi_thread")]
async fn memory_files_are_read_from_the_control_plane() {
    const MEMORY_FILE: &str = everruns_contracts::host_calls::MEMORY_FILE;
    let (fleet, cp) = org_fleet(&[]).await;
    cp.answer_lookups(ORG, DATA_KEY, DATA_KEY_ID);
    cp.set_lookup(
        MEMORY_FILE,
        HostCallAnswer::Value(json!({ "ok": {
            "id": uuid::Uuid::new_v4(),
            "session_id": uuid::Uuid::new_v4(),
            "path": "/memory/user/notes.md",
            "name": "notes.md",
            "content": "likes green tea",
            "encoding": "text",
            "is_directory": false,
            "is_readonly": false,
            "size_bytes": 15,
            "created_at": "2026-11-19T00:00:00Z",
            "updated_at": "2026-11-19T00:00:00Z",
        }})),
    );

    let create = |session: &str, memory: serde_json::Value| {
        json!({
            "id": session,
            "agent_ref": "files",
            "definition": everruns_agentd_testkit::definition("files").unwrap(),
            "memory": memory,
        })
    };
    let session = SessionId::new().to_string();
    let (status, created) = cp
        .call(
            &fleet.base(),
            reqwest::Method::POST,
            &format!("/dp/v1/orgs/{ORG}/sessions"),
            &create(&session, json!(["/memory/user"])),
        )
        .await
        .unwrap();
    assert_eq!(status, reqwest::StatusCode::CREATED, "{created}");
    let person = uuid::Uuid::new_v4().to_string();
    say_as(
        &fleet,
        &session,
        MessageId::new(),
        &person,
        "read /memory/user/notes.md",
    )
    .await;
    fleet.turns(&session, 1).await;

    let asked = cp.lookups();
    let events = fleet.events(&session).await;
    let (_, _, params) = asked
        .iter()
        .find(|(_, command, _)| command == MEMORY_FILE)
        .unwrap_or_else(|| panic!("Memory was never asked: {asked:?}\n{events:#?}"));
    assert_eq!(params["op"], json!("read"));
    assert_eq!(params["path"], json!("/memory/user/notes.md"));
    assert_eq!(params["session_id"], json!(session));
    let completed = events
        .iter()
        .find(|e| e["type"] == "tool.completed")
        .unwrap_or_else(|| panic!("no tool call completed: {events:?}"));
    assert!(
        completed.to_string().contains("likes green tea"),
        "{completed}"
    );

    // A mount the control plane may not give is refused at the create.
    let (status, _) = cp
        .call(
            &fleet.base(),
            reqwest::Method::POST,
            &format!("/dp/v1/orgs/{ORG}/sessions"),
            &create(&SessionId::new().to_string(), json!(["/workspace"])),
        )
        .await
        .unwrap();
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);

    // No Memory named: the same read stays in the session's own files.
    let before = cp
        .lookups()
        .iter()
        .filter(|(_, command, _)| command == MEMORY_FILE)
        .count();
    let plain = SessionId::new().to_string();
    let (status, created) = cp
        .call(
            &fleet.base(),
            reqwest::Method::POST,
            &format!("/dp/v1/orgs/{ORG}/sessions"),
            &create(&plain, json!([])),
        )
        .await
        .unwrap();
    assert_eq!(status, reqwest::StatusCode::CREATED, "{created}");
    say_as(
        &fleet,
        &plain,
        MessageId::new(),
        &person,
        "read /memory/user/notes.md",
    )
    .await;
    fleet.turns(&plain, 1).await;
    let after = cp
        .lookups()
        .iter()
        .filter(|(_, command, _)| command == MEMORY_FILE)
        .count();
    assert_eq!(before, after);
}

/// The person's own MCP servers (`user_mcp`, Console Chat on shards): a turn
/// a person sent asks the control plane for their servers once, as that
/// person, and the agent calls them with the person's key; a server that
/// signs in as the person gets the person's token per use. A turn nobody
/// sent never asks.
#[tokio::test(flavor = "multi_thread")]
async fn a_persons_own_mcp_servers_join_their_turns() {
    use everruns_agentd_testkit::stub_mcp;
    const USER_MCP_SERVERS: &str = everruns_contracts::host_calls::USER_MCP_SERVERS;
    const MCP_TOKEN: &str = everruns_agentd::mcp::MCP_CONNECTION_COMMAND;
    let (fleet, cp) = org_fleet(&[]).await;
    cp.answer_lookups(ORG, DATA_KEY, DATA_KEY_ID);
    let authorizations = || stub_mcp::AUTHORIZATIONS.lock().unwrap().clone();
    let create = |session: &str| {
        let mut definition = everruns_agentd_testkit::definition("files").unwrap();
        definition["capabilities"] = json!([{ "ref": "user_mcp" }]);
        json!({ "id": session, "agent_ref": "chat", "definition": definition })
    };
    let asked_for = |command: &str| {
        cp.lookups()
            .into_iter()
            .filter(|(_, c, _)| c == command)
            .map(|(_, _, params)| params)
            .collect::<Vec<_>>()
    };

    // A server with the person's own key.
    cp.set_lookup(
        USER_MCP_SERVERS,
        HostCallAnswer::Value(json!({ "ok": {
            "notes": { "url": stub_mcp::URL, "headers": { "Authorization": "Bearer person-key" } },
        }})),
    );
    let session = SessionId::new().to_string();
    let (status, created) = cp
        .call(
            &fleet.base(),
            reqwest::Method::POST,
            &format!("/dp/v1/orgs/{ORG}/sessions"),
            &create(&session),
        )
        .await
        .unwrap();
    assert_eq!(status, reqwest::StatusCode::CREATED, "{created}");
    let person = uuid::Uuid::new_v4().to_string();
    let theirs = MessageId::new();
    say_as(&fleet, &session, theirs, &person, "mcp green tea").await;
    fleet.turns(&session, 1).await;

    let events = fleet.events(&session).await;
    let asked = asked_for(USER_MCP_SERVERS);
    assert_eq!(
        asked.len(),
        1,
        "asked once for the turn: {asked:?}\n{events:#?}"
    );
    assert_eq!(asked[0]["session_id"], json!(session));
    assert_eq!(
        asked[0]["input_message_id"],
        json!(theirs.uuid().to_string())
    );
    assert_eq!(asked[0]["acting_user"], json!(person));
    let completed = events
        .iter()
        .find(|e| e["type"] == "tool.completed")
        .unwrap_or_else(|| panic!("no tool call completed: {events:#?}"));
    assert!(
        completed.to_string().contains("stub found green tea"),
        "{completed}"
    );
    assert!(
        authorizations().contains(&"Bearer person-key".to_owned()),
        "{:?}",
        authorizations()
    );

    // A server that signs in as the person: its token per use, as them.
    cp.set_lookup(
        USER_MCP_SERVERS,
        HostCallAnswer::Value(json!({ "ok": {
            "notes": {
                "url": stub_mcp::URL,
                "auth_mode": "oauth",
                "oauth_provider_id": "mcp_oauth_notes",
                "actsAs": "user",
            },
        }})),
    );
    cp.set_lookup(MCP_TOKEN, HostCallAnswer::Value(json!("their-mcp-token")));
    let second = MessageId::new();
    say_as(&fleet, &session, second, &person, "mcp oolong").await;
    fleet.turns(&session, 2).await;
    assert_eq!(asked_for(USER_MCP_SERVERS).len(), 2);
    let tokens = asked_for(MCP_TOKEN);
    let events = fleet.events(&session).await;
    let token = tokens
        .last()
        .unwrap_or_else(|| panic!("the token was never asked: {events:#?}"));
    assert_eq!(token["provider"], json!("mcp_oauth_notes"));
    assert_eq!(token["acts_as"], json!("user"));
    assert_eq!(token["server_prefix"], json!("notes"));
    assert_eq!(token["acting_user"], json!(person));
    assert!(
        authorizations().contains(&"Bearer their-mcp-token".to_owned()),
        "{:?}",
        authorizations()
    );

    // A message nobody sent (a channel's, a schedule's): no person, no call.
    let mut replies = Vec::new();
    let mut taken = false;
    for base in fleet.bases() {
        let reply = fleet
            .http
            .post(format!("{base}/shard/sessions/{session}/messages"))
            .json(&json!({ "text": "mcp jasmine", "id": MessageId::new() }))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = reply.json().await.unwrap_or_default();
        if body["reply"] == "ok" {
            taken = true;
            break;
        }
        replies.push(body);
    }
    assert!(taken, "no node took the message: {replies:?}");
    fleet.turns(&session, 3).await;
    assert_eq!(asked_for(USER_MCP_SERVERS).len(), 2);
}

/// The session's own MCP records (a server added to this chat only, an
/// ARD MCP target) join any agent's turns, as on v1: a turn of a session
/// with a record asks the control plane for them resolved, naming no
/// person when nobody sent it; a session without one never asks.
#[tokio::test(flavor = "multi_thread")]
async fn a_sessions_own_mcp_servers_join_its_turns() {
    use everruns_agentd_testkit::stub_mcp;
    const USER_MCP_SERVERS: &str = everruns_contracts::host_calls::USER_MCP_SERVERS;
    let (fleet, cp) = org_fleet(&[]).await;
    cp.answer_lookups(ORG, DATA_KEY, DATA_KEY_ID);
    let asked = || {
        cp.lookups()
            .into_iter()
            .filter(|(_, c, _)| c == USER_MCP_SERVERS)
            .map(|(_, _, params)| params)
            .collect::<Vec<_>>()
    };
    cp.set_lookup(
        USER_MCP_SERVERS,
        HostCallAnswer::Value(json!({ "ok": {
            "notes": { "url": stub_mcp::URL, "headers": { "Authorization": "Bearer chat-key" } },
        }})),
    );
    // An agent without `user_mcp`.
    let session = SessionId::new().to_string();
    let definition = everruns_agentd_testkit::definition("files").unwrap();
    let (status, created) = cp
        .call(
            &fleet.base(),
            reqwest::Method::POST,
            &format!("/dp/v1/orgs/{ORG}/sessions"),
            &json!({ "id": session, "agent_ref": "files", "definition": definition }),
        )
        .await
        .unwrap();
    assert_eq!(status, reqwest::StatusCode::CREATED, "{created}");

    // No record yet: the turn asks for nothing.
    say_as(
        &fleet,
        &session,
        MessageId::new(),
        &uuid::Uuid::new_v4().to_string(),
        "hello",
    )
    .await;
    fleet.turns(&session, 1).await;
    assert!(asked().is_empty(), "{:?}", asked());

    // A record in the session's storage, as "this chat only" writes it.
    let record = json!({
        "name": "notes",
        "server": { "url": stub_mcp::URL },
        "source": { "type": "user_mcp" },
    });
    let (status, set) = support::org::command(
        &fleet,
        &cp,
        ORG,
        &session,
        "storage",
        json!({ "kind": "set_value", "key": "session_mcp:notes", "value": record.to_string() }),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK, "{set}");
    let message = MessageId::new();
    say_as(
        &fleet,
        &session,
        message,
        &uuid::Uuid::new_v4().to_string(),
        "mcp green tea",
    )
    .await;
    fleet.turns(&session, 2).await;

    let events = fleet.events(&session).await;
    let asked = asked();
    assert_eq!(asked.len(), 1, "{asked:?}\n{events:#?}");
    assert_eq!(asked[0]["session_id"], json!(session));
    assert_eq!(asked[0]["session_servers"], json!(true));
    // Not a `user_mcp` agent: the person's servers are not asked for.
    assert!(asked[0].get("input_message_id").is_none(), "{:?}", asked[0]);
    assert!(
        events.iter().any(
            |e| e["type"] == "tool.completed" && e.to_string().contains("stub found green tea")
        ),
        "{events:#?}"
    );
    assert!(
        stub_mcp::AUTHORIZATIONS
            .lock()
            .unwrap()
            .contains(&"Bearer chat-key".to_owned())
    );
}

/// A tool the client runs (the console's page tools, Console Chat on
/// shards): the session's definition declares it, the turn parks on its
/// call until the browser answers, and the answer reaches the model.
#[tokio::test(flavor = "multi_thread")]
async fn a_client_side_tool_parks_until_the_client_answers() {
    let (fleet, cp) = org_fleet(&[]).await;
    cp.answer_lookups(ORG, DATA_KEY, DATA_KEY_ID);
    let mut definition = everruns_agentd_testkit::definition("files").unwrap();
    definition["tools"] = json!([{
        "type": "client_side",
        "name": "read_page",
        "description": "Read the page beside Chat.",
        "parameters": { "type": "object", "properties": { "binding": { "type": "string" } } },
    }]);
    let session = SessionId::new().to_string();
    let (status, created) = cp
        .call(
            &fleet.base(),
            reqwest::Method::POST,
            &format!("/dp/v1/orgs/{ORG}/sessions"),
            &json!({ "id": session, "agent_ref": "chat", "definition": definition }),
        )
        .await
        .unwrap();
    assert_eq!(status, reqwest::StatusCode::CREATED, "{created}");
    let person = uuid::Uuid::new_v4().to_string();
    say_as(
        &fleet,
        &session,
        MessageId::new(),
        &person,
        r#"call read_page {"binding":"draft"}"#,
    )
    .await;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let call = loop {
        let events = fleet.events(&session).await;
        if let Some(asked) = events.iter().find(|e| e["type"] == "tool.call_requested") {
            break asked["data"]["tool_calls"][0].clone();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the turn never parked on the client's tool: {events:#?}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    assert_eq!(call["name"], json!("read_page"), "{call}");
    assert_eq!(call["arguments"]["binding"], json!("draft"), "{call}");

    let answer = json!({
        "command": uuid::Uuid::new_v4(),
        "tool_results": [{ "tool_call_id": call["id"], "result": { "title": "Billing settings" } }],
    });
    let mut replies = Vec::new();
    let mut taken = false;
    for base in fleet.bases() {
        let reply = fleet
            .http
            .post(format!("{base}/shard/sessions/{session}/tool-results"))
            .json(&answer)
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = reply.json().await.unwrap_or_default();
        if body["reply"] == "ok" {
            taken = true;
            break;
        }
        replies.push(body);
    }
    assert!(taken, "no node took the answer: {replies:?}");
    fleet.turns(&session, 1).await;
    let events = fleet.events(&session).await;
    let replied = events
        .iter()
        .filter(|e| e["type"] == "output.message.completed")
        .any(|e| e.to_string().contains("Billing settings"));
    assert!(replied, "the answer never reached the model: {events:#?}");
}

/// A session's resources and leases stay in the control plane (parity,
/// step 10): a fleet tool lists them there, as the session's org, and the
/// turn finishes with the answer.
#[tokio::test(flavor = "multi_thread")]
async fn a_sessions_resources_are_kept_by_the_control_plane() {
    let (fleet, cp) = org_fleet(&[]).await;
    cp.answer_lookups(ORG, DATA_KEY, DATA_KEY_ID);
    cp.set_lookup(
        "worker_list_session_resources",
        HostCallAnswer::Value(json!([{
            "session_id": SessionId::new().to_string(),
            "resource_id": "sandbox_build",
            "kind": "sandbox",
            "display_name": "Build box",
            "status": "active",
            "metadata": { "role": "resource", "provider": "daytona" },
            "created_at": "2026-11-19T00:00:00Z",
            "updated_at": "2026-11-19T00:00:00Z",
        }])),
    );
    let mut definition = everruns_agentd_testkit::definition("files").unwrap();
    definition["capabilities"] = json!([{ "ref": "sandbox_fleet" }]);
    let session = SessionId::new().to_string();
    let (status, created) = cp
        .call(
            &fleet.base(),
            reqwest::Method::POST,
            &format!("/dp/v1/orgs/{ORG}/sessions"),
            &json!({ "id": session, "agent_ref": "fleet", "definition": definition }),
        )
        .await
        .unwrap();
    assert_eq!(status, reqwest::StatusCode::CREATED, "{created}");
    fleet.say(&session, "call sandbox_list").await;
    fleet.turns(&session, 1).await;

    let asked = cp.lookups();
    let listed = asked
        .iter()
        .find(|(_, name, _)| name == "worker_list_session_resources")
        .unwrap_or_else(|| panic!("no list asked: {asked:?}"));
    assert_eq!(listed.0, ORG);
    assert_eq!(listed.2["session_id"], json!(session));
    let events = fleet.events(&session).await;
    let answered = events
        .iter()
        .filter(|e| e["type"] == "tool.completed")
        .any(|e| e.to_string().contains("Build box"));
    assert!(answered, "the list never reached the turn: {events:#?}");
}

/// Dreaming's state stays in the control plane, as for a v1 session: a
/// turn records itself and leaves its dream owed there, and the executor's
/// sweep runs a dream the control plane reports due (parity, step 10).
#[tokio::test(flavor = "multi_thread")]
async fn a_sessions_dreams_are_kept_and_found_by_the_control_plane() {
    const MEMORY_FILE: &str = everruns_contracts::host_calls::MEMORY_FILE;
    let (fleet, cp) = org_fleet(&[]).await;
    cp.answer_lookups(ORG, DATA_KEY, DATA_KEY_ID);
    cp.set_lookup(MEMORY_FILE, HostCallAnswer::Value(json!({ "ok": true })));
    let mut definition = everruns_agentd_testkit::definition("files").unwrap();
    // The turn's own timer waits an hour: only the sweep can run it here.
    definition["capabilities"] = json!([{ "ref": "dreaming", "config": {
        "destinations": [{ "mount": "/memory/user", "path": "/notes.md" }],
        "trigger": { "mode": "idle", "idle_after_seconds": 3600, "min_new_turns": 1 },
    }}]);
    let session = SessionId::new().to_string();
    let (status, created) = cp
        .call(
            &fleet.base(),
            reqwest::Method::POST,
            &format!("/dp/v1/orgs/{ORG}/sessions"),
            &json!({
                "id": session,
                "agent_ref": "dreamer",
                "definition": definition,
                "memory": ["/memory/user"],
            }),
        )
        .await
        .unwrap();
    assert_eq!(status, reqwest::StatusCode::CREATED, "{created}");
    fleet.say(&session, "I drink green tea every morning").await;
    fleet.turns(&session, 1).await;

    let job = format!("session:{session}");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let owed = loop {
        if let Some(row) = cp.dream_state(ORG, &job)
            && !row["due_at"].is_null()
        {
            break row;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the turn left no dream owed in the control plane: {:?}",
            cp.dream_calls()
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    assert_eq!(owed["session_id"], json!(session));
    assert!(
        cp.dream_state(ORG, &format!("turns:{session}")).is_some(),
        "the turn was not counted in the control plane"
    );

    // Due now: the sweep finds it in the org's due dreams, claims it and
    // runs it against the session's Memory (whose answer here fails it, so
    // it is put off for a retry).
    cp.make_dream_due(ORG, &job);
    let attempts = |row: &serde_json::Value| {
        row["value"]
            .as_str()
            .and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok())
            .and_then(|v| v["job"]["attempts"].as_i64())
            .unwrap_or(0)
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let row = cp.dream_state(ORG, &job).unwrap();
        // The fenced claim increments attempts and sets due_at before the
        // runner reads Memory. Observe that effect, not just its admission.
        let memory_reached = cp
            .lookups()
            .iter()
            .any(|(_, name, params)| name == MEMORY_FILE && params["session_id"] == json!(session));
        if attempts(&row) >= 1 && row["due_at"].as_i64() > Some(0) && memory_reached {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the due dream never ran: {row}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        cp.dream_calls()
            .iter()
            .any(|(org, params)| org == ORG && params["operation"] == "due_org"),
        "the sweep never asked for the org's due dreams"
    );
    let asked = cp.lookups();
    assert!(
        asked
            .iter()
            .any(|(_, name, params)| name == MEMORY_FILE && params["session_id"] == json!(session)),
        "the dream never reached the session's Memory: {asked:?}"
    );
}
