//! Credit shortfall inside a reason step: the balance check a credit-check
//! turn runs before any model call, and the wait a refusal starts. The policy
//! and its durability notes live in [`crate::engine::credit_shortfall`].

use chrono::Utc;

use crate::engine::credit_shortfall::{self as policy, CreditCheck, Record, Wait};
use crate::engine::error::{AgentLoopError, Result};
use crate::engine::events::CreditShortfallAction;
use crate::engine::runtime_context::AssembledTurnContext;
use crate::engine::{ExecutionContext, UserFacingErrorContext, user_facing_error_codes};
use everruns_contracts::provider_limit::CreditStatus;

use super::{ReasonAtom, ReasonResult, default_max_iterations};

fn record<'a>(atom: &'a ReasonAtom, context: &ExecutionContext) -> Record<'a> {
    Record {
        emitter: atom.event_emitter.as_ref(),
        session_id: context.session_id,
        turn_id: context.turn_id,
        input_message_id: context.input_message_id,
    }
}

/// A reason that ends the turn without calling the model or answering.
fn quiet(max_iterations: usize) -> ReasonResult {
    ReasonResult {
        success: true,
        commentary: true,
        max_iterations,
        ..ReasonResult::default()
    }
}

/// What a credit check found before the model call.
pub(crate) enum BeforeCall {
    /// Call the model. `balance_checked`: the provider's balance said credits
    /// are back (`false`: no balance API, the request is the check).
    Call { balance_checked: bool },
    /// Do not call the model: end the reason with this.
    Done(Box<Result<ReasonResult>>),
}

/// Read the balance on a credit-check turn, without spending tokens.
pub(crate) async fn before_call(
    atom: &ReasonAtom,
    context: &ExecutionContext,
    assembled: &AssembledTurnContext,
    check: CreditCheck,
) -> BeforeCall {
    let Some(store) = atom.schedule_store.as_deref() else {
        return BeforeCall::Call {
            balance_checked: false,
        };
    };
    let record = record(atom, context);
    match policy::read_balance(assembled.model.driver.as_ref()).await {
        Some(CreditStatus::Exhausted) => {}
        status => {
            // Back, or only the request can tell: no other check may resume
            // this wait a second time.
            policy::cancel_queued_checks(store, context.session_id).await;
            let balance_checked = status.is_some();
            if balance_checked {
                let mut data = record.data(CreditShortfallAction::Resumed);
                data.wait_until = Some(check.deadline);
                data.balance_checked = Some(true);
                record.emit(data).await;
            }
            return BeforeCall::Call { balance_checked };
        }
    }
    match policy::schedule_check(store, context.session_id, Some(check), Utc::now()).await {
        Wait::Scheduled { deadline, next } => {
            let mut data = record.data(CreditShortfallAction::StillWaiting);
            data.wait_until = Some(deadline);
            data.next_check_at = Some(next);
            data.balance_checked = Some(true);
            record.emit(data).await;
            BeforeCall::Done(Box::new(Ok(quiet(assembled.runtime_agent.max_iterations))))
        }
        // A replayed check: the next one is already queued and recorded.
        Wait::Queued { .. } => {
            BeforeCall::Done(Box::new(Ok(quiet(assembled.runtime_agent.max_iterations))))
        }
        // The refusal path records giving up, as for a refused request.
        Wait::Expired { .. } | Wait::Unavailable => {
            BeforeCall::Done(Box::new(Err(policy::gave_up_error())))
        }
    }
}

/// Record that a request without a balance check went through.
pub(crate) async fn resumed_by_request(
    atom: &ReasonAtom,
    context: &ExecutionContext,
    check: CreditCheck,
) {
    let record = record(atom, context);
    let mut data = record.data(CreditShortfallAction::Resumed);
    data.wait_until = Some(check.deadline);
    data.balance_checked = Some(false);
    record.emit(data).await;
}

/// After a terminal refusal: start or continue the wait for credits.
///
/// Returns the reason result (a credit-check turn that keeps waiting ends
/// quietly) and whether the refusal's copy should say a wait is running.
pub(crate) async fn after_refusal(
    atom: &ReasonAtom,
    context: &ExecutionContext,
    check: Option<CreditCheck>,
    error: AgentLoopError,
) -> (Result<ReasonResult>, bool) {
    let classified = error.user_facing_error(UserFacingErrorContext::default());
    if classified.code != user_facing_error_codes::PROVIDER_QUOTA_EXHAUSTED {
        return (Err(error), false);
    }
    let Some(store) = atom.schedule_store.as_deref() else {
        return (Err(error), false);
    };
    let record = record(atom, context);
    let refusal = |action| policy::with_refusal(record.data(action), &classified);
    match policy::schedule_check(store, context.session_id, check, Utc::now()).await {
        Wait::Scheduled { deadline, next } => {
            let mut data = refusal(if check.is_some() {
                CreditShortfallAction::StillWaiting
            } else {
                CreditShortfallAction::WaitStarted
            });
            data.wait_until = Some(deadline);
            data.next_check_at = Some(next);
            if check.is_some() {
                data.balance_checked = Some(false);
            }
            record.emit(data).await;
            match check {
                Some(_) => (Ok(quiet(default_max_iterations())), true),
                None => (Err(error), true),
            }
        }
        Wait::Queued { .. } => match check {
            Some(_) => (Ok(quiet(default_max_iterations())), true),
            None => (Err(error), true),
        },
        Wait::Expired { deadline } if check.is_some() => {
            let mut data = refusal(CreditShortfallAction::GaveUp);
            data.wait_until = Some(deadline);
            record.emit(data).await;
            (Err(error), false)
        }
        Wait::Expired { .. } | Wait::Unavailable => (Err(error), false),
    }
}
