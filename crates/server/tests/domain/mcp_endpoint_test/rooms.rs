//! Room tools on the MCP endpoint: a client catches up, posts, sees who read
//! it, and keeps a shared file, through `tools/call` like any MCP client.

use super::*;

async fn room_call(server: &TestServer, tool: &str, arguments: Value) -> Value {
    let response = mcp_tool_call(server, tool, arguments).await;
    assert!(!tool_is_error(&response), "{tool}: {response}");
    tool_json(&response)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn room_tools_let_an_mcp_client_take_part_in_a_room() {
    let server = TestServer::in_memory().await;
    let room = room_call(
        &server,
        "execute",
        json!({
            "commands": r#"create_room --name Launch --members '[{"kind":"guest","display_name":"Ana"},{"kind":"outside_agent","display_name":"Planner"}]'"#
        }),
    )
    .await;
    let room_id = room["id"].as_str().unwrap();
    let members = room_call(
        &server,
        "execute",
        json!({
            "commands": format!("list_room_members --room_id {room_id}")
        }),
    )
    .await;
    let member = |handle: &str| {
        members["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["handle"] == handle)
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let (ana, planner) = (member("ana"), member("planner"));

    // The outside agent posts; Ana catches up and the message is marked read.
    let posted = room_call(
        &server,
        "room_post",
        json!({
            "room_id": room_id, "text": "@ana the launch plan is in /plan.md", "as_member": planner
        }),
    )
    .await;
    let message_id = posted["message"]["id"].as_str().unwrap();
    let unread = room_call(
        &server,
        "room_read_unread",
        json!({
            "room_id": room_id, "as_member": ana
        }),
    )
    .await;
    assert_eq!(unread["data"][0]["id"], message_id);
    let again = room_call(
        &server,
        "room_read_unread",
        json!({
            "room_id": room_id, "as_member": ana
        }),
    )
    .await;
    assert!(again["data"].as_array().unwrap().is_empty());
    let readers = room_call(
        &server,
        "room_who_read",
        json!({
            "room_id": room_id, "message_id": message_id
        }),
    )
    .await;
    assert!(readers.to_string().contains("ana"), "{readers}");

    // A shared file: written, edited in part, read back, with its history.
    let written = room_call(
        &server,
        "room_write_file",
        json!({
            "room_id": room_id, "path": "plan.md", "content": "- [ ] Draft\n- [ ] Ship\n",
            "as_member": planner
        }),
    )
    .await;
    assert_eq!(written["version"], 1);
    let edited = room_call(
        &server,
        "room_edit_file",
        json!({
            "room_id": room_id, "path": "/plan.md",
            "edits": [{ "old_text": "- [ ] Draft", "new_text": "- [x] Draft" }],
            "as_member": ana
        }),
    )
    .await;
    assert_eq!(edited["version"], 2);
    assert_eq!(edited["author"]["handle"], "ana");
    let file = room_call(
        &server,
        "room_read_file",
        json!({ "room_id": room_id, "path": "/plan.md" }),
    )
    .await;
    assert_eq!(file["content"], "- [x] Draft\n- [ ] Ship\n");
    let first = room_call(
        &server,
        "room_read_file",
        json!({
            "room_id": room_id, "path": "/plan.md", "version": 1
        }),
    )
    .await;
    assert_eq!(first["content"], "- [ ] Draft\n- [ ] Ship\n");
    let history = room_call(
        &server,
        "room_file_history",
        json!({ "room_id": room_id, "path": "/plan.md" }),
    )
    .await;
    let authors: Vec<&str> = history["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["author"]["handle"].as_str().unwrap())
        .collect();
    assert_eq!(authors, ["ana", "planner"]);

    // A write against a version that is no longer current changes nothing.
    let stale = mcp_tool_call(
        &server,
        "room_write_file",
        json!({
            "room_id": room_id, "path": "/plan.md", "content": "lost", "if_version": 1,
            "as_member": planner
        }),
    )
    .await;
    assert!(tool_is_error(&stale), "{stale}");
    let files = room_call(&server, "room_list_files", json!({ "room_id": room_id })).await;
    assert_eq!(files["data"][0]["version"], 2);

    // Reading the thread and listing rooms are read-only tools.
    let listed = room_call(&server, "room_list", json!({})).await;
    assert!(listed.to_string().contains(room_id));
    let read = room_call(&server, "room_read", json!({ "room_id": room_id })).await;
    assert_eq!(read["data"][0]["id"], message_id);

    // The same files over HTTP.
    let listed_http: Value = server
        .get(&format!("/v1/rooms/{room_id}/files"))
        .await
        .assert_success()
        .json();
    assert_eq!(listed_http["data"][0]["path"], "/plan.md");
}
