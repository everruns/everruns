use super::*;
use everruns_core::MESSAGE_REACTION_REMOVED;
use serde_json::json;

fn event(
    sequence: i64,
    added: bool,
    actor: (&str, &str),
    emoji: &str,
    message: &str,
) -> ReactionEventRow {
    let kind = if actor.0 == "agent" { "agent" } else { "user" };
    ReactionEventRow {
        sequence,
        event_type: if added {
            MESSAGE_REACTION_ADDED
        } else {
            MESSAGE_REACTION_REMOVED
        }
        .to_string(),
        data: json!({
            "message_id": message,
            "emoji": emoji,
            "actor": {"kind": kind, "id": actor.0, "name": actor.1},
        }),
        created_at: Utc::now(),
    }
}

fn agent_messages() -> HashMap<String, String> {
    HashMap::from([(
        "message_reply".to_string(),
        "Deployed 0.50.1 to production.".to_string(),
    )])
}

#[test]
fn adding_twice_and_removing_fold_to_the_latest_state() {
    let events = vec![
        event(1, true, ("u1", "Mykhailo"), "👍", "message_reply"),
        event(2, true, ("u1", "Mykhailo"), "👍", "message_reply"),
        event(3, true, ("u2", "Alice"), "👍", "message_reply"),
        event(4, false, ("u1", "Mykhailo"), "👍", "message_reply"),
    ];
    let state = fold(&events, None);
    let active: Vec<&str> = state
        .iter()
        .filter(|(_, s)| s.active)
        .map(|(k, _)| k.actor_id.as_str())
        .collect();
    assert_eq!(active, vec!["u2"]);
    // Folding only up to sequence 2 still has the first reaction on.
    assert!(
        fold(&events, Some(2))
            .values()
            .any(|s| s.active && s.actor.id == "u1")
    );
}

#[test]
fn untold_reactions_become_note_items_in_order() {
    let events = vec![
        event(5, true, ("u1", "Mykhailo"), "👍", "message_reply"),
        event(6, true, ("u2", "Alice"), "🎉", "message_reply"),
    ];
    let items = untold_items(&events, None, &agent_messages());
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].actor_name, "Mykhailo");
    assert_eq!(items[1].emoji, "🎉");
    assert_eq!(
        items[0].excerpt.as_deref(),
        Some("Deployed 0.50.1 to production.")
    );
    assert!(
        items
            .iter()
            .all(|item| !item.removed && item.on_agent_message)
    );
}

#[test]
fn reactions_already_told_are_not_told_again() {
    let events = vec![
        event(5, true, ("u1", "Mykhailo"), "👍", "message_reply"),
        event(6, true, ("u2", "Alice"), "🎉", "message_reply"),
    ];
    let items = untold_items(&events, Some(6), &agent_messages());
    assert!(items.is_empty());
    let items = untold_items(&events, Some(5), &agent_messages());
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].actor_name, "Alice");
}

#[test]
fn an_add_and_remove_between_notes_cancel_out() {
    let events = vec![
        event(5, true, ("u1", "Mykhailo"), "👍", "message_reply"),
        event(6, false, ("u1", "Mykhailo"), "👍", "message_reply"),
    ];
    assert!(untold_items(&events, None, &agent_messages()).is_empty());
}

#[test]
fn taking_back_a_told_reaction_is_told() {
    let events = vec![
        event(5, true, ("u1", "Mykhailo"), "👍", "message_reply"),
        event(9, false, ("u1", "Mykhailo"), "👍", "message_reply"),
    ];
    let items = untold_items(&events, Some(5), &agent_messages());
    assert_eq!(items.len(), 1);
    assert!(items[0].removed);
}

#[test]
fn agent_reactions_and_reactions_on_other_messages_are_never_told() {
    let events = vec![
        event(5, true, ("agent", "Release Bot"), "👀", "message_reply"),
        event(6, true, ("u1", "Mykhailo"), "👍", "message_person"),
    ];
    assert!(untold_items(&events, None, &agent_messages()).is_empty());
}

#[test]
fn message_text_joins_text_parts() {
    let message = json!({"content": [{"type": "text", "text": "a"}, {"type": "image"}, {"type": "text", "text": "b"}]});
    assert_eq!(message_text(&message), "a\nb");
    assert_eq!(message_text(&json!({})), "");
}
