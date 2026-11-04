//! Where the live window starts.
//!
//! Decision (2026-11-01): the window start moves in steps, not one message at
//! a time. A provider prompt cache matches an exact request prefix, and the
//! window start plus the "N earlier messages" notice sit at the very front of
//! the request. Re-trimming to the tightest fit on every call moved both on
//! every model call, so a session over its budget never read its history from
//! cache (a Muse session at ~385k tokens: 1,393 of 385k cached per call).
//!
//! The cut is rounded up to the next multiple of a fixed step on the
//! conversation's absolute message index. The same message stays first, and
//! the notice keeps its count, until the history grows past the next step,
//! so the request only appends in between. The step is about a quarter of the
//! budget at the candidate-load average message size, so a session pays one
//! full prompt read per quarter-budget of new history. Absolute indexes keep
//! the cut fixed while the database's candidate load slides along the tail.

use super::config::{CANDIDATE_AVG_TOKENS_PER_MESSAGE, InfinityContextConfig};
use super::{estimate_message_tokens, resolve_keep_first_messages};
use crate::builtins::message::RuntimeMessage;
use crate::builtins::message_filter::anchored_window;

/// Outcome of token-budget trimming.
#[derive(Default)]
pub(super) struct TrimOutcome {
    /// Messages dropped from the middle.
    pub(super) hidden_count: usize,
    /// Length of the preserved leading anchor (where the notice is inserted).
    pub(super) head_len: usize,
}

/// Messages the window start advances by when it has to move.
fn cut_step(config: &InfinityContextConfig) -> usize {
    (config.context_budget_tokens / 4 / CANDIDATE_AVG_TOKENS_PER_MESSAGE).max(1)
}

/// Round the tightest window start up to the next step boundary.
///
/// `tightest` indexes `costs`; `already_hidden` messages between the head and
/// `costs[head_len]` were left out by the database load, so `costs[i]` is
/// message `already_hidden + i` of the conversation. Returns `tightest` when
/// nothing is trimmed, or when the rounded cut would leave less than the
/// minimum recent messages or under half the budget: a stable prefix is not
/// worth starving the model of context.
pub(super) fn stable_window_start(
    costs: &[usize],
    head_len: usize,
    tightest: usize,
    already_hidden: usize,
    min_recent: usize,
    config: &InfinityContextConfig,
) -> usize {
    if tightest == head_len && already_hidden == 0 {
        return tightest;
    }
    let step = cut_step(config);
    let absolute = already_hidden + tightest;
    let stable = absolute.div_ceil(step) * step - already_hidden;
    let tail_floor = costs.len().saturating_sub(min_recent).max(tightest);
    if stable > tail_floor {
        return tightest;
    }
    let kept: usize =
        costs[..head_len].iter().sum::<usize>() + costs[stable..].iter().sum::<usize>();
    if kept < config.context_budget_tokens / 2 {
        return tightest;
    }
    stable
}

/// Trim the live window to the token budget while always keeping the first
/// `keep_first_messages` (the original task/goal) and the recent tail. Drops a
/// single contiguous block from the middle and reports how many were hidden.
pub(super) fn trim_messages_to_token_budget(
    messages: &mut Vec<RuntimeMessage>,
    config: &InfinityContextConfig,
    already_hidden: usize,
) -> TrimOutcome {
    if messages.is_empty() {
        return TrimOutcome::default();
    }

    let costs: Vec<usize> = messages.iter().map(estimate_message_tokens).collect();
    let window = anchored_window(
        &costs,
        resolve_keep_first_messages(config),
        config.min_recent_messages,
        config.max_recent_messages,
        config.context_budget_tokens,
    );
    let recent_start = stable_window_start(
        &costs,
        window.head_len,
        window.recent_start,
        already_hidden,
        config.min_recent_messages,
        config,
    );

    let hidden_count = recent_start - window.head_len;
    if hidden_count > 0 {
        // Rebuild as [0, head_len) ++ [recent_start, len) without cloning.
        let tail = messages.split_off(recent_start);
        messages.truncate(window.head_len);
        messages.extend(tail);
    }

    TrimOutcome {
        hidden_count,
        head_len: window.head_len,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(budget: usize) -> InfinityContextConfig {
        InfinityContextConfig {
            context_budget_tokens: budget,
            min_recent_messages: 2,
            ..InfinityContextConfig::default()
        }
    }

    /// The first kept message and the hidden count over successive calls of a
    /// growing conversation, each call loading the whole history.
    fn windows(config: &InfinityContextConfig, calls: usize, start_len: usize) -> Vec<usize> {
        (0..calls)
            .map(|call| {
                let costs = vec![CANDIDATE_AVG_TOKENS_PER_MESSAGE; start_len + call];
                let window = anchored_window(&costs, 0, 2, None, config.context_budget_tokens);
                stable_window_start(&costs, 0, window.recent_start, 0, 2, config)
            })
            .collect()
    }

    #[test]
    fn over_budget_history_keeps_its_window_start_between_steps() {
        // 100k budget = 400 messages of 250 tokens; the step is 100 messages.
        let config = config(100_000);
        let starts = windows(&config, 120, 450);
        // The tightest fit would move every call; the stable start moves once
        // per 100 messages of growth and always lands on a step boundary.
        assert_eq!(starts[0], 100);
        assert!(starts[..51].iter().all(|start| *start == 100));
        assert!(starts[51..].iter().all(|start| *start == 200));
        for (call, start) in starts.iter().enumerate() {
            let len = 450 + call;
            let kept = (len - start) * CANDIDATE_AVG_TOKENS_PER_MESSAGE;
            assert!((50_000..=100_000).contains(&kept), "call {call}: {kept}");
        }
    }

    #[test]
    fn under_budget_history_is_never_trimmed() {
        assert!(
            windows(&config(100_000), 30, 10)
                .iter()
                .all(|start| *start == 0)
        );
    }

    #[test]
    fn cut_is_fixed_on_the_conversation_while_the_database_load_slides() {
        let config = config(100_000);
        // The database loads the latest 1,600 messages of a growing session;
        // the window start names the same conversation message each call.
        let absolute: Vec<usize> = (0..60)
            .map(|call| {
                let total = 3_001 + call;
                let already_hidden = total - 1_600;
                let costs = vec![CANDIDATE_AVG_TOKENS_PER_MESSAGE; 1_600];
                let window = anchored_window(&costs, 0, 2, None, 100_000);
                already_hidden
                    + stable_window_start(
                        &costs,
                        0,
                        window.recent_start,
                        already_hidden,
                        2,
                        &config,
                    )
            })
            .collect();
        assert!(absolute.iter().all(|start| *start == absolute[0]));
        assert_eq!(absolute[0] % cut_step(&config), 0);
    }

    #[test]
    fn rounding_never_starves_the_window() {
        let config = config(100_000);
        // The tightest start is message 101, and messages 101..200 hold most of
        // the budget: rounding up to 200 would keep 1k tokens of 100k.
        let mut costs = vec![10_000; 101];
        costs.extend(vec![910; 99]);
        costs.extend(vec![100; 10]);
        let window = anchored_window(&costs, 0, 2, None, 100_000);
        assert_eq!(window.recent_start, 101);
        let start = stable_window_start(&costs, 0, window.recent_start, 0, 2, &config);
        assert_eq!(start, 101);
        // And never past the minimum recent messages.
        let costs = vec![60_000; 5];
        let window = anchored_window(&costs, 0, 2, None, 100_000);
        assert_eq!(
            stable_window_start(&costs, 0, window.recent_start, 0, 2, &config),
            window.recent_start
        );
    }
}
