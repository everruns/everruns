//! Tool changes sent inside the conversation, so `tools` stays the epoch's
//! baseline and the cached prefix survives a reveal.
//!
//! Each recorded change becomes developer items placed after the input it took
//! effect at, before the next model output: an `additional_tools` item with the
//! full definitions of added and redefined tools (a same-name definition
//! replaces the declared one, which is how a deferred stub becomes its schema),
//! and a developer message for removals and notes. Removal is soft: the tool
//! stays declared and a call to it is refused by the runtime.
//!
//! This runs only against `api.openai.com`, on models whose profile says a live
//! round trip kept the cache after a redefinition. Anything else, including a
//! change that touches a tool a native tool stands in for, falls back to
//! sending the tools wanted now, which is the behavior before recorded changes
//! existed.

// Design of record: knowledge/execution/cache-stable-tool-changes.md.
use serde_json::json;

use super::{OpenResponsesProtocolChatDriver, ResponsesContent, ResponsesInputItem};
use crate::driver_registry::{LlmCallConfig, Message};
use crate::model::INLINE_TOOL_CHANGES_PARAMETER;
use crate::tool_changes::ToolChanges;
use crate::tool_types::{DeferrablePolicy, ToolDefinition};

/// Which endpoints may receive inline tool changes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum InlineToolChangeHosts {
    #[default]
    Off,
    /// `api.openai.com` only: `additional_tools` is an OpenAI extension that
    /// compatible endpoints and proxies may reject or drop.
    OpenAi,
    /// Any endpoint; lets request tests run against a mock server.
    #[cfg(test)]
    Any,
}

impl InlineToolChangeHosts {
    fn allows(self, base_url: Option<&str>) -> bool {
        match self {
            Self::Off => false,
            Self::OpenAi => base_url.is_some_and(|url| url.starts_with("https://api.openai.com/")),
            #[cfg(test)]
            Self::Any => true,
        }
    }
}

/// How the request renders recorded changes, when it can.
#[derive(Debug, Clone, Copy)]
pub(crate) struct InlineToolChanges {
    /// Hosted `tool_search` is on: added tools arrive deferred, like the
    /// deferrable tools of the baseline.
    pub(crate) hosted_search: bool,
}

/// Whether this request sends the baseline as `tools` and the changes inline.
/// `replaced` names the function tools a native tool stands in for; a change
/// that touches one of them cannot be expressed and falls back.
pub(crate) fn eligible(
    hosts: InlineToolChangeHosts,
    base_url: Option<&str>,
    config: &LlmCallConfig,
    messages: &[Message],
    replaced: &[&str],
) -> bool {
    hosts.allows(base_url)
        && config.tool_baseline.as_ref().is_some_and(|b| !b.is_empty())
        && crate::get_model_profile(&crate::DriverId::OpenAI, &config.model)
            .is_some_and(|profile| profile.supports_parameter(INLINE_TOOL_CHANGES_PARAMETER))
        && messages
            .iter()
            .filter_map(|message| message.tool_changes.as_ref())
            .all(|changes| {
                changes
                    .added
                    .iter()
                    .chain(&changes.redefined)
                    .map(ToolDefinition::name)
                    .chain(changes.removed.iter().map(String::as_str))
                    .all(|name| !replaced.contains(&name))
            })
}

/// The developer items for one recorded change: a message for the note and
/// removals, then one `additional_tools` item for added and redefined tools.
pub(crate) fn items(changes: &ToolChanges, mode: InlineToolChanges) -> Vec<ResponsesInputItem> {
    let mut lines: Vec<String> = changes.note.iter().cloned().collect();
    if !changes.removed.is_empty() {
        lines.push(format!(
            "These tools are no longer available; do not call them: {}.",
            changes.removed.join(", ")
        ));
    }
    let note = (!lines.is_empty()).then(|| ResponsesInputItem::Message {
        r#type: "message".to_string(),
        role: "developer".to_string(),
        content: ResponsesContent::Text(lines.join("\n\n")),
        phase: None,
    });
    let tools: Vec<_> = changes
        .added
        .iter()
        .chain(&changes.redefined)
        .map(|tool| {
            let defer = mode.hosted_search && *tool.deferrable() != DeferrablePolicy::Never;
            OpenResponsesProtocolChatDriver::function_tool(tool, defer.then_some(true))
        })
        .collect();
    let additions = (!tools.is_empty()).then(|| {
        ResponsesInputItem::ProviderItem(json!({
            "type": "additional_tools",
            "role": "developer",
            "tools": tools,
        }))
    });
    note.into_iter().chain(additions).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver_registry::MessageRole;
    use serde_json::Value;

    const MODEL: &str = "gpt-6.1-sol";

    fn tool(name: &str) -> ToolDefinition {
        ToolDefinition::function(
            name,
            "Deploy status",
            json!({"type": "object", "properties": {"env": {"type": "string"}}}),
        )
    }

    fn config(baseline: Vec<ToolDefinition>) -> LlmCallConfig {
        let mut config = LlmCallConfig::new(MODEL);
        config.tool_baseline = Some(baseline);
        config
    }

    fn changed(changes: ToolChanges) -> Vec<Message> {
        let mut message = Message::text(MessageRole::User, "go");
        message.tool_changes = Some(changes);
        vec![message]
    }

    fn addition(name: &str) -> ToolChanges {
        ToolChanges {
            added: vec![tool(name)],
            ..ToolChanges::default()
        }
    }

    #[test]
    fn only_verified_models_on_openai_with_a_baseline_go_inline() {
        let openai = Some("https://api.openai.com/v1");
        let hosts = InlineToolChangeHosts::OpenAi;
        let messages = changed(addition("deploy"));
        assert!(eligible(
            hosts,
            openai,
            &config(vec![tool("a")]),
            &messages,
            &[]
        ));
        assert!(
            !eligible(
                hosts,
                Some("https://proxy.example/v1"),
                &config(vec![tool("a")]),
                &messages,
                &[]
            ),
            "a compatible endpoint"
        );
        assert!(
            !eligible(
                InlineToolChangeHosts::Off,
                openai,
                &config(vec![tool("a")]),
                &messages,
                &[]
            ),
            "a driver without the option"
        );
        let mut unverified = config(vec![tool("a")]);
        unverified.model = "gpt-4.1".to_string();
        assert!(
            !eligible(hosts, openai, &unverified, &messages, &[]),
            "unverified"
        );
        assert!(
            !eligible(hosts, openai, &config(Vec::new()), &messages, &[]),
            "empty baseline"
        );
        assert!(
            !eligible(
                hosts,
                openai,
                &config(vec![tool("a")]),
                &messages,
                &["deploy"]
            ),
            "a change to a tool a native tool replaces"
        );
    }

    #[test]
    fn a_change_renders_a_note_and_one_additional_tools_item() {
        let changes = ToolChanges {
            added: vec![tool("deploy")],
            redefined: vec![tool("search")],
            removed: vec!["old".to_string()],
            note: Some("Deploys need approval.".to_string()),
            ..ToolChanges::default()
        };
        let items: Vec<Value> = items(
            &changes,
            InlineToolChanges {
                hosted_search: false,
            },
        )
        .iter()
        .map(|item| serde_json::to_value(item).unwrap())
        .collect();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["role"], "developer");
        assert_eq!(
            items[0]["content"],
            "Deploys need approval.\n\nThese tools are no longer available; do not call them: old."
        );
        assert_eq!(items[1]["type"], "additional_tools");
        assert_eq!(items[1]["role"], "developer");
        let names: Vec<_> = items[1]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["deploy", "search"]);
        assert!(items[1]["tools"][0].get("defer_loading").is_none());
    }

    #[test]
    fn a_change_sits_after_its_input_and_before_the_next_output() {
        let mut call = Message::text(MessageRole::Assistant, "");
        call.tool_calls = Some(vec![crate::tool_types::ToolCall {
            id: "call_1".to_string(),
            name: "tool_search".to_string(),
            arguments: json!({"query": "deploy"}),
        }]);
        let mut result = Message::text(MessageRole::Tool, "found deploy");
        result.tool_call_id = Some("call_1".to_string());
        result.tool_changes = Some(ToolChanges {
            redefined: vec![tool("deploy")],
            ..ToolChanges::default()
        });
        let messages = vec![
            Message::text(MessageRole::System, "prompt"),
            Message::text(MessageRole::User, "deploy prod"),
            call,
            result,
        ];
        let mode = Some(InlineToolChanges {
            hosted_search: false,
        });
        let types = |messages: &[Message]| -> Vec<String> {
            let (_, items) =
                OpenResponsesProtocolChatDriver::build_input_with_changes(messages, false, mode);
            items
                .iter()
                .map(|item| {
                    let item = serde_json::to_value(item).unwrap();
                    item["type"].as_str().unwrap_or("message").to_string()
                })
                .collect()
        };
        assert_eq!(
            types(&messages),
            [
                "message",
                "function_call",
                "function_call_output",
                "additional_tools"
            ]
        );
        let mut answered = messages.clone();
        answered.push(Message::text(MessageRole::Assistant, "deployed"));
        assert_eq!(
            types(&answered),
            [
                "message",
                "function_call",
                "function_call_output",
                "additional_tools",
                "message"
            ]
        );
        let (_, plain) = OpenResponsesProtocolChatDriver::build_input(&answered, false);
        assert_eq!(
            plain.len(),
            4,
            "without inline mode the change is not rendered"
        );
    }

    #[tokio::test]
    async fn the_request_keeps_the_baseline_and_carries_the_change() {
        use crate::driver_registry::ChatDriver;
        use futures::StreamExt;
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(
                        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r\",\"status\":\"completed\",\"output\":[]}}\n\n",
                    ),
            )
            .mount(&server)
            .await;
        let provider = crate::runtime_provider::RuntimeProvider::new(
            "inline-test",
            OpenResponsesProtocolChatDriver::new(),
        )
        .base_url(server.uri());
        let driver = OpenResponsesProtocolChatDriver::new()
            .with_inline_tool_changes_on_any_host()
            .with_retry_config(crate::llm_retry::LlmRetryConfig::no_retry());
        let mut config = config(vec![tool("a")]);
        config.tools = vec![tool("a"), tool("deploy")];
        let mut stream = driver
            .chat_completion_stream(provider.endpoint(), changed(addition("deploy")), &config)
            .await
            .unwrap();
        while stream.next().await.is_some() {}

        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        let names: Vec<_> = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["a"], "tools stays the baseline");
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.last().unwrap()["type"], "additional_tools");
        assert_eq!(input.last().unwrap()["tools"][0]["name"], "deploy");
    }

    #[test]
    fn with_hosted_search_added_tools_arrive_deferred() {
        let pinned = ToolDefinition::ClientSide(
            crate::tool_types::ClientSideTool::new("pinned", "Pinned", json!({"type": "object"}))
                .with_deferrable(DeferrablePolicy::Never),
        );
        let changes = ToolChanges {
            added: vec![tool("deploy"), pinned],
            ..ToolChanges::default()
        };
        let items = items(
            &changes,
            InlineToolChanges {
                hosted_search: true,
            },
        );
        let item = serde_json::to_value(&items[0]).unwrap();
        assert_eq!(item["tools"][0]["defer_loading"], true);
        assert!(item["tools"][1].get("defer_loading").is_none());
    }
}
