//! Whole-message transport, normalized through the shared Messages parser.
use super::*;

pub(super) fn message_frames(
    message: Value,
) -> Result<everruns_contracts::stream_reconnect::SseStream> {
    let blocks = message["content"].as_array().ok_or_else(|| {
        AgentLoopError::llm_kind(
            everruns_contracts::llm_error::LlmErrorKind::MalformedResponse,
            "Anthropic JSON response has no content list",
        )
    })?;
    if message["stop_reason"].as_str().is_none() {
        return Err(AgentLoopError::llm_kind(
            everruns_contracts::llm_error::LlmErrorKind::MalformedResponse,
            "Anthropic JSON response has no stop reason",
        ));
    }
    let mut start = message.clone();
    start["content"] = json!([]);
    let mut frames = vec![json!({"type":"message_start","message":start})];
    for (index, block) in blocks.iter().enumerate() {
        let mut opening = block.clone();
        match block["type"].as_str() {
            Some("text") => opening["text"] = json!(""),
            Some("thinking") => {
                opening["thinking"] = json!("");
                if let Some(value) = opening.as_object_mut() {
                    value.remove("signature");
                }
            }
            _ => {}
        }
        frames.push(json!({"type":"content_block_start","index":index,"content_block":opening}));
        let delta = match block["type"].as_str() {
            Some("text") => Some(json!({"type":"text_delta","text":block["text"]})),
            Some("thinking") => Some(json!({"type":"thinking_delta","thinking":block["thinking"]})),
            Some("tool_use" | "server_tool_use") => {
                Some(json!({"type":"input_json_delta","partial_json":block["input"].to_string()}))
            }
            _ => None,
        };
        if let Some(delta) = delta {
            frames.push(json!({"type":"content_block_delta","index":index,"delta":delta}));
        }
        if let Some(signature) = block.get("signature") {
            frames.push(json!({"type":"content_block_delta","index":index,"delta":{"type":"signature_delta","signature":signature}}));
        }
        frames.push(json!({"type":"content_block_stop","index":index}));
    }
    frames.push(json!({"type":"message_delta","delta":{"stop_reason":message["stop_reason"],"stop_sequence":message["stop_sequence"]},"usage":message["usage"]}));
    frames.push(json!({"type":"message_stop"}));
    Ok(Box::pin(futures::stream::iter(frames.into_iter().map(
        |frame| {
            Ok(eventsource_stream::Event {
                event: frame["type"].as_str().unwrap_or_default().to_string(),
                data: frame.to_string(),
                ..Default::default()
            })
        },
    ))))
}
