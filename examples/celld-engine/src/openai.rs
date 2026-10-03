//! A Chat Completions driver over an injected HTTP transport.
//!
//! The provider crate's OpenAI drivers sit behind its `http` feature
//! (reqwest), which does not build for a JavaScript isolate. This one is a
//! non-streaming call through whatever `fetch` the host has; the engine gets
//! the reply as one stream of events.

use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::driver_registry::{
    ChatDriver, LlmCallConfig, LlmCompletionMetadata, LlmResponseStream, LlmStreamEvent, Message,
    MessageRole,
};
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::runtime_provider::ProviderEndpoint;
use everruns_contracts::tool_types::{ToolCall, ToolDefinition};
use serde_json::{Value, json};

/// POST a JSON body and return the JSON reply.
#[async_trait]
pub trait Transport: Send + Sync {
    async fn post_json(&self, url: &str, bearer: &str, body: Value) -> Result<Value>;
}

pub struct ChatCompletions {
    pub base_url: String,
    pub api_key: String,
    pub transport: Arc<dyn Transport>,
}

#[async_trait]
impl ChatDriver for ChatCompletions {
    async fn chat_completion_stream(
        &self,
        _endpoint: &ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let reply = self
            .transport
            .post_json(&url, &self.api_key, request(&messages, config))
            .await?;
        let events = response(&reply)?;
        Ok(Box::pin(futures::stream::iter(events.into_iter().map(Ok))))
    }
}

pub fn request(messages: &[Message], config: &LlmCallConfig) -> Value {
    let messages: Vec<Value> = messages.iter().map(message).collect();
    let mut body = json!({ "model": config.model, "messages": messages });
    let tools: Vec<Value> = config.tools.iter().map(tool).collect();
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
    }
    body
}

fn message(message: &Message) -> Value {
    let role = match message.role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
    };
    let mut out = json!({ "role": role, "content": message.content.to_text() });
    if let Some(calls) = message
        .tool_calls
        .as_ref()
        .filter(|calls| !calls.is_empty())
    {
        out["tool_calls"] = calls
            .iter()
            .map(|call| {
                json!({
                    "id": call.id,
                    "type": "function",
                    "function": { "name": call.name, "arguments": call.arguments.to_string() },
                })
            })
            .collect();
    }
    if let Some(id) = &message.tool_call_id {
        out["tool_call_id"] = json!(id);
    }
    out
}

fn tool(definition: &ToolDefinition) -> Value {
    let (name, description, parameters) = match definition {
        ToolDefinition::Builtin(tool) => (&tool.name, &tool.description, &tool.parameters),
        ToolDefinition::ClientSide(tool) => (&tool.name, &tool.description, &tool.parameters),
    };
    json!({
        "type": "function",
        "function": { "name": name, "description": description, "parameters": parameters },
    })
}

pub fn response(reply: &Value) -> Result<Vec<LlmStreamEvent>> {
    let message = &reply["choices"][0]["message"];
    if message.is_null() {
        return Err(AgentLoopError::llm(format!(
            "chat completions reply has no message: {reply}"
        )));
    }
    let mut events = Vec::new();
    if let Some(text) = message["content"].as_str().filter(|text| !text.is_empty()) {
        events.push(LlmStreamEvent::TextDelta(text.to_string()));
    }
    let calls: Vec<ToolCall> = message["tool_calls"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|call| ToolCall {
            id: call["id"].as_str().unwrap_or_default().to_string(),
            name: call["function"]["name"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            arguments: call["function"]["arguments"]
                .as_str()
                .and_then(|raw| serde_json::from_str(raw).ok())
                .unwrap_or_else(|| json!({})),
        })
        .collect();
    if !calls.is_empty() {
        events.push(LlmStreamEvent::ToolCalls(calls));
    }
    let mut done = LlmCompletionMetadata::default();
    let usage = &reply["usage"];
    done.prompt_tokens = usage["prompt_tokens"].as_u64().map(|n| n as u32);
    done.completion_tokens = usage["completion_tokens"].as_u64().map(|n| n as u32);
    done.total_tokens = usage["total_tokens"].as_u64().map(|n| n as u32);
    events.push(LlmStreamEvent::Done(Box::new(done)));
    Ok(events)
}
