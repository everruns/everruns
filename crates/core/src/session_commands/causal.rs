//! Telemetry retries do not change the durable command's business identity.
use serde_json::Value;

pub(super) fn normalize_fingerprint(value: &mut Value) {
    let message = if value.get("cancelled") == Some(&Value::Bool(true)) {
        &mut value["message"]
    } else {
        value
    };
    if let Some(object) = message.as_object_mut()
        && let Some(metadata) = object.get_mut("metadata").and_then(Value::as_object_mut)
    {
        metadata.remove(crate::causal::CAUSAL_LINKS_KEY);
        metadata.remove(crate::causal::TURN_TRACEPARENT_KEY);
        if metadata.is_empty() {
            object.remove("metadata");
        }
    }
}
