use super::*;
use crate::typed_id::SessionId;

#[test]
fn provider_managed_reduction_preserves_the_complete_loaded_history() {
    let provider = InfinityContextFilterProvider;
    let config = message_filter_config(
        &json!({
            "context_budget_tokens": 1,
            "min_recent_messages": 1,
            "max_recent_messages": 1,
            "keep_first_messages": 1
        }),
        false,
        true,
    );
    let mut query = MessageQuery::new(SessionId::new());
    provider.apply_filters(&mut query, &config);
    assert_eq!(query.limit, None);
    assert_eq!(query.keep_head, None);
    assert!(query.prepend_transform.is_none());

    let original = vec![
        RuntimeMessage::user("first"),
        RuntimeMessage::assistant("middle ".repeat(400)),
        RuntimeMessage::user("last"),
    ];
    let mut messages = original.clone();
    provider.post_load(&mut messages, &config);
    assert_eq!(
        serde_json::to_value(&messages).unwrap(),
        serde_json::to_value(&original).unwrap()
    );
    assert!(
        messages
            .iter()
            .all(|message| !extract_text_content(message).contains("not in this context"))
    );
}
