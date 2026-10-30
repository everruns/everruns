//! Normalize a whole Responses resource into the same parser inputs as SSE.
//!
//! Request construction, tool settlement, reasoning replay, citations, usage and
//! error handling stay shared. These are in-process frames: no SSE is received
//! from or sent to the provider on this path.
use super::*;
use crate::stream_reconnect::{SseStream, connect_sse_with_reconnect};

impl OpenResponsesProtocolChatDriver {
    pub(super) async fn connect_response(
        &self,
        endpoint: &crate::ProviderEndpoint,
        api_url: &str,
        body: &Value,
        headers: &HeaderMap,
        config: &LlmCallConfig,
    ) -> Result<(SseStream, RetryMetadata)> {
        if body.get("stream") != Some(&Value::Bool(false)) {
            return connect_sse_with_reconnect(
                &self.retry_config,
                "OpenResponsesProtocolDriver",
                |attempts| {
                    self.send_responses_request(endpoint, api_url, body, headers, config, attempts)
                },
            )
            .await;
        }
        let (response, retries) = self
            .send_responses_request(endpoint, api_url, body, headers, config, 0)
            .await?;
        // A truncated JSON body is a failed call, never an empty successful
        // completion. Do not replay a POST after it may have run hosted tools.
        let resource: Value = response.json().await.map_err(|error| {
            AgentLoopError::llm_kind(
                crate::llm_error::LlmErrorKind::MalformedResponse,
                format!("Invalid Responses JSON response: {error}"),
            )
        })?;
        let frames = resource_frames(resource)?;
        Ok((
            Box::pin(futures::stream::iter(frames.into_iter().map(|frame| {
                Ok(eventsource_stream::Event {
                    event: frame["type"].as_str().unwrap_or_default().to_string(),
                    data: frame.to_string(),
                    ..Default::default()
                })
            }))),
            retries,
        ))
    }
}

fn resource_frames(resource: Value) -> Result<Vec<Value>> {
    let terminal = match resource["status"].as_str() {
        Some("completed") => "response.completed",
        Some("incomplete") => "response.incomplete",
        Some("failed") => "response.failed",
        _ => {
            return Err(AgentLoopError::llm_kind(
                crate::llm_error::LlmErrorKind::MalformedResponse,
                "Responses JSON response has no terminal status",
            ));
        }
    };
    if terminal == "response.failed" {
        return Ok(vec![json!({"type":"error","error":resource["error"]})]);
    }
    let output = resource["output"].as_array().ok_or_else(|| {
        AgentLoopError::llm_kind(
            crate::llm_error::LlmErrorKind::MalformedResponse,
            "Responses JSON response has no output list",
        )
    })?;
    let mut frames = Vec::new();
    for (index, item) in output.iter().enumerate() {
        if matches!(
            item["type"].as_str(),
            Some("function_call" | "custom_tool_call")
        ) && item["status"] != "completed"
        {
            // The terminal resource counts truncated calls as dropped. A
            // synthetic done frame would incorrectly make them executable.
            continue;
        }
        if item["type"] == "message" {
            frames.push(json!({"type":"response.output_item.added","output_index":index,"item":{
                "type":"message", "id":item["id"], "role":"assistant", "content":[], "status":item["status"], "phase":item["phase"]
            }}));
            for part in item["content"].as_array().into_iter().flatten() {
                if let Some(text) = part["text"].as_str() {
                    frames.push(json!({"type":"response.output_text.delta","delta":text}));
                } else if let Some(refusal) = part["refusal"].as_str() {
                    frames.push(json!({"type":"response.output_text.delta","delta":refusal}));
                }
            }
        }
        if item["type"] != "message" {
            frames
                .push(json!({"type":"response.output_item.done","output_index":index,"item":item}));
        }
    }
    frames.push(json!({"type":terminal,"response":resource}));
    for (sequence, frame) in frames.iter_mut().enumerate() {
        frame["sequence_number"] = json!(sequence);
    }
    Ok(frames)
}
