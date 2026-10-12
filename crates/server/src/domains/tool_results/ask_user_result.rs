//! Building the `ask_user` result the model reads.
//!
//! Decision: pure result construction lives outside the HTTP layer so the
//! message domain (cancelling a pending question set) and every answer surface
//! share one attribution rule. `api::question_answers` re-exports it.

use everruns_core::builtins::ask_user::{
    AskUserAnswer, AskUserAnsweredBy, AskUserResult, AskUserStatus,
};

/// Build the result the model reads.
///
/// `answered_by` is decided here, never taken from the payload: a model-asserted
/// "a human answered this" would answer nothing. Only an outcome a person
/// actually produced is attributed to `User`.
pub(crate) fn build_result(status: AskUserStatus, answers: Vec<AskUserAnswer>) -> AskUserResult {
    build_result_with_source(status, default_answered_by(status), answers)
}

pub(crate) fn default_answered_by(status: AskUserStatus) -> AskUserAnsweredBy {
    match status {
        AskUserStatus::Answered | AskUserStatus::Declined => AskUserAnsweredBy::User,
        AskUserStatus::TimedOut => AskUserAnsweredBy::Timeout,
        AskUserStatus::Cancelled => AskUserAnsweredBy::Unattended,
    }
}

/// Build a result for a trusted resolution surface whose source cannot be
/// inferred from the status, such as a timeout that declines a secret.
pub(crate) fn build_result_with_source(
    status: AskUserStatus,
    answered_by: AskUserAnsweredBy,
    answers: Vec<AskUserAnswer>,
) -> AskUserResult {
    AskUserResult {
        status,
        answered_by,
        // `answered` and `timed_out` carry answers; `declined` and `cancelled`
        // do not. A decline that shipped the options the person refused to
        // choose between would read to the model as a choice, but a timeout
        // *is* the declared defaults being applied (EVE-1056) — dropping them
        // would leave the model with no value at all. `answered_by: timeout`
        // is what tells it no human spoke, so the value is not consent.
        answers: if matches!(status, AskUserStatus::Answered | AskUserStatus::TimedOut) {
            answers
        } else {
            Vec::new()
        },
    }
}
