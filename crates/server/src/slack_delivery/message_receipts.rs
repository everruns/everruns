//! Observe explicit delivery without publishing a second copy.
use everruns_core::channel_messaging::CHANNEL_POST_MESSAGE_TOOL_NAME;

pub(crate) fn channel_message_was_delivered(data: &serde_json::Value) -> bool {
    if data.get("tool_name").and_then(|v| v.as_str()) != Some(CHANNEL_POST_MESSAGE_TOOL_NAME) {
        return false;
    }
    if !data
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return false;
    }

    let Some(result) = data.get("result").and_then(|value| value.as_array()) else {
        return false;
    };
    let json_text = result.iter().find_map(|part| {
        if part.get("type")?.as_str()? == "text" {
            part.get("text").and_then(|text| text.as_str())
        } else {
            None
        }
    });
    let Some(payload) =
        json_text.and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
    else {
        return false;
    };
    payload["delivered"] == true
        && payload["platform"] == "slack"
        && payload["message_ref"]
            .as_str()
            .is_some_and(|reference| !reference.is_empty())
}
