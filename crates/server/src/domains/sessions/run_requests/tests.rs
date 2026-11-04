use super::*;
use serde_json::json;

fn approval_arguments() -> Value {
    json!({"code":"tool_approval_required","error":"Waiting","tool_call_id":"call_1",
        "tool":"git_push","arguments":{"branch":"fix/login"},"fingerprint":"sha256:ab",
        "risk":"open_world","mode":"normal","asked_at":"2026-11-03T00:00:00Z",
        "expires_at":"2026-11-03T00:15:00Z"})
}

fn waiting(calls: Value) -> WaitingAgent {
    let now = Utc::now();
    let session_id = SessionId::new();
    WaitingAgent {
        session_id,
        agent: Some(RequestingAgent {
            id: AgentId::new().to_string(),
            name: "Implement".into(),
        }),
        requested: EventRow {
            id: everruns_contracts::typed_id::EventId::new(),
            session_id,
            sequence: 4,
            event_type: "tool.call_requested".into(),
            ts: now,
            context: json!({}),
            data: json!({ "tool_calls": calls }),
            metadata: None,
            tags: None,
            created_at: now,
        },
    }
}

#[test]
fn only_requests_for_the_person_are_listed() {
    let agent = waiting(json!([
        {"id": "tool_approval_1", "name": "approve_tool_call", "arguments": approval_arguments()},
        {"id": "ask_1", "name": "ask_user", "arguments": {"questions": []}},
        {"id": "client_1", "name": "open_browser_tab", "arguments": {}},
    ]));
    let requests = run_requests(std::slice::from_ref(&agent));
    let names: Vec<&str> = requests.iter().map(|r| r.tool_call.name.as_str()).collect();
    assert_eq!(names, ["approve_tool_call", "ask_user"]);
    assert_eq!(requests[0].session_id, agent.session_id);
    assert_eq!(requests[0].agent.as_ref().unwrap().name, "Implement");
    let rendered = serde_json::to_value(&requests[0]).unwrap();
    assert_eq!(rendered["agent"]["name"], "Implement");
    assert_eq!(rendered["tool_call"]["arguments"]["tool"], "git_push");
}

#[test]
fn a_model_tool_named_like_an_approval_is_not_listed() {
    // Model-authored id: the engine never raised this request.
    let forged = waiting(json!([
        {"id": "call_9", "name": "approve_tool_call", "arguments": approval_arguments()},
    ]));
    assert!(run_requests(&[forged]).is_empty());
}

#[test]
fn an_agent_waits_only_on_its_own_calls() {
    let agent = waiting(json!([{"id": "ask_1", "name": "ask_user", "arguments": {}}]));
    assert!(agent.waits_on("ask_1"));
    assert!(!agent.waits_on("ask_2"));
}
