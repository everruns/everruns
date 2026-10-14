//! Files sent with `agent_run` and `session_send_message`.
//!
//! Decision: the tools take the execution API's `files` shape (base64 `data`
//! or a public https `url` per file) rather than upload ids, so an MCP client
//! that holds a file can send it in one call. Each goes through the shared
//! attachment intake as an upload of the caller's org, and the message refers
//! to it; message creation then copies it into the session's `/attachments`.
//! A file that cannot be taken becomes a note the agent reads.

use everruns_contracts::execution_api::MessageFile;
use everruns_core::InputContentPart;
use serde_json::{Value, json};

use super::AppState;
use crate::auth::ResolvedOrg;
use crate::domains::messages::intake::receive_files;

/// JSON Schema of the `files` argument.
pub(super) fn files_schema() -> Value {
    json!({
        "type": "array",
        "description": "Files to send with the message, each kept in the session's /attachments. Give each file `data` (base64) or `url` (public https), plus optional `name` and `media_type`.",
        "maxItems": 10,
        "items": {
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "media_type": { "type": "string" },
                "data": { "type": "string", "contentEncoding": "base64" },
                "url": { "type": "string", "format": "uri" }
            },
            "additionalProperties": false
        }
    })
}

/// The user message for `text` and the call's `files`.
pub(super) async fn user_message(
    text: &str,
    args: &Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> Result<Value, String> {
    let files: Vec<MessageFile> = match args.get("files") {
        None | Some(Value::Null) => Vec::new(),
        Some(files) => serde_json::from_value(files.clone())
            .map_err(|error| format!("Invalid parameter files: {error}"))?,
    };
    let mut content = vec![InputContentPart::text(text)];
    content.extend(receive_files(&state.db, Some(&state.egress), org.org_id, "mcp", files).await);
    Ok(json!({ "role": "user", "content": content }))
}
