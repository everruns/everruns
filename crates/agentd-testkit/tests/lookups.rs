//! A tool's call-time lookups on the data plane: what crosses to the
//! control plane goes to its injected servers (`credentials`,
//! `platform_commands`, `person_mcp`, `memory_files`) with a session token
//! the executor signs, naming the session's org and the person whose turn
//! it is; org settings are read from the config the control plane pushed.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::sync::Arc;

use everruns_agentd::providers::OrgMaterial;
use everruns_agentd_testkit::fake_cp::Answer;
const PLATFORM_COMMAND_SURFACE: &str = "platform_command_surface";
const MEMORY_FILE: &str = "memory_file";
const USER_MCP_SERVERS: &str = "user_mcp_servers";
const MCP_TOKEN: &str = "worker_get_mcp_connection_token";
use everruns_contracts::session_sandbox::SessionSandboxCredential;
use everruns_contracts::typed_id::{MessageId, SessionId};
use everruns_core::connection_services::UserConnectionResolver;
use serde_json::json;
use support::org::{ORG, org_executor, org_fleet, session};

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

/// A tool's connection lookup goes to the control plane's credentials
/// server as the session's org, acting as no one outside a turn and as the
/// sender of the turn it is bound to; org settings come from the pushed
/// config and never cross; sandbox keys and MCP grants stay here.
#[tokio::test(flavor = "multi_thread")]
async fn a_tools_lookup_reaches_the_control_plane_and_comes_back_for_its_org() {
    let (fleet, cp) = org_fleet(&[]).await;
    cp.set_lookup(
        "worker_get_connection_token",
        Answer::Value(json!("gh-token-1")),
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
    cp.set_lookup(
        "worker_get_connection_token",
        Answer::Error {
            kind: 2,
            message: "not yours".into(),
        },
    );
    let error = resolver
        .get_connection_token(id, "linear")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("not yours"), "{error}");
    assert_eq!(cp.lookups().len(), 2);

    // Org settings are the pushed config's: none pushed, none asked for.
    resolver.get_web_search_settings(id).await.unwrap();
    assert_eq!(cp.lookups().len(), 2);

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

    // Bound to a person's turn, the lookup names them and their message,
    // from the session's own record, so the control plane resolves their
    // connection, not the session owner's. A message nobody is recorded
    // for (the first, sent without a verified sender) names no one.
    let person = uuid::Uuid::new_v4().to_string();
    let theirs = MessageId::new();
    say_as(&fleet, &session, theirs, &person, "ping").await;
    fleet.turns(&session, 2).await;
    cp.set_lookup(
        "worker_get_connection_token",
        Answer::Value(json!("their-token")),
    );
    let token = resolver
        .for_execution(theirs.uuid())
        .expect("bound to the turn")
        .get_connection_token(id, "linear")
        .await
        .unwrap();
    assert_eq!(token.as_deref(), Some("their-token"));
    let asked = cp.lookups();
    let (_, command, params) = asked.last().unwrap();
    assert_eq!(command, "worker_get_connection_token");
    assert_eq!(params["input_message_id"], json!(theirs.uuid().to_string()));
    assert_eq!(params["acting_user"], json!(person));

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

/// The shell's `everruns` command (data plane v2 parity gap 7): an agent
/// with a shell and the `platform` capability runs `everruns ...` in its
/// shell, and the line goes to the control plane's catalog as the
/// `execute` tool's would, acting as the person who sent the turn.
#[tokio::test(flavor = "multi_thread")]
async fn the_shells_everruns_command_runs_the_catalog_as_the_turns_person() {
    let (fleet, cp) = org_fleet(&[]).await;
    cp.set_lookup(PLATFORM_COMMAND_SURFACE, Answer::Value(json!("3 agents")));

    let session = SessionId::new().to_string();
    let mut definition = everruns_agentd_testkit::definition("files").unwrap();
    definition["capabilities"] = json!([{ "ref": "bashkit_shell" }, { "ref": "platform" }]);
    let (status, created) = cp
        .call(
            &fleet.base(),
            reqwest::Method::POST,
            &format!("/dp/v1/orgs/{ORG}/sessions"),
            &json!({ "id": session, "agent_ref": "shell", "definition": definition, "servers": cp.servers() }),
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
    assert_eq!(params["input_message_id"], json!(theirs.uuid().to_string()));
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
    let (fleet, cp) = org_fleet(&[]).await;
    cp.set_lookup(
        MEMORY_FILE,
        Answer::Value(json!({ "ok": {
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
            "servers": cp.servers(),
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
    let (fleet, cp) = org_fleet(&[]).await;
    let authorizations = || stub_mcp::AUTHORIZATIONS.lock().unwrap().clone();
    let create = |session: &str| {
        let mut definition = everruns_agentd_testkit::definition("files").unwrap();
        definition["capabilities"] = json!([{ "ref": "user_mcp" }]);
        json!({ "id": session, "agent_ref": "chat", "definition": definition, "servers": cp.servers() })
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
        Answer::Value(json!({ "ok": {
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
        Answer::Value(json!({ "ok": {
            "notes": {
                "url": stub_mcp::URL,
                "auth_mode": "oauth",
                "oauth_provider_id": "mcp_oauth_notes",
                "actsAs": "user",
            },
        }})),
    );
    cp.set_lookup(MCP_TOKEN, Answer::Value(json!("their-mcp-token")));
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
    let (fleet, cp) = org_fleet(&[]).await;
    let asked = || {
        cp.lookups()
            .into_iter()
            .filter(|(_, c, _)| c == USER_MCP_SERVERS)
            .map(|(_, _, params)| params)
            .collect::<Vec<_>>()
    };
    cp.set_lookup(
        USER_MCP_SERVERS,
        Answer::Value(json!({ "ok": {
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
            &json!({ "id": session, "agent_ref": "files", "definition": definition, "servers": cp.servers() }),
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
    assert_eq!(asked[0]["person"], json!(false), "{:?}", asked[0]);
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

/// Dreaming's state is the session's own, kept in its shard: a turn records
/// itself and leaves its dream owed there, and once it is due it runs
/// against the session's Memory on the control plane's files server.
#[tokio::test(flavor = "multi_thread")]
async fn a_sessions_dream_is_kept_in_its_shard_and_reaches_its_memory() {
    let (fleet, cp) = org_fleet(&[]).await;
    cp.set_lookup(MEMORY_FILE, Answer::Value(json!({ "ok": true })));
    let mut definition = everruns_agentd_testkit::definition("files").unwrap();
    definition["capabilities"] = json!([{ "ref": "dreaming", "config": {
        "destinations": [{ "mount": "/memory/user", "path": "/notes.md" }],
        "trigger": { "mode": "idle", "idle_after_seconds": 1, "min_new_turns": 1 },
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
                "servers": cp.servers(),
            }),
        )
        .await
        .unwrap();
    assert_eq!(status, reqwest::StatusCode::CREATED, "{created}");
    fleet.say(&session, "I drink green tea every morning").await;
    fleet.turns(&session, 1).await;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let asked = cp.lookups();
        if asked.iter().any(|(org, name, params)| {
            org == ORG && name == MEMORY_FILE && params["session_id"] == json!(session)
        }) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the dream never reached the session's Memory: {asked:?}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}
