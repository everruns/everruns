use super::{AppState, ResolvedOrg, dispatch_command};
use everruns_core::events::{CONVERSATION_MESSAGE, OUTPUT_MESSAGE_COMPLETED};
use serde_json::json;

pub(super) async fn latest_output(
    session_id: &str,
    org: &ResolvedOrg,
    state: &AppState,
) -> Result<Option<String>, String> {
    // A status snapshot must not depend on the caller's incremental event page.
    // Read backwards in bounded pages: commentary can outnumber an entire page,
    // and a silent turn must not erase what the agent last said.
    let mut before_sequence = None;
    loop {
        let result = dispatch_command(
            "list_events",
            json!({
                "session_id": session_id,
                "types": [OUTPUT_MESSAGE_COMPLETED, CONVERSATION_MESSAGE],
                "order_desc": true,
                "before_sequence": before_sequence,
                "limit": 50,
            }),
            org,
            state,
        )
        .await?;
        let events = result["data"].as_array().ok_or("Invalid event page")?;
        if let Some(text) = events.iter().find_map(|event| {
            everruns_core::conversation::said_in_event(
                event.get("type")?.as_str()?,
                event.get("data")?,
            )
        }) {
            return Ok(Some(text));
        }
        let Some(last) = events.last() else {
            return Ok(None);
        };
        before_sequence = Some(last["sequence"].as_i64().ok_or("Missing event sequence")?);
    }
}
