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
    fn plain_text_with_no_tools_is_achieved_without_an_evaluator() {
        assert_eq!(
            gate_turn(&summary("Here is the summary you asked for.", 0, true)),
            GateDecision::Conclusive(CompletionState::Achieved)
        );
    }

    #[test]
    fn a_tool_only_turn_is_in_progress() {
        assert_eq!(
            gate_turn(&summary("", 1, true)),
            GateDecision::Conclusive(CompletionState::InProgress)
        );
    }

    #[test]
    fn a_tool_only_turn_with_background_work_is_waiting_not_stalled() {
        let mut view = summary("", 1, true);
        view.has_active_background = true;
        assert_eq!(
            gate_turn(&view),
            GateDecision::Conclusive(CompletionState::WaitingOnBackground)
        );
    }

    #[test]
    fn tool_using_candidate_final_answers_are_sent_for_evaluation() {
        // The expensive case, and the only one: tools ran and text came back,
        // so whether the request is done is a semantic question.
        assert_eq!(
            gate_turn(&summary("Done.", 3, true)),
            GateDecision::Evaluate
        );
    }

    #[test]
    fn a_permanent_failure_never_continues() {
        assert_eq!(
            gate_turn(&summary("", 0, false)),
            GateDecision::Conclusive(CompletionState::Failed)
        );
    }

    #[test]
    fn cancellation_is_blocked_rather_than_failed() {
        // Blocked and Failed differ in what a host does next: a cancelled turn
        // is the user's decision, not an error to report or retry.
        let mut view = summary("", 0, false);
        view.stop_reason = TurnStopReason::Cancelled;
        assert_eq!(
            gate_turn(&view),
            GateDecision::Conclusive(CompletionState::Blocked)
        );

        let mut succeeded_but_cancelled = summary("partial", 1, true);
        succeeded_but_cancelled.stop_reason = TurnStopReason::Cancelled;
        assert_eq!(
            gate_turn(&succeeded_but_cancelled),
            GateDecision::Conclusive(CompletionState::Blocked)
        );
    }

    #[test]
    fn hitting_a_ceiling_is_in_progress_even_with_text() {
        for stop_reason in [TurnStopReason::MaxTokens, TurnStopReason::MaxTurnRequests] {
            let mut view = summary("I was in the middle of", 2, true);
            view.stop_reason = stop_reason;
            assert_eq!(
                gate_turn(&view),
                GateDecision::Conclusive(CompletionState::InProgress),
                "{stop_reason:?} means stopped mid-task"
            );
        }
    }

    #[test]
    fn a_question_back_to_the_user_blocks() {
        assert_eq!(
            gate_turn(&summary("Which environment should I deploy to?", 0, true)),
            GateDecision::Conclusive(CompletionState::Blocked)
        );
    }

    #[test]
    fn a_rhetorical_question_does_not_block() {
        // No marker phrase, so this stays an ordinary answer rather than
        // stranding the user waiting for a reply nobody asked for.
        assert_eq!(
            gate_turn(&summary("Ready to ship?", 0, true)),
            GateDecision::Conclusive(CompletionState::Achieved)
        );
    }

    #[test]
    fn the_budget_stops_at_the_turn_that_crosses_a_limit() {
        let mut budget = ContinuationBudget::default();
        for _ in 0..DEFAULT_MAX_CONTINUATION_TURNS {
            assert!(budget.observe_turn(1));
        }
        assert!(!budget.observe_turn(1), "the ceiling is on work done");

        budget.reset();
        assert_eq!(budget.usage(), (0, 0));
        assert!(!budget.observe_turn(DEFAULT_MAX_CONTINUATION_TOKENS + 1));
        assert_eq!(
            budget.usage(),
            (1, DEFAULT_MAX_CONTINUATION_TOKENS + 1),
            "the crossing turn is still recorded"
        );
    }

    #[test]
    fn an_elapsed_budget_stops_a_cheap_but_slow_loop() {
        // Turns and tokens both untouched; only wall-clock is exhausted.
        let mut budget = ContinuationBudget::new(100, u64::MAX, Duration::ZERO);
        std::thread::sleep(Duration::from_millis(1));
        assert!(!budget.observe_turn(1));
    }
}
