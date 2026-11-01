//! Reviewed metadata at conversation API boundaries. Storage and replay retain
//! the original maps; adding a runtime key never implicitly publishes it.

use serde_json::{Map, Value};

fn strings(value: &Value, keys: &[&str]) -> Map<String, Value> {
    keys.iter()
        .filter_map(|key| {
            value
                .get(*key)
                .filter(|v| v.is_string())
                .map(|v| ((*key).into(), v.clone()))
        })
        .collect()
}

fn identity(value: &Value) -> Value {
    Value::Object(strings(
        value,
        &[
            "type",
            "user_id",
            "principal_id",
            "virtual_user_id",
            "schedule_id",
            "trigger_id",
            "channel_id",
        ],
    ))
}

/// Public provenance, without execution authority, routing, or arbitrary source
/// envelopes. This is also used by the SDK's reviewed event envelope.
pub fn event(value: &Value) -> Value {
    let mut public = strings(value, &["participant_id", "dreaming_operation_id"]);
    if let Some(value) = value.get("initiator").filter(|v| v.is_object()) {
        public.insert("initiator".into(), identity(value));
    }
    // One normalized shape whatever marker the row was written with, so every
    // client reads origins the same way (knowledge/execution/message-origin.md).
    if let Some(origin) = value
        .as_object()
        .and_then(crate::message_origin::MessageOrigin::from_metadata)
    {
        public.insert(
            crate::message_origin::MESSAGE_ORIGIN_METADATA_KEY.into(),
            origin.to_value(),
        );
    }
    Value::Object(public)
}

fn page(value: &Value) -> Value {
    let mut page = strings(
        value,
        &["route", "label", "entity_type", "entity_id", "form_name"],
    );
    if let Some(editing) = value.get("editing").filter(|v| v.is_boolean()) {
        page.insert("editing".into(), editing.clone());
    }
    // The browser uses the descriptor to bind page tools to the originating
    // tab. Form values and schemas are not part of this descriptor.
    if let Some(value) = value.get("draft").filter(|v| v.is_object()) {
        let mut draft = strings(value, &["binding", "revision"]);
        for key in ["dirty", "writable"] {
            if let Some(v) = value.get(key).filter(|v| v.is_boolean()) {
                draft.insert(key.into(), v.clone());
            }
        }
        page.insert("draft".into(), Value::Object(draft));
    }
    Value::Object(page)
}

/// Public message annotations. `custom` is the explicit caller-owned namespace;
/// its contents are never populated from runtime state. Unknown legacy keys
/// have mixed provenance and are deliberately not inferred to be public.
// THREAT[TM-API-023]: allowlist names AND shapes, including nested annotations.
pub fn message(value: &Value) -> Value {
    let mut public = strings(
        value,
        &[
            "everruns_client_message_id",
            "source",
            "locale",
            "thread_ts",
            "model",
            "reasoning_effort",
            "provider",
            "response_id",
            "error_code",
        ],
    );
    if let Some(value) = value.get("initiator").filter(|v| v.is_object()) {
        public.insert("initiator".into(), identity(value));
    }
    if let Some(value) = value.get("error_fields").filter(|v| v.is_object()) {
        let mut fields = strings(
            value,
            &[
                "provider",
                "model_id",
                "schema_path",
                "confirm_url",
                "detail",
                "message",
                "budget_id",
                "currency",
            ],
        );
        for key in ["retry_after", "resets_at", "spent", "limit", "soft_limit"] {
            if let Some(v) = value.get(key).filter(|v| v.is_number()) {
                fields.insert(key.into(), v.clone());
            }
        }
        for key in ["auto_retry", "auto_continue"] {
            if let Some(v) = value.get(key).filter(|v| v.is_boolean()) {
                fields.insert(key.into(), v.clone());
            }
        }
        if let Some(types) = value.get("missing_types").and_then(Value::as_array) {
            fields.insert(
                "missing_types".into(),
                Value::Array(types.iter().filter(|v| v.is_string()).cloned().collect()),
            );
        }
        public.insert("error_fields".into(), Value::Object(fields));
    }
    if let Some(value) = value.get("page_context").filter(|v| v.is_object()) {
        public.insert("page_context".into(), page(value));
    }
    if let Some(actions) = value.get("page_actions").and_then(Value::as_array) {
        public.insert(
            "page_actions".into(),
            Value::Array(
                actions
                    .iter()
                    .filter(|v| v.is_object())
                    .map(|action| {
                        let mut public = strings(action, &["id", "label"]);
                        if let Some(value) = action.get("page").filter(|v| v.is_object()) {
                            public.insert("page".into(), page(value));
                        }
                        Value::Object(public)
                    })
                    .collect(),
            ),
        );
    }
    if let Some(custom) = value.get("custom").filter(|v| v.is_object()) {
        public.insert("custom".into(), custom.clone());
    }
    Value::Object(public)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn malformed_fields_cannot_smuggle_nested_runtime_state() {
        let public = message(&json!({
            "model": {"secret": "CANARY"}, "source": ["CANARY"],
            "initiator": {"user_id": {"secret": "CANARY"}},
            "error_fields": {"retry_after": {"secret": "CANARY"}, "detail": ["CANARY"]},
            "page_context": {"draft": {"binding": "tab-1", "revision": "v1", "dirty": true,
                "writable": false, "values": {"secret": "CANARY"}}},
            "page_actions": [{"id": "a1", "label": "Edited name", "secret": "CANARY",
                "page": {"route": "/agents", "label": "Agents", "secret": "CANARY"}}]
        }));
        assert!(!public.to_string().contains("CANARY"));
        assert_eq!(
            public["page_context"]["draft"],
            json!({"binding":"tab-1","revision":"v1","dirty":true,"writable":false})
        );
        assert_eq!(public["page_actions"][0]["label"], "Edited name");
        for value in [Value::Null, json!([]), json!("CANARY"), json!(42)] {
            assert_eq!(message(&value), json!({}));
            assert_eq!(event(&value), json!({}));
        }
    }

    #[test]
    fn custom_is_caller_owned_and_is_not_recursively_scrubbed() {
        let input =
            json!({"custom":{"metadata":{"tool_changes":"customer data"}},"unknown":"CANARY"});
        assert_eq!(message(&input), json!({"custom":input["custom"]}));
    }
}
