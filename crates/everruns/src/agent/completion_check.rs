//! The completion check: the builder side of
//! [`CompletionCheck`] and [`ContinuationLimits`].

use super::AgentBuilder;
use crate::completion_check::{CompletionCheck, ContinuationLimits};

impl AgentBuilder {
    /// When a turn about to finish is reviewed on the agent's own model for
    /// whether it did what was asked. A turn that ended on an announcement
    /// ("Running the tests now") without the work gets an automatic
    /// continuation in the same turn, labeled as not written by the person.
    ///
    /// The default, [`CompletionCheck::Unattended`], checks only sessions no
    /// person watches. A coding agent or any host that wants every finishing
    /// turn checked sets [`CompletionCheck::Always`].
    ///
    /// ```
    /// use everruns::completion_check::{CompletionCheck, ContinuationLimits};
    ///
    /// let builder = everruns::Agent::builder()
    ///     .name("coder")
    ///     .instructions("Fix the failing test.")
    ///     .completion_check(CompletionCheck::Always)
    ///     .continuation_limits(ContinuationLimits {
    ///         max_turns: 5,
    ///         max_tokens: 256_000,
    ///         max_elapsed_secs: 600,
    ///     });
    /// # let _ = builder;
    /// ```
    pub fn completion_check(mut self, check: CompletionCheck) -> Self {
        self.completion_check = check;
        self
    }

    /// Bound the completion check's automatic continuations for one request.
    /// Unset keeps the runtime defaults. The time limit restarts whenever a
    /// continuation runs tools, so a long task that keeps working is bounded
    /// by turns and tokens, and only a stalled one runs out of time.
    pub fn continuation_limits(mut self, limits: ContinuationLimits) -> Self {
        self.continuation_limits = Some(limits);
        self
    }
}
