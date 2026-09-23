//! Request layout rules that keep an Anthropic conversation append-only,
//! split out of `driver.rs` to keep that file under its size ratchet.
//!
//! Claude Opus 5.5 and Fable 5.1 bind every thinking block to the conversation
//! prefix that produced it: the top-level `system`, the tools, and every earlier
//! message. When a later request changes that prefix, the API drops the block,
//! or rejects the request for organizations created on or after 2026-08-31. The
//! same edits also restart the prompt cache on every model. Two rules here keep
//! the prefix stable:
//!
//! - Only the leading run of system messages (the agent prompt and anything the
//!   runtime prepends to it) goes into top-level `system`. Notices the runtime
//!   inserts later in the conversation (the hidden-history notice, loop
//!   warnings) are sent in place as mid-conversation `role: "system"` messages
//!   on models that accept them. Folding them into `system` would rewrite the
//!   head of the prefix whenever one appears, changes or goes away.
//! - Requests to preserved-thinking models ask the API to drop, not reject, a
//!   block whose prefix changed. Context management (infinity-context trimming,
//!   compaction masking and summaries) edits earlier history by design, and a
//!   dropped block degrades that turn instead of failing it.

use std::borrow::Cow;

use everruns_provider::driver_registry::{Message, MessageRole};

use super::{
    AnthropicCacheControl, AnthropicContentBlock, AnthropicMessage, MESSAGE_CACHE_BREAKPOINTS,
    normalize_anthropic_id, split_million_context,
};

/// Families that accept `role: "system"` entries inside `messages`.
const MID_CONVERSATION_SYSTEM_FAMILIES: &[&str] = &[
    "claude-fable-5-1",
    "claude-fable-5",
    "claude-opus-5-5",
    "claude-opus-5",
    "claude-opus-4-8",
];

/// Families whose thinking blocks are bound to the conversation prefix.
const PRESERVED_THINKING_FAMILIES: &[&str] = &["claude-fable-5-1", "claude-opus-5-5"];

/// Beta that lets a request choose what happens to a block whose prefix changed.
pub(super) const THINKING_BINDING_BETA: &str = "thinking-binding-controls-2026-08-01";

/// Marks a system message that must stay in place while it passes through
/// `convert_messages`, which only knows how to fold system messages away. The
/// NUL bytes keep it from colliding with real message text.
const IN_PLACE_MARKER: &str = "\u{0}everruns:in-place-system\u{0}";

fn in_families(model: &str, families: &[&str]) -> bool {
    let family = normalize_anthropic_id(split_million_context(model).0);
    families.iter().any(|f| family.eq_ignore_ascii_case(f))
}

/// Whether requests to `model` bind thinking blocks to the conversation and so
/// need the `drop_block` setting and its beta header.
pub(super) fn binds_thinking_to_conversation(model: &str) -> bool {
    in_families(model, PRESERVED_THINKING_FAMILIES)
}

/// Mark every system message after the leading run to be kept in place, on
/// models that accept mid-conversation system messages. Other models get the
/// messages unchanged and keep folding every system message into `system`.
pub(super) fn keep_later_system_messages_in_place<'a>(
    messages: &'a [Message],
    model: &str,
) -> Cow<'a, [Message]> {
    let leading = messages
        .iter()
        .take_while(|m| m.role == MessageRole::System)
        .count();
    let has_later = messages[leading..]
        .iter()
        .any(|m| m.role == MessageRole::System);
    if !has_later || !in_families(model, MID_CONVERSATION_SYSTEM_FAMILIES) {
        return Cow::Borrowed(messages);
    }
    let marked = messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            if index >= leading && message.role == MessageRole::System {
                let text = format!("{IN_PLACE_MARKER}{}", message.content.to_text());
                Message::text(MessageRole::User, text)
            } else {
                message.clone()
            }
        })
        .collect();
    Cow::Owned(marked)
}

/// Turn the marked messages back into `role: "system"` entries at positions
/// the API accepts.
///
/// A mid-conversation system message must follow a user message and be either
/// last or followed by an assistant turn. Each marked message therefore waits
/// until just before the next assistant turn (or the end), so it lands after
/// the user and tool-result messages around it; neighbours are merged into
/// one. The position is a pure function of the conversation, so replaying the
/// same history lays it out the same way. With no user message to follow, the
/// text goes into top-level `system` as it did before.
pub(super) fn place_system_messages(
    system_prompt: &mut Option<String>,
    messages: &mut Vec<AnthropicMessage>,
) {
    if !messages.iter().any(|m| in_place_text(m).is_some()) {
        return;
    }
    let mut placed = Vec::with_capacity(messages.len());
    let mut pending: Vec<String> = Vec::new();
    for message in messages.drain(..) {
        if let Some(text) = in_place_text(&message) {
            pending.push(text.to_string());
            continue;
        }
        if message.role == "assistant" {
            flush(&mut pending, &mut placed, system_prompt);
        }
        placed.push(message);
    }
    flush(&mut pending, &mut placed, system_prompt);
    *messages = placed;
}

fn in_place_text(message: &AnthropicMessage) -> Option<&str> {
    match message.content.as_slice() {
        [AnthropicContentBlock::Text { text, .. }] if message.role == "user" => {
            text.strip_prefix(IN_PLACE_MARKER)
        }
        _ => None,
    }
}

fn flush(
    pending: &mut Vec<String>,
    placed: &mut Vec<AnthropicMessage>,
    system_prompt: &mut Option<String>,
) {
    if pending.is_empty() {
        return;
    }
    let text = pending.join("\n\n");
    pending.clear();
    if placed.last().is_some_and(|m| m.role == "user") {
        placed.push(AnthropicMessage {
            role: "system".to_string(),
            content: vec![AnthropicContentBlock::Text {
                text,
                cache_control: None,
            }],
            preserved_content: None,
        });
    } else {
        *system_prompt = Some(match system_prompt.take() {
            Some(existing) if !existing.is_empty() => format!("{existing}\n\n{text}"),
            _ => text,
        });
    }
}

/// Place the message-level prompt-cache breakpoints on the last text
/// blocks, skipping `volatile_suffix_len` trailing messages.
///
/// A caller can mark trailing messages as volatile (content that will not be
/// replayed on the next request). Anchoring a breakpoint on that tail would
/// make the cached prefix diverge from the next turn's prefix right after the
/// last stable message, evicting the conversation-history cache. Skipping it
/// keeps the breakpoints on stable blocks. Mid-conversation system messages
/// are skipped as well; the breakpoint goes on the user turn before them.
///
/// Two breakpoints, not one, and the pair is what makes caching
/// *incremental*: the newest marks where this turn's history gets written,
/// the one behind it sits at a position the previous turn already wrote, so
/// each turn reads the cache its predecessor created instead of re-paying
/// for the whole transcript. With the system prompt and the tool array this
/// totals four, Anthropic's per-request maximum.
pub(super) fn mark_recent_text_blocks_for_cache(
    messages: &mut [AnthropicMessage],
    volatile_suffix_len: usize,
) {
    let anchor_len = messages.len().saturating_sub(volatile_suffix_len);
    let mut remaining = MESSAGE_CACHE_BREAKPOINTS;
    for msg in messages[..anchor_len].iter_mut().rev() {
        if msg.role == "system" || msg.preserved_content.is_some() {
            continue;
        }
        // At most one breakpoint per message: a second marker inside the
        // same message would spend a scarce breakpoint on a position the
        // first one already covers.
        for block in msg.content.iter_mut().rev() {
            if let AnthropicContentBlock::Text { cache_control, .. } = block {
                *cache_control = Some(AnthropicCacheControl::ephemeral());
                remaining -= 1;
                break;
            }
        }
        if remaining == 0 {
            return;
        }
    }
}

/// Log the thinking blocks the API dropped because the history changed.
///
/// Expected after context management rewrites earlier turns; anything else
/// means the harness edited history it should have appended to.
pub(super) fn log_input_transformations(model: Option<&str>, entries: &[serde_json::Value]) {
    for entry in entries {
        tracing::info!(
            model = model.unwrap_or_default(),
            kind = entry
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or_default(),
            reason = entry
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or_default(),
            path = entry
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or_default(),
            "AnthropicDriver: API transformed request input"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::{AnthropicChatDriver, AnthropicThinking};
    use serde_json::{Value, json};

    fn msg(role: MessageRole, text: &str) -> Message {
        Message::text(role, text)
    }

    /// The request `system` and `messages` as JSON, roles and text only.
    fn layout_for(model: &str, messages: &[Message]) -> (Option<String>, Value) {
        let prepared = keep_later_system_messages_in_place(messages, model);
        let (system, converted) = AnthropicChatDriver::convert_messages(&prepared, false, 0);
        let converted = converted
            .iter()
            .map(|m| {
                let text: Vec<&str> = m
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        AnthropicContentBlock::Text { text, .. } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                json!([m.role, text.join("|")])
            })
            .collect();
        (system, converted)
    }

    #[test]
    fn later_system_messages_stay_in_place_and_system_stays_the_agent_prompt() {
        use MessageRole::*;
        let messages = [
            msg(System, "agent prompt"),
            msg(User, "task"),
            msg(System, "3 earlier messages are not in this context"),
            msg(Assistant, "answer"),
            msg(User, "next"),
            msg(System, "loop warning"),
            msg(User, "<facts>now</facts>"),
        ];
        let (system, converted) = layout_for("claude-opus-5-5", &messages);
        assert_eq!(system.as_deref(), Some("agent prompt"));
        assert_eq!(
            converted,
            json!([
                ["user", "task"],
                ["system", "3 earlier messages are not in this context"],
                ["assistant", "answer"],
                ["user", "next"],
                // Deferred past the user turn it preceded: a system message
                // must be last or followed by an assistant turn.
                ["user", "<facts>now</facts>"],
                ["system", "loop warning"],
            ])
        );
    }

    #[test]
    fn neighbouring_notices_merge_and_one_with_no_user_turn_before_it_folds() {
        use MessageRole::*;
        let messages = [
            msg(System, "agent prompt"),
            msg(User, "task"),
            msg(Assistant, "first"),
            msg(System, "a"),
            msg(System, "b"),
            msg(Assistant, "second"),
            msg(User, "next"),
            msg(System, "c"),
            msg(System, "d"),
        ];
        let (system, converted) = layout_for("claude-fable-5-1", &messages);
        assert_eq!(system.as_deref(), Some("agent prompt\n\na\n\nb"));
        assert_eq!(
            converted,
            json!([
                ["user", "task"],
                ["assistant", "first"],
                ["assistant", "second"],
                ["user", "next"],
                ["system", "c\n\nd"],
            ])
        );
    }

    #[test]
    fn models_without_mid_conversation_system_messages_fold_as_before() {
        use MessageRole::*;
        let messages = [
            msg(System, "agent prompt"),
            msg(User, "task"),
            msg(System, "notice"),
        ];
        for model in ["claude-sonnet-5", "claude-sonnet-4-6", "claude-haiku-4-5"] {
            let (system, converted) = layout_for(model, &messages);
            assert_eq!(system.as_deref(), Some("agent prompt\n\nnotice"), "{model}");
            assert_eq!(converted, json!([["user", "task"]]), "{model}");
        }
    }

    #[test]
    fn replaying_the_same_history_lays_it_out_identically() {
        use MessageRole::*;
        let turn_one = vec![
            msg(System, "agent prompt"),
            msg(User, "task"),
            msg(System, "notice"),
        ];
        let mut turn_two = turn_one.clone();
        turn_two.extend([msg(Assistant, "answer"), msg(User, "next")]);
        let (system_one, one) = layout_for("claude-opus-5-5", &turn_one);
        let (system_two, two) = layout_for("claude-opus-5-5", &turn_two);
        assert_eq!(system_one, system_two);
        let one = one.as_array().unwrap();
        assert_eq!(&two.as_array().unwrap()[..one.len()], one.as_slice());
    }

    #[test]
    fn cache_breakpoints_skip_mid_conversation_system_messages() {
        use MessageRole::*;
        let messages = [
            msg(System, "agent prompt"),
            msg(User, "task"),
            msg(Assistant, "answer"),
            msg(User, "next"),
            msg(System, "notice"),
        ];
        let prepared = keep_later_system_messages_in_place(&messages, "claude-opus-5-5");
        let (_, converted) = AnthropicChatDriver::convert_messages(&prepared, true, 0);
        let marked: Vec<(&str, bool)> = converted
            .iter()
            .map(|m| {
                let marked = m.content.iter().any(|b| {
                    matches!(
                        b,
                        AnthropicContentBlock::Text {
                            cache_control: Some(_),
                            ..
                        }
                    )
                });
                (m.role.as_str(), marked)
            })
            .collect();
        assert_eq!(
            marked,
            [
                ("user", false),
                ("assistant", true),
                ("user", true),
                ("system", false),
            ]
        );
    }

    #[test]
    fn preserved_thinking_models_ask_the_api_to_drop_mismatched_blocks() {
        for (model, binds) in [
            ("claude-opus-5-5", true),
            ("claude-fable-5-1[1m]", true),
            ("claude-opus-5", false),
            ("claude-opus-4-8", false),
        ] {
            let thinking = serde_json::to_value(AnthropicThinking::adaptive(model)).unwrap();
            let mut expected = json!({"type": "adaptive", "display": "summarized"});
            if binds {
                expected["block_binding"] = json!({"prefix_mismatch_behavior": "drop_block"});
            }
            assert_eq!(thinking, expected, "{model}");
            assert_eq!(binds_thinking_to_conversation(model), binds, "{model}");
        }
    }
}
