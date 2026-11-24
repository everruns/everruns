//! Warn the model before the per-turn model-call limit ends its turn.
//!
//! At `max_iterations` the turn stops even when the model asked for more
//! tools, so a long unattended run (a scheduled agent) ended mid-work with no
//! report and no continuation scheduled (Housekeeping agent, 2026-11-24, 500
//! calls). For the last few calls a turn-scoped reminder at the tail says how
//! many calls are left, so the model can schedule a continuation and answer.
//!
//! The reminder trails the request and is not stored, so earlier requests'
//! cached prefixes are unchanged; it only shifts the tail of the last calls.

use crate::engine::message::RuntimeMessage;

/// Calls, counting the current one, that carry the reminder.
const WARN_WITHIN: u32 = 3;

/// The reminder for `iteration` (1-based) of a turn limited to `max`, if any.
pub(super) fn notice(iteration: u32, max: usize) -> Option<String> {
    let max = u32::try_from(max).unwrap_or(u32::MAX);
    if max == 0 || iteration > max {
        return None;
    }
    let left = max - iteration + 1;
    if left > WARN_WITHIN {
        return None;
    }
    Some(if left == 1 {
        format!(
            "Turn limit: this is the last model call of this turn ({max} calls). \
Tool calls you request now will not run. Answer now with a report of what you did, \
what is unfinished and what should happen next."
        )
    } else {
        format!(
            "Turn limit: {left} model calls are left in this turn, including this one \
(limit {max}). After the last one the turn stops and requested tool calls do not run. \
Wrap up now: if work remains, schedule a continuation if you can, then answer with a \
report of what you did and what is unfinished."
        )
    })
}

/// Append the reminder to interleaved messages and extend their volatile
/// suffix, which context updates stay in front of.
pub(in crate::engine::execution::reason) fn with_notice(
    (mut messages, volatile): (Vec<RuntimeMessage>, usize),
    iteration: u32,
    max: usize,
) -> (Vec<RuntimeMessage>, usize) {
    let added = append_notice(&mut messages, iteration, max);
    (messages, volatile + added)
}

/// Append the reminder when it applies. Returns how many trailing messages it
/// added.
fn append_notice(messages: &mut Vec<RuntimeMessage>, iteration: u32, max: usize) -> usize {
    match notice(iteration, max) {
        Some(text) => {
            messages.push(RuntimeMessage::turn_scoped_system(text));
            1
        }
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warns_only_for_the_last_three_calls() {
        assert!(notice(1, 500).is_none());
        assert!(notice(497, 500).is_none());
        assert!(notice(498, 500).unwrap().contains("3 model calls are left"));
        assert!(notice(499, 500).unwrap().contains("2 model calls are left"));
        assert!(notice(500, 500).unwrap().contains("last model call"));
        assert!(notice(501, 500).is_none());
    }

    #[test]
    fn tiny_limits_warn_from_the_first_call_and_zero_never_warns() {
        assert!(notice(1, 1).unwrap().contains("last model call"));
        assert!(notice(1, 2).unwrap().contains("2 model calls are left"));
        assert!(notice(1, 0).is_none());
    }

    #[test]
    fn the_reminder_is_turn_scoped_and_trails_the_request() {
        let mut messages = vec![RuntimeMessage::user("task")];
        assert_eq!(append_notice(&mut messages, 10, 500), 0);
        assert_eq!(messages.len(), 1);
        assert_eq!(append_notice(&mut messages, 500, 500), 1);
        let last = messages.last().unwrap();
        assert!(last.is_turn_scoped_reminder());
        assert!(last.text().unwrap_or_default().contains("last model call"));
    }
}
