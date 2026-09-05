//! The published integration executes through the ordinary Framework facade.

use everruns::{Agent, ContentPart, Engine, IntoCapability, LlmSimConfig, Model};
use everruns_core::Capability;
use everruns_integrations_brave_search::{
    BraveSearch, BraveSearchCapability, client::BraveSearchClient,
};
use everruns_llmsim::{SimToolCall, SimTurn};
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path, query_param},
};

#[tokio::test]
async fn framework_reports_http_errors_without_upstream_credential_echoes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(401).set_body_string("rejected sentinel-brave-credential"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let definition = BraveSearch::with_client(BraveSearchClient::with_base_url(
        "sentinel-brave-credential".into(),
        server.uri(),
    ))
    .into_capability()
    .into_parts()
    .definition
    .unwrap();
    let error = definition.tools()[0]
        .invoke(
            json!({"query": "test"}),
            everruns::capability::Context::new("brave_web_search", "session", "workspace"),
        )
        .await
        .unwrap_err();
    assert!(format!("{error:?}").contains("401"));
    assert!(!format!("{error:?}").contains("sentinel-brave-credential"));
}

#[test]
fn adapters_share_tool_protocol_and_keep_credentials_private() {
    let spec = BraveSearch::new("sentinel-brave-credential").into_capability();
    assert!(!format!("{spec:?}").contains("sentinel-brave-credential"));
    assert_eq!(spec.capability_ref().id(), "brave_search");
    let definition = spec.into_parts().definition.unwrap();
    let hosted = BraveSearchCapability;
    let tools = hosted.tools();
    let framework = definition.tools()[0].spec();
    assert_eq!(framework.name(), tools[0].name());
    assert_eq!(framework.description(), tools[0].description());
    let mut schema = framework.input_schema().clone();
    schema.as_object_mut().unwrap().remove("$schema");
    schema.as_object_mut().unwrap().remove("title");
    assert_eq!(schema, tools[0].parameters_schema());
    assert_eq!(
        definition.instructions_text(),
        hosted.system_prompt_addition()
    );
}

#[tokio::test]
async fn framework_turn_searches_with_real_integration_and_records_results() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/web/search"))
        .and(header("X-Subscription-Token", "sentinel-brave-credential"))
        .and(query_param("q", "Rust & agents"))
        .and(query_param("count", "3"))
        .and(query_param("offset", "2"))
        .and(query_param("freshness", "pw"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"web": {"results": [
            {"title": "Primary source", "url": "https://example.com/source", "description": "Evidence", "age": "1 day ago"}
        ]}})))
        .expect(1)
        .mount(&server)
        .await;
    let agent = Agent::builder()
        .instructions("Search before answering.")
        .model(Model::simulated_with_config(LlmSimConfig::scripted(vec![
            SimTurn::ToolCalls(vec![SimToolCall {
                name: "brave_web_search".into(),
                arguments: json!({"query": "Rust & agents", "count": 3, "offset": 2, "freshness": "pw"}),
                id: Some("search_1".into()),
            }]),
            SimTurn::Assistant("Search complete.".into()),
        ])))
        .capability(BraveSearch::with_client(BraveSearchClient::with_base_url(
            "sentinel-brave-credential".into(),
            server.uri(),
        )))
        .build()
        .unwrap();
    assert!(!format!("{agent:?}").contains("sentinel-brave-credential"));
    let session = Engine::new().create(agent);
    let turn = session
        .send_and_wait("Research Rust agents.")
        .await
        .unwrap();
    assert!(turn.success);
    assert_eq!(turn.tool_calls, 1);
    let history = session.history().page().await.unwrap();
    let result = history
        .messages
        .iter()
        .flat_map(|message| &message.content)
        .find_map(|part| match part {
            ContentPart::ToolResult(result) if result.tool_call_id == "search_1" => Some(result),
            _ => None,
        })
        .expect("recorded search result");
    assert!(result.error.is_none());
    assert_eq!(
        result.result,
        Some(json!({
            "query": "Rust & agents", "count": 1,
            "results": [{"title": "Primary source", "url": "https://example.com/source", "description": "Evidence", "age": "1 day ago"}],
        }))
    );
    let transcript = serde_json::to_string(
        &history
            .messages
            .iter()
            .map(|message| &message.content)
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert!(!transcript.contains("sentinel-brave-credential"));
}
