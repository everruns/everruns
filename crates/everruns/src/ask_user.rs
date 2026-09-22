//! Stable host contract and values for structured in-process questions.
//!
//! Embedded hosts implement [`AskUser`] and register the responder through
//! [`AgentBuilder::ask_user`](crate::AgentBuilder::ask_user). Hosted clients use
//! the same request and outcome values through a client-side capability.

pub use async_trait::async_trait;
pub use everruns_builtins::{
    AskUser, AskUserAnswer, AskUserAnsweredBy, AskUserOption, AskUserQuestion, AskUserQuestionKind,
    AskUserRequest, AskUserResult, AskUserStatus, DefaultsResponder, session_secret_ref,
};

/// One answer in an [`Outcome`].
pub use AskUserAnswer as Answer;
/// The source that supplied an [`Outcome`].
pub use AskUserAnsweredBy as AnsweredBy;
/// One structured question in a responder request.
pub use AskUserQuestion as Question;
/// The supported question kinds: a choice, or a credential.
pub use AskUserQuestionKind as QuestionKind;
/// The complete result returned by an [`AskUser`] responder.
pub use AskUserResult as Outcome;
/// The terminal state of an [`Outcome`].
pub use AskUserStatus as Status;
