//! End-to-end driver tests for the OpenAI Agents API prototype (EVE-1120).
//!
//! The mock test runs everywhere. The live test talks to OpenAI only when
//! `EVERRUNS_OPENAI_AGENTS_API_LIVE=1` and `OPENAI_API_KEY` are set, because
//! the Agents API is a paid beta and CI must not depend on its quota.
#![cfg(feature = "openai-agents-api-prototype")]

use std::sync::{Arc, Mutex};

use everruns_core::{RuntimeAgent, ScopedMcpServer, ScopedMcpServers};
use everruns_host::openai_agents_api::{
    AgentsApiClient, AgentsApiEventMapper, AgentsApiPrototypeError, FunctionCallAction,
    RootTurnOutcome, build_session_config, run_root_turn,
};
use everruns_provider::tool_types::{ClientSideTool, ToolDefinition};
use everruns_provider::typed_id::{MessageId, SessionId, TurnId};
use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn agent() -> RuntimeAgent {
    let mut agent = RuntimeAgent::new(
        "Look the customer up, then search the docs, then answer in one sentence.",
        "gpt-6-astra",
    );
    agent
        .tools
        .push(ToolDefinition::ClientSide(ClientSideTool::new(
            "lookup_customer",
            "Look up one customer",
            json!({
                "type": "object",
                "properties": {"customer_id": {"type": "string"}},
                "required": ["customer_id"],
                "additionalProperties": false
            }),
        )));
    agent
}

fn docs_mcp() -> ScopedMcpServers {
    ScopedMcpServers::from([(
        "docs".to_string(),
        ScopedMcpServer {
            url: "https://developers.openai.com/mcp".to_string(),
            ..ScopedMcpServer::default()
        },
    )])
}

fn mapper() -> AgentsApiEventMapper {
    AgentsApiEventMapper::new(
        SessionId::from_seed(1),
        TurnId::from_seed(2),
        MessageId::from_seed(3),
        "gpt-6-astra",
    )
}

fn sse(events: &[Value]) -> String {
    events
        .iter()
        .map(|event| {
            format!(
                "event: {}\ndata: {event}\n\n",
                event["type"].as_str().unwrap()
            )
        })
        .collect()
}

#[tokio::test]
async fn one_function_and_one_mcp_tool_run_end_to_end() {
    let server = MockServer::start().await;
    let fixture: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/agents_api_events.json")).unwrap();
    Mock::given(method("POST"))
        .and(path("/agents/sessions"))
        .and(header("OpenAI-Beta", "agents=v1"))
        .and(header("authorization", "Bearer test-key"))
        .and(body_partial_json(json!({
            "stream": true,
            "agent": {"model": "gpt-6-astra", "tools": [
                {"type": "function", "name": "lookup_customer"},
                {"type": "mcp", "server_label": "docs"}
            ]}
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse(&fixture)),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/agents/sessions/sess_fixture/events"))
        .and(body_partial_json(json!({"events": [{
            "type": "agent.session.input.tool_result",
            "turn_id": "turn_fixture",
            "call_id": "call_customer",
            "success": true,
            "output": "{\"name\":\"Ada\"}"
        }]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;

    let client = AgentsApiClient::new("test-key").with_base_url(server.uri());
    let config = build_session_config(&agent(), &docs_mcp(), "Who is customer 123?", None).unwrap();
    let handled = Arc::new(Mutex::new(Vec::<FunctionCallAction>::new()));
    let mut events = Vec::new();
    let mut mapper = mapper();
    let outcome = run_root_turn(
        &client,
        &config,
        &mut mapper,
        |action| {
            let handled = handled.clone();
            async move {
                handled.lock().unwrap().push(action);
                Ok(r#"{"name":"Ada"}"#.to_string())
            }
        },
        |event| events.push(event),
    )
    .await
    .unwrap();

    assert_eq!(outcome, RootTurnOutcome::Completed);
    assert_eq!(
        handled.lock().unwrap()[0].arguments,
        json!({"customer_id": "123"})
    );
    let types = events
        .iter()
        .map(|e| e.event_type.as_str())
        .collect::<Vec<_>>();
    for expected in [
        "turn.started",
        "tool.call_requested",
        "tool.started",
        "tool.completed",
        "output.message.completed",
        "turn.completed",
        "session.idled",
    ] {
        assert!(types.contains(&expected), "missing {expected} in {types:?}");
    }
}

#[tokio::test]
async fn http_errors_surface_status_and_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/agents/sessions"))
        .respond_with(
            ResponseTemplate::new(403).set_body_string(r#"{"error":{"code":"beta_access"}}"#),
        )
        .mount(&server)
        .await;
    let client = AgentsApiClient::new("test-key").with_base_url(server.uri());
    let config = build_session_config(&agent(), &ScopedMcpServers::default(), "hi", None).unwrap();
    let result = run_root_turn(
        &client,
        &config,
        &mut mapper(),
        |_| async { Ok(String::new()) },
        |_| {},
    )
    .await;
    assert!(matches!(
        result,
        Err(AgentsApiPrototypeError::Api { status: 403, ref body }) if body.contains("beta_access")
    ));
}

#[tokio::test]
async fn stream_that_closes_mid_turn_is_not_success() {
    let server = MockServer::start().await;
    let partial = [json!({
        "type": "agent.session.turn.created", "session_id": "sess_1", "turn_id": "turn_1",
        "turn": {"id": "turn_1", "subagent_id": null}
    })];
    Mock::given(method("POST"))
        .and(path("/agents/sessions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse(&partial)),
        )
        .mount(&server)
        .await;
    let client = AgentsApiClient::new("test-key").with_base_url(server.uri());
    let config = build_session_config(&agent(), &ScopedMcpServers::default(), "hi", None).unwrap();
    let result = run_root_turn(
        &client,
        &config,
        &mut mapper(),
        |_| async { Ok(String::new()) },
        |_| {},
    )
    .await;
    assert!(matches!(
        result,
        Err(AgentsApiPrototypeError::StreamClosedBeforeTurnEnded)
    ));
}

/// Opt-in live run against api.openai.com with one function and one MCP tool.
#[tokio::test]
async fn live_agents_api_round_trip() {
    if std::env::var("EVERRUNS_OPENAI_AGENTS_API_LIVE").as_deref() != Ok("1") {
        eprintln!("SKIP: set EVERRUNS_OPENAI_AGENTS_API_LIVE=1 to call the live Agents API");
        return;
    }
    let api_key =
        std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY is required for the live run");
    let client = AgentsApiClient::new(api_key);
    let config = build_session_config(
        &agent(),
        &docs_mcp(),
        "Look up customer 123, then search the OpenAI docs for 'Agents API sessions'.",
        None,
    )
    .unwrap();
    let mut events = Vec::new();
    let mut mapper = mapper();
    let outcome = run_root_turn(
        &client,
        &config,
        &mut mapper,
        |action| async move {
            eprintln!("function call: {} {}", action.name, action.arguments);
            Ok(r#"{"customer_id":"123","name":"Ada Lovelace"}"#.to_string())
        },
        |event| events.push(event),
    )
    .await;
    for event in &events {
        eprintln!(
            "{} {}",
            event.event_type,
            serde_json::to_string(&event.data).unwrap()
        );
    }
    assert_eq!(outcome.unwrap(), RootTurnOutcome::Completed);
}
