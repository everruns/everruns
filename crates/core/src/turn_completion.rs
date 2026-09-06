//! Did the turn finish the user's request, or just stop?
//!
//! A turn ending is not the same as work being done. A turn that called tools
//! and produced no text has stopped mid-task; a turn that ended with a question
//! is waiting on the user; a turn whose detached background run is still going
//! is neither finished nor stuck. A host that auto-continues needs to tell these
//! apart, and every host that has tried has written the same classification.
//!
//! This is the cheap half, and deliberately only the cheap half: a pure
//! function over what the turn already reported. It reaches a verdict on the
//! clear-cut cases and answers [`GateDecision::Evaluate`] on the ambiguous one —
//! tool-using work that produced a candidate final answer — where only a
//! semantic check (an evaluator model, a goal capability) can decide. Hosts pay
//! for that check on the small fraction of turns that need it.
//!
//! [`ContinuationBudget`] bounds whatever the host does next. Auto-continuation
//! that is not bounded in turns, tokens, *and* wall-clock is a runaway; all
//! three limits exist because each one alone has been observed to leak.
//!
//! Ported from yolop, where this gate sits between the turn loop and the
//! terminal/ACP hosts.

use std::time::{Duration, Instant};

use crate::turn::TurnStopReason;

/// Default ceiling on automatic continuations for one user request.
pub const DEFAULT_MAX_CONTINUATION_TURNS: u32 = 6;

/// Default token ceiling across those continuations.
pub const DEFAULT_MAX_CONTINUATION_TOKENS: u64 = 64_000;

/// Default wall-clock ceiling for one user request.
pub const DEFAULT_MAX_CONTINUATION_ELAPSED: Duration = Duration::from_secs(10 * 60);

/// Where the user's request stands after a turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionState {
    /// A final answer was delivered.
    Achieved,
    /// Stopped for a reason only the user can clear — cancelled, or a question
    /// was asked back.
    Blocked,
    /// Ended in a permanent failure.
    Failed,
    /// Detached background work is still running; the request is neither done
    /// nor stuck.
    WaitingOnBackground,
    /// Stopped without a final answer, and continuing would make progress.
    InProgress,
}

/// The gate's verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateDecision {
    /// Decided from the turn alone.
    Conclusive(CompletionState),
    /// Tool-using work produced a candidate final answer. Mutations and
    /// multi-step work are ambiguous enough to justify a semantic check.
    Evaluate,
}

/// What the gate reads from a finished turn.
///
/// A view rather than a concrete turn result, because the runtime and the
/// in-memory loop have their own; both can fill this in.
#[derive(Clone, Copy, Debug)]
pub struct TurnSummary<'a> {
    /// Whether the turn completed without an unrecoverable failure.
    pub success: bool,
    /// Why the turn stopped.
    pub stop_reason: TurnStopReason,
    /// Final text the turn produced.
    pub response: &'a str,
    /// How many tool calls ran during the turn.
    pub tool_calls_count: usize,
    /// Whether the session has detached background work still running.
    pub has_active_background: bool,
}

/// Classify a finished turn.
pub fn gate_turn(summary: &TurnSummary<'_>) -> GateDecision {
    if !summary.success {
        return GateDecision::Conclusive(match summary.stop_reason {
            TurnStopReason::Cancelled => CompletionState::Blocked,
            _ => CompletionState::Failed,
        });
    }

    match summary.stop_reason {
        TurnStopReason::Error | TurnStopReason::Refusal => {
            return GateDecision::Conclusive(CompletionState::Failed);
        }
        TurnStopReason::Cancelled => {
            return GateDecision::Conclusive(CompletionState::Blocked);
        }
        // Hitting a token or request ceiling is the definition of stopped
        // mid-task, whatever text came with it.
        TurnStopReason::MaxTokens | TurnStopReason::MaxTurnRequests => {
            return GateDecision::Conclusive(CompletionState::InProgress);
        }
        TurnStopReason::EndTurn => {}
    }

    // Tools ran and produced no text: the model handed off to work that is
    // still going, rather than finishing.
    if summary.has_active_background && summary.tool_calls_count > 0 {
        return GateDecision::Conclusive(CompletionState::WaitingOnBackground);
    }

    if summary.response.trim().is_empty() {
        return GateDecision::Conclusive(CompletionState::InProgress);
    }

    if asks_the_user_something(summary.response) {
        return GateDecision::Conclusive(CompletionState::Blocked);
    }

    if summary.tool_calls_count == 0 {
        // No tools, plain text: an answer, and nothing to second-guess.
        GateDecision::Conclusive(CompletionState::Achieved)
    } else {
        GateDecision::Evaluate
    }
}

/// A trailing question that also reads like a request for input.
///
/// The question mark alone is not enough — "shall I continue? I'll start with
/// the parser." is not a block — so a marker phrase must appear too. Cheap and
/// deliberately conservative: a missed block costs one wasted continuation,
/// while a false block strands the user waiting.
fn asks_the_user_something(response: &str) -> bool {
    let normalized = response.trim().to_ascii_lowercase();
    normalized.ends_with('?')
        && ["need", "which", "what", "could you", "please provide"]
            .iter()
            .any(|marker| normalized.contains(marker))
}

/// Bounds automatic continuation for one user request.
///
/// Turns, tokens, and elapsed time are all enforced: a cheap loop exhausts
/// turns, an expensive one exhausts tokens, and one that stalls on slow calls
/// exhausts neither.
#[derive(Clone, Debug)]
pub struct ContinuationBudget {
    started: Instant,
    turns: u32,
    tokens: u64,
    max_turns: u32,
    max_tokens: u64,
    max_elapsed: Duration,
}

impl Default for ContinuationBudget {
    fn default() -> Self {
        Self::new(
            DEFAULT_MAX_CONTINUATION_TURNS,
            DEFAULT_MAX_CONTINUATION_TOKENS,
            DEFAULT_MAX_CONTINUATION_ELAPSED,
        )
    }
}

impl ContinuationBudget {
    /// Build a budget with explicit limits.
    pub fn new(max_turns: u32, max_tokens: u64, max_elapsed: Duration) -> Self {
        Self {
            started: Instant::now(),
            turns: 0,
            tokens: 0,
            max_turns,
            max_tokens,
            max_elapsed,
        }
    }

    /// Start a fresh request, keeping the configured limits.
    pub fn reset(&mut self) {
        *self = Self::new(self.max_turns, self.max_tokens, self.max_elapsed);
    }

    /// Record a turn and report whether continuing stays within budget.
    ///
    /// The turn being recorded is counted first, so the call that crosses a
    /// limit returns `false` — the budget is a ceiling on work done, not on
    /// work attempted.
    pub fn observe_turn(&mut self, tokens: u64) -> bool {
        self.turns = self.turns.saturating_add(1);
        self.tokens = self.tokens.saturating_add(tokens);
        self.turns <= self.max_turns
            && self.tokens <= self.max_tokens
            && self.started.elapsed() <= self.max_elapsed
    }

    /// Turns and tokens spent so far.
    pub fn usage(&self) -> (u32, u64) {
        (self.turns, self.tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary<'a>(response: &'a str, tool_calls_count: usize, success: bool) -> TurnSummary<'a> {
        TurnSummary {
            success,
            stop_reason: if success {
                TurnStopReason::EndTurn
            } else {
                TurnStopReason::Error
            },
            response,
            tool_calls_count,
            has_active_background: false,
        }
    }

    #[test]
    fn end_turn_gate_distinguishes_answers_work_and_background_precedence() {
        for (response, calls, background, expected) in [
            (
                "Done.",
                0,
                false,
                GateDecision::Conclusive(CompletionState::Achieved),
            ),
            ("Done.", 1, false, GateDecision::Evaluate),
            (
                "",
                1,
                false,
                GateDecision::Conclusive(CompletionState::InProgress),
            ),
            (
                " \n\u{2003}",
                0,
                false,
                GateDecision::Conclusive(CompletionState::InProgress),
            ),
            (
                "",
                1,
                true,
                GateDecision::Conclusive(CompletionState::WaitingOnBackground),
            ),
            (
                "Done.",
                1,
                true,
                GateDecision::Conclusive(CompletionState::WaitingOnBackground),
            ),
            (
                "Which environment?",
                1,
                true,
                GateDecision::Conclusive(CompletionState::WaitingOnBackground),
            ),
            (
                "Done.",
                0,
                true,
                GateDecision::Conclusive(CompletionState::Achieved),
            ),
        ] {
            let mut view = summary(response, calls, true);
            view.has_active_background = background;
            assert_eq!(gate_turn(&view), expected, "{view:?}");
        }
    }

    #[test]
    fn stop_reason_and_failure_take_precedence_over_text_and_background() {
        for (reason, expected) in [
            (TurnStopReason::Error, CompletionState::Failed),
            (TurnStopReason::Refusal, CompletionState::Failed),
            (TurnStopReason::Cancelled, CompletionState::Blocked),
            (TurnStopReason::MaxTokens, CompletionState::InProgress),
            (TurnStopReason::MaxTurnRequests, CompletionState::InProgress),
        ] {
            let view = TurnSummary {
                success: true,
                stop_reason: reason,
                response: "Which environment?",
                tool_calls_count: 2,
                has_active_background: true,
            };
            assert_eq!(gate_turn(&view), GateDecision::Conclusive(expected));
        }
        for reason in [
            TurnStopReason::EndTurn,
            TurnStopReason::Error,
            TurnStopReason::Refusal,
            TurnStopReason::MaxTokens,
            TurnStopReason::MaxTurnRequests,
            TurnStopReason::Cancelled,
        ] {
            let view = TurnSummary {
                success: false,
                stop_reason: reason,
                response: "Done.",
                tool_calls_count: 2,
                has_active_background: true,
            };
            let expected = if reason == TurnStopReason::Cancelled {
                CompletionState::Blocked
            } else {
                CompletionState::Failed
            };
            assert_eq!(
                gate_turn(&view),
                GateDecision::Conclusive(expected),
                "{reason:?}"
            );
        }
    }

    #[test]
    fn questions_require_both_a_marker_and_trailing_question_mark() {
        for response in [
            "Which environment?",
            " WHAT should change? \n",
            "Could you clarify?",
            "Please provide the file?",
            "Need more details?",
        ] {
            for calls in [0, 1] {
                assert_eq!(
                    gate_turn(&summary(response, calls, true)),
                    GateDecision::Conclusive(CompletionState::Blocked),
                    "{response}"
                );
            }
        }
        for response in [
            "Ready to ship?",
            "Which file? I will start with the parser.",
            "Need more details.",
        ] {
            assert_eq!(
                gate_turn(&summary(response, 0, true)),
                GateDecision::Conclusive(CompletionState::Achieved),
                "{response}"
            );
            assert_eq!(
                gate_turn(&summary(response, 1, true)),
                GateDecision::Evaluate,
                "{response}"
            );
        }
    }

    #[test]
    fn default_budget_enforces_literal_turn_and_token_ceilings() {
        let mut budget = ContinuationBudget::default();
        for turn in 1..=6 {
            assert!(budget.observe_turn(0));
            assert_eq!(budget.usage(), (turn, 0));
        }
        assert!(!budget.observe_turn(0));
        assert_eq!(budget.usage(), (7, 0));
        budget.reset();
        assert!(budget.observe_turn(64_000));
        assert!(!budget.observe_turn(1));
        assert_eq!(budget.usage(), (2, 64_001));
        budget.reset();
        budget.started = Instant::now() - Duration::from_secs(601);
        assert!(!budget.observe_turn(0));
    }

    #[test]
    fn custom_budget_reset_clears_usage_and_keeps_all_configured_limits() {
        let mut budget = ContinuationBudget::new(2, 3, Duration::from_secs(60));
        assert!(budget.observe_turn(1));
        assert!(budget.observe_turn(2));
        assert_eq!(budget.usage(), (2, 3));
        assert!(!budget.observe_turn(0));
        budget.reset();
        assert_eq!(budget.usage(), (0, 0));
        assert!(budget.observe_turn(0));
        assert!(budget.observe_turn(0));
        assert!(!budget.observe_turn(0));
        budget.reset();
        assert!(!budget.observe_turn(4));
        assert_eq!(budget.usage(), (1, 4));
        budget.reset();
        budget.started = Instant::now() - Duration::from_secs(61);
        assert!(!budget.observe_turn(0));
        budget.reset();
        assert!(budget.observe_turn(1));
        assert_eq!(budget.usage(), (1, 1));
        for (turns, tokens) in [(0, 100), (100, 0)] {
            assert!(
                !ContinuationBudget::new(turns, tokens, Duration::from_secs(60)).observe_turn(1)
            );
        }
    }

    #[test]
    fn an_elapsed_budget_stops_a_cheap_but_slow_loop() {
        let mut budget = ContinuationBudget::new(100, u64::MAX, Duration::from_secs(1));
        budget.started = Instant::now() - Duration::from_secs(2);
        assert!(!budget.observe_turn(1));
        assert_eq!(budget.usage(), (1, 1));
    }

    #[test]
    fn usage_saturates_instead_of_wrapping_after_overflow() {
        let mut budget = ContinuationBudget::new(u32::MAX, u64::MAX - 1, Duration::from_secs(60));
        budget.turns = u32::MAX;
        budget.tokens = u64::MAX - 1;
        assert!(!budget.observe_turn(10));
        assert_eq!(budget.usage(), (u32::MAX, u64::MAX));
        assert!(!budget.observe_turn(1));
        assert_eq!(budget.usage(), (u32::MAX, u64::MAX));
    }
}
