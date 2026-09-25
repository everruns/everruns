// The server assigns a durable starter identity, including requests from older
// browser bundles. Browser-local checks cannot arbitrate between clients.

use super::types::CreateSessionRequest;
use crate::domains::common::CommandError;

pub(crate) const PLATFORM_CHAT_STARTER_TAG: &str = "platform-chat-starter";

pub(crate) fn mark_platform_chat_starter(
    req: &mut CreateSessionRequest,
    harness_name: &str,
    has_agent: bool,
) -> Result<(), CommandError> {
    let is_starter = harness_name == "platform-chat"
        && !has_agent
        && req.title.as_deref() == Some("Platform Chat")
        && req.tags.iter().any(|tag| tag == "chat")
        && req.parent_session_id.is_none()
        && req.forked_from_session_id.is_none();
    let has_marker = req.tags.iter().any(|tag| tag == PLATFORM_CHAT_STARTER_TAG);
    if has_marker && !is_starter {
        return Err(CommandError::bad_request(
            "platform-chat-starter is reserved for the Platform Chat starter",
        ));
    }
    if is_starter && !has_marker {
        req.tags.push(PLATFORM_CHAT_STARTER_TAG.to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn legacy_platform_chat_create_gets_starter_identity() {
        let mut req: CreateSessionRequest = serde_json::from_value(json!({
            "source": "chat", "title": "Platform Chat", "tags": ["chat"]
        }))
        .unwrap();
        mark_platform_chat_starter(&mut req, "platform-chat", false).unwrap();
        assert_eq!(req.tags, ["chat", PLATFORM_CHAT_STARTER_TAG]);

        let mut older_req: CreateSessionRequest = serde_json::from_value(json!({
            "title": "Platform Chat", "tags": ["chat"]
        }))
        .unwrap();
        mark_platform_chat_starter(&mut older_req, "platform-chat", false).unwrap();
        assert!(
            older_req
                .tags
                .contains(&PLATFORM_CHAT_STARTER_TAG.to_string())
        );

        let mut ordinary: CreateSessionRequest = serde_json::from_value(json!({
            "source": "chat", "tags": ["chat"]
        }))
        .unwrap();
        mark_platform_chat_starter(&mut ordinary, "platform-chat", false).unwrap();
        assert_eq!(ordinary.tags, ["chat"]);

        ordinary.tags.push(PLATFORM_CHAT_STARTER_TAG.to_string());
        assert!(mark_platform_chat_starter(&mut ordinary, "generic", false).is_err());
    }
}
