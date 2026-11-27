use super::*;
use everruns_contracts::ToolRequestProjection;
use everruns_contracts::tool_changes::ToolChanges;

fn input(model: &str) -> (Vec<Message>, LlmCallConfig) {
    let tool =
        |name| ToolDefinition::function(name, name, json!({"type":"object","properties":{}}));
    let mut config = LlmCallConfig::new(model);
    config.tools = vec![tool("current")];
    config.tool_baseline = Some(vec![tool("old")]);
    config.system_prompt_baseline = Some("Base".into());
    let mut last = Message::text(MessageRole::User, "Next");
    last.tool_changes = Some(ToolChanges {
        added: config.tools.clone(),
        removed: vec!["old".into()],
        ..Default::default()
    });
    (
        vec![
            Message::text(MessageRole::System, "Base"),
            Message::text(MessageRole::User, "First"),
            Message::text(MessageRole::Assistant, "Done"),
            last,
        ],
        config,
    )
}

#[test]
fn native_custom_and_hosted_search_projection_matches_the_prepared_wire() {
    for (endpoint, hosted_search, expected) in [
        (
            DEFAULT_BASE_URL,
            false,
            ToolRequestProjection::BaselineWithChanges,
        ),
        (
            "https://company.example/v1/messages",
            false,
            ToolRequestProjection::Current,
        ),
        (DEFAULT_BASE_URL, true, ToolRequestProjection::Current),
    ] {
        let driver = AnthropicChatDriver::new();
        let provider =
            everruns_contracts::Provider::new("arbitrary-framework-provider", driver.clone())
                .base_url(endpoint);
        let (messages, mut config) = input("claude-sonnet-5-5");
        if hosted_search {
            config.tool_search = Some(everruns_contracts::driver_registry::ToolSearchConfig {
                enabled: true,
                threshold: 1,
            });
        }
        assert_eq!(
            driver.tool_request_projection(provider.endpoint(), &messages, &config),
            expected
        );
        let prepared = driver
            .prepare_request(provider.endpoint(), messages, &config)
            .expect("valid request");
        assert_eq!(prepared.tool_projection, expected);
        let body = serde_json::to_value(prepared.request).unwrap();
        let name = if expected == ToolRequestProjection::Current {
            "current"
        } else {
            "old"
        };
        assert!(
            body["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == name),
            "{body}"
        );
        let other = if name == "old" { "current" } else { "old" };
        assert!(
            !body["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == other),
            "{body}"
        );
        assert!(
            config.tool_baseline.is_some(),
            "planning preserves neutral replay metadata"
        );
    }
}

#[test]
fn a_model_without_inline_changes_uses_current_declarations() {
    let driver = AnthropicChatDriver::new();
    let provider =
        everruns_contracts::Provider::new("custom", driver.clone()).base_url(DEFAULT_BASE_URL);
    let (messages, config) = input("application-model-without-profile");
    let prepared = driver
        .prepare_request(provider.endpoint(), messages.clone(), &config)
        .unwrap();
    assert_eq!(
        driver.tool_request_projection(provider.endpoint(), &messages, &config),
        ToolRequestProjection::Current
    );
    let body = serde_json::to_value(prepared.request).unwrap();
    assert_eq!(body["tools"][0]["name"], "current");
}
