//! The CLI's command transport through the edge (`edge.rs`).
//!
//! `everruns-cli` sends every call as `POST /v1/commands/{name}` with an
//! envelope (`{ "params": ..., "api_version": ... }`) to the control plane,
//! which runs the command in process. A v2 session's running calls must
//! reach its data plane instead, where the control plane refuses them.
//!
//! Decisions:
//! - The edge translates the stage 1 session commands (send and list
//!   messages, list events, cancel, tool results) into the `/v1` call they
//!   name and answers in the command envelope, so the CLI needs no change
//!   and the control plane stays off the message path.
//! - Every other command, and these for a v1 session, go to the control
//!   plane unchanged, body and all.
//! - The answer's `schema_hash` echoes the request's: the edge does not
//!   hold the command catalog, and a client only warns on a mismatch.
//! - A message send stays idempotent: its id is derived from the
//!   command's `client_message_id`, else its `Idempotency-Key`, so a
//!   retried send is admitted once.

use axum::http::Method;
use everruns_contracts::typed_id::{MessageId, SessionId};
use serde_json::{Map, Value, json};

/// The largest command body the edge reads to route it.
pub(crate) const MAX_BODY: usize = 4 << 20;

/// A session command as the `/v1` call it names.
#[derive(Debug, PartialEq)]
pub(crate) struct Call {
    pub session: SessionId,
    pub method: Method,
    /// `/v1/sessions/{id}/...`, with the query for a read.
    pub path_and_query: String,
    pub body: Option<Value>,
}

/// The stage 1 session commands: wire name, method, path under the session.
const COMMANDS: &[(&str, &str, &str)] = &[
    ("create_message", "POST", "messages"),
    ("list_messages", "GET", "messages"),
    ("list_events", "GET", "events"),
    ("cancel_session", "POST", "cancel"),
    ("submit_tool_results", "POST", "tool-results"),
];

/// Whether `/v1/commands/{name}` may be a v2 session's call.
pub(crate) fn is_session_command(name: &str) -> bool {
    COMMANDS.iter().any(|(wire, ..)| *wire == name)
}

/// `name`'s call for the command `envelope` (sent with `idempotency_key`),
/// or None when it names no session (the control plane answers it as it
/// always has).
pub(crate) fn translate(
    name: &str,
    envelope: &Value,
    idempotency_key: Option<&str>,
) -> Option<Call> {
    let (_, method, tail) = COMMANDS.iter().find(|(wire, ..)| *wire == name)?;
    let mut params: Map<String, Value> = envelope.get("params")?.as_object()?.clone();
    let session = SessionId::parse(params.remove("session_id")?.as_str()?).ok()?;
    let path = format!("/v1/sessions/{session}/{tail}");
    if *method == "GET" {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        for (key, value) in &params {
            match value {
                Value::Null => {}
                Value::Array(items) => {
                    for item in items {
                        query.append_pair(key, &scalar(item));
                    }
                }
                other => {
                    query.append_pair(key, &scalar(other));
                }
            }
        }
        let query = query.finish();
        let path_and_query = if query.is_empty() {
            path
        } else {
            format!("{path}?{query}")
        };
        return Some(Call {
            session,
            method: Method::GET,
            path_and_query,
            body: None,
        });
    }
    params.retain(|_, value| !value.is_null());
    if name == "create_message" {
        let key = params
            .get("client_message_id")
            .and_then(Value::as_str)
            .or(idempotency_key);
        if let Some(key) = key {
            let uuid = uuid::Uuid::new_v5(
                &uuid::Uuid::NAMESPACE_OID,
                format!("message:{session}:{key}").as_bytes(),
            );
            params.insert("id".into(), json!(MessageId::from_uuid(uuid).to_string()));
        }
    }
    Some(Call {
        session,
        method: Method::POST,
        path_and_query: path,
        body: Some(Value::Object(params)),
    })
}

fn scalar(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The command envelope around a `/v1` call's answer.
pub(crate) fn answer(name: &str, envelope: &Value, output: Value) -> Value {
    json!({
        "command": name,
        "api_version": envelope.get("api_version").and_then(Value::as_str).unwrap_or("v1"),
        "schema_hash": envelope.get("schema_hash").and_then(Value::as_str).unwrap_or_default(),
        "output": output,
        "warnings": [],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_command_becomes_the_call_it_names() {
        let session = SessionId::new();
        let sent = translate(
            "create_message",
            &json!({ "params": {
                "session_id": session.to_string(),
                "message": { "content": [{ "type": "text", "text": "hi" }] },
                "client_message_id": null,
            }}),
            None,
        )
        .unwrap();
        assert_eq!(sent.method, Method::POST);
        assert_eq!(
            sent.path_and_query,
            format!("/v1/sessions/{session}/messages")
        );
        assert_eq!(
            sent.body,
            Some(json!({ "message": { "content": [{ "type": "text", "text": "hi" }] } }))
        );

        let read = translate(
            "list_events",
            &json!({ "params": {
                "session_id": session.to_string(),
                "limit": 1,
                "order_desc": true,
                "types": ["turn.completed", "turn.failed"],
            }}),
            None,
        )
        .unwrap();
        assert_eq!(read.method, Method::GET);
        assert_eq!(
            read.path_and_query,
            format!(
                "/v1/sessions/{session}/events?limit=1&order_desc=true&types=turn.completed&types=turn.failed"
            )
        );
        assert_eq!(read.body, None);
    }

    #[test]
    fn a_retried_send_keeps_its_message_id() {
        let session = SessionId::new();
        let envelope = json!({ "params": {
            "session_id": session.to_string(),
            "message": { "content": [{ "type": "text", "text": "hi" }] },
        }});
        let id = |key| {
            translate("create_message", &envelope, key)
                .unwrap()
                .body
                .unwrap()["id"]
                .clone()
        };
        assert_eq!(id(Some("k1")), id(Some("k1")));
        assert_ne!(id(Some("k1")), id(Some("k2")));
        assert_eq!(
            id(None),
            Value::Null,
            "no key, no id: the data plane makes one"
        );
        assert!(MessageId::parse(id(Some("k1")).as_str().unwrap()).is_ok());
    }

    #[test]
    fn other_commands_and_sessionless_ones_stay_with_the_control_plane() {
        assert!(!is_session_command("create_agent"));
        assert!(!is_session_command("archive_session"));
        assert_eq!(
            translate("create_agent", &json!({ "params": {} }), None),
            None
        );
        assert_eq!(
            translate("list_events", &json!({ "params": { "limit": 1 } }), None),
            None
        );
        assert_eq!(
            translate(
                "list_events",
                &json!({ "params": { "session_id": "nope" } }),
                None
            ),
            None
        );
    }
}
